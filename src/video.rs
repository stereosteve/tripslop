//! Random-access video clips.
//!
//! On import, ffmpeg transcodes the file once into a stream of intra-only JPEG frames at the
//! composition resolution (the same idea as Resolume's DXV codec). The compressed frames
//! stay in memory (~100-150KB each), so any frame can be decoded instantly: that is what makes
//! bounce, reverse, random and speed changes possible. A worker thread decodes whichever
//! frame the playhead wants most recently.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};

use crate::source::Frame;

pub struct VideoMedia {
    pub path: PathBuf,
    pub fps: f32,
    /// Frame count reported by ffprobe (an estimate until loading finishes).
    pub expected_frames: usize,
    frames: Arc<RwLock<Vec<Arc<[u8]>>>>,
    loaded: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    decoder: Decoder,
}

impl VideoMedia {
    pub fn import(path: &Path, width: u32, height: u32) -> Result<Self, String> {
        if !path.exists() {
            return Err(format!("no such file: {}", path.display()));
        }
        let (fps, expected_frames) = probe(path)?;
        let frames: Arc<RwLock<Vec<Arc<[u8]>>>> = Arc::default();
        let loaded = Arc::new(AtomicBool::new(false));
        let error: Arc<Mutex<Option<String>>> = Arc::default();

        let mut child = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i"])
            .arg(path)
            .args(["-an", "-vf"])
            .arg(format!(
                "scale={width}:{height}:force_original_aspect_ratio=increase,crop={width}:{height}"
            ))
            .args(["-fps_mode", "passthrough", "-c:v", "mjpeg", "-q:v", "3", "-pix_fmt", "yuvj420p"])
            .args(["-f", "image2pipe", "pipe:1"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("failed to start ffmpeg (is it installed and on PATH?): {e}"))?;

        let mut stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let (f, l, e) = (frames.clone(), loaded.clone(), error.clone());
        std::thread::spawn(move || {
            let mut splitter = JpegSplitter::default();
            let mut buf = vec![0u8; 1 << 16];
            loop {
                match stdout.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        for jpeg in splitter.push(&buf[..n]) {
                            f.write().unwrap().push(jpeg.into());
                        }
                    }
                    Err(err) => {
                        *e.lock().unwrap() = Some(err.to_string());
                        break;
                    }
                }
            }
            let mut msg = String::new();
            let _ = stderr.read_to_string(&mut msg);
            let ok = child.wait().map(|s| s.success()).unwrap_or(false);
            if !ok || f.read().unwrap().is_empty() {
                let last = msg.trim().lines().last().unwrap_or("could not decode video").to_string();
                *e.lock().unwrap() = Some(last);
            }
            l.store(true, Ordering::Release);
        });

        Ok(Self {
            path: path.to_path_buf(),
            fps,
            expected_frames,
            decoder: Decoder::new(frames.clone()),
            frames,
            loaded,
            error,
        })
    }

    pub fn frame_count(&self) -> usize {
        self.frames.read().unwrap().len()
    }

    /// Frames to use for playback math: the real count once loaded, the estimate before.
    pub fn length(&self) -> usize {
        if self.is_loaded() {
            self.frame_count()
        } else {
            self.expected_frames.max(self.frame_count())
        }
        .max(1)
    }

    pub fn is_loaded(&self) -> bool {
        self.loaded.load(Ordering::Acquire)
    }

    /// 0..1 import progress.
    pub fn progress(&self) -> f32 {
        if self.is_loaded() {
            1.0
        } else {
            (self.frame_count() as f32 / self.expected_frames.max(1) as f32).min(0.99)
        }
    }

    pub fn error(&self) -> Option<String> {
        self.error.lock().unwrap().clone()
    }

    pub fn memory_bytes(&self) -> usize {
        self.frames.read().unwrap().iter().map(|f| f.len()).sum()
    }

    /// Ask for a frame (clamped to what has been imported so far).
    pub fn request(&self, index: usize) {
        let n = self.frame_count();
        if n > 0 {
            self.decoder.request(index.min(n - 1));
        }
    }

    /// The most recently decoded frame, if it's new since the last call.
    pub fn take_decoded(&self) -> Option<(usize, Frame)> {
        self.decoder.take()
    }

    /// Small RGBA preview of the first frame, for the clip grid.
    pub fn thumbnail(&self, w: u32, h: u32) -> Option<image::RgbaImage> {
        let jpeg = self.frames.read().unwrap().first()?.clone();
        let img = image::load_from_memory_with_format(&jpeg, image::ImageFormat::Jpeg).ok()?;
        Some(img.thumbnail_exact(w, h).to_rgba8())
    }
}

/// `(fps, frame count)` from ffprobe.
fn probe(path: &Path) -> Result<(f32, usize), String> {
    let out = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0"])
        .args(["-show_entries", "stream=avg_frame_rate,r_frame_rate,nb_frames:format=duration"])
        .args(["-of", "default=nw=1"])
        .arg(path)
        .output()
        .map_err(|e| format!("failed to run ffprobe: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut fps = 0.0f32;
    let mut frames = 0usize;
    let mut duration = 0.0f32;
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        match k {
            "avg_frame_rate" | "r_frame_rate" if fps <= 0.0 => fps = parse_rate(v),
            "nb_frames" => frames = v.parse().unwrap_or(0),
            "duration" => duration = v.parse().unwrap_or(0.0),
            _ => {}
        }
    }
    if fps <= 0.0 || !fps.is_finite() {
        if text.trim().is_empty() {
            return Err(format!("{}: not a video ffmpeg can read", path.display()));
        }
        fps = 30.0;
    }
    if frames == 0 {
        frames = (duration * fps).round() as usize;
    }
    Ok((fps.clamp(1.0, 240.0), frames))
}

fn parse_rate(v: &str) -> f32 {
    match v.split_once('/') {
        Some((n, d)) => {
            let (n, d) = (n.parse::<f32>().unwrap_or(0.0), d.parse::<f32>().unwrap_or(0.0));
            if d > 0.0 { n / d } else { 0.0 }
        }
        None => v.parse().unwrap_or(0.0),
    }
}

/// Splits a byte stream of concatenated JPEGs into individual images, by walking the marker
/// segments properly (table data inside headers can contain 0xFFD9, so a naive search for EOI
/// isn't safe).
#[derive(Default)]
struct JpegSplitter {
    buf: Vec<u8>,
}

impl JpegSplitter {
    fn push(&mut self, data: &[u8]) -> Vec<Vec<u8>> {
        self.buf.extend_from_slice(data);
        let mut out = Vec::new();
        while let Some(end) = jpeg_end(&self.buf) {
            out.push(self.buf.drain(..end).collect());
        }
        out
    }
}

/// Length of the complete JPEG at the start of `b`, if it's all there.
fn jpeg_end(b: &[u8]) -> Option<usize> {
    if b.len() < 4 || b[0] != 0xFF || b[1] != 0xD8 {
        return None;
    }
    let mut i = 2;
    loop {
        if i + 4 > b.len() {
            return None;
        }
        if b[i] != 0xFF {
            return None; // corrupt; wait for more data (won't happen with ffmpeg output)
        }
        let marker = b[i + 1];
        let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
        i += 2 + len;
        if marker == 0xDA {
            // Start of scan: entropy-coded data until a marker that isn't stuffing or RSTn.
            while i + 1 < b.len() {
                if b[i] == 0xFF {
                    let m = b[i + 1];
                    if m == 0x00 || (0xD0..=0xD7).contains(&m) || m == 0xFF {
                        i += if m == 0xFF { 1 } else { 2 };
                        continue;
                    }
                    if m == 0xD9 {
                        return Some(i + 2);
                    }
                    // Another segment (e.g. progressive scans): keep walking segments.
                    break;
                }
                i += 1;
            }
            if i + 1 >= b.len() {
                return None;
            }
        }
    }
}

struct DecoderShared {
    wanted: Mutex<Option<usize>>,
    wake: Condvar,
    result: Mutex<Option<(usize, Frame)>>,
    stop: AtomicBool,
}

/// Background JPEG decoder that always works on the most recently requested frame.
struct Decoder {
    shared: Arc<DecoderShared>,
    last_requested: Mutex<Option<usize>>,
}

impl Decoder {
    fn new(frames: Arc<RwLock<Vec<Arc<[u8]>>>>) -> Self {
        let shared = Arc::new(DecoderShared {
            wanted: Mutex::new(None),
            wake: Condvar::new(),
            result: Mutex::new(None),
            stop: AtomicBool::new(false),
        });
        let s = shared.clone();
        std::thread::spawn(move || {
            loop {
                let idx = {
                    let mut w = s.wanted.lock().unwrap();
                    while w.is_none() && !s.stop.load(Ordering::Relaxed) {
                        w = s.wake.wait(w).unwrap();
                    }
                    if s.stop.load(Ordering::Relaxed) {
                        return;
                    }
                    w.take().unwrap()
                };
                let Some(jpeg) = frames.read().unwrap().get(idx).cloned() else { continue };
                if let Ok(img) = image::load_from_memory_with_format(&jpeg, image::ImageFormat::Jpeg) {
                    let rgba = img.to_rgba8();
                    let frame = Frame {
                        width: rgba.width(),
                        height: rgba.height(),
                        rgba: rgba.into_raw(),
                    };
                    *s.result.lock().unwrap() = Some((idx, frame));
                }
            }
        });
        Self {
            shared,
            last_requested: Mutex::new(None),
        }
    }

    fn request(&self, idx: usize) {
        let mut last = self.last_requested.lock().unwrap();
        if *last == Some(idx) {
            return;
        }
        *last = Some(idx);
        *self.shared.wanted.lock().unwrap() = Some(idx);
        self.shared.wake.notify_one();
    }

    fn take(&self) -> Option<(usize, Frame)> {
        self.shared.result.lock().unwrap().take()
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.shared.wake.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_jpeg(seed: u8) -> Vec<u8> {
        let img = image::RgbImage::from_fn(16, 8, |x, y| image::Rgb([x as u8 * 9 + seed, y as u8 * 20, 0xD9]));
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90)
            .encode_image(&img)
            .unwrap();
        out
    }

    #[test]
    fn splitter_handles_arbitrary_chunking() {
        let jpegs: Vec<Vec<u8>> = (0..5).map(tiny_jpeg).collect();
        let stream: Vec<u8> = jpegs.concat();
        for chunk in [1, 7, 64, 1000, stream.len()] {
            let mut s = JpegSplitter::default();
            let mut got = Vec::new();
            for c in stream.chunks(chunk) {
                got.extend(s.push(c));
            }
            assert_eq!(got, jpegs, "chunk size {chunk}");
        }
    }

    #[test]
    fn parse_rates() {
        assert_eq!(parse_rate("30000/1001"), 30000.0 / 1001.0);
        assert_eq!(parse_rate("25/1"), 25.0);
        assert_eq!(parse_rate("0/0"), 0.0);
    }
}

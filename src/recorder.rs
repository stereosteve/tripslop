//! Video recording of the output.
//!
//! Every 60Hz tick, the finished output frame is copied into one of a few GPU staging
//! buffers. Buffers are mapped asynchronously and drained in order on later ticks. Raw RGBA
//! frames go to a writer thread that pipes them into an `ffmpeg` encoder.
//!
//! What happens when the GPU or ffmpeg falls behind depends on the [`Mode`]:
//! * [`Mode::Live`] never blocks the render loop. A frame that can't be captured or queued
//!   is dropped, and the writer repeats the previous frame in its place, so the file keeps
//!   real-time duration at a constant 60 fps.
//! * [`Mode::Exact`] waits instead, so every tick becomes exactly one frame (scripts).

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, OnceLock};
use std::thread::JoinHandle;

use eframe::egui_wgpu::wgpu;

use crate::renderer::{HEIGHT, WIDTH};

pub const FPS: u32 = 60;
const STAGING_BUFFERS: usize = 4;
/// Frames buffered between the render loop and ffmpeg (~3.7MB each).
const WRITE_QUEUE: usize = 16;

const PENDING: u8 = 0;
const MAPPED: u8 = 1;
const FAILED: u8 = 2;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// Drop frames rather than stall the render loop (performing).
    Live,
    /// Wait for the GPU and encoder so no frame is ever dropped (scripts, export).
    Exact,
}

struct InFlight {
    buffer: wgpu::Buffer,
    state: Arc<AtomicU8>,
    /// Ticks after this one that were skipped (no staging buffer was free); the writer
    /// repeats this frame for them.
    repeats: u32,
}

/// What the render loop sends the writer thread.
enum Msg {
    Frame(Vec<u8>),
    /// Write the previous frame again this many times.
    Repeat(u32),
}

/// The render-loop end of the writer queue: applies the [`Mode`]'s overload policy.
struct FrameSink {
    tx: SyncSender<Msg>,
    mode: Mode,
    /// Repeats that couldn't be queued yet (the queue was full); sent before the next frame.
    owed: u32,
    /// Frames in the file so far, repeats included.
    frames: u64,
    /// Frames that were replaced by a repeat.
    dropped: u64,
}

impl FrameSink {
    fn new(tx: SyncSender<Msg>, mode: Mode) -> Self {
        Self { tx, mode, owed: 0, frames: 0, dropped: 0 }
    }

    /// Queue `frame` followed by `repeats` copies of it. With `block` (or in exact mode),
    /// waits for room in the queue; otherwise a full queue drops the frame.
    fn push(&mut self, frame: Vec<u8>, repeats: u32, block: bool) -> Result<(), String> {
        let block = block || self.mode == Mode::Exact;
        if self.owed > 0 {
            if !self.send(Msg::Repeat(self.owed), block)? {
                self.drop_frames(1 + repeats);
                return Ok(());
            }
            self.owed = 0;
        }
        if !self.send(Msg::Frame(frame), block)? {
            self.drop_frames(1 + repeats);
            return Ok(());
        }
        self.frames += 1;
        if repeats > 0 {
            self.dropped += repeats as u64;
            if !self.send(Msg::Repeat(repeats), block)? {
                self.owed = repeats;
            }
        }
        self.frames += repeats as u64;
        Ok(())
    }

    /// Frames that will be filled in by repeating the last one written.
    fn drop_frames(&mut self, n: u32) {
        self.owed += n;
        self.dropped += n as u64;
        self.frames += n as u64;
    }

    /// Ok(false) if the queue was full and `block` is off.
    fn send(&self, msg: Msg, block: bool) -> Result<bool, String> {
        let closed = || "ffmpeg stopped accepting frames".to_string();
        if block {
            return self.tx.send(msg).map(|()| true).map_err(|_| closed());
        }
        match self.tx.try_send(msg) {
            Ok(()) => Ok(true),
            Err(TrySendError::Full(_)) => Ok(false),
            Err(TrySendError::Disconnected(_)) => Err(closed()),
        }
    }

    /// Flush any owed repeats, waiting for room.
    fn finish(mut self) -> Result<(), String> {
        if self.owed > 0 {
            self.send(Msg::Repeat(self.owed), true)?;
            self.owed = 0;
        }
        Ok(())
    }
}

/// The writer thread's loop: frames and repeats go to `out` in order.
fn write_frames(rx: Receiver<Msg>, out: &mut impl Write) -> std::io::Result<()> {
    let mut last: Option<Vec<u8>> = None;
    for msg in rx {
        match msg {
            Msg::Frame(f) => {
                out.write_all(&f)?;
                last = Some(f);
            }
            Msg::Repeat(n) => {
                // Nothing to repeat if the very first frames were dropped; the file starts
                // a little late rather than with garbage.
                if let Some(f) = &last {
                    for _ in 0..n {
                        out.write_all(f)?;
                    }
                }
            }
        }
    }
    Ok(())
}

pub struct Recorder {
    pub path: PathBuf,
    pub encoder: &'static str,
    pub mode: Mode,
    free: Vec<wgpu::Buffer>,
    in_flight: VecDeque<InFlight>,
    sink: Option<FrameSink>,
    writer: Option<JoinHandle<Result<(), String>>>,
}

/// A recording that has stopped capturing and is waiting for ffmpeg to finish writing.
pub struct Finishing {
    pub path: PathBuf,
    pub frames: u64,
    pub dropped: u64,
    handle: JoinHandle<Result<(), String>>,
}

impl Finishing {
    pub fn is_done(&self) -> bool {
        self.handle.is_finished()
    }

    pub fn join(self) -> Result<(PathBuf, u64), String> {
        match self.handle.join() {
            Ok(Ok(())) => Ok((self.path, self.frames)),
            Ok(Err(e)) => Err(e),
            Err(_) => Err("recording writer thread panicked".into()),
        }
    }
}

/// Hardware H.264 on macOS when available, otherwise x264.
fn encoder_args() -> (&'static str, &'static [&'static str]) {
    static HW: OnceLock<bool> = OnceLock::new();
    let hw = *HW.get_or_init(|| {
        Command::new("ffmpeg")
            .args(["-hide_banner", "-encoders"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).contains("h264_videotoolbox"))
            .unwrap_or(false)
    });
    if hw {
        ("h264_videotoolbox", &["-c:v", "h264_videotoolbox", "-b:v", "24M"])
    } else {
        ("libx264", &["-c:v", "libx264", "-preset", "veryfast", "-crf", "16"])
    }
}

impl Recorder {
    pub fn start(device: &wgpu::Device, path: PathBuf, mode: Mode) -> Result<Self, String> {
        let (encoder, codec) = encoder_args();
        let size = format!("{WIDTH}x{HEIGHT}");
        let fps = FPS.to_string();
        let mut child = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y"])
            .args(["-f", "rawvideo", "-pix_fmt", "rgba", "-s", &size, "-framerate", &fps, "-i", "pipe:0"])
            .args(codec)
            .args(["-pix_fmt", "yuv420p", "-movflags", "+faststart"])
            .arg(&path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("failed to start ffmpeg (is it installed and on PATH?): {e}"))?;

        let (tx, rx) = sync_channel::<Msg>(WRITE_QUEUE);
        let writer = std::thread::spawn(move || -> Result<(), String> {
            let mut stdin = child.stdin.take().expect("piped stdin");
            let write_err = write_frames(rx, &mut stdin).err();
            // Closing stdin tells ffmpeg the stream is over, so it finalizes the file.
            drop(stdin);
            finish_ffmpeg(child, write_err)
        });

        let free = (0..STAGING_BUFFERS)
            .map(|_| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("record staging"),
                    size: (4 * WIDTH * HEIGHT) as u64,
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                    mapped_at_creation: false,
                })
            })
            .collect();

        Ok(Self {
            path,
            encoder,
            mode,
            free,
            in_flight: VecDeque::new(),
            sink: Some(FrameSink::new(tx, mode)),
            writer: Some(writer),
        })
    }

    /// Frames in the file so far, including repeats standing in for dropped ones.
    pub fn frames(&self) -> u64 {
        self.sink.as_ref().map_or(0, |s| s.frames)
    }

    /// Frames that were dropped (and filled in by repeating the previous one).
    pub fn dropped(&self) -> u64 {
        self.sink.as_ref().map_or(0, |s| s.dropped)
    }

    /// Queue a copy of `texture` (the output frame) for encoding.
    pub fn capture(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Result<(), String> {
        self.drain(device, false, false)?;
        if self.free.is_empty() {
            match self.mode {
                // GPU is behind: wait for the oldest copy rather than drop a frame.
                Mode::Exact => self.drain(device, true, false)?,
                // Skip this tick; the newest captured frame stands in for it.
                Mode::Live => {
                    let newest = self.in_flight.back_mut().expect("no free buffer means some are in flight");
                    newest.repeats += 1;
                    return Ok(());
                }
            }
        }
        let buffer = self.free.pop().expect("a staging buffer is free after draining");

        let mut enc = device.create_command_encoder(&Default::default());
        enc.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    // 5120 bytes: already a multiple of COPY_BYTES_PER_ROW_ALIGNMENT.
                    bytes_per_row: Some(4 * WIDTH),
                    rows_per_image: Some(HEIGHT),
                },
            },
            wgpu::Extent3d {
                width: WIDTH,
                height: HEIGHT,
                depth_or_array_layers: 1,
            },
        );
        queue.submit([enc.finish()]);

        let state = Arc::new(AtomicU8::new(PENDING));
        let cb_state = state.clone();
        buffer.map_async(wgpu::MapMode::Read, .., move |r| {
            cb_state.store(if r.is_ok() { MAPPED } else { FAILED }, Ordering::Release);
        });
        self.in_flight.push_back(InFlight { buffer, state, repeats: 0 });
        Ok(())
    }

    /// Hand finished copies to the writer, oldest first. With `wait`, block until at least
    /// the oldest one is done. With `block`, wait for room in the writer queue even in live
    /// mode (when stopping).
    fn drain(&mut self, device: &wgpu::Device, wait: bool, block: bool) -> Result<(), String> {
        let poll = if wait {
            wgpu::PollType::Wait {
                submission_index: None,
                timeout: None,
            }
        } else {
            wgpu::PollType::Poll
        };
        let _ = device.poll(poll);
        while let Some(front) = self.in_flight.front() {
            match front.state.load(Ordering::Acquire) {
                PENDING => break,
                MAPPED => {
                    let f = self.in_flight.pop_front().unwrap();
                    let data = f
                        .buffer
                        .slice(..)
                        .get_mapped_range()
                        .map_err(|e| format!("{e:?}"))?
                        .to_vec();
                    f.buffer.unmap();
                    self.free.push(f.buffer);
                    let sink = self.sink.as_mut().ok_or("recorder already stopped")?;
                    sink.push(data, f.repeats, block)?;
                }
                _ => {
                    let f = self.in_flight.pop_front().unwrap();
                    self.free.push(f.buffer);
                    return Err("GPU readback failed".into());
                }
            }
        }
        Ok(())
    }

    /// Flush the remaining frames and let ffmpeg finish in the background.
    pub fn stop(mut self, device: &wgpu::Device) -> Finishing {
        while !self.in_flight.is_empty() {
            if self.drain(device, true, true).is_err() {
                break;
            }
        }
        let (frames, dropped) = (self.frames(), self.dropped());
        if let Some(sink) = self.sink.take() {
            // An error here means ffmpeg already quit; the writer thread reports why.
            let _ = sink.finish();
        }
        Finishing {
            path: self.path.clone(),
            frames,
            dropped,
            handle: self.writer.take().expect("writer thread"),
        }
    }
}

fn finish_ffmpeg(mut child: Child, write_err: Option<std::io::Error>) -> Result<(), String> {
    let mut stderr = String::new();
    if let Some(mut s) = child.stderr.take() {
        let _ = s.read_to_string(&mut stderr);
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    let last_line = stderr.trim().lines().last().unwrap_or("").to_string();
    if !status.success() {
        return Err(format!("ffmpeg failed ({status}): {last_line}"));
    }
    if let Some(e) = write_err {
        return Err(format!("writing frames to ffmpeg failed: {e} {last_line}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(n: u8) -> Vec<u8> {
        vec![n; 4]
    }

    /// Run the writer over everything the sink queued and return the frame ids written.
    fn written(sink: FrameSink, rx: Receiver<Msg>) -> Vec<u8> {
        sink.finish().unwrap();
        let mut out = Vec::new();
        write_frames(rx, &mut out).unwrap();
        out.chunks(4).map(|c| c[0]).collect()
    }

    #[test]
    fn exact_mode_keeps_every_frame() {
        let (tx, rx) = sync_channel(64);
        let mut sink = FrameSink::new(tx, Mode::Exact);
        for i in 0..5 {
            sink.push(frame(i), 0, false).unwrap();
        }
        assert_eq!((sink.frames, sink.dropped), (5, 0));
        assert_eq!(written(sink, rx), [0, 1, 2, 3, 4]);
    }

    #[test]
    fn skipped_ticks_repeat_the_previous_frame() {
        let (tx, rx) = sync_channel(64);
        let mut sink = FrameSink::new(tx, Mode::Live);
        sink.push(frame(1), 2, false).unwrap();
        sink.push(frame(2), 0, false).unwrap();
        assert_eq!((sink.frames, sink.dropped), (4, 2));
        assert_eq!(written(sink, rx), [1, 1, 1, 2]);
    }

    #[test]
    fn full_queue_drops_without_blocking_and_keeps_duration() {
        // Room for two messages and nobody draining: a stalled ffmpeg.
        let (tx, rx) = sync_channel(2);
        let mut sink = FrameSink::new(tx, Mode::Live);
        for i in 0..6 {
            sink.push(frame(i), 0, false).unwrap();
        }
        assert_eq!((sink.frames, sink.dropped), (6, 4));
        // Once ffmpeg catches up, the dropped frames come out as repeats of the last one queued.
        let mut out = Vec::new();
        let drain = std::thread::spawn(move || {
            write_frames(rx, &mut out).unwrap();
            out.chunks(4).map(|c| c[0]).collect::<Vec<_>>()
        });
        sink.push(frame(9), 0, true).unwrap();
        sink.finish().unwrap();
        assert_eq!(drain.join().unwrap(), [0, 1, 1, 1, 1, 1, 9]);
    }

    #[test]
    fn closed_writer_is_an_error() {
        let (tx, rx) = sync_channel(4);
        drop(rx);
        let mut sink = FrameSink::new(tx, Mode::Live);
        assert!(sink.push(frame(0), 0, false).is_err());
    }
}

//! Still images and live capture devices. (Video files live in `video.rs`.)
//!
//! Cameras are read through an `ffmpeg` subprocess that writes raw RGBA frames to stdout.
//! That keeps the build free of native ffmpeg bindings, and gives us every capture device
//! ffmpeg knows about.

use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

/// A decoded frame ready for upload.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

const VIDEO_EXTS: &[&str] = &[
    "mp4", "mov", "m4v", "mkv", "webm", "avi", "gif", "mpg", "mpeg", "wmv", "flv", "ts", "mts",
];

/// Fragment shader files: GLSL (Shadertoy / GLSL Sandbox style) or WGSL.
pub fn is_shader(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| matches!(e.to_ascii_lowercase().as_str(), "glsl" | "frag" | "fs" | "shader" | "wgsl"))
        .unwrap_or(false)
}

pub fn is_video(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| VIDEO_EXTS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Decode a still image (premultiplied later, on the GPU).
pub fn load_image(path: &Path) -> Result<Frame, String> {
    let img = image::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    // Keep textures within sane GPU limits.
    let img = if img.width() > 4096 || img.height() > 4096 {
        img.resize(4096, 4096, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let rgba = img.to_rgba8();
    Ok(Frame {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    })
}

pub fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

struct Shared {
    latest: Mutex<Option<Vec<u8>>>,
    frames: AtomicU64,
    stop: AtomicBool,
    error: Mutex<Option<String>>,
}

/// A live ffmpeg-decoded capture device (webcam, capture card, or on macOS a screen).
pub struct Stream {
    width: u32,
    height: u32,
    shared: Arc<Shared>,
    child: Option<Child>,
    thread: Option<JoinHandle<()>>,
    seen: u64,
}

impl Stream {
    /// Open a capture device by index (webcam, capture card, or on macOS a screen).
    pub fn camera(index: u32, width: u32, height: u32) -> Result<Self, String> {
        let mut args: Vec<String> = Vec::new();
        if cfg!(target_os = "macos") {
            args.extend(
                ["-f", "avfoundation", "-framerate", "30", "-pixel_format", "uyvy422", "-i"]
                    .map(String::from),
            );
            args.push(format!("{index}:none"));
        } else if cfg!(target_os = "windows") {
            // On Windows ffmpeg needs a device *name*; index-based selection is not supported.
            return Err("camera input on Windows: use `ffmpeg -list_devices true -f dshow -i dummy` and open via a file/URL instead".into());
        } else {
            args.extend(["-f", "v4l2", "-i"].map(String::from));
            args.push(format!("/dev/video{index}"));
        }
        args.extend(output_args(width, height));
        Self::spawn(args, width, height)
    }

    fn spawn(args: Vec<String>, width: u32, height: u32) -> Result<Self, String> {
        let mut child = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-nostdin"])
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("failed to start ffmpeg (is it installed and on PATH?): {e}"))?;

        let mut stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        let shared = Arc::new(Shared {
            latest: Mutex::new(None),
            frames: AtomicU64::new(0),
            stop: AtomicBool::new(false),
            error: Mutex::new(None),
        });

        // Collect stderr so we can show ffmpeg errors in the UI.
        let err_shared = shared.clone();
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = stderr.read_to_string(&mut s);
            let s = s.trim();
            if !s.is_empty() {
                *err_shared.error.lock().unwrap() = Some(s.lines().last().unwrap_or(s).to_string());
            }
        });

        let frame_len = (width * height * 4) as usize;
        let t_shared = shared.clone();
        let thread = std::thread::spawn(move || {
            let mut buf = vec![0u8; frame_len];
            while !t_shared.stop.load(Ordering::Relaxed) {
                if stdout.read_exact(&mut buf).is_err() {
                    break;
                }
                // Hand over the frame, recycling the previous buffer if the UI hasn't taken it.
                let mut slot = t_shared.latest.lock().unwrap();
                let next = slot.take().unwrap_or_else(|| vec![0u8; frame_len]);
                *slot = Some(std::mem::replace(&mut buf, next));
                drop(slot);
                t_shared.frames.fetch_add(1, Ordering::Release);
            }
        });

        Ok(Self {
            width,
            height,
            shared,
            child: Some(child),
            thread: Some(thread),
            seen: 0,
        })
    }

    pub fn error(&self) -> Option<String> {
        self.shared.error.lock().unwrap().clone()
    }

    /// Returns a new frame if one arrived since the last call.
    pub fn poll(&mut self) -> Option<Frame> {
        let n = self.shared.frames.load(Ordering::Acquire);
        if n == self.seen {
            return None;
        }
        self.seen = n;
        let rgba = self.shared.latest.lock().unwrap().take()?;
        Some(Frame {
            width: self.width,
            height: self.height,
            rgba,
        })
    }
}

/// Scale-to-cover + crop to the engine resolution, raw RGBA on stdout.
fn output_args(width: u32, height: u32) -> Vec<String> {
    vec![
        "-an".into(),
        "-vf".into(),
        format!("scale={width}:{height}:force_original_aspect_ratio=increase,crop={width}:{height}"),
        // Emit each decoded/captured frame exactly once. Without this, rawvideo output is
        // constant-frame-rate, and capture devices often report a bogus rate (avfoundation
        // says 1,000,000 fps) so ffmpeg floods the pipe with duplicates of the first frame:
        // the camera looks frozen.
        "-fps_mode".into(),
        "passthrough".into(),
        "-pix_fmt".into(),
        "rgba".into(),
        "-f".into(),
        "rawvideo".into(),
        "pipe:1".into(),
    ]
}

impl Drop for Stream {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

//! Frame-exact video recording of the output.
//!
//! Every 60Hz tick, the finished output frame is copied into one of a few GPU staging
//! buffers. Buffers are mapped asynchronously and drained in order on later ticks, so the
//! render loop never waits on the GPU. Raw RGBA frames go to a writer thread that pipes
//! them into an `ffmpeg` encoder.

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::{SyncSender, sync_channel};
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

struct InFlight {
    buffer: wgpu::Buffer,
    state: Arc<AtomicU8>,
}

pub struct Recorder {
    pub path: PathBuf,
    /// Frames handed to the encoder so far.
    pub frames: u64,
    pub encoder: &'static str,
    free: Vec<wgpu::Buffer>,
    in_flight: VecDeque<InFlight>,
    tx: Option<SyncSender<Vec<u8>>>,
    writer: Option<JoinHandle<Result<(), String>>>,
}

/// A recording that has stopped capturing and is waiting for ffmpeg to finish writing.
pub struct Finishing {
    pub path: PathBuf,
    pub frames: u64,
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
    pub fn start(device: &wgpu::Device, path: PathBuf) -> Result<Self, String> {
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

        let (tx, rx) = sync_channel::<Vec<u8>>(WRITE_QUEUE);
        let writer = std::thread::spawn(move || -> Result<(), String> {
            let mut stdin = child.stdin.take().expect("piped stdin");
            let mut write_err = None;
            for frame in rx {
                if let Err(e) = stdin.write_all(&frame) {
                    write_err = Some(e);
                    break;
                }
            }
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
            frames: 0,
            encoder,
            free,
            in_flight: VecDeque::new(),
            tx: Some(tx),
            writer: Some(writer),
        })
    }

    /// Queue a copy of `texture` (the output frame) for encoding.
    pub fn capture(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Result<(), String> {
        self.drain(device, false)?;
        if self.free.is_empty() {
            // GPU is behind: wait for the oldest copy rather than drop a frame.
            self.drain(device, true)?;
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
        self.in_flight.push_back(InFlight { buffer, state });
        Ok(())
    }

    /// Hand finished copies to the writer, oldest first. With `wait`, block until at least
    /// the oldest one is done.
    fn drain(&mut self, device: &wgpu::Device, wait: bool) -> Result<(), String> {
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
                    self.send(data)?;
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

    fn send(&mut self, frame: Vec<u8>) -> Result<(), String> {
        let tx = self.tx.as_ref().ok_or("recorder already stopped")?;
        // Blocks if ffmpeg falls behind, so the file never skips frames.
        tx.send(frame).map_err(|_| "ffmpeg stopped accepting frames".to_string())?;
        self.frames += 1;
        Ok(())
    }

    /// Flush the remaining frames and let ffmpeg finish in the background.
    pub fn stop(mut self, device: &wgpu::Device) -> Finishing {
        while !self.in_flight.is_empty() {
            if self.drain(device, true).is_err() {
                break;
            }
        }
        self.tx = None;
        Finishing {
            path: self.path.clone(),
            frames: self.frames,
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

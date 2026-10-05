//! Clips: media in a grid cell plus its transport (speed, loop mode, BPM sync, fit).

use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::model::ModelRef;
use crate::modulation::Clock;
use crate::param::{Param, Params, Spec};
use crate::source::{self, Frame, Stream};
use crate::shader::{CustomShader, Role};
use crate::video::VideoMedia;

pub fn next_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LoopMode {
    Loop,
    Bounce,
    /// Jump to a random spot on every beat, play forward in between.
    Random,
    /// Play to the end, then the layer goes empty.
    PlayOnce,
    /// Play to the end and hold the last frame.
    PlayOnceHold,
}

impl LoopMode {
    pub const ALL: [LoopMode; 5] = [
        LoopMode::Loop,
        LoopMode::Bounce,
        LoopMode::Random,
        LoopMode::PlayOnce,
        LoopMode::PlayOnceHold,
    ];
    pub fn name(self) -> &'static str {
        match self {
            LoopMode::Loop => "Loop",
            LoopMode::Bounce => "Bounce",
            LoopMode::Random => "Random",
            LoopMode::PlayOnce => "Play once",
            LoopMode::PlayOnceHold => "Play once & hold",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    Forward,
    Reverse,
    Paused,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sync {
    /// Native frame rate × speed.
    Timeline,
    /// Stretch the clip to a whole number of beats at the current BPM.
    Bpm,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fit {
    /// Fill the frame, cropping.
    Fill,
    /// Show the whole image, letterboxed.
    Contain,
    /// Distort to fill the frame.
    Stretch,
}

impl Fit {
    pub const ALL: [Fit; 3] = [Fit::Fill, Fit::Contain, Fit::Stretch];
    pub fn name(self) -> &'static str {
        match self {
            Fit::Fill => "Fill",
            Fit::Contain => "Fit",
            Fit::Stretch => "Stretch",
        }
    }
}

/// Momentary playback overrides from punch-in FX. Reset every tick by the punch engine;
/// they never change the clip's own settings.
#[derive(Clone, Copy, Debug)]
pub struct ClipPerf {
    /// Speed multiplier (tape stop ramps this to 0).
    pub speed: f64,
    pub reverse: bool,
    /// Beat-repeat: loop `beats` beats starting at frame `anchor`, phase-locked to `start_beat`.
    pub roll: Option<Roll>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Roll {
    pub anchor: f64,
    pub start_beat: f64,
    pub beats: f64,
}

impl Default for ClipPerf {
    fn default() -> Self {
        Self {
            speed: 1.0,
            reverse: false,
            roll: None,
        }
    }
}

pub const PATTERNS: &[&str] = &["Bars", "Rings", "Plasma", "Checker", "Dot", "Noise", "Solid"];

pub struct Generator {
    pub pattern: Param,
    pub freq: Param,
    pub speed: Param,
    pub hue: Param,
}

impl Generator {
    pub fn new(pattern: usize) -> Self {
        Self {
            pattern: Param::with(Spec::choice("pattern", PATTERNS, 0), pattern as f32),
            freq: Param::new(Spec::new("frequency", 0.5, 40.0, 6.0).log()),
            speed: Param::new(Spec::new("speed", 0.0, 4.0, 0.5)),
            hue: Param::new(Spec::new("hue", 0.0, 1.0, 0.0)),
        }
    }
}

/// A 3D model clip's controls (`meshes::clip_draw` reads them by position).
pub const MODEL_SPECS: &[Spec] = &[
    Spec::choice("material", crate::meshes::MATERIALS, 0),
    Spec::new("hue", 0.0, 1.0, 0.0),
    Spec::new("size", 0.1, 3.0, 0.9),
    Spec::new("rotate x °", -180.0, 180.0, 15.0),
    Spec::new("rotate y °", -180.0, 180.0, 0.0),
    Spec::new("rotate z °", -180.0, 180.0, 0.0),
    Spec::new("spin x (turns/bar)", -2.0, 2.0, 0.0),
    Spec::new("spin y (turns/bar)", -2.0, 2.0, 0.125),
    Spec::new("spin z (turns/bar)", -2.0, 2.0, 0.0),
    Spec::new("x", -1.0, 1.0, 0.0),
    Spec::new("y", -1.0, 1.0, 0.0),
    Spec::new("field of view °", 10.0, 120.0, 40.0),
    Spec::new("lighting", 0.0, 1.0, 0.85),
    Spec::new("wire", 0.0, 1.0, 0.0),
    Spec::new("wire width (px)", 0.5, 6.0, 1.2),
    Spec::choice("shading", &["Smooth", "Flat"], 0),
    Spec::new("explode", 0.0, 2.0, 0.0),
    Spec::new("twist", -2.0, 2.0, 0.0),
    Spec::new("wobble", 0.0, 1.0, 0.0),
    Spec::choice("background", &["Transparent", "Black"], 0),
];

/// A 3D model playing as a clip.
pub struct ModelClip {
    pub model: ModelRef,
    pub params: Vec<Param>,
}

impl ModelClip {
    pub fn new(model: ModelRef) -> Self {
        Self { model, params: MODEL_SPECS.iter().map(|s| Param::new(*s)).collect() }
    }
}

pub enum Media {
    Video(VideoMedia),
    Image { frame: Frame, uploaded: bool },
    Camera { index: u32, stream: Stream },
    Generator(Box<Generator>),
    /// User GLSL (Shadertoy style).
    Shader(Box<CustomShader>),
    /// A 3D model, rendered with its own materials.
    Model(Box<ModelClip>),
}

pub struct Clip {
    pub id: u64,
    pub name: String,
    pub media: Media,
    pub speed: Param,
    /// Clip length in beats when BPM-synced.
    pub beats: Param,
    pub loop_mode: LoopMode,
    pub direction: Direction,
    pub sync: Sync,
    pub fit: Fit,
    /// Playhead in frames.
    pub position: f64,
    /// +1 / -1 while bouncing.
    bounce: f64,
    pub finished: bool,
    last_random_beat: i64,
    /// Frame index last handed to the GPU (or None if it needs an upload).
    pub shown_frame: Option<usize>,
    pub thumbnail: Option<eframe::egui::TextureHandle>,
    pub perf: ClipPerf,
}

impl Clip {
    fn new(name: String, media: Media) -> Self {
        Self {
            id: next_id(),
            name,
            media,
            speed: Param::new(Spec::new("speed", 0.05, 4.0, 1.0).log()),
            beats: Param::new(Spec::new("length (beats)", 1.0, 64.0, 8.0).int()),
            loop_mode: LoopMode::Loop,
            direction: Direction::Forward,
            sync: Sync::Timeline,
            fit: Fit::Fill,
            position: 0.0,
            bounce: 1.0,
            finished: false,
            last_random_beat: i64::MIN,
            shown_frame: None,
            thumbnail: None,
            perf: ClipPerf::default(),
        }
    }

    pub fn open(path: &Path, width: u32, height: u32) -> Result<Self, String> {
        let name = source::file_name(path);
        let stem = Path::new(&name)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or(name.clone());
        let media = if source::is_shader(path) {
            Media::Shader(Box::new(CustomShader::from_file(path, Role::Source)?))
        } else if crate::model::formats::is_model(path) {
            let model = std::sync::Arc::new(crate::model::formats::load(path)?);
            Media::Model(Box::new(ModelClip::new(ModelRef { key: path.display().to_string(), model })))
        } else if source::is_video(path) {
            Media::Video(VideoMedia::import(path, width, height)?)
        } else {
            Media::Image {
                frame: source::load_image(path)?,
                uploaded: false,
            }
        };
        Ok(Self::new(stem, media))
    }

    pub fn camera(index: u32, width: u32, height: u32) -> Result<Self, String> {
        let stream = Stream::camera(index, width, height)?;
        Ok(Self::new(format!("Camera {index}"), Media::Camera { index, stream }))
    }

    pub fn shader(name: &str, code: &str) -> Self {
        Self::from_shader(CustomShader::new(name, code, Role::Source))
    }

    pub fn from_shader(shader: CustomShader) -> Self {
        Self::new(shader.name.clone(), Media::Shader(Box::new(shader)))
    }

    pub fn model(model: ModelRef) -> Self {
        Self::new(model.name().to_string(), Media::Model(Box::new(ModelClip::new(model))))
    }

    pub fn generator(pattern: usize) -> Self {
        Self::new(PATTERNS[pattern].to_string(), Media::Generator(Box::new(Generator::new(pattern))))
    }

    /// Clip has a timeline (loop modes, speed etc. apply).
    pub fn is_timeline(&self) -> bool {
        matches!(self.media, Media::Video(_))
    }

    pub fn length(&self) -> usize {
        match &self.media {
            Media::Video(v) => v.length(),
            _ => 1,
        }
    }

    pub fn fps(&self) -> f32 {
        match &self.media {
            Media::Video(v) => v.fps,
            _ => 30.0,
        }
    }

    pub fn duration_secs(&self) -> f32 {
        self.length() as f32 / self.fps()
    }

    pub fn error(&self) -> Option<String> {
        match &self.media {
            Media::Video(v) => v.error(),
            Media::Camera { stream, .. } => stream.error(),
            _ => None,
        }
    }

    /// Start from the top (on launch).
    pub fn restart(&mut self) {
        self.finished = false;
        self.bounce = 1.0;
        self.last_random_beat = i64::MIN;
        self.position = if self.direction == Direction::Reverse {
            (self.length() - 1) as f64
        } else {
            0.0
        };
        self.shown_frame = None;
        if let Media::Image { uploaded, .. } = &mut self.media {
            *uploaded = false;
        }
    }

    pub fn tick(&mut self, dt: f64, clock: Clock, playing: bool) {
        self.speed.tick(clock);
        self.beats.tick(clock);
        match &mut self.media {
            Media::Generator(g) => {
                for p in [&mut g.pattern, &mut g.freq, &mut g.speed, &mut g.hue] {
                    p.tick(clock);
                }
            }
            Media::Shader(s) => s.tick(clock),
            Media::Model(m) => {
                for p in &mut m.params {
                    p.tick(clock);
                }
            }
            _ => {}
        }
        if !self.is_timeline() || !playing || self.finished {
            return;
        }
        let len = self.length() as f64;
        let rate = self.rate(clock);
        if let Some(r) = self.perf.roll {
            // Beat repeat: the playhead is a function of the beat clock.
            let frames_per_beat = rate * 60.0 / clock.bpm.max(1.0) as f64;
            let offset = (clock.beat - r.start_beat).rem_euclid(r.beats.max(1e-3)) * frames_per_beat;
            self.position = (r.anchor + offset).rem_euclid(len.max(1.0));
            return;
        }
        let mut dir = match self.direction {
            Direction::Forward => 1.0,
            Direction::Reverse => -1.0,
            Direction::Paused => return,
        };
        if self.perf.reverse {
            dir = -dir;
        }
        if self.loop_mode == LoopMode::Random {
            let beat = clock.beat.floor() as i64;
            if beat != self.last_random_beat {
                self.last_random_beat = beat;
                self.position = random_frame(self.id, beat, len);
            }
        }
        let step = dt * rate * dir;
        let (pos, bounce, finished) = advance(self.position, len, step, self.loop_mode, self.bounce);
        self.position = pos;
        self.bounce = bounce;
        self.finished = finished;
    }

    /// Playback rate in frames per second (speed, BPM sync and punch overrides included).
    pub fn rate(&self, clock: Clock) -> f64 {
        let len = self.length() as f64;
        let base = match self.sync {
            Sync::Timeline => self.fps() as f64,
            // `len` frames over `beats` beats at the current tempo.
            Sync::Bpm => len / self.beats.get().max(1.0) as f64 * clock.bpm as f64 / 60.0,
        };
        base * self.speed.get() as f64 * self.perf.speed
    }

    /// Whether the clip should currently be drawn.
    pub fn visible(&self) -> bool {
        !(self.finished && self.loop_mode == LoopMode::PlayOnce)
    }

    /// A new frame to upload to the GPU, if any.
    pub fn poll_frame(&mut self) -> Option<Frame> {
        match &mut self.media {
            Media::Video(v) => {
                let want = (self.position.floor().max(0.0) as usize).min(v.length() - 1);
                v.request(want);
                let (idx, frame) = v.take_decoded()?;
                self.shown_frame = Some(idx);
                Some(frame)
            }
            Media::Image { frame, uploaded } => {
                if *uploaded {
                    return None;
                }
                *uploaded = true;
                Some(Frame {
                    width: frame.width,
                    height: frame.height,
                    rgba: frame.rgba.clone(),
                })
            }
            Media::Camera { stream, .. } => stream.poll(),
            Media::Generator(_) | Media::Shader(_) | Media::Model(_) => None,
        }
    }

    /// The GPU texture was (re)created: send the current frame again.
    pub fn invalidate_upload(&mut self) {
        if let Media::Image { uploaded, .. } = &mut self.media {
            *uploaded = false;
        }
    }
}

impl Params for Clip {
    fn visit_params(&mut self, f: &mut dyn FnMut(&str, &mut Param)) {
        f("speed", &mut self.speed);
        f("length (beats)", &mut self.beats);
        match &mut self.media {
            Media::Generator(g) => {
                f("pattern", &mut g.pattern);
                f("frequency", &mut g.freq);
                f("pattern speed", &mut g.speed);
                f("hue", &mut g.hue);
            }
            Media::Shader(s) => {
                for p in &mut s.params {
                    let label = p.spec.label;
                    f(label, p);
                }
                f("alpha", &mut s.alpha);
            }
            Media::Model(m) => {
                for p in &mut m.params {
                    let label = p.spec.label;
                    f(label, p);
                }
            }
            _ => {}
        }
    }
}

fn random_frame(seed: u64, beat: i64, len: f64) -> f64 {
    let mut z = seed.wrapping_mul(0x9e3779b97f4a7c15) ^ (beat as u64).wrapping_mul(0xbf58476d1ce4e5b9);
    z ^= z >> 31;
    z = z.wrapping_mul(0x94d049bb133111eb);
    z ^= z >> 29;
    ((z >> 11) as f64 / (1u64 << 53) as f64 * len).floor()
}

/// Move a playhead by `step` frames within `[0, len)`.
/// Returns `(position, bounce direction, finished)`.
pub fn advance(pos: f64, len: f64, step: f64, mode: LoopMode, bounce: f64) -> (f64, f64, bool) {
    let last = (len - 1.0).max(0.0);
    match mode {
        LoopMode::Loop | LoopMode::Random => ((pos + step).rem_euclid(len.max(1.0)), bounce, false),
        LoopMode::Bounce => {
            if last == 0.0 {
                return (0.0, bounce, false);
            }
            let mut p = pos + step * bounce;
            let mut b = bounce;
            // Reflect off both ends (loop handles very large steps).
            for _ in 0..8 {
                if p > last {
                    p = 2.0 * last - p;
                    b = -b;
                } else if p < 0.0 {
                    p = -p;
                    b = -b;
                } else {
                    break;
                }
            }
            (p.clamp(0.0, last), b, false)
        }
        LoopMode::PlayOnce | LoopMode::PlayOnceHold => {
            let p = pos + step;
            if p > last {
                (last, bounce, true)
            } else if p < 0.0 {
                (0.0, bounce, true)
            } else {
                (p, bounce, false)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loop_wraps_both_ways() {
        assert_eq!(advance(9.5, 10.0, 1.0, LoopMode::Loop, 1.0).0, 0.5);
        assert_eq!(advance(0.5, 10.0, -1.0, LoopMode::Loop, 1.0).0, 9.5);
    }

    #[test]
    fn bounce_reflects_and_flips() {
        let (p, b, done) = advance(8.5, 10.0, 1.0, LoopMode::Bounce, 1.0);
        assert_eq!((p, b, done), (8.5, -1.0, false)); // 9.5 reflects off 9 -> 8.5
        let (p, b, _) = advance(0.5, 10.0, 1.0, LoopMode::Bounce, -1.0);
        assert_eq!((p, b), (0.5, 1.0));
    }

    #[test]
    fn play_once_finishes() {
        assert_eq!(advance(8.5, 10.0, 1.0, LoopMode::PlayOnce, 1.0), (9.0, 1.0, true));
        assert_eq!(advance(0.5, 10.0, -1.0, LoopMode::PlayOnceHold, 1.0), (0.0, 1.0, true));
        assert_eq!(advance(3.0, 10.0, 1.0, LoopMode::PlayOnce, 1.0), (4.0, 1.0, false));
    }

    #[test]
    fn random_frames_in_range_and_vary() {
        let frames: Vec<f64> = (0..50).map(|b| random_frame(7, b, 120.0)).collect();
        assert!(frames.iter().all(|f| (0.0..120.0).contains(f)));
        let distinct = frames.iter().map(|f| *f as i64).collect::<std::collections::HashSet<_>>();
        assert!(distinct.len() > 20);
    }
}

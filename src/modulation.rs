//! Per-parameter automation: a signal generator (or drawn envelope, or the audio input) that
//! sweeps a slider around its hand-set value.

use crate::audio::{Band, Levels};

/// Global clock the modulators run on.
#[derive(Clone, Copy, Debug, Default)]
pub struct Clock {
    /// Beats elapsed at the current BPM.
    pub beat: f64,
    /// Seconds elapsed.
    pub time: f64,
    pub bpm: f32,
    /// The audio input's analysis this tick (silence when there's none).
    pub audio: Levels,
}

impl Clock {
    pub fn new(beat: f64, time: f64, bpm: f32) -> Self {
        Self { beat, time, bpm, audio: Levels::default() }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shape {
    Sine,
    Triangle,
    SawUp,
    SawDown,
    Square,
    SampleHold,
    SmoothRandom,
    Envelope,
    /// Follows a band of the audio input instead of a cycle.
    Audio,
}

impl Shape {
    pub const ALL: [Shape; 9] = [
        Shape::Sine,
        Shape::Triangle,
        Shape::SawUp,
        Shape::SawDown,
        Shape::Square,
        Shape::SampleHold,
        Shape::SmoothRandom,
        Shape::Envelope,
        Shape::Audio,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Shape::Sine => "Sine",
            Shape::Triangle => "Tri",
            Shape::SawUp => "Saw ↗",
            Shape::SawDown => "Saw ↘",
            Shape::Square => "Square",
            Shape::SampleHold => "S&H",
            Shape::SmoothRandom => "Drift",
            Shape::Envelope => "Envelope",
            Shape::Audio => "Audio",
        }
    }
    pub fn is_random(self) -> bool {
        matches!(self, Shape::SampleHold | Shape::SmoothRandom)
    }
}

/// How the 0..1 signal is applied around the slider's own value.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Polarity {
    /// Swing above and below the slider value.
    Bipolar,
    /// Only push the value up.
    Up,
    /// Only pull the value down.
    Down,
}

impl Polarity {
    pub const ALL: [Polarity; 3] = [Polarity::Bipolar, Polarity::Up, Polarity::Down];
    pub fn name(self) -> &'static str {
        match self {
            Polarity::Bipolar => "± around",
            Polarity::Up => "+ above",
            Polarity::Down => "− below",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Rate {
    /// Cycle length in beats (tempo-synced).
    Beats(f32),
    /// Free-running cycles per second.
    Hz(f32),
}

pub const BEAT_CHOICES: [f32; 10] = [0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0];

#[derive(Clone, Debug)]
pub struct Modulator {
    pub enabled: bool,
    pub shape: Shape,
    pub rate: Rate,
    /// Fraction of the slider's full range covered by the sweep.
    pub depth: f32,
    pub polarity: Polarity,
    /// Phase offset in cycles.
    pub phase: f32,
    /// Duty cycle for the square wave.
    pub width: f32,
    /// Breakpoints (x = position in cycle 0..1, y = level 0..1), sorted by x. Periodic.
    pub points: Vec<[f32; 2]>,
    /// Makes random shapes differ between parameters.
    pub seed: u64,
    /// What the Audio shape follows.
    pub band: Band,
}

impl Modulator {
    pub fn new(seed: u64) -> Self {
        Self {
            enabled: true,
            shape: Shape::Sine,
            rate: Rate::Beats(8.0),
            depth: 0.25,
            polarity: Polarity::Bipolar,
            phase: 0.0,
            width: 0.5,
            // A "pluck": fast attack on the beat, then decay.
            points: vec![[0.0, 0.0], [0.08, 1.0], [0.6, 0.15]],
            seed,
            band: Band::Bass,
        }
    }

    pub fn lfo(seed: u64, shape: Shape, beats: f32, depth: f32) -> Self {
        Self {
            shape,
            rate: Rate::Beats(beats),
            depth,
            ..Self::new(seed)
        }
    }

    /// Continuous position in cycles (integer part = cycle number).
    pub fn position(&self, clock: Clock) -> f64 {
        let cycles = match self.rate {
            Rate::Beats(b) => clock.beat / b.max(1.0 / 64.0) as f64,
            Rate::Hz(hz) => clock.time * hz as f64,
        };
        cycles + self.phase as f64
    }

    /// The raw signal (0..1) at a position in cycles.
    pub fn signal_at(&self, pos: f64) -> f32 {
        let x = pos.rem_euclid(1.0) as f32;
        let cycle = pos.floor() as i64;
        match self.shape {
            Shape::Sine => 0.5 + 0.5 * (x * std::f32::consts::TAU).sin(),
            Shape::Triangle => 1.0 - (2.0 * x - 1.0).abs(),
            Shape::SawUp => x,
            Shape::SawDown => 1.0 - x,
            Shape::Square => {
                if x < self.width {
                    1.0
                } else {
                    0.0
                }
            }
            Shape::SampleHold => self.hash(cycle),
            Shape::SmoothRandom => {
                let t = x * x * (3.0 - 2.0 * x);
                self.hash(cycle) * (1.0 - t) + self.hash(cycle + 1) * t
            }
            Shape::Envelope => envelope(&self.points, x),
            // Not periodic: see `signal`.
            Shape::Audio => 0.0,
        }
    }

    pub fn signal(&self, clock: Clock) -> f32 {
        match self.shape {
            Shape::Audio => clock.audio.get(self.band),
            _ => self.signal_at(self.position(clock)),
        }
    }

    /// Apply to a slider value with the given range; the result stays in range.
    pub fn apply(&self, base: f32, min: f32, max: f32, signal: f32) -> f32 {
        let span = (max - min) * self.depth;
        let v = match self.polarity {
            Polarity::Bipolar => base + span * (signal - 0.5),
            Polarity::Up => base + span * signal,
            Polarity::Down => base - span * signal,
        };
        v.clamp(min, max)
    }

    fn hash(&self, n: i64) -> f32 {
        // splitmix64
        let mut z = (n as u64).wrapping_add(self.seed).wrapping_add(0x9e3779b97f4a7c15);
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb133111eb);
        z ^= z >> 31;
        (z >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// Linear interpolation through periodic breakpoints.
pub fn envelope(points: &[[f32; 2]], x: f32) -> f32 {
    match points.len() {
        0 => return 0.0,
        1 => return points[0][1],
        _ => {}
    }
    let n = points.len();
    // Segment that wraps from the last point to the first point of the next cycle.
    let (a, b) = match points.iter().position(|p| p[0] > x) {
        Some(0) => ([points[n - 1][0] - 1.0, points[n - 1][1]], points[0]),
        Some(i) => (points[i - 1], points[i]),
        None => (points[n - 1], [points[0][0] + 1.0, points[0][1]]),
    };
    let w = b[0] - a[0];
    if w <= 1e-6 {
        return b[1];
    }
    a[1] + (b[1] - a[1]) * ((x - a[0]) / w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_wraps_and_interpolates() {
        let pts = [[0.25, 0.0], [0.75, 1.0]];
        assert!((envelope(&pts, 0.5) - 0.5).abs() < 1e-5);
        assert!((envelope(&pts, 0.25) - 0.0).abs() < 1e-5);
        // Wrap segment: 0.75 -> 1.25 goes 1.0 -> 0.0, so x = 1.0 (== 0.0) is halfway.
        assert!((envelope(&pts, 0.0) - 0.5).abs() < 1e-5);
        assert!((envelope(&pts, 0.95) - 0.6).abs() < 1e-5);
    }

    #[test]
    fn apply_respects_polarity_and_range() {
        let mut m = Modulator::new(1);
        m.depth = 0.5;
        assert_eq!(m.apply(0.5, 0.0, 1.0, 0.5), 0.5);
        assert_eq!(m.apply(0.5, 0.0, 1.0, 1.0), 0.75);
        m.polarity = Polarity::Down;
        assert_eq!(m.apply(0.5, 0.0, 1.0, 0.5), 0.25);
        m.polarity = Polarity::Up;
        assert_eq!(m.apply(0.9, 0.0, 1.0, 1.0), 1.0);
    }

    #[test]
    fn shapes_stay_in_unit_range() {
        for shape in Shape::ALL {
            let mut m = Modulator::new(2);
            m.shape = shape;
            for i in 0..1000 {
                let s = m.signal_at(i as f64 * 0.0137);
                assert!((0.0..=1.0).contains(&s), "{shape:?} gave {s}");
            }
        }
    }
}

//! An automatable parameter: a hand-set value, its range, an optional modulator, and the
//! live value the renderer actually uses this frame.

use std::sync::atomic::{AtomicU64, Ordering};

use crate::modulation::{Clock, Modulator};

#[derive(Clone, Copy, Debug)]
pub struct Spec {
    pub label: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub log: bool,
    /// Whole numbers only.
    pub int: bool,
    /// Named options; the value is the index. Implies `int`.
    pub choices: &'static [&'static str],
}

impl Spec {
    pub const fn new(label: &'static str, min: f32, max: f32, default: f32) -> Self {
        Self {
            label,
            min,
            max,
            default,
            log: false,
            int: false,
            choices: &[],
        }
    }
    pub const fn log(mut self) -> Self {
        self.log = true;
        self
    }
    pub const fn int(mut self) -> Self {
        self.int = true;
        self
    }
    pub const fn choice(label: &'static str, choices: &'static [&'static str], default: usize) -> Self {
        Self {
            label,
            min: 0.0,
            max: (choices.len() - 1) as f32,
            default: default as f32,
            log: false,
            int: true,
            choices,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Param {
    pub spec: Spec,
    pub value: f32,
    pub modulator: Option<Modulator>,
    /// Value after modulation, updated once per tick.
    pub live: f32,
    /// Seeds random modulator shapes so every parameter wanders differently.
    pub seed: u64,
}

fn next_seed() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed).wrapping_mul(0x9e3779b97f4a7c15)
}

impl Param {
    pub fn new(spec: Spec) -> Self {
        Self {
            spec,
            value: spec.default,
            modulator: None,
            live: spec.default,
            seed: next_seed(),
        }
    }

    pub fn with(spec: Spec, value: f32) -> Self {
        let mut p = Self::new(spec);
        p.set(value);
        p
    }

    pub fn set(&mut self, v: f32) {
        self.value = v.clamp(self.spec.min, self.spec.max);
        self.live = self.value;
    }

    pub fn tick(&mut self, clock: Clock) {
        let mut v = self.value;
        if let Some(m) = &self.modulator
            && m.enabled
            && m.depth > 0.0
        {
            v = m.apply(v, self.spec.min, self.spec.max, m.signal(clock));
        }
        if self.spec.int {
            v = v.round();
        }
        self.live = v;
    }

    pub fn get(&self) -> f32 {
        self.live
    }

    pub fn index(&self) -> usize {
        self.live.round().max(0.0) as usize
    }

    pub fn is_automated(&self) -> bool {
        self.modulator.as_ref().is_some_and(|m| m.enabled)
    }

    /// Attach a beat-synced LFO (used by presets).
    pub fn lfo(mut self, shape: crate::modulation::Shape, beats: f32, depth: f32) -> Self {
        self.modulator = Some(Modulator::lfo(self.seed, shape, beats, depth));
        self
    }

    /// Slider position (0..1) of a value, matching egui's linear/log mapping.
    pub fn normalized(&self, v: f32) -> f32 {
        let s = &self.spec;
        if s.log && s.min > 0.0 {
            ((v.max(s.min) / s.min).ln() / (s.max / s.min).ln()).clamp(0.0, 1.0)
        } else {
            ((v - s.min) / (s.max - s.min)).clamp(0.0, 1.0)
        }
    }
}

/// Anything holding parameters can expose them for automation sweeps and the overview list.
pub trait Params {
    fn visit_params(&mut self, f: &mut dyn FnMut(&str, &mut Param));
}

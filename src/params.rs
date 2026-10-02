//! All live-tweakable state for the mixer + feedback loop, plus presets and LFOs.

use std::collections::BTreeMap;

use crate::engine::MAX_DELAY;
use crate::modulation::{Clock, Modulator, Shape};

/// What a deck shows when it has no file loaded (or the user picks a generator).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pattern {
    Bars,
    Rings,
    Plasma,
    Checker,
    Dot,
}

impl Pattern {
    pub const ALL: [Pattern; 5] = [
        Pattern::Bars,
        Pattern::Rings,
        Pattern::Plasma,
        Pattern::Checker,
        Pattern::Dot,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Pattern::Bars => "Bars",
            Pattern::Rings => "Rings",
            Pattern::Plasma => "Plasma",
            Pattern::Checker => "Checker",
            Pattern::Dot => "Dot",
        }
    }
}

#[derive(Clone, Debug)]
pub struct DeckParams {
    /// Use the procedural oscillator instead of the loaded media.
    pub use_pattern: bool,
    pub pattern: Pattern,
    pub osc_freq: f32,
    pub osc_speed: f32,
    pub gain: f32,
    pub hue: f32,
    pub invert: bool,
    /// Size of the deck's box relative to the screen (1 = full screen).
    pub scale: f32,
    /// Box center offset in screen fractions; +x right, +y up.
    pub pos_x: f32,
    pub pos_y: f32,
    /// Show the whole image (letterboxed) instead of filling the box and cropping.
    pub fit_whole: bool,
}

impl DeckParams {
    pub fn reset_placement(&mut self) {
        self.scale = 1.0;
        self.pos_x = 0.0;
        self.pos_y = 0.0;
    }

    pub fn new(pattern: Pattern) -> Self {
        Self {
            use_pattern: true,
            pattern,
            osc_freq: 6.0,
            osc_speed: 0.5,
            gain: 1.0,
            hue: 0.0,
            invert: false,
            scale: 1.0,
            pos_x: 0.0,
            pos_y: 0.0,
            fit_whole: false,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BlendMode {
    Crossfade,
    Add,
    Multiply,
    Difference,
    LumaKey,
    Lighten,
}

impl BlendMode {
    pub const ALL: [BlendMode; 6] = [
        BlendMode::Crossfade,
        BlendMode::Add,
        BlendMode::Multiply,
        BlendMode::Difference,
        BlendMode::LumaKey,
        BlendMode::Lighten,
    ];
    pub fn name(self) -> &'static str {
        match self {
            BlendMode::Crossfade => "Crossfade",
            BlendMode::Add => "Add",
            BlendMode::Multiply => "Multiply",
            BlendMode::Difference => "Difference",
            BlendMode::LumaKey => "Luma key (B over A)",
            BlendMode::Lighten => "Lighten",
        }
    }
}

/// How the fed-back image is laid over by fresh input — this is the "keyer" in an
/// analog feedback rig.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InputMode {
    LumaKey,
    Add,
    Lighten,
    Difference,
}

impl InputMode {
    pub const ALL: [InputMode; 4] = [
        InputMode::LumaKey,
        InputMode::Add,
        InputMode::Lighten,
        InputMode::Difference,
    ];
    pub fn name(self) -> &'static str {
        match self {
            InputMode::LumaKey => "Luma key",
            InputMode::Add => "Add",
            InputMode::Lighten => "Lighten",
            InputMode::Difference => "Difference",
        }
    }
}

/// How the N "monitor copies" of the previous frame are merged.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CopyCombine {
    Lighten,
    Add,
    Average,
}

impl CopyCombine {
    pub const ALL: [CopyCombine; 3] = [CopyCombine::Lighten, CopyCombine::Add, CopyCombine::Average];
    pub fn name(self) -> &'static str {
        match self {
            CopyCombine::Lighten => "Lighten",
            CopyCombine::Add => "Add",
            CopyCombine::Average => "Average",
        }
    }
}

/// What happens outside the "monitor bezel" when a copy samples off-screen.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EdgeMode {
    Black,
    Mirror,
    Wrap,
}

impl EdgeMode {
    pub const ALL: [EdgeMode; 3] = [EdgeMode::Black, EdgeMode::Mirror, EdgeMode::Wrap];
    pub fn name(self) -> &'static str {
        match self {
            EdgeMode::Black => "Black (bezel)",
            EdgeMode::Mirror => "Mirror",
            EdgeMode::Wrap => "Wrap / tile",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Symmetry {
    None,
    MirrorX,
    MirrorXY,
    Kaleido,
}

impl Symmetry {
    pub const ALL: [Symmetry; 4] = [Symmetry::None, Symmetry::MirrorX, Symmetry::MirrorXY, Symmetry::Kaleido];
    pub fn name(self) -> &'static str {
        match self {
            Symmetry::None => "None",
            Symmetry::MirrorX => "Mirror X",
            Symmetry::MirrorXY => "Mirror XY",
            Symmetry::Kaleido => "Kaleidoscope",
        }
    }
}

#[derive(Clone, Debug)]
pub struct FxParams {
    // --- feedback loop ("camera pointed at the monitors") ---
    pub feedback: f32,
    pub copies: u32,
    pub zoom: f32,
    pub rotate: f32, // degrees
    pub spread: f32,
    pub twist: f32, // degrees, extra rotation per copy
    pub center_x: f32,
    pub center_y: f32,
    pub combine: CopyCombine,
    pub edge: EdgeMode,
    pub symmetry: Symmetry,
    pub kaleido_segments: u32,
    pub hue_shift: f32, // per pass, 0..1
    pub saturation: f32,
    pub contrast: f32,
    pub blur: f32,
    pub noise: f32,

    // --- input keyer ---
    pub input_mode: InputMode,
    pub input_level: f32,
    pub key_threshold: f32,
    pub key_softness: f32,

    // --- video delay line ---
    /// Frames of delay inside the feedback loop (1 = classic feedback).
    pub loop_delay: u32,
    /// Echo taps: mix in older output frames.
    pub echo_amount: f32,
    pub echo_spacing: u32,
    /// RGB time-split: green/blue channels come from older frames.
    pub chroma_delay: u32,
    pub chroma_amount: f32,

    // --- output stage (not fed back) ---
    pub out_hue: f32,
    pub out_invert: bool,
    pub posterize: u32,
    pub scanlines: f32,
    pub vignette: f32,
    pub brightness: f32,
}

impl Default for FxParams {
    fn default() -> Self {
        Self {
            feedback: 0.92,
            copies: 1,
            zoom: 0.95,
            rotate: 2.0,
            spread: 0.0,
            twist: 0.0,
            center_x: 0.0,
            center_y: 0.0,
            combine: CopyCombine::Lighten,
            edge: EdgeMode::Black,
            symmetry: Symmetry::None,
            kaleido_segments: 6,
            hue_shift: 0.01,
            saturation: 1.05,
            contrast: 1.0,
            blur: 0.0,
            noise: 0.0,
            input_mode: InputMode::LumaKey,
            input_level: 1.0,
            key_threshold: 0.15,
            key_softness: 0.1,
            loop_delay: 1,
            echo_amount: 0.0,
            echo_spacing: 8,
            chroma_delay: 0,
            chroma_amount: 0.0,
            out_hue: 0.0,
            out_invert: false,
            posterize: 0,
            scanlines: 0.0,
            vignette: 0.2,
            brightness: 1.0,
        }
    }
}

/// Everything the engine needs for one frame.
#[derive(Clone, Debug)]
pub struct Params {
    pub deck_a: DeckParams,
    pub deck_b: DeckParams,
    pub crossfade: f32,
    pub blend: BlendMode,
    pub fx: FxParams,
    pub bpm: f32,
    /// Automation, keyed by `ParamDef::key`.
    pub mods: BTreeMap<&'static str, Modulator>,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            deck_a: DeckParams::new(Pattern::Dot),
            deck_b: DeckParams::new(Pattern::Bars),
            crossfade: 0.0,
            blend: BlendMode::Crossfade,
            fx: FxParams::default(),
            bpm: 120.0,
            mods: BTreeMap::new(),
        }
    }
}

// ---------------------------------------------------------------- parameter table

#[derive(Clone, Copy)]
pub enum Access {
    F(fn(&mut Params) -> &mut f32),
    U(fn(&mut Params) -> &mut u32),
}

/// A slider: one entry per automatable parameter. The UI and the modulation engine both
/// go through this table, so every slider can be automated.
pub struct ParamDef {
    pub key: &'static str,
    pub label: &'static str,
    pub min: f32,
    pub max: f32,
    pub log: bool,
    pub access: Access,
}

impl ParamDef {
    pub fn get(&self, p: &mut Params) -> f32 {
        match self.access {
            Access::F(f) => *f(p),
            Access::U(f) => *f(p) as f32,
        }
    }
    pub fn set(&self, p: &mut Params, v: f32) {
        match self.access {
            Access::F(f) => *f(p) = v,
            Access::U(f) => *f(p) = v.round().max(0.0) as u32,
        }
    }
    /// Slider position (0..1) of a value, matching egui's linear/log mapping.
    pub fn normalized(&self, v: f32) -> f32 {
        if self.log && self.min > 0.0 {
            ((v.max(self.min) / self.min).ln() / (self.max / self.min).ln()).clamp(0.0, 1.0)
        } else {
            ((v - self.min) / (self.max - self.min)).clamp(0.0, 1.0)
        }
    }
}

macro_rules! def {
    ($key:literal, $label:literal, $min:expr, $max:expr, f32, $($path:tt)+) => {
        def!(@ $key, $label, $min, $max, false, Access::F({ fn get(p: &mut Params) -> &mut f32 { &mut p.$($path)+ } get }))
    };
    ($key:literal, $label:literal, $min:expr, $max:expr, log, $($path:tt)+) => {
        def!(@ $key, $label, $min, $max, true, Access::F({ fn get(p: &mut Params) -> &mut f32 { &mut p.$($path)+ } get }))
    };
    ($key:literal, $label:literal, $min:expr, $max:expr, u32, $($path:tt)+) => {
        def!(@ $key, $label, $min, $max, false, Access::U({ fn get(p: &mut Params) -> &mut u32 { &mut p.$($path)+ } get }))
    };
    (@ $key:literal, $label:literal, $min:expr, $max:expr, $log:expr, $access:expr) => {
        ParamDef { key: $key, label: $label, min: $min as f32, max: $max as f32, log: $log, access: $access }
    };
}

pub static PARAMS: &[ParamDef] = &[
    // decks
    def!("a.osc_freq", "freq", 0.5, 40.0, log, deck_a.osc_freq),
    def!("a.osc_speed", "speed", 0.0, 4.0, f32, deck_a.osc_speed),
    def!("a.gain", "gain", 0.0, 2.0, f32, deck_a.gain),
    def!("a.hue", "hue", 0.0, 1.0, f32, deck_a.hue),
    def!("a.scale", "size", 0.05, 3.0, log, deck_a.scale),
    def!("a.pos_x", "x", -1.0, 1.0, f32, deck_a.pos_x),
    def!("a.pos_y", "y", -1.0, 1.0, f32, deck_a.pos_y),
    def!("b.osc_freq", "freq", 0.5, 40.0, log, deck_b.osc_freq),
    def!("b.osc_speed", "speed", 0.0, 4.0, f32, deck_b.osc_speed),
    def!("b.gain", "gain", 0.0, 2.0, f32, deck_b.gain),
    def!("b.hue", "hue", 0.0, 1.0, f32, deck_b.hue),
    def!("b.scale", "size", 0.05, 3.0, log, deck_b.scale),
    def!("b.pos_x", "x", -1.0, 1.0, f32, deck_b.pos_x),
    def!("b.pos_y", "y", -1.0, 1.0, f32, deck_b.pos_y),
    def!("crossfade", "crossfade", 0.0, 1.0, f32, crossfade),
    // feedback / fractal
    def!("feedback", "feedback", 0.0, 1.2, f32, fx.feedback),
    def!("copies", "copies (monitors)", 1, 8, u32, fx.copies),
    def!("zoom", "copy scale", 0.05, 2.0, f32, fx.zoom),
    def!("rotate", "rotate °", -180.0, 180.0, f32, fx.rotate),
    def!("spread", "spread", 0.0, 1.5, f32, fx.spread),
    def!("twist", "twist ° / copy", -180.0, 180.0, f32, fx.twist),
    def!("center_x", "center x", -0.8, 0.8, f32, fx.center_x),
    def!("center_y", "center y", -0.5, 0.5, f32, fx.center_y),
    def!("kaleido_segments", "segments", 2, 16, u32, fx.kaleido_segments),
    // loop colour
    def!("hue_shift", "hue / pass", -0.1, 0.1, f32, fx.hue_shift),
    def!("saturation", "saturation", 0.0, 2.0, f32, fx.saturation),
    def!("contrast", "contrast", 0.5, 2.0, f32, fx.contrast),
    def!("blur", "blur / soften", 0.0, 1.0, f32, fx.blur),
    def!("noise", "noise", 0.0, 1.0, f32, fx.noise),
    // keyer
    def!("input_level", "input level", 0.0, 1.5, f32, fx.input_level),
    def!("key_threshold", "key threshold", 0.0, 1.0, f32, fx.key_threshold),
    def!("key_softness", "key softness", 0.0, 0.5, f32, fx.key_softness),
    // delay
    def!("loop_delay", "loop delay (frames)", 1, MAX_DELAY, u32, fx.loop_delay),
    def!("echo_amount", "echo", 0.0, 1.0, f32, fx.echo_amount),
    def!("echo_spacing", "echo spacing", 1, MAX_DELAY / 3, u32, fx.echo_spacing),
    def!("chroma_delay", "RGB split (frames)", 0, MAX_DELAY / 2, u32, fx.chroma_delay),
    def!("chroma_amount", "RGB split amount", 0.0, 1.0, f32, fx.chroma_amount),
    // output
    def!("out_hue", "hue", 0.0, 1.0, f32, fx.out_hue),
    def!("brightness", "brightness", 0.0, 2.0, f32, fx.brightness),
    def!("posterize", "posterize", 0, 16, u32, fx.posterize),
    def!("scanlines", "scanlines", 0.0, 1.0, f32, fx.scanlines),
    def!("vignette", "vignette", 0.0, 1.0, f32, fx.vignette),
];

pub fn param_def(key: &str) -> &'static ParamDef {
    PARAMS
        .iter()
        .find(|d| d.key == key)
        .unwrap_or_else(|| panic!("unknown parameter {key}"))
}

/// Deck and mixer automation belongs to the decks, so presets leave it alone.
fn is_deck_key(key: &str) -> bool {
    key.starts_with("a.") || key.starts_with("b.") || key == "crossfade"
}

impl Params {
    /// Returns a copy with all enabled automation applied.
    pub fn modulated(&self, clock: Clock) -> Params {
        let mut p = self.clone();
        for (key, m) in &self.mods {
            if !m.enabled || m.depth == 0.0 {
                continue;
            }
            let def = param_def(key);
            let base = def.get(&mut p);
            def.set(&mut p, m.apply(base, def.min, def.max, m.signal(clock)));
        }
        p
    }
}

/// Built-in starting points. Each preset only rewrites the FX section and its automation,
/// so the loaded decks (and their automation) stay as they are.
pub const PRESET_NAMES: [&str; 9] = [
    "1 Tunnel",
    "2 Sierpinski",
    "3 Mandala",
    "4 Slow echo",
    "5 Time smear",
    "6 Spiral galaxy",
    "7 Hall of mirrors",
    "8 Melt",
    "9 Clean (no fx)",
];

pub fn apply_preset(p: &mut Params, idx: usize) {
    let mut fx = FxParams::default();
    let mut mods: Vec<(&'static str, Shape, f32, f32)> = Vec::new();
    match idx {
        // Classic camera-at-monitor zoom tunnel.
        0 => {
            fx.feedback = 0.95;
            fx.zoom = 0.93;
            fx.rotate = 3.0;
            fx.hue_shift = 0.012;
            mods.push(("rotate", Shape::Sine, 16.0, 0.075));
        }
        // Three half-size copies: the textbook IFS that the analog rig produces with 3 monitors.
        1 => {
            fx.feedback = 1.0;
            fx.copies = 3;
            fx.zoom = 0.5;
            fx.spread = 0.5;
            fx.rotate = 0.0;
            fx.edge = EdgeMode::Black;
            fx.combine = CopyCombine::Lighten;
            fx.hue_shift = 0.02;
            fx.key_threshold = 0.3;
            mods.push(("twist", Shape::Sine, 32.0, 0.075));
        }
        // Many rotating copies + kaleidoscope.
        2 => {
            fx.feedback = 0.97;
            fx.copies = 6;
            fx.zoom = 0.42;
            fx.spread = 0.62;
            fx.rotate = 0.0;
            fx.twist = 30.0;
            fx.symmetry = Symmetry::Kaleido;
            fx.kaleido_segments = 6;
            fx.hue_shift = 0.015;
            mods.push(("rotate", Shape::Triangle, 32.0, 0.125));
            mods.push(("spread", Shape::Sine, 16.0, 0.1));
        }
        // Delay-line trails, little feedback geometry.
        3 => {
            fx.feedback = 0.6;
            fx.zoom = 1.0;
            fx.rotate = 0.0;
            fx.loop_delay = 12;
            fx.echo_amount = 0.5;
            fx.echo_spacing = 10;
            fx.hue_shift = 0.08;
            fx.input_mode = InputMode::Lighten;
        }
        // RGB time split.
        4 => {
            fx.feedback = 0.75;
            fx.zoom = 1.01;
            fx.rotate = 0.0;
            fx.chroma_delay = 6;
            fx.chroma_amount = 1.0;
            fx.hue_shift = 0.0;
            fx.input_mode = InputMode::Add;
            fx.input_level = 0.5;
        }
        // Rotating, slightly offset copies -> spiral arms.
        5 => {
            fx.feedback = 0.98;
            fx.copies = 2;
            fx.zoom = 0.72;
            fx.spread = 0.35;
            fx.rotate = 20.0;
            fx.twist = 0.0;
            fx.hue_shift = 0.006;
            fx.combine = CopyCombine::Lighten;
            mods.push(("rotate", Shape::Sine, 64.0, 0.1));
        }
        // Tiled infinite mirror room.
        6 => {
            fx.feedback = 0.96;
            fx.copies = 4;
            fx.zoom = 0.5;
            fx.spread = 0.7;
            fx.rotate = 45.0;
            fx.edge = EdgeMode::Mirror;
            fx.combine = CopyCombine::Average;
            fx.hue_shift = 0.01;
            fx.contrast = 1.15;
            mods.push(("zoom", Shape::Sine, 16.0, 0.05));
        }
        // Blurry zoom-out melt with delay.
        7 => {
            fx.feedback = 1.0;
            fx.zoom = 1.04;
            fx.rotate = -1.5;
            fx.blur = 0.6;
            fx.loop_delay = 3;
            fx.hue_shift = 0.004;
            fx.saturation = 1.2;
            fx.input_mode = InputMode::Difference;
            fx.input_level = 0.8;
            mods.push(("center_x", Shape::Sine, 8.0, 0.094));
            mods.push(("center_y", Shape::Sine, 12.0, 0.15));
        }
        _ => {
            fx.feedback = 0.0;
            fx.input_mode = InputMode::Add;
            fx.vignette = 0.0;
        }
    }
    p.fx = fx;
    p.mods.retain(|k, _| is_deck_key(k));
    for (key, shape, beats, depth) in mods {
        p.mods.insert(key, Modulator::lfo(key, shape, beats, depth));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn param_keys_are_unique_and_ranges_sane() {
        let mut p = Params::default();
        for (i, d) in PARAMS.iter().enumerate() {
            assert!(PARAMS[i + 1..].iter().all(|o| o.key != d.key), "duplicate key {}", d.key);
            assert!(d.min < d.max, "{}", d.key);
            // Round-trips through the accessor.
            let v = d.get(&mut p);
            d.set(&mut p, v);
            assert_eq!(d.get(&mut p), v);
        }
    }

    #[test]
    fn presets_only_reference_known_params() {
        let mut p = Params::default();
        for i in 0..PRESET_NAMES.len() {
            apply_preset(&mut p, i);
            for k in p.mods.keys() {
                param_def(k);
            }
            let _ = p.modulated(Clock { beat: 3.3, time: 1.0 });
        }
    }
}

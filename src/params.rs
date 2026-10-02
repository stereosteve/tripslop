//! All live-tweakable state for the mixer + feedback loop, plus presets and LFOs.

use std::f32::consts::TAU;

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
}

impl DeckParams {
    pub fn new(pattern: Pattern) -> Self {
        Self {
            use_pattern: true,
            pattern,
            osc_freq: 6.0,
            osc_speed: 0.5,
            gain: 1.0,
            hue: 0.0,
            invert: false,
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LfoTarget {
    Off,
    Zoom,
    Rotate,
    Spread,
    Twist,
    Feedback,
    HueShift,
    Crossfade,
    CenterX,
    CenterY,
    KeyThreshold,
}

impl LfoTarget {
    pub const ALL: [LfoTarget; 11] = [
        LfoTarget::Off,
        LfoTarget::Zoom,
        LfoTarget::Rotate,
        LfoTarget::Spread,
        LfoTarget::Twist,
        LfoTarget::Feedback,
        LfoTarget::HueShift,
        LfoTarget::Crossfade,
        LfoTarget::CenterX,
        LfoTarget::CenterY,
        LfoTarget::KeyThreshold,
    ];
    pub fn name(self) -> &'static str {
        match self {
            LfoTarget::Off => "Off",
            LfoTarget::Zoom => "Zoom",
            LfoTarget::Rotate => "Rotate",
            LfoTarget::Spread => "Spread",
            LfoTarget::Twist => "Twist",
            LfoTarget::Feedback => "Feedback",
            LfoTarget::HueShift => "Hue shift",
            LfoTarget::Crossfade => "Crossfade",
            LfoTarget::CenterX => "Center X",
            LfoTarget::CenterY => "Center Y",
            LfoTarget::KeyThreshold => "Key threshold",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LfoShape {
    Sine,
    Triangle,
    Saw,
    Square,
}

impl LfoShape {
    pub const ALL: [LfoShape; 4] = [LfoShape::Sine, LfoShape::Triangle, LfoShape::Saw, LfoShape::Square];
    pub fn name(self) -> &'static str {
        match self {
            LfoShape::Sine => "Sine",
            LfoShape::Triangle => "Tri",
            LfoShape::Saw => "Saw",
            LfoShape::Square => "Square",
        }
    }
    /// Bipolar wave in [-1, 1] for phase in [0, 1).
    pub fn eval(self, phase: f32) -> f32 {
        let p = phase.rem_euclid(1.0);
        match self {
            LfoShape::Sine => (p * TAU).sin(),
            LfoShape::Triangle => 1.0 - 4.0 * (p - 0.5).abs(),
            LfoShape::Saw => 2.0 * p - 1.0,
            LfoShape::Square => {
                if p < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct Lfo {
    pub target: LfoTarget,
    pub shape: LfoShape,
    /// Length of one cycle, in beats of the global BPM.
    pub beats: f32,
    /// 0..1, scaled to a sensible range for each target.
    pub depth: f32,
}

impl Lfo {
    pub fn off() -> Self {
        Self {
            target: LfoTarget::Off,
            shape: LfoShape::Sine,
            beats: 8.0,
            depth: 0.3,
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
    pub lfos: [Lfo; 3],
    pub bpm: f32,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            deck_a: DeckParams::new(Pattern::Dot),
            deck_b: DeckParams::new(Pattern::Bars),
            crossfade: 0.0,
            blend: BlendMode::Crossfade,
            fx: FxParams::default(),
            lfos: [Lfo::off(), Lfo::off(), Lfo::off()],
            bpm: 120.0,
        }
    }
}

impl Params {
    /// Returns a copy with LFO modulation applied at `beat` (fractional beat count).
    pub fn modulated(&self, beat: f64) -> Params {
        let mut p = self.clone();
        for lfo in &self.lfos {
            if lfo.target == LfoTarget::Off || lfo.depth == 0.0 {
                continue;
            }
            let phase = (beat / lfo.beats.max(0.0625) as f64).fract() as f32;
            let v = lfo.shape.eval(phase) * lfo.depth;
            let fx = &mut p.fx;
            match lfo.target {
                LfoTarget::Off => {}
                LfoTarget::Zoom => fx.zoom = (fx.zoom + v * 0.25).clamp(0.05, 2.0),
                LfoTarget::Rotate => fx.rotate += v * 45.0,
                LfoTarget::Spread => fx.spread = (fx.spread + v * 0.5).max(0.0),
                LfoTarget::Twist => fx.twist += v * 90.0,
                LfoTarget::Feedback => fx.feedback = (fx.feedback + v * 0.2).clamp(0.0, 1.2),
                LfoTarget::HueShift => fx.hue_shift += v * 0.05,
                LfoTarget::Crossfade => p.crossfade = (p.crossfade + v).clamp(0.0, 1.0),
                LfoTarget::CenterX => fx.center_x += v * 0.5,
                LfoTarget::CenterY => fx.center_y += v * 0.5,
                LfoTarget::KeyThreshold => fx.key_threshold = (fx.key_threshold + v * 0.5).clamp(0.0, 1.0),
            }
        }
        p
    }
}

/// Built-in starting points. Each preset only rewrites the FX + LFO section so the
/// loaded decks stay as they are.
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
    let mut lfos = [Lfo::off(), Lfo::off(), Lfo::off()];
    match idx {
        // Classic camera-at-monitor zoom tunnel.
        0 => {
            fx.feedback = 0.95;
            fx.zoom = 0.93;
            fx.rotate = 3.0;
            fx.hue_shift = 0.012;
            lfos[0] = Lfo { target: LfoTarget::Rotate, shape: LfoShape::Sine, beats: 16.0, depth: 0.3 };
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
            lfos[0] = Lfo { target: LfoTarget::Twist, shape: LfoShape::Sine, beats: 32.0, depth: 0.15 };
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
            lfos[0] = Lfo { target: LfoTarget::Rotate, shape: LfoShape::Triangle, beats: 32.0, depth: 0.5 };
            lfos[1] = Lfo { target: LfoTarget::Spread, shape: LfoShape::Sine, beats: 16.0, depth: 0.15 };
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
            lfos[0] = Lfo { target: LfoTarget::Rotate, shape: LfoShape::Sine, beats: 64.0, depth: 0.4 };
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
            lfos[0] = Lfo { target: LfoTarget::Zoom, shape: LfoShape::Sine, beats: 16.0, depth: 0.2 };
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
            lfos[0] = Lfo { target: LfoTarget::CenterX, shape: LfoShape::Sine, beats: 8.0, depth: 0.15 };
            lfos[1] = Lfo { target: LfoTarget::CenterY, shape: LfoShape::Sine, beats: 12.0, depth: 0.15 };
        }
        _ => {
            fx.feedback = 0.0;
            fx.input_mode = InputMode::Add;
            fx.vignette = 0.0;
        }
    }
    p.fx = fx;
    p.lfos = lfos;
}

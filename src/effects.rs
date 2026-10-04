//! Effect registry. Every effect is one GPU pass with the same interface (see
//! `shaders/fx/header.wgsl`): an input, up to 24 parameters, and optionally a ring of past
//! frames for delays and feedback.

use crate::clip::next_id;
use crate::modulation::Shape;
use crate::param::{Param, Params, Spec};
use crate::shader::{CustomShader, Role};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum EffectKind {
    Feedback,
    Echo,
    RgbSplit,
    Kaleidoscope,
    Mirror,
    Color,
    LumaKey,
    Blur,
    Wave,
    Edges,
    Pixelate,
    Crt,
    Strobe,
    Transform,
    /// User GLSL (see `shader.rs`).
    Shader,
}

/// Which frames the history ring keeps.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HistorySource {
    /// The effect's input (delay lines that shouldn't feed back on themselves).
    Input,
    /// The effect's own output (feedback).
    Output,
}

#[derive(Clone, Copy)]
pub struct History {
    pub frames: u32,
    pub source: HistorySource,
    /// "Frames ago" for each of the four taps, from the live parameter values.
    pub taps: fn(&[Param]) -> [u32; 4],
}

pub struct EffectDef {
    pub kind: EffectKind,
    pub name: &'static str,
    pub category: &'static str,
    pub specs: &'static [Spec],
    pub shader: &'static str,
    pub history: Option<History>,
}

const ON_OFF: &[&str] = &["Off", "On"];

const FEEDBACK: &[Spec] = &[
    Spec::new("feedback", 0.0, 1.2, 0.92),
    Spec::new("copies (monitors)", 1.0, 8.0, 1.0).int(),
    Spec::new("copy scale", 0.05, 2.0, 0.95),
    Spec::new("rotate °", -180.0, 180.0, 2.0),
    Spec::new("spread", 0.0, 1.5, 0.0),
    Spec::new("twist ° / copy", -180.0, 180.0, 0.0),
    Spec::new("center x", -0.8, 0.8, 0.0),
    Spec::new("center y", -0.5, 0.5, 0.0),
    Spec::choice("combine", &["Lighten", "Add", "Average"], 0),
    Spec::choice("edges", &["Black (bezel)", "Mirror", "Wrap / tile"], 0),
    Spec::choice("symmetry", &["None", "Mirror X", "Mirror XY", "Kaleidoscope"], 0),
    Spec::new("segments", 2.0, 16.0, 6.0).int(),
    Spec::new("hue / pass", -0.1, 0.1, 0.01),
    Spec::new("saturation", 0.0, 2.0, 1.05),
    Spec::new("contrast", 0.5, 2.0, 1.0),
    Spec::new("blur", 0.0, 1.0, 0.0),
    Spec::new("noise", 0.0, 1.0, 0.0),
    Spec::choice("input mode", &["Luma key", "Add", "Lighten", "Difference", "Over (alpha)"], 4),
    Spec::new("input level", 0.0, 1.5, 1.0),
    Spec::new("key threshold", 0.0, 1.0, 0.15),
    Spec::new("key softness", 0.0, 0.5, 0.1),
    Spec::new("loop delay (frames)", 1.0, 30.0, 1.0).int(),
];

pub static EFFECTS: &[EffectDef] = &[
    EffectDef {
        kind: EffectKind::Feedback,
        name: "Feedback / Fractal",
        category: "Feedback",
        specs: FEEDBACK,
        shader: include_str!("shaders/fx/feedback.wgsl"),
        history: Some(History {
            frames: 32,
            source: HistorySource::Output,
            taps: |p| [p[21].index() as u32, 0, 0, 0],
        }),
    },
    EffectDef {
        kind: EffectKind::Echo,
        name: "Echo trails",
        category: "Time",
        specs: &[
            Spec::new("amount", 0.0, 1.0, 0.6),
            Spec::new("spacing (frames)", 1.0, 10.0, 4.0).int(),
            Spec::new("decay", 0.0, 1.0, 0.7),
            Spec::choice("mode", &["Lighten", "Add", "Average"], 0),
        ],
        shader: include_str!("shaders/fx/echo.wgsl"),
        history: Some(History {
            frames: 32,
            source: HistorySource::Input,
            taps: |p| {
                let s = p[1].index() as u32;
                [0, s, s * 2, s * 3]
            },
        }),
    },
    EffectDef {
        kind: EffectKind::RgbSplit,
        name: "RGB time split",
        category: "Time",
        specs: &[
            Spec::new("delay (frames)", 1.0, 15.0, 4.0).int(),
            Spec::new("amount", 0.0, 1.0, 1.0),
        ],
        shader: include_str!("shaders/fx/chroma.wgsl"),
        history: Some(History {
            frames: 32,
            source: HistorySource::Input,
            taps: |p| {
                let d = p[0].index() as u32;
                [0, d, d * 2, 0]
            },
        }),
    },
    EffectDef {
        kind: EffectKind::Kaleidoscope,
        name: "Kaleidoscope",
        category: "Space",
        specs: &[
            Spec::new("segments", 2.0, 24.0, 6.0).int(),
            Spec::new("rotation °", -180.0, 180.0, 0.0),
            Spec::new("center x", -0.8, 0.8, 0.0),
            Spec::new("center y", -0.5, 0.5, 0.0),
            Spec::new("zoom", 0.2, 4.0, 1.0).log(),
        ],
        shader: include_str!("shaders/fx/kaleido.wgsl"),
        history: None,
    },
    EffectDef {
        kind: EffectKind::Mirror,
        name: "Mirror",
        category: "Space",
        specs: &[Spec::choice(
            "mode",
            &["Left → right", "Right → left", "Top → bottom", "Bottom → top", "Quad"],
            0,
        )],
        shader: include_str!("shaders/fx/mirror.wgsl"),
        history: None,
    },
    EffectDef {
        kind: EffectKind::Transform,
        name: "Transform",
        category: "Space",
        specs: &[
            Spec::new("zoom", 0.1, 4.0, 1.0).log(),
            Spec::new("rotate °", -180.0, 180.0, 0.0),
            Spec::new("x", -1.0, 1.0, 0.0),
            Spec::new("y", -1.0, 1.0, 0.0),
            Spec::choice("tile", &["Off", "Repeat", "Mirror"], 0),
        ],
        shader: include_str!("shaders/fx/transform.wgsl"),
        history: None,
    },
    EffectDef {
        kind: EffectKind::Wave,
        name: "Wave warp",
        category: "Space",
        specs: &[
            Spec::new("amplitude", 0.0, 0.2, 0.02),
            Spec::new("frequency", 0.5, 40.0, 6.0).log(),
            Spec::new("speed", 0.0, 10.0, 1.0),
            Spec::choice("direction", &["Horizontal", "Vertical", "Radial"], 0),
        ],
        shader: include_str!("shaders/fx/wave.wgsl"),
        history: None,
    },
    EffectDef {
        kind: EffectKind::Color,
        name: "Color",
        category: "Color",
        specs: &[
            Spec::new("hue", 0.0, 1.0, 0.0),
            Spec::new("saturation", 0.0, 2.0, 1.0),
            Spec::new("contrast", 0.0, 2.0, 1.0),
            Spec::new("brightness", 0.0, 2.0, 1.0),
            Spec::choice("invert", ON_OFF, 0),
            Spec::new("gamma", 0.2, 3.0, 1.0).log(),
        ],
        shader: include_str!("shaders/fx/color.wgsl"),
        history: None,
    },
    EffectDef {
        kind: EffectKind::LumaKey,
        name: "Luma key",
        category: "Color",
        specs: &[
            Spec::new("threshold", 0.0, 1.0, 0.1),
            Spec::new("softness", 0.0, 0.5, 0.1),
            Spec::choice("key out bright", ON_OFF, 0),
        ],
        shader: include_str!("shaders/fx/lumakey.wgsl"),
        history: None,
    },
    EffectDef {
        kind: EffectKind::Pixelate,
        name: "Pixelate / posterize",
        category: "Color",
        specs: &[
            Spec::new("pixel size", 1.0, 64.0, 8.0).log(),
            Spec::new("posterize levels", 0.0, 16.0, 0.0).int(),
        ],
        shader: include_str!("shaders/fx/pixelate.wgsl"),
        history: None,
    },
    EffectDef {
        kind: EffectKind::Blur,
        name: "Blur",
        category: "Stylize",
        specs: &[Spec::new("radius (px)", 0.0, 40.0, 6.0)],
        shader: include_str!("shaders/fx/blur.wgsl"),
        history: None,
    },
    EffectDef {
        kind: EffectKind::Edges,
        name: "Edges",
        category: "Stylize",
        specs: &[Spec::new("strength", 0.0, 6.0, 2.0), Spec::new("mix", 0.0, 1.0, 1.0)],
        shader: include_str!("shaders/fx/edges.wgsl"),
        history: None,
    },
    EffectDef {
        kind: EffectKind::Crt,
        name: "CRT",
        category: "Stylize",
        specs: &[
            Spec::new("scanlines", 0.0, 1.0, 0.4),
            Spec::new("vignette", 0.0, 1.0, 0.4),
            Spec::new("noise", 0.0, 1.0, 0.1),
            Spec::new("RGB shift (px)", 0.0, 10.0, 1.5),
            Spec::new("curvature", 0.0, 0.3, 0.05),
        ],
        shader: include_str!("shaders/fx/crt.wgsl"),
        history: None,
    },
    EffectDef {
        kind: EffectKind::Strobe,
        name: "Strobe",
        category: "Stylize",
        specs: &[
            Spec::choice("rate (beats)", &["1/16", "1/8", "1/4", "1/2", "1", "2"], 2),
            Spec::new("on time", 0.05, 0.95, 0.5),
            Spec::choice("mode", &["Black", "White flash", "Invert", "Hide"], 0),
        ],
        shader: include_str!("shaders/fx/strobe.wgsl"),
        history: None,
    },
];

/// Registry entry for user shaders; rendering is special-cased (no built-in WGSL).
static SHADER_DEF: EffectDef = EffectDef {
    kind: EffectKind::Shader,
    name: "Custom shader (GLSL)",
    category: "Code",
    specs: &[],
    shader: "",
    history: None,
};

pub fn def(kind: EffectKind) -> &'static EffectDef {
    if kind == EffectKind::Shader {
        return &SHADER_DEF;
    }
    EFFECTS.iter().find(|d| d.kind == kind).expect("every kind is registered")
}

pub struct Effect {
    pub id: u64,
    pub kind: EffectKind,
    pub enabled: bool,
    pub params: Vec<Param>,
    /// Code and state for `EffectKind::Shader`.
    pub custom: Option<Box<CustomShader>>,
}

impl Effect {
    pub fn new(kind: EffectKind) -> Self {
        Self {
            id: next_id(),
            kind,
            enabled: true,
            params: def(kind).specs.iter().map(|s| Param::new(*s)).collect(),
            custom: None,
        }
    }

    pub fn custom(name: &str, source: &str) -> Self {
        let mut e = Self::new(EffectKind::Shader);
        e.custom = Some(Box::new(CustomShader::new(name, source, Role::Effect)));
        e
    }

    pub fn name(&self) -> &str {
        match &self.custom {
            Some(c) => &c.name,
            None => self.def().name,
        }
    }

    pub fn def(&self) -> &'static EffectDef {
        def(self.kind)
    }

    /// Set a parameter by its label (used by presets).
    pub fn set(&mut self, label: &str, v: f32) -> &mut Param {
        let p = self
            .params
            .iter_mut()
            .find(|p| p.spec.label == label)
            .unwrap_or_else(|| panic!("no parameter {label}"));
        p.set(v);
        p
    }

    fn reset(&mut self) {
        for p in &mut self.params {
            p.set(p.spec.default);
            p.modulator = None;
        }
    }
}

impl Params for Effect {
    fn visit_params(&mut self, f: &mut dyn FnMut(&str, &mut Param)) {
        for p in &mut self.params {
            let label = p.spec.label;
            f(label, p);
        }
        if let Some(c) = &mut self.custom {
            for p in &mut c.params {
                let label = p.spec.label;
                f(label, p);
            }
            f("alpha", &mut c.alpha);
        }
    }
}

/// Starting points for the Feedback effect (the old single-rig presets).
pub const FEEDBACK_PRESETS: &[&str] = &[
    "Tunnel",
    "Sierpinski",
    "Mandala",
    "Slow trails",
    "Spiral galaxy",
    "Hall of mirrors",
    "Melt",
];

pub fn apply_feedback_preset(e: &mut Effect, idx: usize) {
    e.reset();
    let lfo = |p: &mut Param, shape, beats, depth| {
        *p = p.clone().lfo(shape, beats, depth);
    };
    match idx {
        0 => {
            e.set("feedback", 0.95);
            e.set("copy scale", 0.93);
            let p = e.set("rotate °", 3.0);
            lfo(p, Shape::Sine, 16.0, 0.075);
            e.set("hue / pass", 0.012);
        }
        1 => {
            e.set("feedback", 1.0);
            e.set("copies (monitors)", 3.0);
            e.set("copy scale", 0.5);
            e.set("spread", 0.5);
            e.set("rotate °", 0.0);
            e.set("hue / pass", 0.02);
            let p = e.set("twist ° / copy", 0.0);
            lfo(p, Shape::Sine, 32.0, 0.075);
        }
        2 => {
            e.set("feedback", 0.97);
            e.set("copies (monitors)", 6.0);
            e.set("copy scale", 0.42);
            let p = e.set("spread", 0.62);
            lfo(p, Shape::Sine, 16.0, 0.1);
            let p = e.set("rotate °", 0.0);
            lfo(p, Shape::Triangle, 32.0, 0.125);
            e.set("twist ° / copy", 30.0);
            e.set("symmetry", 3.0);
            e.set("hue / pass", 0.015);
        }
        3 => {
            e.set("feedback", 0.6);
            e.set("copy scale", 1.0);
            e.set("rotate °", 0.0);
            e.set("loop delay (frames)", 12.0);
            e.set("hue / pass", 0.08);
            e.set("input mode", 2.0);
        }
        4 => {
            e.set("feedback", 0.98);
            e.set("copies (monitors)", 2.0);
            e.set("copy scale", 0.72);
            e.set("spread", 0.35);
            let p = e.set("rotate °", 20.0);
            lfo(p, Shape::Sine, 64.0, 0.1);
            e.set("hue / pass", 0.006);
        }
        5 => {
            e.set("feedback", 0.96);
            e.set("copies (monitors)", 4.0);
            let p = e.set("copy scale", 0.5);
            lfo(p, Shape::Sine, 16.0, 0.05);
            e.set("spread", 0.7);
            e.set("rotate °", 45.0);
            e.set("edges", 1.0);
            e.set("combine", 2.0);
            e.set("hue / pass", 0.01);
            e.set("contrast", 1.15);
        }
        _ => {
            e.set("feedback", 1.0);
            e.set("copy scale", 1.04);
            e.set("rotate °", -1.5);
            e.set("blur", 0.6);
            e.set("loop delay (frames)", 3.0);
            e.set("hue / pass", 0.004);
            e.set("saturation", 1.2);
            e.set("input mode", 3.0);
            e.set("input level", 0.8);
            let p = e.set("center x", 0.0);
            lfo(p, Shape::Sine, 8.0, 0.094);
            let p = e.set("center y", 0.0);
            lfo(p, Shape::Sine, 12.0, 0.15);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effects_fit_the_uniform_and_history() {
        for d in EFFECTS {
            assert!(d.specs.len() <= 24, "{} has too many params", d.name);
            if let Some(h) = d.history {
                // Worst-case taps must fit in the ring.
                let mut e = Effect::new(d.kind);
                for p in &mut e.params {
                    p.set(p.spec.max);
                }
                let taps = (h.taps)(&e.params);
                assert!(taps.iter().all(|t| *t < h.frames), "{}: taps {taps:?}", d.name);
            }
        }
    }

    #[test]
    fn presets_use_real_params() {
        for i in 0..FEEDBACK_PRESETS.len() {
            apply_feedback_preset(&mut Effect::new(EffectKind::Feedback), i);
        }
    }

    #[test]
    fn every_shader_validates() {
        let common = include_str!("shaders/common.wgsl");
        let header = include_str!("shaders/fx/header.wgsl");
        let mut sources: Vec<(String, String)> = EFFECTS
            .iter()
            .map(|d| (d.name.to_string(), format!("{common}\n{header}\n{}", d.shader)))
            .collect();
        for (name, body) in [
            ("clip", include_str!("shaders/clip.wgsl")),
            ("composite", include_str!("shaders/composite.wgsl")),
            ("final", include_str!("shaders/final.wgsl")),
        ] {
            sources.push((name.into(), format!("{common}\n{body}")));
        }
        for (name, src) in sources {
            let module = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{name}: {}", e.emit_to_string(&src)));
            naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::empty())
                .validate(&module)
                .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        }
    }
}

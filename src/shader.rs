//! User shaders, written or pasted at runtime (Shadertoy and GLSL Sandbox style).
//!
//! The user's GLSL is wrapped in a GLSL 4.50 template that provides Shadertoy's inputs,
//! compiled with naga (so errors come back as messages with line numbers instead of GPU
//! crashes), translated to WGSL, and only then handed to wgpu.
//!
//! Extra over Shadertoy:
//! * `uniform float name; // min max default` (or `int`) becomes a slider that can be automated.
//! * `iBeat`, `iBpm`: the global tempo clock.
//! * Channels: `iChannel0` = input (the layer, when used as an effect), `iChannel1` = this
//!   shader's previous frame (feedback), `iChannel2` = RGBA noise, `iChannel3` = the
//!   composition output from the previous frame.

use std::time::{Duration, Instant};

use crate::clip::next_id;
use crate::modulation::Clock;
use crate::param::{Param, Spec};

pub const MAX_PARAMS: usize = 16;
const COMPILE_DELAY: Duration = Duration::from_millis(350);

const HEADER: &str = "#version 450
layout(set = 0, binding = 0, std140) uniform TrippyUniforms {
    vec3 iResolution;
    float iTime;
    vec4 iMouse;
    vec4 iDate;
    vec4 _trippy_chres[4];
    float iTimeDelta;
    int iFrame;
    float iBeat;
    float iBpm;
    vec4 _trippy_p[4];
};
layout(set = 0, binding = 1) uniform sampler _trippy_samp;
layout(set = 0, binding = 2) uniform texture2D _trippy_ch0;
layout(set = 0, binding = 3) uniform texture2D _trippy_ch1;
layout(set = 0, binding = 4) uniform texture2D _trippy_ch2;
layout(set = 0, binding = 5) uniform texture2D _trippy_ch3;
layout(location = 0) out vec4 _trippy_out;
#define iChannel0 sampler2D(_trippy_ch0, _trippy_samp)
#define iChannel1 sampler2D(_trippy_ch1, _trippy_samp)
#define iChannel2 sampler2D(_trippy_ch2, _trippy_samp)
#define iChannel3 sampler2D(_trippy_ch3, _trippy_samp)
#define iChannelResolution _trippy_chres
#define iSampleRate 44100.0
#define texture2D texture
";

const SANDBOX_HEADER: &str = "#define time iTime
#define resolution (iResolution.xy)
#define mouse (iMouse.xy / iResolution.xy)
#define backbuffer iChannel1
#define gl_FragColor _trippy_out
";

const SHADERTOY_FOOTER: &str = "
void main() {
    vec4 _trippy_c = vec4(0.0, 0.0, 0.0, 1.0);
    mainImage(_trippy_c, gl_FragCoord.xy);
    _trippy_out = _trippy_c;
}
";

/// Names the template already provides; `uniform` declarations of these are dropped.
const BUILTINS: &[&str] = &[
    "iResolution", "iTime", "iTimeDelta", "iFrame", "iMouse", "iDate", "iChannel0", "iChannel1",
    "iChannel2", "iChannel3", "iChannelResolution", "iChannelTime", "iSampleRate", "iFrameRate",
    "time", "resolution", "mouse", "backbuffer", "surfaceSize", "iBeat", "iBpm",
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dialect {
    /// `void mainImage(out vec4 fragColor, in vec2 fragCoord)`
    Shadertoy,
    /// `void main()` writing `gl_FragColor`, with `time` / `resolution` / `mouse`.
    Sandbox,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParamDecl {
    pub name: String,
    pub int: bool,
    pub min: f32,
    pub max: f32,
    pub default: f32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Glsl,
    /// Detected by `@fragment`. Runs in screen space (y down), entry point chosen by the user.
    Wgsl,
}

pub struct Prepared {
    pub lang: Lang,
    /// Full shader source handed to the compiler.
    pub code: String,
    pub header_lines: usize,
    pub user_lines: usize,
    pub params: Vec<ParamDecl>,
}

/// A successfully compiled shader, ready for the renderer.
#[derive(Debug)]
pub struct Compiled {
    pub wgsl: String,
    pub entry: String,
    /// WGSL shaders work in screen space (top-left origin); GLSL ones in Shadertoy's GL
    /// space and get flipped.
    pub screen_space: bool,
}

pub fn lang_of(user: &str) -> Lang {
    if user.contains("@fragment") { Lang::Wgsl } else { Lang::Glsl }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CompileError {
    /// 1-based line in the user's code, when it points there.
    pub line: Option<usize>,
    pub message: String,
}

fn is_ident(s: &str) -> bool {
    let mut c = s.chars();
    matches!(c.next(), Some(ch) if ch.is_ascii_alphabetic() || ch == '_') && c.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

/// Wrap user code into a complete GLSL 4.50 fragment shader. Every user line maps to exactly
/// one output line, so error line numbers can be mapped back.
pub fn prepare(user: &str) -> Prepared {
    if lang_of(user) == Lang::Wgsl {
        return prepare_wgsl(user);
    }
    let dialect = if user.contains("mainImage") { Dialect::Shadertoy } else { Dialect::Sandbox };
    let mut params = Vec::new();
    let mut body = String::new();
    let user_lines = user.lines().count();
    for line in user.lines() {
        body.push_str(&rewrite_line(line, &mut params));
        body.push('\n');
    }
    let mut header = HEADER.to_string();
    if dialect == Dialect::Sandbox {
        header.push_str(SANDBOX_HEADER);
    }
    let header_lines = header.matches('\n').count();
    let mut glsl = header;
    glsl.push_str(&body);
    if dialect == Dialect::Shadertoy {
        glsl.push_str(SHADERTOY_FOOTER);
    }
    Prepared {
        lang: Lang::Glsl,
        code: glsl,
        header_lines,
        user_lines,
        params,
    }
}

/// Names of fields already in the WGSL `inputs` struct.
const WGSL_FIELDS: &[&str] = &["size", "time", "mouse", "date", "channel_resolution", "time_delta", "frame", "beat", "bpm"];

/// WGSL: user code is kept verbatim after a prelude declaring `inputs`, `samp` and
/// `iChannel0..3`. Sliders are declared with `// @param name min max default` and read as
/// `inputs.name`.
fn prepare_wgsl(user: &str) -> Prepared {
    let mut params = Vec::new();
    for line in user.lines() {
        let t = line.trim_start();
        let Some(rest) = t.strip_prefix("//").map(str::trim_start).and_then(|r| r.strip_prefix("@param")) else {
            continue;
        };
        let mut words = rest.split_whitespace();
        let Some(name) = words.next() else { continue };
        if !is_ident(name) || WGSL_FIELDS.contains(&name) || params.len() >= MAX_PARAMS || params.iter().any(|p: &ParamDecl| p.name == name) {
            continue;
        }
        let nums: Vec<f32> = words.filter_map(|w| w.trim_matches(',').parse().ok()).collect();
        let (min, max) = match nums.as_slice() {
            [a, b, ..] if b > a => (*a, *b),
            _ => (0.0, 1.0),
        };
        params.push(ParamDecl {
            name: name.to_string(),
            int: false,
            min,
            max,
            default: nums.get(2).copied().unwrap_or(min).clamp(min, max),
        });
    }
    let mut fields = String::new();
    for i in 0..MAX_PARAMS {
        match params.get(i) {
            Some(p) => fields.push_str(&format!("    {}: f32,\n", p.name)),
            None => fields.push_str(&format!("    _trippy_p{i}: f32,\n")),
        }
    }
    let prelude = format!(
        "struct TrippyInputs {{
    size: vec3f,
    time: f32,
    mouse: vec4f,
    date: vec4f,
    channel_resolution: array<vec4f, 4>,
    time_delta: f32,
    frame: i32,
    beat: f32,
    bpm: f32,
{fields}}};
@group(0) @binding(0) var<uniform> inputs: TrippyInputs;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var iChannel0: texture_2d<f32>;
@group(0) @binding(3) var iChannel1: texture_2d<f32>;
@group(0) @binding(4) var iChannel2: texture_2d<f32>;
@group(0) @binding(5) var iChannel3: texture_2d<f32>;
"
    );
    let header_lines = prelude.matches('\n').count();
    Prepared {
        lang: Lang::Wgsl,
        code: format!("{prelude}{user}\n"),
        header_lines,
        user_lines: user.lines().count(),
        params,
    }
}

fn rewrite_line(line: &str, params: &mut Vec<ParamDecl>) -> String {
    let t = line.trim_start();
    if t.starts_with("#version") || t.starts_with("#extension") || t.starts_with("precision ") {
        return String::new();
    }
    let Some(rest) = t.strip_prefix("uniform ") else {
        return line.to_string();
    };
    let (decl, comment) = match rest.split_once("//") {
        Some((d, c)) => (d, c),
        None => (rest, ""),
    };
    let mut words = decl.trim().trim_end_matches(';').split_whitespace().filter(|w| !matches!(*w, "lowp" | "mediump" | "highp"));
    let (Some(ty), Some(name)) = (words.next(), words.next()) else {
        return line.to_string();
    };
    let name = name.split('[').next().unwrap_or(name);
    if BUILTINS.contains(&name) {
        return String::new();
    }
    if !(ty == "float" || ty == "int") || !is_ident(name) || params.len() >= MAX_PARAMS {
        return line.to_string();
    }
    let int = ty == "int";
    let nums: Vec<f32> = comment.split(|c: char| c.is_whitespace() || c == ',').filter_map(|w| w.parse().ok()).collect();
    let (min, max) = match nums.as_slice() {
        [a, b, ..] if b > a => (*a, *b),
        _ if int => (0.0, 10.0),
        _ => (0.0, 1.0),
    };
    let default = nums.get(2).copied().unwrap_or(min).clamp(min, max);
    let i = params.len();
    params.push(ParamDecl {
        name: name.to_string(),
        int,
        min,
        max,
        default,
    });
    let slot = format!("_trippy_p[{}].{}", i / 4, ["x", "y", "z", "w"][i % 4]);
    if int {
        format!("#define {name} int({slot})")
    } else {
        format!("#define {name} ({slot})")
    }
}

/// User code -> validated naga module -> WGSL for wgpu.
pub fn compile(p: &Prepared) -> Result<Compiled, Vec<CompileError>> {
    let src = &p.code;
    let map_line = |full: u32| -> Option<usize> {
        let l = full as usize;
        (l > p.header_lines && l <= p.header_lines + p.user_lines).then(|| l - p.header_lines)
    };
    let module = match p.lang {
        Lang::Glsl => {
            let mut frontend = naga::front::glsl::Frontend::default();
            frontend
                .parse(&naga::front::glsl::Options::from(naga::ShaderStage::Fragment), src)
                .map_err(|errs| {
                    errs.errors
                        .iter()
                        .map(|e| CompileError {
                            line: map_line(e.meta.location(src).line_number),
                            message: e.kind.to_string(),
                        })
                        .collect::<Vec<_>>()
                })?
        }
        Lang::Wgsl => naga::front::wgsl::parse_str(src).map_err(|e| {
            let mut message = e.message().to_string();
            for (_, label) in e.labels() {
                if !label.is_empty() {
                    message.push_str(&format!(" ({label})"));
                }
            }
            vec![CompileError {
                line: e.location(src).and_then(|l| map_line(l.line_number)),
                message,
            }]
        })?,
    };
    let entry = module
        .entry_points
        .iter()
        .find(|e| e.stage == naga::ShaderStage::Fragment)
        .map(|e| e.name.clone())
        .ok_or_else(|| {
            vec![CompileError {
                line: None,
                message: "no @fragment entry point found".into(),
            }]
        })?;
    let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::default())
        .validate(&module)
        .map_err(|e| {
            let mut message = e.as_inner().to_string();
            let mut source = std::error::Error::source(e.as_inner());
            while let Some(s) = source {
                message.push_str(": ");
                message.push_str(&s.to_string());
                source = s.source();
            }
            // Spans go from outer (the function) to inner (the expression): use the innermost.
            let line = e
                .spans()
                .filter_map(|(span, _)| map_line(span.location(src).line_number))
                .last();
            vec![CompileError { line, message }]
        })?;
    let wgsl = naga::back::wgsl::write_string(&module, &info, naga::back::wgsl::WriterFlags::empty()).map_err(|e| {
        vec![CompileError {
            line: None,
            message: format!("could not translate to WGSL: {e}"),
        }]
    })?;
    Ok(Compiled {
        wgsl,
        entry,
        screen_space: p.lang == Lang::Wgsl,
    })
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    /// Generates an image (a clip).
    Source,
    /// Processes its input (an effect; `iChannel0` is the input).
    Effect,
}

pub const ALPHA_MODES: &[&str] = &["Opaque", "Luminance (black = transparent)", "Shader alpha"];

pub struct CustomShader {
    pub id: u64,
    pub name: String,
    pub source: String,
    /// Bumped on every edit.
    pub rev: u64,
    pub edited_at: Instant,
    /// Recompile automatically shortly after edits.
    pub live: bool,
    /// Compile on the next frame regardless of `live`.
    pub compile_requested: bool,
    pub compiled_rev: Option<u64>,
    /// Errors from the latest compile attempt (the last good version keeps running).
    pub errors: Vec<CompileError>,
    /// Whether a working version is running.
    pub running: bool,
    pub params: Vec<Param>,
    pub alpha: Param,
    /// Shadertoy iMouse, in output pixels (y up).
    pub mouse: [f32; 4],
}

impl CustomShader {
    pub fn new(name: &str, source: &str, role: Role) -> Self {
        let alpha_default = if role == Role::Effect { 2 } else { 0 };
        Self {
            id: next_id(),
            name: name.to_string(),
            source: source.to_string(),
            rev: 1,
            edited_at: Instant::now() - COMPILE_DELAY,
            live: true,
            compile_requested: true,
            compiled_rev: None,
            errors: Vec::new(),
            running: false,
            params: Vec::new(),
            alpha: Param::with(Spec::choice("alpha", ALPHA_MODES, alpha_default), alpha_default as f32),
            mouse: [0.0; 4],
        }
    }

    pub fn edited(&mut self) {
        self.rev += 1;
        self.edited_at = Instant::now();
    }

    /// If the code changed (and the edit has settled, or a compile was requested), compile
    /// it. Returns the compiled shader on success; errors are stored on `self`.
    pub fn poll_compile(&mut self) -> Option<Compiled> {
        if self.compiled_rev == Some(self.rev) && !self.compile_requested {
            return None;
        }
        let settled = self.live && self.edited_at.elapsed() >= COMPILE_DELAY;
        if !(settled || self.compile_requested) {
            return None;
        }
        self.compile_requested = false;
        self.compiled_rev = Some(self.rev);
        let prepared = prepare(&self.source);
        match compile(&prepared) {
            Ok(compiled) => {
                self.errors.clear();
                self.sync_params(&prepared.params);
                Some(compiled)
            }
            Err(errs) => {
                self.errors = errs;
                None
            }
        }
    }

    /// Keep values and automation of parameters that still exist (matched by name).
    fn sync_params(&mut self, decls: &[ParamDecl]) {
        let old = std::mem::take(&mut self.params);
        self.params = decls
            .iter()
            .map(|d| {
                let prev = old.iter().find(|p| p.spec.label == d.name);
                // Labels are &'static: reuse the existing one, leak only genuinely new names.
                let label: &'static str = match prev {
                    Some(p) => p.spec.label,
                    None => Box::leak(d.name.clone().into_boxed_str()),
                };
                let mut spec = Spec::new(label, d.min, d.max, d.default);
                if d.int {
                    spec = spec.int();
                }
                match prev {
                    Some(prev) => {
                        let mut p = prev.clone();
                        p.spec = spec;
                        p.set(prev.value);
                        p
                    }
                    None => Param::new(spec),
                }
            })
            .collect();
    }

    pub fn tick(&mut self, clock: Clock) {
        for p in &mut self.params {
            p.tick(clock);
        }
        self.alpha.tick(clock);
    }
}

/// Starting points. `(name, role, code)`.
pub const TEMPLATES: &[(&str, Role, &str)] = &[
    (
        "Shadertoy default",
        Role::Source,
        "void mainImage( out vec4 fragColor, in vec2 fragCoord )
{
    // Normalized pixel coordinates (from 0 to 1)
    vec2 uv = fragCoord/iResolution.xy;

    // Time varying pixel color
    vec3 col = 0.5 + 0.5*cos(iTime+uv.xyx+vec3(0,2,4));

    // Output to screen
    fragColor = vec4(col,1.0);
}
",
    ),
    (
        "Plasma (with sliders)",
        Role::Source,
        "// `uniform float name; // min max default` becomes a slider you can automate.
uniform float speed; // 0 4 1
uniform float scale; // 1 20 6
uniform float hue;   // 0 1 0

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = (fragCoord - 0.5 * iResolution.xy) / iResolution.y;
    float t = iTime * speed;
    float v = sin(uv.x * scale + t)
            + sin(uv.y * scale * 1.3 - t)
            + sin(length(uv) * scale * 1.7 + t * 0.7);
    vec3 col = 0.5 + 0.5 * cos(6.28318 * (v * 0.25 + hue + vec3(0.0, 0.33, 0.67)));
    fragColor = vec4(col, 1.0);
}
",
    ),
    (
        "Beat tunnel",
        Role::Source,
        "// iBeat counts beats at the current BPM.
uniform float rings; // 2 30 10
uniform float twist; // 0 4 1

mat2 rot(float a) { float c = cos(a), s = sin(a); return mat2(c, -s, s, c); }

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 p = (fragCoord - 0.5 * iResolution.xy) / iResolution.y;
    float r = length(p);
    float a = atan(p.y, p.x);
    float pulse = exp(-4.0 * fract(iBeat));
    float z = 1.0 / max(r, 0.001) + iTime * 2.0;
    float stripes = smoothstep(0.4, 0.5, abs(fract(z * rings * 0.05 + a * twist / 6.28318) - 0.5));
    vec3 col = 0.5 + 0.5 * cos(vec3(0.0, 2.0, 4.0) + z * 0.3 + iBeat * 0.25);
    fragColor = vec4(col * stripes * (0.6 + pulse) * smoothstep(0.0, 0.3, r), 1.0);
}
",
    ),
    (
        "Feedback ripples (iChannel1)",
        Role::Source,
        "// iChannel1 is this shader's previous frame: classic feedback.
uniform float decay; // 0.8 1 0.97
uniform float zoom;  // 0.9 1.1 0.99

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = fragCoord / iResolution.xy;
    vec2 c = uv - 0.5;
    float a = 0.01 * sin(iTime * 0.3);
    mat2 r = mat2(cos(a), -sin(a), sin(a), cos(a));
    vec2 prev_uv = r * c * zoom + 0.5;
    vec3 prev = texture(iChannel1, prev_uv).rgb * decay;
    vec2 dot_pos = 0.5 + 0.3 * vec2(cos(iTime * 1.3), sin(iTime * 1.7));
    float d = length((uv - dot_pos) * vec2(iResolution.x / iResolution.y, 1.0));
    vec3 ink = (0.5 + 0.5 * cos(iTime + vec3(0, 2, 4))) * smoothstep(0.03, 0.0, d);
    fragColor = vec4(max(prev, ink), 1.0);
}
",
    ),
    (
        "Input: RGB shift + wave (effect)",
        Role::Effect,
        "// As an effect, iChannel0 is the layer's input.
uniform float amount; // 0 0.05 0.01
uniform float wave;   // 0 0.1 0.02

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = fragCoord / iResolution.xy;
    uv.x += wave * sin(uv.y * 20.0 + iTime * 3.0);
    float r = texture(iChannel0, uv + vec2(amount, 0.0)).r;
    vec4 g = texture(iChannel0, uv);
    float b = texture(iChannel0, uv - vec2(amount, 0.0)).b;
    fragColor = vec4(r, g.g, b, g.a);
}
",
    ),
    (
        "Input: zoom feedback (effect)",
        Role::Effect,
        "// Mixes the input over a zooming copy of the previous frame (iChannel1).
uniform float feedback; // 0 1 0.9
uniform float zoom;     // 0.8 1.2 0.97
uniform float spin;     // -0.1 0.1 0.01

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = fragCoord / iResolution.xy;
    vec2 c = (uv - 0.5) * vec2(iResolution.x / iResolution.y, 1.0);
    mat2 r = mat2(cos(spin), -sin(spin), sin(spin), cos(spin));
    c = r * c * zoom;
    vec2 puv = c / vec2(iResolution.x / iResolution.y, 1.0) + 0.5;
    vec4 prev = texture(iChannel1, puv) * feedback;
    prev.rgb = 0.5 + 0.5 * cos(6.28318 * (prev.rgb + 0.02)) * prev.a;
    vec4 inp = texture(iChannel0, uv);
    fragColor = inp + prev * (1.0 - inp.a);
}
",
    ),
    (
        "WGSL starter",
        Role::Source,
        "// WGSL works too (detected by @fragment). It runs in screen space: pos.y grows downward.
// Available: inputs.size (vec3f, pixels), inputs.time, inputs.mouse, inputs.date,
// inputs.frame, inputs.time_delta, inputs.beat, inputs.bpm,
// iChannel0..iChannel3 (texture_2d<f32>) with sampler `samp`.
// Sliders: `// @param name min max default`, read as inputs.name.
// @param speed 0 4 1
// @param rings 1 40 12

@fragment
fn fs(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let uv = pos.xy / inputs.size.xy;
    let p = (pos.xy - 0.5 * inputs.size.xy) / inputs.size.y;
    let t = inputs.time * inputs.speed;
    let r = length(p);
    let v = 0.5 + 0.5 * sin(r * inputs.rings - t * 3.0 + atan2(p.y, p.x) * 3.0);
    let col = 0.5 + 0.5 * cos(vec3f(0.0, 2.0, 4.0) + t + uv.x * 2.0);
    return vec4f(col * v, 1.0);
}
",
    ),
    (
        "GLSL Sandbox style",
        Role::Source,
        "#ifdef GL_ES
precision mediump float;
#endif

uniform float time;
uniform vec2 resolution;

void main(void) {
    vec2 p = (gl_FragCoord.xy * 2.0 - resolution) / min(resolution.x, resolution.y);
    float l = 0.1 / abs(length(p) - 0.5 - 0.1 * sin(time * 2.0 + atan(p.y, p.x) * 6.0));
    gl_FragColor = vec4(vec3(l * 0.4, l * 0.7, l), 1.0);
}
",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn compiles(code: &str) {
        let p = prepare(code);
        if let Err(errs) = compile(&p) {
            panic!("compile failed: {errs:#?}\n---\n{}", p.code);
        }
    }

    #[test]
    fn all_templates_compile() {
        for (name, _, code) in TEMPLATES {
            let p = prepare(code);
            compile(&p).unwrap_or_else(|e| panic!("{name}: {e:#?}"));
        }
    }

    #[test]
    fn params_become_sliders() {
        let p = prepare("uniform float speed; // 0 4 1\nuniform int count; // 1 8\nuniform float plain;\nvoid mainImage(out vec4 c, in vec2 f) { c = vec4(speed * float(count) * plain); }\n");
        assert_eq!(
            p.params,
            vec![
                ParamDecl { name: "speed".into(), int: false, min: 0.0, max: 4.0, default: 1.0 },
                ParamDecl { name: "count".into(), int: true, min: 1.0, max: 8.0, default: 1.0 },
                ParamDecl { name: "plain".into(), int: false, min: 0.0, max: 1.0, default: 0.0 },
            ]
        );
        compile(&p).unwrap();
    }

    #[test]
    fn errors_point_at_user_lines() {
        let p = prepare("void mainImage(out vec4 c, in vec2 f) {\n    c = vec4(1.0);\n    c.x = undefined_thing;\n}\n");
        let errs = compile(&p).unwrap_err();
        assert_eq!(errs[0].line, Some(3), "{errs:?}");
    }

    #[test]
    fn validation_errors_point_at_the_expression() {
        // naga rejects `ivec2 % int`; the error should name the line with the expression.
        let p = prepare("void mainImage(out vec4 c, in vec2 f) {\n    c = vec4(0.0);\n    ivec2 m = ivec2(f) % 3;\n    c.x = float(m.x);\n}\n");
        let errs = compile(&p).unwrap_err();
        assert_eq!(errs[0].line, Some(3), "{errs:?}");
    }

    #[test]
    fn wgsl_with_params_and_own_entry_point() {
        let p = prepare("// @param speed 0 4 2\n@fragment\nfn frag(@builtin(position) pos: vec4f) -> @location(0) vec4f {\n    return vec4f(inputs.time * inputs.speed, pos.x / inputs.size.x, 0.0, 1.0);\n}\n");
        assert_eq!(p.lang, Lang::Wgsl);
        assert_eq!(p.params[0], ParamDecl { name: "speed".into(), int: false, min: 0.0, max: 4.0, default: 2.0 });
        let c = compile(&p).unwrap();
        assert_eq!(c.entry, "frag");
        assert!(c.screen_space);
    }

    #[test]
    fn wgsl_errors_point_at_user_lines() {
        let p = prepare("@fragment\nfn f() -> @location(0) vec4f {\n    return vec4f(nope);\n}\n");
        let errs = compile(&p).unwrap_err();
        assert_eq!(errs[0].line, Some(3), "{errs:?}");
    }

    /// The repo's own WGSL port of Shadertoy MtX3Ws, pasted as-is.
    #[test]
    fn marble_wgsl_pastes_as_is() {
        let Ok(code) = std::fs::read_to_string("src/shaders/marble.wgsl") else { return };
        let c = compile(&prepare(&code)).unwrap_or_else(|e| panic!("{e:#?}"));
        assert_eq!(c.entry, "fragmentMain");
    }

    #[test]
    fn typical_shadertoy_code() {
        // Constructs commonly found in pasted Shadertoy shaders.
        compiles(
            "#define PI 3.14159265
#define R iResolution.xy
const int STEPS = 64;
uniform sampler2D iChannel0;
precision highp float;

float hash(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
mat2 rot(float a) { float c = cos(a), s = sin(a); return mat2(c, -s, s, c); }
float sdBox(vec3 p, vec3 b) { vec3 q = abs(p) - b; return length(max(q, 0.0)) + min(max(q.x, max(q.y, q.z)), 0.0); }

float map(vec3 p) {
    p.xy *= rot(iTime * 0.5);
    p = mod(p + 2.0, 4.0) - 2.0;
    return sdBox(p, vec3(0.5));
}

void mainImage(out vec4 fragColor, in vec2 fragCoord) {
    vec2 uv = (fragCoord - 0.5 * R) / R.y;
    vec3 ro = vec3(0.0, 0.0, -3.0 + iTime);
    vec3 rd = normalize(vec3(uv, 1.0));
    float t = 0.0;
    for (int i = 0; i < STEPS; i++) {
        float d = map(ro + rd * t);
        if (d < 0.001) break;
        t += d;
    }
    vec3 col = vec3(1.0 / (1.0 + t * t * 0.05));
    col += texture(iChannel0, uv).rgb * 0.1 + texelFetch(iChannel2, ivec2(fragCoord) % ivec2(256), 0).rgb * 0.02;
    col *= 0.9 + 0.1 * hash(fragCoord + iTime);
    col += iMouse.xyz * 0.0 + iChannelResolution[0].xyz * 0.0 + vec3(float(iFrame) * 0.0, iDate.w * 0.0, iTimeDelta * 0.0);
    fragColor = vec4(pow(col, vec3(0.4545)), 1.0);
}
",
        );
    }
}

//! Sets: a whole composition in a `.tripset` file (JSON), saved and opened again. The demo sets
//! in `sets/` are bundled into the app in this format. `docs/sets.md` describes the format.
//!
//! The file only holds what differs from a fresh composition, so a hand-written set can be
//! short. Parameters are stored by label rather than position, so adding parameters to an
//! effect later doesn't break old sets; labels a set has that the app doesn't are reported and
//! skipped. Choice parameters are written by name ("Repeat") and enums in snake case
//! ("play_once_hold").

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::clip::{Clip, Direction, Fit, LoopMode, Media, PATTERNS, Sync};
use crate::composition::{Blend, Composition, Crossfade, FadeCurve, Quantize, Side};
use crate::effects::{EFFECTS, Effect, EffectKind, FEEDBACK_PRESETS, apply_feedback_preset};
use crate::isf_library::{Kind, Library};
use crate::modulation::{Modulator, Polarity, Rate, Shape};
use crate::param::{Param, Params};
use crate::renderer::{MEDIA_HEIGHT, MEDIA_WIDTH};
use crate::shader::{CustomShader, Role};
use crate::source;

/// The format version this build writes. Sets from a newer version still open, with a warning.
pub const VERSION: u32 = 1;
pub const EXTENSION: &str = "tripset";

mod bundle {
    include!(concat!(env!("OUT_DIR"), "/set_bundle.rs"));
}

/// Media compiled into the app, by path from the repo root: the stills and logos the bundled
/// sets use, so they work from any folder and in the browser. A set can name one directly as
/// `builtin:logos/tripslop-flower-color.svg`.
pub static MEDIA: &[(&str, &[u8])] = &[
    ("samples/crab-nebula.jpg", include_bytes!("../samples/crab-nebula.jpg")),
    ("samples/pillars-of-creation.jpg", include_bytes!("../samples/pillars-of-creation.jpg")),
    ("logos/tripslop-flower-color.svg", include_bytes!("../logos/tripslop-flower-color.svg")),
    ("logos/tripslop-flower-cream.svg", include_bytes!("../logos/tripslop-flower-cream.svg")),
    ("logos/tripslop-flower-plum.svg", include_bytes!("../logos/tripslop-flower-plum.svg")),
    ("logos/tripslop-wordmark-color.svg", include_bytes!("../logos/tripslop-wordmark-color.svg")),
    ("logos/tripslop-wordmark-cream.svg", include_bytes!("../logos/tripslop-wordmark-cream.svg")),
    ("logos/tripslop-wordmark-plum.svg", include_bytes!("../logos/tripslop-wordmark-plum.svg")),
];

const BUILTIN: &str = "builtin:";

// ------------------------------------------------------------------ the file format

type ParamMap = BTreeMap<String, ParamSpec>;

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
#[serde(default)]
pub struct SetFile {
    /// Format version.
    pub tripslop: u32,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bpm: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quantize: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crossfade: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fade_curve: Option<String>,
    /// At least this many scenes (default 8; there are always enough for the names and clips).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub columns: Option<usize>,
    /// Scene names, left to right.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub scenes: Vec<String>,
    /// The scene launched on opening (1-based; 0 for none). Default 1.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<usize>,
    /// `master` and `crossfader`.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub params: ParamMap,
    /// The master chain.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<EffectSpec>,
    /// Bottom layer first (layer 1 in scripts and the UI).
    pub layers: Vec<LayerSpec>,
}

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
#[serde(default)]
pub struct LayerSpec {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Index into the UI's layer colors.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blend: Option<String>,
    /// Crossfader side: `a`, `b` or `both`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub side: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub bypass: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub solo: bool,
    /// opacity, transition, position x / y, scale, rotation.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub params: ParamMap,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<EffectSpec>,
    /// One per scene, `null` for an empty cell.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub clips: Vec<Option<ClipSpec>>,
}

/// A clip: one of `file`, `generator`, `shader`, `library`, `model` or `camera`, then how it
/// plays.
#[derive(Serialize, Deserialize, Default, Debug, Clone)]
#[serde(default)]
pub struct ClipSpec {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// A video, image, SVG or shader file, relative to the set (or `builtin:…`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// An SVG's size in the frame (0..1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fill: Option<f32>,
    /// A built-in generator: bars, rings, plasma, checker, dot, noise, solid.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shader: Option<ShaderSpec>,
    /// A generator from the shader library, by name (or key).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub library: Option<String>,
    /// A 3D model from the library, by name (or key).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub camera: Option<u32>,
    #[serde(rename = "loop", skip_serializing_if = "Option::is_none")]
    pub loop_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fit: Option<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub params: ParamMap,
    /// Used instead when this clip can't be opened: a video in the browser, a missing file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fallback: Option<Box<ClipSpec>>,
}

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
#[serde(default)]
pub struct EffectSpec {
    /// Snake case (`feedback`, `rgb_split`, `crt`, …); `shader` for code, which is implied by
    /// `shader`, `library` or `file`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "is_false")]
    pub half_history: bool,
    /// A Feedback preset by name, applied before `params`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shader: Option<ShaderSpec>,
    /// An effect from the shader library, by name (or key).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub library: Option<String>,
    /// A shader file, relative to the set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// The Shape projector's / Projection mapping's library model, by name (or key).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub params: ParamMap,
}

/// Shader code kept in the set.
#[derive(Serialize, Deserialize, Default, Debug, Clone)]
#[serde(default)]
pub struct ShaderSpec {
    pub name: String,
    pub code: String,
    /// An ISF shader's vertex shader.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vertex: Option<String>,
}

/// A parameter: just its value, or `{ "value": …, "mod": { … } }`.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(untagged)]
pub enum ParamSpec {
    Value(Scalar),
    Full {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<Scalar>,
        #[serde(rename = "mod", default, skip_serializing_if = "Option::is_none")]
        modulator: Option<ModSpec>,
    },
}

/// A number, or a choice parameter's option by name.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(untagged)]
pub enum Scalar {
    Num(f32),
    Name(String),
}

/// A modulator; anything left out has the app's default.
#[derive(Serialize, Deserialize, Default, Debug, Clone, PartialEq)]
#[serde(default)]
pub struct ModSpec {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shape: Option<String>,
    /// Cycle length in beats (tempo-synced)…
    #[serde(skip_serializing_if = "Option::is_none")]
    pub beats: Option<f32>,
    /// …or free-running cycles per second.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hz: Option<f32>,
    /// Fraction of the parameter's range.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub polarity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<f32>,
    /// The square wave's duty cycle.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<f32>,
    /// The envelope's breakpoints, `[position in cycle, level]`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub points: Option<Vec<[f32; 2]>>,
    /// What the audio shape follows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub band: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

pub fn parse(json: &str) -> Result<SetFile, String> {
    serde_json::from_str(json).map_err(|e| e.to_string())
}

pub fn to_json(set: &SetFile) -> String {
    serde_json::to_string_pretty(set).unwrap_or_default() + "\n"
}

// ------------------------------------------------------------------ enums by name

/// `PlayOnceHold` → `play_once_hold`.
fn key<T: Debug>(v: T) -> String {
    let mut out = String::new();
    for (i, c) in format!("{v:?}").chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push('_');
        }
        out.extend(c.to_lowercase());
    }
    out
}

fn pick<T: Copy + Debug>(all: &[T], s: &str, what: &str) -> Result<T, String> {
    let want = s.trim().to_lowercase().replace([' ', '-'], "_");
    all.iter().copied().find(|v| key(*v) == want).ok_or_else(|| {
        let names: Vec<String> = all.iter().map(|v| key(*v)).collect();
        format!("no {what} {s:?} (use {})", names.join(", "))
    })
}

/// `Some(name)` unless it's the default.
fn name_unless<T: Copy + Debug + PartialEq>(v: T, default: T) -> Option<String> {
    (v != default).then(|| key(v))
}

const SIDES: [Side; 3] = [Side::Both, Side::A, Side::B];
const DIRECTIONS: [Direction; 3] = [Direction::Forward, Direction::Reverse, Direction::Paused];
const SYNCS: [Sync; 2] = [Sync::Timeline, Sync::Bpm];

fn effect_kinds() -> Vec<EffectKind> {
    EFFECTS.iter().map(|d| d.kind).chain([EffectKind::Shader]).collect()
}

// ------------------------------------------------------------------ parameters

/// Apply `specs` to `target`'s parameters, matching labels case-insensitively (exactly, else
/// by prefix: `rotate` finds `rotate °`). Returns the entries `target` has no parameter for.
fn apply_params(target: &mut dyn Params, specs: &ParamMap, what: &str, warnings: &mut Vec<String>) -> Vec<(String, ParamSpec)> {
    let mut labels = Vec::new();
    target.visit_params(&mut |l, _| labels.push(l.to_lowercase()));
    let mut wanted: Vec<Option<&ParamSpec>> = vec![None; labels.len()];
    let mut unmatched = Vec::new();
    for (label, spec) in specs {
        let l = label.to_lowercase();
        match labels.iter().position(|x| *x == l).or_else(|| labels.iter().position(|x| x.starts_with(&l))) {
            Some(i) => wanted[i] = Some(spec),
            None => unmatched.push((label.clone(), spec.clone())),
        }
    }
    let mut i = 0;
    target.visit_params(&mut |label, p| {
        if let Some(spec) = wanted[i]
            && let Err(e) = apply_param(p, spec)
        {
            warnings.push(format!("{what} › {label}: {e}"));
        }
        i += 1;
    });
    unmatched
}

fn apply_param(p: &mut Param, spec: &ParamSpec) -> Result<(), String> {
    let (value, modulator) = match spec {
        ParamSpec::Value(v) => (Some(v), None),
        ParamSpec::Full { value, modulator } => (value.as_ref(), modulator.as_ref()),
    };
    match value {
        Some(Scalar::Num(v)) => p.set(*v),
        Some(Scalar::Name(n)) => {
            let i = p.spec.choices.iter().position(|c| c.eq_ignore_ascii_case(n)).ok_or_else(|| format!("no option {n:?} (have: {})", p.spec.choices.join(", ")))?;
            p.set(i as f32);
        }
        None => {}
    }
    if let Some(m) = modulator {
        p.modulator = Some(modulator_from(m, p.seed)?);
    }
    Ok(())
}

fn modulator_from(m: &ModSpec, seed: u64) -> Result<Modulator, String> {
    let mut out = Modulator::new(seed);
    if let Some(s) = &m.shape {
        out.shape = pick(&Shape::ALL, s, "modulator shape")?;
    }
    out.rate = match (m.beats, m.hz) {
        (_, Some(hz)) => Rate::Hz(hz),
        (Some(b), None) => Rate::Beats(b),
        (None, None) => out.rate,
    };
    if let Some(d) = m.depth {
        out.depth = d;
    }
    if let Some(p) = &m.polarity {
        out.polarity = pick(&Polarity::ALL, p, "polarity")?;
    }
    if let Some(p) = m.phase {
        out.phase = p;
    }
    if let Some(w) = m.width {
        out.width = w;
    }
    if let Some(p) = &m.points {
        out.points = p.clone();
        out.points.sort_by(|a, b| a[0].total_cmp(&b[0]));
    }
    if let Some(b) = &m.band {
        out.band = pick(&crate::audio::Band::ALL, b, "audio band")?;
    }
    if let Some(e) = m.enabled {
        out.enabled = e;
    }
    Ok(out)
}

fn mod_spec(m: &Modulator) -> ModSpec {
    let d = Modulator::new(m.seed);
    let (beats, hz) = match m.rate {
        Rate::Beats(b) => (Some(b), None),
        Rate::Hz(h) => (None, Some(h)),
    };
    ModSpec {
        shape: Some(key(m.shape)),
        beats,
        hz,
        depth: Some(m.depth),
        polarity: name_unless(m.polarity, d.polarity),
        phase: (m.phase != d.phase).then_some(m.phase),
        width: (m.shape == Shape::Square && m.width != d.width).then_some(m.width),
        points: (m.shape == Shape::Envelope).then(|| m.points.clone()),
        band: (m.shape == Shape::Audio).then(|| key(m.band)),
        enabled: (!m.enabled).then_some(false),
    }
}

fn param_spec(p: &Param) -> Option<ParamSpec> {
    let changed = p.value != p.spec.default;
    let value = changed.then(|| match p.spec.choices.get(p.value.round().max(0.0) as usize) {
        Some(name) => Scalar::Name(name.to_string()),
        None => Scalar::Num(p.value),
    });
    match (&p.modulator, value) {
        (None, None) => None,
        (None, Some(v)) => Some(ParamSpec::Value(v)),
        (Some(m), value) => Some(ParamSpec::Full { value, modulator: Some(mod_spec(m)) }),
    }
}

/// Every parameter that isn't at its default (or has a modulator).
fn collect_params(target: &mut dyn Params) -> ParamMap {
    let mut out = ParamMap::new();
    target.visit_params(&mut |label, p| {
        if let Some(s) = param_spec(p) {
            out.insert(label.to_string(), s);
        }
    });
    out
}

/// A shader compiles (and so gets its parameters) a frame or two after it's made: until then,
/// saved values wait in `initial`.
fn defer_shader_params(s: &mut CustomShader, rest: Vec<(String, ParamSpec)>, what: &str, warnings: &mut Vec<String>) {
    for (label, spec) in rest {
        let (value, modulator) = match spec {
            ParamSpec::Value(v) => (Some(v), None),
            ParamSpec::Full { value, modulator } => (value, modulator),
        };
        match value {
            Some(Scalar::Num(v)) => {
                s.initial.retain(|(n, _)| *n != label);
                s.initial.push((label.clone(), v));
            }
            Some(Scalar::Name(n)) => warnings.push(format!("{what} › {label}: use a number for {n:?} (the shader isn't compiled yet)")),
            None => {}
        }
        if let Some(m) = modulator {
            match modulator_from(&m, 0) {
                Ok(m) => s.initial_mods.push((label, m)),
                Err(e) => warnings.push(format!("{what} › {label}: {e}")),
            }
        }
    }
}

/// A shader's values: its parameters once compiled, else what's waiting to be applied.
fn shader_params(s: &mut CustomShader) -> ParamMap {
    let mut out = ParamMap::new();
    if s.params.is_empty() {
        for (label, v) in &s.initial {
            out.insert(label.clone(), ParamSpec::Value(Scalar::Num(*v)));
        }
        for (label, m) in &s.initial_mods {
            let value = s.initial.iter().find(|(n, _)| n == label).map(|(_, v)| Scalar::Num(*v));
            out.insert(label.clone(), ParamSpec::Full { value, modulator: Some(mod_spec(m)) });
        }
    }
    out
}

// ------------------------------------------------------------------ opening

/// Where a set's relative paths start.
#[derive(Clone, Debug)]
pub enum Base {
    /// A set bundled into the app (it lives in `sets/` in the repo).
    Bundled,
    /// The folder of the set's file.
    Dir(PathBuf),
}

/// What a media path in a set leads to.
enum Found {
    /// Compiled in (`MEDIA`), with its key.
    Bytes(&'static str, &'static [u8]),
    File(PathBuf),
}

/// Lexically resolve `.` and `..` (`sets/../samples/x.jpg` → `samples/x.jpg`).
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            c => out.push(c),
        }
    }
    out
}

fn builtin_media(key: &str) -> Option<(&'static str, &'static [u8])> {
    MEDIA.iter().find(|(k, _)| *k == key).copied()
}

/// A compiled-in file, by its path from the repo root.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn media_bytes(path: &Path) -> Option<&'static [u8]> {
    let p = normalize(path);
    let p = p.to_string_lossy().replace('\\', "/");
    MEDIA.iter().find(|(k, _)| p == *k || p.ends_with(&format!("/{k}"))).map(|(_, b)| *b)
}

/// Turns a set into a composition, collecting what couldn't be done as warnings.
struct Opener<'a> {
    library: &'a Library,
    base: Base,
    warnings: Vec<String>,
}

impl Opener<'_> {
    fn find(&self, file: &str) -> Result<Found, String> {
        if let Some(k) = file.strip_prefix(BUILTIN) {
            return builtin_media(k).map(|(k, b)| Found::Bytes(k, b)).ok_or_else(|| format!("{file} isn't built in"));
        }
        let missing = || Err(format!("{file}: not found"));
        match &self.base {
            Base::Bundled => {
                // Bundled sets are in `sets/`: try the compiled-in media, then the repo (from
                // the current folder, or where the app was built).
                let rel = normalize(&Path::new("sets").join(file));
                let k = rel.to_string_lossy().replace('\\', "/");
                if let Some((k, b)) = builtin_media(&k) {
                    return Ok(Found::Bytes(k, b));
                }
                let roots = [std::env::current_dir().unwrap_or_default(), PathBuf::from(env!("CARGO_MANIFEST_DIR"))];
                match roots.iter().map(|r| r.join(&rel)).find(|p| p.exists()) {
                    Some(p) => Ok(Found::File(p)),
                    None => missing(),
                }
            }
            Base::Dir(dir) => {
                let p = normalize(&dir.join(file));
                if p.exists() {
                    return Ok(Found::File(p));
                }
                // A set saved next to a copy of the repo's logos or stills still finds them.
                let s = p.to_string_lossy().replace('\\', "/");
                match MEDIA.iter().find(|(k, _)| s.ends_with(&format!("/{k}"))) {
                    Some((k, b)) => Ok(Found::Bytes(k, b)),
                    None => missing(),
                }
            }
        }
    }

    /// A library entry of `kind` by key, or by name like `isf_library::find_by_name` (exact,
    /// then prefix, then part; `category/name` works too).
    fn library_entry(&self, name: &str, kind: Kind) -> Result<&crate::isf_library::Entry, String> {
        let want = name.trim().to_lowercase();
        let pool = || self.library.entries.iter().filter(|e| e.kind == kind);
        let full = |e: &crate::isf_library::Entry| format!("{}/{}", e.category, e.name).to_lowercase();
        pool()
            .find(|e| e.key == name)
            .or_else(|| pool().find(|e| e.name.to_lowercase() == want || full(e) == want))
            .or_else(|| pool().find(|e| e.name.to_lowercase().starts_with(&want) || full(e).starts_with(&want)))
            .or_else(|| pool().find(|e| e.name.to_lowercase().contains(&want)))
            .ok_or_else(|| match kind {
                Kind::Model => format!("no library model {name:?}"),
                Kind::Effect => format!("no library effect {name:?}"),
                _ => format!("no library generator {name:?}"),
            })
    }

    fn model(&self, name: &str) -> Result<crate::model::ModelRef, String> {
        let e = self.library_entry(name, Kind::Model)?;
        self.library.model(&e.key)
    }

    fn shader_file(&self, file: &str, role: Role) -> Result<CustomShader, String> {
        match self.find(file)? {
            Found::File(p) => CustomShader::from_file(&p, role),
            Found::Bytes(k, _) => Err(format!("{k} isn't a shader")),
        }
    }

    fn file_clip(&self, file: &str, fill: Option<f32>) -> Result<Clip, String> {
        let found = self.find(file)?;
        let path = match &found {
            Found::Bytes(k, _) => PathBuf::from(format!("{BUILTIN}{k}")),
            Found::File(p) => p.clone(),
        };
        let svg = source::is_svg(&path);
        let mut clip = match found {
            Found::File(p) if !svg => Clip::open(&p, MEDIA_WIDTH, MEDIA_HEIGHT)?,
            Found::Bytes(k, b) if !svg => Clip::from_bytes(k.rsplit('/').next().unwrap_or(k), b)?,
            found => {
                let bytes = match found {
                    Found::Bytes(_, b) => b.to_vec(),
                    Found::File(p) => std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?,
                };
                let frame = source::render_svg(&bytes, MEDIA_WIDTH, MEDIA_HEIGHT, fill.unwrap_or(source::SVG_FILL)).map_err(|e| format!("{file}: {e}"))?;
                let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                let mut c = Clip::image(&stem, frame);
                c.fill = fill;
                c
            }
        };
        clip.file = Some(path);
        Ok(clip)
    }

    fn clip(&mut self, spec: &ClipSpec, what: &str) -> Result<Clip, String> {
        let mut clip = match self.clip_media(spec) {
            Ok(c) => c,
            Err(e) => match &spec.fallback {
                Some(f) => return self.clip(f, what),
                None => return Err(e),
            },
        };
        if let Some(n) = &spec.name {
            clip.name = n.clone();
        }
        let what = format!("{what} › {}", clip.name);
        let mut warn = |e: String| self.warnings.push(format!("{what}: {e}"));
        if let Some(s) = &spec.loop_mode {
            pick(&LoopMode::ALL, s, "loop mode").map(|v| clip.loop_mode = v).unwrap_or_else(&mut warn);
        }
        if let Some(s) = &spec.direction {
            pick(&DIRECTIONS, s, "direction").map(|v| clip.direction = v).unwrap_or_else(&mut warn);
        }
        if let Some(s) = &spec.sync {
            pick(&SYNCS, s, "sync").map(|v| clip.sync = v).unwrap_or_else(&mut warn);
        }
        if let Some(s) = &spec.fit {
            pick(&Fit::ALL, s, "fit").map(|v| clip.fit = v).unwrap_or_else(&mut warn);
        }
        let rest = apply_params(&mut clip, &spec.params, &what, &mut self.warnings);
        match &mut clip.media {
            Media::Shader(s) => defer_shader_params(s, rest, &what, &mut self.warnings),
            _ => self.unknown(&what, rest),
        }
        Ok(clip)
    }

    fn clip_media(&self, spec: &ClipSpec) -> Result<Clip, String> {
        if let Some(f) = &spec.file {
            return self.file_clip(f, spec.fill);
        }
        if let Some(g) = &spec.generator {
            let i = PATTERNS.iter().position(|p| p.eq_ignore_ascii_case(g)).ok_or_else(|| format!("no generator {g:?} (have: {})", PATTERNS.join(", ")))?;
            return Ok(Clip::generator(i));
        }
        if let Some(s) = &spec.shader {
            let mut c = CustomShader::new(&s.name, &s.code, Role::Source);
            c.vertex = s.vertex.clone();
            return Ok(Clip::from_shader(c));
        }
        if let Some(name) = &spec.library {
            return Ok(Clip::from_shader(self.library_entry(name, Kind::Generator)?.shader()?));
        }
        if let Some(m) = &spec.model {
            return Ok(Clip::model(self.model(m)?));
        }
        if let Some(i) = spec.camera {
            if crate::WEB {
                return Err("cameras need the desktop app".into());
            }
            return Clip::camera(i, MEDIA_WIDTH, MEDIA_HEIGHT);
        }
        Err("a clip needs a file, generator, shader, library, model or camera".into())
    }

    fn effect(&mut self, spec: &EffectSpec, what: &str) -> Result<Effect, String> {
        let code = spec.shader.is_some() || spec.library.is_some() || spec.file.is_some();
        let kind = match &spec.kind {
            Some(k) => pick(&effect_kinds(), k, "effect")?,
            None if code => EffectKind::Shader,
            None => return Err("an effect needs a kind".into()),
        };
        let mut e = Effect::new(kind);
        if kind == EffectKind::Shader {
            let s = match (&spec.shader, &spec.library, &spec.file) {
                (Some(s), _, _) => {
                    let mut c = CustomShader::new(&s.name, &s.code, Role::Effect);
                    c.vertex = s.vertex.clone();
                    c
                }
                (None, Some(name), _) => self.library_entry(name, Kind::Effect)?.shader()?,
                (None, None, Some(f)) => self.shader_file(f, Role::Effect)?,
                _ => return Err("a shader effect needs a shader, library or file".into()),
            };
            e.custom = Some(Box::new(s));
        }
        let what = format!("{what} › {}", e.name());
        if let Some(p) = &spec.preset {
            match FEEDBACK_PRESETS.iter().position(|n| n.eq_ignore_ascii_case(p)) {
                Some(i) if kind == EffectKind::Feedback => apply_feedback_preset(&mut e, i),
                _ => self.warnings.push(format!("{what}: no preset {p:?} (Feedback has: {})", FEEDBACK_PRESETS.join(", "))),
            }
        }
        if let Some(m) = &spec.model {
            match self.model(m) {
                // This switches the shape to Model; a `shape` in the params says otherwise.
                Ok(m) if e.takes_model() => e.set_model(m),
                Ok(_) => self.warnings.push(format!("{what}: only the Shape projector and Projection mapping take a model")),
                Err(err) => self.warnings.push(format!("{what}: {err}")),
            }
        }
        e.enabled = spec.enabled.unwrap_or(true);
        e.half_history = spec.half_history;
        let rest = apply_params(&mut e, &spec.params, &what, &mut self.warnings);
        match e.custom.as_deref_mut() {
            Some(s) => defer_shader_params(s, rest, &what, &mut self.warnings),
            None => self.unknown(&what, rest),
        }
        Ok(e)
    }

    fn effects(&mut self, specs: &[EffectSpec], what: &str) -> Vec<Effect> {
        let mut out = Vec::new();
        for (i, s) in specs.iter().enumerate() {
            match self.effect(s, what) {
                Ok(e) => out.push(e),
                Err(e) => self.warnings.push(format!("{what} › effect {}: {e}", i + 1)),
            }
        }
        out
    }

    fn unknown(&mut self, what: &str, rest: Vec<(String, ParamSpec)>) {
        for (label, _) in rest {
            self.warnings.push(format!("{what}: no parameter {label:?}"));
        }
    }
}

/// Build the composition a set describes. Whatever can't be opened (missing media, unknown
/// effects or parameters) is left out and reported.
pub fn open(set: &SetFile, library: &Library, base: Base) -> (Composition, Vec<String>) {
    let mut o = Opener { library, base, warnings: Vec::new() };
    if set.tripslop > VERSION {
        o.warnings.push(format!("this set is format version {}, newer than this tripslop ({VERSION}): some of it may be missing", set.tripslop));
    }
    let columns = set.layers.iter().map(|l| l.clips.len()).chain([set.scenes.len(), set.columns.unwrap_or(8), 1]).max().unwrap_or(1);
    let mut comp = Composition::new(0, columns);
    comp.scenes = set.scenes.clone();
    if let Some(b) = set.bpm {
        comp.bpm = b.clamp(30.0, 300.0);
    }
    let warn = |w: &mut Vec<String>, r: Result<(), String>| {
        if let Err(e) = r {
            w.push(e);
        }
    };
    if let Some(q) = &set.quantize {
        let r = pick(&Quantize::ALL, q, "quantize").map(|v| comp.quantize = v);
        warn(&mut o.warnings, r);
    }
    if let Some(c) = &set.crossfade {
        let r = pick(&Crossfade::ALL, c, "crossfade").map(|v| comp.crossfade = v);
        warn(&mut o.warnings, r);
    }
    if let Some(c) = &set.fade_curve {
        let r = pick(&FadeCurve::ALL, c, "fade curve").map(|v| comp.fade_curve = v);
        warn(&mut o.warnings, r);
    }
    let rest = apply_params(&mut MasterParams(&mut comp), &set.params, "Master", &mut o.warnings);
    o.unknown("Master", rest);
    comp.effects = o.effects(&set.effects, "Master");
    for (li, spec) in set.layers.iter().enumerate() {
        comp.add_layer();
        let mut layer = std::mem::replace(&mut comp.layers[li], crate::composition::Layer::new(String::new(), 0));
        if !spec.name.is_empty() {
            layer.name = spec.name.clone();
        }
        let what = layer.name.clone();
        if let Some(c) = spec.color {
            layer.color = c;
        }
        if let Some(b) = &spec.blend {
            let r = pick(&Blend::ALL, b, "blend").map(|v| layer.blend = v);
            warn(&mut o.warnings, r);
        }
        if let Some(s) = &spec.side {
            let r = pick(&SIDES, s, "side").map(|v| layer.side = v);
            warn(&mut o.warnings, r);
        }
        layer.bypass = spec.bypass;
        layer.solo = spec.solo;
        let rest = apply_params(&mut layer, &spec.params, &what, &mut o.warnings);
        o.unknown(&what, rest);
        layer.effects = o.effects(&spec.effects, &what);
        for (col, c) in spec.clips.iter().enumerate() {
            let Some(c) = c else { continue };
            match o.clip(c, &format!("{what} › scene {}", col + 1)) {
                Ok(clip) => layer.clips[col] = Some(clip),
                Err(e) => o.warnings.push(format!("{what} › scene {}: {e}", col + 1)),
            }
        }
        comp.layers[li] = layer;
    }
    if comp.layers.is_empty() {
        comp.add_layer();
    }
    (comp, o.warnings)
}

/// The scene a set starts on (0-based), if any.
pub fn start_scene(set: &SetFile, comp: &Composition) -> Option<usize> {
    match set.start {
        Some(0) => None,
        Some(n) => Some(n - 1).filter(|c| *c < comp.columns),
        None => Some(0),
    }
}

/// The composition's own parameters, under the labels a set uses.
struct MasterParams<'a>(&'a mut Composition);

impl Params for MasterParams<'_> {
    fn visit_params(&mut self, f: &mut dyn FnMut(&str, &mut Param)) {
        f("master", &mut self.0.master);
        f("crossfader", &mut self.0.crossfader);
    }
}

// ------------------------------------------------------------------ saving

/// `path` for a set in `dir`: relative when it's nearby, else absolute.
fn relative(path: &Path, dir: Option<&Path>) -> String {
    let s = |p: &Path| p.to_string_lossy().replace('\\', "/");
    if path.to_string_lossy().starts_with(BUILTIN) {
        return s(path);
    }
    let abs = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| normalize(&std::env::current_dir().unwrap_or_default().join(p)));
    let Some(dir) = dir else { return s(&abs(path)) };
    let (path, dir) = (abs(path), abs(dir));
    let common = path.components().zip(dir.components()).take_while(|(a, b)| a == b).count();
    let ups = dir.components().count() - common;
    // Up to two folders up (a set next to its media folder, or in a sibling folder).
    if common <= 1 || ups > 2 {
        return s(&path);
    }
    let mut rel = PathBuf::new();
    for _ in 0..ups {
        rel.push("..");
    }
    rel.extend(path.components().skip(common));
    s(&rel)
}

/// Describe the composition as a set; `dir` is where it will be saved (paths are made relative
/// to it). Clips that can't be saved (an image dropped into the browser, with no file) are
/// left out and reported.
pub fn capture(comp: &mut Composition, name: &str, description: &str, dir: Option<&Path>) -> (SetFile, Vec<String>) {
    let mut warnings = Vec::new();
    let mut set = SetFile {
        tripslop: VERSION,
        name: name.to_string(),
        description: description.to_string(),
        bpm: Some(comp.bpm),
        quantize: name_unless(comp.quantize, Quantize::Off),
        crossfade: name_unless(comp.crossfade, Crossfade::Bank),
        fade_curve: name_unless(comp.fade_curve, FadeCurve::Linear),
        columns: Some(comp.columns),
        scenes: comp.scenes.clone(),
        start: Some(comp.active_column.map_or(0, |c| c + 1)),
        params: collect_params(&mut MasterParams(comp)),
        effects: comp.effects.iter_mut().map(effect_spec).collect(),
        layers: Vec::new(),
    };
    while set.scenes.last().is_some_and(|s| s.trim().is_empty()) {
        set.scenes.pop();
    }
    if set.start == Some(1) {
        set.start = None;
    }
    for (li, l) in comp.layers.iter_mut().enumerate() {
        let mut clips: Vec<Option<ClipSpec>> = l
            .clips
            .iter_mut()
            .map(|c| {
                let c = c.as_mut()?;
                let spec = clip_spec(c, dir);
                if spec.is_none() {
                    warnings.push(format!("{} › {}: left out (it isn't from a file)", l.name, c.name));
                }
                spec
            })
            .collect();
        while clips.last().is_some_and(|c| c.is_none()) {
            clips.pop();
        }
        set.layers.push(LayerSpec {
            name: l.name.clone(),
            color: (l.color != li).then_some(l.color),
            blend: name_unless(l.blend, Blend::Normal),
            side: name_unless(l.side, Side::Both),
            bypass: l.bypass,
            solo: l.solo,
            params: collect_params(l),
            effects: l.effects.iter_mut().map(effect_spec).collect(),
            clips,
        });
    }
    (set, warnings)
}

fn shader_spec(s: &CustomShader) -> ShaderSpec {
    ShaderSpec { name: s.name.clone(), code: s.source.clone(), vertex: s.vertex.clone() }
}

fn effect_spec(e: &mut Effect) -> EffectSpec {
    let mut params = collect_params(e);
    let shader = e.custom.as_deref_mut().map(|s| {
        params.extend(shader_params(s));
        shader_spec(s)
    });
    // A model switches the shape to Model when it's opened: say what it really is.
    if e.model.is_some()
        && let Some(shape) = e.shape()
    {
        params.entry("shape".into()).or_insert(ParamSpec::Value(Scalar::Name(shape.into())));
    }
    EffectSpec {
        kind: (e.kind != EffectKind::Shader).then(|| key(e.kind)),
        enabled: (!e.enabled).then_some(false),
        half_history: e.half_history,
        preset: None,
        shader,
        library: None,
        file: None,
        model: e.model.as_ref().map(|m| m.key.clone()),
        params,
    }
}

fn clip_spec(c: &mut Clip, dir: Option<&Path>) -> Option<ClipSpec> {
    let mut spec = ClipSpec {
        name: Some(c.name.clone()),
        loop_mode: name_unless(c.loop_mode, LoopMode::Loop),
        direction: name_unless(c.direction, Direction::Forward),
        sync: name_unless(c.sync, Sync::Timeline),
        fit: name_unless(c.fit, Fit::Fill),
        params: collect_params(c),
        ..Default::default()
    };
    match &mut c.media {
        Media::Video(v) => spec.file = Some(relative(c.file.as_deref().unwrap_or(&v.path), dir)),
        Media::Image { .. } => {
            spec.file = Some(relative(c.file.as_deref()?, dir));
            spec.fill = c.fill;
        }
        Media::Camera { index, .. } => spec.camera = Some(*index),
        Media::Generator(g) => {
            spec.generator = Some(PATTERNS[g.pattern.value.round() as usize % PATTERNS.len()].to_lowercase());
            if matches!(spec.params.get("pattern"), Some(ParamSpec::Value(_))) {
                spec.params.remove("pattern");
            }
        }
        Media::Shader(s) => {
            spec.params.extend(shader_params(s));
            spec.shader = Some(shader_spec(s));
        }
        Media::Model(m) => spec.model = Some(m.model.key.clone()),
    }
    Some(spec)
}

// ------------------------------------------------------------------ the bundled sets

/// A set bundled into the app (`sets/`).
pub struct Builtin {
    /// File name without the order number: `public-access`.
    pub id: &'static str,
    pub set: SetFile,
    /// The welcome screen's picture (JPEG).
    pub thumb: Option<&'static [u8]>,
}

/// The bundled sets, in order.
pub fn builtins() -> Vec<Builtin> {
    bundle::SETS
        .iter()
        .map(|(stem, json, thumb)| Builtin {
            id: stem.trim_start_matches(|c: char| c.is_ascii_digit() || c == '-'),
            set: parse(json).unwrap_or_else(|e| SetFile { name: format!("{stem} (broken: {e})"), ..Default::default() }),
            thumb: *thumb,
        })
        .collect()
}

/// A bundled set by id or name.
pub fn builtin(name: &str) -> Option<Builtin> {
    let want = name.trim().to_lowercase();
    builtins().into_iter().find(|b| b.id == want || b.set.name.to_lowercase() == want)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library() -> Library {
        Library::new(Vec::new())
    }

    #[test]
    fn bundled_sets_open_cleanly() {
        let lib = library();
        let sets = builtins();
        assert!(sets.len() >= 4, "only {} bundled sets", sets.len());
        for b in sets {
            assert!(!b.set.name.is_empty() && !b.set.name.contains("broken"), "{}: {}", b.id, b.set.name);
            assert!(!b.set.description.is_empty(), "{} has no description", b.id);
            let (comp, warnings) = open(&b.set, &lib, Base::Bundled);
            assert!(warnings.is_empty(), "{}: {warnings:#?}", b.id);
            assert!(!comp.layers.is_empty());
            assert!(b.set.scenes.len() >= 2, "{} names its scenes", b.id);
        }
    }

    #[test]
    fn enum_names_are_snake_case() {
        assert_eq!(key(LoopMode::PlayOnceHold), "play_once_hold");
        assert_eq!(key(EffectKind::RgbSplit), "rgb_split");
        assert_eq!(pick(&LoopMode::ALL, "Play once-hold", "loop").unwrap(), LoopMode::PlayOnceHold);
        assert!(pick(&Blend::ALL, "glow", "blend").unwrap_err().contains("screen"));
    }

    #[test]
    fn round_trip_keeps_values_and_modulators() {
        let lib = library();
        let json = r#"{
            "tripslop": 1, "name": "t", "bpm": 97, "quantize": "bar", "scenes": ["One", "Two"],
            "params": { "crossfader": 0.25 },
            "effects": [ { "kind": "crt", "params": { "noise": 0.5 } } ],
            "layers": [
                { "name": "Base", "blend": "screen", "side": "a",
                  "params": { "opacity": { "value": 0.5, "mod": { "shape": "sine", "beats": 4, "depth": 0.2 } }, "scale": 0.4 },
                  "effects": [
                      { "kind": "transform", "params": { "tile": "Repeat", "x": { "mod": { "shape": "saw_up", "beats": 16, "depth": 0.5 } } } },
                      { "kind": "feedback", "preset": "Sierpinski", "enabled": false }
                  ],
                  "clips": [ { "generator": "plasma", "params": { "hue": 0.3 } }, null, { "generator": "dot" } ] },
                { "clips": [ null, { "file": "builtin:logos/tripslop-flower-color.svg", "fill": 0.5, "fit": "contain" } ] }
            ]
        }"#;
        let set = parse(json).unwrap();
        let (mut comp, warnings) = open(&set, &lib, Base::Bundled);
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(comp.columns, 8);
        assert_eq!(comp.bpm, 97.0);
        assert_eq!(comp.quantize, Quantize::Bar);
        assert_eq!(comp.scene_name(1), Some("Two"));
        let l = &comp.layers[0];
        assert_eq!((l.blend, l.side), (Blend::Screen, Side::A));
        assert_eq!(l.opacity.value, 0.5);
        assert_eq!(l.opacity.modulator.as_ref().unwrap().rate, Rate::Beats(4.0));
        assert_eq!(l.effects[0].params[4].value, 1.0, "tile = Repeat");
        assert!(!l.effects[1].enabled);
        assert_eq!(l.effects[1].params[1].value, 3.0, "Sierpinski's copies");
        assert!(l.clips[1].is_none());
        assert_eq!(comp.layers[1].clips[1].as_ref().unwrap().fit, Fit::Contain);

        let (saved, warnings) = capture(&mut comp, "t", "", None);
        assert!(warnings.is_empty(), "{warnings:?}");
        let json = to_json(&saved);
        let (mut again, warnings) = open(&parse(&json).unwrap(), &lib, Base::Bundled);
        assert!(warnings.is_empty(), "{warnings:?}");
        let values = |c: &mut Composition| {
            let mut v = Vec::new();
            c.visit_paths(&mut |path, p| v.push((path.to_string(), p.value, p.modulator.as_ref().map(|m| (m.shape, m.depth)))));
            v
        };
        assert_eq!(values(&mut comp), values(&mut again));
        assert_eq!(to_json(&capture(&mut again, "t", "", None).0), json, "saving again changes nothing");
    }

    #[test]
    fn unknown_things_are_reported_not_fatal() {
        let lib = library();
        let json = r#"{ "layers": [ {
            "params": { "wobble": 1 },
            "effects": [ { "kind": "warp drive" }, { "kind": "blur", "params": { "radius": 3, "sparkle": 1 } } ],
            "clips": [ { "file": "nope.mp4" }, { "file": "nope.mp4", "fallback": { "generator": "rings" } } ] } ] }"#;
        let (comp, warnings) = open(&parse(json).unwrap(), &lib, Base::Dir(std::env::temp_dir()));
        assert_eq!(warnings.len(), 4, "{warnings:#?}");
        let l = &comp.layers[0];
        assert_eq!(l.effects.len(), 1);
        assert_eq!(l.effects[0].params[0].value, 3.0);
        assert!(l.clips[0].is_none());
        assert_eq!(l.clips[1].as_ref().unwrap().name, "Rings");
    }

    #[test]
    fn relative_paths() {
        assert_eq!(relative(Path::new("/a/b/media/x.mp4"), Some(Path::new("/a/b"))), "media/x.mp4");
        assert_eq!(relative(Path::new("/a/b/media/x.mp4"), Some(Path::new("/a/b/sets"))), "../media/x.mp4");
        assert_eq!(relative(Path::new("/x/y/z.jpg"), Some(Path::new("/a/b/c"))), "/x/y/z.jpg");
        assert_eq!(relative(Path::new("builtin:logos/a.svg"), Some(Path::new("/a"))), "builtin:logos/a.svg");
        assert_eq!(normalize(Path::new("sets/../samples/x.jpg")), PathBuf::from("samples/x.jpg"));
    }
}

//! Runs `--script` events against the live app (see `script.rs` for the language).

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use crate::clip::{Clip, PATTERNS};
use crate::composition::{Launch, Quantize};
use crate::effects::{EFFECTS, Effect, EffectKind, apply_feedback_preset};
use crate::param::{Param, Params};
use crate::script::{Cmd, Event, Query, When};
use crate::renderer::{MEDIA_HEIGHT, MEDIA_WIDTH};
use crate::{App, Tab};

/// Set when a script command fails or an assert doesn't hold; `main` exits with status 1.
pub static FAILED: AtomicBool = AtomicBool::new(false);
static ASSERTS: AtomicUsize = AtomicUsize::new(0);
static ASSERTS_FAILED: AtomicUsize = AtomicUsize::new(0);

pub fn summary() -> Option<String> {
    let n = ASSERTS.load(Ordering::Relaxed);
    (n > 0).then(|| format!("script: {n} assert(s), {} failed", ASSERTS_FAILED.load(Ordering::Relaxed)))
}

/// A script event and whether it has run.
pub struct Pending {
    pub event: Event,
    pub done: bool,
}

/// Case-insensitive match: exact label first, then prefix ("rotate" finds "rotate °").
fn best_match(labels: &[String], want: &str) -> Option<usize> {
    let want = want.to_lowercase();
    labels
        .iter()
        .position(|l| *l == want)
        .or_else(|| labels.iter().position(|l| l.starts_with(&want)))
}

/// Run `f` on the parameter labelled `seg` in `target`.
fn on_param(target: &mut dyn Params, seg: &str, f: impl FnOnce(&mut Param)) -> Result<(), String> {
    let mut labels = Vec::new();
    target.visit_params(&mut |l, _| labels.push(l.to_lowercase()));
    let i = best_match(&labels, seg).ok_or_else(|| format!("no parameter {seg:?} (have: {})", labels.join(", ")))?;
    let mut f = Some(f);
    let mut k = 0;
    target.visit_params(&mut |_, p| {
        if k == i
            && let Some(f) = f.take()
        {
            f(p);
        }
        k += 1;
    });
    Ok(())
}

/// Effect by 1-based `fxN` or by (prefix of) its name.
fn find_effect<'a>(effects: &'a mut [Effect], seg: &str) -> Result<&'a mut Effect, String> {
    if let Some(n) = seg.strip_prefix("fx").and_then(|n| n.parse::<usize>().ok()) {
        let len = effects.len();
        return effects.get_mut(n.wrapping_sub(1)).ok_or_else(|| format!("no effect {seg} (there are {len})"));
    }
    let names: Vec<String> = effects.iter().map(|e| e.name().to_lowercase()).collect();
    let i = best_match(&names, seg).ok_or_else(|| format!("no effect {seg:?} (have: {})", names.join(", ")))?;
    Ok(&mut effects[i])
}

fn show_when(w: When) -> String {
    match w {
        When::Frame(f) => format!("{f}"),
        When::Seconds(s) => format!("{s}s"),
        When::Beat(b) => format!("{b}b"),
    }
}

impl App {
    /// Run every script event that's due. Called at the start of each tick.
    pub(crate) fn run_script(&mut self) {
        for i in 0..self.script.len() {
            if self.script[i].done || !self.due(self.script[i].event.at) {
                continue;
            }
            self.script[i].done = true;
            let ev = self.script[i].event.clone();
            if let Err(e) = self.exec(&ev.cmd) {
                eprintln!("script line {} (at {}): {e}", ev.line, show_when(ev.at));
                FAILED.store(true, Ordering::Relaxed);
            }
        }
    }

    fn due(&self, w: When) -> bool {
        match w {
            When::Frame(f) => self.frame_count >= f,
            When::Seconds(s) => self.sim_time + 1e-9 >= s,
            When::Beat(b) => self.beat + 1e-9 >= b,
        }
    }

    fn stamp(&self) -> String {
        format!("[f{} b{:.2}]", self.frame_count, self.beat)
    }

    /// Layers are created on demand so scripts can address layer 4 of a 3-layer set.
    fn ensure_layer(&mut self, l: usize) {
        while self.comp.layers.len() <= l {
            self.comp.add_layer();
        }
    }

    fn exec(&mut self, cmd: &Cmd) -> Result<(), String> {
        match cmd {
            Cmd::Demo => self.load_demo(),
            Cmd::LaunchScene(c) => {
                if *c >= self.comp.columns {
                    return Err(format!("no scene {}", c + 1));
                }
                self.comp.launch(Launch::Column(*c));
            }
            Cmd::Launch(l, c) => {
                if self.comp.clip_mut(*l, *c).is_none() {
                    return Err(format!("no clip at layer {} column {}", l + 1, c + 1));
                }
                self.comp.launch(Launch::Clip { layer: *l, col: *c });
            }
            Cmd::Stop(l) => self.comp.layers.get_mut(*l).ok_or("no such layer")?.clear(),
            Cmd::Load(l, c, path) => {
                self.ensure_layer(*l);
                self.try_load(*l, *c, path)?;
            }
            Cmd::Generator(l, c, name) => {
                let i = PATTERNS
                    .iter()
                    .position(|p| p.eq_ignore_ascii_case(name))
                    .ok_or_else(|| format!("no generator {name:?} (have: {})", PATTERNS.join(", ")))?;
                self.ensure_layer(*l);
                self.comp.set_clip(*l, *c, Clip::generator(i));
            }
            Cmd::Shader(l, c, name) => {
                let (n, _, code) = template(name)?;
                self.ensure_layer(*l);
                self.comp.set_clip(*l, *c, Clip::shader(n, code));
            }
            Cmd::Camera(l, c, index) => {
                self.ensure_layer(*l);
                self.comp.set_clip(*l, *c, Clip::camera(*index, MEDIA_WIDTH, MEDIA_HEIGHT)?);
            }
            Cmd::Isf(l, c, name) => {
                let entry = self.library.find_by_name(name).ok_or_else(|| format!("no library shader {name:?}"))?;
                if entry.kind != crate::isf_library::Kind::Generator {
                    return Err(format!("{} is an effect; use add-effect L isf:NAME", entry.name));
                }
                let clip = Clip::from_shader(entry.shader()?);
                self.ensure_layer(*l);
                self.comp.set_clip(*l, *c, clip);
            }
            Cmd::Automate(path, None) => self.with_param(path, |p| p.modulator = None)?,
            Cmd::Automate(path, Some((shape, band, depth, beats))) => self.with_param(path, |p| {
                let mut m = crate::modulation::Modulator::lfo(p.seed, *shape, *beats, *depth);
                if let Some(b) = band {
                    m.band = *b;
                    m.polarity = crate::modulation::Polarity::Up;
                }
                p.modulator = Some(m);
            })?,
            Cmd::Audio(None) => self.audio.stop(),
            Cmd::Audio(Some(path)) => self.audio.open_file(path, self.sim_time)?,
            Cmd::Library(effects, category) => {
                use crate::isf_library::Kind;
                self.show_library = true;
                self.library_view.kind = if *effects { Kind::Effect } else { Kind::Generator };
                self.library_view.category = match category {
                    None => None,
                    Some(c) => Some(
                        self.library
                            .categories
                            .iter()
                            .find(|k| k.eq_ignore_ascii_case(c))
                            .ok_or_else(|| format!("no library category {c:?} (have: {})", self.library.categories.join(", ")))?
                            .clone(),
                    ),
                };
            }
            Cmd::AddEffect(target, name) => {
                let e = match (name.strip_prefix("shader:"), name.strip_prefix("file:")) {
                    _ if name.starts_with("isf:") => {
                        let entry = self.library.find_by_name(&name[4..]).ok_or_else(|| format!("no library shader {:?}", &name[4..]))?;
                        if entry.kind != crate::isf_library::Kind::Effect {
                            return Err(format!("{} is a generator; use isf L C NAME", entry.name));
                        }
                        let mut e = Effect::new(EffectKind::Shader);
                        e.custom = Some(Box::new(entry.shader()?));
                        e
                    }
                    (Some(t), _) => {
                        let (n, _, code) = template(t)?;
                        Effect::custom(n, code)
                    }
                    (_, Some(path)) => {
                        let mut e = Effect::new(EffectKind::Shader);
                        e.custom = Some(Box::new(crate::shader::CustomShader::from_file(std::path::Path::new(path), crate::shader::Role::Effect)?));
                        e
                    }
                    (None, None) => {
                        let names: Vec<String> = EFFECTS.iter().map(|d| d.name.to_lowercase()).collect();
                        let i = best_match(&names, name).ok_or_else(|| format!("no effect {name:?} (have: {})", names.join(", ")))?;
                        let mut e = Effect::new(EFFECTS[i].kind);
                        if e.kind == EffectKind::Feedback {
                            apply_feedback_preset(&mut e, 0);
                        }
                        e
                    }
                };
                match target {
                    None => self.comp.effects.push(e),
                    Some(l) => {
                        self.ensure_layer(*l);
                        self.comp.layers[*l].effects.push(e);
                    }
                }
            }
            Cmd::Set(path, v) => self.with_param(path, |p| p.set(*v))?,
            Cmd::Bpm(b) => self.comp.bpm = b.clamp(30.0, 300.0),
            Cmd::Quantize(q) => {
                self.comp.quantize = match q.as_str() {
                    "off" | "now" => Quantize::Off,
                    "beat" => Quantize::Beat,
                    "bar" => Quantize::Bar,
                    _ => return Err(format!("quantize {q:?}: use off / beat / bar")),
                }
            }
            Cmd::Crossfade(mode, curve) => {
                self.comp.crossfade = *mode;
                if let Some(c) = curve {
                    self.comp.fade_curve = *c;
                }
            }
            Cmd::Size(w, h) => self.set_size((*w, *h))?,
            Cmd::History(path, half) => self.with_effect(path, |e| e.half_history = *half)?,
            Cmd::Side(l, side) => {
                self.ensure_layer(*l);
                self.comp.layers[*l].side = *side;
            }
            Cmd::Play(p) => self.comp.playing = *p,
            Cmd::PadDown(i) => self.punch.pads[*i].script_held = true,
            Cmd::PadUp(i) => self.punch.pads[*i].script_held = false,
            Cmd::PadLatch(i, on) => self.punch.pads[*i].latched = *on,
            Cmd::Select(l, c) => {
                self.grid.selected_layer = *l;
                self.grid.selected_clip = Some((*l, *c));
            }
            Cmd::OpenEditor(l, c) => {
                let clip = self.comp.clip_mut(*l, *c).ok_or("no clip there")?;
                match &clip.media {
                    crate::clip::Media::Shader(s) => self.editing = Some(s.id),
                    _ => return Err("that clip isn't a shader".into()),
                }
            }
            Cmd::Tab(t) => {
                self.tab = match t.as_str() {
                    "layer" => Tab::Layer,
                    "composition" => Tab::Composition,
                    _ => return Err(format!("tab {t:?}: use layer / composition")),
                }
            }
            Cmd::Snapshot(p) => {
                make_parent(p)?;
                self.pending_snapshot = Some(p.clone());
            }
            Cmd::Screenshot(p) => {
                make_parent(p)?;
                self.pending_screenshot = Some(p.clone());
            }
            Cmd::Record(start) => {
                if *start != self.recorder.is_some() {
                    self.toggle_recording();
                }
            }
            Cmd::Print(q) => {
                let v = self.query(q)?;
                println!("{} {q:?} = {v}", self.stamp());
            }
            Cmd::Assert(q, op, want) => {
                let got = self.query(q)?;
                ASSERTS.fetch_add(1, Ordering::Relaxed);
                if op.check(got, *want) {
                    println!("{} ok   {q:?} {op:?} {want} (got {got})", self.stamp());
                } else {
                    ASSERTS_FAILED.fetch_add(1, Ordering::Relaxed);
                    FAILED.store(true, Ordering::Relaxed);
                    eprintln!("{} FAIL {q:?} {op:?} {want} (got {got})", self.stamp());
                }
            }
            Cmd::Quit => self.quit_requested = true,
        }
        Ok(())
    }

    fn query(&mut self, q: &Query) -> Result<f64, String> {
        Ok(match q {
            Query::Param(path) => {
                let mut v = 0.0;
                self.with_param(path, |p| v = p.live as f64)?;
                v
            }
            Query::Playhead(l) => self
                .comp
                .layers
                .get(*l)
                .and_then(|l| l.active_clip())
                .map(|c| c.position)
                .ok_or("no clip playing on that layer")?,
            Query::Active(l) => self.comp.layers.get(*l).ok_or("no such layer")?.active.map_or(0.0, |c| c as f64 + 1.0),
            Query::Pad(i) => self.punch.pads[*i].amount as f64,
            Query::Errors(l, c) => match &self.comp.clip_mut(*l, *c).ok_or("no clip there")?.media {
                crate::clip::Media::Shader(s) => {
                    for e in &s.errors {
                        eprintln!("  shader error: {}{}", e.line.map(|l| format!("line {l}: ")).unwrap_or_default(), e.message);
                    }
                    s.errors.len() as f64
                }
                _ => return Err("that clip isn't a shader".into()),
            },
            Query::Layers => self.comp.layers.len() as f64,
            Query::Width => self.renderer.size().0 as f64,
            Query::Audio(band) => self.audio.levels.get(*band) as f64,
            Query::Height => self.renderer.size().1 as f64,
            Query::Bpm => self.comp.bpm as f64,
            Query::Beat => self.beat,
        })
    }

    /// Resolve a parameter path and run `f` on it:
    /// * `master/master`, `master/crossfader`, `master/<effect>/<param>`
    /// * `<layer>/<param>` (layer = 1-based number or name), `<layer>/<effect>/<param>`
    /// * `<layer>/clip/<param>` (playing clip) or `<layer>/clipN/<param>` (column N)
    ///
    /// Names match case-insensitively, by prefix; effects also as `fxN`.
    pub(crate) fn with_param(&mut self, path: &str, f: impl FnOnce(&mut Param)) -> Result<(), String> {
        let segs: Vec<&str> = path.split('/').map(str::trim).collect();
        let comp = &mut self.comp;
        match segs.as_slice() {
            ["master", "master"] => f(&mut comp.master),
            ["master", "crossfader"] => f(&mut comp.crossfader),
            ["master", fx, param] => on_param(find_effect(&mut comp.effects, fx)?, param, f)?,
            [layer, rest @ ..] => {
                let li = layer_index(comp, layer)?;
                let l = &mut comp.layers[li];
                match rest {
                    [param] => on_param(l, param, f)?,
                    [clip, param] if clip.starts_with("clip") => {
                        let c = match clip[4..].parse::<usize>() {
                            Ok(n) => l.clips.get_mut(n.wrapping_sub(1)).and_then(|c| c.as_mut()),
                            Err(_) => l.active.and_then(|c| l.clips[c].as_mut()),
                        }
                        .ok_or_else(|| format!("no clip for {clip:?} on layer {}", li + 1))?;
                        on_param(c, param, f)?
                    }
                    [fx, param] => on_param(find_effect(&mut l.effects, fx)?, param, f)?,
                    _ => return Err(format!("bad parameter path {path:?}")),
                }
            }
            _ => return Err(format!("bad parameter path {path:?}")),
        }
        Ok(())
    }
}

impl App {
    /// Run `f` on the effect at `LAYER/EFFECT` or `master/EFFECT`.
    pub(crate) fn with_effect(&mut self, path: &str, f: impl FnOnce(&mut Effect)) -> Result<(), String> {
        let segs: Vec<&str> = path.split('/').map(str::trim).collect();
        let comp = &mut self.comp;
        let e = match segs.as_slice() {
            ["master", fx] => find_effect(&mut comp.effects, fx)?,
            [layer, fx] => {
                let li = layer_index(comp, layer)?;
                find_effect(&mut comp.layers[li].effects, fx)?
            }
            _ => return Err(format!("bad effect path {path:?} (use LAYER/EFFECT or master/EFFECT)")),
        };
        f(e);
        Ok(())
    }
}

/// A layer by 1-based number or name.
fn layer_index(comp: &crate::composition::Composition, seg: &str) -> Result<usize, String> {
    match seg.parse::<usize>() {
        Ok(n) if n >= 1 && n <= comp.layers.len() => Ok(n - 1),
        Ok(n) => Err(format!("no layer {n}")),
        Err(_) => comp
            .layers
            .iter()
            .position(|l| l.name.eq_ignore_ascii_case(seg))
            .ok_or_else(|| format!("no layer named {seg:?}")),
    }
}

fn template(name: &str) -> Result<(&'static str, crate::shader::Role, &'static str), String> {
    let names: Vec<String> = crate::shader::TEMPLATES.iter().map(|t| t.0.to_lowercase()).collect();
    let i = best_match(&names, name).ok_or_else(|| format!("no shader template {name:?} (have: {})", names.join(", ")))?;
    Ok(crate::shader::TEMPLATES[i])
}

fn make_parent(p: &Path) -> Result<(), String> {
    match p.parent() {
        Some(d) if !d.as_os_str().is_empty() => std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display())),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_matching_prefers_exact_then_prefix() {
        let labels: Vec<String> = ["position x", "position y", "scale", "rotation °"].iter().map(|s| s.to_string()).collect();
        assert_eq!(best_match(&labels, "Scale"), Some(2));
        assert_eq!(best_match(&labels, "rot"), Some(3));
        assert_eq!(best_match(&labels, "position y"), Some(1));
        assert_eq!(best_match(&labels, "zoom"), None);
    }
}

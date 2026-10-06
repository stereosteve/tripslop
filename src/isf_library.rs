//! The library: the ISF shaders and 3D models bundled with tripslop, plus any folders added
//! with `--isf DIR` or *Add folder…* (their `.fs` shaders and model files) and model files
//! added one by one.
//!
//! Models are listed by `assets/models/library.json` (files in `assets/models`, and shapes
//! generated in code). They're loaded the first time something needs them, and shared after
//! that (`Library::model`).
//!
//! The bundle lives in `assets/isf` and is baked into the binary by `build.rs`, so the library
//! works the same everywhere without touching the disk. `assets/isf/library.json` gives each
//! bundled shader its display name, category and tuned starting values. Shaders it doesn't list
//! (and everything in added folders) are classified from their ISF header instead.
//!
//! Bundled shaders come pre-checked: `tripslop --bake-library` compiles each one on the GPU and
//! renders its card picture, and those results (`assets/isf/baked.json`, `assets/isf/thumbs`)
//! are baked into the binary too. Anything without a current bake (a folder shader, or a
//! bundled one edited since) is test-compiled on a background thread instead, so the browser
//! can hide the ones tripslop can't run yet (multi-pass, ...).

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use crate::model::{ModelRef, formats, shapes};
use crate::shader::{self, CustomShader, Role};

mod bundle {
    include!(concat!(env!("OUT_DIR"), "/isf_bundle.rs"));
}

/// The curated index of the bundle.
const INDEX: &str = include_str!("../assets/isf/library.json");
/// The index of the bundled models.
const MODEL_INDEX: &str = include_str!("../assets/models/library.json");
/// Category of model files from added folders (when they aren't in a subfolder) and of
/// models added one by one.
const MY_MODELS: &str = "My Models";

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Kind {
    /// No image inputs: makes its own picture (a clip).
    Generator,
    /// Processes an image (a layer or master effect).
    Effect,
    /// `startImage` → `endImage`. Not supported yet; listed so the counts add up.
    Transition,
    /// A 3D model: a clip, or the object of the Shape projector / Projection mapping.
    Model,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Status {
    Checking,
    Ok,
    Unsupported(String),
}

#[derive(Clone, PartialEq, Debug)]
pub enum Source {
    /// Path inside `assets/isf`.
    Bundled(&'static str),
    File(PathBuf),
    /// Path inside `assets/models`.
    BundledModel(&'static str),
    /// A model generated in code (an id in `model::shapes::IDS`).
    Shape(&'static str),
}

#[derive(Clone, Debug)]
pub struct Entry {
    /// Stable identity: `isf:<path in the bundle>`, or the file path.
    pub key: String,
    pub source: Source,
    pub name: String,
    pub kind: Kind,
    pub category: String,
    pub tags: Vec<String>,
    pub description: String,
    pub credit: String,
    /// Starting values for parameters, by name.
    pub defaults: Vec<(String, f32)>,
    pub status: Status,
    /// Pre-rendered card picture (JPEG), for bundled shaders with a current bake.
    pub thumb: Option<&'static [u8]>,
    /// Models: degrees about x, y, z to stand it up.
    pub rotate: [f32; 3],
}

impl Entry {
    pub fn code(&self) -> Result<Cow<'static, str>, String> {
        match &self.source {
            _ if self.kind == Kind::Model => Err(format!("{} is a 3D model, not a shader", self.name)),
            Source::Bundled(p) => bundled(p).map(Cow::Borrowed).ok_or_else(|| format!("{p} isn't in the bundle")),
            Source::File(p) => std::fs::read_to_string(p).map(Cow::Owned).map_err(|e| format!("{}: {e}", p.display())),
            Source::BundledModel(_) | Source::Shape(_) => unreachable!("models are Kind::Model"),
        }
    }

    /// The companion vertex shader (`Name.vs` next to `Name.fs`), if any.
    pub fn vertex(&self) -> Option<String> {
        match &self.source {
            Source::Bundled(p) => bundled(&p.replace(".fs", ".vs")).map(str::to_string),
            Source::File(p) if self.kind != Kind::Model => std::fs::read_to_string(p.with_extension("vs")).ok(),
            _ => None,
        }
    }

    /// A fresh shader for a clip (generators) or an effect, with the entry's starting values.
    pub fn shader(&self) -> Result<CustomShader, String> {
        let role = if self.kind == Kind::Generator { Role::Source } else { Role::Effect };
        let mut s = CustomShader::new(&self.name, &self.code()?, role);
        s.vertex = self.vertex();
        s.initial = self.defaults.clone();
        Ok(s)
    }

    pub fn location(&self) -> String {
        match &self.source {
            Source::Bundled(p) => format!("bundled: {p}"),
            Source::File(p) => p.display().to_string(),
            Source::BundledModel(p) => format!("bundled: models/{p}"),
            Source::Shape(_) => "generated".into(),
        }
    }
}

fn bundled(path: &str) -> Option<&'static str> {
    bundle::FILES.iter().find(|(p, _)| *p == path).map(|(_, code)| *code)
}

/// What's being dragged out of the browser (see `ui::library`).
#[derive(Clone, Debug)]
pub struct Drag {
    pub key: String,
    pub name: String,
    pub kind: Kind,
}

enum Msg {
    Scanned(Vec<Entry>),
    Checked(String, Status),
    Done,
}

pub struct Library {
    /// Extra folders (`--isf`, *Add folder…*).
    pub dirs: Vec<PathBuf>,
    /// Model files added one by one (*Add models…*).
    pub files: Vec<PathBuf>,
    /// Sorted by kind, category, then name.
    pub entries: Vec<Entry>,
    /// Category order: the bundle's, then any others alphabetically.
    pub categories: Vec<String>,
    /// The bundle's category order (from `library.json`).
    index_order: Vec<String>,
    pub scanning: bool,
    rx: Option<Receiver<Msg>>,
    /// Models loaded so far, by key.
    models: Mutex<HashMap<String, ModelRef>>,
}

impl Library {
    pub fn new(dirs: Vec<PathBuf>) -> Self {
        let mut lib = Self::empty();
        lib.dirs = dirs;
        lib.rescan();
        lib
    }

    fn empty() -> Self {
        Self {
            dirs: Vec::new(),
            files: Vec::new(),
            entries: Vec::new(),
            categories: Vec::new(),
            index_order: Vec::new(),
            scanning: false,
            rx: None,
            models: Mutex::new(HashMap::new()),
        }
    }

    /// List model files (added to *My Models*).
    pub fn add_files(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        for p in paths {
            if !self.files.contains(&p) {
                self.files.push(p);
            }
        }
        self.rescan();
    }

    /// A model from the library, loaded the first time it's asked for.
    pub fn model(&self, key: &str) -> Result<ModelRef, String> {
        if let Some(m) = self.models.lock().unwrap().get(key) {
            return Ok(m.clone());
        }
        let e = self.find(key).ok_or_else(|| format!("{key} is no longer in the library"))?;
        let mut model = match &e.source {
            Source::BundledModel(p) => formats::load_bundled(p, &e.name)?,
            Source::Shape(id) => shapes::build(id, &e.name)?,
            Source::File(p) if e.kind == Kind::Model => formats::load(p)?,
            _ => return Err(format!("{} isn't a model", e.name)),
        };
        if e.rotate != [0.0; 3] {
            model.rotate(e.rotate);
        }
        let m = ModelRef { key: key.to_string(), model: Arc::new(model) };
        self.models.lock().unwrap().insert(key.to_string(), m.clone());
        Ok(m)
    }

    pub fn find(&self, key: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.key == key)
    }

    /// Look a shader (or, with `models`, a model) up by name, for scripts: case-insensitive,
    /// and `category/name` works too. An exact match wins, then a prefix, then a part of the
    /// name ("teapot" finds Utah Teapot).
    pub fn find_by_name(&self, name: &str, models: bool) -> Option<&Entry> {
        let want = name.trim().to_lowercase();
        let full = |e: &Entry| format!("{}/{}", e.category, e.name).to_lowercase();
        let pool = || self.entries.iter().filter(|e| (e.kind == Kind::Model) == models);
        pool()
            .find(|e| e.name.to_lowercase() == want || full(e) == want)
            .or_else(|| pool().find(|e| e.name.to_lowercase().starts_with(&want) || full(e).starts_with(&want)))
            .or_else(|| pool().find(|e| e.name.to_lowercase().contains(&want)))
    }

    pub fn rescan(&mut self) {
        let (mut index, mut order) = bundle_entries();
        let (models, model_order) = model_entries();
        index.extend(models);
        order.extend(model_order);
        self.index_order = order;
        self.entries = index;
        self.sort();
        let (tx, rx) = channel();
        let dirs = self.dirs.clone();
        let files = self.files.clone();
        // Baked entries already know their status.
        let bundled: Vec<(String, Source, Kind)> =
            self.entries.iter().filter(|e| e.status == Status::Checking).map(|e| (e.key.clone(), e.source.clone(), e.kind)).collect();
        // The browser has no threads (or folders): there this only checks bundled shaders
        // edited since the last bake, which is normally none.
        #[cfg(target_arch = "wasm32")]
        scan_thread(dirs, files, bundled, tx);
        #[cfg(not(target_arch = "wasm32"))]
        std::thread::spawn(move || scan_thread(dirs, files, bundled, tx));
        self.rx = Some(rx);
        self.scanning = true;
    }

    fn sort(&mut self) {
        let mut others: Vec<String> = self.entries.iter().map(|e| e.category.clone()).filter(|c| !self.index_order.contains(c)).collect();
        others.sort();
        others.dedup();
        self.categories = self.index_order.iter().cloned().chain(others).collect();
        let cats = &self.categories;
        let rank = |c: &str| cats.iter().position(|x| x == c).unwrap_or(usize::MAX);
        self.entries
            .sort_by(|a, b| (a.kind as u8, rank(&a.category), a.name.to_lowercase()).cmp(&(b.kind as u8, rank(&b.category), b.name.to_lowercase())));
    }

    /// Take in results from the scan thread; call once per frame.
    pub fn poll(&mut self) {
        let Some(rx) = &self.rx else { return };
        let mut added = false;
        while let Ok(msg) = rx.try_recv() {
            match msg {
                Msg::Scanned(mut found) => {
                    // A folder shader with the same name as a bundled one is listed separately.
                    self.entries.append(&mut found);
                    added = true;
                }
                Msg::Checked(key, status) => {
                    if let Some(e) = self.entries.iter_mut().find(|e| e.key == key) {
                        e.status = status;
                    }
                }
                Msg::Done => {
                    self.scanning = false;
                    self.rx = None;
                    break;
                }
            }
        }
        if added {
            self.sort();
        }
    }
}

/// FNV-1a: a hash of a shader's source that's the same on every build (std's isn't).
pub fn source_hash(code: &str) -> String {
    let h = code.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    format!("{h:016x}")
}

fn thumb_path(path: &str) -> String {
    format!("{}.jpg", path.trim_end_matches(".fs"))
}

/// The bundle, indexed by `library.json`, and the index's category order. Shaders whose bake is
/// current get its status and picture.
fn bundle_entries() -> (Vec<Entry>, Vec<String>) {
    let baked: serde_json::Value = serde_json::from_str(bundle::BAKED).unwrap_or_default();
    let index: serde_json::Value = serde_json::from_str(INDEX).expect("assets/isf/library.json is valid JSON");
    let order: Vec<String> = index["categories"].as_array().into_iter().flatten().filter_map(|c| c.as_str().map(String::from)).collect();
    let listed: Vec<&serde_json::Value> = index["shaders"].as_array().into_iter().flatten().collect();
    let mut entries = Vec::new();
    for (path, code) in bundle::FILES {
        if !path.ends_with(".fs") {
            continue;
        }
        let Some(mut e) = read_entry(code, Source::Bundled(path), file_stem(path), None) else { continue };
        let bake = &baked["shaders"][*path];
        if bake["hash"].as_str() == Some(&source_hash(code)) {
            e.status = match bake["status"].as_str() {
                Some("ok") => Status::Ok,
                _ => Status::Unsupported(bake["reason"].as_str().unwrap_or("doesn't compile").to_string()),
            };
            let tp = thumb_path(path);
            e.thumb = bundle::THUMBS.iter().find(|(p, _)| *p == tp).map(|(_, jpg)| *jpg);
        }
        if let Some(meta) = listed.iter().find(|m| m["file"] == *path) {
            if let Some(n) = meta["name"].as_str() {
                e.name = n.to_string();
            }
            if let Some(c) = meta["category"].as_str() {
                e.category = c.to_string();
            }
            e.tags = meta["tags"].as_array().into_iter().flatten().filter_map(|t| t.as_str().map(String::from)).collect();
            e.defaults = meta["defaults"]
                .as_object()
                .into_iter()
                .flatten()
                .filter_map(|(k, v)| Some((k.clone(), v.as_f64()? as f32)))
                .collect();
        }
        entries.push(e);
    }
    (entries, order)
}

fn file_stem(path: &str) -> String {
    Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

fn scan_thread(dirs: Vec<PathBuf>, models: Vec<PathBuf>, bundled: Vec<(String, Source, Kind)>, tx: Sender<Msg>) {
    let mut files = Vec::new();
    for d in &dirs {
        collect(d, d, 0, &mut files);
    }
    files.extend(models.into_iter().map(|p| (p, None)));
    let found: Vec<Entry> = files
        .into_iter()
        .filter_map(|(path, folder)| {
            if formats::is_model(&path) {
                // Loaded (and checked) when first used.
                return Some(model_file_entry(&path, folder));
            }
            let code = std::fs::read_to_string(&path).ok()?;
            read_entry(&code, Source::File(path.clone()), path.file_stem()?.to_string_lossy().into_owned(), folder)
        })
        .collect();
    let mut checks = bundled;
    checks.extend(found.iter().filter(|e| e.kind != Kind::Model).map(|e| (e.key.clone(), e.source.clone(), e.kind)));
    if tx.send(Msg::Scanned(found)).is_err() {
        return;
    }
    for (key, source, kind) in checks {
        if tx.send(Msg::Checked(key, check(&source, kind))).is_err() {
            return;
        }
    }
    let _ = tx.send(Msg::Done);
}

/// A model file from an added folder (or added on its own).
fn model_file_entry(path: &Path, folder: Option<String>) -> Entry {
    let ext = path.extension().map(|e| e.to_string_lossy().to_uppercase()).unwrap_or_default();
    Entry {
        key: path.display().to_string(),
        source: Source::File(path.to_path_buf()),
        name: file_stem(&path.to_string_lossy()),
        kind: Kind::Model,
        category: folder.unwrap_or_else(|| MY_MODELS.into()),
        tags: vec![ext.to_lowercase()],
        description: format!("{ext} model file"),
        credit: String::new(),
        defaults: Vec::new(),
        status: Status::Ok,
        thumb: None,
        rotate: [0.0; 3],
    }
}

/// The bundled models, indexed by `assets/models/library.json`, and the index's category order.
fn model_entries() -> (Vec<Entry>, Vec<String>) {
    let index: serde_json::Value = serde_json::from_str(MODEL_INDEX).expect("assets/models/library.json is valid JSON");
    let order = index["categories"].as_array().into_iter().flatten().filter_map(|c| c.as_str().map(String::from)).collect();
    let text = |m: &serde_json::Value, k: &str| m[k].as_str().unwrap_or_default().to_string();
    let entries = index["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let (key, source) = if let Some(f) = m["file"].as_str() {
                let path = formats::bundled_models().find(|p| *p == f)?;
                (format!("model:{path}"), Source::BundledModel(path))
            } else {
                let id = shapes::IDS.iter().find(|i| m["shape"] == **i)?;
                (format!("shape:{id}"), Source::Shape(id))
            };
            let rotate = m["rotate"].as_array().map(|r| std::array::from_fn(|i| r.get(i).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32));
            Some(Entry {
                key,
                source,
                name: text(m, "name"),
                kind: Kind::Model,
                category: text(m, "category"),
                tags: m["tags"].as_array().into_iter().flatten().filter_map(|t| t.as_str().map(String::from)).collect(),
                description: text(m, "description"),
                credit: text(m, "credit"),
                defaults: Vec::new(),
                status: Status::Ok,
                thumb: None,
                rotate: rotate.unwrap_or([0.0; 3]),
            })
        })
        .collect();
    (entries, order)
}

/// Shader (`.fs`) and model files under `dir`, with the subfolder they're in (relative to
/// `root`), if any.
fn collect(root: &Path, dir: &Path, depth: usize, out: &mut Vec<(PathBuf, Option<String>)>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let path = e.path();
        if path.is_dir() {
            if depth < 4 {
                collect(root, &path, depth + 1, out);
            }
        } else if path.extension().is_some_and(|x| x.eq_ignore_ascii_case("fs")) || formats::is_model(&path) {
            let folder = (dir != root).then(|| dir.file_name().unwrap_or_default().to_string_lossy().into_owned());
            out.push((path, folder));
        }
    }
}

/// Classify a shader from its ISF header. `None` if it has no (valid) header.
fn read_entry(code: &str, source: Source, name: String, folder: Option<String>) -> Option<Entry> {
    let json = header_json(code)?;
    let images: Vec<&str> = json["INPUTS"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|i| i["TYPE"] == "image")
        .filter_map(|i| i["NAME"].as_str())
        .collect();
    let kind = if images.contains(&"startImage") && images.contains(&"endImage") {
        Kind::Transition
    } else if images.is_empty() {
        Kind::Generator
    } else {
        Kind::Effect
    };
    let categories: Vec<&str> = json["CATEGORIES"].as_array().into_iter().flatten().filter_map(|c| c.as_str()).collect();
    let category = categories
        .iter()
        .map(|c| normalize_category(c))
        .find(|c| !c.is_empty())
        .or(folder)
        .unwrap_or_else(|| "Other".into());
    let text = |k: &str| json[k].as_str().unwrap_or_default().trim().to_string();
    let key = match &source {
        Source::Bundled(p) => format!("isf:{p}"),
        Source::File(p) => p.display().to_string(),
        Source::BundledModel(_) | Source::Shape(_) => unreachable!("read_entry reads shaders"),
    };
    Some(Entry {
        key,
        source,
        name,
        kind,
        category,
        tags: Vec::new(),
        description: text("DESCRIPTION"),
        credit: text("CREDIT"),
        defaults: Vec::new(),
        status: Status::Checking,
        thumb: None,
        rotate: [0.0; 3],
    })
}

/// Re-check every bundled shader and write the results for the next build: `baked.json` and a
/// card picture per working shader in `thumbs/`. `render` draws a card on the GPU, or says why
/// the GPU rejects the shader. Returns a summary.
pub fn bake(mut render: impl FnMut(&Entry) -> Result<image::RgbaImage, String>) -> Result<String, String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/isf");
    let thumbs = root.join("thumbs");
    let _ = std::fs::remove_dir_all(&thumbs);
    let (entries, _) = bundle_entries();
    let mut shaders = serde_json::Map::new();
    let (mut ok, mut bad) = (0, 0);
    for e in &entries {
        let Source::Bundled(path) = e.source else { continue };
        let code = bundled(path).unwrap_or_default();
        let status = match check(&e.source, e.kind) {
            Status::Ok => render(e).and_then(|img| {
                let out = thumbs.join(thumb_path(path));
                std::fs::create_dir_all(out.parent().unwrap()).map_err(|err| err.to_string())?;
                let rgb = image::DynamicImage::ImageRgba8(img).to_rgb8();
                let file = std::fs::File::create(&out).map_err(|err| format!("{}: {err}", out.display()))?;
                image::codecs::jpeg::JpegEncoder::new_with_quality(std::io::BufWriter::new(file), 85)
                    .encode_image(&rgb)
                    .map_err(|err| format!("writing {} failed: {err}", out.display()))
            }),
            Status::Unsupported(why) => Err(why),
            Status::Checking => unreachable!(),
        };
        let mut rec = serde_json::Map::new();
        rec.insert("hash".into(), source_hash(code).into());
        match status {
            Ok(()) => {
                ok += 1;
                rec.insert("status".into(), "ok".into());
            }
            Err(why) => {
                bad += 1;
                rec.insert("status".into(), "unsupported".into());
                rec.insert("reason".into(), why.into());
            }
        }
        shaders.insert(path.to_string(), rec.into());
    }
    let doc = serde_json::json!({
        "//": "Generated by `cargo run --release -- --bake-library`; don't edit. Compile results for the bundled shaders (by source hash).",
        "shaders": shaders,
    });
    let json = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())? + "\n";
    std::fs::write(root.join("baked.json"), json).map_err(|e| e.to_string())?;
    Ok(format!("baked {} shaders into {}: {ok} work, {bad} unsupported", ok + bad, root.display()))
}

/// The JSON object in the leading `/*{ ... }*/` comment.
fn header_json(code: &str) -> Option<serde_json::Value> {
    let rest = code.trim_start().strip_prefix("/*")?;
    let json: serde_json::Value = serde_json::from_str(&rest[..rest.find("*/")?]).ok()?;
    json.is_object().then_some(json)
}

/// Folds the many spellings in the wild ("Color Effect", "Color Adjustment", "color") into one
/// category; returns "" for tags that say nothing about what a shader does.
fn normalize_category(c: &str) -> String {
    let c = c.trim();
    let c = [" Effect", " Effects", " Adjustment"].iter().find_map(|s| c.strip_suffix(s)).unwrap_or(c);
    if matches!(c.to_lowercase().as_str(), "" | "generator" | "filter" | "filters" | "v002" | "utility" | "xxx") {
        return String::new();
    }
    c.split_whitespace()
        .map(|w| {
            let mut ch = w.chars();
            ch.next().map(|f| f.to_uppercase().chain(ch).collect::<String>()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn check(source: &Source, kind: Kind) -> Status {
    match kind {
        Kind::Transition => return Status::Unsupported("ISF transitions aren't supported yet".into()),
        Kind::Model => return Status::Ok,
        _ => {}
    }
    let (code, vertex) = match source {
        Source::Bundled(p) => (bundled(p).map(str::to_string), bundled(&p.replace(".fs", ".vs")).map(str::to_string)),
        Source::File(p) => (std::fs::read_to_string(p).ok(), std::fs::read_to_string(p.with_extension("vs")).ok()),
        Source::BundledModel(_) | Source::Shape(_) => return Status::Ok,
    };
    let Some(code) = code else {
        return Status::Unsupported("can't read file".into());
    };
    match shader::compile(&shader::prepare_with(&code, vertex.as_deref())) {
        Ok(_) => Status::Ok,
        Err(errs) => Status::Unsupported(errs.first().map(|e| e.message.clone()).unwrap_or_default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categories_fold_together() {
        assert_eq!(normalize_category("Color Effect"), "Color");
        assert_eq!(normalize_category("Color Adjustment"), "Color");
        assert_eq!(normalize_category("glitch"), "Glitch");
        assert_eq!(normalize_category("test pattern"), "Test Pattern");
        assert_eq!(normalize_category("v002"), "");
        assert_eq!(normalize_category("Generator"), "");
    }

    #[test]
    fn classifies_folder_shaders_from_header() {
        let dir = std::env::temp_dir().join(format!("tripslop-isf-{}", std::process::id()));
        let sub = dir.join("Mine");
        std::fs::create_dir_all(&sub).unwrap();
        let write = |p: PathBuf, inputs: &str, cats: &str| {
            std::fs::write(p, format!("/*{{ \"CATEGORIES\": [{cats}], \"INPUTS\": [{inputs}] }}*/\nvoid main() {{ gl_FragColor = vec4(1.0); }}\n")).unwrap()
        };
        write(dir.join("Gen.fs"), "", "\"Generator\", \"Pattern\"");
        write(dir.join("Fx.fs"), r#"{"NAME": "inputImage", "TYPE": "image"}"#, "\"Color Effect\"");
        write(dir.join("Tr.fs"), r#"{"NAME": "startImage", "TYPE": "image"}, {"NAME": "endImage", "TYPE": "image"}"#, "\"Wipe\"");
        write(sub.join("Plain.fs"), "", "");
        std::fs::write(dir.join("notes.txt"), "not a shader").unwrap();

        let mut files = Vec::new();
        collect(&dir, &dir, 0, &mut files);
        let mut entries: Vec<Entry> = files
            .into_iter()
            .filter(|(p, _)| !formats::is_model(p))
            .filter_map(|(p, f)| read_entry(&std::fs::read_to_string(&p).unwrap(), Source::File(p.clone()), file_stem(&p.to_string_lossy()), f))
            .collect();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        let got: Vec<(&str, Kind, &str)> = entries.iter().map(|e| (e.name.as_str(), e.kind, e.category.as_str())).collect();
        assert_eq!(
            got,
            [("Fx", Kind::Effect, "Color"), ("Gen", Kind::Generator, "Pattern"), ("Plain", Kind::Generator, "Mine"), ("Tr", Kind::Transition, "Wipe")]
        );
        assert_eq!(check(&Source::File(dir.join("Gen.fs")), Kind::Generator), Status::Ok);
        assert!(matches!(check(&Source::File(dir.join("Tr.fs")), Kind::Transition), Status::Unsupported(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn index_matches_the_bundle() {
        let index: serde_json::Value = serde_json::from_str(INDEX).unwrap();
        let cats: Vec<&str> = index["categories"].as_array().unwrap().iter().map(|c| c.as_str().unwrap()).collect();
        for m in index["shaders"].as_array().unwrap() {
            let file = m["file"].as_str().unwrap();
            assert!(bundled(file).is_some(), "library.json lists {file}, which isn't in assets/isf");
            let cat = m["category"].as_str().unwrap();
            assert!(cats.contains(&cat), "{file}: category {cat:?} isn't in the categories list");
        }
        let (entries, _) = bundle_entries();
        assert!(entries.len() > 300, "only {} bundled shaders", entries.len());
        let mut keys: Vec<&str> = entries.iter().map(|e| e.key.as_str()).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), entries.len(), "duplicate keys");
    }

    #[test]
    fn bake_is_current() {
        let baked: serde_json::Value = serde_json::from_str(bundle::BAKED).unwrap_or_default();
        let fix = "re-bake with `cargo run --release -- --bake-library`";
        for (path, code) in bundle::FILES.iter().filter(|(p, _)| p.ends_with(".fs")) {
            let bake = &baked["shaders"][*path];
            assert_eq!(bake["hash"].as_str(), Some(source_hash(code).as_str()), "{path} changed since the last bake: {fix}");
            if bake["status"] == "ok" {
                assert!(bundle::THUMBS.iter().any(|(p, _)| *p == thumb_path(path)), "{path} has no card picture: {fix}");
            }
        }
        assert!(bundle_entries().0.iter().all(|e| e.status != Status::Checking));
    }

    #[test]
    fn finds_by_name_and_applies_defaults() {
        let lib = Library { entries: bundle_entries().0, ..Library::empty() };
        let e = lib.find_by_name("center crosshair", false).expect("Center Crosshair is bundled");
        assert_eq!(e.category, "Basics");
        assert!(e.defaults.iter().any(|(k, _)| k == "lineWidth"));
        assert_eq!(lib.find_by_name("three-body orbits/figure", false).map(|e| e.name.as_str()), Some("Figure Eight"));
        let s = e.shader().unwrap();
        assert_eq!(s.initial, e.defaults);
    }

    #[test]
    fn every_listed_model_loads_once_and_is_shared() {
        let index: serde_json::Value = serde_json::from_str(MODEL_INDEX).unwrap();
        let listed = index["models"].as_array().unwrap().len();
        let (entries, cats) = model_entries();
        assert_eq!(entries.len(), listed, "library.json lists a file that isn't bundled, or a shape that doesn't exist");
        for e in &entries {
            assert!(cats.contains(&e.category), "{}: category {:?} isn't in the categories list", e.name, e.category);
        }
        // Every bundled model file is listed.
        for path in formats::bundled_models() {
            assert!(entries.iter().any(|e| e.source == Source::BundledModel(path)), "{path} isn't in assets/models/library.json");
        }
        let lib = Library { entries, ..Library::empty() };
        for e in &lib.entries {
            let m = lib.model(&e.key).unwrap_or_else(|err| panic!("{}: {err}", e.name));
            assert_eq!(m.name(), e.name);
        }
        let teapot = lib.find_by_name("teapot", true).unwrap();
        assert!(Arc::ptr_eq(&lib.model(&teapot.key).unwrap().model, &lib.model(&teapot.key).unwrap().model));
        // Shaders and models with the same name stay apart.
        assert_eq!(lib.find_by_name("torus knot", true).unwrap().kind, Kind::Model);
    }

    #[test]
    fn folders_list_their_models() {
        let dir = std::env::temp_dir().join(format!("tripslop-models-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Scans")).unwrap();
        std::fs::write(dir.join("tri.off"), "OFF\n3 1 0\n0 0 0\n1 0 0\n0 1 0\n3 0 1 2\n").unwrap();
        std::fs::write(dir.join("Scans/tri.stl"), "solid t\nfacet normal 0 0 1\nouter loop\nvertex 0 0 0\nvertex 1 0 0\nvertex 0 1 0\nendloop\nendfacet\nendsolid t\n").unwrap();
        let mut files = Vec::new();
        collect(&dir, &dir, 0, &mut files);
        let mut entries: Vec<Entry> = files.iter().filter(|(p, _)| formats::is_model(p)).map(|(p, f)| model_file_entry(p, f.clone())).collect();
        entries.sort_by(|a, b| a.category.cmp(&b.category));
        let got: Vec<(&str, &str)> = entries.iter().map(|e| (e.name.as_str(), e.category.as_str())).collect();
        assert_eq!(got, [("tri", MY_MODELS), ("tri", "Scans")]);
        let lib = Library { entries, ..Library::empty() };
        for e in &lib.entries {
            assert_eq!(lib.model(&e.key).unwrap().model.triangles(), 1);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Compatibility report for the bundle and any `TRIPSLOP_ISF` folder:
    /// `cargo test --release isf_report -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn isf_report() {
        let mut entries = bundle_entries().0;
        if let Some(dir) = std::env::var_os("TRIPSLOP_ISF").map(PathBuf::from) {
            let mut files = Vec::new();
            collect(&dir, &dir, 0, &mut files);
            entries.extend(files.into_iter().filter_map(|(p, f)| {
                read_entry(&std::fs::read_to_string(&p).ok()?, Source::File(p.clone()), file_stem(&p.to_string_lossy()), f)
            }));
        }
        let mut reasons = std::collections::BTreeMap::<String, Vec<String>>::new();
        let mut ok = 0;
        let start = web_time::Instant::now();
        for e in entries.iter().filter(|e| e.kind != Kind::Transition) {
            match check(&e.source, e.kind) {
                Status::Ok => ok += 1,
                Status::Unsupported(m) => reasons.entry(m.chars().take(60).collect()).or_default().push(e.name.clone()),
                Status::Checking => unreachable!(),
            }
        }
        let failed: usize = reasons.values().map(Vec::len).sum();
        println!("{ok} of {} generators / effects compile ({:?})", ok + failed, start.elapsed());
        let mut reasons: Vec<_> = reasons.into_iter().collect();
        reasons.sort_by_key(|(_, v)| std::cmp::Reverse(v.len()));
        for (m, names) in reasons {
            println!("{:3}  {m}  ({})", names.len(), names.join(", "));
        }
    }
}

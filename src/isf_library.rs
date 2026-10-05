//! The shader library: the ISF shaders bundled with tripslop, plus any folders added with
//! `--isf DIR` or *Add folder…*.
//!
//! The bundle lives in `assets/isf` and is baked into the binary by `build.rs`, so the library
//! works the same everywhere without touching the disk. `assets/isf/library.json` gives each
//! bundled shader its display name, category and tuned starting values. Shaders it doesn't list
//! (and everything in added folders) are classified from their ISF header instead.
//!
//! Every shader is test-compiled on a background thread, so the browser can hide the ones
//! tripslop can't run yet (multi-pass, ...).

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

use crate::shader::{self, CustomShader, Role};

mod bundle {
    include!(concat!(env!("OUT_DIR"), "/isf_bundle.rs"));
}

/// The curated index of the bundle.
const INDEX: &str = include_str!("../assets/isf/library.json");

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Kind {
    /// No image inputs: makes its own picture (a clip).
    Generator,
    /// Processes an image (a layer or master effect).
    Effect,
    /// `startImage` → `endImage`. Not supported yet; listed so the counts add up.
    Transition,
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
}

impl Entry {
    pub fn code(&self) -> Result<Cow<'static, str>, String> {
        match &self.source {
            Source::Bundled(p) => bundled(p).map(Cow::Borrowed).ok_or_else(|| format!("{p} isn't in the bundle")),
            Source::File(p) => std::fs::read_to_string(p).map(Cow::Owned).map_err(|e| format!("{}: {e}", p.display())),
        }
    }

    /// The companion vertex shader (`Name.vs` next to `Name.fs`), if any.
    pub fn vertex(&self) -> Option<String> {
        match &self.source {
            Source::Bundled(p) => bundled(&p.replace(".fs", ".vs")).map(str::to_string),
            Source::File(p) => std::fs::read_to_string(p.with_extension("vs")).ok(),
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
    /// Sorted by kind, category, then name.
    pub entries: Vec<Entry>,
    /// Category order: the bundle's, then any others alphabetically.
    pub categories: Vec<String>,
    /// The bundle's category order (from `library.json`).
    index_order: Vec<String>,
    pub scanning: bool,
    rx: Option<Receiver<Msg>>,
}

impl Library {
    pub fn new(dirs: Vec<PathBuf>) -> Self {
        let mut lib = Self { dirs, entries: Vec::new(), categories: Vec::new(), index_order: Vec::new(), scanning: false, rx: None };
        lib.rescan();
        lib
    }

    pub fn find(&self, key: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.key == key)
    }

    /// Look a shader up by name (case-insensitive; prefix and `category/name` work too), for
    /// scripts.
    pub fn find_by_name(&self, name: &str) -> Option<&Entry> {
        let want = name.trim().to_lowercase();
        let full = |e: &Entry| format!("{}/{}", e.category, e.name).to_lowercase();
        self.entries
            .iter()
            .find(|e| e.name.to_lowercase() == want || full(e) == want)
            .or_else(|| self.entries.iter().find(|e| e.name.to_lowercase().starts_with(&want) || full(e).starts_with(&want)))
    }

    pub fn rescan(&mut self) {
        let (index, order) = bundle_entries();
        self.index_order = order;
        self.entries = index;
        self.sort();
        let (tx, rx) = channel();
        let dirs = self.dirs.clone();
        let bundled: Vec<(String, Source, Kind)> = self.entries.iter().map(|e| (e.key.clone(), e.source.clone(), e.kind)).collect();
        std::thread::spawn(move || scan_thread(dirs, bundled, tx));
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

/// The bundle, indexed by `library.json`, and the index's category order.
fn bundle_entries() -> (Vec<Entry>, Vec<String>) {
    let index: serde_json::Value = serde_json::from_str(INDEX).expect("assets/isf/library.json is valid JSON");
    let order: Vec<String> = index["categories"].as_array().into_iter().flatten().filter_map(|c| c.as_str().map(String::from)).collect();
    let listed: Vec<&serde_json::Value> = index["shaders"].as_array().into_iter().flatten().collect();
    let mut entries = Vec::new();
    for (path, code) in bundle::FILES {
        if !path.ends_with(".fs") {
            continue;
        }
        let Some(mut e) = read_entry(code, Source::Bundled(path), file_stem(path), None) else { continue };
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

fn scan_thread(dirs: Vec<PathBuf>, bundled: Vec<(String, Source, Kind)>, tx: Sender<Msg>) {
    let mut files = Vec::new();
    for d in &dirs {
        collect(d, d, 0, &mut files);
    }
    let found: Vec<Entry> = files
        .into_iter()
        .filter_map(|(path, folder)| {
            let code = std::fs::read_to_string(&path).ok()?;
            read_entry(&code, Source::File(path.clone()), path.file_stem()?.to_string_lossy().into_owned(), folder)
        })
        .collect();
    let mut checks = bundled;
    checks.extend(found.iter().map(|e| (e.key.clone(), e.source.clone(), e.kind)));
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

/// `.fs` files under `dir`, with the subfolder they're in (relative to `root`), if any.
fn collect(root: &Path, dir: &Path, depth: usize, out: &mut Vec<(PathBuf, Option<String>)>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let path = e.path();
        if path.is_dir() {
            if depth < 4 {
                collect(root, &path, depth + 1, out);
            }
        } else if path.extension().is_some_and(|x| x.eq_ignore_ascii_case("fs")) {
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
    })
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
    if kind == Kind::Transition {
        return Status::Unsupported("ISF transitions aren't supported yet".into());
    }
    let (code, vertex) = match source {
        Source::Bundled(p) => (bundled(p).map(str::to_string), bundled(&p.replace(".fs", ".vs")).map(str::to_string)),
        Source::File(p) => (std::fs::read_to_string(p).ok(), std::fs::read_to_string(p.with_extension("vs")).ok()),
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
    fn finds_by_name_and_applies_defaults() {
        let lib = Library { dirs: Vec::new(), entries: bundle_entries().0, categories: Vec::new(), index_order: Vec::new(), scanning: false, rx: None };
        let e = lib.find_by_name("center crosshair").expect("Center Crosshair is bundled");
        assert_eq!(e.category, "Basics");
        assert!(e.defaults.iter().any(|(k, _)| k == "lineWidth"));
        assert_eq!(lib.find_by_name("three-body orbits/figure").map(|e| e.name.as_str()), Some("Figure Eight"));
        let s = e.shader().unwrap();
        assert_eq!(s.initial, e.defaults);
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
        let start = std::time::Instant::now();
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

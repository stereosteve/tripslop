//! A browsable library of ISF shaders (`.fs` files, as installed by VDMX and friends in
//! `/Library/Graphics/ISF`).
//!
//! Folders are scanned on a background thread. Each shader is classified from its JSON header
//! (generator, effect or transition), filed under its first meaningful `CATEGORIES` entry, and
//! test-compiled so the browser can grey out the ones tripslop can't run yet (multi-pass, ...).

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

use crate::shader;

/// Where ISF shaders are installed system-wide and per user.
pub fn default_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![PathBuf::from("/Library/Graphics/ISF")];
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join("Library/Graphics/ISF"));
    }
    dirs.into_iter().filter(|d| d.is_dir()).collect()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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

#[derive(Clone, Debug)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub kind: Kind,
    pub category: String,
    pub description: String,
    pub credit: String,
    pub status: Status,
}

/// What's being dragged out of the browser (see `ui::library`).
#[derive(Clone, Debug)]
pub struct Drag {
    pub path: PathBuf,
    pub name: String,
    pub kind: Kind,
}

enum Msg {
    Scanned(Vec<Entry>),
    Checked(usize, Status),
    Done,
}

pub struct Library {
    pub dirs: Vec<PathBuf>,
    /// Sorted by kind, category, then name.
    pub entries: Vec<Entry>,
    pub scanning: bool,
    rx: Option<Receiver<Msg>>,
}

impl Library {
    pub fn new(dirs: Vec<PathBuf>) -> Self {
        let mut lib = Self { dirs, entries: Vec::new(), scanning: false, rx: None };
        lib.rescan();
        lib
    }

    pub fn rescan(&mut self) {
        self.entries.clear();
        if self.dirs.is_empty() {
            self.scanning = false;
            self.rx = None;
            return;
        }
        let (tx, rx) = channel();
        let dirs = self.dirs.clone();
        std::thread::spawn(move || scan_thread(dirs, tx));
        self.rx = Some(rx);
        self.scanning = true;
    }

    /// Take in results from the scan thread; call once per frame.
    pub fn poll(&mut self) {
        let Some(rx) = &self.rx else { return };
        while let Ok(msg) = rx.try_recv() {
            match msg {
                Msg::Scanned(entries) => self.entries = entries,
                Msg::Checked(i, status) => {
                    if let Some(e) = self.entries.get_mut(i) {
                        e.status = status;
                    }
                }
                Msg::Done => {
                    self.scanning = false;
                    self.rx = None;
                    return;
                }
            }
        }
    }
}

fn scan_thread(dirs: Vec<PathBuf>, tx: Sender<Msg>) {
    let mut files = Vec::new();
    for d in &dirs {
        collect(d, d, 0, &mut files);
    }
    let mut entries: Vec<Entry> = files.into_iter().filter_map(|(path, folder)| read_entry(&path, folder)).collect();
    entries.sort_by(|a, b| {
        (a.kind as u8, &a.category, a.name.to_lowercase()).cmp(&(b.kind as u8, &b.category, b.name.to_lowercase()))
    });
    // Same shader installed in two places: keep the first.
    entries.dedup_by(|a, b| a.kind == b.kind && a.name == b.name);
    let checks: Vec<(PathBuf, Kind)> = entries.iter().map(|e| (e.path.clone(), e.kind)).collect();
    if tx.send(Msg::Scanned(entries)).is_err() {
        return;
    }
    for (i, (path, kind)) in checks.into_iter().enumerate() {
        if tx.send(Msg::Checked(i, check(&path, kind))).is_err() {
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

fn read_entry(path: &Path, folder: Option<String>) -> Option<Entry> {
    let code = std::fs::read_to_string(path).ok()?;
    let json = header_json(&code)?;
    let name = path.file_stem()?.to_string_lossy().into_owned();
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
    Some(Entry {
        path: path.to_path_buf(),
        name,
        kind,
        category,
        description: text("DESCRIPTION"),
        credit: text("CREDIT"),
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

fn check(path: &Path, kind: Kind) -> Status {
    if kind == Kind::Transition {
        return Status::Unsupported("ISF transitions aren't supported yet".into());
    }
    let Ok(code) = std::fs::read_to_string(path) else {
        return Status::Unsupported("can't read file".into());
    };
    let vertex = std::fs::read_to_string(path.with_extension("vs")).ok();
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
    fn classifies_from_header() {
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
        let mut entries: Vec<Entry> = files.into_iter().filter_map(|(p, f)| read_entry(&p, f)).collect();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        let got: Vec<(&str, Kind, &str)> = entries.iter().map(|e| (e.name.as_str(), e.kind, e.category.as_str())).collect();
        assert_eq!(
            got,
            [("Fx", Kind::Effect, "Color"), ("Gen", Kind::Generator, "Pattern"), ("Plain", Kind::Generator, "Mine"), ("Tr", Kind::Transition, "Wipe")]
        );
        assert_eq!(check(&dir.join("Gen.fs"), Kind::Generator), Status::Ok);
        assert!(matches!(check(&dir.join("Tr.fs"), Kind::Transition), Status::Unsupported(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Compatibility report for an installed ISF folder (`cargo test --release isf_folder -- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn isf_folder_report() {
        let dirs = default_dirs();
        let mut files = Vec::new();
        for d in &dirs {
            collect(d, d, 0, &mut files);
        }
        let entries: Vec<Entry> = files.into_iter().filter_map(|(p, f)| read_entry(&p, f)).collect();
        let mut reasons = std::collections::BTreeMap::<String, Vec<String>>::new();
        let mut ok = 0;
        for e in entries.iter().filter(|e| e.kind != Kind::Transition) {
            match check(&e.path, e.kind) {
                Status::Ok => ok += 1,
                Status::Unsupported(m) => reasons.entry(m.chars().take(60).collect()).or_default().push(e.name.clone()),
                Status::Checking => unreachable!(),
            }
        }
        let failed: usize = reasons.values().map(Vec::len).sum();
        println!("{dirs:?}: {ok} of {} generators / effects compile", ok + failed);
        let mut reasons: Vec<_> = reasons.into_iter().collect();
        reasons.sort_by_key(|(_, v)| std::cmp::Reverse(v.len()));
        for (m, names) in reasons {
            println!("{:3}  {m}  ({})", names.len(), names.join(", "));
        }
    }
}

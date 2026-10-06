//! Bakes the bundled ISF shaders (`assets/isf`) into the binary: writes `isf_bundle.rs` with
//! an `include_str!` for every `.fs` / `.vs` file, plus the card pictures and compile results
//! made by `tripslop --bake-library` (`thumbs/*.jpg`, `baked.json`), so the library needs
//! nothing on disk and does no work at startup. The bundled 3D models (`assets/models`) are
//! baked in the same way, as bytes.

use std::path::{Path, PathBuf};

fn collect(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir).expect("assets/isf is readable").flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect(root, &path, out);
        } else if path.extension().is_some_and(|x| x == "fs" || x == "vs" || x == "jpg") {
            out.push(path);
        }
    }
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/isf");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut files = Vec::new();
    collect(&root, &root, &mut files);
    let rel = |f: &Path| f.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
    let mut src = String::from("/// (path inside assets/isf, contents) of every bundled shader file.\npub static FILES: &[(&str, &str)] = &[\n");
    for f in files.iter().filter(|f| f.extension().is_some_and(|x| x != "jpg")) {
        // Rerun when a file is edited, not just when the folder's listing changes.
        println!("cargo:rerun-if-changed={}", f.display());
        src += &format!("    ({:?}, include_str!({:?})),\n", rel(f), f.display().to_string());
    }
    src += "];\n\n/// (path inside assets/isf/thumbs, JPEG) of every baked card picture.\npub static THUMBS: &[(&str, &[u8])] = &[\n";
    for f in files.iter().filter(|f| f.extension().is_some_and(|x| x == "jpg")) {
        println!("cargo:rerun-if-changed={}", f.display());
        let r = rel(f);
        src += &format!("    ({:?}, include_bytes!({:?})),\n", r.strip_prefix("thumbs/").unwrap_or(&r), f.display().to_string());
    }
    src += "];\n\n";
    // Compile results from the last bake; empty until there's been one.
    let baked = root.join("baked.json");
    println!("cargo:rerun-if-changed={}", baked.display());
    src += &match baked.exists() {
        true => format!("pub static BAKED: &str = include_str!({:?});\n", baked.display().to_string()),
        false => "pub static BAKED: &str = \"{}\";\n".into(),
    };
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    std::fs::write(out.join("isf_bundle.rs"), src).unwrap();
    models(&out);
    sets(&out);
}

/// `set_bundle.rs`: the demo sets in `sets/` (`NN-name.tripset`, listed in file name order) and
/// their welcome-screen pictures (`sets/thumbs/NN-name.jpg`, made by `scripts/bake-sets.sh`).
fn sets(out: &Path) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("sets");
    println!("cargo:rerun-if-changed={}", root.display());
    println!("cargo:rerun-if-changed={}", root.join("thumbs").display());
    let mut files: Vec<PathBuf> = std::fs::read_dir(&root).expect("sets/ is readable").flatten().map(|e| e.path()).collect();
    files.retain(|f| f.extension().is_some_and(|x| x == "tripset"));
    files.sort();
    let mut src = String::from("/// (file stem, contents, welcome picture) of every bundled set.\npub static SETS: &[(&str, &str, Option<&[u8]>)] = &[\n");
    for f in files {
        println!("cargo:rerun-if-changed={}", f.display());
        let stem = f.file_stem().unwrap().to_string_lossy().into_owned();
        let thumb = root.join("thumbs").join(format!("{stem}.jpg"));
        println!("cargo:rerun-if-changed={}", thumb.display());
        let thumb = match thumb.exists() {
            true => format!("Some(include_bytes!({:?}))", thumb.display().to_string()),
            false => "None".into(),
        };
        src += &format!("    ({stem:?}, include_str!({:?}), {thumb}),\n", f.display().to_string());
    }
    src += "];\n";
    std::fs::write(out.join("set_bundle.rs"), src).unwrap();
}

/// `model_bundle.rs`: every file in `assets/models` (models, their materials and textures) but
/// the index and the readme, which the library reads separately.
fn models(out: &Path) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/models");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut files = Vec::new();
    walk(&root, &mut files);
    let mut src = String::from("/// (path inside assets/models, contents) of every bundled model file.\npub static FILES: &[(&str, &[u8])] = &[\n");
    for f in files {
        let rel = f.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
        if rel == "library.json" || rel.ends_with(".md") || rel.starts_with('.') || rel.contains("/.") {
            continue;
        }
        println!("cargo:rerun-if-changed={}", f.display());
        src += &format!("    ({rel:?}, include_bytes!({:?})),\n", f.display().to_string());
    }
    src += "];\n";
    std::fs::write(out.join("model_bundle.rs"), src).unwrap();
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir).expect("assets/models is readable").flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            walk(&path, out);
        } else {
            out.push(path);
        }
    }
}

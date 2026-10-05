//! Bakes the bundled ISF shaders (`assets/isf`) into the binary: writes `isf_bundle.rs` with
//! an `include_str!` for every `.fs` / `.vs` file, so the library needs nothing on disk.

use std::path::{Path, PathBuf};

fn collect(root: &Path, dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir).expect("assets/isf is readable").flatten().map(|e| e.path()).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect(root, &path, out);
        } else if path.extension().is_some_and(|x| x == "fs" || x == "vs") {
            out.push(path);
        }
    }
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/isf");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut files = Vec::new();
    collect(&root, &root, &mut files);
    let mut src = String::from("/// (path inside assets/isf, contents) of every bundled shader file.\npub static FILES: &[(&str, &str)] = &[\n");
    for f in &files {
        // Rerun when a file is edited, not just when the folder's listing changes.
        println!("cargo:rerun-if-changed={}", f.display());
        let rel = f.strip_prefix(&root).unwrap().to_string_lossy().replace('\\', "/");
        src += &format!("    ({rel:?}, include_str!({:?})),\n", f.display().to_string());
    }
    src += "];\n";
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("isf_bundle.rs");
    std::fs::write(out, src).unwrap();
}

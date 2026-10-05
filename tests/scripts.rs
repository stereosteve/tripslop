//! End-to-end tests: run the real app on the scripts in `scripts/`.
//!
//! They open a window and need a GPU, so they're opt-in:
//!     cargo test --release -- --ignored
//! Captures land in `target/script-out/`.

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(180);

fn run(script: &str) {
    let root = env!("CARGO_MANIFEST_DIR");
    let mut child = Command::new(env!("CARGO_BIN_EXE_tripslop"))
        .args(["--script", script])
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to start tripslop");
    let start = Instant::now();
    let status = loop {
        if let Some(s) = child.try_wait().expect("wait failed") {
            break s;
        }
        if start.elapsed() > TIMEOUT {
            let _ = child.kill();
            panic!("{script} timed out after {TIMEOUT:?} (missing `quit`?)");
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let out = child.wait_with_output().expect("no output");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    println!("{stdout}");
    eprintln!("{stderr}");
    assert!(status.success(), "{script} failed ({status}):\n{stderr}");
}

#[test]
#[ignore = "opens a window; needs a GPU"]
fn smoke() {
    run("scripts/smoke.tripslop");
}

#[test]
#[ignore = "opens a window; needs a GPU"]
fn shaders() {
    run("scripts/shaders.tripslop");
}

#[test]
#[ignore = "opens a window; needs a GPU"]
fn punch_tour() {
    run("scripts/punch-tour.tripslop");
}

#[test]
#[ignore = "opens a window; needs a GPU"]
fn shape_projector() {
    run("scripts/shape.tripslop");
}

#[test]
#[ignore = "opens a window; needs a GPU"]
fn projection_mapping() {
    run("scripts/projection.tripslop");
}

#[test]
#[ignore = "opens a window; needs a GPU"]
fn crossfade() {
    run("scripts/crossfade.tripslop");
    let px = |name: &str| -> [f32; 3] {
        let path = format!("{}/target/script-out/crossfade/{name}.png", env!("CARGO_MANIFEST_DIR"));
        let img = image::open(&path).unwrap_or_else(|e| panic!("{path}: {e}")).to_rgb8();
        let p = img.get_pixel(img.width() / 2, img.height() / 2).0;
        println!("{name}: {p:?}");
        p.map(|c| c as f32)
    };
    let close = |what: &str, got: [f32; 3], want: [f32; 3]| {
        assert!(got.iter().zip(want).all(|(g, w)| (g - w).abs() <= 2.0), "{what}: got {got:?}, want {want:?}");
    };
    let lerp = |a: [f32; 3], b: [f32; 3], t: f32| [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t);
    let (a, b) = (px("a"), px("b"));
    assert!(a.iter().zip(b).any(|(x, y)| (x - y).abs() > 40.0), "the banks should look different: {a:?} vs {b:?}");
    // The middle is a true 50/50 dissolve, with the unassigned layer still on top.
    close("mid", px("mid"), lerp(a, b, 0.5));
    // Smooth curve at 0.25: 0.25² · (3 − 2 · 0.25) = 0.15625 of B.
    close("smooth quarter", px("smooth-quarter"), lerp(a, b, 0.15625));
    // Layer-opacity mode: both sides are at full opacity in the middle, so the opaque A layer
    // on top hides B completely.
    close("layer mid", px("layer-mid"), a);
    // The unassigned layer is part of the banks (here: hidden again, bank A changes).
    let alone = px("a-alone");
    assert!(alone.iter().zip(a).any(|(x, y)| (x - y).abs() > 10.0), "unassigned layer missing from bank A");
}

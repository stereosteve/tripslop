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
    let mut child = Command::new(env!("CARGO_BIN_EXE_trippy"))
        .args(["--script", script])
        .current_dir(root)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to start trippy");
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
    run("scripts/smoke.trippy");
}

#[test]
#[ignore = "opens a window; needs a GPU"]
fn shaders() {
    run("scripts/shaders.trippy");
}

#[test]
#[ignore = "opens a window; needs a GPU"]
fn punch_tour() {
    run("scripts/punch-tour.trippy");
}

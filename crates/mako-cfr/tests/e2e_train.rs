//! M-24 end-to-end: the `mako-train` BINARY trains the CI tiny artifact
//! from `configs/ci_tiny.toml`, and a straight run equals a checkpoint +
//! resume run byte for byte. The artifact loads through the library and is
//! labelled as a weak plumbing fixture. Outputs go to cargo's test temp dir.

use std::path::{Path, PathBuf};
use std::process::Command;

use mako_cfr::infer::{Agent, CfrAgent};
use mako_core::rules::{RulesConfig, StackDepth};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn out_dir(name: &str) -> PathBuf {
    let p = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("e2e-train-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    p
}

fn train(args: &[&str]) {
    let o = Command::new(env!("CARGO_BIN_EXE_mako-train"))
        .current_dir(repo())
        .args(["--config", "configs/ci_tiny.toml", "--quiet"])
        .args(args)
        .output()
        .expect("spawn mako-train");
    assert!(
        o.status.success(),
        "mako-train {args:?} failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
}

#[test]
fn e2e_train_binary_resume_is_byte_identical() {
    let straight = out_dir("straight");
    train(&["--out", straight.to_str().unwrap()]);
    let half = out_dir("half");
    train(&["--iterations", "1500", "--out", half.to_str().unwrap()]);
    let resumed = out_dir("resumed");
    let ckpt = half.join("ci_tiny_20bb.ckpt");
    train(&[
        "--resume",
        ckpt.to_str().unwrap(),
        "--out",
        resumed.to_str().unwrap(),
    ]);
    let a = std::fs::read(straight.join("ci_tiny_20bb.mako")).unwrap();
    let b = std::fs::read(resumed.join("ci_tiny_20bb.mako")).unwrap();
    let h = std::fs::read(half.join("ci_tiny_20bb.mako")).unwrap();
    assert_eq!(a, b, "straight 3000 vs 1500 + resume to 3000");
    assert_ne!(a, h, "the half run is a different (earlier) artifact");

    // Loads at its depth, refused at others; labelled as a weak fixture.
    let agent = CfrAgent::from_artifact_bytes(&a, &RulesConfig::standard(StackDepth::Bb20), 1)
        .expect("ci tiny artifact loads at 20BB");
    let label = agent.label();
    assert!(
        label.contains("weak") && label.contains("not GTO"),
        "{label}"
    );
    for d in [StackDepth::Bb50, StackDepth::Bb100] {
        assert!(CfrAgent::from_artifact_bytes(&a, &RulesConfig::standard(d), 1).is_err());
    }
    for d in [straight, half, resumed] {
        let _ = std::fs::remove_dir_all(d);
    }
}

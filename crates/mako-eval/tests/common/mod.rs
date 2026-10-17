//! Shared test fixtures: a tiny artifact trained in-process.
#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::AtomicBool;

use mako_cfr::artifact::{ArtifactMetadata, StrategyArtifact};
use mako_cfr::train::{TrainConfig, train};

/// Training seed of the tiny fixture.
pub const TINY_TRAIN_SEED: u64 = 77;

/// Bytes of a tiny 20BB artifact (3/3/3 buckets, 300 iterations), trained
/// once per test binary. A plumbing fixture with no strength claim.
pub fn tiny_artifact() -> &'static [u8] {
    static BYTES: OnceLock<Vec<u8>> = OnceLock::new();
    BYTES.get_or_init(|| {
        let cfg = TrainConfig::tiny(TINY_TRAIN_SEED, 300);
        let out = train(&cfg, None, &AtomicBool::new(false), &mut |_| {}).expect("train");
        let meta = ArtifactMetadata::from_trainer(&out.trainer, "test fixture: tiny, no claims");
        StrategyArtifact::from_table(out.trainer.table(), meta)
            .expect("artifact")
            .to_bytes()
    })
}

/// A fresh scratch directory under cargo's target tmp dir.
pub fn scratch(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("mako-eval-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("mkdir");
    d
}

/// Writes the tiny artifact into `dir` and returns its path.
pub fn write_tiny(dir: &std::path::Path) -> PathBuf {
    let p = dir.join("tiny.mako");
    std::fs::write(&p, tiny_artifact()).expect("write artifact");
    p
}

/// Workspace root (two levels above this crate).
pub fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

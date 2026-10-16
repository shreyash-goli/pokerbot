//! M-13: `mako-train` CLI and config parser. Tests are prefixed `train_cli_`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use mako_cfr::artifact::Policy;
use mako_cfr::config::{MAX_CONFIG_ITERATIONS, RunConfig};

const BIN: &str = env!("CARGO_BIN_EXE_mako-train");

const TINY: &str = r#"# tiny test config
[rules]
depth_bb = 20

[abstraction]
flop_buckets = 3
turn_buckets = 3
river_buckets = 3
flop_samples = 32
turn_samples = 32

[train]
seed = 7
iterations = 300
checkpoint_every = 100
max_wall_secs = 120
linear_averaging = true   # inline comment

[output]
dir = "unused"
artifact = "t.mako"
checkpoint = "t.ckpt"
manifest = "t.json"
label = "smoke: trained for plumbing, not competition"
"#;

fn tmpdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("mako-train-cli-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write_cfg(dir: &Path, text: &str) -> PathBuf {
    let p = dir.join("cfg.toml");
    std::fs::write(&p, text).unwrap();
    p
}

fn run(args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .output()
        .expect("run mako-train")
}

fn train_into(dir: &Path, cfg: &Path, extra: &[&str]) -> Output {
    let mut a = vec![
        "--config",
        cfg.to_str().unwrap(),
        "--out",
        dir.to_str().unwrap(),
        "--quiet",
    ];
    a.extend_from_slice(extra);
    run(&a)
}

fn manifest_without_timing(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("t.json"))
        .unwrap()
        .lines()
        .filter(|l| {
            ![
                "\"wall_secs\"",
                "\"it_per_sec\"",
                "\"artifact_path\"",
                "\"checkpoint_path\"",
            ]
            .iter()
            .any(|k| l.contains(k))
        })
        .map(str::to_string)
        .collect()
}

#[test]
fn train_cli_trains_and_artifact_loads() {
    let d = tmpdir("basic");
    let cfg = write_cfg(&d, TINY);
    let out = train_into(&d, &cfg, &[]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let bytes = std::fs::read(d.join("t.mako")).unwrap();
    let rc = RunConfig::parse(TINY).unwrap();
    let p = Policy::load(&bytes, &rc.train.rules, &rc.train.abstraction).unwrap();
    assert!(!p.is_empty());
    assert_eq!(p.metadata().iterations, 300);
    assert_eq!(
        p.metadata().label,
        "smoke: trained for plumbing, not competition"
    );
    assert!(d.join("t.ckpt").exists() && !d.join("t.ckpt.tmp").exists());
    let m = std::fs::read_to_string(d.join("t.json")).unwrap();
    for needle in [
        "\"label\": \"smoke: trained for plumbing, not competition\"",
        "\"iterations_completed\": 300",
        "\"seed\": 7",
        "\"stop_reason\": \"completed\"",
        "\"artifact_fnv1a64\"",
        "\"abstraction_fingerprint\"",
        "\"config_fnv1a64\"",
        "\"git_commit\"",
    ] {
        assert!(m.contains(needle), "manifest lacks {needle}\n{m}");
    }
    // No strength claims (the only mention is the explicit disclaimer).
    let lower = m.to_lowercase();
    assert!(lower.contains("no strength, nash or gto claim"));
    for bad in ["superhuman", "unexploitable", "optimal play", "solved"] {
        assert!(!lower.contains(bad), "manifest claims {bad}");
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn train_cli_runs_are_reproducible_and_resume_matches() {
    let (a, b, c) = (tmpdir("rep-a"), tmpdir("rep-b"), tmpdir("rep-c"));
    let cfg = write_cfg(&a, TINY);
    for d in [&a, &b] {
        let o = train_into(d, &cfg, &[]);
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    }
    let art_a = std::fs::read(a.join("t.mako")).unwrap();
    assert_eq!(art_a, std::fs::read(b.join("t.mako")).unwrap());
    assert_eq!(manifest_without_timing(&a), manifest_without_timing(&b));

    // Stop at 120, then resume to the config's 300.
    let o = train_into(&c, &cfg, &["--iterations", "120"]);
    assert!(o.status.success());
    let mid = c.join("mid.ckpt");
    std::fs::rename(c.join("t.ckpt"), &mid).unwrap();
    let o = train_into(&c, &cfg, &["--resume", mid.to_str().unwrap()]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(std::fs::read(c.join("t.mako")).unwrap(), art_a);
    let m = std::fs::read_to_string(c.join("t.json")).unwrap();
    assert!(m.contains("\"resumed_from\": 120"));
    for d in [a, b, c] {
        let _ = std::fs::remove_dir_all(&d);
    }
}

#[test]
fn train_cli_bad_configs_fail_with_line_numbers() {
    let d = tmpdir("bad");
    let cases: Vec<(String, &str)> = vec![
        (
            TINY.replace("flop_samples = 32", "flop_samples = 32 32"),
            "line 9",
        ),
        (TINY.replace("seed = 7", "seeed = 7"), "line 13"),
        (TINY.replace("[train]", "[trian]"), "line 12"),
        (TINY.replace("depth_bb = 20", "depth_bb = \"20\""), "line 3"),
        (
            TINY.replace("iterations = 300\n", ""),
            "missing required key `iterations`",
        ),
        (
            TINY.replace("label = \"smoke", "label = \"smoke\\q"),
            "line 24",
        ),
    ];
    for (text, want) in cases {
        let cfg = write_cfg(&d, &text);
        let o = train_into(&d, &cfg, &[]);
        let stderr = String::from_utf8_lossy(&o.stderr);
        assert_eq!(o.status.code(), Some(1), "{want}: {stderr}");
        assert!(stderr.contains(want), "expected {want:?} in {stderr}");
    }
    // Missing file / missing --config / unknown flag.
    assert_eq!(
        run(&["--config", "/nonexistent/x.toml"]).status.code(),
        Some(1)
    );
    assert_eq!(run(&[]).status.code(), Some(1));
    assert_eq!(run(&["--bogus"]).status.code(), Some(1));
    let help = run(&["--help"]);
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("Hard cap: 100000000"));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn train_cli_iteration_cap_enforced() {
    let d = tmpdir("cap");
    let cfg = write_cfg(&d, TINY);
    let over = (MAX_CONFIG_ITERATIONS + 1).to_string();
    for bad in [over.as_str(), "0", "abc"] {
        let o = train_into(&d, &cfg, &["--iterations", bad]);
        assert_eq!(o.status.code(), Some(1), "--iterations {bad}");
    }
    let text = TINY.replace("iterations = 300", &format!("iterations = {over}"));
    let o = train_into(&d, &write_cfg(&d, &text), &[]);
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("iterations"));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn train_cli_resume_with_mismatched_config_fails() {
    let d = tmpdir("mismatch");
    let cfg = write_cfg(&d, TINY);
    assert!(
        train_into(&d, &cfg, &["--iterations", "50"])
            .status
            .success()
    );
    let ck = d.join("first.ckpt");
    std::fs::rename(d.join("t.ckpt"), &ck).unwrap();
    let other = write_cfg(&d, &TINY.replace("seed = 7", "seed = 8"));
    let o = train_into(&d, &other, &["--resume", ck.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("different seed"));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn train_cli_parser_accepts_repo_smoke_config() {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../configs/smoke.toml"
    ))
    .unwrap();
    let c = RunConfig::parse(&text).unwrap();
    assert_eq!(c.train.rules.starting_stack, 40);
    assert_eq!(c.train.rules.big_blind, 2);
    assert!(c.train.max_wall_secs.unwrap() <= 300);
    assert!(c.train.abstraction.turn_samples > 0);
    assert_eq!(
        c.output.label,
        "smoke: trained for plumbing, not competition"
    );
}

/// Sanity readout from the trained smoke artifact (not a strength claim).
/// Run after `scripts/train_smoke.sh`:
/// `cargo test --release -p mako-cfr --test train_cli -- --ignored --nocapture`.
#[test]
#[ignore = "needs artifacts/smoke_20bb.mako from a smoke run"]
fn train_cli_smoke_artifact_sanity_readout() {
    use mako_cfr::abstraction::preflop::PreflopClass;
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../artifacts/smoke_20bb.mako"
    );
    let bytes = std::fs::read(path).expect("run scripts/train_smoke.sh first");
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../configs/smoke.toml"
    ))
    .unwrap();
    let c = RunConfig::parse(&text).unwrap();
    let p = Policy::load(&bytes, &c.train.rules, &c.train.abstraction).unwrap();
    let names = ["fold", "check", "call", "half", "pot", "allin"];
    let show = |label: &str, key: &[u8]| {
        let e = p.lookup_bytes(key).map(|e| e.probs());
        let s = match e {
            None => "no entry".to_string(),
            Some(pr) => names
                .iter()
                .zip(pr)
                .filter(|(_, v)| *v > 0.0)
                .map(|(n, v)| format!("{n} {v:.3}"))
                .collect::<Vec<_>>()
                .join(", "),
        };
        eprintln!("{label}: {s}");
    };
    for h in ["AA", "AKs", "T9s", "72o"] {
        let b = h.parse::<PreflopClass>().unwrap().index();
        show(&format!("BTN first action {h}"), &[1, 0, 0, b, 0, 0]);
        // Big blind facing a button all-in (history: AllIn).
        show(&format!("BB vs BTN shove {h}"), &[1, 0, 1, b, 0, 1, 5]);
    }
}

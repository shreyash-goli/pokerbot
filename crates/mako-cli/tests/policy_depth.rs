//! M-16: policy artifacts must match the configured stack depth. Runs the
//! real `mako` binary. Tiny artifacts are trained in-process (a few hundred
//! iterations; plumbing fixtures with no strength claim). Each run uses an
//! empty working directory so the default `artifacts/smoke_<bb>bb.mako`
//! path resolves to nothing unless a test puts a file there.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use mako_cfr::artifact::{ArtifactMetadata, StrategyArtifact};
use mako_cfr::train::{TrainConfig, train};
use mako_core::rules::{RulesConfig, StackDepth};

const LABEL: &str = "test: tiny policy_depth fixture, no strength claim";

/// Tiny artifact for `depth` (trained once per test binary per depth).
fn tiny_bytes(depth: StackDepth) -> &'static [u8] {
    static B20: OnceLock<Vec<u8>> = OnceLock::new();
    static B50: OnceLock<Vec<u8>> = OnceLock::new();
    let cell = match depth {
        StackDepth::Bb20 => &B20,
        StackDepth::Bb50 => &B50,
        StackDepth::Bb100 => panic!("no 100BB fixture is trained in these tests"),
    };
    cell.get_or_init(|| {
        let mut cfg = TrainConfig::tiny(41, 300);
        cfg.rules = RulesConfig::standard(depth);
        let out = train(&cfg, None, &AtomicBool::new(false), &mut |_| {}).unwrap();
        let meta = ArtifactMetadata::from_trainer(&out.trainer, LABEL);
        StrategyArtifact::from_table(out.trainer.table(), meta)
            .unwrap()
            .to_bytes()
    })
}

/// A fresh empty directory under cargo's test temp dir.
fn empty_dir(name: &str) -> PathBuf {
    let p = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("policy-depth-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn write_artifact(dir: &Path, rel: &str, depth: StackDepth) -> PathBuf {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, tiny_bytes(depth)).unwrap();
    p
}

/// A fresh per-invocation sandbox under cargo's test temp dir, used as the
/// child's `MAKO_HOME` and `HOME` (set on the child `Command` only), so no
/// run can ever touch the real home directory or `~/.mako`.
fn sandbox_home() -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "policy-depth-home-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&d).expect("sandbox home");
    d
}

/// Runs `mako` in `cwd` with a fixed environment and empty stdin. Every
/// invocation gets `--no-history` plus its own sandboxed `MAKO_HOME`/`HOME`.
fn mako(cwd: &Path, args: &[&str]) -> Output {
    let home = sandbox_home();
    let mut child = Command::new(env!("CARGO_BIN_EXE_mako"))
        .args(args)
        .arg("--no-history")
        .current_dir(cwd)
        .env("MAKO_HOME", &home)
        .env("HOME", &home)
        .env_remove("NO_COLOR")
        .env_remove("COLUMNS")
        .env_remove("COLORTERM")
        .env("LC_ALL", "C")
        .env("TERM", "xterm")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn mako");
    child.stdin.take().unwrap().write_all(b"").unwrap();
    child.wait_with_output().expect("wait for mako")
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn bb(d: StackDepth) -> String {
    d.big_blinds().to_string()
}

const SESSION: [&str; 7] = [
    "--seed",
    "5",
    "--hands",
    "3",
    "--no-color",
    "--auto",
    "checkcall",
];

fn session<'a>(extra: &[&'a str]) -> Vec<&'a str> {
    let mut v: Vec<&str> = extra.to_vec();
    v.extend_from_slice(&SESSION);
    v
}

/// The header and first table show the depth's stack (blinds posted).
fn assert_stacks_shown(o: &str, d: StackDepth) {
    let s = d.starting_stack();
    assert!(
        o.contains(&format!("{} BB deep ({s} chips)", d.big_blinds())),
        "{o}"
    );
    let first_hand = o.split("-- Hand 1").nth(1).expect("hand 1");
    assert!(first_hand.contains(&format!("stack {:>4}", s - 1)), "{o}");
    assert!(first_hand.contains(&format!("stack {:>4}", s - 2)), "{o}");
}

#[test]
fn policy_depth_cfr_without_artifact_exits_2() {
    for d in StackDepth::ALL {
        let dir = empty_dir(&format!("missing{}", d.big_blinds()));
        let o = mako(&dir, &session(&["--bot", "cfr", "--depth", &bb(d)]));
        assert_eq!(o.status.code(), Some(2), "{}", err(&o));
        let e = err(&o);
        assert!(
            e.contains(&format!("no trained policy for {}BB", d.big_blinds())),
            "{e}"
        );
        assert!(
            e.contains(&format!("artifacts/smoke_{}bb.mako", d.big_blinds())),
            "{e}"
        );
        assert!(out(&o).is_empty() || !out(&o).contains("-- Hand 1"));
    }
}

#[test]
fn policy_depth_cfr_wrong_depth_artifact_exits_2() {
    let dir = empty_dir("wrong");
    let a20 = write_artifact(&dir, "a20.mako", StackDepth::Bb20);
    let a50 = write_artifact(&dir, "a50.mako", StackDepth::Bb50);
    let cases = [
        (&a20, StackDepth::Bb50),
        (&a20, StackDepth::Bb100),
        (&a50, StackDepth::Bb20),
        (&a50, StackDepth::Bb100),
    ];
    for (art, d) in cases {
        let o = mako(
            &dir,
            &session(&[
                "--bot",
                "cfr",
                "--depth",
                &bb(d),
                "--strategy",
                art.to_str().unwrap(),
            ]),
        );
        assert_eq!(o.status.code(), Some(2), "{}", err(&o));
        let e = err(&o);
        assert!(e.contains("wrong depth"), "{e}");
        assert!(e.contains(&format!("{}BB", d.big_blinds())), "{e}");
    }
}

#[test]
fn policy_depth_auto_plays_loud_untrained_baseline() {
    for d in [StackDepth::Bb50, StackDepth::Bb100] {
        let dir = empty_dir(&format!("auto{}", d.big_blinds()));
        let o = mako(&dir, &session(&["--bot", "auto", "--depth", &bb(d)]));
        let s = out(&o);
        assert!(o.status.success(), "{}\n{s}", err(&o));
        assert!(
            s.contains(&format!("No trained policy for {}BB", d.big_blinds())),
            "{s}"
        );
        assert!(s.contains("Playing the UNTRAINED baseline"), "{s}");
        assert!(!s.contains("Opponent: CFR policy"), "{s}");
        assert!(s.contains("Session complete."), "{s}");
        assert!(s.contains("Final: 3 hand(s) played."), "{s}");
        assert_stacks_shown(&s, d);
    }
    // A wrong-depth artifact under --bot auto also falls back loudly.
    let dir = empty_dir("auto-wrong");
    let a20 = write_artifact(&dir, "a20.mako", StackDepth::Bb20);
    let o = mako(
        &dir,
        &session(&[
            "--bot",
            "auto",
            "--depth",
            "100",
            "--strategy",
            a20.to_str().unwrap(),
        ]),
    );
    let s = out(&o);
    assert!(o.status.success());
    assert!(s.contains("wrong depth"), "{s}");
    assert!(s.contains("Playing the UNTRAINED baseline"), "{s}");
}

#[test]
fn policy_depth_20_with_valid_artifact_uses_cfr() {
    let dir = empty_dir("cfr20");
    let a20 = write_artifact(&dir, "a20.mako", StackDepth::Bb20);
    let o = mako(
        &dir,
        &session(&[
            "--bot",
            "cfr",
            "--depth",
            "20",
            "--strategy",
            a20.to_str().unwrap(),
        ]),
    );
    let s = out(&o);
    assert!(o.status.success(), "{}\n{s}", err(&o));
    assert!(s.contains("Opponent: CFR policy"), "{s}");
    assert!(s.contains("20BB"), "{s}");
    assert!(s.contains(LABEL), "{s}");
    assert!(!s.contains("UNTRAINED"), "{s}");
    assert!(s.contains("Session complete."));
    assert_stacks_shown(&s, StackDepth::Bb20);
}

#[test]
fn policy_depth_50_loads_a_50bb_artifact_when_present() {
    // Explicit --strategy with --bot cfr.
    let dir = empty_dir("cfr50");
    let a50 = write_artifact(&dir, "a50.mako", StackDepth::Bb50);
    let o = mako(
        &dir,
        &session(&[
            "--bot",
            "cfr",
            "--depth",
            "50",
            "--strategy",
            a50.to_str().unwrap(),
        ]),
    );
    let s = out(&o);
    assert!(o.status.success(), "{}\n{s}", err(&o));
    assert!(s.contains("Opponent: CFR policy"), "{s}");
    assert!(s.contains("50BB"), "{s}");
    assert!(!s.contains("UNTRAINED"), "{s}");
    assert_stacks_shown(&s, StackDepth::Bb50);
    // Default path resolution per depth: artifacts/smoke_50bb.mako in cwd
    // is picked up by --bot auto at --depth 50 ...
    let dir = empty_dir("auto50-default");
    write_artifact(&dir, "artifacts/smoke_50bb.mako", StackDepth::Bb50);
    let o = mako(&dir, &session(&["--bot", "auto", "--depth", "50"]));
    let s = out(&o);
    assert!(o.status.success(), "{}\n{s}", err(&o));
    assert!(s.contains("Opponent: CFR policy"), "{s}");
    assert!(!s.contains("UNTRAINED"), "{s}");
    // ... but not at --depth 100, which looks for smoke_100bb.mako.
    let o = mako(&dir, &session(&["--bot", "auto", "--depth", "100"]));
    let s = out(&o);
    assert!(o.status.success());
    assert!(s.contains("No trained policy for 100BB"), "{s}");
    assert!(s.contains("Playing the UNTRAINED baseline"), "{s}");
}

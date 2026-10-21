//! `pokerbot-train`: bounded MCCFR training and `.pokerbot` export.
//!
//! Usage: `pokerbot-train --config <file> [--resume <ckpt>] [--iterations <n>]
//! [--out <dir>] [--quiet]`. Run `pokerbot-train --help` for details.
//!
//! Exit codes: 0 = completed; 2 = stopped early by `max_wall_secs` (the
//! checkpoint, artifact and manifest are still written and can be resumed);
//! 1 = error (bad arguments, bad config, I/O, incompatible checkpoint).
//!
//! Interruption: the Rust standard library has no signal API, and the
//! workspace forbids `unsafe` code, so Ctrl-C is not intercepted. A killed
//! run loses at most `checkpoint_every` iterations: rerun with `--resume`
//! on the last checkpoint, which continues bit-identically.

#[cfg(not(target_arch = "wasm32"))]
mod cli {
    use std::path::PathBuf;
    use std::process::ExitCode;
    use std::sync::atomic::AtomicBool;
    use std::time::Instant;

    use pokerbot_cfr::abstraction::key::fnv1a64;
    use pokerbot_cfr::artifact::{ArtifactMetadata, Policy, StrategyArtifact};
    use pokerbot_cfr::checkpoint::{load_file, save_atomic};
    use pokerbot_cfr::config::{MAX_CONFIG_ITERATIONS, RunConfig};
    use pokerbot_cfr::train::{StopReason, TrainEvent, train};

    const HELP: &str = "\
    pokerbot-train: train an external-sampling MCCFR policy and export a .pokerbot artifact

    USAGE:
        pokerbot-train --config <file> [OPTIONS]

    OPTIONS:
        --config <file>      Run configuration (see configs/smoke.toml). Required.
        --resume <ckpt>      Continue from a checkpoint. Its rules, abstraction,
                             flags and seed must match the config.
        --iterations <n>     Override the total iteration target. Hard cap: 100000000.
        --out <dir>          Override [output] dir.
        --quiet              Only print errors and the final summary.
        -h, --help           Show this help.

    Outputs (in the output dir): checkpoint, artifact and JSON manifest, each
    written atomically (temp file + rename). Exit codes: 0 completed, 2 stopped by
    max_wall_secs (outputs written; resume later), 1 error.
    Ctrl-C is not intercepted (no std signal API, no unsafe code): resume from the
    last periodic checkpoint instead. The policy is a plumbing smoke test unless the
    label says otherwise; no strength or GTO claim is made.
    ";

    struct Args {
        config: PathBuf,
        resume: Option<PathBuf>,
        iterations: Option<u64>,
        out: Option<PathBuf>,
        quiet: bool,
    }

    fn parse_args() -> Result<Option<Args>, String> {
        let mut it = std::env::args().skip(1);
        let mut config = None;
        let mut resume = None;
        let mut iterations = None;
        let mut out = None;
        let mut quiet = false;
        while let Some(a) = it.next() {
            let mut val = |name: &str| it.next().ok_or_else(|| format!("{name} needs a value"));
            match a.as_str() {
                "-h" | "--help" => return Ok(None),
                "--config" => config = Some(PathBuf::from(val("--config")?)),
                "--resume" => resume = Some(PathBuf::from(val("--resume")?)),
                "--out" => out = Some(PathBuf::from(val("--out")?)),
                "--iterations" => {
                    let v = val("--iterations")?;
                    let n: u64 = v
                        .replace('_', "")
                        .parse()
                        .map_err(|_| format!("--iterations: not a number: {v}"))?;
                    if n == 0 || n > MAX_CONFIG_ITERATIONS {
                        return Err(format!(
                            "--iterations must be in 1..={MAX_CONFIG_ITERATIONS}, got {n}"
                        ));
                    }
                    iterations = Some(n);
                }
                "--quiet" => quiet = true,
                other => return Err(format!("unknown argument {other:?} (see --help)")),
            }
        }
        let config = config.ok_or("--config <file> is required (see --help)")?;
        Ok(Some(Args {
            config,
            resume,
            iterations,
            out,
            quiet,
        }))
    }

    fn json_str(s: &str) -> String {
        let mut o = String::with_capacity(s.len() + 2);
        o.push('"');
        for c in s.chars() {
            match c {
                '"' => o.push_str("\\\""),
                '\\' => o.push_str("\\\\"),
                '\n' => o.push_str("\\n"),
                '\r' => o.push_str("\\r"),
                '\t' => o.push_str("\\t"),
                c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
                c => o.push(c),
            }
        }
        o.push('"');
        o
    }

    fn git_commit() -> Option<String> {
        let out = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
        (!s.is_empty()).then_some(s)
    }

    fn run(args: Args) -> Result<ExitCode, String> {
        let text = std::fs::read_to_string(&args.config)
            .map_err(|e| format!("cannot read {}: {e}", args.config.display()))?;
        let mut cfg =
            RunConfig::parse(&text).map_err(|e| format!("{}: {e}", args.config.display()))?;
        if let Some(n) = args.iterations {
            cfg.train.iterations = n;
        }
        let dir = args
            .out
            .clone()
            .unwrap_or_else(|| PathBuf::from(&cfg.output.dir));
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        let ckpt_path = dir.join(&cfg.output.checkpoint);
        let art_path = dir.join(&cfg.output.artifact);
        let manifest_path = dir.join(&cfg.output.manifest);

        let resume = match &args.resume {
            Some(p) => Some(load_file(p).map_err(|e| format!("{}: {e}", p.display()))?),
            None => None,
        };
        let resumed_from = resume.as_ref().map(|c| c.iteration);

        let quiet = args.quiet;
        let start = Instant::now();
        let start_iter = resumed_from.unwrap_or(0);
        let mut save_err: Option<String> = None;
        let cancel = AtomicBool::new(false);
        let outcome = train(&cfg.train, resume, &cancel, &mut |e| {
            if let TrainEvent::Checkpoint(cp) = e {
                if let Err(err) = save_atomic(&ckpt_path, &cp.to_bytes()) {
                    save_err.get_or_insert(format!("{}: {err}", ckpt_path.display()));
                }
                if !quiet {
                    let secs = start.elapsed().as_secs_f64();
                    let done = cp.iteration - start_iter;
                    eprintln!(
                        "[pokerbot-train] iter {}/{}  {:.0} it/s  infosets {}  elapsed {:.1}s",
                        cp.iteration,
                        cfg.train.iterations,
                        done as f64 / secs.max(1e-9),
                        cp.table.len(),
                        secs
                    );
                }
            }
        })
        .map_err(|e| e.to_string())?;
        if let Some(e) = save_err {
            return Err(format!("checkpoint write failed: {e}"));
        }
        let wall = start.elapsed().as_secs_f64();
        let t = &outcome.trainer;
        let run_iters = outcome.iterations_run;

        let meta = ArtifactMetadata::from_trainer(t, cfg.output.label.clone());
        let artifact = StrategyArtifact::from_table(t.table(), meta).map_err(|e| e.to_string())?;
        let bytes = artifact.to_bytes();
        // Self-check: the artifact must load with the config's rules/abstraction.
        Policy::load(&bytes, &cfg.train.rules, &cfg.train.abstraction)
            .map_err(|e| format!("exported artifact failed to load: {e}"))?;
        save_atomic(&art_path, &bytes).map_err(|e| format!("{}: {e}", art_path.display()))?;

        let checksum = artifact.checksum();
        let omitted = t.table().len() - artifact.entries.len();
        let reason = match outcome.reason {
            StopReason::Completed => "completed",
            StopReason::Cancelled => "cancelled",
            StopReason::WallClock => "max_wall_secs",
        };
        let r = cfg.train.rules;
        let w = cfg.train.weighting;
        let fields: Vec<(&str, String)> = vec![
            ("format", json_str("pokerbot-train-manifest v1")),
            ("label", json_str(&cfg.output.label)),
            (
                "claims",
                json_str("none: smoke/plumbing artifact; no strength, Nash or GTO claim"),
            ),
            ("trainer_id", json_str(&artifact.metadata.trainer_id)),
            (
                "git_commit",
                git_commit().map_or("null".into(), |c| json_str(&c)),
            ),
            ("config_path", json_str(&args.config.display().to_string())),
            (
                "config_fnv1a64",
                json_str(&format!("{:#018x}", fnv1a64(text.as_bytes()))),
            ),
            ("config_text", json_str(&text)),
            ("seed", t.config().seed.to_string()),
            ("rules_small_blind", r.small_blind.to_string()),
            ("rules_big_blind", r.big_blind.to_string()),
            ("rules_starting_stack", r.starting_stack.to_string()),
            (
                "abstraction_fingerprint",
                json_str(&format!("{:#018x}", cfg.train.abstraction.fingerprint())),
            ),
            ("linear_averaging", w.linear_averaging.to_string()),
            ("linear_regrets", w.linear_regrets.to_string()),
            (
                "regret_matching_plus",
                cfg.train.update.regret_matching_plus.to_string(),
            ),
            ("iterations_target", cfg.train.iterations.to_string()),
            ("iterations_completed", t.iteration().to_string()),
            (
                "resumed_from",
                resumed_from.map_or("null".into(), |i| i.to_string()),
            ),
            ("stop_reason", json_str(reason)),
            ("infosets", t.table().len().to_string()),
            ("artifact_entries", artifact.entries.len().to_string()),
            ("omitted_zero_average_mass", omitted.to_string()),
            ("artifact_path", json_str(&art_path.display().to_string())),
            ("artifact_bytes", bytes.len().to_string()),
            ("artifact_fnv1a64", json_str(&format!("{checksum:#018x}"))),
            (
                "checkpoint_path",
                json_str(&ckpt_path.display().to_string()),
            ),
            ("wall_secs", format!("{wall:.3}")),
            (
                "it_per_sec",
                format!("{:.1}", run_iters as f64 / wall.max(1e-9)),
            ),
        ];
        let mut manifest = String::from("{\n");
        for (i, (k, v)) in fields.iter().enumerate() {
            let comma = if i + 1 < fields.len() { "," } else { "" };
            manifest.push_str(&format!("  {}: {v}{comma}\n", json_str(k)));
        }
        manifest.push_str("}\n");
        save_atomic(&manifest_path, manifest.as_bytes())
            .map_err(|e| format!("{}: {e}", manifest_path.display()))?;

        println!(
            "pokerbot-train: {reason} at {} iterations in {wall:.1}s ({:.0} it/s); {} infosets, {} artifact entries; \
             {} ({} bytes, fnv1a64 {checksum:#018x}); manifest {}",
            t.iteration(),
            run_iters as f64 / wall.max(1e-9),
            t.table().len(),
            artifact.entries.len(),
            art_path.display(),
            bytes.len(),
            manifest_path.display()
        );
        Ok(match outcome.reason {
            StopReason::Completed => ExitCode::SUCCESS,
            _ => ExitCode::from(2),
        })
    }

    pub fn main() -> ExitCode {
        match parse_args() {
            Ok(None) => {
                print!("{HELP}");
                ExitCode::SUCCESS
            }
            Ok(Some(args)) => match run(args) {
                Ok(code) => code,
                Err(e) => {
                    eprintln!("pokerbot-train: error: {e}");
                    ExitCode::from(1)
                }
            },
            Err(e) => {
                eprintln!("pokerbot-train: error: {e}");
                ExitCode::from(1)
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> std::process::ExitCode {
    cli::main()
}

/// The trainer CLI needs a file system and process environment; on wasm32 it
/// is an inert stub so `cargo check --target wasm32-unknown-unknown` passes.
#[cfg(target_arch = "wasm32")]
fn main() {}

//! `pokerbot-eval` command-line tool. See `docs/EVALUATION.md`.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use pokerbot_eval::config::EvalConfig;
use pokerbot_eval::evaluate::{MAX_CLI_PAIRS, load_artifact, run_evaluation};
use pokerbot_eval::gate::Verdict;
use pokerbot_eval::output::{render_json, render_text};

const SMOKE_CONFIG: &str = "configs/eval_smoke.toml";

const USAGE: &str = "\
pokerbot-eval: paired-seed, seat-swapped evaluation of a .pokerbot policy artifact

USAGE:
    pokerbot-eval --smoke [OPTIONS]
    pokerbot-eval --config <path> [OPTIONS]

OPTIONS:
    --smoke              use configs/eval_smoke.toml (relative to the current directory)
    --config <path>      evaluation config (TOML subset; see docs/EVALUATION.md)
    --artifact <path>    override [policy] artifact
    --pairs <n>          override [eval] deal_pairs (1..=200000); each pair is 2 hands
    --json <path>        also write a machine-readable report
    --require-promote    exit with status 3 when the gate verdict is HOLD
    -h, --help           show this help

Exit status: 0 evaluation completed, 2 usage/config/artifact/match error,
3 HOLD with --require-promote. No GTO, Nash or exploitability claim is made.";

struct Args {
    config: PathBuf,
    artifact: Option<PathBuf>,
    pairs: Option<u64>,
    json: Option<PathBuf>,
    require_promote: bool,
}

fn parse_args(raw: Vec<String>) -> Result<Option<Args>, String> {
    let mut config: Option<PathBuf> = None;
    let mut smoke = false;
    let mut artifact = None;
    let mut pairs = None;
    let mut json = None;
    let mut require_promote = false;
    let mut it = raw.into_iter();
    let value = |it: &mut std::vec::IntoIter<String>, flag: &str| {
        it.next().ok_or_else(|| format!("{flag} needs a value"))
    };
    while let Some(a) = it.next() {
        match a.as_str() {
            "-h" | "--help" => return Ok(None),
            "--smoke" => smoke = true,
            "--config" => config = Some(value(&mut it, "--config")?.into()),
            "--artifact" => artifact = Some(value(&mut it, "--artifact")?.into()),
            "--json" => json = Some(value(&mut it, "--json")?.into()),
            "--require-promote" => require_promote = true,
            "--pairs" => {
                let v = value(&mut it, "--pairs")?;
                let n: u64 = v
                    .parse()
                    .map_err(|_| format!("--pairs: {v:?} is not a positive integer"))?;
                if n == 0 || n > MAX_CLI_PAIRS {
                    return Err(format!("--pairs must be in 1..={MAX_CLI_PAIRS}, got {n}"));
                }
                pairs = Some(n);
            }
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    let config = match (smoke, config) {
        (true, None) => PathBuf::from(SMOKE_CONFIG),
        (false, Some(c)) => c,
        (true, Some(_)) => return Err("use either --smoke or --config, not both".into()),
        (false, None) => return Err("one of --smoke or --config is required".into()),
    };
    Ok(Some(Args {
        config,
        artifact,
        pairs,
        json,
        require_promote,
    }))
}

fn run(args: Args) -> Result<Option<Verdict>, String> {
    let text = std::fs::read_to_string(&args.config)
        .map_err(|e| format!("cannot read config {}: {e}", args.config.display()))?;
    let cfg = EvalConfig::parse(&text).map_err(|e| format!("{}: {e}", args.config.display()))?;
    let path = args
        .artifact
        .clone()
        .unwrap_or_else(|| PathBuf::from(&cfg.artifact));
    let art = load_artifact(&path, &cfg.rules).map_err(|e| e.to_string())?;
    let start = Instant::now();
    let report = run_evaluation(&cfg, &art, args.pairs).map_err(|e| e.to_string())?;
    let secs = start.elapsed().as_secs_f64();
    print!("{}", render_text(&report));
    let hands: u64 = report
        .matchups
        .iter()
        .chain(&report.sanity)
        .map(|m| m.hands)
        .sum();
    println!(
        "\nruntime: {secs:.2}s for {hands} hands ({:.0} hands/s)",
        hands as f64 / secs.max(1e-9)
    );
    if let Some(j) = &args.json {
        std::fs::write(j, render_json(&report))
            .map_err(|e| format!("cannot write {}: {e}", j.display()))?;
        println!("json report: {}", j.display());
    }
    Ok(report.gate.map(|g| g.verdict))
}

fn main() -> ExitCode {
    let args = match parse_args(std::env::args().skip(1).collect()) {
        Ok(Some(a)) => a,
        Ok(None) => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("pokerbot-eval: {e}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let require = args.require_promote;
    match run(args) {
        Ok(Some(Verdict::Hold)) if require => ExitCode::from(3),
        Ok(None) if require => {
            eprintln!("pokerbot-eval: --require-promote: comparison mode gives no verdict");
            ExitCode::from(2)
        }
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("pokerbot-eval: error: {e}");
            ExitCode::from(2)
        }
    }
}

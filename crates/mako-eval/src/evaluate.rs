//! End-to-end evaluation: load and check the artifact, run every configured
//! matchup, summarize, and apply the promotion gate (promotion mode) or
//! report a fixed-vs-adaptive comparison (comparison mode).

use std::fmt;
use std::path::Path;

use mako_analysis::opponent::{AdaptConfig, AdaptiveAgent};
use mako_core::rules::RulesConfig;
use mako_core::state::Seat;

use crate::agent::{Agent, CfrAgent, InferStats};
use crate::baselines::{CheckCall, RandomLegal, TightRule};
use crate::config::{EvalConfig, Mode};
use crate::gate::{ChecksumStatus, GateDecision, GateInput, decide};
use crate::matchup::{
    DecisionCounts, MAX_PAIRS, MatchError, MatchResult, MatchSpec, PairResult, Participant,
    run_match, run_match_with,
};
use crate::report::{MatchupReport, summarize_match};
use crate::stats::{Summary, Z95, bootstrap_ci, summarize};

/// Printed with every comparison-mode report.
pub const COMPARISON_DISCLAIMER: &str = "Adaptive mode is experimental; this comparison does not \
establish general strength. It compares two agents against a few fixed rule bots on the same \
deals; no exploitability, Nash-distance or GTO measurement was performed.";

/// Hard cap for `--pairs` on the command line.
pub const MAX_CLI_PAIRS: u64 = 200_000;

/// Evaluation errors. None is papered over with a substitute.
#[derive(Debug)]
pub enum EvalError {
    /// The artifact file could not be read.
    ArtifactIo { path: String, error: String },
    /// The artifact is invalid or for another game.
    ArtifactInvalid { path: String, error: String },
    /// The configuration is inconsistent.
    Config(String),
    /// A match failed.
    Match(MatchError),
}

impl fmt::Display for EvalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EvalError::ArtifactIo { path, error } => {
                write!(f, "cannot read artifact {path}: {error}")
            }
            EvalError::ArtifactInvalid { path, error } => {
                write!(f, "artifact {path} rejected: {error}")
            }
            EvalError::Config(m) => write!(f, "{m}"),
            EvalError::Match(e) => write!(f, "match failed: {e}"),
        }
    }
}

impl std::error::Error for EvalError {}

impl From<MatchError> for EvalError {
    fn from(e: MatchError) -> Self {
        EvalError::Match(e)
    }
}

/// Artifact provenance as read from the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactInfo {
    pub path: String,
    pub size_bytes: usize,
    pub checksum: u64,
    pub training_seed: u64,
    pub abstraction_seed: u64,
    pub iterations: u64,
    pub label: String,
    pub trainer_id: String,
    pub infosets: usize,
    pub rules: RulesConfig,
}

/// A loaded artifact: bytes plus provenance.
#[derive(Clone, Debug)]
pub struct LoadedArtifact {
    pub bytes: Vec<u8>,
    pub info: ArtifactInfo,
}

impl LoadedArtifact {
    /// Builds a fresh agent for this artifact.
    pub fn agent(&self, seed: u64) -> Result<CfrAgent, EvalError> {
        CfrAgent::from_artifact_bytes(&self.bytes, &self.info.rules, seed).map_err(|e| {
            EvalError::ArtifactInvalid {
                path: self.info.path.clone(),
                error: e.to_string(),
            }
        })
    }
}

/// Reads and validates an artifact for `rules`. A missing or invalid file is
/// an error, never replaced by a default policy.
pub fn load_artifact(path: &Path, rules: &RulesConfig) -> Result<LoadedArtifact, EvalError> {
    let p = path.display().to_string();
    let bytes = std::fs::read(path).map_err(|e| EvalError::ArtifactIo {
        path: p.clone(),
        error: e.to_string(),
    })?;
    from_bytes(p, bytes, rules)
}

/// Validates in-memory artifact bytes (see [`load_artifact`]).
pub fn from_bytes(
    path: String,
    bytes: Vec<u8>,
    rules: &RulesConfig,
) -> Result<LoadedArtifact, EvalError> {
    let agent = CfrAgent::from_artifact_bytes(&bytes, rules, 0).map_err(|e| {
        EvalError::ArtifactInvalid {
            path: path.clone(),
            error: e.to_string(),
        }
    })?;
    let m = agent.metadata();
    let info = ArtifactInfo {
        path,
        size_bytes: bytes.len(),
        checksum: m.checksum,
        training_seed: m.seed,
        abstraction_seed: m.abstraction.seed,
        iterations: m.iterations,
        label: m.label.clone(),
        trainer_id: m.trainer_id.clone(),
        infosets: m.infosets,
        rules: m.rules,
    };
    Ok(LoadedArtifact { bytes, info })
}

/// Full evaluation report (deterministic for a given config and artifact).
#[derive(Clone, Debug, PartialEq)]
pub struct EvalReport {
    pub depth_bb: u32,
    pub rules: RulesConfig,
    pub base_seed: u64,
    pub pairs: u64,
    pub artifact: ArtifactInfo,
    pub checksum: ChecksumStatus,
    pub mode: Mode,
    pub candidate_label: String,
    /// Candidate (agent A) versus each configured opponent, in config order
    /// (candidates outer, opponents inner).
    pub matchups: Vec<MatchupReport>,
    /// Baseline-vs-baseline sanity matchups.
    pub sanity: Vec<MatchupReport>,
    /// Harness-counted candidate decisions over all matchups (both sides of
    /// the mirror).
    pub candidate_counts: DecisionCounts,
    /// Sum of the candidate agents' own `InferStats`.
    pub candidate_infer: InferStats,
    /// Promotion verdict (promotion mode only).
    pub gate: Option<GateDecision>,
    /// Fixed − adaptive differences (comparison of `cfr` and `adaptive_cfr`
    /// against the same opponent on the same deals).
    pub comparisons: Vec<Comparison>,
}

/// Paired difference `fixed − adaptive` against one opponent.
#[derive(Clone, Debug, PartialEq)]
pub struct Comparison {
    pub opponent_id: String,
    pub pairs: u64,
    /// BB/100 of (fixed result − adaptive result) per deal pair. Positive
    /// means the fixed policy did better on these deals.
    pub fixed_minus_adaptive_bb100: Summary,
    pub bootstrap: Option<(f64, f64)>,
}

/// Paired difference of two matches played on the same deals.
pub fn paired_difference(
    opponent_id: &str,
    fixed: &[PairResult],
    adaptive: &[PairResult],
    big_blind: u32,
    bootstrap: Option<(u32, u64)>,
) -> Result<Comparison, EvalError> {
    if fixed.len() != adaptive.len()
        || fixed
            .iter()
            .zip(adaptive)
            .any(|(f, a)| f.deal_seed != a.deal_seed)
    {
        return Err(EvalError::Config(
            "comparison needs both matches on the same deal seeds".into(),
        ));
    }
    let d: Vec<f64> = fixed
        .iter()
        .zip(adaptive)
        .map(|(f, a)| (f.a_total() - a.a_total()) as f64)
        .collect();
    let scale = 100.0 / f64::from(big_blind) / 2.0;
    Ok(Comparison {
        opponent_id: opponent_id.to_string(),
        pairs: d.len() as u64,
        fixed_minus_adaptive_bb100: summarize(&d, Z95).scaled(scale),
        bootstrap: bootstrap
            .and_then(|(b, seed)| bootstrap_ci(&d, b, seed).map(|(l, h)| (l * scale, h * scale))),
    })
}

/// Builds a seat occupant of `kind` sitting in `seat`.
fn participant(
    kind: &str,
    seat: Seat,
    art: &LoadedArtifact,
    seed: u64,
    adapt: Option<&AdaptConfig>,
) -> Result<Box<dyn Participant>, EvalError> {
    match kind {
        "cfr" => Ok(Box::new(Owned(Box::new(art.agent(seed)?)))),
        "adaptive_cfr" => {
            let cfg = *adapt.ok_or_else(|| {
                EvalError::Config("adaptive_cfr needs an [adaptive] section".into())
            })?;
            // The agent models whoever sits in the other seat.
            let a = AdaptiveAgent::new(Box::new(art.agent(seed)?), seat.other(), cfg)
                .map_err(|e| EvalError::Config(format!("[adaptive] {e}")))?;
            Ok(Box::new(a))
        }
        other => Ok(Box::new(Owned(opponent(other, art, seed)?))),
    }
}

/// An owned plain agent.
struct Owned(Box<dyn Agent>);

impl Participant for Owned {
    fn agent(&mut self) -> &mut dyn Agent {
        self.0.as_mut()
    }
    fn label(&self) -> String {
        self.0.label()
    }
    fn infer_stats(&self) -> Option<InferStats> {
        self.0.infer_stats()
    }
}

fn opponent(id: &str, art: &LoadedArtifact, seed: u64) -> Result<Box<dyn Agent>, EvalError> {
    Ok(match id {
        "random" => Box::new(RandomLegal::new(seed ^ 0x5241_4E44)),
        "checkcall" => Box::new(CheckCall),
        "tight" => Box::new(TightRule),
        "mirror" | "cfr" => Box::new(art.agent(seed ^ 0x4D49_5252)?),
        other => return Err(EvalError::Config(format!("unknown opponent {other:?}"))),
    })
}

fn add_infer(acc: &mut InferStats, s: Option<InferStats>) {
    if let Some(s) = s {
        acc.decisions += s.decisions;
        acc.policy_hits += s.policy_hits;
        acc.fallbacks += s.fallbacks;
    }
}

/// Runs the configured evaluation. `pairs_override` replaces
/// `cfg.deal_pairs` (capped at [`MAX_CLI_PAIRS`]).
pub fn run_evaluation(
    cfg: &EvalConfig,
    art: &LoadedArtifact,
    pairs_override: Option<u64>,
) -> Result<EvalReport, EvalError> {
    if let Some(p) = pairs_override
        && (p == 0 || p > MAX_CLI_PAIRS)
    {
        return Err(EvalError::Config(format!(
            "--pairs must be in 1..={MAX_CLI_PAIRS}, got {p}"
        )));
    }
    let pairs = pairs_override.unwrap_or(cfg.deal_pairs);
    if pairs == 0 || pairs > MAX_PAIRS {
        return Err(EvalError::Config(format!(
            "deal_pairs must be in 1..={MAX_PAIRS}, got {pairs}"
        )));
    }
    if art.info.rules != cfg.rules {
        return Err(EvalError::ArtifactInvalid {
            path: art.info.path.clone(),
            error: format!(
                "trained for {:?}, config evaluates {:?}",
                art.info.rules, cfg.rules
            ),
        });
    }
    // Train/test separation: refuse an evaluation stream seeded like training.
    if cfg.base_seed == art.info.training_seed || cfg.base_seed == art.info.abstraction_seed {
        return Err(EvalError::Config(format!(
            "eval base_seed {} equals the artifact's training or abstraction seed; \
             choose a disjoint evaluation seed",
            cfg.base_seed
        )));
    }
    let spec = MatchSpec {
        rules: cfg.rules,
        base_seed: cfg.base_seed,
        pairs,
        max_actions: cfg.max_actions_per_hand,
    };
    let boot =
        (cfg.bootstrap_resamples > 0).then_some((cfg.bootstrap_resamples, cfg.bootstrap_seed));

    let mut matchups = Vec::new();
    let mut raw: Vec<(String, String, MatchResult)> = Vec::new();
    let mut counts = DecisionCounts::default();
    let mut infer = InferStats::default();
    let mut candidate_label = String::new();
    for cand_id in &cfg.candidates {
        for id in &cfg.opponents {
            if cand_id == "cfr" && id == "cfr" {
                continue; // identical to the mirror
            }
            let opp_kind = if id == "mirror" { cand_id.as_str() } else { id };
            let opp_seed = if id == "mirror" || id == "cfr" {
                cfg.agent_seed ^ 0x4D49_5252
            } else {
                cfg.agent_seed
            };
            let mut cand = participant(
                cand_id,
                Seat::Zero,
                art,
                cfg.agent_seed,
                cfg.adaptive.as_ref(),
            )?;
            let mut opp = participant(opp_kind, Seat::One, art, opp_seed, cfg.adaptive.as_ref())?;
            let m = run_match_with(&spec, cand.as_mut(), opp.as_mut())?;
            if cand_id == "cfr" {
                candidate_label = m.label_a.clone();
                counts.decisions += m.counts_a.decisions;
                counts.fallbacks += m.counts_a.fallbacks;
                add_infer(&mut infer, m.infer_a);
                if id == "mirror" {
                    counts.decisions += m.counts_b.decisions;
                    counts.fallbacks += m.counts_b.fallbacks;
                    add_infer(&mut infer, m.infer_b);
                }
            } else if candidate_label.is_empty() {
                candidate_label = m.label_a.clone();
            }
            let mut r = summarize_match(id, &m, boot);
            r.candidate_id = cand_id.clone();
            matchups.push(r);
            raw.push((cand_id.clone(), id.clone(), m));
        }
    }

    let mut comparisons = Vec::new();
    for (_, opp, fixed) in raw.iter().filter(|(c, o, _)| c == "cfr" && o != "mirror") {
        if let Some((_, _, adaptive)) = raw
            .iter()
            .find(|(c2, o2, _)| c2 == "adaptive_cfr" && o2 == opp)
        {
            comparisons.push(paired_difference(
                opp,
                &fixed.pairs,
                &adaptive.pairs,
                cfg.rules.big_blind,
                boot,
            )?);
        }
    }

    let mut sanity = Vec::new();
    if cfg.sanity_random_vs_checkcall {
        let mut r = RandomLegal::new(cfg.agent_seed ^ 0x5241_4E44);
        let mut c = CheckCall;
        let m = run_match(&spec, &mut r, &mut c)?;
        sanity.push(summarize_match("checkcall", &m, boot));
    }

    let checksum = match cfg.expected_checksum {
        Some(e) if e == art.info.checksum => ChecksumStatus::Matches(e),
        Some(e) => ChecksumStatus::Mismatch {
            expected: e,
            found: art.info.checksum,
        },
        None => ChecksumStatus::NotPinned(art.info.checksum),
    };
    let gate = match (&cfg.mode, &cfg.gate) {
        (Mode::Promotion, Some(g)) => {
            let fixed: Vec<MatchupReport> = matchups
                .iter()
                .filter(|m| m.candidate_id == "cfr")
                .cloned()
                .collect();
            Some(decide(
                g,
                &GateInput {
                    matchups: &fixed,
                    candidate: counts,
                    checksum,
                },
            ))
        }
        _ => None,
    };
    Ok(EvalReport {
        depth_bb: cfg.depth_bb,
        rules: cfg.rules,
        base_seed: cfg.base_seed,
        pairs,
        artifact: art.info.clone(),
        checksum,
        mode: cfg.mode,
        candidate_label,
        matchups,
        sanity,
        candidate_counts: counts,
        candidate_infer: infer,
        gate,
        comparisons,
    })
}

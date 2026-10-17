//! End-to-end evaluation: load and check the artifact, run every configured
//! matchup, summarize, and apply the promotion gate.

use std::fmt;
use std::path::Path;

use mako_core::rules::RulesConfig;

use crate::agent::{Agent, CfrAgent, InferStats};
use crate::baselines::{CheckCall, RandomLegal, TightRule};
use crate::config::EvalConfig;
use crate::gate::{ChecksumStatus, GateDecision, GateInput, decide};
use crate::matchup::{DecisionCounts, MAX_PAIRS, MatchError, MatchSpec, run_match};
use crate::report::{MatchupReport, summarize_match};

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
    pub candidate_label: String,
    /// Candidate (agent A) versus each configured opponent.
    pub matchups: Vec<MatchupReport>,
    /// Baseline-vs-baseline sanity matchups.
    pub sanity: Vec<MatchupReport>,
    /// Harness-counted candidate decisions over all matchups (both sides of
    /// the mirror).
    pub candidate_counts: DecisionCounts,
    /// Sum of the candidate agents' own `InferStats`.
    pub candidate_infer: InferStats,
    pub gate: GateDecision,
}

fn opponent(id: &str, art: &LoadedArtifact, seed: u64) -> Result<Box<dyn Agent>, EvalError> {
    Ok(match id {
        "random" => Box::new(RandomLegal::new(seed ^ 0x5241_4E44)),
        "checkcall" => Box::new(CheckCall),
        "tight" => Box::new(TightRule),
        "mirror" => Box::new(art.agent(seed ^ 0x4D49_5252)?),
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
    let mut counts = DecisionCounts::default();
    let mut infer = InferStats::default();
    let mut candidate_label = String::new();
    for id in &cfg.opponents {
        let mut cand = art.agent(cfg.agent_seed)?;
        let mut opp = opponent(id, art, cfg.agent_seed)?;
        let m = run_match(&spec, &mut cand, opp.as_mut())?;
        candidate_label = m.label_a.clone();
        counts.decisions += m.counts_a.decisions;
        counts.fallbacks += m.counts_a.fallbacks;
        add_infer(&mut infer, m.infer_a);
        if id == "mirror" {
            counts.decisions += m.counts_b.decisions;
            counts.fallbacks += m.counts_b.fallbacks;
            add_infer(&mut infer, m.infer_b);
        }
        matchups.push(summarize_match(id, &m, boot));
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
    let gate = decide(
        &cfg.gate,
        &GateInput {
            matchups: &matchups,
            candidate: counts,
            checksum,
        },
    );
    Ok(EvalReport {
        depth_bb: cfg.depth_bb,
        rules: cfg.rules,
        base_seed: cfg.base_seed,
        pairs,
        artifact: art.info.clone(),
        checksum,
        candidate_label,
        matchups,
        sanity,
        candidate_counts: counts,
        candidate_infer: infer,
        gate,
    })
}

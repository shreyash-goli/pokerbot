//! Choosing and describing the opponent agent.
//!
//! The opponent is any [`mako_cfr::infer::Agent`]: it is handed only its own
//! `PlayerObservation`, never the `HandState`. This module decides which
//! agent plays (`--bot auto|cfr|baseline`) and builds the banner that tells
//! the player, without ambiguity, whether a trained policy or the untrained
//! baseline is at the table, and why.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use mako_cfr::artifact::ArtifactError;
use mako_cfr::infer::{Agent, AgentError, BaselineAgent, CfrAgent, short_count};
use mako_core::rules::{RulesConfig, StackDepth};

use crate::args::BotKind;
use crate::ui::screens::{Banner, BannerLevel};

pub use mako_cfr::infer::BASELINE_LABEL;

/// The script that trains the smoke policy.
pub const TRAIN_HINT: &str = "Train with scripts/train_smoke.sh";

/// Default artifact path for a depth: `artifacts/smoke_<bb>bb.mako`.
pub fn default_strategy_path(depth: StackDepth) -> PathBuf {
    PathBuf::from(format!("artifacts/smoke_{}bb.mako", depth.big_blinds()))
}

/// Why a policy artifact could not be used.
#[derive(Debug)]
pub enum LoadFailure {
    /// No file at the path.
    Missing(io::Error),
    /// The file exists but could not be read.
    Unreadable(io::Error),
    /// The artifact was trained for other rules (depth or blinds).
    WrongDepth(AgentError),
    /// Corrupt, truncated, incompatible or otherwise invalid.
    Invalid(AgentError),
}

impl fmt::Display for LoadFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadFailure::Missing(e) => write!(f, "missing: {e}"),
            LoadFailure::Unreadable(e) => write!(f, "unreadable: {e}"),
            LoadFailure::WrongDepth(e) => write!(f, "wrong depth: {e}"),
            LoadFailure::Invalid(e) => write!(f, "corrupt or invalid: {e}"),
        }
    }
}

/// Reads and validates a policy artifact for `rules`.
pub fn load_cfr(path: &Path, rules: &RulesConfig, seed: u64) -> Result<CfrAgent, LoadFailure> {
    let bytes = std::fs::read(path).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => LoadFailure::Missing(e),
        _ => LoadFailure::Unreadable(e),
    })?;
    CfrAgent::from_artifact_bytes(&bytes, rules, seed).map_err(|e| match e {
        AgentError::Artifact(ArtifactError::RulesMismatch { .. }) => LoadFailure::WrongDepth(e),
        _ => LoadFailure::Invalid(e),
    })
}

/// What kind of agent sits in the opponent seat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpponentKind {
    /// A trained CFR policy.
    Cfr,
    /// The baseline, requested explicitly.
    Baseline,
    /// The baseline because `--bot auto` found no usable policy.
    BaselineFallback,
}

/// The opponent agent and the banner announcing it.
pub struct Opponent {
    pub agent: Box<dyn Agent>,
    pub kind: OpponentKind,
    /// Announcement printed at the start of the session.
    pub banner: Banner,
    /// The artifact path that was tried (for the replay hint).
    pub strategy_path: PathBuf,
    /// Checksum of the loaded policy artifact (`None` for the baseline).
    pub checksum: Option<u64>,
    /// A separate copy of the same policy for the post-hand coach (M-18),
    /// queried with `strategy()` only; `None` for the baseline.
    pub coach: Option<CfrAgent>,
}

impl Opponent {
    /// The label stored with each saved hand (M-17): the agent's label and,
    /// for a trained policy, the artifact checksum.
    pub fn history_label(&self) -> mako_analysis::OpponentLabel {
        mako_analysis::OpponentLabel::new(&self.agent.label(), self.checksum)
    }
}

/// Selection failed (only for `--bot cfr`); the message names the reason.
#[derive(Debug)]
pub struct SelectError(pub String);

impl fmt::Display for SelectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn cfr_banner(agent: &CfrAgent, path: &Path) -> Banner {
    let m = agent.metadata();
    Banner {
        level: BannerLevel::Info,
        lines: vec![
            format!("Opponent: CFR policy -- {}", agent.label()),
            format!(
                "artifact {} | checksum {:#018x} | {} infosets | abstraction {:#018x}",
                path.display(),
                m.checksum,
                m.infosets,
                m.fingerprint
            ),
            format!(
                "trained {} iterations ({}) | trainer seed {} | {} | {}BB, blinds {}/{}",
                m.iterations,
                short_count(m.iterations),
                m.seed,
                m.trainer_id,
                m.depth_bb(),
                m.rules.small_blind,
                m.rules.big_blind
            ),
            "NOTE: plumbing-only smoke policy -- not competitive, not GTO / Nash. Unseen \
             situations use an explicit uniform-random legal fallback ('stats' counts them)."
                .to_string(),
        ],
    }
}

fn baseline_banner() -> Banner {
    Banner {
        level: BannerLevel::Info,
        lines: vec![format!(
            "Opponent: {BASELINE_LABEL}. This bot is NOT a trained CFR policy."
        )],
    }
}

fn loud_fallback_banner(depth: StackDepth, path: &Path, why: &LoadFailure) -> Banner {
    Banner {
        level: BannerLevel::Warning,
        lines: vec![
            format!(
                "No trained policy for {}BB at {} ({why}).",
                depth.big_blinds(),
                path.display()
            ),
            format!("Playing the UNTRAINED baseline: {BASELINE_LABEL}."),
            format!("{TRAIN_HINT}, or pass --strategy <path>."),
        ],
    }
}

/// Loads a second copy of the policy for the coach; dropped if the file
/// changed in between (checksum differs).
fn coach_copy(path: &Path, rules: &RulesConfig, agent: &CfrAgent) -> Option<CfrAgent> {
    load_cfr(path, rules, 0)
        .ok()
        .filter(|c| c.metadata().checksum == agent.metadata().checksum)
}

/// Picks the opponent. `seed` seeds the agent's own RNG (derive it from the
/// session seed). `--bot cfr` never falls back: it returns an error.
pub fn select(
    kind: BotKind,
    strategy: Option<&Path>,
    depth: StackDepth,
    seed: u64,
) -> Result<Opponent, SelectError> {
    let rules = RulesConfig::standard(depth);
    let default_path = default_strategy_path(depth);
    let path = strategy.unwrap_or(&default_path);
    match kind {
        BotKind::Baseline => Ok(Opponent {
            agent: Box::new(BaselineAgent::new(seed)),
            kind: OpponentKind::Baseline,
            banner: baseline_banner(),
            strategy_path: path.to_path_buf(),
            checksum: None,
            coach: None,
        }),
        BotKind::Cfr => match load_cfr(path, &rules, seed) {
            Ok(agent) => Ok(Opponent {
                checksum: Some(agent.metadata().checksum),
                coach: coach_copy(path, &rules, &agent),
                banner: cfr_banner(&agent, path),
                agent: Box::new(agent),
                kind: OpponentKind::Cfr,
                strategy_path: path.to_path_buf(),
            }),
            Err(why) => Err(SelectError(format!(
                "--bot cfr: no trained policy for {}BB at {} ({why}). \
                 {TRAIN_HINT}, or use --bot baseline.",
                depth.big_blinds(),
                path.display()
            ))),
        },
        BotKind::Auto => match load_cfr(path, &rules, seed) {
            Ok(agent) => Ok(Opponent {
                checksum: Some(agent.metadata().checksum),
                coach: coach_copy(path, &rules, &agent),
                banner: cfr_banner(&agent, path),
                agent: Box::new(agent),
                kind: OpponentKind::Cfr,
                strategy_path: path.to_path_buf(),
            }),
            Err(why) => Ok(Opponent {
                agent: Box::new(BaselineAgent::new(seed)),
                kind: OpponentKind::BaselineFallback,
                banner: loud_fallback_banner(depth, path, &why),
                strategy_path: path.to_path_buf(),
                checksum: None,
                coach: None,
            }),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_paths() {
        assert_eq!(
            default_strategy_path(StackDepth::Bb20),
            PathBuf::from("artifacts/smoke_20bb.mako")
        );
        assert_eq!(
            default_strategy_path(StackDepth::Bb100),
            PathBuf::from("artifacts/smoke_100bb.mako")
        );
    }

    #[test]
    fn missing_artifact_cfr_errors_auto_falls_back_loudly() {
        let p = Path::new("/nonexistent/dir/policy.mako");
        let e = select(BotKind::Cfr, Some(p), StackDepth::Bb50, 1)
            .err()
            .expect("cfr must not fall back");
        assert!(e.0.contains("no trained policy for 50BB"), "{e}");
        assert!(e.0.contains("missing"), "{e}");

        let o = select(BotKind::Auto, Some(p), StackDepth::Bb50, 1).unwrap();
        assert_eq!(o.kind, OpponentKind::BaselineFallback);
        assert_eq!(o.agent.label(), BASELINE_LABEL);
        assert_eq!(o.banner.level, BannerLevel::Warning);
        let b = o.banner.lines.join("\n");
        assert!(b.contains("No trained policy for 50BB"), "{b}");
        assert!(b.contains("UNTRAINED baseline"), "{b}");
        assert!(b.contains("scripts/train_smoke.sh"), "{b}");
    }

    #[test]
    fn baseline_explicit() {
        let o = select(BotKind::Baseline, None, StackDepth::Bb20, 1).unwrap();
        assert_eq!(o.kind, OpponentKind::Baseline);
        assert!(o.banner.lines[0].contains("NOT a trained CFR policy"));
    }
}

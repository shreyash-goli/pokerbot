//! Promotion gate: PROMOTE or HOLD from locked criteria.
//!
//! The gate only answers "does this artifact clear the pre-registered bar
//! against the configured weak baselines?". It never makes, and must never
//! be read as making, any claim about Nash equilibrium, GTO play or
//! exploitability: none of those is measured here.

use crate::matchup::DecisionCounts;
use crate::report::MatchupReport;

/// Printed with every report.
pub const DISCLAIMER: &str = "DISCLAIMER: results are BB/100 against a few fixed baseline bots under \
the six-action abstraction. Confidence intervals are statistical statements, not guarantees. \
Beating weak baselines does not imply strength against capable opponents. No exploitability, \
best-response, Nash-distance or GTO measurement was performed, and none is claimed.";

/// Locked gate thresholds (from config, before running).
#[derive(Clone, Debug, PartialEq)]
pub struct GateConfig {
    /// Minimum hands in each required matchup.
    pub min_hands: u64,
    /// Opponent ids whose BB/100 CI lower bound must be > 0.
    pub require_positive_ci_vs: Vec<String>,
    /// Maximum candidate fallback (unseen info set) rate, as a fraction.
    pub max_fallback_rate: f64,
    /// Whether an expected artifact checksum must be pinned and match.
    pub require_pinned_checksum: bool,
}

/// Artifact checksum versus the pinned value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChecksumStatus {
    Matches(u64),
    Mismatch { expected: u64, found: u64 },
    NotPinned(u64),
}

/// Gate inputs.
#[derive(Clone, Debug)]
pub struct GateInput<'a> {
    /// Matchups where the candidate is agent A.
    pub matchups: &'a [MatchupReport],
    /// Candidate decision counts over all its matchups.
    pub candidate: DecisionCounts,
    pub checksum: ChecksumStatus,
}

/// Verdict.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Promote,
    Hold,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Promote => "PROMOTE",
            Verdict::Hold => "HOLD",
        }
    }
}

/// One gate criterion.
#[derive(Clone, Debug, PartialEq)]
pub struct GateCheck {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

/// Verdict with every check and its reason.
#[derive(Clone, Debug, PartialEq)]
pub struct GateDecision {
    pub verdict: Verdict,
    pub checks: Vec<GateCheck>,
}

impl GateDecision {
    /// Reasons for a HOLD (failed checks).
    pub fn failures(&self) -> Vec<&GateCheck> {
        self.checks.iter().filter(|c| !c.passed).collect()
    }
}

/// Apply the gate. PROMOTE only if every check passes.
pub fn decide(cfg: &GateConfig, input: &GateInput<'_>) -> GateDecision {
    let mut checks = Vec::new();
    if cfg.require_positive_ci_vs.is_empty() {
        checks.push(GateCheck {
            name: "baselines".into(),
            passed: false,
            detail: "no required baselines configured; refusing to promote on no evidence".into(),
        });
    }
    for opp in &cfg.require_positive_ci_vs {
        let Some(m) = input.matchups.iter().find(|m| &m.opponent_id == opp) else {
            checks.push(GateCheck {
                name: format!("vs {opp}"),
                passed: false,
                detail: "required matchup was not run".into(),
            });
            continue;
        };
        checks.push(GateCheck {
            name: format!("hands vs {opp}"),
            passed: m.hands >= cfg.min_hands,
            detail: format!("{} hands (minimum {})", m.hands, cfg.min_hands),
        });
        checks.push(GateCheck {
            name: format!("CI vs {opp}"),
            passed: m.bb100.significantly_positive(),
            detail: format!(
                "BB/100 {:.2}, 95% CI [{:.2}, {:.2}] (lower bound must be > 0)",
                m.bb100.mean, m.bb100.ci_low, m.bb100.ci_high
            ),
        });
    }
    let rate = input.candidate.fallback_rate();
    checks.push(GateCheck {
        name: "fallback rate".into(),
        passed: input.candidate.decisions > 0 && rate <= cfg.max_fallback_rate,
        detail: format!(
            "{:.2}% of {} decisions (maximum {:.2}%)",
            rate * 100.0,
            input.candidate.decisions,
            cfg.max_fallback_rate * 100.0
        ),
    });
    let (ok, detail) = match input.checksum {
        ChecksumStatus::Matches(c) => (true, format!("{c:#018x} matches pinned value")),
        ChecksumStatus::Mismatch { expected, found } => (
            false,
            format!("{found:#018x} does not match pinned {expected:#018x}"),
        ),
        ChecksumStatus::NotPinned(c) => (
            !cfg.require_pinned_checksum,
            format!("{c:#018x} (no pinned checksum in config)"),
        ),
    };
    checks.push(GateCheck {
        name: "artifact checksum".into(),
        passed: ok,
        detail,
    });
    let verdict = if checks.iter().all(|c| c.passed) {
        Verdict::Promote
    } else {
        Verdict::Hold
    };
    GateDecision { verdict, checks }
}

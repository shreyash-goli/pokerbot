//! Policy inference: agents that pick actions from a [`PlayerObservation`]
//! (M-14).
//!
//! An [`Agent`] sees only what its seat may legally know: its own hole
//! cards, the revealed board and the public history. It never receives a
//! `HandState`, so it cannot read the opponent's cards or the undealt board.
//!
//! * [`CfrAgent`] plays the average strategy stored in a `.mako` artifact
//!   (see `docs/STRATEGY_FORMAT.md`). Its card abstraction is rebuilt from
//!   the artifact's own stored `AbstractionConfig`, then the artifact is
//!   loaded through [`Policy::load`], which rejects other depths/blinds.
//!   Information sets absent from the artifact are played uniformly at
//!   random over the legal actions, and every such decision is reported as
//!   [`DecisionSource::Fallback`] and counted in [`InferStats`].
//! * [`BaselineAgent`] is the untrained weighted-random legal bot.
//!
//! Sampling is seeded ([`MakoRng`]) and uses integer draws over the
//! artifact's quantized probabilities, so a seed reproduces the same choices
//! on every platform, wasm32 included.
//!
//! A trained policy approximates an equilibrium of the *abstract* game only;
//! it is not GTO. Labels say what an artifact was trained for.

use std::fmt;

use mako_core::action::ActionKind;
use mako_core::observation::PlayerObservation;
use mako_core::rng::MakoRng;
use mako_core::rules::RulesConfig;

use crate::abstraction::key::{InfoKey, KeyError};
use crate::abstraction::postflop::{AbstractionConfig, EquityAbstraction};
use crate::artifact::{ArtifactError, PROB_SCALE, Policy, stored_abstraction};
use crate::regret::{LegalMask, NUM_SLOTS, is_legal, mask_of};

/// Label of the untrained baseline agent.
pub const BASELINE_LABEL: &str = "baseline (untrained, random legal)";

/// Equity-cache capacity of a [`CfrAgent`]'s abstraction (results do not
/// depend on it).
pub const AGENT_CACHE_CAPACITY: usize = 1 << 16;

/// Why a decision did not come from the trained policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FallbackReason {
    /// The information set is not stored in the artifact (`lookup` gave
    /// `None`). Played uniformly over the legal actions.
    UnseenInfoset,
    /// The stored entry has no probability mass on the legal actions.
    /// Cannot happen for an artifact that passed validation; handled
    /// explicitly anyway. Played uniformly over the legal actions.
    ZeroMass,
}

impl fmt::Display for FallbackReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FallbackReason::UnseenInfoset => write!(f, "unseen infoset"),
            FallbackReason::ZeroMass => write!(f, "zero-mass infoset"),
        }
    }
}

/// Where a decision's distribution came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DecisionSource {
    /// The trained average strategy.
    Policy,
    /// Explicit uniform-legal fallback of a policy agent.
    Fallback(FallbackReason),
    /// The untrained baseline agent (never a trained policy).
    Baseline,
}

/// One chosen action and the distribution it was drawn from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Decision {
    /// The sampled action class (always legal for the observation).
    pub action: ActionKind,
    /// Probability per `ActionKind` slot (index = `ActionKind::index`);
    /// 0 on illegal slots, sums to 1.
    pub distribution: [f64; NUM_SLOTS],
    /// Origin of `distribution`.
    pub source: DecisionSource,
}

/// Decision counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct InferStats {
    /// Decisions made via [`Agent::act`].
    pub decisions: u64,
    /// Decisions drawn from the trained policy.
    pub policy_hits: u64,
    /// Decisions that used an explicit fallback.
    pub fallbacks: u64,
}

/// Inference errors. No error is ever papered over with a silent default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentError {
    /// The artifact could not be loaded (corrupt, wrong depth, ...).
    Artifact(ArtifactError),
    /// The observation is terminal.
    Terminal,
    /// The observing seat is not the one to act.
    NotToAct,
    /// The observation lists no legal actions.
    NoLegalActions,
    /// Building the information key failed.
    Key(KeyError),
    /// The stored legal mask differs from the runtime legal set.
    MaskMismatch {
        /// Mask stored in the artifact.
        stored: LegalMask,
        /// Mask of `obs.legal_actions`.
        runtime: LegalMask,
    },
}

impl fmt::Display for AgentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentError::Artifact(e) => write!(f, "{e}"),
            AgentError::Terminal => write!(f, "the hand is over; there is no decision"),
            AgentError::NotToAct => write!(f, "the observing seat is not to act"),
            AgentError::NoLegalActions => write!(f, "no legal actions in the observation"),
            AgentError::Key(e) => write!(f, "information key: {e}"),
            AgentError::MaskMismatch { stored, runtime } => write!(
                f,
                "artifact legal mask {stored:#08b} differs from the runtime legal set {runtime:#08b}"
            ),
        }
    }
}

impl std::error::Error for AgentError {}

impl From<ArtifactError> for AgentError {
    fn from(e: ArtifactError) -> Self {
        AgentError::Artifact(e)
    }
}

impl From<KeyError> for AgentError {
    fn from(e: KeyError) -> Self {
        AgentError::Key(e)
    }
}

/// A player policy driven only by its own observation.
pub trait Agent {
    /// Honest description shown to users, e.g.
    /// `"baseline (untrained, random legal)"`.
    fn label(&self) -> String;

    /// Chooses an action. `obs.to_act` must be `obs.seat`.
    fn act(&mut self, obs: &PlayerObservation) -> Result<Decision, AgentError>;

    /// Policy-hit / fallback counters, for agents that track them.
    fn infer_stats(&self) -> Option<InferStats> {
        None
    }
}

/// Checks the observation is a live decision and returns its legal mask.
fn decision_mask(obs: &PlayerObservation) -> Result<LegalMask, AgentError> {
    if obs.terminal.is_some() {
        return Err(AgentError::Terminal);
    }
    if !obs.is_my_turn() {
        return Err(AgentError::NotToAct);
    }
    let mask = mask_of(obs.legal_actions.iter().map(|a| a.kind));
    if mask == 0 {
        return Err(AgentError::NoLegalActions);
    }
    Ok(mask)
}

/// Integer weights: draws `0..Σw` and returns the slot it lands in.
fn sample_weighted(rng: &mut MakoRng, weights: &[u64; NUM_SLOTS]) -> usize {
    let total: u64 = weights.iter().sum();
    debug_assert!(total > 0);
    let mut pick = rng.gen_range(total);
    for (i, &w) in weights.iter().enumerate() {
        if pick < w {
            return i;
        }
        pick -= w;
    }
    unreachable!("pick < total by construction")
}

fn distribution_of(weights: &[u64; NUM_SLOTS]) -> [f64; NUM_SLOTS] {
    let total: u64 = weights.iter().sum();
    weights.map(|w| w as f64 / total as f64)
}

fn uniform_weights(mask: LegalMask) -> [u64; NUM_SLOTS] {
    std::array::from_fn(|i| u64::from(is_legal(mask, i)))
}

/// Note shown with every difficulty label.
pub const DIFFICULTY_NOTE: &str = "difficulty modes are blends of a plumbing-grade smoke policy; \
     relative strength has not been benchmarked unless shown by mako-eval";

/// How a [`CfrAgent`] transforms the trained distribution before sampling.
///
/// Every mode is an explicit blend with the uniform distribution over the
/// **legal** actions `L` (`n = |L|`), with share `λ`:
///
/// ```text
/// p'(a) = (1 − λ) · p(a) + λ / n     for a in L,   p'(a) = 0 otherwise
/// ```
///
/// | mode | λ | meaning |
/// |---|---|---|
/// | `Challenging` (default) | 0 | the trained average strategy unchanged |
/// | `Balanced` | 1/4 | 75% policy, 25% uniform-random legal |
/// | `Training` | 3/5 | 40% policy, 60% uniform-random legal |
///
/// `p` is the artifact's quantized mix `q / 65535`. Sampling stays integer:
/// with `λ = a/b` the slot weights are `(b − a)·q(i)·n + a·65535` (legal
/// slots), which are exactly proportional to `p'`. `Challenging` keeps the
/// original weights `q`, so it is bit-identical to an untransformed agent.
/// Explicit fallbacks (unseen infosets) are uniform already and unchanged.
/// No mode adds rules or uses any information beyond the observation.
/// Relative strength of the modes is not claimed; see [`DIFFICULTY_NOTE`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Difficulty {
    /// 40% policy, 60% uniform-random legal play.
    Training,
    /// 75% policy, 25% uniform-random legal play.
    Balanced,
    /// The trained policy as-is.
    #[default]
    Challenging,
}

impl Difficulty {
    /// All modes, from most to least uniform.
    pub const ALL: [Difficulty; 3] = [
        Difficulty::Training,
        Difficulty::Balanced,
        Difficulty::Challenging,
    ];

    /// Name used on the command line.
    pub const fn name(self) -> &'static str {
        match self {
            Difficulty::Training => "training",
            Difficulty::Balanced => "balanced",
            Difficulty::Challenging => "challenging",
        }
    }

    /// Parses a [`Difficulty::name`].
    pub fn parse(s: &str) -> Option<Difficulty> {
        Difficulty::ALL
            .into_iter()
            .find(|d| d.name().eq_ignore_ascii_case(s))
    }

    /// The uniform share `λ = a / b` as `(a, b)`.
    pub const fn uniform_share(self) -> (u64, u64) {
        match self {
            Difficulty::Training => (3, 5),
            Difficulty::Balanced => (1, 4),
            Difficulty::Challenging => (0, 1),
        }
    }

    /// Honest one-line description.
    pub const fn description(self) -> &'static str {
        match self {
            Difficulty::Training => "training: policy blended 60% with uniform-random legal play",
            Difficulty::Balanced => "balanced: policy blended 25% with uniform-random legal play",
            Difficulty::Challenging => "challenging: trained smoke policy as-is",
        }
    }

    /// Integer sampling weights for policy weights `q` (legal slots of
    /// `mask`), per the formula above.
    pub fn transform_weights(self, q: &[u64; NUM_SLOTS], mask: LegalMask) -> [u64; NUM_SLOTS] {
        let (a, b) = self.uniform_share();
        if a == 0 {
            return *q;
        }
        let n = u64::from((mask & crate::regret::FULL_MASK).count_ones());
        std::array::from_fn(|i| {
            if is_legal(mask, i) {
                (b - a) * q[i] * n + a * u64::from(PROB_SCALE)
            } else {
                0
            }
        })
    }

    /// The same transform on a probability vector (for callers holding
    /// floats, e.g. evaluation): `(1 − λ)·p + λ/n` on legal slots.
    pub fn transform(self, p: &[f64; NUM_SLOTS], mask: LegalMask) -> [f64; NUM_SLOTS] {
        let (a, b) = self.uniform_share();
        let lambda = a as f64 / b as f64;
        let n = f64::from((mask & crate::regret::FULL_MASK).count_ones());
        std::array::from_fn(|i| {
            if is_legal(mask, i) {
                (1.0 - lambda) * p[i] + lambda / n
            } else {
                0.0
            }
        })
    }
}

/// Provenance of a loaded artifact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentMetadata {
    /// Trainer seed.
    pub seed: u64,
    /// Completed training iterations.
    pub iterations: u64,
    /// Artifact label (what the run was for).
    pub label: String,
    /// Trainer id string.
    pub trainer_id: String,
    /// Abstraction fingerprint.
    pub fingerprint: u64,
    /// Artifact checksum (FNV-1a 64, the file's last 8 bytes).
    pub checksum: u64,
    /// Rules the policy was trained for.
    pub rules: RulesConfig,
    /// The stored card abstraction.
    pub abstraction: AbstractionConfig,
    /// Number of stored information sets.
    pub infosets: usize,
}

impl AgentMetadata {
    /// Depth in big blinds (starting stack / big blind, rounded down).
    pub fn depth_bb(&self) -> u32 {
        self.rules.starting_stack / self.rules.big_blind
    }
}

/// Plays the average strategy of a trained artifact.
#[derive(Debug)]
pub struct CfrAgent {
    policy: Policy,
    abstraction: EquityAbstraction,
    rng: MakoRng,
    stats: InferStats,
    metadata: AgentMetadata,
    difficulty: Difficulty,
}

/// Internal resolution of one decision.
struct Resolved {
    weights: [u64; NUM_SLOTS],
    source: DecisionSource,
}

impl CfrAgent {
    /// Loads an artifact for a game played with `rules`; `seed` seeds the
    /// action-sampling RNG. Fails (never substitutes) if the artifact is
    /// corrupt, for other rules/depth, or internally inconsistent.
    pub fn from_artifact_bytes(
        bytes: &[u8],
        rules: &RulesConfig,
        seed: u64,
    ) -> Result<Self, AgentError> {
        // Header-only read (magic, version, checksum, header fields), then
        // the full validating load against the caller's rules.
        let abstraction_config = stored_abstraction(bytes)?;
        let policy = Policy::load(bytes, rules, &abstraction_config)?;
        let abstraction =
            EquityAbstraction::with_cache(abstraction_config, AGENT_CACHE_CAPACITY)
                .map_err(|e| AgentError::Artifact(ArtifactError::Invalid(e.to_string())))?;
        let m = policy.metadata();
        let checksum = u64::from_le_bytes(
            bytes[bytes.len() - 8..]
                .try_into()
                .expect("validated artifact has a checksum"),
        );
        let metadata = AgentMetadata {
            seed: m.seed,
            iterations: m.iterations,
            label: m.label.clone(),
            trainer_id: m.trainer_id.clone(),
            fingerprint: m.abstraction.fingerprint(),
            checksum,
            rules: m.rules,
            abstraction: m.abstraction,
            infosets: policy.len(),
        };
        Ok(Self {
            policy,
            abstraction,
            rng: MakoRng::seed_from_u64(seed),
            stats: InferStats::default(),
            metadata,
            difficulty: Difficulty::Challenging,
        })
    }

    /// The same agent playing at `difficulty` (see [`Difficulty`]).
    pub fn with_difficulty(mut self, difficulty: Difficulty) -> Self {
        self.difficulty = difficulty;
        self
    }

    /// The difficulty this agent plays at.
    pub fn difficulty(&self) -> Difficulty {
        self.difficulty
    }

    /// Weights actually sampled from: the policy weights transformed by the
    /// difficulty; fallbacks unchanged.
    fn resolve(&self, obs: &PlayerObservation) -> Result<Resolved, AgentError> {
        let r = self.resolve_policy(obs)?;
        if r.source != DecisionSource::Policy || self.difficulty == Difficulty::Challenging {
            return Ok(r);
        }
        let mask = decision_mask(obs)?;
        Ok(Resolved {
            weights: self.difficulty.transform_weights(&r.weights, mask),
            source: r.source,
        })
    }

    /// Distribution of resolved weights: `q / 65535` for the untransformed
    /// policy, otherwise normalized weights.
    fn distribution(&self, r: &Resolved, transformed: bool) -> [f64; NUM_SLOTS] {
        if r.source == DecisionSource::Policy
            && (!transformed || self.difficulty == Difficulty::Challenging)
        {
            r.weights.map(|w| w as f64 / f64::from(PROB_SCALE))
        } else {
            distribution_of(&r.weights)
        }
    }

    fn resolve_policy(&self, obs: &PlayerObservation) -> Result<Resolved, AgentError> {
        let runtime = decision_mask(obs)?;
        let key = InfoKey::from_observation(obs, &self.abstraction)?;
        let Some(entry) = self.policy.lookup_bytes(&key.to_bytes()) else {
            return Ok(Resolved {
                weights: uniform_weights(runtime),
                source: DecisionSource::Fallback(FallbackReason::UnseenInfoset),
            });
        };
        if entry.mask != runtime {
            return Err(AgentError::MaskMismatch {
                stored: entry.mask,
                runtime,
            });
        }
        let weights: [u64; NUM_SLOTS] = std::array::from_fn(|i| {
            if is_legal(runtime, i) {
                u64::from(entry.q[i])
            } else {
                0
            }
        });
        if weights.iter().sum::<u64>() == 0 {
            return Ok(Resolved {
                weights: uniform_weights(runtime),
                source: DecisionSource::Fallback(FallbackReason::ZeroMass),
            });
        }
        Ok(Resolved {
            weights,
            source: DecisionSource::Policy,
        })
    }

    /// The distribution the agent would sample from at its difficulty,
    /// without sampling or counting. At `Challenging`, for a stored infoset
    /// it equals the artifact's probabilities (`q / 65535`).
    pub fn strategy(
        &self,
        obs: &PlayerObservation,
    ) -> Result<([f64; NUM_SLOTS], DecisionSource), AgentError> {
        let r = self.resolve(obs)?;
        Ok((self.distribution(&r, true), r.source))
    }

    /// The fixed trained mix (`q / 65535`, or the explicit fallback),
    /// ignoring the difficulty. For the coach.
    pub fn policy_strategy(
        &self,
        obs: &PlayerObservation,
    ) -> Result<([f64; NUM_SLOTS], DecisionSource), AgentError> {
        let r = self.resolve_policy(obs)?;
        Ok((self.distribution(&r, false), r.source))
    }

    /// Counters since construction.
    pub fn stats(&self) -> InferStats {
        self.stats
    }

    /// Provenance of the loaded artifact.
    pub fn metadata(&self) -> &AgentMetadata {
        &self.metadata
    }

    /// The loaded policy.
    pub fn policy(&self) -> &Policy {
        &self.policy
    }
}

impl Agent for CfrAgent {
    fn label(&self) -> String {
        let m = &self.metadata;
        let base = format!(
            "cfr {}BB ({} it): {}",
            m.depth_bb(),
            short_count(m.iterations),
            m.label
        );
        match self.difficulty {
            // The default keeps the original label: it is the policy as-is.
            Difficulty::Challenging => base,
            d => format!("{base} | difficulty {}", d.description()),
        }
    }

    fn act(&mut self, obs: &PlayerObservation) -> Result<Decision, AgentError> {
        let r = self.resolve(obs)?;
        let dist = self.distribution(&r, true);
        let slot = sample_weighted(&mut self.rng, &r.weights);
        self.stats.decisions += 1;
        match r.source {
            DecisionSource::Policy => self.stats.policy_hits += 1,
            _ => self.stats.fallbacks += 1,
        }
        Ok(Decision {
            action: ActionKind::ALL[slot],
            distribution: dist,
            source: r.source,
        })
    }

    fn infer_stats(&self) -> Option<InferStats> {
        Some(self.stats)
    }
}

/// `500000` → `"500k"`, `1500000` → `"1.5M"`, small numbers unchanged.
pub fn short_count(n: u64) -> String {
    if n >= 1_000_000 && n % 100_000 == 0 {
        let s = format!("{:.1}M", n as f64 / 1e6);
        s.replace(".0M", "M")
    } else if n >= 1_000 && n % 1_000 == 0 {
        format!("{}k", n / 1_000)
    } else {
        n.to_string()
    }
}

/// Untrained, seeded, weighted-random legal agent. Exists so the game is
/// playable without a trained policy; it is not strategic in any sense.
#[derive(Clone, Debug)]
pub struct BaselineAgent {
    rng: MakoRng,
    decisions: u64,
}

impl BaselineAgent {
    /// An agent whose decisions are fully determined by `seed`.
    pub fn new(seed: u64) -> Self {
        Self {
            rng: MakoRng::seed_from_u64(seed),
            decisions: 0,
        }
    }

    /// Integer selection weight per action class. Passive actions are
    /// favoured so hands do not end almost immediately.
    pub const fn weight(kind: ActionKind) -> u64 {
        match kind {
            ActionKind::Fold => 2,
            ActionKind::Check => 6,
            ActionKind::Call => 5,
            ActionKind::HalfPot => 3,
            ActionKind::Pot => 2,
            ActionKind::AllIn => 1,
        }
    }

    /// The distribution the baseline samples from.
    pub fn strategy(obs: &PlayerObservation) -> Result<[f64; NUM_SLOTS], AgentError> {
        let mask = decision_mask(obs)?;
        Ok(distribution_of(&Self::weights(mask)))
    }

    fn weights(mask: LegalMask) -> [u64; NUM_SLOTS] {
        std::array::from_fn(|i| {
            if is_legal(mask, i) {
                Self::weight(ActionKind::ALL[i])
            } else {
                0
            }
        })
    }
}

impl Agent for BaselineAgent {
    fn label(&self) -> String {
        BASELINE_LABEL.to_string()
    }

    fn act(&mut self, obs: &PlayerObservation) -> Result<Decision, AgentError> {
        let mask = decision_mask(obs)?;
        let weights = Self::weights(mask);
        let slot = sample_weighted(&mut self.rng, &weights);
        self.decisions += 1;
        Ok(Decision {
            action: ActionKind::ALL[slot],
            distribution: distribution_of(&weights),
            source: DecisionSource::Baseline,
        })
    }

    fn infer_stats(&self) -> Option<InferStats> {
        Some(InferStats {
            decisions: self.decisions,
            policy_hits: 0,
            fallbacks: 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn difficulty_integer_weights_match_the_float_formula() {
        // A legal mask of fold/call/pot/all-in and a skewed quantized mix.
        let mask: LegalMask = 0b11_0101;
        let q: [u64; NUM_SLOTS] = [60_000, 0, 5_000, 0, 535, 0];
        let p = q.map(|w| w as f64 / f64::from(PROB_SCALE));
        for d in Difficulty::ALL {
            let w = d.transform_weights(&q, mask);
            let total: u64 = w.iter().sum();
            let from_weights = w.map(|x| x as f64 / total as f64);
            let from_formula = d.transform(&p, mask);
            for i in 0..NUM_SLOTS {
                assert!(
                    (from_weights[i] - from_formula[i]).abs() < 1e-12,
                    "{d:?} slot {i}"
                );
                if !is_legal(mask, i) {
                    assert_eq!(w[i], 0);
                } else if d != Difficulty::Challenging {
                    assert!(w[i] > 0, "blends give every legal action mass");
                }
            }
        }
        assert_eq!(Difficulty::Challenging.transform_weights(&q, mask), q);
        assert_eq!(Difficulty::default(), Difficulty::Challenging);
        assert_eq!(Difficulty::parse("BALANCED"), Some(Difficulty::Balanced));
        assert_eq!(Difficulty::parse("easy"), None);
    }

    #[test]
    fn short_counts() {
        assert_eq!(short_count(500_000), "500k");
        assert_eq!(short_count(1_000_000), "1M");
        assert_eq!(short_count(1_500_000), "1.5M");
        assert_eq!(short_count(300), "300");
        assert_eq!(short_count(1_234), "1234");
    }

    #[test]
    fn garbage_bytes_are_rejected() {
        let rules = RulesConfig::standard(mako_core::rules::StackDepth::Bb20);
        for bytes in [&b""[..], b"MAKOSTRT", b"not an artifact at all, really"] {
            let e = CfrAgent::from_artifact_bytes(bytes, &rules, 0).unwrap_err();
            assert!(matches!(e, AgentError::Artifact(_)), "{e:?}");
        }
    }
}

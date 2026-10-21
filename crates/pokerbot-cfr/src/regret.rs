//! Regret matching and CFR update rules over the six abstract action slots
//! (M-10).
//!
//! An information set stores one slot per [`ActionKind`] in canonical order
//! (`Fold, Check, Call, HalfPot, Pot, AllIn`) plus a **legal mask** (bit `i`
//! set iff slot `i` is legal). Illegal slots never receive probability or
//! regret.
//!
//! - Regret matching (Hart & Mas-Colell 2000; Zinkevich et al. 2007): play
//!   legal actions in proportion to positive cumulative regret, and uniformly
//!   over legal actions if none is positive.
//! - Regret-matching+ (Tammelin 2014) is an optional flag: cumulative regrets
//!   are floored at 0 after every update. Plain external-sampling MCCFR
//!   (Lanctot et al. 2009) is the default.
//! - Strategy accumulation takes an explicit `weight`, so M-11 can supply a
//!   linear schedule (Brown & Sandholm 2019); the default weight is 1.
//!
//! The slice-based helpers from the Kuhn reference are re-exported for
//! game-agnostic use.

use pokerbot_core::rules::ActionKind;

pub use crate::toy::regret::{RegretNode, normalize_average, regret_matching, regret_matching_vec};

/// Number of abstract action slots.
pub const NUM_SLOTS: usize = 6;

/// Bitmask over the six slots.
pub type LegalMask = u8;

/// Mask with every slot legal.
pub const FULL_MASK: LegalMask = 0b11_1111;

/// Build a mask from action kinds.
pub fn mask_of(kinds: impl IntoIterator<Item = ActionKind>) -> LegalMask {
    kinds.into_iter().fold(0, |m, k| m | (1 << k.index()))
}

/// Whether slot `i` is legal in `mask`.
#[inline]
pub fn is_legal(mask: LegalMask, i: usize) -> bool {
    mask & (1 << i) != 0
}

/// Number of legal slots.
#[inline]
pub fn legal_count(mask: LegalMask) -> u32 {
    (mask & FULL_MASK).count_ones()
}

/// Masked regret matching into `out`.
///
/// # Panics
/// If `mask` has no legal slot.
pub fn regret_matching_masked(
    regrets: &[f64; NUM_SLOTS],
    mask: LegalMask,
    out: &mut [f64; NUM_SLOTS],
) {
    let n = legal_count(mask);
    assert!(n > 0, "information set with no legal actions");
    let mut sum = 0.0;
    for i in 0..NUM_SLOTS {
        let r = regrets[i];
        out[i] = if is_legal(mask, i) && r > 0.0 && r.is_finite() {
            r
        } else {
            0.0
        };
        sum += out[i];
    }
    if sum > 0.0 && sum.is_finite() {
        for o in out.iter_mut() {
            *o /= sum;
        }
    } else {
        let u = 1.0 / f64::from(n);
        for (i, o) in out.iter_mut().enumerate() {
            *o = if is_legal(mask, i) { u } else { 0.0 };
        }
    }
}

/// Masked average-strategy normalization (uniform over legal slots if the
/// information set has no accumulated mass).
pub fn average_masked(sums: &[f64; NUM_SLOTS], mask: LegalMask, out: &mut [f64; NUM_SLOTS]) {
    let n = legal_count(mask);
    assert!(n > 0, "information set with no legal actions");
    let total: f64 = (0..NUM_SLOTS)
        .filter(|&i| is_legal(mask, i) && sums[i] > 0.0)
        .map(|i| sums[i])
        .sum();
    if total > 0.0 && total.is_finite() {
        for i in 0..NUM_SLOTS {
            out[i] = if is_legal(mask, i) && sums[i] > 0.0 {
                sums[i] / total
            } else {
                0.0
            };
        }
    } else {
        let u = 1.0 / f64::from(n);
        for (i, o) in out.iter_mut().enumerate() {
            *o = if is_legal(mask, i) { u } else { 0.0 };
        }
    }
}

/// Update-rule switches. `Default` is plain ES-MCCFR.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct UpdateConfig {
    /// Floor cumulative regrets at zero after each update (RM+).
    pub regret_matching_plus: bool,
}

/// Iteration weighting (M-11), after Brown & Sandholm, "Solving
/// Imperfect-Information Games via Discounted Regret Minimization", AAAI 2019
/// (Linear CFR). With `t >= 1` the 1-based iteration number:
///
/// - `linear_averaging`: `S[a] += t · σ_t[a]` at opponent nodes (else weight 1).
/// - `linear_regrets`: `R[a] += t · (u_t[a] − u_t(node))` at traverser nodes
///   (else weight 1). Equivalent to Linear CFR's "multiply accumulated regrets
///   by t/(t+1) each iteration", up to a positive per-iteration scale, which
///   regret matching ignores.
///
/// The two flags are independent. Both off is plain ES-MCCFR.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Weighting {
    /// Weight average-strategy contributions by `t`.
    pub linear_averaging: bool,
    /// Weight regret contributions by `t`.
    pub linear_regrets: bool,
}

impl Weighting {
    /// Strategy weight at iteration `t` (`t = 0` is treated as 1).
    pub fn strategy_weight(self, t: u64) -> f64 {
        if self.linear_averaging {
            t.max(1) as f64
        } else {
            1.0
        }
    }

    /// Regret weight at iteration `t` (`t = 0` is treated as 1).
    pub fn regret_weight(self, t: u64) -> f64 {
        if self.linear_regrets {
            t.max(1) as f64
        } else {
            1.0
        }
    }
}

/// Packs update and weighting flags into a stable bitfield:
/// bit 0 = RM+, bit 1 = linear averaging, bit 2 = linear regrets.
pub fn flags_to_bits(update: UpdateConfig, weighting: Weighting) -> u32 {
    u32::from(update.regret_matching_plus)
        | u32::from(weighting.linear_averaging) << 1
        | u32::from(weighting.linear_regrets) << 2
}

/// Inverse of [`flags_to_bits`]; `None` if unknown bits are set.
pub fn flags_from_bits(bits: u32) -> Option<(UpdateConfig, Weighting)> {
    if bits & !0b111 != 0 {
        return None;
    }
    Some((
        UpdateConfig {
            regret_matching_plus: bits & 1 != 0,
        },
        Weighting {
            linear_averaging: bits & 2 != 0,
            linear_regrets: bits & 4 != 0,
        },
    ))
}

/// Traverser-node regret update: `R[a] += weight · (u[a] − u_node)` for
/// legal `a`, where `u_node = Σ σ[a]·u[a]`. Returns `u_node`.
///
/// In external sampling the sampled counterfactual value already carries the
/// importance weight (opponent and chance are sampled from their own
/// distributions), so no reach-probability factor appears here.
pub fn update_regrets(
    regrets: &mut [f64; NUM_SLOTS],
    mask: LegalMask,
    sigma: &[f64; NUM_SLOTS],
    action_utils: &[f64; NUM_SLOTS],
    weight: f64,
    cfg: UpdateConfig,
) -> f64 {
    let node: f64 = (0..NUM_SLOTS)
        .filter(|&i| is_legal(mask, i))
        .map(|i| sigma[i] * action_utils[i])
        .sum();
    for i in 0..NUM_SLOTS {
        if is_legal(mask, i) {
            regrets[i] += weight * (action_utils[i] - node);
            if cfg.regret_matching_plus && regrets[i] < 0.0 {
                regrets[i] = 0.0;
            }
        }
    }
    node
}

/// Opponent-node average-strategy accumulation: `S[a] += weight · σ[a]`.
pub fn accumulate_strategy(
    sums: &mut [f64; NUM_SLOTS],
    mask: LegalMask,
    sigma: &[f64; NUM_SLOTS],
    weight: f64,
) {
    for i in 0..NUM_SLOTS {
        if is_legal(mask, i) {
            sums[i] += weight * sigma[i];
        }
    }
}

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

use mako_core::rules::ActionKind;

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

//! Generic regret matching and average-strategy accumulation.
//!
//! Regret matching (Hart & Mas-Colell, 2000; used by CFR, Zinkevich et al. 2007):
//! the current strategy plays each legal action with probability proportional
//! to its positive cumulative regret, and uniformly if no regret is positive.
//!
//! These helpers are game-agnostic: they operate on slices indexed by legal
//! action, so they can be reused by the NLHE trainer with variable action
//! counts.

/// Compute the regret-matching strategy for `cumulative_regrets` into `out`.
///
/// - Positive-part normalization: `out[a] = max(R[a], 0) / Σ max(R, 0)`.
/// - Uniform fallback when every regret is `<= 0` (or non-finite / NaN, which
///   is treated as non-positive).
/// - Empty input produces empty output.
///
/// # Panics
/// If `out.len() != cumulative_regrets.len()`.
pub fn regret_matching(cumulative_regrets: &[f64], out: &mut [f64]) {
    assert_eq!(
        cumulative_regrets.len(),
        out.len(),
        "regret/out length mismatch"
    );
    let n = cumulative_regrets.len();
    if n == 0 {
        return;
    }
    let mut positive_sum = 0.0;
    for (o, &r) in out.iter_mut().zip(cumulative_regrets) {
        let p = if r > 0.0 && r.is_finite() { r } else { 0.0 };
        *o = p;
        positive_sum += p;
    }
    if positive_sum > 0.0 && positive_sum.is_finite() {
        for o in out.iter_mut() {
            *o /= positive_sum;
        }
    } else {
        out.fill(1.0 / n as f64);
    }
}

/// Allocating convenience wrapper around [`regret_matching`].
pub fn regret_matching_vec(cumulative_regrets: &[f64]) -> Vec<f64> {
    let mut out = vec![0.0; cumulative_regrets.len()];
    regret_matching(cumulative_regrets, &mut out);
    out
}

/// Normalize accumulated strategy weights into an average strategy.
/// Uniform if the total weight is zero (information set never reached).
pub fn normalize_average(strategy_sum: &[f64], out: &mut [f64]) {
    assert_eq!(strategy_sum.len(), out.len(), "sum/out length mismatch");
    let n = strategy_sum.len();
    if n == 0 {
        return;
    }
    let total: f64 = strategy_sum.iter().copied().filter(|x| *x > 0.0).sum();
    if total > 0.0 && total.is_finite() {
        for (o, &s) in out.iter_mut().zip(strategy_sum) {
            *o = if s > 0.0 { s / total } else { 0.0 };
        }
    } else {
        out.fill(1.0 / n as f64);
    }
}

/// Per-information-set CFR accumulators over a fixed number of legal actions.
#[derive(Debug, Clone, PartialEq)]
pub struct RegretNode {
    /// Cumulative counterfactual regret per action.
    pub regret_sum: Vec<f64>,
    /// Cumulative (possibly weighted) strategy mass per action.
    pub strategy_sum: Vec<f64>,
}

impl RegretNode {
    /// New node with `num_actions` zeroed accumulators.
    pub fn new(num_actions: usize) -> Self {
        Self {
            regret_sum: vec![0.0; num_actions],
            strategy_sum: vec![0.0; num_actions],
        }
    }

    /// Number of legal actions.
    pub fn num_actions(&self) -> usize {
        self.regret_sum.len()
    }

    /// Current regret-matching strategy.
    pub fn current_strategy(&self, out: &mut [f64]) {
        regret_matching(&self.regret_sum, out);
    }

    /// Add `regret[a]` to cumulative regrets.
    pub fn add_regrets(&mut self, regret: &[f64]) {
        assert_eq!(regret.len(), self.regret_sum.len());
        for (r, &d) in self.regret_sum.iter_mut().zip(regret) {
            *r += d;
        }
    }

    /// Accumulate `weight * strategy[a]` into the average-strategy sums.
    pub fn accumulate_strategy(&mut self, strategy: &[f64], weight: f64) {
        assert_eq!(strategy.len(), self.strategy_sum.len());
        for (s, &p) in self.strategy_sum.iter_mut().zip(strategy) {
            *s += weight * p;
        }
    }

    /// Normalized average strategy.
    pub fn average_strategy(&self, out: &mut [f64]) {
        normalize_average(&self.strategy_sum, out);
    }
}

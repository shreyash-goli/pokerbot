//! `mako-cfr`: counterfactual regret minimization for heads-up NLHE.
//!
//! * [`toy`]: Kuhn poker CFR / MCCFR reference (M-09)
//! * [`abstraction`]: preflop classes, history encoding, info keys, postflop
//!   equity buckets (M-07, M-08)
//! * [`equity`]: equity vs a random hand, suit canonicalization (M-08)
//! * [`regret`], [`infoset`], [`traverse`]: external-sampling MCCFR (M-10)

/// Crate version, exposed so the scaffold has a testable item.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

// Scaffold: reference workspace dependencies so the crate graph is exercised.
const _: &str = mako_core::VERSION;

pub mod abstraction;
pub mod equity;
pub mod infoset;
pub mod regret;
pub mod toy;
pub mod traverse;

#[cfg(test)]
mod tests {
    #[test]
    fn scaffold_version_is_set() {
        assert!(!super::VERSION.is_empty());
    }
}

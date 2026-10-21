//! `pokerbot-cfr`: counterfactual regret minimization for heads-up NLHE.
//!
//! * [`toy`]: Kuhn poker CFR / MCCFR reference
//! * [`abstraction`]: preflop classes, history encoding, info keys, postflop
//!   equity buckets
//! * [`equity`]: equity vs a random hand, suit canonicalization
//! * [`regret`], [`infoset`], [`traverse`]: external-sampling MCCFR
//! * [`checkpoint`], [`train`]: linear weighting, bounded resumable training
//! * [`artifact`]: versioned `.pokerbot` average-strategy artifact and policy reader
//! * [`config`]: strict training-config parser used by `pokerbot-train`
//! * [`infer`]: observation-only agents: CFR policy inference and baseline

/// Crate version, exposed so the scaffold has a testable item.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

// Scaffold: reference workspace dependencies so the crate graph is exercised.
const _: &str = pokerbot_core::VERSION;

pub mod abstraction;
pub mod artifact;
pub mod checkpoint;
pub mod config;
pub mod equity;
pub mod infer;
pub mod infoset;
pub mod regret;
pub mod toy;
pub mod train;
pub mod traverse;

#[cfg(test)]
mod tests {
    #[test]
    fn scaffold_version_is_set() {
        assert!(!super::VERSION.is_empty());
    }
}

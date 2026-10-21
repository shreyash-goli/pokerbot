//! Toy-game reference implementations used to validate CFR machinery.
//!
//! Before CFR is applied to abstracted no-limit hold'em, the regret-matching,
//! vanilla CFR, and external-sampling MCCFR code paths are validated on
//! **Kuhn poker** (Kuhn, 1950), whose equilibria are known in closed form.
//!
//! # Modules
//! - [`kuhn`]: game rules, terminal utilities, observation-only information-set keys.
//! - [`regret`]: generic regret matching and average-strategy accumulation
//!   (reusable by the NLHE trainer).
//! - [`cfr`]: vanilla tabular CFR (full chance enumeration) and
//!   external-sampling MCCFR.
//! - [`eval`]: exact expected value, best response, and exploitability.
//! - [`rng`]: a small, test-local deterministic SplitMix64 generator.
//!
//! # Benchmark (documented known equilibrium)
//! In Kuhn poker (cards J < Q < K, ante 1 each, single bet size 1, player 0
//! acts first) the Nash equilibria form a one-parameter family indexed by
//! `α ∈ [0, 1/3]`:
//!
//! | Information set              | P(bet / call)       |
//! |------------------------------|---------------------|
//! | P0 holds J, first action     | `α`                 |
//! | P0 holds Q, first action     | `0`                 |
//! | P0 holds K, first action     | `3α`                |
//! | P0 holds J, facing bet (pb)  | `0` (always fold)   |
//! | P0 holds Q, facing bet (pb)  | `α + 1/3`           |
//! | P0 holds K, facing bet (pb)  | `1`                 |
//! | P1 holds J, after check (p)  | `1/3` (bluff)       |
//! | P1 holds Q, after check (p)  | `0`                 |
//! | P1 holds K, after check (p)  | `1`                 |
//! | P1 holds J, facing bet (b)   | `0`                 |
//! | P1 holds Q, facing bet (b)   | `1/3`               |
//! | P1 holds K, facing bet (b)   | `1`                 |
//!
//! The game value for player 0 is `-1/18` under every equilibrium.
//!
//! References:
//! - H. W. Kuhn, "A simplified two-person poker", *Contributions to the Theory
//!   of Games* I, 1950.
//! - M. Zinkevich, M. Johanson, M. Bowling, C. Piccione, "Regret Minimization
//!   in Games with Incomplete Information", NIPS 2007 (vanilla CFR).
//! - M. Lanctot, K. Waugh, M. Zinkevich, M. Bowling, "Monte Carlo Sampling for
//!   Regret Minimization in Extensive Games", NIPS 2009 (external sampling).
//! - N. Brown, T. Sandholm, "Solving Imperfect-Information Games via
//!   Discounted Regret Minimization", AAAI 2019 (linear averaging).
//!
//! CFR average strategies converge toward an equilibrium of *this toy game*;
//! nothing here implies GTO play in hold'em.

pub mod cfr;
pub mod eval;
pub mod kuhn;
pub mod regret;
pub mod rng;

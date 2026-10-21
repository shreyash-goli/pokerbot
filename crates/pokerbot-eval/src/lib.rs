//! `pokerbot-eval`: independent, paired-seed, seat-swapped policy evaluation
//! (M-19).
//!
//! * [`agent`]: the `pokerbot_cfr::infer` agent contract (re-export)
//! * [`baselines`]: rule-bot baselines (random legal, check/call, tight)
//! * [`matchup`]: the paired match runner and its information boundary
//! * [`stats`]: mean / sample SD / normal CI / seeded bootstrap
//! * [`report`]: per-matchup BB/100 summaries, by position
//! * [`gate`]: locked PROMOTE/HOLD criteria and the disclaimer
//! * [`config`]: strict `configs/eval*.toml` parser
//! * [`evaluate`]: end-to-end orchestration
//! * [`output`]: text and JSON reports
//!
//! Methodology and limitations: `docs/EVALUATION.md`. Nothing here measures
//! exploitability or distance to equilibrium, and no GTO claim is made.

pub mod agent;
pub mod baselines;
pub mod config;
pub mod evaluate;
pub mod gate;
pub mod matchup;
pub mod output;
pub mod report;
pub mod stats;

/// Crate version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

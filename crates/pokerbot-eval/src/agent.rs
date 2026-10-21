//! The agent interface the evaluation harness drives: a re-export of the
//! `pokerbot_cfr::infer` contract.
//!
//! Agents see only a [`PlayerObservation`](pokerbot_core::observation::PlayerObservation)
//! for their own seat. The harness does **not** use the agent's own sampled
//! `action` beyond a legality check; it re-samples from the reported
//! `distribution` with an evaluation-owned RNG stream (see
//! [`crate::matchup`]), so that paired mirror matches are exactly
//! antisymmetric.

pub use pokerbot_cfr::infer::{
    Agent, AgentError, CfrAgent, Decision, DecisionSource, FallbackReason, InferStats,
};

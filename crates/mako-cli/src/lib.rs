//! `mako-cli` — the native terminal game (M-05: minimal playable loop).
//!
//! * [`args`] — hand-rolled command-line parsing
//! * [`baseline`] — the [`baseline::Bot`] trait and the untrained baseline bot
//! * [`input`] — parsing of player commands against the engine's legal actions
//! * [`render`] — plain/ANSI text rendering from a `PlayerObservation`
//! * [`game`] — the session loop (human vs bot) and seed derivation
//!
//! The binary (`src/main.rs`) is a thin wrapper over [`game::run_session`].
//! All game-state transitions go through `mako_core::state::HandState`;
//! this crate only ever submits actions taken from the engine's legal list.

pub mod args;
pub mod baseline;
pub mod game;
pub mod input;
pub mod render;

// Later milestones plug in the CFR policy (M-14) and analysis (M-17/M-18);
// reference those workspace crates so the dependency graph stays exercised.
const _: &str = mako_cfr::VERSION;
const _: &str = mako_analysis::VERSION;

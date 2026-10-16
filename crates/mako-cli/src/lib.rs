//! `mako-cli` — the native terminal game (M-05: minimal playable loop).
//!
//! * [`args`] — hand-rolled command-line parsing
//! * [`opponent`] — selects the opponent agent (CFR policy or untrained
//!   baseline, both `mako_cfr::infer::Agent`s) and builds its banner
//! * [`input`] — parsing of player commands against the engine's legal actions
//! * [`render`] — plain/ANSI text rendering from a `PlayerObservation`
//! * [`game`] — the session loop (human vs bot) and seed derivation
//!
//! The binary (`src/main.rs`) is a thin wrapper over [`game::run_session`].
//! All game-state transitions go through `mako_core::state::HandState`;
//! this crate only ever submits actions taken from the engine's legal list.

pub mod args;
pub mod game;
pub mod input;
pub mod opponent;
pub mod render;

// Analysis (M-17/M-18) plugs in later; keep the dependency graph exercised.
const _: &str = mako_analysis::VERSION;

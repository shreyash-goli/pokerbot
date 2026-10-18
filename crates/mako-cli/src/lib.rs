//! `mako-cli` — the native terminal game (M-05 loop, M-14 policy, M-15 UI).
//!
//! * [`args`] — hand-rolled command-line parsing
//! * [`opponent`] — selects the opponent agent (CFR policy or untrained
//!   baseline, both `mako_cfr::infer::Agent`s) and builds its banner
//! * [`input`] — parsing of player commands against the engine's legal actions
//! * [`ui`] — retro cream/sepia terminal UI rendered from a `PlayerObservation`
//! * [`game`] — the session loop (human vs bot) and seed derivation
//! * [`history`] — opening/resetting the saved hand history (M-17, storage
//!   and statistics in `mako-analysis`)
//!
//! The binary (`src/main.rs`) is a thin wrapper over [`game::run_session`].
//! All game-state transitions go through `mako_core::state::HandState`;
//! this crate only ever submits actions taken from the engine's legal list.

pub mod args;
pub mod game;
pub mod history;
pub mod input;
pub mod opponent;
pub mod ui;

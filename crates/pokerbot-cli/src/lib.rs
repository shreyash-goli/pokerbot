//! `pokerbot-cli` — the native terminal game (game loop, CFR policy, retro UI).
//!
//! * [`args`] — hand-rolled command-line parsing
//! * [`coach`] — the trained policy as the post-hand coach's mix source
//! * [`opponent`] — selects the opponent agent (CFR policy or untrained
//!   baseline, both `pokerbot_cfr::infer::Agent`s) and builds its banner
//! * [`input`] — parsing of player commands against the engine's legal actions
//! * [`ui`] — retro cream/sepia terminal UI rendered from a `PlayerObservation`
//! * [`game`] — the session loop (human vs bot) and seed derivation
//! * [`history`] — opening/resetting the saved hand history (storage
//!   and statistics in `pokerbot-analysis`)
//!
//! The binary (`src/main.rs`) is a thin wrapper over [`game::run_session`].
//! All game-state transitions go through `pokerbot_core::state::HandState`;
//! this crate only ever submits actions taken from the engine's legal list.

pub mod args;
pub mod coach;
pub mod game;
pub mod history;
pub mod input;
pub mod opponent;
pub mod ui;

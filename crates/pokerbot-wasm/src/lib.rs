//! `pokerbot-wasm`: the browser bridge.
//!
//! The same `pokerbot-core` engine and `pokerbot-cfr` policy reader that the native
//! game uses, compiled to `wasm32-unknown-unknown` and exposed to
//! JavaScript through `wasm-bindgen`. No training code runs here: the
//! browser only loads a trained `.pokerbot` artifact and samples from it.
//!
//! * [`session`]: pure-Rust [`GameSession`] (testable natively)
//! * [`encode`]: JSON built from the human's observation only
//! * [`json`]: hand-written JSON value and string escaping
//! * [`seeds`]: per-hand deck and bot seeds, identical to `pokerbot-cli`
//! * [`wasm`]: the `PokerbotSession` JS class
//!
//! No threads, filesystem, or clock are used. See `docs/WEB_INTEGRATION.md`.

pub mod encode;
pub mod json;
pub mod seeds;
pub mod session;
pub mod wasm;

pub use session::{BOT, BotKind, GameSession, HUMAN, SessionError};

/// Crate version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

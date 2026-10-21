//! Session seed derivation, bit-for-bit identical to the native terminal
//! game (`pokerbot-cli`'s `game` module), so a session seed deals the same
//! cards and seeds the same bot RNG natively and in the browser.
//!
//! Re-implemented here so the browser build does not depend on the CLI
//! crate; `tests/parity.rs` checks equality against `pokerbot_cli::game`.

use pokerbot_core::deck::Deck;
use pokerbot_core::rng::splitmix64_next;

/// Domain tag for per-hand decks (`"DECK"`).
pub const DOMAIN_DECK: u64 = 0x4445_434B;
/// Domain tag for the bot's sampling RNG (`"BOT"`).
pub const DOMAIN_BOT: u64 = 0x0042_4F54;

/// Derives an independent sub-seed from the session seed.
pub fn derive_seed(session: u64, domain: u64, index: u64) -> u64 {
    let mut s = session ^ domain.rotate_left(32);
    let a = splitmix64_next(&mut s);
    let mut t = a ^ index.wrapping_mul(0xD1B5_4A32_D192_ED03);
    splitmix64_next(&mut t)
}

/// Deck seed for hand `index` (0-based) of a session.
pub fn hand_seed(session: u64, index: u64) -> u64 {
    derive_seed(session, DOMAIN_DECK, index)
}

/// The deck for hand `index` (0-based) of a session.
pub fn hand_deck(session: u64, index: u64) -> Deck {
    Deck::from_seed(hand_seed(session, index))
}

/// Seed of the bot agent's sampling RNG (baseline or CFR).
pub fn bot_seed(session: u64) -> u64 {
    derive_seed(session, DOMAIN_BOT, 0)
}

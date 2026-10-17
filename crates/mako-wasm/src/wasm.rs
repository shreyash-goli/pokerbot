//! `#[wasm_bindgen]` wrappers over [`GameSession`].
//!
//! Every method returns JSON text (parse it with `JSON.parse`) or throws a
//! JS `Error` whose message starts with a stable code, e.g.
//! `"illegal_action: pot is not legal now; legal: fold, call, all_in"`.
//! Seeds are `u64`, i.e. JavaScript `BigInt` (`7n`, `BigInt(7)`).

use wasm_bindgen::prelude::*;

use crate::session::{GameSession, SessionError};

fn js_err(e: SessionError) -> JsError {
    JsError::new(&e.to_string())
}

/// A human-vs-bot session running entirely in the browser.
#[wasm_bindgen]
pub struct MakoSession {
    inner: GameSession,
}

#[wasm_bindgen]
impl MakoSession {
    /// `new MakoSession(depthBB, seed, policyBytes, bot)`:
    /// `depthBB` is 20, 50 or 100; `seed` a BigInt; `policyBytes` a
    /// `Uint8Array` with a `.mako` artifact (required for `bot = "cfr"`,
    /// ignored for `"baseline"`, may be `undefined`/`null` then).
    #[wasm_bindgen(constructor)]
    pub fn new(
        depth_bb: u32,
        seed: u64,
        policy_bytes: Option<Vec<u8>>,
        bot: &str,
    ) -> Result<MakoSession, JsError> {
        GameSession::new(depth_bb, seed, policy_bytes, bot)
            .map(|inner| MakoSession { inner })
            .map_err(js_err)
    }

    /// The `PublicState` JSON for the human player.
    pub fn snapshot(&self) -> String {
        self.inner.snapshot_json()
    }

    /// Plays the human's action (`fold|check|call|half_pot|pot|all_in`),
    /// then the bot's replies; returns the new `PublicState` JSON. Invalid
    /// input throws and changes nothing.
    pub fn step(&mut self, action: &str) -> Result<String, JsError> {
        self.inner.step(action).map_err(js_err)?;
        Ok(self.inner.snapshot_json())
    }

    /// Deals the next hand (stacks reset); throws mid-hand.
    #[wasm_bindgen(js_name = nextHand)]
    pub fn next_hand(&mut self) -> Result<String, JsError> {
        self.inner.next_hand().map_err(js_err)?;
        Ok(self.inner.snapshot_json())
    }

    /// Restarts the session with a new seed (score and bot RNG reset).
    pub fn reset(&mut self, seed: u64) -> Result<String, JsError> {
        self.inner.reset(seed).map_err(js_err)?;
        Ok(self.inner.snapshot_json())
    }

    /// The current hand's events as the human may see them (JSON).
    pub fn history(&self) -> String {
        self.inner.history_json()
    }

    /// Policy metadata JSON (label, checksum, iterations, abstraction).
    #[wasm_bindgen(js_name = strategyInfo)]
    pub fn strategy_info(&self) -> String {
        self.inner.strategy_info_json()
    }
}

/// The crate version.
#[wasm_bindgen(js_name = makoVersion)]
pub fn mako_version() -> String {
    crate::VERSION.to_string()
}

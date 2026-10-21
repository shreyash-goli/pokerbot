//! `#[wasm_bindgen]` wrappers over [`GameSession`].
//!
//! Every method returns JSON text (parse it with `JSON.parse`) or throws a
//! JS `Error` whose message starts with a stable code, e.g.
//! `"illegal_action: pot is not legal now; legal: fold, call, all_in"`.
//! Seeds are JavaScript `BigInt`s in `0..2^64` (`7n`, `BigInt(7)`).
//! Anything else (a `Number`, a negative or too-large `BigInt`) throws
//! `invalid_seed` instead of wrapping.

use wasm_bindgen::prelude::*;

use crate::session::{GameSession, SessionError};

fn js_err(e: SessionError) -> JsError {
    JsError::new(&e.to_string())
}

/// Exact `u64` from a JS `BigInt`; rejects other types and out-of-range
/// values (no modular wrap).
fn seed_from_js(v: JsValue) -> Result<u64, SessionError> {
    if !v.is_bigint() {
        let what = match v.as_f64() {
            Some(_) => "a Number (use a BigInt, e.g. 7n)",
            None if v.is_undefined() => "undefined",
            None if v.is_null() => "null",
            None if v.is_string() => "a string",
            None => "a non-BigInt value",
        };
        return Err(SessionError::InvalidSeed(what.to_string()));
    }
    u64::try_from(v).map_err(|_| SessionError::InvalidSeed("a BigInt outside 0..2^64".into()))
}

/// A human-vs-bot session running entirely in the browser.
#[wasm_bindgen]
pub struct PokerbotSession {
    inner: GameSession,
}

#[wasm_bindgen]
impl PokerbotSession {
    /// `new PokerbotSession(depthBB, seed, policyBytes, bot)`:
    /// `depthBB` is 20, 50 or 100; `seed` a BigInt; `policyBytes` a
    /// `Uint8Array` with a `.pokerbot` artifact (required for `bot = "cfr"`,
    /// ignored for `"baseline"`, may be `undefined`/`null` then).
    #[wasm_bindgen(constructor)]
    pub fn new(
        depth_bb: u32,
        #[wasm_bindgen(unchecked_param_type = "bigint")] seed: JsValue,
        policy_bytes: Option<Vec<u8>>,
        bot: &str,
    ) -> Result<PokerbotSession, JsError> {
        let seed = seed_from_js(seed).map_err(js_err)?;
        GameSession::new(depth_bb, seed, policy_bytes, bot)
            .map(|inner| PokerbotSession { inner })
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
    pub fn reset(
        &mut self,
        #[wasm_bindgen(unchecked_param_type = "bigint")] seed: JsValue,
    ) -> Result<String, JsError> {
        let seed = seed_from_js(seed).map_err(js_err)?;
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
#[wasm_bindgen(js_name = pokerbotVersion)]
pub fn pokerbot_version() -> String {
    crate::VERSION.to_string()
}

# Browser/website integration contract (M-22, implemented)

`crates/mako-wasm` compiles the native engine (`mako-core`) and the policy
reader and inference code (`mako-cfr::infer`) to WebAssembly with
`wasm-bindgen`. The browser runs the same Rust code as the terminal game.
Game rules and inference are not rewritten in TypeScript. No backend, API
key or network access is needed at runtime, and no training runs in the
browser: the page loads a `.mako` artifact that was trained offline and
samples from it.

The React demo and the Web Worker wrapper are M-23
(`examples/react-demo`, `packages/mako-browser`).

## Build

```sh
export PATH="$HOME/.cargo/bin:$PATH"
# Output dirs must be ignored or outside the repo.
wasm-pack build crates/mako-wasm --target bundler --out-dir ../../target/wasm-bundler
wasm-pack build crates/mako-wasm --target nodejs  --out-dir ../../target/wasm-node
node crates/mako-wasm/smoke/smoke.mjs target/wasm-node artifacts/smoke_20bb.mako
```

Measured at M-22, release build, wasm-opt as run by wasm-pack 0.15.1:
`mako_wasm_bg.wasm` is 142,210 bytes (62,880 gzipped). The 20BB smoke
artifact is about 890 KB and is fetched separately. The wasm module imports
only the wasm-bindgen glue (`throw`, `Error`, externref table). It uses no
threads, filesystem or clock.

## JavaScript API

```typescript
export class MakoSession {
  constructor(depthBB: 20 | 50 | 100, seed: bigint,
              policyBytes: Uint8Array | null | undefined, bot: 'cfr' | 'baseline');
  snapshot(): string;              // PublicState JSON
  step(action: MakoAction): string; // human acts, bot replies; new PublicState JSON
  nextHand(): string;              // next hand (stacks reset, button moves); PublicState
  reset(seed: bigint): string;     // fresh session, same depth/bot/policy; PublicState
  history(): string;               // current hand's events, human's view (JSON)
  strategyInfo(): string;          // policy metadata (JSON), never strategy contents
  free(): void;
}
export function makoVersion(): string;
export type MakoAction = 'fold' | 'check' | 'call' | 'half_pot' | 'pot' | 'all_in';
```

- Every method returns JSON text. Call `JSON.parse` on it. Serde is not
  used; the JSON is written by hand in `src/json.rs` and checked by a strict
  parser in the tests.
- **Seeds are `u64`, passed as JS `BigInt`** (`7n`, `BigInt(7)`), so every
  native seed, including those above 2^53, works unchanged. In JSON the seed
  is a decimal string, and checksums are `"0x…"` hex strings.
- `bot: 'cfr'` needs valid artifact bytes for the chosen depth. Missing or
  invalid bytes throw. **There is never a silent fallback to the
  baseline.** With `'baseline'` the bytes are ignored, and the
  snapshot/strategy info report `kind: "baseline"`.
- The human is seat Zero and holds the button on hand 1. The button
  alternates after that. Bot turns run inside the constructor, `step`,
  `nextHand` and `reset`. After any call, either `actor === 'human'` or the
  hand is over.
- Inference runs synchronously on the calling thread. The measured cost is
  well under 1 ms per decision (Node: about 0.02 ms per bot decision, about
  20 ms to load the smoke artifact). Use a Web Worker if needed (M-23).

### Errors

Errors are thrown as JS `Error`s whose message starts with a stable code and
a colon, e.g. `"illegal_action: pot is not legal now; legal: fold, call, all_in"`.

| code | when |
|---|---|
| `invalid_depth` | depth not 20/50/100 |
| `invalid_bot` | bot not exactly `cfr` / `baseline` |
| `policy_missing` | `cfr` without bytes |
| `policy_invalid` | corrupt, truncated or incompatible artifact |
| `policy_rules_mismatch` | artifact trained for another depth/blinds |
| `unknown_action` | not one of the six exact lowercase names |
| `illegal_action` | a known action that is not legal now |
| `hand_over` | `step` after the hand ended (call `nextHand`) |
| `hand_in_progress` | `nextHand` mid-hand |
| `not_your_turn` | defensive; unreachable through this API |
| `bot_error` / `engine_error` | internal invariant failure; the session is then poisoned |
| `session_poisoned` | any call after such a failure; `reset` to recover |

Input errors never change state. The tests compare the snapshot, history
and strategy info before and after every rejected call.

## `PublicState` (`schema: "mako.public_state.v1"`)

```typescript
type Who = 'human' | 'mako';
interface PublicState {
  schema: 'mako.public_state.v1';
  handNumber: number;                 // 1-based
  seed: string;                       // u64, decimal
  depthBB: number; startingStack: number; smallBlind: number; bigBlind: number;
  button: Who;
  street: 'preflop' | 'flop' | 'turn' | 'river' | 'showdown';
  board: string[];                    // e.g. ["Ah","7d","2c"]
  potChips: number;
  actor: Who | null;                  // null when the hand is over
  myCards: [string, string];          // the human's own hole cards
  myStack: number; botStack: number; myStreetChips: number; botStreetChips: number;
  legalActions: Array<{ type: MakoAction; chipsToCommit: number; raiseTo: number }>;
  actionLog: Array<{ actor: Who; street: string; type: MakoAction; raiseTo: number; chipsCommitted: number }>;
  terminal: null | { reason: 'fold'; winner: Who } | { reason: 'showdown' };
  payoff: null | { human: number; mako: number };      // net chips, zero-sum
  botCards?: [string, string];        // ONLY when terminal.reason === 'showdown'
  showdown?: { myHand: string; botHand: string };      // ONLY at showdown
  session: { handsCompleted: number; score: { human: number; mako: number }; poisoned: boolean };
  bot: { kind: 'cfr' | 'baseline'; label: string; artifactChecksum: string | null;
         notGto: true; disclaimer: string;
         statsAtLastHandEnd: { decisions: number; policyHits: number; fallbacks: number } };
  note: string;                       // entertainment-only / client-side caveat
}
```

`legalActions` is empty unless the human is to act. All chip values are
integers. The bot's policy-hit and fallback counters are only updated when a
hand ends. Mid-hand counts would leak information, because whether the bot's
current spot is in the policy depends on its private cards. The bot's action
distributions are never exposed, for the same reason.

`history()` returns `{schema: "mako.history.v1", events: [...]}`. Event types
are `hand_started`, `hole_cards` (the human's only), `action`, `board_dealt`,
`showdown` (both hands, only when the hand reached showdown) and
`hand_ended`. `strategyInfo()` returns `{schema: "mako.strategy_info.v1",
kind, label, notGto, disclaimer, trainedInBrowser: false, policy:
{checksum, label, trainerId, trainerSeed, iterations, infosets, depthBB,
blinds, abstractionFingerprint, abstraction{...}} | null, statsAtLastHandEnd}`.

## Hidden information

- Snapshots are built only from `HandState::observation(HUMAN)`, so the
  encoder never receives a `HandState`. The bot's cards can only come from
  `opponent_hole_cards`, which the engine fills only at a showdown terminal,
  and the encoder also checks `terminal == Showdown`. History is
  `events_for(Viewer::Seat(HUMAN))`, and the encoder drops any other seat's
  `HoleCards` event. `GameSession`'s `Debug` output is deliberately opaque.
- The bot agent receives only `observation(BOT)`.
- Tests (`crates/mako-wasm/tests/hidden.rs`):
  - Over 12 seeds × 15 hands × 2 bots, the bot's cards are computed
    independently from the seed. They never appear in a snapshot or history
    before a terminal state or after a fold, and they do appear correctly at
    showdown.
  - Swap test: 300 hand pairs whose decks differ only in the bot's hole
    cards give byte-identical snapshot and history JSON at every
    pre-showdown point.
- The API accepts no cards from JavaScript. The only inputs are depth,
  seed, artifact bytes (validated by `CfrAgent::from_artifact_bytes`), bot
  kind and action names, which are checked against the engine's legal list.
  Externally supplied cards never reach the equity functions.
- **Caveat:** WebAssembly runs on the client, so a determined user can read
  memory and see the bot's cards and the undealt board. This is a casual
  demo, not a secure game. Real concealment would need server-authoritative
  dealing. Never use this architecture for real money.

## Native parity

`crates/mako-wasm/src/seeds.rs` re-implements `mako-cli`'s `derive_seed` /
`hand_deck` and the bot-RNG domain, so the browser does not depend on the
CLI crate. `tests/parity.rs` checks the following:

- Decks and derived seeds are equal to `mako_cli::game` for 7 session seeds
  × 64 hands. The CLI's private domain constants are checked at the source
  level.
- Action-for-action identical histories and payoffs between `GameSession`
  and the CLI session-loop semantics (one `Match`, button from the human,
  one agent seeded once per session, observation-only bot). This holds for
  the smoke CFR artifact (3 seeds × 2 scripted humans × 20 hands) and for
  the baseline (10 seeds × 15 hands).
- An FNV-1a digest of every snapshot and history JSON in a check/call
  session is identical natively and from the wasm build under Node
  (`smoke.mjs`). The baseline digest is a constant both sides assert. The
  CFR (smoke artifact) digest `0x2580963c01fbe8fc` matched in both builds
  at M-22.

## Next.js / website notes (M-23)

- Import the package dynamically in a client component (`'use client'`, no
  SSR for wasm initialization). Serve `.wasm` as `application/wasm`.
- Fetch the `.mako` file as an `ArrayBuffer`, wrap it in `new Uint8Array(...)`
  and pass it to the constructor. Ship only the read-only artifact and this
  package. Training code is never bundled; `mako-wasm` links only the
  inference paths it calls.
- Show `bot.label`, `bot.disclaimer` and `note` to players. The CFR bot is
  not GTO. The smoke policy is weak.

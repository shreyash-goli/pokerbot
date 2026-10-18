# Browser/website integration contract (M-22 bridge, M-23 browser client)

`crates/mako-wasm` compiles the native engine (`mako-core`) and the policy
reader and inference code (`mako-cfr::infer`) to WebAssembly with
`wasm-bindgen`. The browser runs the same Rust code as the terminal game.
Game rules and inference are not rewritten in TypeScript. No backend, API
key or network access is needed at runtime, and no training runs in the
browser: the page loads a `.mako` artifact that was trained offline and
samples from it.

Layers:

- `crates/mako-wasm` (M-22): the wasm-bindgen `MakoSession` class, which
  returns JSON.
- `packages/mako-browser` (M-23): typed DTOs, the async `MakoClient` that
  runs the engine in a Web Worker, and the reusable `<MakoTable />` React
  component.
- `examples/react-demo` (M-23): a Vite + React demo that plays the 20BB
  smoke policy.

## Build

```sh
export PATH="$HOME/.cargo/bin:$PATH"
# Output dirs must be ignored or outside the repo.
wasm-pack build crates/mako-wasm --target bundler --out-dir ../../target/wasm-bundler
wasm-pack build crates/mako-wasm --target nodejs  --out-dir ../../target/wasm-node
node crates/mako-wasm/smoke/smoke.mjs target/wasm-node artifacts/smoke_20bb.mako
```

Measured at M-22, release build, wasm-opt as run by wasm-pack 0.15.1:
`mako_wasm_bg.wasm` is 142,210 bytes (62,880 gzipped) for `--target
bundler`. The `--target web` build used by `mako-browser` is 144,718 bytes
(about 65 KB gzipped). The 20BB smoke
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
- **Seeds are JS `BigInt`s in `0..2^64`** (`7n`, `BigInt(7)`), so every
  native `u64` seed, including those above 2^53, works unchanged. Anything
  else throws `invalid_seed` and is never wrapped modulo 2^64: a negative or
  too-large BigInt, a `Number`, a string, `null` or `undefined`.
  - **Number vs BigInt:** in the original M-22 build the seed parameter was
    a plain wasm-bindgen `u64`, so passing a `Number` (`new MakoSession(20, 7,
    ...)`) threw a bare `TypeError` ("Cannot convert 7 to a BigInt"), and a
    negative BigInt silently wrapped. Since M-23 the seed is validated
    explicitly and both cases throw `invalid_seed: ...`. The TypeScript
    signature still says `bigint`.
  - `MakoClient` (in `mako-browser`) also accepts a non-negative safe-integer
    `Number` and converts it with `BigInt(n)`. Unsafe, fractional and
    negative Numbers are rejected before reaching wasm.
  - In JSON the seed is a decimal string, and checksums are `"0x…"` hex
    strings.
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
| `invalid_seed` | seed not a BigInt in `0..2^64` |
| `unknown_action` | not one of the six exact lowercase names (input echoed, truncated to 64 characters) |
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

## Browser client and React component (M-23)

### `packages/mako-browser`

This package is consumed as TypeScript source; its `exports` point at
`src/`. The wasm glue lives in `packages/mako-browser/pkg/` (gitignored) and
is built with `npm run build:wasm`, which runs `wasm-pack --target web`. The
web target loads the `.wasm` file through `new URL(..., import.meta.url)`,
which Vite, webpack 5 and Next.js understand without a wasm plugin. Bundler
`asyncWebAssembly` / `vite-plugin-wasm` are not needed.

```typescript
import { createMakoClient, MakoTable, type PublicState } from 'mako-browser';

const client = createMakoClient();            // { worker: false } to force main thread
let s: PublicState = await client.start({ depthBB: 20, seed: 7n, policyUrl: '/policy/smoke_20bb.mako' });
s = await client.step('call');                // engine validates; MakoError with .code on failure
s = await client.nextHand();
const h = await client.history();             // HistoryDTO
client.dispose();
```

- `client.mode` is `'worker'` or `'main-thread'`.
  - The engine runs in a module Web Worker (`src/worker.ts`) by default, so
    policy loading and inference never block rendering.
  - If `Worker` is missing, cannot be constructed, or fails to load during
    `start`, the same engine runs on the main thread. Requests are processed
    strictly in order either way.
- The policy is fetched on the main thread (`policyUrl`) or passed as bytes
  (`policy`), then transferred to the worker. A failed fetch throws
  `policy_fetch_failed`. Client-side codes are `policy_fetch_failed`,
  `client_disposed`, `not_started` and `worker_failed`; all other codes are
  the engine's.
- The wasm module is loaded with a dynamic `import()` the first time a
  session starts, so it is a separate chunk and never part of the initial
  bundle.

`<MakoTable />` props:

| prop | type | default |
|---|---|---|
| `policyUrl` / `policy` | `string` / `Uint8Array` | needed for `bot="cfr"` |
| `depthBB` | `20 \| 50 \| 100` | `20` (must match the policy) |
| `seed` | `bigint \| number` | random per mount |
| `bot` | `'cfr' \| 'baseline'` | `'cfr'` |
| `theme` | `'sepia' \| 'terminal' \| Partial<MakoTheme>` | `'sepia'` |
| `worker` | `boolean` | `true` |
| `onState` | `(s: PublicState) => void` | none |
| `className`, `style` | | |

- The component renders only the `PublicState` JSON: board, pot, stacks,
  the human's cards, face-down bot cards until `botCards` arrives at
  showdown, legal-action buttons with their chip amounts (`chipsToCommit`,
  `raiseTo`), the hand log by street, the result, showdown hands, next hand,
  session score, the bot's label and checksum, the "not GTO" disclaimer, and
  the client-side note. It contains no poker rules.
- Styles are scoped under `.mako-table` and driven by `--mako-*` CSS
  variables. The cream/sepia palette matches the terminal theme. The layout
  is a single column below 560 px, with 44 px tap targets.
- If the CFR policy cannot be loaded, the component shows the error and a
  "Play the untrained baseline" button. **It never switches to the baseline
  by itself.**

### `examples/react-demo`

```sh
# from the repository root, once
scripts/train_smoke.sh                 # trains artifacts/smoke_20bb.mako (~2 min, gitignored)
cd examples/react-demo
npm ci
npm run build    # wasm-pack (--target web) -> copy policy -> tsc --noEmit -> vite build
npm run smoke    # local headless Chrome smoke (below)
npm run preview  # http://127.0.0.1:4173
```

- `scripts/copy-policy.mjs` copies `artifacts/smoke_20bb.mako` to
  `public/policy/` at build time. The file is never committed (`*.mako` is
  gitignored). If it is missing, the build still succeeds with a warning,
  and the page shows the explicit error screen.
- `?seed=N` in the URL fixes the session seed.
- Dependencies are pinned in `package-lock.json`. Runtime: `react`,
  `react-dom` (MIT). Dev: `vite` (MIT; pulls in rolldown/oxc MIT, postcss
  MIT, lightningcss MPL-2.0 at build time only), `typescript` 7 (Apache-2.0),
  `@types/react{,-dom}` (MIT), and `playwright-core` (Apache-2.0; no browser
  download). No `@vitejs/plugin-react`: Vite 8 compiles TSX with oxc, so
  only fast refresh is lost.
- Measured build: the app JS is 239 KB (75 KB gzipped), React included. The
  wasm glue chunk is 6 KB, the worker 1 KB, and the `.wasm` 145 KB (65 KB
  gzipped). The smoke policy (891 KB) is fetched separately when a session
  starts.

### Browser smoke (`examples/react-demo/scripts/smoke.mjs`)

The script serves `dist/` on `127.0.0.1` with `application/wasm`. It drives
the locally installed Google Chrome headless through `playwright-core`
(`channel: 'chrome'`; nothing is downloaded) and fails with a nonzero exit
if any check fails:

- At least 3 hands are played by clicking the rendered legal-action
  buttons, including at least one showdown.
- No bot card appears in the DOM (data attributes, aria labels or rendered
  glyphs) before a showdown. None appears after a fold. The cards revealed
  at showdown never appeared earlier in that hand.
- The engine runs in a Web Worker. There are no page or console errors.
- A 375×812 mobile viewport has no horizontal overflow and plays a hand.
- A missing policy shows `policy_fetch_failed`, and the baseline plays only
  after the user clicks it.
- With `Worker` deleted, the engine falls back to the main thread and plays
  a hand.

Use `--shots <dir>` for screenshots and `--hands N` for more hands.

### Next.js (App Router) embedding

1. Copy or link `packages/mako-browser` into the site, or depend on it with
   `"mako-browser": "file:..."`. Build its `pkg/` with `npm run build:wasm`,
   then add the package to `transpilePackages`, because it ships TypeScript
   source:
   ```js
   // next.config.mjs
   export default { transpilePackages: ['mako-browser'] };
   ```
2. Put the policy at `public/mako/smoke_20bb.mako` so it is served from
   `/mako/smoke_20bb.mako`, or set `policyUrl` to any same-origin path. Do
   not import it as a module.
3. Render the table only on the client, so wasm and Worker setup never run
   during SSR:
   ```tsx
   // app/poker/MakoEmbed.tsx
   'use client';
   import dynamic from 'next/dynamic';
   const MakoTable = dynamic(() => import('mako-browser').then((m) => m.MakoTable), {
     ssr: false,
     loading: () => <p>Loading Mako…</p>,
   });
   export default function MakoEmbed() {
     return <MakoTable policyUrl="/mako/smoke_20bb.mako" depthBB={20} theme="sepia" />;
   }
   ```
   Then use `<MakoEmbed />` from a server `page.tsx`.
4. Next.js serves its own emitted `.wasm` assets with `application/wasm`.
   If you host `dist/` or `.wasm` files elsewhere (nginx, S3, a CDN), make
   sure `.wasm` is served as `application/wasm` and `.mako` as
   `application/octet-stream`. Streaming instantiation needs the correct
   type. The glue falls back to a slower path with a console warning
   otherwise.
5. Bundle cost: about 7 KB of JS glue plus a 145 KB (65 KB gzipped) `.wasm`,
   loaded only when the table mounts, plus the policy file. React is shared
   with the site. Nothing calls a server or needs an API key. Training code
   is never shipped.
6. Keep the bot label, disclaimer and client-side note visible. The policy
   is a weak smoke policy and is not GTO, and browser play is for
   entertainment only.

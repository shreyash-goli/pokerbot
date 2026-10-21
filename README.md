# pokerbot

An independently implemented heads-up no-limit Texas Hold'em game ("Mako"):
a Rust rules engine, a from-scratch external-sampling MCCFR bot (with linear strategy averaging), a
retro terminal UI, an evaluation harness, and a WebAssembly bridge with a
React component for embedding in a website.

Simulation only. No real money, accounts or betting services. At runtime the
only network request is the browser fetching the policy file from your own site.

> **The bundled training budget yields a plumbing-grade "smoke" policy.** It
> beats deliberately weak rule bots and nothing more is claimed. It is an
> abstraction-based approximation, **not** a GTO or Nash-equilibrium strategy,
> and no exploitability has been measured. See [Limitations](#limitations).

## Quick start

Requires [rustup](https://rustup.rs). `rust-toolchain.toml` pins the toolchain
(with `rustfmt`, `clippy`, `wasm32-unknown-unknown`).

```sh
cargo test --workspace                     # all unit / integration / e2e tests
scripts/train_smoke.sh                     # train artifacts/smoke_20bb.mako (~2 min, ≤300 s cap)
cargo run --release -p mako-cli -- --depth 20 --hands 10   # play against it
```

Without a trained artifact, `mako --bot auto` falls back to an untrained
baseline bot and says so in a loud `!!` banner. `--bot cfr` refuses to start
instead. The trained policy is never silently replaced.

## What's included

| Path | Purpose |
|------|---------|
| `crates/mako-core` | Cards, seeded deck/RNG, 7-card evaluator, hand state machine, per-seat observations, event log/replay |
| `crates/mako-cfr` | Kuhn-poker CFR reference, preflop/postflop abstraction, ES-MCCFR trainer with linear averaging, checkpoints, `.mako` artifact format, `mako-train` binary, inference agent |
| `crates/mako-cli` | `mako` terminal game: retro table, coach/analysis, hand replay, history and stats, difficulty blends, experimental `--adapt` |
| `crates/mako-analysis` | History store, stats, equity, coach, opponent model |
| `crates/mako-eval` | `mako-eval`: paired-seed, seat-swapped evaluation against rule-bot baselines with confidence intervals and a pre-registered promotion gate |
| `crates/mako-wasm` | wasm-bindgen bridge exposing only public state to JavaScript |
| `packages/mako-browser` | Browser client (Web Worker) and `<MakoTable>` React component |
| `examples/react-demo` | Vite demo app and headless-Chrome smoke test |
| `configs/` | Training and evaluation configs (`smoke.toml`, `eval_smoke.toml`, `eval_adaptive*.toml`, `ci_tiny.toml`) |
| `docs/` | Specifications and per-component guides (below) |

### Architecture

```
mako-core ──► mako-cfr ──► mako-analysis ──► mako-eval
    │            │              │
    ├────────────┴──────────────┴──► mako-cli (native terminal game)
    └────────────┴─────────────────► mako-wasm ──► packages/mako-browser ──► examples/react-demo
```

The engine is the single source of truth for rules. Agents, the terminal UI
and the browser only see a seat's `Observation` or public state, so the
opponent's hole cards never leave the engine before showdown. The CLI and
WASM bridge each define the seed-domain constants. Tests pin both copies to the
same values, and a native-vs-WASM parity test checks that they make identical
decisions.

## Documentation

| Doc | Contents |
|-----|----------|
| [`docs/GAME_RULES.md`](docs/GAME_RULES.md) | Exact heads-up NLHE rules implemented (blinds, min-raise, all-in, side cases) |
| [`docs/CLI.md`](docs/CLI.md) | Every `mako` flag and in-game command, replay, colour/width |
| [`docs/STRATEGY_FORMAT.md`](docs/STRATEGY_FORMAT.md) | `.mako` artifact layout, checksums, abstraction fingerprint |
| [`docs/EVALUATION.md`](docs/EVALUATION.md) | Evaluation methodology, results, limitations |
| [`docs/WEB_INTEGRATION.md`](docs/WEB_INTEGRATION.md) | WASM API, hidden-information contract, React/Next.js embedding |
| [`docs/PROGRESS.md`](docs/PROGRESS.md) / [`docs/TASKS.json`](docs/TASKS.json) | Task-by-task build log with QA evidence |

## Train, play, evaluate, embed

**Train** (reproducible: same config + seed → same artifact checksum):

```sh
scripts/train_smoke.sh                                   # configs/smoke.toml: 20BB, 500k iterations, seed 20261115
scripts/train_smoke.sh --resume artifacts/smoke_20bb.ckpt
```

Outputs go to `artifacts/` (gitignored): `smoke_20bb.mako`, `.ckpt` and a
`.manifest.json` recording iterations, seed, checksum and abstraction
fingerprint. Only a 20BB smoke config ships. `--depth 50/100` play the
baseline bot unless you train an artifact for that depth.

**Play**: see [`docs/CLI.md`](docs/CLI.md). Every session prints its seed and
a replay command. `--difficulty training|balanced|challenging` blends the
policy with uniform-random legal play. `--no-history` keeps stats in memory,
and `--history <path>` / `MAKO_HOME` choose where they are saved (default
`~/.mako/`).

**Evaluate**:

```sh
cargo run --release -p mako-eval -- --smoke              # configs/eval_smoke.toml, ~20k hands
cargo run --release -p mako-eval -- --config <file.toml> --json out.json
```

Results are deterministic for a given config. For the adaptive comparison,
only the confirmation run's seed (3141592653, `configs/eval_adaptive_v2.toml`)
was committed before it was run. The earlier runs (A: original rule, config
not committed beforehand; C: post-hoc) are described in `docs/EVALUATION.md`
and `docs/PROGRESS.md`.

**Embed in a website**:

Needs Node ≥ 22 with npm, `wasm-pack` 0.15.1 (`cargo install wasm-pack
--version 0.15.1 --locked`), a trained `artifacts/smoke_20bb.mako` (above), and
Google Chrome for the smoke test. `npm ci` needs network access.

```sh
(cd packages/mako-browser && npm ci)   # the component's own react types
cd examples/react-demo
npm ci
npm run build      # wasm-pack --target web → copy policy → tsc → vite build
npm run smoke      # headless local Chrome full-hand smoke
npm run preview    # http://127.0.0.1:4173
```

`docs/WEB_INTEGRATION.md` covers the `mako-browser` package, the `<MakoTable>`
props, serving the `.mako` file, and Next.js App Router notes. The Next.js
steps have not been tested in a real Next.js app. The wasm module is about
145 KB (65 KB gzipped) and the smoke policy about 891 KB, fetched on demand.

## Checks

```sh
make ci            # fmt-check, check, clippy -D warnings, test, task-graph validation
make ci-local      # the above plus binary, artifact, eval, wasm and browser end-to-end steps
make clean-clone   # ci-local in a fresh `git archive HEAD` export with its own target dir
```

`ci-local` writes a summary table and log under `target/ci-local/`. Steps that
need something absent (no local Chrome, no trained artifact) print a loud
`!! SKIPPED` banner and appear as `SKIP` in the summary. A run whose only
non-PASS steps are SKIPs still exits 0, so read the summary.
`.github/workflows/ci.yml` runs every `ci_local.sh` step, from fmt to the browser
smoke, and trains the smoke artifact first. It has not been run remotely, since
nothing has been pushed.

## Benchmark results (smoke policy, 20BB)

From `mako-eval --smoke` on the committed config (BB/100, 95% CI):

| Opponent | Result |
|----------|--------|
| uniform random | +89.8 [65.1, 114.5] |
| check/call | +146.8 |
| tight hand-strength rule | +25.1 |

These opponents are intentionally weak, so the numbers only show that training
did something.

The experimental `--adapt` mode was tested in a pre-registered confirmation
run (seed 3141592653). It did better than the fixed policy against two
exploitable rule bots: +37.7 BB/100 [35.4, 40.0] vs check/call and +2.2
[1.7, 2.8] vs tight. It changed nothing against random play or against the
fixed policy itself, where its rules never fire. Its original rule did
worse than the fixed policy (run A, not pre-registered). Nothing
shows it helps against a balanced or human opponent, so it stays off by
default and is labelled experimental (`docs/EVALUATION.md`).

## Limitations

- Coarse abstraction: 169 preflop hand classes, 8 equity buckets on each
  postflop street, and only ½-pot, pot and all-in bet sizes. Unseen
  situations are played uniformly at random over legal actions and counted as
  *explicit fallback* in `stats` and in eval reports.
- No exploitability or best-response measurement. No claims against strong or
  human opponents.
- Only a 20BB policy is trained by the shipped scripts.
- Small known UI gaps: the stats-view history path is truncated at width 100,
  and the last hand of a session can't be analysed in that session.
- With difficulty blending or `--adapt`, a decision is still counted as
  coming from the policy in decision-source stats. Labels show the difficulty
  and adapted counts separately.

## Provenance and license

All code, tests and configs in this repository were newly written for it. Trained artifacts come only
from its own trainer and none are committed. The project is inspired at the feature level by
[PvRao-29/Mako](https://github.com/PvRao-29/Mako). No source, tests, binaries
or strategy files from that project are used. Algorithms follow the published
literature:

- Zinkevich, Johanson, Bowling, Piccione (2007), *Regret Minimization in Games
  with Incomplete Information* (CFR).
- Lanctot, Waugh, Zinkevich, Bowling (2009), *Monte Carlo Sampling for Regret
  Minimization in Extensive Games* (external-sampling MCCFR).
- Brown & Sandholm (2019), *Solving Imperfect-Information Games via Discounted
  Regret Minimization* (linear CFR / linear averaging).
- Kuhn (1950), *A Simplified Two-Person Poker* (toy reference game).

Released under the [MIT License](LICENSE). Shipped runtime dependencies
(`wasm-bindgen`, `react`, `react-dom`, `scheduler`) are MIT or Apache-2.0.
Build- and test-time tools pull in other permissive licenses: `unicode-ident`
is (MIT OR Apache-2.0) AND Unicode-3.0, `picocolors` is ISC, `source-map-js`
is BSD-3-Clause, and `lightningcss` (with its platform binaries) is MPL-2.0,
used by Vite at build time only.

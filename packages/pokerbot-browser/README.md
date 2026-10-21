# pokerbot-browser

This is a small TypeScript/React layer over the `pokerbot-wasm` WebAssembly build
of the Pokerbot engine and CFR policy reader. The Rust engine decides every rule.
This package only moves JSON and renders it.

- `createPokerbotClient({ worker?: boolean })` returns an async `PokerbotClient`
  with `start`, `step`, `nextHand`, `reset`, `snapshot`, `history`,
  `strategyInfo` and `dispose`. The engine runs in a module Web Worker. If
  the Worker cannot be created or fails to load, it runs on the main thread
  instead.
- `<PokerbotTable policyUrl depthBB seed bot theme worker onState />` is a
  self-contained table with a cream/sepia retro theme (or `"terminal"`).
- Typed DTOs (`PublicState`, `HistoryDTO`, `StrategyInfo`, `PokerbotError`
  with `.code`).

The package is consumed as TypeScript source (`exports` points at `src/`).
Vite handles that directly. In Next.js, add it to `transpilePackages`. The
wasm glue goes in `pkg/`, which is gitignored and built with
`npm run build:wasm` (wasm-pack, `--target web`). It is loaded with a
dynamic `import()` the first time a session starts, so it stays out of the
initial bundle.

See `docs/WEB_INTEGRATION.md` for the full contract and the Next.js steps.

Typecheck:

```sh
npm install
npm run build:wasm
npm run typecheck
```

# Mako React demo

This is a standalone Vite + React + TypeScript page that plays heads-up
no-limit hold'em against Mako's from-scratch CFR smoke policy. Everything
runs in the browser: the Rust engine and the policy reader are compiled to
WebAssembly, and inference runs in a Web Worker. No server, API key or
account is needed.

```sh
scripts/train_smoke.sh      # from the repo root, once: artifacts/smoke_20bb.mako (gitignored)
cd examples/react-demo
npm ci
npm run build               # needs wasm-pack and the wasm32-unknown-unknown target
npm run smoke               # headless local Chrome; add -- --shots <dir> for screenshots
npm run preview             # http://127.0.0.1:4173 (?seed=N for a fixed session)
```

The UI is the reusable `<MakoTable />` from `packages/mako-browser`. See
`docs/WEB_INTEGRATION.md` for the API, the Next.js embedding steps and the
honest limits: the smoke policy is weak and not GTO, and client-side code
cannot truly hide the bot's cards.

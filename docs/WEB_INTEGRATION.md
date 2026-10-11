# Browser/website integration contract (specification, not implemented)

The website should import a JavaScript package created from Rust via wasm-bindgen/wasm-pack. The Rust `mako-core` and `mako-cfr` inference code are used directly, NOT rewritten in TypeScript. The browser package is a library for existing Next.js/React portfolios; it must support code-splitting and an optional Web Worker.

## Proposed TypeScript surface

```typescript
export type MakoAction = 'fold'|'check'|'call'|'half_pot'|'pot'|'all_in';
export type DepthBB = 20|50|100;
export interface PublicState {
  handNumber: number; street: 'preflop'|'flop'|'turn'|'river'|'showdown';
  board: string[]; potChips: number; actor: 'human'|'mako';
  myCards: string[]; myStack: number; botStack: number;
  legalActions: Array<{type:MakoAction, chipsToCommit:number}>;
  botCards?: string[]; // ONLY if legitimately revealed at showdown
}
export interface MakoClient {
  start(options: {depthBB: DepthBB; seed: number; policyBytes: Uint8Array}): Promise<PublicState>;
  step(action: MakoAction): Promise<PublicState>;
  snapshot(): Promise<PublicState>;
  history(): Promise<ReadonlyArray<unknown>>;
  reset(seed?: number): Promise<PublicState>;
}
```

Interfaces are illustrative and may evolve with versioned contract tests. Never reveal the bot's unrevealed hole cards in snapshots or browser worker messages; only reveal at legitimate showdown or end-of-hand where rules permit. This matters because WASM running client-side generally cannot protect secrets against a determined user; for truly concealed bot cards, future deployment may need server-authoritative dealing. For a casual portfolio demo, treat browser game as entertainment rather than real-money/security-critical.

## Build and integration

- Rust core compiles to native and `wasm32-unknown-unknown`.
- `wasm-pack build crates/mako-wasm --target bundler` produces the JS/WASM package.
- The tiny `examples/react-demo` demonstrates importing it and loading a locally generated `.mako` file.
- Next.js personal-site integration should dynamically import the client in a client component, avoid SSR for WebAssembly initialization, load assets with correct MIME type, and avoid bundling large CFR training libraries.
- Train offline; ship only the compact read-only strategy artifact and inference code to browser. Separate native trainer dependencies behind Cargo features so `mako-wasm` never pulls threads, filesystem or native-only training components.
- Use seeded full-hand native vs browser parity tests and version-compatible artifact metadata.
- No external credentials/API required.

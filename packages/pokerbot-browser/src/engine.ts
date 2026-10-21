// Runs requests against one wasm PokerbotSession. Used inside the Web Worker
// and, as a fallback, on the main thread. The wasm module is loaded with a
// dynamic import so it stays out of the initial bundle.

import type { EngineRequest } from './protocol';

type WasmModule = typeof import('../pkg/pokerbot_wasm.js');
type Session = InstanceType<WasmModule['PokerbotSession']>;

let wasmPromise: Promise<WasmModule> | null = null;

/** Loads and instantiates the wasm module once per realm. */
export function loadWasm(): Promise<WasmModule> {
  wasmPromise ??= import('../pkg/pokerbot_wasm.js').then(async (m) => {
    await m.default();
    return m;
  });
  return wasmPromise;
}

export class Engine {
  private session: Session | null = null;
  private seed: bigint = 0n;

  async handle(req: EngineRequest): Promise<string> {
    const wasm = await loadWasm();
    switch (req.op) {
      case 'start': {
        const bytes = req.policy ? new Uint8Array(req.policy) : undefined;
        const next = new wasm.PokerbotSession(req.depthBB, req.seed, bytes, req.bot);
        this.session?.free();
        this.session = next;
        this.seed = req.seed;
        return next.snapshot();
      }
      case 'reset': {
        const s = this.need();
        const seed = req.seed ?? this.seed;
        const json = s.reset(seed);
        this.seed = seed;
        return json;
      }
      case 'step':
        return this.need().step(req.action);
      case 'nextHand':
        return this.need().nextHand();
      case 'snapshot':
        return this.need().snapshot();
      case 'history':
        return this.need().history();
      case 'strategyInfo':
        return this.need().strategyInfo();
    }
  }

  dispose(): void {
    this.session?.free();
    this.session = null;
  }

  private need(): Session {
    if (!this.session) throw new Error('not_started: call start() first');
    return this.session;
  }
}

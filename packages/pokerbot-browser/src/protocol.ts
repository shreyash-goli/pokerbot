// Messages between PokerbotClient and the engine (Web Worker or main thread).
// Payloads are JSON text from the wasm module; the client parses them.

export type EngineRequest =
  | { op: 'start'; depthBB: number; seed: bigint; policy: ArrayBuffer | null; bot: string }
  | { op: 'step'; action: string }
  | { op: 'nextHand' }
  | { op: 'reset'; seed: bigint | null }
  | { op: 'snapshot' }
  | { op: 'history' }
  | { op: 'strategyInfo' };

export type EngineResponse = { id: number; ok: true; json: string } | { id: number; ok: false; error: string };

export type WorkerRequest = EngineRequest & { id: number };

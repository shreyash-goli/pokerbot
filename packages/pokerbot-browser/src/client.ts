// PokerbotClient: an async, promise-based client for one game session.
//
// By default the wasm engine runs in a dedicated module Web Worker so CFR
// inference never blocks rendering. Where Workers are unavailable (or with
// `{ worker: false }`) the same engine runs on the main thread.

import { Engine } from './engine';
import type { EngineRequest, EngineResponse, WorkerRequest } from './protocol';
import {
  type BotKind,
  type DepthBB,
  type HistoryDTO,
  type PokerbotAction,
  PokerbotError,
  type PublicState,
  type StrategyInfo,
} from './types';

export interface StartOptions {
  depthBB: DepthBB;
  /** Session seed: a BigInt in 0..2^64, or a non-negative safe integer Number. */
  seed: bigint | number;
  /** `'cfr'` (default) requires a policy; it never falls back to the baseline. */
  bot?: BotKind;
  /** `.pokerbot` artifact bytes. */
  policy?: ArrayBuffer | Uint8Array;
  /** Or a URL to fetch the `.pokerbot` artifact from (same origin recommended). */
  policyUrl?: string;
}

export interface ClientOptions {
  /** Run the engine in a Web Worker (default true when available). */
  worker?: boolean;
}

export interface PokerbotClient {
  /** Where the engine runs. */
  readonly mode: 'worker' | 'main-thread';
  start(options: StartOptions): Promise<PublicState>;
  step(action: PokerbotAction): Promise<PublicState>;
  nextHand(): Promise<PublicState>;
  /** Restart with a new seed (same depth, bot and policy); omit to replay the same seed. */
  reset(seed?: bigint | number): Promise<PublicState>;
  snapshot(): Promise<PublicState>;
  history(): Promise<HistoryDTO>;
  strategyInfo(): Promise<StrategyInfo>;
  dispose(): void;
}

/** Converts a JS seed to BigInt without rounding or wrapping. */
export function toSeed(seed: bigint | number): bigint {
  if (typeof seed === 'bigint') return seed; // range checked by the engine
  if (!Number.isSafeInteger(seed) || seed < 0) {
    throw new PokerbotError(`invalid_seed: ${seed} is not a non-negative safe integer; pass a BigInt`);
  }
  return BigInt(seed);
}

async function policyBuffer(o: StartOptions): Promise<ArrayBuffer | null> {
  if (o.policy) {
    const u = o.policy instanceof Uint8Array ? o.policy : new Uint8Array(o.policy);
    return u.slice().buffer; // private copy, safe to transfer
  }
  if (o.policyUrl) {
    let res: Response;
    try {
      res = await fetch(o.policyUrl);
    } catch (e) {
      throw new PokerbotError(`policy_fetch_failed: ${o.policyUrl}: ${String(e)}`);
    }
    if (!res.ok) throw new PokerbotError(`policy_fetch_failed: ${o.policyUrl}: HTTP ${res.status}`);
    return res.arrayBuffer();
  }
  return null;
}

type Transport = (req: EngineRequest, transfer?: Transferable[]) => Promise<string>;

function workerTransport(): { send: Transport; close: () => void } | null {
  if (typeof Worker === 'undefined') return null;
  let w: Worker;
  try {
    w = new Worker(new URL('./worker.ts', import.meta.url), { type: 'module', name: 'pokerbot-engine' });
  } catch {
    return null;
  }
  let next = 1;
  const pending = new Map<number, { resolve: (s: string) => void; reject: (e: Error) => void }>();
  w.onmessage = (ev: MessageEvent<EngineResponse>) => {
    const r = ev.data;
    const p = pending.get(r.id);
    if (!p) return;
    pending.delete(r.id);
    if (r.ok) p.resolve(r.json);
    else p.reject(new PokerbotError(r.error));
  };
  w.onerror = (ev: ErrorEvent) => {
    ev.preventDefault();
    for (const p of pending.values()) p.reject(new PokerbotError(`worker_failed: ${ev.message || 'worker error'}`));
    pending.clear();
  };
  return {
    send: (req, transfer = []) =>
      new Promise((resolve, reject) => {
        const id = next++;
        pending.set(id, { resolve, reject });
        w.postMessage({ ...req, id } satisfies WorkerRequest, transfer);
      }),
    close: () => {
      w.terminate();
      for (const p of pending.values()) p.reject(new PokerbotError('client_disposed: client was disposed'));
      pending.clear();
    },
  };
}

function mainThreadTransport(): { send: Transport; close: () => void } {
  const engine = new Engine();
  let queue: Promise<unknown> = Promise.resolve();
  return {
    send: (req) => {
      const run = queue.then(() => engine.handle(req));
      queue = run.catch(() => undefined);
      return run.catch((e: unknown) => {
        throw new PokerbotError(e instanceof Error ? e.message : String(e));
      });
    },
    close: () => engine.dispose(),
  };
}

/** Creates a client. The engine (and wasm) loads lazily on `start`. */
export function createPokerbotClient(options: ClientOptions = {}): PokerbotClient {
  let t = (options.worker ?? true) ? workerTransport() : null;
  let mode: PokerbotClient['mode'] = t ? 'worker' : 'main-thread';
  let transport = t ?? mainThreadTransport();
  let disposed = false;
  const call = async <T>(req: EngineRequest, transfer?: Transferable[]): Promise<T> => {
    if (disposed) throw new PokerbotError('client_disposed: client was disposed');
    return JSON.parse(await transport.send(req, transfer)) as T;
  };
  return {
    get mode() {
      return mode;
    },
    async start(o) {
      const seed = toSeed(o.seed);
      const policy = await policyBuffer(o);
      const req: EngineRequest = { op: 'start', depthBB: o.depthBB, seed, policy, bot: o.bot ?? 'cfr' };
      // Keep a copy: the buffer is transferred to the worker.
      const backup = mode === 'worker' && policy ? policy.slice(0) : null;
      try {
        return await call<PublicState>(req, policy ? [policy] : []);
      } catch (e) {
        if (!(e instanceof PokerbotError) || e.code !== 'worker_failed' || disposed) throw e;
        // The worker could not load (e.g. no module-worker support): run
        // the same engine on the main thread instead.
        transport.close();
        transport = mainThreadTransport();
        mode = 'main-thread';
        t = null;
        return call<PublicState>({ ...req, policy: backup });
      }
    },
    step: (action) => call<PublicState>({ op: 'step', action }),
    nextHand: () => call<PublicState>({ op: 'nextHand' }),
    reset: (seed) => call<PublicState>({ op: 'reset', seed: seed === undefined ? null : toSeed(seed) }),
    snapshot: () => call<PublicState>({ op: 'snapshot' }),
    history: () => call<HistoryDTO>({ op: 'history' }),
    strategyInfo: () => call<StrategyInfo>({ op: 'strategyInfo' }),
    dispose() {
      disposed = true;
      transport.close();
    },
  };
}

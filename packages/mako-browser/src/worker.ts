// Web Worker entry: owns the wasm session so inference never blocks the UI.

import { Engine } from './engine';
import type { EngineResponse, WorkerRequest } from './protocol';

const engine = new Engine();
// Typed structurally so the package needs only the DOM lib.
const scope = self as unknown as {
  onmessage: ((ev: MessageEvent<WorkerRequest>) => void) | null;
  postMessage(msg: EngineResponse): void;
};

// Requests are processed strictly in order.
let queue: Promise<void> = Promise.resolve();

scope.onmessage = (ev: MessageEvent<WorkerRequest>) => {
  const { id, ...req } = ev.data;
  queue = queue.then(async () => {
    let res: EngineResponse;
    try {
      res = { id, ok: true, json: await engine.handle(req) };
    } catch (e) {
      res = { id, ok: false, error: e instanceof Error ? e.message : String(e) };
    }
    scope.postMessage(res);
  });
};

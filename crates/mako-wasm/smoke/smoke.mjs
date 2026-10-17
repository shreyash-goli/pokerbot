// Node smoke test for the mako-wasm bridge (M-22).
//
//   node crates/mako-wasm/smoke/smoke.mjs <pkg-dir built with --target nodejs> [policy.mako]
//
// Loads the wasm package, reads the policy bytes from disk, plays complete
// seeded hands through the JS API choosing legal actions only, and checks:
// no bot hole cards before a showdown terminal or after a fold, typed errors
// for bad input without state change, chip conservation, zero-sum payoffs,
// and a native-parity digest. Exits nonzero on any failure.

import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { performance } from "node:perf_hooks";

const [pkgDir, policyPath] = process.argv.slice(2);
if (!pkgDir) {
  console.error("usage: node smoke.mjs <pkg-dir> [policy.mako]");
  process.exit(2);
}
const require = createRequire(import.meta.url);
const { MakoSession, makoVersion } = require(resolve(pkgDir, "mako_wasm.js"));

// Same constant as BASELINE_DIGEST_SEED7 in crates/mako-wasm/tests/parity.rs.
const BASELINE_DIGEST_SEED7 = 0x13924bcf7385351bn;
const HANDS = 20;

let failures = 0;
function check(cond, msg) {
  if (!cond) {
    failures++;
    console.error("FAIL:", msg);
  }
}
function code(fn) {
  try {
    fn();
  } catch (e) {
    return String(e.message).split(":")[0];
  }
  return null;
}

// mulberry32: a tiny seeded JS RNG for the scripted human.
function rng(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function fnv(h, text) {
  for (const b of Buffer.from(text, "utf8")) {
    h ^= BigInt(b);
    h = (h * 0x100000001b3n) & 0xffffffffffffffffn;
  }
  return h;
}

function checkCallDigest(s, hands) {
  let h = 0xcbf29ce484222325n;
  for (let i = 0; i < hands; i++) {
    if (i > 0) s.nextHand();
    let text = s.snapshot();
    h = fnv(h, text);
    for (let st = JSON.parse(text); st.terminal === null; st = JSON.parse(text)) {
      const l = st.legalActions.map((a) => a.type);
      const a = l.includes("check") ? "check" : l.includes("call") ? "call" : l[0];
      text = s.step(a);
      h = fnv(h, text);
    }
    h = fnv(h, s.history());
  }
  return h;
}

/// Plays `hands` hands with random legal actions; returns stats.
function playSession(s, hands, seed) {
  const r = rng(seed);
  const stats = { hands: 0, showdowns: 0, folds: 0, steps: 0, ms: 0, net: 0 };
  for (let i = 0; i < hands; i++) {
    if (i > 0) s.nextHand();
    const texts = [];
    let st = JSON.parse(s.snapshot());
    const total = 2 * st.startingStack;
    texts.push(s.snapshot(), s.history());
    for (let guard = 0; st.terminal === null; guard++) {
      check(guard < 500, "hand did not finish");
      check(st.actor === "human", "bot turns must run automatically");
      check(st.myStack + st.botStack + st.potChips === total, "chip conservation");
      check(!("botCards" in st), "botCards before terminal");
      check(st.myCards.length === 2, "two private cards");
      const legal = st.legalActions.map((a) => a.type);
      // Invalid input: typed error, no state change.
      const before = s.snapshot();
      check(code(() => s.step("raise")) === "unknown_action", "unknown action rejected");
      for (const k of ["fold", "check", "call", "half_pot", "pot", "all_in"]) {
        if (!legal.includes(k)) check(code(() => s.step(k)) === "illegal_action", `${k} illegal`);
      }
      check(code(() => s.nextHand()) === "hand_in_progress", "nextHand mid-hand rejected");
      check(s.snapshot() === before, "rejected input changed state");
      const a = legal[Math.floor(r() * legal.length)];
      const t0 = performance.now();
      const text = s.step(a);
      stats.ms += performance.now() - t0;
      stats.steps++;
      st = JSON.parse(text);
      texts.push(text, s.history());
    }
    check(st.payoff.human + st.payoff.mako === 0, "zero-sum payoff");
    check(code(() => s.step("fold")) === "hand_over", "step after hand end rejected");
    stats.net += st.payoff.human;
    stats.hands++;
    if (st.terminal.reason === "showdown") {
      stats.showdowns++;
      check(Array.isArray(st.botCards) && st.botCards.length === 2, "botCards at showdown");
      // The revealed cards must not appear in any earlier text of this hand.
      for (const t of texts.slice(0, -2)) {
        for (const c of st.botCards) check(!t.includes(`"${c}"`), `bot card ${c} leaked pre-showdown`);
      }
    } else {
      stats.folds++;
      check(!("botCards" in st), "no botCards after fold");
    }
  }
  return stats;
}

console.log(`mako-wasm ${makoVersion()} smoke (Node ${process.version})`);

// Construction errors are explicit.
check(code(() => new MakoSession(20, 1n, undefined, "cfr")) === "policy_missing", "cfr without policy");
check(code(() => new MakoSession(20, 1n, new Uint8Array([1, 2, 3]), "cfr")) === "policy_invalid", "garbage policy");
check(code(() => new MakoSession(30, 1n, undefined, "baseline")) === "invalid_depth", "bad depth");
check(code(() => new MakoSession(20, 1n, undefined, "gto")) === "invalid_bot", "bad bot");

// Native parity digest (baseline: needs no artifact).
const digest = checkCallDigest(new MakoSession(20, 7n, undefined, "baseline"), HANDS);
check(digest === BASELINE_DIGEST_SEED7, `baseline digest ${digest.toString(16)} != native`);
console.log(`baseline check/call digest seed 7 x${HANDS}: 0x${digest.toString(16).padStart(16, "0")} (native: match=${digest === BASELINE_DIGEST_SEED7})`);

const base = playSession(new MakoSession(20, 3n, null, "baseline"), HANDS, 3);
console.log(`baseline: ${base.hands} hands, ${base.showdowns} showdowns, ${base.folds} folds, human net ${base.net}`);

if (policyPath) {
  const bytes = new Uint8Array(readFileSync(policyPath));
  const t0 = performance.now();
  const s = new MakoSession(20, 7n, bytes, "cfr");
  const loadMs = performance.now() - t0;
  const info = JSON.parse(s.strategyInfo());
  check(info.kind === "cfr" && info.notGto === true && info.trainedInBrowser === false, "strategy info");
  check(code(() => new MakoSession(50, 1n, bytes, "cfr")) === "policy_rules_mismatch", "depth mismatch");
  console.log(`policy: ${policyPath} (${bytes.length} bytes) checksum ${info.policy.checksum}, ${info.policy.iterations} iterations, label "${info.policy.label}", load ${loadMs.toFixed(1)} ms`);
  const cfrDigest = checkCallDigest(new MakoSession(20, 7n, bytes, "cfr"), HANDS);
  console.log(`cfr check/call digest seed 7 x${HANDS}: 0x${cfrDigest.toString(16).padStart(16, "0")}`);
  const st = playSession(s, HANDS, 7);
  const bot = JSON.parse(s.snapshot()).bot.statsAtLastHandEnd;
  console.log(`cfr: ${st.hands} hands, ${st.showdowns} showdowns, ${st.folds} folds, human net ${st.net}; bot ${bot.decisions} decisions (${bot.policyHits} policy, ${bot.fallbacks} fallback)`);
  console.log(`cfr: ${(st.ms / Math.max(1, bot.decisions)).toFixed(3)} ms per bot decision (step wall time incl. engine + JSON, ${st.steps} human steps)`);
} else {
  console.log("no policy path given: CFR section skipped");
}

if (failures) {
  console.error(`SMOKE FAILED: ${failures} check(s)`);
  process.exit(1);
}
console.log("SMOKE OK");

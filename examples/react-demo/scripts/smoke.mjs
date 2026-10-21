// Browser smoke test for the built demo.
//
//   npm run build && node scripts/smoke.mjs [--shots <dir>] [--hands N]
//
// Serves dist/ on 127.0.0.1 (random port, local only), drives the locally
// installed Google Chrome headless through playwright-core (no browser
// download), plays complete hands by clicking the legal-action buttons the
// page renders, and checks:
//   * the bot's cards never appear in the DOM (text, attributes or labels)
//     before a showdown, and never after a fold;
//   * at least 3 complete hands, with at least one showdown reveal;
//   * the engine runs in a Web Worker; no page errors or console errors;
//   * a 375x812 mobile viewport renders without horizontal overflow and
//     plays a hand.
// Exits nonzero on any failure.

import { createReadStream, existsSync, mkdirSync, statSync } from 'node:fs';
import { createServer } from 'node:http';
import { extname, join, normalize, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

import { chromium } from 'playwright-core';

const args = process.argv.slice(2);
const opt = (name, dflt) => {
  const i = args.indexOf(name);
  return i >= 0 && args[i + 1] ? args[i + 1] : dflt;
};
const root = resolve(fileURLToPath(new URL('../dist', import.meta.url)));
const shots = opt('--shots', null);
const minHands = Number(opt('--hands', '3'));
const MAX_HANDS = 40;

if (!existsSync(join(root, 'index.html'))) {
  console.error(`no build at ${root}; run npm run build first`);
  process.exit(2);
}
if (shots) mkdirSync(shots, { recursive: true });

// ---------------------------------------------------------------- server
const MIME = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.wasm': 'application/wasm',
  '.pokerbot': 'application/octet-stream',
  '.svg': 'image/svg+xml',
  '.json': 'application/json',
};
const server = createServer((req, res) => {
  const url = new URL(req.url ?? '/', 'http://127.0.0.1');
  let p = normalize(join(root, decodeURIComponent(url.pathname)));
  if (!p.startsWith(root + sep) && p !== root) {
    res.writeHead(403).end();
    return;
  }
  if (existsSync(p) && statSync(p).isDirectory()) p = join(p, 'index.html');
  if (!existsSync(p)) {
    res.writeHead(404).end('not found');
    return;
  }
  res.writeHead(200, { 'content-type': MIME[extname(p)] ?? 'application/octet-stream' });
  createReadStream(p).pipe(res);
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const base = `http://127.0.0.1:${server.address().port}/`;

// ---------------------------------------------------------------- checks
let failures = 0;
const fail = (msg) => {
  failures++;
  console.error('FAIL:', msg);
};
const check = (cond, msg) => cond || fail(msg);

// Small seeded RNG for the scripted clicker.
let rs = 12345;
const allLogLines = [];
const rand = () => ((rs = (Math.imul(rs, 1103515245) + 12345) >>> 0) / 4294967296);

function watch(page, label, allowed = null) {
  page.on('pageerror', (e) => fail(`${label}: page error: ${e.message}`));
  page.on('console', (m) => {
    if (m.type() === 'error' && !(allowed && allowed.test(m.text()))) fail(`${label}: console error: ${m.text()}`);
  });
}

const table = '[data-testid="pokerbot-table"]';

/** Waits until the engine is idle at a human decision or a finished hand. */
async function settled(page, previousHtml) {
  await page.waitForFunction(
    ({ sel, prev }) => {
      const t = document.querySelector(sel);
      if (!t) return false;
      const st = t.getAttribute('data-status');
      if (st === 'error') return true;
      return (
        t.getAttribute('aria-busy') === 'false' &&
        (st === 'your-turn' || st === 'hand-over') &&
        t.outerHTML !== prev
      );
    },
    { sel: table, prev: previousHtml },
    { timeout: 20000 },
  );
  const status = await page.getAttribute(table, 'data-status');
  if (status === 'error') {
    throw new Error(`table error: ${await page.textContent('[data-testid="load-error"]')}`);
  }
  return status;
}

// CLI wording: "You call 1", "Pokerbot bets 2 (to 4, half pot)", "You raise to 6 (pot)".
const LOG_LINE =
  /^(You|Pokerbot) (folds?|checks?|calls? \d+|bets? \d+( \((to \d+, )?(half pot|pot|all-in)\))?|raises? to \d+ \((half pot|pot|all-in)\))$/;
async function checkLog(page, label) {
  const lines = await page.$$eval('[data-testid="log"] li', (ls) => ls.map((l) => l.textContent.trim()));
  for (const l of lines) check(LOG_LINE.test(l), `${label}: unexpected log wording "${l}"`);
  const pinned = await page.$eval('[data-testid="log"]', (el) => el.scrollTop + el.clientHeight >= el.scrollHeight - 2);
  check(pinned, `${label}: log not scrolled to the latest entry`);
  return lines;
}

const html = (page) => page.$eval(table, (t) => t.outerHTML);
const botCardCodes = (page) =>
  page.$$eval('[data-testid="bot-cards"] [data-card]', (els) => els.map((e) => e.getAttribute('data-card')));

function cardForms(code) {
  const ranks = { T: '10' };
  const suits = { c: ['♣', 'clubs'], d: ['♦', 'diamonds'], h: ['♥', 'hearts'], s: ['♠', 'spades'] };
  const r = ranks[code[0]] ?? code[0];
  const [sym, name] = suits[code[1]];
  return [`data-card="${code}"`, `${r} of ${name}`, `>${r}</span><span class="pokerbot-suit">${sym}<`];
}

/** Plays one hand by clicking legal buttons; returns its summary. */
async function playHand(page, label, shotPrefix) {
  const seen = [];
  let status = await settled(page, '');
  for (let guard = 0; status === 'your-turn'; guard++) {
    if (guard > 200) throw new Error('hand did not finish');
    const h = await html(page);
    seen.push(h);
    check((await botCardCodes(page)).length === 0, `${label}: bot cards rendered face-up mid-hand`);
    const buttons = await page.$$eval('[data-testid="actions"] button[data-action]', (bs) =>
      bs.map((b) => b.getAttribute('data-action')),
    );
    check(buttons.length > 0, `${label}: no action buttons on your turn`);
    // Mostly check/call (to reach showdowns), sometimes any legal action.
    const passive = buttons.find((b) => b === 'check') ?? buttons.find((b) => b === 'call');
    const pick = passive && rand() < 0.75 ? passive : buttons[Math.floor(rand() * buttons.length)];
    await page.click(`[data-testid="actions"] button[data-action="${pick}"]`);
    status = await settled(page, h);
    allLogLines.push(...(await checkLog(page, label)));
  }
  const final = await html(page);
  const showdown = (await page.$('[data-testid="showdown"]')) !== null;
  const revealed = await botCardCodes(page);
  if (showdown) {
    check(revealed.length === 2, `${label}: showdown without both bot cards`);
    for (const c of revealed) {
      for (const form of cardForms(c)) {
        for (const h of seen) check(!h.includes(form), `${label}: bot card ${c} visible before showdown (${form})`);
      }
    }
  } else {
    check(revealed.length === 0, `${label}: bot cards shown after a fold`);
  }
  check(/You win|Pokerbot wins|Split pot/.test(await page.textContent('[data-testid="result"]')), `${label}: result text`);
  if (shotPrefix && showdown) {
    await page.screenshot({ path: `${shotPrefix}-showdown.png`, fullPage: true });
    await page.locator('[data-testid="log"]').screenshot({ path: `${shotPrefix}-log.png` });
  }
  return { showdown, decisions: seen.length, final };
}

async function nextHand(page, prevHtml) {
  await page.click('[data-testid="next-hand"]');
  await page.waitForFunction(
    ({ sel, prev }) => {
      const t = document.querySelector(sel);
      return t && t.outerHTML !== prev && t.getAttribute('aria-busy') === 'false';
    },
    { sel: table, prev: prevHtml },
  );
}

const browser = await chromium.launch({ channel: 'chrome', headless: true });
try {
  // ------------------------------------------------------------ desktop
  const desk = await browser.newPage({ viewport: { width: 1280, height: 900 } });
  watch(desk, 'desktop');
  await desk.goto(`${base}?seed=7`);
  await desk.waitForSelector(`${table}[data-status="your-turn"], ${table}[data-status="hand-over"], ${table}[data-status="error"]`);
  const footer = await desk.textContent('footer');
  check(footer.includes('engine: worker'), `engine not in a Web Worker: ${footer}`);
  check(/NOT GTO/.test(footer), 'disclaimer missing');
  const label = await desk.textContent('[data-testid="bot-label"]');
  check(label.startsWith('cfr 20BB'), `unexpected bot label ${label}`);
  if (shots) await desk.screenshot({ path: join(shots, 'desktop-start.png'), fullPage: true });

  let hands = 0;
  let showdowns = 0;
  let decisions = 0;
  let score = '';
  while (hands < MAX_HANDS && (hands < minHands || showdowns < 1)) {
    const r = await playHand(desk, `desktop hand ${hands + 1}`, shots && showdowns === 0 ? join(shots, 'desktop') : null);
    hands++;
    decisions += r.decisions;
    if (r.showdown) showdowns++;
    // A hand can end before any human decision (Pokerbot folds first to act),
    // so compare the session count right after each hand.
    score = await desk.textContent('[data-testid="score"]');
    const played = Number(/\((\d+) played\)/.exec(score)?.[1]);
    check(played >= hands, `score shows ${score} after ${hands} hands`);
    await nextHand(desk, r.final);
  }
  check(hands >= minHands && showdowns >= 1, `only ${hands} hands / ${showdowns} showdowns`);
  console.log(`desktop: ${hands} hands played by clicking (${showdowns} showdowns), ${decisions} human clicks; ${score.trim()}`);
  console.log(`desktop: bot "${label}"`);

  // ------------------------------------------------------------- mobile
  const ctx = await browser.newContext({
    viewport: { width: 375, height: 812 },
    deviceScaleFactor: 2,
    isMobile: true,
    hasTouch: true,
  });
  const mob = await ctx.newPage();
  watch(mob, 'mobile');
  await mob.goto(`${base}?seed=11`);
  await mob.waitForSelector(`${table}[data-status="your-turn"], ${table}[data-status="hand-over"]`);
  const overflow = await mob.evaluate(() => ({
    scrollWidth: document.documentElement.scrollWidth,
    innerWidth: window.innerWidth,
    wide: [...document.querySelectorAll('[data-testid="pokerbot-table"] *')]
      .filter((e) => e.getBoundingClientRect().right > window.innerWidth + 0.5)
      .map((e) => e.className || e.tagName)
      .slice(0, 5),
  }));
  check(overflow.scrollWidth <= overflow.innerWidth, `mobile horizontal overflow: ${JSON.stringify(overflow)}`);
  check(overflow.wide.length === 0, `mobile elements past the right edge: ${overflow.wide.join(', ')}`);
  if (shots) await mob.screenshot({ path: join(shots, 'mobile-start.png'), fullPage: true });
  const m = await playHand(mob, 'mobile hand 1', null);
  if (shots) {
    await mob.screenshot({ path: join(shots, 'mobile-hand-over.png'), fullPage: true });
    await mob.locator('[data-testid="log"]').screenshot({ path: join(shots, 'mobile-log.png') });
  }
  console.log(
    `mobile 375x812: scrollWidth ${overflow.scrollWidth} <= ${overflow.innerWidth}; played 1 hand (${m.showdown ? 'showdown' : 'fold'})`,
  );

  // ---------------------------------------- missing policy: no silent swap
  const np = await browser.newPage();
  watch(np, 'no-policy', /404|Failed to load resource/);
  await np.route('**/*.pokerbot', (r) => r.fulfill({ status: 404, body: 'missing' }));
  await np.goto(`${base}?seed=3`);
  await np.waitForSelector(`${table}[data-status="error"]`);
  const err = await np.textContent('[data-testid="load-error"]');
  check(err.startsWith('policy_fetch_failed'), `missing policy error: ${err}`);
  await np.click('[data-testid="use-baseline"]');
  await np.waitForSelector(`${table}[data-status="your-turn"], ${table}[data-status="hand-over"]`);
  const bl = await np.textContent('[data-testid="bot-label"]');
  check(bl.startsWith('baseline (untrained'), `baseline label: ${bl}`);
  await playHand(np, 'baseline hand', null);
  console.log(`missing policy: explicit "${err.split(':')[0]}" error; user-chosen baseline plays ("${bl}")`);

  // --------------------------------------------- no Worker: main thread
  const nw = await browser.newPage();
  watch(nw, 'no-worker');
  await nw.addInitScript(() => {
    delete window.Worker;
  });
  await nw.goto(`${base}?seed=7`);
  await nw.waitForSelector(`${table}[data-status="your-turn"], ${table}[data-status="hand-over"]`);
  const f2 = await nw.textContent('footer');
  check(f2.includes('engine: main-thread'), `fallback mode: ${f2}`);
  await playHand(nw, 'main-thread hand', null);
  console.log('no Worker: engine fell back to main-thread and played a hand');
} catch (e) {
  fail(String(e?.stack ?? e));
} finally {
  await browser.close();
  server.close();
}

const betLines = [...new Set(allLogLines.filter((l) => /bet|raise/.test(l)))];
console.log(`log wording: ${allLogLines.length} lines checked; e.g. ${betLines.slice(0, 4).map((l) => `"${l}"`).join(', ')}`);
if (shots) console.log(`screenshots: ${shots}`);
if (failures) {
  console.error(`BROWSER SMOKE FAILED: ${failures} check(s)`);
  process.exit(1);
}
console.log('BROWSER SMOKE OK');

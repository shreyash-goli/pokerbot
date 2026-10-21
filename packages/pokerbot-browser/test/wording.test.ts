// Run: node --experimental-strip-types test/wording.test.ts
// Cases mirror the CLI's tests in crates/pokerbot-cli/src/ui/table.rs.
import { strict as assert } from 'node:assert';

import { phrase, wordLog } from '../src/wording.ts';

assert.equal(phrase('half_pot', 4, 3, 1, 1, true), 'raise to 4 (half pot)');
assert.equal(phrase('half_pot', 4, 2, 2, 0, false), 'bets 2 (to 4, half pot)');
assert.equal(phrase('pot', 12, 12, 0, 0, false), 'bets 12 (pot)');
assert.equal(phrase('all_in', 30, 30, 0, 0, true), 'bet 30 (all-in)');
assert.equal(phrase('all_in', 40, 34, 6, 6, false), 'raises to 40 (all-in)');
assert.equal(phrase('call', 6, 4, 2, 4, true), 'call 4');
assert.equal(phrase('check', 2, 0, 2, 0, false), 'checks');

// Limp, BB bets the option, then a flop bet and raise.
const ctx = { button: 'human', smallBlind: 1, bigBlind: 2 } as const;
const lines = wordLog(
  [
    { actor: 'human', street: 'preflop', type: 'call', raiseTo: 2, chipsCommitted: 1 },
    { actor: 'pokerbot', street: 'preflop', type: 'half_pot', raiseTo: 4, chipsCommitted: 2 },
    { actor: 'human', street: 'preflop', type: 'call', raiseTo: 4, chipsCommitted: 2 },
    { actor: 'pokerbot', street: 'flop', type: 'check', raiseTo: 0, chipsCommitted: 0 },
    { actor: 'human', street: 'flop', type: 'pot', raiseTo: 8, chipsCommitted: 8 },
    { actor: 'pokerbot', street: 'flop', type: 'half_pot', raiseTo: 20, chipsCommitted: 20 },
    { actor: 'human', street: 'flop', type: 'all_in', raiseTo: 36, chipsCommitted: 28 },
  ],
  ctx,
).map((l) => l.text);
assert.deepEqual(lines, [
  'You call 1',
  'Pokerbot bets 2 (to 4, half pot)',
  'You call 2',
  'Pokerbot checks',
  'You bet 8 (pot)',
  'Pokerbot raises to 20 (half pot)',
  'You raise to 36 (all-in)',
]);
// Button raise preflop faces the BB: a raise.
assert.deepEqual(
  wordLog([{ actor: 'pokerbot', street: 'preflop', type: 'pot', raiseTo: 6, chipsCommitted: 5 }], {
    ...ctx,
    button: 'pokerbot',
  }).map((l) => l.text),
  ['Pokerbot raises to 6 (pot)'],
);
console.log('wording tests OK');

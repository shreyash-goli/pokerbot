// Action wording, matching the terminal game's convention
// (crates/pokerbot-cli/src/ui/table.rs `phrase`):
//   * no unmatched bet to face -> "bets X (half pot)"; when the actor already
//     has chips in on this street (the BB option after a limp), the amount
//     is what goes in now, with the total: "bets 2 (to 4, half pot)";
//   * facing a bet -> "raises to X (pot)";
//   * all-ins are tagged "(all-in)"; calls are "calls N".
// Pure presentation over public JSON: contributions are rebuilt from the
// button, the blinds and each logged action's `raiseTo`. No rules decided.

import type { ActionLogEntry, PokerbotAction, PublicState, StreetName, Who } from './types';

const TAG: Partial<Record<PokerbotAction, string>> = { half_pot: 'half pot', pot: 'pot', all_in: 'all-in' };

/** `you` selects "call" over "calls". */
export function phrase(
  kind: PokerbotAction,
  raiseTo: number,
  committed: number,
  before: number,
  toCall: number,
  you: boolean,
): string {
  const v = (base: string, third: string) => (you ? base : third);
  const tag = TAG[kind];
  switch (kind) {
    case 'fold':
      return v('fold', 'folds');
    case 'check':
      return v('check', 'checks');
    case 'call':
      return `${v('call', 'calls')} ${committed}`;
  }
  if (toCall === 0) {
    const notes: string[] = [];
    if (before > 0) notes.push(`to ${raiseTo}`);
    if (tag) notes.push(tag);
    const amount = before > 0 ? committed : raiseTo;
    return `${v('bet', 'bets')} ${amount}${notes.length ? ` (${notes.join(', ')})` : ''}`;
  }
  return `${v('raise', 'raises')} to ${raiseTo}${tag ? ` (${tag})` : ''}`;
}

export interface LogLine {
  street: StreetName;
  actor: Who;
  text: string;
}

type Ctx = Pick<PublicState, 'button' | 'smallBlind' | 'bigBlind'>;

/** Worded log lines ("Pokerbot raises to 6 (pot)"), in order. */
export function wordLog(log: readonly ActionLogEntry[], s: Ctx): LogLine[] {
  const out: LogLine[] = [];
  let street: StreetName | null = null;
  const c: Record<Who, number> = { human: 0, pokerbot: 0 };
  const other = (w: Who): Who => (w === 'human' ? 'pokerbot' : 'human');
  for (const e of log) {
    if (e.street !== street) {
      street = e.street;
      c.human = 0;
      c.pokerbot = 0;
      if (street === 'preflop') {
        c[s.button] = s.smallBlind;
        c[other(s.button)] = s.bigBlind;
      }
    }
    const before = c[e.actor];
    const toCall = Math.max(0, c[other(e.actor)] - before);
    const you = e.actor === 'human';
    out.push({
      street: e.street,
      actor: e.actor,
      text: `${you ? 'You' : 'Pokerbot'} ${phrase(e.type, e.raiseTo, e.chipsCommitted, before, toCall, you)}`,
    });
    c[e.actor] = e.raiseTo;
  }
  return out;
}

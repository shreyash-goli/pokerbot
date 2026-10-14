# Mako game rules (heads-up no-limit hold'em, six-action abstraction)

Status: specification for M-01, implemented by `crates/mako-core/src/rules.rs`
(betting/accounting) and `card.rs`/`deck.rs`/`rng.rs` (M-02). The hand state
machine (M-04) must follow this document. Written independently from the
standard public rules of Texas hold'em; no code or text from other poker
projects was used.

## 1. Table and units

- Exactly two seats, one standard 52-card deck, no jokers, no antes.
- All amounts are whole chips, type `Chips = u32`. No fractional chips ever
  exist; all divisions round **down** (`floor`) and any remainder is handled by
  an explicit rule (§5.3, §7.3).
- `RulesConfig { small_blind, big_blind, starting_stack }`. Default blinds
  1/2. `StackDepth` Bb20 / Bb50 / Bb100 gives starting stacks 40 / 100 / 200.
- Validation: `1 <= small_blind <= big_blind`, `1 <= starting_stack <=
  u32::MAX / 4`. Anything else is `RulesError::InvalidConfig`.
- Both stacks are reset to `starting_stack` at the start of **every** hand;
  sessions track score (sum of per-hand payoffs) only.

## 2. Positions, button and blinds

- Positions are `Button` and `BigBlind`. Heads-up, the button **is** the
  small blind.
- The button alternates every hand: `button_seat(first, h) = (first + h) % 2`.
- Blinds: the button posts `min(small_blind, stack)`, the big blind posts
  `min(big_blind, stack)`. A player who cannot cover a blind posts the whole
  stack and is all-in. Posting a blind is **not** a voluntary action.
- Action order: preflop the button acts first; on flop, turn and river the
  big blind acts first.

## 3. Dealing and streets

- Shuffle: Fisher–Yates over the ordered deck (index order `2c 2d 2h 2s … As`,
  `index = rank*4 + suit`) using the seeded `MakoRng` (xoshiro256**, seeded via
  SplitMix64; unbiased bounded draws by Lemire's method). Same seed ⇒ same
  deck on every platform.
- Dealing order (recommended for M-04): hole cards one at a time, alternating,
  starting with the big blind: BB, Button, BB, Button (deck positions 0–3).
  Board: flop = positions 4–6, turn = 7, river = 8. No burn cards.
- Streets: Preflop (0 board cards), Flop (3 revealed, 3 total), Turn (+1, 4),
  River (+1, 5), then Showdown.
- If at the end of a betting round at least one player is all-in, no further
  betting is possible: remaining board cards are dealt with no actions and the
  hand goes to showdown.

## 4. Actions

Six action classes, canonical order: `Fold, Check, Call, HalfPot, Pot, AllIn`.
Each legal action is returned as `LegalAction { kind, raise_to,
chips_to_commit }`:

- `raise_to` — the actor's **total contribution on this street** after the
  action (for Fold/Check it is the unchanged contribution).
- `chips_to_commit` — chips moved from the actor's stack now
  (`raise_to - actor_street`).

Only actions returned by `legal_actions` may be applied. Any other request
(e.g. Check while facing a bet) is `RulesError::IllegalAction` and must not
mutate state (`resolve_action`).

## 5. Legality and sizing

Input (`BettingSnapshot`): `actor_street`, `opp_street`, `actor_stack`,
`opp_stack`, `pot` (all chips committed this hand by both players, current
street included), `last_full_raise`, `big_blind`, `can_raise`.

Consistency (`validate`, else `InconsistentSnapshot`/`ActorAllIn`):
`opp_street >= actor_street` (it is only the actor's turn if they do not lead),
`pot >= actor_street + opp_street`, `big_blind > 0`, `actor_stack > 0`, and
`pot + stacks <= 2 * (u32::MAX/4)`.

Definitions: `to_call = opp_street - actor_street`;
`min_inc = max(last_full_raise, big_blind)`;
`pot_after_call = pot + to_call`; `all_in_to = actor_street + actor_stack`.

### 5.1 Passive actions
- **Fold**: legal iff `to_call > 0`.
- **Check**: legal iff `to_call == 0`.
- **Call**: legal iff `to_call > 0`; commits `min(to_call, actor_stack)`.
  If the stack is smaller than `to_call` this is a **call for less** (all-in).

### 5.2 When raising is possible
Bet/raise options exist only if **all** hold: `can_raise`, `opp_stack > 0`
(no raising an all-in player), and `actor_stack > to_call` (otherwise the
all-in is just a call; `AllIn` is then omitted and `Call` kept).

### 5.3 Sized options (pot-after-call basis)
- **Pot**: `raise_to = opp_street + pot_after_call`.
- **HalfPot**: `raise_to = opp_street + floor(pot_after_call / 2)`.
- With no bet facing (`to_call == 0`) these are bets of `pot` and
  `floor(pot/2)` on top of the matched contribution.
- **Min-raise bump**: if `raise_to - opp_street < min_inc`, set
  `raise_to = opp_street + min_inc`.
- **All-in cap**: if `raise_to >= all_in_to`, the sized option is dropped
  (AllIn covers it).
- **Dedup**: if HalfPot and Pot resolve to the same `raise_to`, HalfPot is
  dropped.
- **AllIn**: `raise_to = all_in_to`, present whenever §5.2 holds.

Note: with pot-after-call sizing a HalfPot increment is always at least the
amount being raised over, so the bump only binds when the pot is below two big
blinds or for unusual configs; it is still enforced.

### 5.4 Worked examples (1/2 blinds; verified in `tests/rules.rs`)
| Situation | Legal list (raise_to / commit) |
|---|---|
| Preflop, SB first to act, 100BB (pot 3, to_call 1) | Fold, Call 2/1, HalfPot 4/3, Pot 6/5, AllIn 200/199 |
| BB option after SB limp (pot 4, to_call 0) | Check, HalfPot 4/2, Pot 6/4, AllIn 200/198 |
| Flop opening bet, pot 12, stacks 194 | Check, HalfPot 6/6, Pot 12/12, AllIn 194/194 |
| Facing a 12 bet into 12 (pot 24) | Fold, Call 12/12, HalfPot 30/30, Pot 48/48, AllIn 194/194 |
| 20BB, SB facing 3-bet to 18 (actor 6, stack 34, pot 24) | Fold, Call 18/12, HalfPot 36/30, AllIn 40/34 (Pot 54 dropped) |
| Facing 50 with 30 behind | Fold, Call 30/30 (call for less; no AllIn) |

## 6. Raise bookkeeping and reopening

Per street the state machine keeps `last_full_raise` (reset to `big_blind` at
the start of every street, including preflop after blinds), and for each
player `has_acted` (reset to false each street; blinds do not count) and
`can_raise` (reset to true each street).

After an action from snapshot `s` (`next_can_raise`):
- `increment = raise_to - opp_street` (0 for Fold/Check/Call).
- A **full raise** is `increment > 0 && increment >= min_inc`; it sets
  `last_full_raise = increment`. A short (all-in) raise leaves it unchanged.
- The opponent's `can_raise` becomes `!opponent_has_acted || full_raise`.
  So a short all-in raise does **not** reopen betting to a player who already
  acted this street: they may only call or fold.

Betting round ends when both players have acted and contributions are equal,
when a player calls all-in for less, or on a fold. Note: because stacks are
equal at the start of every hand, a player making a short all-in raise is
all-in, so the opponent is also blocked by `opp_stack == 0`; the reopening
flag is kept for correctness with unequal stacks.

State-machine formulation (M-04, `state.rs`): a seat **needs to act** iff it
has chips behind, its street contribution does not exceed the opponent's, and
either it is facing chips (`street_c < opp_street_c`) or it has not yet acted
this street while the opponent still has chips behind. After every non-fold
action the opponent acts if it needs to; otherwise the round closes. At the
start of a round (including preflop right after the blinds) the first actor
of §2 acts if it needs to, else the other seat, else the round closes
immediately. This covers blinds that put a player all-in: e.g. a big blind
all-in from posting still lets the button fold or call, and a button all-in
from posting the small blind goes straight to the runout with the big
blind's excess refunded (§7.2).

When a round closes: if either player is all-in, the remaining board is
dealt and the hand goes to showdown with no further actions; otherwise the
next street begins (or showdown after the river). The public action history
records voluntary actions only; blinds are implied by the config and button.
The API reports street `Showdown` for every terminal state, including a fold.

## 7. Settlement

1. **Fold**: the other player wins the contested pot.
2. **Unmatched chips**: before any award, the larger total contributor is
   refunded `|c_button - c_bb|` (uncalled bet or excess over an all-in for
   less). The contested pot is `2 * min(c_button, c_bb)`.
3. **Showdown**: each player's best five of seven cards (M-03 evaluator); the
   better hand wins the contested pot; equal hands split it with
   `split_pot(pot, n)`: each gets `floor(pot / n)` and the `pot % n` odd chips
   go one each to winners in priority order, with the **big blind
   (out-of-position player) first**. Heads-up without antes the contested
   pot is always even, so the odd-chip rule exists only as a safety rule.
4. Net payoff per position = `refund + award - contribution`.

## 8. Invariants (must hold after every transition)

- Conservation: `stack_button + stack_bb + pot == 2 * starting_stack`
  (`check_conservation`); after settlement the pot is 0 and stacks sum to
  `2 * starting_stack`.
- Zero-sum: per-hand payoffs sum to 0 (`check_zero_sum`).
- No negative stacks (enforced by `u32` and by `chips_to_commit <= stack`).
- Every aggressive option has `raise_to > opp_street`, and non-all-in options
  have increment `>= min_inc`.
- Legal list is in canonical order with unique kinds and strictly increasing
  aggressive `raise_to`.

## 9. Limitations

- The six-action abstraction is a deliberate discretisation; it is **not**
  full no-limit bet sizing.
- The RNG is for simulation only, not cryptographic.

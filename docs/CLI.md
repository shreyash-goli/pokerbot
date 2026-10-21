# `pokerbot` terminal game

`pokerbot` plays heads-up no-limit hold'em in the terminal: you against a bot,
for a fixed number of hands, with stacks reset every hand and the button
alternating. The engine (`pokerbot-core`) decides every legal action; the
terminal only ever submits an action taken from the engine's legal list.
The UI is original work written for this project; its warm
cream/sepia look is inspired by a reference screenshot, not copied from any
other program.

```
cargo run -p pokerbot-cli -- --depth 20 --hands 3 --seed 7
```

## Options

| Option | Meaning |
|---|---|
| `--seed <u64>` | Session seed. Default: from the clock. Always printed, together with a replay command. |
| `--hands <n>` | Hands to play, `1..=100000` (default 10). |
| `--depth <20\|50\|100>` | Starting stack in big blinds at blinds 1/2 (40, 100 or 200 chips). Default 20. |
| `--bot <auto\|cfr\|baseline>` | Opponent (default `auto`); see [Opponents](#opponents). |
| `--strategy <path>` | Policy artifact. Default `artifacts/smoke_<depth>bb.pokerbot`, relative to the working directory. |
| `--auto <checkcall\|random>` | Plays *your* seat with a scripted policy and reads no input (tests, demos). |
| `--color <auto\|never\|16\|256\|truecolor>` | Colour depth (default `auto`). |
| `--no-color` | Same as `--color never`. |
| `--glyphs <auto\|unicode\|ascii>` | Suit symbols and box frames, or plain ASCII (default `auto`). |
| `--width <48..240>` | Layout width in columns (default: `COLUMNS`, else 80). |
| `--difficulty <training\|balanced\|challenging>` | How the CFR bot's trained mix is blended with uniform-random legal play (default `challenging` = as-is); see [Difficulty](#difficulty). |
| `--adapt` | Experimental, off by default: the bot conservatively adapts to your public tendencies (never your cards); not evaluated as stronger. |
| `--history <path>` / `--no-history` | Where completed hands are saved (default `$POKERBOT_HOME/session-history.txt`, else `~/.pokerbot/`), or keep stats in memory only. |
| `--reset-stats` | Delete the saved Pokerbot history file and exit. |
| `--fast` | No pauses between bot actions. |
| `-h`, `--help` / `-V`, `--version` | Usage / version. |

Each option may be given **at most once**; a repeated option (including
`--no-color` together with `--color`) is an error (exit status 2), never
"last one wins". Unknown options and bad values also exit with status 2.

## Opponents

The opponent's label is shown at the start of the session, under every
table and in `help`; there is never a silent substitution.

* `--bot cfr`: plays the trained average strategy from `--strategy`. The
  banner shows the artifact label, training iterations, trainer seed,
  checksum, abstraction fingerprint and the note *plumbing-only smoke
  policy: not competitive, not GTO / Nash*. Situations missing from the
  artifact are played uniformly at random over the legal actions; these
  decisions are counted as *explicit fallback* in `stats`. If the artifact
  is missing, unreadable, for another depth, or corrupt, `pokerbot` exits with
  status 2 and says why.
* `--bot baseline`: an **untrained**, seeded, weighted-random legal bot,
  labelled `baseline (untrained, random legal)`.
* `--bot auto` (default): `cfr` if a valid artifact for the chosen depth
  loads; otherwise the baseline, announced by a loud `!!` banner naming the
  reason (missing / wrong depth / corrupt) and how to train one
  (`scripts/train_smoke.sh`, currently 20BB only).

## Difficulty

`--difficulty` applies to the CFR policy only; with the baseline it is
accepted, does nothing, and the banner says so. Every mode samples, with a
seeded RNG, from an explicit blend of the trained mix `p` with the uniform
distribution over the **legal** actions (`n` of them), share `λ`:

```
p'(a) = (1 − λ) · p(a) + λ / n      (legal a; illegal actions stay 0)
```

| Mode | λ | Label |
|---|---|---|
| `challenging` (default) | 0 | `challenging: trained smoke policy as-is` |
| `balanced` | 1/4 | `balanced: policy blended 25% with uniform-random legal play` |
| `training` | 3/5 | `training: policy blended 60% with uniform-random legal play` |

Sampling stays integer (weights `(b − a)·q·n + a·65535` for `λ = a/b` and
quantized `q`), so a seed reproduces the same choices on every platform.
`challenging` keeps the original weights and is bit-identical to the
untransformed agent. Unseen situations still use the explicit uniform
fallback, counted in `stats`. The banner and `stats` show the difficulty,
and the replay hint includes `--difficulty`.

These modes are blends of a plumbing-grade smoke policy; relative strength
has not been benchmarked unless shown by `pokerbot-eval`. "Training" means
"more random", not "easier by a measured amount".

With `--adapt`, the order is: trained policy → difficulty blend → adaptive
shift. `stats` then reports how many decisions were adapted, so they are
not presented as pure policy. The coach (`analysis`) always shows the
**fixed** trained mix, labelled `trained policy (fixed, unadapted,
challenging)` (`fixed trained policy (not adapted, challenging)` with
`--adapt`), and says when the opponent plays a blend instead.

Bots only ever see their own observation: their own cards, the revealed
board and the public action history. Your cards stay yours, and the bot's
cards are shown face down (`[##]` / `[▓▓]`) until a hand reaches showdown;
after a fold they are never shown.

## Commands

During your turn (`Your action>`):

| Input | Effect |
|---|---|
| `f` `c` `k` `h` `p` `a` | fold, check-or-call, check, half-pot bet/raise, pot bet/raise, all-in |
| `1`..`6` | the action with that number in the Options list |
| `fold` `check` `call` `half` `pot` `allin` / `all-in` | the same actions by name |
| `help` (`?`) | command help, including the opponent modes |
| `stats` (`s`) | session score and where the bot's decisions came from |
| `quit` (`q`) | end the session; the unfinished hand is abandoned and not scored |

Between hands: `Enter` / `continue` (`c`) deals the next hand;
`analysis` (`a`, or `analysis <n>`) reviews the last or a saved hand with
the fixed policy's mix; `replay` (`r`, or `replay <n>`) steps through it;
`stats` (`s`), `help` (`?`) and `quit` (`q`) as above. With `--adapt` only,
help also lists `profile` (`adapt`), `adapt off`, `adapt on` and
`reset-profile`. End of input (Ctrl-D or the end of a piped script) ends
the session cleanly.

Only the actions listed under **Options** are legal. Anything else is
rejected with a message and the game state does not change. The help text
lists exactly the commands above; nothing is shown that does not work.

Bot decision counts in `stats` cover **completed hands only**: whether the
bot's current spot was in the policy depends on its private cards.

## Wording and amounts

Every option shows exact chip amounts. "Raise to N" and "bet N" use N as
the actor's total for the street:

* **bet** when there is nothing to call. After a small-blind limp the big
  blind faces nothing, so its half-pot option reads `bet 2 (to 4)`: 2 more
  chips, 4 in total.
* **raise to** when facing an unmatched wager. The small blind's preflop
  open faces the big blind: `raise to 4 (half pot)`.
* `call N` is the chips added; `all-in to N` is the street total after
  committing the whole stack.

## Colour, glyphs and width

Colour (`--color auto`): no colour if `NO_COLOR` is set (non-empty), stdout
is not a terminal, or `TERM=dumb`; truecolor if `COLORTERM` is `truecolor`
or `24bit`; 256 colours if `TERM` contains `256color`; otherwise the 16
basic colours. `--color` forces a depth even when piping. Every meaning is
also carried by text (brackets, words, `!!` warning bars), so no-colour
output loses nothing.

Glyphs (`--glyphs auto`): Unicode suits `♠♥♦♣` and box frames if the first
non-empty of `LC_ALL`, `LC_CTYPE`, `LANG` names UTF-8; otherwise ASCII
(`As`, `Kh`, `+=|` frames). All glyphs used are one column wide.

Width: `--width`, else `COLUMNS`, else 80, clamped to 48..240. Many shells
do not export `COLUMNS` to programs; run `COLUMNS=$COLUMNS pokerbot ...` or pass
`--width`. Below 100 columns the compact layout is used (designed for
80x24: one table screen is at most 16 lines); from 100 columns the table
gets a side pane with the action log. No line is ever wider than the
chosen width.

Pauses: in interactive play on a terminal, `pokerbot` pauses 350 ms before
each bot action and new street. `--fast`, `--auto`, and non-terminal
input or output skip all pauses.

## Replay

The header prints a command that replays the session: the same seed gives
the same decks, the same bot sampling and the same scripted player, so the
same inputs give an identical transcript (given the same artifact):

```
Replay: pokerbot --seed 7 --hands 3 --depth 20 --bot auto --strategy \
    artifacts/smoke_20bb.pokerbot --difficulty challenging --auto checkcall
```

Long commands wrap with a trailing `\`, so they paste into a shell as-is.

## Screenshot (no colour, ASCII, 80 columns)

```
-- Hand 1 of 3 | button: You ---------------------------------------------------
  Blinds: You 1 (SB) | Bot 2 (BB) | your cards [Ad][Ah]
+= Pokerbot ================================================== Hand 1/3 | Preflop =+
|   Bot  BB     [##][##]  stack   38  bet    2                                 |
|                                                                              |
|         Board [--][--][--][--][--]   Pot 3                                   |
|                                                                              |
| > You  BTN/SB [Ad][Ah]  stack   39  bet    1                                 |
+= vs cfr 20BB (500k it): smoke: trained for plumbing, not competition ========+
  Options: [f] fold   [c] call 1   [h] raise to 4 (half pot)
           [p] raise to 6 (pot)   [a] all-in to 40
  Your action> c
  > You call 1
  > Bot checks
-- Flop [7c][Jd][2c] | pot 4 ---------------------------------------------------
```

`>` marks the player to act. Golden renderings of every street, a
showdown, a fold and the session header, at 80 and 120 columns, in plain
and 256-colour modes, live in `crates/pokerbot-cli/tests/snapshots/`
(regenerate with `UPDATE_SNAPSHOTS=1 cargo test -p pokerbot-cli --test snapshots`).

## Not included

`pokerbot` has no `restart` command; everything listed in `help` works.

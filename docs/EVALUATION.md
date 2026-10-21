# Pokerbot policy evaluation (M-19)

`pokerbot-eval` (crate `crates/pokerbot-eval`) measures a trained `.pokerbot` policy by
playing it against fixed rule-bot baselines and against itself. It reports
BB/100 with 95% confidence intervals, sample sizes, per-position results and
the policy's fallback (unseen info set) rate. It then applies a promotion gate
whose criteria were fixed in config before the run.

The harness is an original implementation. The
paired/duplicate-deal design is a standard variance-reduction technique
(duplicate poker; common random numbers in simulation). The statistics are
textbook: sample mean, Bessel-corrected variance, normal-approximation CI, and
Efron's percentile bootstrap.

**What it is not.** It computes no exploitability, best response, or distance
to a Nash equilibrium, and it makes no GTO claim. Beating weak baselines is
not evidence of strength against capable opponents.

## How to run

```sh
cargo run --release -p pokerbot-eval -- --smoke                  # configs/eval_smoke.toml
cargo run --release -p pokerbot-eval -- --config <file.toml>     # any eval config
    [--artifact <path>]   # override [policy] artifact
    [--pairs <n>]         # override deal_pairs, 1..=200000 (each pair = 2 hands)
    [--json <out.json>]   # machine-readable report (deterministic, no timings)
    [--require-promote]   # exit 3 on HOLD (for CI gates)
```

Exit status: 0 means the evaluation completed, whatever the verdict. 2 means a
usage, config, artifact or match error, and 3 means HOLD under
`--require-promote`. A missing, corrupt or wrong-depth artifact is an explicit
error. The harness never substitutes a default policy.

The smoke artifact comes from `scripts/train_smoke.sh` (`configs/smoke.toml`).
`--smoke` takes about 0.6 s in release for 20,000 hands.

## Methodology

### Matchups
For each opponent id in `[eval] opponents` the candidate CFR agent is agent A:

| id | opponent |
|---|---|
| `random` | uniform over the legal action classes (rule bot) |
| `checkcall` | checks if possible, else calls; never folds or raises (rule bot) |
| `tight` | fixed hand-strength rule (rule bot; see `baselines.rs`) |
| `mirror` | a second, independently constructed agent from the same artifact |

There is also a sanity matchup, random-legal vs check-call. Every matchup uses
fresh agents and the same deal stream (common random numbers across
opponents).

### Paired seeds and seat swap
Deal `i` uses deck seed `deal_seed(base_seed, i)`, a SplitMix64 derivation.
Each deal is played twice: A on the button, then B on the button. The engine
deals cards by position: BB gets deck cards 0 and 2, the button 1 and 3, and
the board is cards 4 to 8. So both games of a pair give each position
identical hole cards and board, and card luck cancels within the pair. Stacks
reset to the configured depth every hand.

### Decision sampling (why the mirror is exactly 0)
Agents report a distribution over the six action classes. The harness checks
that the agent's own sampled action is legal, then re-samples from the
reported distribution, restricted to legal classes, using a harness-owned RNG
stream seeded by `(deal seed, position)`. In a mirror match the button's
draws in game 1 and game 2 come from the same stream, so the two games are
the same hand with the players swapped, and every pair sums to exactly zero.
For non-mirror matchups this also gives both agents identical randomness per
position. A mirror result different from 0 would indicate a harness or
determinism bug.

### Information boundary
Agents only ever receive `HandState::observation(seat)` for their own seat.
Before each decision the runner checks that the observation belongs to the
acting seat and holds no opponent hole cards. The referee reads full state
only to settle the hand and for test logs. `tests/matchup.rs` records every
observation an agent sees and verifies this.

### Statistics
For a matchup with `n` deal pairs, let `x_i` be A's net chips over both games
of pair `i`, and let `BB` be the big blind in chips.

* mean `m = Σx_i / n`; sample variance `s² = Σ(x_i − m)² / (n − 1)`
* standard error `se = s / √n`
* 95% CI on `m`: `m ± 1.959964·se` (normal approximation; needs `n ≥ 2`)
* BB/100 = chips per hand / BB × 100, so pair values scale by `100 / (2·BB)`.
  The CI scales the same way.
* Percentile bootstrap cross-check: `B` resamples of the `n` pair values,
  with a seed set in config. The 2.5% and 97.5% nearest-rank quantiles of
  the resampled means form the interval.
* Per position: `y_i` is A's net in the game where A held the button, and
  `z_i` the net where A held the BB. Each gets its own mean and CI with
  scale `100 / BB` (`n` hands each). Preflop the button acts first, and
  postflop the BB acts first. Per-position numbers do **not** cancel card
  luck the way paired totals do, so their CIs are wider.
* Fallback rate = fallback decisions / decisions for the candidate,
  counted by the harness from `DecisionSource`. It is cross-checked against
  the agent's own `InferStats` (`tests/cli.rs`).

### Train/test separation
Training (`pokerbot-train`) samples deals from its own trainer RNG stream, seeded
with the training seed. Evaluation deals come from `Deck::from_seed` over a
separately seeded SplitMix64 stream. The harness refuses an eval `base_seed`
equal to the artifact's training seed or abstraction seed. This keeps the
seed streams disjoint. It does **not** make overlapping deals impossible: the
deal space is finite, and an evaluation deal may coincide with one seen in
training by chance. No training samples are reused deliberately.

### Promotion gate
These criteria are locked in `[gate]` before running. Promotion requires all
of them:

1. Every required baseline (smoke: `random`, `checkcall`) was played with at
   least `min_hands` hands.
2. The BB/100 95% CI lower bound is > 0 against each required baseline.
3. The candidate fallback rate is ≤ `max_fallback_rate_bp`, and there was at
   least one decision.
4. The artifact checksum (FNV-1a 64, embedded in the artifact) matches
   `[policy] expected_checksum` if `require_pinned_checksum` is set.
   A retrained or different artifact therefore HOLDs.

The mirror cannot be a gate requirement. An empty requirement list never
promotes. PROMOTE means only that the artifact cleared this bar against
these baselines.

## Smoke results (locked `configs/eval_smoke.toml`)

These numbers are from the artifact with checksum `0xd2767c7dad569fdd`
(500k iterations, `smoke: trained for plumbing, not competition`). The run
used eval seed 4157113009 and 2,000 pairs (4,000 hands) per matchup.

| matchup (A vs B) | BB/100 (95% CI) | A on button | A on BB | fallback |
|---|---|---|---|---|
| CFR vs random-legal | +89.83 [+65.14, +114.51] | +87.58 ± 40.84 | +92.07 ± 39.88 | 0.00% |
| CFR vs check-call | +146.84 [+124.43, +169.24] | +105.42 ± 46.52 | +188.25 ± 47.43 | 0.00% |
| CFR vs tight | +25.09 [+9.88, +40.29] | +12.47 ± 27.06 | +37.70 ± 24.32 | 0.00% |
| CFR vs CFR (mirror) | 0.00 (exact) | −18.07 ± 32.84 | +18.07 ± 32.84 | 0.00% |
| random vs check-call (sanity) | −3.86 [−25.86, +18.14] | −74.88 ± 69.35 | +67.15 ± 76.61 | n/a |

The gate verdict was PROMOTE: both required CIs are above 0, the fallback
rate was 0% over 41,567 decisions, and the checksum matched. Bootstrap CIs
agree with the normal CIs to within about 0.3 BB/100. These are results
against weak rule bots only.

## Limitations

* Confidence intervals are statistical statements, not guarantees of
  future performance. They assume i.i.d. pairs and a normal sampling
  distribution of the mean.
* Beating weak baselines does not imply strength. The baselines are simple
  rule bots, and passive check/call in particular is easy to beat.
* No exploitability or best-response value is computed, so nothing here
  bounds how far the policy is from equilibrium, even in the abstract game.
* Results hold for the six-action abstraction and the configured depth only.
* The mirror test checks harness symmetry and determinism. It says nothing
  about strength.
* A 0% fallback rate means every info set the harness reached was in the
  artifact. Opponents that steer into rarer lines could see a higher rate.

## Fixed vs adaptive comparison (M-20 comparator)

`configs/eval_adaptive.toml` (`mode = "comparison"`) runs the fixed CFR
policy (`cfr`) and the opt-in adaptive wrapper (`adaptive_cfr`, see
`crates/pokerbot-analysis/src/opponent.rs`) against the same opponents on the
**same deal stream**. **Adaptive mode is experimental; this comparison does
not establish general strength.** There is no `[gate]` and no verdict, and
the config parser rejects a `[gate]` section in comparison mode.

```sh
cargo run --release -p pokerbot-eval -- --config configs/eval_adaptive.toml [--json out.json]
```

### Design
* **Observation.** The adaptive agent decides from its own
  `PlayerObservation`. After **every** hand the harness calls
  `observe_hand` with that hand's
  `events_for(Viewer::Seat(own seat))`. The runner checks that the stream
  holds no opponent `HoleCards` event, and `OpponentProfile` itself refuses
  one.
* **Profile scope.** One profile models the opponent across the whole match;
  it is never reset. Events are delivered at the **pair boundary**: both
  seat-swapped games of a deal are played with the same profile state, and
  then both hands are fed in play order. The second game of a deal therefore
  never uses what was seen in the first game of that deal, so the pair stays
  a like-for-like comparison. A side effect is that an adaptive-vs-adaptive
  mirror remains exactly antisymmetric (tested).
* **Sampling.** `AdaptiveAgent::act` reports the *shifted* distribution in
  `Decision.distribution`, and the harness re-samples from it with its own
  keyed stream (see "Decision sampling"). Adaptation is therefore applied,
  not bypassed; `tests/adaptive.rs` checks this on every adapted decision.
* **Paired difference.** For each opponent, `d_i` = fixed result − adaptive
  result on deal pair `i` (chips over both games). The mean and 95% CI of
  `d_i` are scaled to BB/100 exactly as in "Statistics". Positive means the
  fixed policy did better on those deals. Both runs share deals and
  harness draw streams, so the CI on the difference is much tighter than
  either matchup's own CI.
* With `enabled = false` the adaptive agent gives results identical to fixed
  CFR on every deal (tested): every difference is exactly 0.

### Results

Every run below uses artifact `0xd2767c7dad569fdd`, 20,000 pairs = 40,000
hands per matchup, and a release build. Between configs only the eval
`base_seed` differs, and between rule versions only
`crates/pokerbot-analysis/src/opponent.rs` changed. Run A and run C use
`configs/eval_adaptive.toml` (seed 2718281829). Run B, the primary result
for the revised rule, uses `configs/eval_adaptive_v2.toml` (seed
3141592653). That config is identical except for `base_seed` and was
pre-registered in `docs/PROGRESS.md` (commit 2e35cde) before it was run.

#### Run A (2026-11-15): original rule, seed 2718281829

`opponent.rs` v1, applied only when nothing is owed:
* *Over-folder:* move Check to the smallest bet.
* *Calling station:* move the largest bets (AllIn, Pot) to Check,
  **regardless of hand strength**.

| opponent | fixed BB/100 [95% CI] | adaptive BB/100 [95% CI] | fixed − adaptive [95% CI] | decisions shifted |
|---|---|---|---|---|
| random-legal | +88.91 [+81.14, +96.68] | +88.91 [+81.14, +96.68] | 0.00 (exact) | 0 of 70,279 |
| check-call | +132.94 [+125.85, +140.03] | +110.96 [+104.40, +117.51] | **+21.98** [+18.71, +25.25] | 114,598 of 135,297 (84.7%) |
| tight | +23.23 [+18.63, +27.83] | +23.52 [+18.98, +28.06] | −0.29 [−0.90, +0.32] | 11,834 of 55,463 (21.3%) |
| fixed CFR (head-to-head) | n/a | 0.00 (exact) | n/a | 3 of 78,540 |

Run A against check-call: the adaptive agent did **worse** than fixed by
about 22 BB/100, and the CI excludes 0. The calling-station rule removed
value bets as well as bluffs.

#### Run B (2026-11-15): revised rule, fresh pre-registered seed 3141592653 (primary)

`opponent.rs` v2. Here `e` is the agent's own equity vs a random hand, from
its own cards and the visible board only.
* *Calling station:* if `e ≥ 0.65`, move Check to the largest
  non-all-in bet (value-bet more). If `e ≤ 0.40`, move bets to Check (bluff
  less). Otherwise no change.
* *Over-folder:* if `e < 0.65`, move Check to the smallest bet. Strong hands
  are unchanged.

The thresholds (VALUE_EQUITY 0.65, BLUFF_EQUITY 0.40, statistic thresholds
0.60) were frozen before this run.

| opponent | fixed BB/100 [95% CI] | adaptive BB/100 [95% CI] | fixed − adaptive [95% CI] | decisions shifted |
|---|---|---|---|---|
| random-legal | +92.26 [+84.53, +100.00] | +92.26 [+84.53, +100.00] | 0.00 (exact) | 0 of 71,162 |
| check-call | +133.00 [+125.94, +140.07] | +170.71 [+163.74, +177.69] | **−37.71** [−40.00, −35.42] | 68,214 of 133,787 (51.0%) |
| tight | +22.08 [+17.56, +26.60] | +24.30 [+19.81, +28.80] | **−2.22** [−2.75, −1.69] | 8,250 of 55,840 (14.8%) |
| fixed CFR (head-to-head) | n/a | 0.00 (exact) | n/a | 0 of 79,753 |

Run B took 14.6 s. On this pre-registered fresh seed the revised adaptive
agent did better than fixed against check-call (by about 38 BB/100) and
against tight (by about 2.2 BB/100), and both CIs exclude 0. Random-legal
triggers no rule, and against fixed CFR no rule fired at all. All mirrors
are exactly 0, and the fallback rate is 0.00% of 420,226 fixed-candidate
decisions.

*Provenance and reproduction.* The eval seed was pre-registered in
`docs/PROGRESS.md` (commit 2e35cde) before run B. Run B itself was made
from the uncommitted config, before that config was committed in 9b47efa.
After the commit, the committed `configs/eval_adaptive_v2.toml` was run
once more (2026-11-15, release, 14.2 s). Its JSON report was
**byte-identical** to run B's. That second run reproduces run B; it is not a
second independent sample.

#### Run C (2026-11-15): revised rule, seed 2718281829 (post-hoc)

**Post-hoc, same seed that motivated the revision; not an independent
test.** Same rule as run B, on run A's config.

| opponent | fixed BB/100 [95% CI] | adaptive BB/100 [95% CI] | fixed − adaptive [95% CI] | decisions shifted |
|---|---|---|---|---|
| random-legal | +88.91 [+81.14, +96.68] | +88.91 [+81.14, +96.68] | 0.00 (exact) | 0 of 70,279 |
| check-call | +132.94 [+125.85, +140.03] | +169.80 [+162.81, +176.80] | **−36.86** [−39.14, −34.59] | 67,452 of 132,944 (50.7%) |
| tight | +23.23 [+18.63, +27.83] | +24.99 [+20.42, +29.56] | **−1.76** [−2.29, −1.22] | 8,070 of 55,416 (14.6%) |
| fixed CFR (head-to-head) | n/a | 0.00 (exact) | n/a | 3 of 78,540 |

Run C agrees with run B to within about 1 BB/100 on every difference.

#### Caveats

* Run B is the reported result for the revised rule. It is independent of
  the seed that prompted the revision, but the opponents are the same three
  rule bots, so it confirms the effect against *these* bots only.
* Against fixed CFR the rules never or almost never fire (0 shifts in run B,
  3 in runs A and C), so nothing here shows that adaptation helps against a
  capable or balanced opponent.
* Check-call and tight are fixed, non-adapting rule bots. Exploiting them is
  easy and says little about general strength.
* These results cover one smoke artifact and the default adaptation settings
  only.

### JSON report fields

`--json` writes one object (`"format": "pokerbot-eval-report v1"`), with no
timings. The keys added for the comparator are:

* `mode`: `"promotion"` or `"comparison"`.
* `gate`: `{verdict, checks[]}` in promotion mode; **`null`** in comparison
  mode.
* `comparison_note`: the experimental-mode disclaimer string in comparison
  mode, otherwise `null`.
* `comparisons[]`: one entry per opponent played by both `cfr` and
  `adaptive_cfr`, with `opponent`, `pairs`, `fixed_minus_adaptive_bb100`
  (`{n, mean, sd, se, ci95_low, ci95_high}`; positive means fixed did
  better) and `bootstrap_ci95` (`[low, high]` or `null`).
* In each `matchups[]` / `sanity[]` entry:
  * `candidate` is `"cfr"` or `"adaptive_cfr"`.
  * `a_adapt_stats` / `b_adapt_stats` are `{decisions, adapted, mass_moved}`
    for adaptive agents, otherwise `null`.
  * `a_hands_observed` / `b_hands_observed` count the finished hands
    delivered to that side's opponent model.

Non-finite statistics (for example a CI with `n < 2`) are written as
`null`.

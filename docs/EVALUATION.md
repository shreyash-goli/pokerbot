# Mako policy evaluation (M-19)

`mako-eval` (crate `crates/mako-eval`) measures a trained `.mako` policy by
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
cargo run --release -p mako-eval -- --smoke                  # configs/eval_smoke.toml
cargo run --release -p mako-eval -- --config <file.toml>     # any eval config
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
Training (`mako-train`) samples deals from its own trainer RNG stream, seeded
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

These numbers are from the artifact with checksum `0xe673b32895f96760`
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

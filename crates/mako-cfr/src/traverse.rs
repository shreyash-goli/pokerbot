//! External-sampling MCCFR over the abstract heads-up NLHE game (M-10).
//!
//! Algorithm: Lanctot, Waugh, Zinkevich, Bowling, "Monte Carlo Sampling for
//! Regret Minimization in Extensive Games", NIPS 2009, §3 (external
//! sampling), as validated on Kuhn poker in [`crate::toy::cfr`].
//!
//! Each iteration runs one traversal per seat. For each traversal:
//! 1. **Chance** is sampled once: a deck is shuffled from the trainer's
//!    seeded [`MakoRng`], which fixes both hands and the whole board.
//! 2. **Traverser** nodes enumerate every legal action (on `HandState`
//!    clones) and add `u(a) − u(node)` to the cumulative regrets.
//! 3. **Opponent** nodes add their current regret-matching strategy to the
//!    average strategy ("simple" averaging), then sample one action from it.
//! 4. **Terminal** utility is the traverser's chip payoff divided by the big
//!    blind (utilities in big blinds; this scaling does not change the
//!    regret-matching strategy).
//!
//! # Information boundary
//! A node's key is built for the acting seat only, from: its own hole cards,
//! the revealed board, its position and the public action history. The fast
//! path computes each seat's bucket at most once per street per deal
//! (memoized from `state.hole_cards(seat)` and `state.board()`, the revealed
//! cards) and grows the history bytes incrementally. With
//! [`TrainerConfig::verify_keys`] on, every node also builds
//! `InfoKey::from_observation(&state.observation(seat), ..)` and asserts byte
//! equality. Terminal payoffs naturally compare both hands; that is the
//! sampled chance outcome, not information available to a decision.
//!
//! The table is trained for one `RulesConfig` (blinds + equal stacks), which
//! M-12 records in artifact metadata. Average strategies approximate an
//! equilibrium of the *abstract* game only; no claim of GTO play is made.

use mako_core::action::Seat;
use mako_core::deck::Deck;
use mako_core::rng::MakoRng;
use mako_core::rules::{ActionKind, Position, RulesConfig, StackDepth, Street};
use mako_core::state::{HandConfig, HandState};

use crate::abstraction::history::{HistoryKey, STREET_SEPARATOR};
use crate::abstraction::key::{CardAbstraction, InfoKey, KEY_FORMAT_VERSION};
use crate::abstraction::postflop::{AbstractionConfig, ConfigError, EquityAbstraction};
use crate::infoset::StrategyTable;
use crate::regret::{
    NUM_SLOTS, UpdateConfig, Weighting, accumulate_strategy, is_legal, mask_of, update_regrets,
};

/// Trainer configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrainerConfig {
    /// Game rules (blinds and equal starting stacks).
    pub rules: RulesConfig,
    /// Card abstraction.
    pub abstraction: AbstractionConfig,
    /// Seed for the trainer RNG (deals and opponent samples).
    pub seed: u64,
    /// Equity cache capacity (entries); 0 disables the cache.
    pub cache_capacity: usize,
    /// Update-rule flags.
    pub update: UpdateConfig,
    /// Iteration weighting (M-11); default is unweighted.
    pub weighting: Weighting,
    /// Cross-check every fast-path key against `InfoKey::from_observation`
    /// (slow; for tests).
    pub verify_keys: bool,
}

impl TrainerConfig {
    /// Default abstraction at `depth`, with a 2^18-entry equity cache.
    pub fn standard(depth: StackDepth, seed: u64) -> Self {
        Self {
            rules: RulesConfig::standard(depth),
            abstraction: AbstractionConfig::default(),
            seed,
            cache_capacity: 1 << 18,
            update: UpdateConfig::default(),
            weighting: Weighting::default(),
            verify_keys: false,
        }
    }

    /// Tiny test configuration: 20 BB, 3/3/3 buckets, 64 flop and 64 turn
    /// samples.
    pub fn tiny(seed: u64) -> Self {
        Self {
            rules: RulesConfig::standard(StackDepth::Bb20),
            abstraction: AbstractionConfig {
                flop_buckets: 3,
                turn_buckets: 3,
                river_buckets: 3,
                flop_samples: 64,
                turn_samples: 64,
                ..AbstractionConfig::default()
            },
            seed,
            cache_capacity: 1 << 16,
            update: UpdateConfig::default(),
            weighting: Weighting::default(),
            verify_keys: false,
        }
    }
}

/// Counters for one trainer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TrainerStats {
    /// Traversals run (two per iteration).
    pub traversals: u64,
    /// Decision nodes visited.
    pub nodes: u64,
    /// Traverser decision nodes (regret updates).
    pub traverser_nodes: u64,
    /// Opponent actions sampled (all legal by construction).
    pub opponent_samples: u64,
    /// Terminal states reached.
    pub terminals: u64,
    /// Fast-path keys cross-checked against `from_observation`.
    pub keys_verified: u64,
}

/// Per-deal memo of each seat's bucket on each street.
#[derive(Default)]
struct DealCtx {
    buckets: [[Option<u16>; 4]; 2],
}

fn street_index(s: Street) -> usize {
    match s {
        Street::Preflop => 0,
        Street::Flop => 1,
        Street::Turn => 2,
        Street::River => 3,
    }
}

/// Key bytes in the [`InfoKey`] v1 layout from its parts.
pub fn key_bytes(street: Street, position: Position, bucket: u16, history: &[u8]) -> Vec<u8> {
    let mut k = Vec::with_capacity(6 + history.len());
    k.push(KEY_FORMAT_VERSION);
    k.push(street_index(street) as u8);
    k.push(match position {
        Position::Button => 0,
        Position::BigBlind => 1,
    });
    k.extend_from_slice(&bucket.to_le_bytes());
    k.push(history.len() as u8);
    k.extend_from_slice(history);
    k
}

fn position_of(state: &HandState, seat: Seat) -> Position {
    if seat == state.button() {
        Position::Button
    } else {
        Position::BigBlind
    }
}

/// External-sampling MCCFR trainer.
#[derive(Debug)]
pub struct Trainer {
    config: TrainerConfig,
    abstraction: EquityAbstraction,
    table: StrategyTable,
    rng: MakoRng,
    iteration: u64,
    stats: TrainerStats,
}

impl Trainer {
    /// New trainer with an empty table.
    pub fn new(config: TrainerConfig) -> Result<Self, ConfigError> {
        config
            .rules
            .validate()
            .map_err(|_| ConfigError("invalid rules config"))?;
        let abstraction = if config.cache_capacity > 0 {
            EquityAbstraction::with_cache(config.abstraction, config.cache_capacity)?
        } else {
            EquityAbstraction::new(config.abstraction)?
        };
        Ok(Self {
            config,
            abstraction,
            table: StrategyTable::new(),
            rng: MakoRng::seed_from_u64(config.seed),
            iteration: 0,
            stats: TrainerStats::default(),
        })
    }

    /// Configuration.
    pub fn config(&self) -> &TrainerConfig {
        &self.config
    }

    /// The card abstraction.
    pub fn abstraction(&self) -> &EquityAbstraction {
        &self.abstraction
    }

    /// The strategy table.
    pub fn table(&self) -> &StrategyTable {
        &self.table
    }

    /// Completed iterations.
    pub fn iteration(&self) -> u64 {
        self.iteration
    }

    /// Counters.
    pub fn stats(&self) -> TrainerStats {
        self.stats
    }

    /// Weight for average-strategy accumulation this iteration
    /// (see [`Weighting`]).
    pub fn strategy_weight(&self) -> f64 {
        self.config.weighting.strategy_weight(self.iteration)
    }

    /// Weight for regret accumulation this iteration (see [`Weighting`]).
    pub fn regret_weight(&self) -> f64 {
        self.config.weighting.regret_weight(self.iteration)
    }

    /// Current RNG state (for checkpoints).
    pub fn rng_state(&self) -> [u64; 4] {
        self.rng.state()
    }

    /// Rebuild a trainer from checkpointed parts (M-11). The equity cache
    /// starts empty, which does not affect results.
    pub fn restore(
        config: TrainerConfig,
        table: StrategyTable,
        rng_state: [u64; 4],
        iteration: u64,
        stats: TrainerStats,
    ) -> Result<Self, ConfigError> {
        let mut t = Self::new(config)?;
        t.table = table;
        t.rng = MakoRng::from_state(rng_state);
        t.iteration = iteration;
        t.stats = stats;
        Ok(t)
    }

    fn hand_config(&self) -> HandConfig {
        HandConfig {
            rules: self.config.rules,
            button: Seat::Zero,
        }
    }

    /// Run exactly `n` iterations (one traversal per seat each).
    pub fn run_iterations(&mut self, n: u64) {
        for _ in 0..n {
            self.iteration += 1;
            for traverser in Seat::ALL {
                let deck = Deck::shuffled(&mut self.rng);
                self.traverse_deck(deck, traverser);
            }
        }
    }

    /// One traversal on a given deck (chance outcome) for `traverser`.
    /// Returns the sampled traverser utility in big blinds.
    pub fn traverse_deck(&mut self, deck: Deck, traverser: Seat) -> f64 {
        let state = HandState::new(self.hand_config(), deck).expect("valid config and deck");
        self.stats.traversals += 1;
        let mut hist = Vec::with_capacity(48);
        let mut ctx = DealCtx::default();
        self.traverse(&state, traverser, &mut hist, &mut ctx)
    }

    fn bucket_for(&self, ctx: &mut DealCtx, state: &HandState, seat: Seat, street: Street) -> u16 {
        let slot = &mut ctx.buckets[seat.index()][street_index(street)];
        if let Some(b) = *slot {
            return b;
        }
        // Own hole cards + revealed board only.
        let b = self
            .abstraction
            .bucket(street, state.hole_cards(seat), state.board())
            .expect("abstraction supports every street");
        *slot = Some(b);
        b
    }

    /// Non-incremental fast-path key for the seat to act: own bucket, own
    /// position, public history. Exposed for the hidden-information tests.
    pub fn node_key_bytes(&self, state: &HandState) -> Option<Vec<u8>> {
        let seat = state.to_act()?;
        let street = state.street().betting()?;
        let bucket = self
            .abstraction
            .bucket(street, state.hole_cards(seat), state.board())
            .ok()?;
        let hist = HistoryKey::from_records(state.history(), street).ok()?;
        Some(key_bytes(
            street,
            position_of(state, seat),
            bucket,
            hist.as_bytes(),
        ))
    }

    fn traverse(
        &mut self,
        state: &HandState,
        traverser: Seat,
        hist: &mut Vec<u8>,
        ctx: &mut DealCtx,
    ) -> f64 {
        if state.is_terminal() {
            self.stats.terminals += 1;
            let p = state.payoff(traverser).expect("terminal payoff");
            return p as f64 / f64::from(self.config.rules.big_blind);
        }
        self.stats.nodes += 1;
        let seat = state.to_act().expect("non-terminal has an actor");
        let street = state.street().betting().expect("betting street");
        let bucket = self.bucket_for(ctx, state, seat, street);
        let key = key_bytes(street, position_of(state, seat), bucket, hist);
        if self.config.verify_keys {
            let obs_key = InfoKey::from_observation(&state.observation(seat), &self.abstraction)
                .expect("key from observation")
                .to_bytes();
            assert_eq!(obs_key, key, "fast-path key differs from observation key");
            self.stats.keys_verified += 1;
        }
        let legal = state.legal_actions();
        let mask = mask_of(legal.iter().map(|a| a.kind));
        let idx = self.table.index_or_insert(&key, mask);
        let sigma = self.table.entry(idx).current_strategy();

        if seat == traverser {
            self.stats.traverser_nodes += 1;
            let mut utils = [0.0; NUM_SLOTS];
            for a in &legal {
                utils[a.kind.index()] = self.child_value(state, a.kind, traverser, hist, ctx);
            }
            let cfg = self.config.update;
            let w = self.regret_weight();
            let e = self.table.entry_mut(idx);
            update_regrets(&mut e.regret, mask, &sigma, &utils, w, cfg)
        } else {
            let w = self.strategy_weight();
            let e = self.table.entry_mut(idx);
            accumulate_strategy(&mut e.strategy_sum, mask, &sigma, w);
            let kind = self.sample_action(&sigma, mask);
            self.stats.opponent_samples += 1;
            self.child_value(state, kind, traverser, hist, ctx)
        }
    }

    fn sample_action(&mut self, sigma: &[f64; NUM_SLOTS], mask: u8) -> ActionKind {
        let u = self.rng.gen_f64();
        let mut acc = 0.0;
        let mut last = None;
        for (i, &p) in sigma.iter().enumerate() {
            if is_legal(mask, i) {
                last = Some(i);
                acc += p;
                if u < acc {
                    return ActionKind::ALL[i];
                }
            }
        }
        ActionKind::ALL[last.expect("at least one legal action")]
    }

    fn child_value(
        &mut self,
        state: &HandState,
        kind: ActionKind,
        traverser: Seat,
        hist: &mut Vec<u8>,
        ctx: &mut DealCtx,
    ) -> f64 {
        let before = street_index(state.street().betting().expect("betting street"));
        let mut child = state.clone();
        child
            .apply(kind)
            .expect("only legal actions are enumerated or sampled");
        let saved = hist.len();
        hist.push(kind.index() as u8);
        if let Some(s) = child.street().betting() {
            for _ in before..street_index(s) {
                hist.push(STREET_SEPARATOR);
            }
        }
        let v = self.traverse(&child, traverser, hist, ctx);
        hist.truncate(saved);
        v
    }
}

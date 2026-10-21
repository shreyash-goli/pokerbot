//! Postflop equity buckets (M-08).
//!
//! A postflop bucket is `min(floor(equity · n), n − 1)`, where `equity` is
//! own-hand equity against a uniformly random opponent hand
//! ([`crate::equity`]) and `n` is the per-street bucket count from
//! [`AbstractionConfig`]. Preflop uses the 169 [`PreflopClass`]es.
//!
//! # Determinism
//! The bucket is a pure function of `(canonical hole + board, config)`:
//! - river is exact enumeration;
//! - turn is exact by default (`turn_samples = 0`);
//! - flop uses Monte Carlo with an RNG seeded by
//!   `fnv1a64(config.seed LE ‖ canonical_key LE)` (see [`situation_seed`]).
//!   It does not draw from a shared RNG stream, so the same situation gets the
//!   same bucket in any order, in any process, natively or in wasm.
//!   Sampling runs on the canonical cards, so suit-isomorphic situations get
//!   identical equity.
//!
//! # Limitations
//! - Equity-only bucketing ignores hand potential: a flush draw and a weak
//!   made hand with equal equity share a bucket. It is a deliberately coarse
//!   approximation for smoke training, not GTO.
//! - Equity is against a uniformly random hand, not a betting-conditioned
//!   range.
//!
//! # Cache
//! [`EquityCache`] is an optional bounded map from canonical key to
//! [`Equity`], with clear-on-full eviction. It never changes results; it only
//! skips recomputation. Its state sits behind a `std::sync::Mutex`, so an
//! [`EquityAbstraction`] is `Send + Sync`. That works for multi-threaded
//! training and for single-threaded wasm. Under heavy contention, use one
//! abstraction per thread.

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;

use pokerbot_core::card::{Card, CardSet};
use pokerbot_core::rng::PokerbotRng;
use pokerbot_core::rules::Street;

use super::key::{AbstractionError, CardAbstraction, board_len, fnv1a64};
use super::preflop::{NUM_PREFLOP_CLASSES, PreflopClass};
use crate::equity::{Equity, canonical_form, exact_equity, sampled_equity};

/// Version of the postflop abstraction semantics and byte format.
pub const POSTFLOP_ABSTRACTION_VERSION: u16 = 1;

const CONFIG_MAGIC: &[u8; 4] = b"MKEQ";
/// Serialized [`AbstractionConfig`] length in bytes.
pub const CONFIG_BYTES: usize = 28;

/// Card-abstraction parameters. Artifacts record [`Self::to_bytes`] /
/// [`Self::fingerprint`] and must reject a mismatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AbstractionConfig {
    /// Semantics version ([`POSTFLOP_ABSTRACTION_VERSION`]).
    pub version: u16,
    /// Equity buckets on the flop.
    pub flop_buckets: u16,
    /// Equity buckets on the turn.
    pub turn_buckets: u16,
    /// Equity buckets on the river.
    pub river_buckets: u16,
    /// Monte Carlo trials per flop situation.
    pub flop_samples: u32,
    /// Monte Carlo trials per turn situation; `0` = exact enumeration.
    pub turn_samples: u32,
    /// Seed mixed into every per-situation sampling seed.
    pub seed: u64,
}

impl Default for AbstractionConfig {
    /// Smoke-training defaults: 8/8/8 buckets, 1000 flop samples, exact turn.
    fn default() -> Self {
        Self {
            version: POSTFLOP_ABSTRACTION_VERSION,
            flop_buckets: 8,
            turn_buckets: 8,
            river_buckets: 8,
            flop_samples: 1_000,
            turn_samples: 0,
            seed: 0x4D41_4B4F,
        }
    }
}

/// Invalid [`AbstractionConfig`] or serialized bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigError(pub &'static str);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid abstraction config: {}", self.0)
    }
}

impl std::error::Error for ConfigError {}

impl AbstractionConfig {
    /// Checks version, positive bucket counts and positive flop samples.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.version != POSTFLOP_ABSTRACTION_VERSION {
            return Err(ConfigError("unsupported version"));
        }
        if self.flop_buckets == 0 || self.turn_buckets == 0 || self.river_buckets == 0 {
            return Err(ConfigError("bucket counts must be at least 1"));
        }
        if self.flop_samples == 0 {
            return Err(ConfigError("flop_samples must be at least 1"));
        }
        Ok(())
    }

    /// Bucket count for a postflop street (`None` for preflop).
    pub fn buckets(&self, street: Street) -> Option<u16> {
        match street {
            Street::Preflop => None,
            Street::Flop => Some(self.flop_buckets),
            Street::Turn => Some(self.turn_buckets),
            Street::River => Some(self.river_buckets),
        }
    }

    /// Stable 28-byte little-endian encoding:
    /// `"MKEQ" | version u16 | flop/turn/river buckets u16×3 |
    /// flop_samples u32 | turn_samples u32 | seed u64`.
    pub fn to_bytes(&self) -> [u8; CONFIG_BYTES] {
        let mut b = [0u8; CONFIG_BYTES];
        b[0..4].copy_from_slice(CONFIG_MAGIC);
        b[4..6].copy_from_slice(&self.version.to_le_bytes());
        b[6..8].copy_from_slice(&self.flop_buckets.to_le_bytes());
        b[8..10].copy_from_slice(&self.turn_buckets.to_le_bytes());
        b[10..12].copy_from_slice(&self.river_buckets.to_le_bytes());
        b[12..16].copy_from_slice(&self.flop_samples.to_le_bytes());
        b[16..20].copy_from_slice(&self.turn_samples.to_le_bytes());
        b[20..28].copy_from_slice(&self.seed.to_le_bytes());
        b
    }

    /// Decode and validate [`Self::to_bytes`] output.
    pub fn from_bytes(b: &[u8]) -> Result<Self, ConfigError> {
        if b.len() != CONFIG_BYTES || &b[0..4] != CONFIG_MAGIC {
            return Err(ConfigError("bad length or magic"));
        }
        let u16_at = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
        let u32_at = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
        let mut seed = [0u8; 8];
        seed.copy_from_slice(&b[20..28]);
        let c = Self {
            version: u16_at(4),
            flop_buckets: u16_at(6),
            turn_buckets: u16_at(8),
            river_buckets: u16_at(10),
            flop_samples: u32_at(12),
            turn_samples: u32_at(16),
            seed: u64::from_le_bytes(seed),
        };
        c.validate()?;
        Ok(c)
    }

    /// FNV-1a 64 of [`Self::to_bytes`].
    pub fn fingerprint(&self) -> u64 {
        fnv1a64(&self.to_bytes())
    }
}

/// Per-situation sampling seed: `fnv1a64(seed LE ‖ canonical_key LE)`.
pub fn situation_seed(config_seed: u64, canonical_key: u64) -> u64 {
    let mut b = [0u8; 16];
    b[..8].copy_from_slice(&config_seed.to_le_bytes());
    b[8..].copy_from_slice(&canonical_key.to_le_bytes());
    fnv1a64(&b)
}

/// Postflop equity under `config`. A pure function: it depends only on
/// `config` and the canonical form of `(hole, board)`.
///
/// # Panics
/// If `board.len()` is not 3, 4 or 5, or cards repeat.
pub fn postflop_equity(config: &AbstractionConfig, hole: [Card; 2], board: &[Card]) -> Equity {
    let canon = canonical_form(hole, board);
    equity_of_canonical(config, &canon.hole(), &canon.board(), canon.key())
}

fn equity_of_canonical(
    config: &AbstractionConfig,
    hole: &[Card; 2],
    board: &[Card],
    key: u64,
) -> Equity {
    let rng = || PokerbotRng::seed_from_u64(situation_seed(config.seed, key));
    match board.len() {
        5 => exact_equity(*hole, board),
        4 if config.turn_samples == 0 => exact_equity(*hole, board),
        4 => sampled_equity(*hole, board, config.turn_samples, &mut rng()),
        3 => sampled_equity(*hole, board, config.flop_samples, &mut rng()),
        n => panic!("postflop board must have 3-5 cards, got {n}"),
    }
}

/// Snapshot of cache counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CacheStats {
    /// Lookups served from the cache.
    pub hits: u64,
    /// Lookups that computed equity.
    pub misses: u64,
    /// Times the cache was cleared because it was full.
    pub clears: u64,
    /// Current number of entries.
    pub len: usize,
    /// Maximum number of entries.
    pub capacity: usize,
}

#[derive(Debug)]
struct CacheInner {
    map: HashMap<u64, Equity>,
    stats: CacheStats,
}

/// Bounded canonical-key → equity cache with clear-on-full eviction.
#[derive(Debug)]
pub struct EquityCache {
    inner: Mutex<CacheInner>,
}

impl EquityCache {
    /// New cache holding at most `capacity` entries (`>= 1`).
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            inner: Mutex::new(CacheInner {
                map: HashMap::with_capacity(capacity.min(1 << 16)),
                stats: CacheStats {
                    capacity,
                    ..CacheStats::default()
                },
            }),
        }
    }

    /// Return the cached value for `key`, or compute, insert and return it.
    /// The lock is not held while computing.
    pub fn get_or_compute(&self, key: u64, compute: impl FnOnce() -> Equity) -> Equity {
        {
            let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(&e) = g.map.get(&key) {
                g.stats.hits += 1;
                return e;
            }
            g.stats.misses += 1;
        }
        let e = compute();
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if !g.map.contains_key(&key) {
            if g.map.len() >= g.stats.capacity {
                g.map.clear();
                g.stats.clears += 1;
            }
            g.map.insert(key, e);
        }
        e
    }

    /// Current counters.
    pub fn stats(&self) -> CacheStats {
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        CacheStats {
            len: g.map.len(),
            ..g.stats
        }
    }
}

/// Full card abstraction: 169 preflop classes plus postflop equity buckets.
#[derive(Debug)]
pub struct EquityAbstraction {
    config: AbstractionConfig,
    cache: Option<EquityCache>,
}

impl EquityAbstraction {
    /// Uncached abstraction.
    pub fn new(config: AbstractionConfig) -> Result<Self, ConfigError> {
        config.validate()?;
        Ok(Self {
            config,
            cache: None,
        })
    }

    /// Abstraction with a bounded cache of `capacity` entries.
    pub fn with_cache(config: AbstractionConfig, capacity: usize) -> Result<Self, ConfigError> {
        config.validate()?;
        Ok(Self {
            config,
            cache: Some(EquityCache::new(capacity)),
        })
    }

    /// The configuration.
    pub fn config(&self) -> &AbstractionConfig {
        &self.config
    }

    /// Cache counters, if caching is enabled.
    pub fn cache_stats(&self) -> Option<CacheStats> {
        self.cache.as_ref().map(EquityCache::stats)
    }

    /// Equity for a postflop situation (through the cache if enabled).
    pub fn equity(&self, hole: [Card; 2], board: &[Card]) -> Equity {
        let canon = canonical_form(hole, board);
        let key = canon.key();
        let compute = || equity_of_canonical(&self.config, &canon.hole(), &canon.board(), key);
        match &self.cache {
            Some(c) => c.get_or_compute(key, compute),
            None => compute(),
        }
    }
}

impl CardAbstraction for EquityAbstraction {
    /// # Panics
    /// If hole and board cards repeat (impossible for a valid observation).
    fn bucket(
        &self,
        street: Street,
        hole: [Card; 2],
        board: &[Card],
    ) -> Result<u16, AbstractionError> {
        if board.len() != board_len(street) {
            return Err(AbstractionError::BoardLength {
                street,
                len: board.len(),
            });
        }
        let mut all = vec![hole[0], hole[1]];
        all.extend_from_slice(board);
        assert!(
            CardSet::try_from_cards(&all).is_ok(),
            "hole and board cards must be distinct"
        );
        match self.config.buckets(street) {
            None => Ok(u16::from(PreflopClass::of(hole).index())),
            Some(n) => Ok(self.equity(hole, board).bucket(n)),
        }
    }

    fn num_buckets(&self, street: Street) -> u16 {
        self.config
            .buckets(street)
            .unwrap_or(NUM_PREFLOP_CLASSES as u16)
    }
}

//! Resumable training checkpoints (M-11).
//!
//! A checkpoint is the complete trainer state needed to continue training
//! bit-identically: configuration, iteration counter, RNG state, counters
//! and every strategy-table entry (full `f64` regrets and strategy sums) in
//! insertion order. The equity cache is not stored; it never affects
//! results.
//!
//! # Binary format, version 1 (all integers little-endian)
//! | field | type |
//! |---|---|
//! | magic | `b"MAKOCKPT"` (8 bytes) |
//! | format version | `u32` = [`CHECKPOINT_VERSION`] |
//! | rules: small blind, big blind, starting stack | `u32` × 3 |
//! | abstraction config | 28 bytes ([`AbstractionConfig::to_bytes`]) |
//! | abstraction fingerprint | `u64` (must match the bytes) |
//! | flags | `u32` ([`flags_to_bits`]: bit0 RM+, bit1 linear averaging, bit2 linear regrets) |
//! | seed | `u64` |
//! | iteration | `u64` |
//! | RNG state (xoshiro256**) | `u64` × 4 |
//! | stats: traversals, nodes, traverser_nodes, opponent_samples, terminals, keys_verified | `u64` × 6 |
//! | entry count `n` | `u64` |
//! | `n` × entry: key length `u16`, key bytes, mask `u8`, regrets `f64`-bits × 6, strategy sums `f64`-bits × 6 | |
//! | checksum | `u64` FNV-1a 64 over every preceding byte |
//!
//! The checksum is for detecting accidental corruption, not a security
//! measure: anyone can recompute it.
//!
//! The byte format is pure and wasm-safe. The file helpers ([`save_atomic`],
//! [`load_file`]) exist only on non-wasm targets.

use std::fmt;

use mako_core::rules::RulesConfig;

use crate::abstraction::key::fnv1a64;
use crate::abstraction::postflop::{AbstractionConfig, CONFIG_BYTES};
use crate::infoset::{Entry, StrategyTable};
use crate::regret::{FULL_MASK, UpdateConfig, Weighting, flags_from_bits, flags_to_bits};
use crate::traverse::{Trainer, TrainerConfig, TrainerStats};

/// Magic bytes.
pub const CHECKPOINT_MAGIC: &[u8; 8] = b"MAKOCKPT";
/// Current format version.
pub const CHECKPOINT_VERSION: u32 = 1;

/// Errors decoding or applying a checkpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckpointError {
    /// Wrong magic bytes.
    BadMagic,
    /// Format version not supported by this build.
    UnsupportedVersion(u32),
    /// Input ended early.
    Truncated,
    /// Checksum mismatch (corruption).
    ChecksumMismatch,
    /// Bytes after the checksum / inconsistent length.
    TrailingBytes,
    /// A field failed validation.
    Invalid(&'static str),
    /// Two entries with the same key.
    DuplicateKey,
    /// Resume config differs from the checkpoint's.
    ConfigMismatch(&'static str),
    /// File I/O failed (message only, to keep the type `Eq`).
    Io(String),
}

impl fmt::Display for CheckpointError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CheckpointError::BadMagic => write!(f, "not a Mako checkpoint (bad magic)"),
            CheckpointError::UnsupportedVersion(v) => {
                write!(f, "unsupported checkpoint version {v}")
            }
            CheckpointError::Truncated => write!(f, "checkpoint is truncated"),
            CheckpointError::ChecksumMismatch => write!(f, "checkpoint checksum mismatch"),
            CheckpointError::TrailingBytes => write!(f, "unexpected bytes after checkpoint"),
            CheckpointError::Invalid(m) => write!(f, "invalid checkpoint: {m}"),
            CheckpointError::DuplicateKey => write!(f, "duplicate key in checkpoint"),
            CheckpointError::ConfigMismatch(m) => {
                write!(f, "checkpoint was made with a different {m}")
            }
            CheckpointError::Io(m) => write!(f, "checkpoint I/O error: {m}"),
        }
    }
}

impl std::error::Error for CheckpointError {}

/// Decoded checkpoint contents.
#[derive(Clone, Debug)]
pub struct Checkpoint {
    /// Rules the table was trained for.
    pub rules: RulesConfig,
    /// Card abstraction.
    pub abstraction: AbstractionConfig,
    /// Update flags.
    pub update: UpdateConfig,
    /// Iteration weighting flags.
    pub weighting: Weighting,
    /// Trainer seed.
    pub seed: u64,
    /// Completed iterations.
    pub iteration: u64,
    /// RNG state after `iteration` iterations.
    pub rng_state: [u64; 4],
    /// Counters.
    pub stats: TrainerStats,
    /// Strategy table (insertion order preserved).
    pub table: StrategyTable,
}

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], CheckpointError> {
        let end = self.pos.checked_add(n).ok_or(CheckpointError::Truncated)?;
        let s = self
            .b
            .get(self.pos..end)
            .ok_or(CheckpointError::Truncated)?;
        self.pos = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, CheckpointError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, CheckpointError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().expect("2")))
    }
    fn u32(&mut self) -> Result<u32, CheckpointError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("4")))
    }
    fn u64(&mut self) -> Result<u64, CheckpointError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().expect("8")))
    }
}

impl Checkpoint {
    /// Snapshot a trainer.
    pub fn from_trainer(t: &Trainer) -> Self {
        let c = t.config();
        Self {
            rules: c.rules,
            abstraction: c.abstraction,
            update: c.update,
            weighting: c.weighting,
            seed: c.seed,
            iteration: t.iteration(),
            rng_state: t.rng_state(),
            stats: t.stats(),
            table: t.table().clone(),
        }
    }

    /// Check that `config` matches this checkpoint in every field that
    /// affects results (cache capacity and key verification do not).
    pub fn check_compatible(&self, config: &TrainerConfig) -> Result<(), CheckpointError> {
        if config.rules != self.rules {
            return Err(CheckpointError::ConfigMismatch(
                "rules config (blinds/depth)",
            ));
        }
        if config.abstraction != self.abstraction {
            return Err(CheckpointError::ConfigMismatch("abstraction config"));
        }
        if config.update != self.update || config.weighting != self.weighting {
            return Err(CheckpointError::ConfigMismatch("update/weighting flags"));
        }
        if config.seed != self.seed {
            return Err(CheckpointError::ConfigMismatch("seed"));
        }
        Ok(())
    }

    /// Rebuild a trainer, after checking compatibility with `config`.
    pub fn into_trainer(self, config: TrainerConfig) -> Result<Trainer, CheckpointError> {
        self.check_compatible(&config)?;
        Trainer::restore(
            config,
            self.table,
            self.rng_state,
            self.iteration,
            self.stats,
        )
        .map_err(|e| CheckpointError::Invalid(e.0))
    }

    /// Serialize (see module docs).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(160 + self.table.len() * 120);
        b.extend_from_slice(CHECKPOINT_MAGIC);
        b.extend_from_slice(&CHECKPOINT_VERSION.to_le_bytes());
        for v in [
            self.rules.small_blind,
            self.rules.big_blind,
            self.rules.starting_stack,
        ] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&self.abstraction.to_bytes());
        b.extend_from_slice(&self.abstraction.fingerprint().to_le_bytes());
        b.extend_from_slice(&flags_to_bits(self.update, self.weighting).to_le_bytes());
        b.extend_from_slice(&self.seed.to_le_bytes());
        b.extend_from_slice(&self.iteration.to_le_bytes());
        for v in self.rng_state {
            b.extend_from_slice(&v.to_le_bytes());
        }
        let s = self.stats;
        for v in [
            s.traversals,
            s.nodes,
            s.traverser_nodes,
            s.opponent_samples,
            s.terminals,
            s.keys_verified,
        ] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&(self.table.len() as u64).to_le_bytes());
        for (k, e) in self.table.iter() {
            b.extend_from_slice(&(k.len() as u16).to_le_bytes());
            b.extend_from_slice(k);
            b.push(e.mask);
            for v in e.regret.iter().chain(e.strategy_sum.iter()) {
                b.extend_from_slice(&v.to_bits().to_le_bytes());
            }
        }
        let sum = fnv1a64(&b);
        b.extend_from_slice(&sum.to_le_bytes());
        b
    }

    /// Decode and validate.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CheckpointError> {
        if bytes.len() < 12 {
            return Err(CheckpointError::Truncated);
        }
        if &bytes[..8] != CHECKPOINT_MAGIC {
            return Err(CheckpointError::BadMagic);
        }
        let version = u32::from_le_bytes(bytes[8..12].try_into().expect("4"));
        if version != CHECKPOINT_VERSION {
            return Err(CheckpointError::UnsupportedVersion(version));
        }
        if bytes.len() < 20 {
            return Err(CheckpointError::Truncated);
        }
        let (body, tail) = bytes.split_at(bytes.len() - 8);
        let stored = u64::from_le_bytes(tail.try_into().expect("8"));
        if fnv1a64(body) != stored {
            // Could be corruption or truncation; both fail here.
            return Err(CheckpointError::ChecksumMismatch);
        }
        let mut r = Reader { b: body, pos: 12 };
        let rules = RulesConfig {
            small_blind: r.u32()?,
            big_blind: r.u32()?,
            starting_stack: r.u32()?,
        };
        rules
            .validate()
            .map_err(|_| CheckpointError::Invalid("rules config"))?;
        let abstraction = AbstractionConfig::from_bytes(r.take(CONFIG_BYTES)?)
            .map_err(|_| CheckpointError::Invalid("abstraction config"))?;
        if r.u64()? != abstraction.fingerprint() {
            return Err(CheckpointError::Invalid("abstraction fingerprint"));
        }
        let (update, weighting) =
            flags_from_bits(r.u32()?).ok_or(CheckpointError::Invalid("unknown flag bits"))?;
        let seed = r.u64()?;
        let iteration = r.u64()?;
        let rng_state = [r.u64()?, r.u64()?, r.u64()?, r.u64()?];
        let stats = TrainerStats {
            traversals: r.u64()?,
            nodes: r.u64()?,
            traverser_nodes: r.u64()?,
            opponent_samples: r.u64()?,
            terminals: r.u64()?,
            keys_verified: r.u64()?,
        };
        let n = r.u64()?;
        let mut table = StrategyTable::new();
        for _ in 0..n {
            let klen = usize::from(r.u16()?);
            let key = r.take(klen)?;
            let mask = r.u8()?;
            if mask == 0 || mask & !FULL_MASK != 0 {
                return Err(CheckpointError::Invalid("legal mask"));
            }
            let mut e = Entry::new(mask);
            for v in e.regret.iter_mut().chain(e.strategy_sum.iter_mut()) {
                *v = f64::from_bits(r.u64()?);
            }
            if e.regret
                .iter()
                .chain(&e.strategy_sum)
                .any(|v| !v.is_finite())
            {
                return Err(CheckpointError::Invalid("non-finite value"));
            }
            if !table.push_entry(key, e) {
                return Err(CheckpointError::DuplicateKey);
            }
        }
        if r.pos != body.len() {
            return Err(CheckpointError::TrailingBytes);
        }
        Ok(Self {
            rules,
            abstraction,
            update,
            weighting,
            seed,
            iteration,
            rng_state,
            stats,
            table,
        })
    }
}

/// Atomically write `bytes` to `path`: write `path.tmp`, flush and fsync it,
/// then rename it over `path`.
#[cfg(not(target_arch = "wasm32"))]
pub fn save_atomic(path: &std::path::Path, bytes: &[u8]) -> Result<(), CheckpointError> {
    use std::io::Write;
    let io = |e: std::io::Error| CheckpointError::Io(e.to_string());
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    {
        let mut f = std::fs::File::create(&tmp).map_err(io)?;
        f.write_all(bytes).map_err(io)?;
        f.sync_all().map_err(io)?;
    }
    std::fs::rename(&tmp, path).map_err(io)
}

/// Read and decode a checkpoint file.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_file(path: &std::path::Path) -> Result<Checkpoint, CheckpointError> {
    let bytes = std::fs::read(path).map_err(|e| CheckpointError::Io(e.to_string()))?;
    Checkpoint::from_bytes(&bytes)
}

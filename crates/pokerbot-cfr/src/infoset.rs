//! Strategy table: information-set key bytes → regret / strategy sums.
//!
//! - Keys are the exact [`crate::abstraction::InfoKey`] byte encodings, never
//!   just their hash, so distinct information sets cannot alias.
//! - Entries live in a dense `Vec` in **insertion order**. A `HashMap` maps
//!   key bytes to entry index and is never iterated, so iteration,
//!   [`StrategyTable::digest`] and any later serialization are reproducible.
//! - Each entry fixes its legal mask on first visit. A revisit with a
//!   different mask is a bug (the key encodes the full public history, which
//!   determines the legal set for a fixed `RulesConfig`) and panics.
//! - Values are `f64`: training is single-threaded, so the memory cost
//!   (104 bytes per entry for values) is acceptable, and f64 keeps long-run
//!   accumulation precise. A compact f32 form can be produced at export.

use std::collections::HashMap;

use crate::abstraction::key::fnv1a64;
use crate::regret::{LegalMask, NUM_SLOTS, average_masked, regret_matching_masked};

/// Per-information-set accumulators.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// Legal slots (fixed on first visit).
    pub mask: LegalMask,
    /// Cumulative regret per slot (illegal slots stay 0).
    pub regret: [f64; NUM_SLOTS],
    /// Cumulative weighted strategy per slot (illegal slots stay 0).
    pub strategy_sum: [f64; NUM_SLOTS],
}

impl Entry {
    /// Fresh zeroed entry.
    pub fn new(mask: LegalMask) -> Self {
        Self {
            mask,
            regret: [0.0; NUM_SLOTS],
            strategy_sum: [0.0; NUM_SLOTS],
        }
    }

    /// Current regret-matching strategy.
    pub fn current_strategy(&self) -> [f64; NUM_SLOTS] {
        let mut out = [0.0; NUM_SLOTS];
        regret_matching_masked(&self.regret, self.mask, &mut out);
        out
    }

    /// Normalized average strategy.
    pub fn average_strategy(&self) -> [f64; NUM_SLOTS] {
        let mut out = [0.0; NUM_SLOTS];
        average_masked(&self.strategy_sum, self.mask, &mut out);
        out
    }
}

/// Deterministic, insertion-ordered strategy table.
#[derive(Clone, Debug, Default)]
pub struct StrategyTable {
    index: HashMap<Box<[u8]>, u32>,
    keys: Vec<Box<[u8]>>,
    entries: Vec<Entry>,
}

impl StrategyTable {
    /// Empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of information sets.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Index of `key`, inserting a fresh entry with `mask` if absent.
    ///
    /// # Panics
    /// If the key exists with a different legal mask (an abstraction bug).
    pub fn index_or_insert(&mut self, key: &[u8], mask: LegalMask) -> usize {
        if let Some(&i) = self.index.get(key) {
            let i = i as usize;
            assert_eq!(
                self.entries[i].mask, mask,
                "legal mask changed for information set {key:?}"
            );
            return i;
        }
        let i = self.entries.len();
        let boxed: Box<[u8]> = key.into();
        self.index
            .insert(boxed.clone(), u32::try_from(i).expect("table < 2^32"));
        self.keys.push(boxed);
        self.entries.push(Entry::new(mask));
        i
    }

    /// Append a fully populated entry (checkpoint restore). Returns `false`
    /// and leaves the table unchanged if `key` is already present.
    pub fn push_entry(&mut self, key: &[u8], entry: Entry) -> bool {
        if self.index.contains_key(key) {
            return false;
        }
        let i = u32::try_from(self.entries.len()).expect("table < 2^32");
        let boxed: Box<[u8]> = key.into();
        self.index.insert(boxed.clone(), i);
        self.keys.push(boxed);
        self.entries.push(entry);
        true
    }

    /// Look up an existing key.
    pub fn get(&self, key: &[u8]) -> Option<&Entry> {
        self.index.get(key).map(|&i| &self.entries[i as usize])
    }

    /// Entry by index.
    pub fn entry(&self, i: usize) -> &Entry {
        &self.entries[i]
    }

    /// Mutable entry by index.
    pub fn entry_mut(&mut self, i: usize) -> &mut Entry {
        &mut self.entries[i]
    }

    /// `(key bytes, entry)` in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (&[u8], &Entry)> {
        self.keys.iter().map(|k| &**k).zip(self.entries.iter())
    }

    /// FNV-1a 64 over every key and value bit pattern, in insertion order.
    /// Equal digests ⇔ (with overwhelming probability) bit-identical tables.
    pub fn digest(&self) -> u64 {
        let mut buf = Vec::with_capacity(self.len() * 120);
        for (k, e) in self.iter() {
            buf.extend_from_slice(&(k.len() as u32).to_le_bytes());
            buf.extend_from_slice(k);
            buf.push(e.mask);
            for v in e.regret.iter().chain(e.strategy_sum.iter()) {
                buf.extend_from_slice(&v.to_bits().to_le_bytes());
            }
        }
        fnv1a64(&buf)
    }

    /// Rough heap footprint in bytes (entries, keys, index).
    pub fn approx_bytes(&self) -> usize {
        let key_bytes: usize = self.keys.iter().map(|k| k.len()).sum();
        let per_entry = std::mem::size_of::<Entry>()
            + 2 * std::mem::size_of::<Box<[u8]>>() // keys vec + index map key
            + std::mem::size_of::<u32>()
            + 8; // hash-map control/overhead estimate
        self.len() * per_entry + 2 * key_bytes
    }
}

//! Versioned `.mako` strategy artifact and policy reader (M-12).
//!
//! An artifact holds the **average strategy** of a trained table, quantized
//! to `u16` per legal action, plus everything needed to check that a caller
//! uses it with the same game: rules-spec version, `RulesConfig` (blinds and
//! depth), card abstraction, trainer flags and seed. The exact byte layout,
//! validation rules and versioning policy are in `docs/STRATEGY_FORMAT.md`.
//!
//! Reading is pure (no file system), so it works natively and in wasm.
//! Artifacts are byte-reproducible: the same table and metadata always give
//! identical bytes. There is no timestamp inside the file.
//!
//! An artifact approximates an equilibrium of the abstract game only. It is
//! not GTO, and its quality depends on training length; see the label.

use std::fmt;

use mako_core::action::Seat;
use mako_core::deck::Deck;
use mako_core::rules::{Position, RulesConfig};
use mako_core::state::{HandConfig, HandState};

use crate::abstraction::key::{InfoKey, fnv1a64};
use crate::abstraction::postflop::{AbstractionConfig, CONFIG_BYTES};
use crate::abstraction::preflop::NUM_PREFLOP_CLASSES;
use crate::infoset::StrategyTable;
use crate::regret::{
    FULL_MASK, NUM_SLOTS, UpdateConfig, Weighting, flags_from_bits, flags_to_bits, is_legal,
    mask_of,
};
use crate::traverse::Trainer;

/// Magic bytes.
pub const ARTIFACT_MAGIC: &[u8; 8] = b"MAKOSTRT";
/// Current artifact format version.
pub const ARTIFACT_VERSION: u16 = 1;
/// Rules specification the abstraction and keys follow (`docs/GAME_RULES.md`).
pub const RULES_SPEC: &str = "GAME_RULES v1";
/// Quantized probabilities of one entry sum to exactly this.
pub const PROB_SCALE: u16 = u16::MAX;
/// Maximum length of the trainer id and label strings.
pub const MAX_TEXT_BYTES: usize = 4096;

/// Artifact errors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArtifactError {
    /// Wrong magic bytes.
    BadMagic,
    /// Unsupported format version.
    UnsupportedVersion(u16),
    /// Unsupported rules specification string.
    UnsupportedRulesSpec(String),
    /// Input ended early.
    Truncated,
    /// Checksum mismatch.
    ChecksumMismatch,
    /// Extra bytes after the last entry.
    TrailingBytes,
    /// Keys not strictly increasing.
    UnsortedKeys,
    /// The same key twice.
    DuplicateKey,
    /// A field failed validation.
    Invalid(String),
    /// The artifact was trained for different rules (e.g. another depth).
    RulesMismatch {
        /// Rules the caller plays.
        expected: RulesConfig,
        /// Rules in the artifact.
        found: RulesConfig,
    },
    /// The artifact uses a different card abstraction.
    AbstractionMismatch {
        /// Caller's fingerprint.
        expected: u64,
        /// Artifact's fingerprint.
        found: u64,
    },
}

impl fmt::Display for ArtifactError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ArtifactError::BadMagic => write!(f, "not a Mako strategy artifact (bad magic)"),
            ArtifactError::UnsupportedVersion(v) => write!(f, "unsupported artifact version {v}"),
            ArtifactError::UnsupportedRulesSpec(s) => write!(f, "unsupported rules spec {s:?}"),
            ArtifactError::Truncated => write!(f, "artifact is truncated"),
            ArtifactError::ChecksumMismatch => write!(f, "artifact checksum mismatch"),
            ArtifactError::TrailingBytes => write!(f, "unexpected bytes after artifact entries"),
            ArtifactError::UnsortedKeys => write!(f, "artifact keys are not sorted"),
            ArtifactError::DuplicateKey => write!(f, "artifact contains a duplicate key"),
            ArtifactError::Invalid(m) => write!(f, "invalid artifact: {m}"),
            ArtifactError::RulesMismatch { expected, found } => write!(
                f,
                "artifact was trained for blinds {}/{} with {} starting chips, \
                 but the game uses {}/{} with {} (wrong depth or blinds)",
                found.small_blind,
                found.big_blind,
                found.starting_stack,
                expected.small_blind,
                expected.big_blind,
                expected.starting_stack
            ),
            ArtifactError::AbstractionMismatch { expected, found } => write!(
                f,
                "artifact abstraction fingerprint {found:#018x} differs from {expected:#018x}"
            ),
        }
    }
}

impl std::error::Error for ArtifactError {}

/// Provenance and compatibility metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactMetadata {
    /// Rules the policy was trained for.
    pub rules: RulesConfig,
    /// Card abstraction.
    pub abstraction: AbstractionConfig,
    /// Trainer seed.
    pub seed: u64,
    /// Training iterations.
    pub iterations: u64,
    /// Update flags.
    pub update: UpdateConfig,
    /// Weighting flags.
    pub weighting: Weighting,
    /// Trainer / build identifier.
    pub trainer_id: String,
    /// Free-text label, e.g. "smoke: trained for plumbing, not competition".
    pub label: String,
}

impl ArtifactMetadata {
    /// Metadata describing `trainer`'s current state.
    pub fn from_trainer(trainer: &Trainer, label: impl Into<String>) -> Self {
        let c = trainer.config();
        Self {
            rules: c.rules,
            abstraction: c.abstraction,
            seed: c.seed,
            iterations: trainer.iteration(),
            update: c.update,
            weighting: c.weighting,
            trainer_id: default_trainer_id(),
            label: label.into(),
        }
    }
}

/// `"mako-cfr <version> es-mccfr"`.
pub fn default_trainer_id() -> String {
    format!("mako-cfr {} es-mccfr", crate::VERSION)
}

/// One stored information set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactEntry {
    /// `InfoKey` bytes.
    pub key: Vec<u8>,
    /// Legal slots.
    pub mask: u8,
    /// Quantized probabilities per slot (illegal slots 0; sum 65535).
    pub q: [u16; NUM_SLOTS],
}

impl ArtifactEntry {
    /// Probabilities as `f64` (`q / 65535`).
    pub fn probs(&self) -> [f64; NUM_SLOTS] {
        self.q.map(|v| f64::from(v) / f64::from(PROB_SCALE))
    }
}

/// Largest-remainder quantization of a distribution over the legal slots
/// of `mask` to integers summing to exactly 65535.
///
/// 1. `x[i] = p[i] / Σp · 65535` for legal `i` (0 for illegal slots).
/// 2. `q[i] = floor(x[i])`.
/// 3. The remaining `65535 − Σq` units go one each to legal slots in order of
///    decreasing fractional part `x[i] − q[i]`, ties broken by lower slot
///    index.
///
/// A distribution with no positive legal mass becomes uniform over legal
/// slots before quantization.
///
/// # Panics
/// If `mask` is empty.
pub fn quantize(p: &[f64; NUM_SLOTS], mask: u8) -> [u16; NUM_SLOTS] {
    let legal: Vec<usize> = (0..NUM_SLOTS).filter(|&i| is_legal(mask, i)).collect();
    assert!(!legal.is_empty(), "cannot quantize with an empty mask");
    let mut w = [0.0f64; NUM_SLOTS];
    for &i in &legal {
        w[i] = if p[i].is_finite() && p[i] > 0.0 {
            p[i]
        } else {
            0.0
        };
    }
    let total: f64 = w.iter().sum();
    if total <= 0.0 || !total.is_finite() {
        for &i in &legal {
            w[i] = 1.0;
        }
    }
    let total: f64 = w.iter().sum();
    let scale = f64::from(PROB_SCALE);
    let mut q = [0u16; NUM_SLOTS];
    let mut frac = [0.0f64; NUM_SLOTS];
    let mut sum: u32 = 0;
    for &i in &legal {
        let x = (w[i] / total * scale).min(scale);
        let f = x.floor();
        q[i] = f as u16;
        frac[i] = x - f;
        sum += u32::from(q[i]);
    }
    let mut order = legal.clone();
    order.sort_by(|&a, &b| frac[b].total_cmp(&frac[a]).then(a.cmp(&b)));
    let target = u32::from(PROB_SCALE);
    let mut k = 0;
    while sum < target {
        q[order[k % order.len()]] += 1;
        sum += 1;
        k += 1;
    }
    // Rounding can never overshoot (Σ floor(x) <= Σ x = 65535), but stay safe.
    let mut k = 0;
    while sum > target {
        let i = order[order.len() - 1 - (k % order.len())];
        if q[i] > 0 {
            q[i] -= 1;
            sum -= 1;
        }
        k += 1;
    }
    q
}

/// Legal mask at the decision described by `key`, found by replaying its
/// public history under `rules` (cards do not affect legality). Errors if
/// the history is not a reachable decision for `key`'s street and position.
pub fn legal_mask_for_key(rules: &RulesConfig, key: &InfoKey) -> Result<u8, ArtifactError> {
    let bad = |m: &str| ArtifactError::Invalid(format!("key {key}: {m}"));
    let cfg = HandConfig {
        rules: *rules,
        button: Seat::Zero,
    };
    let mut state = HandState::new(cfg, Deck::ordered()).map_err(|e| bad(&e.to_string()))?;
    for (street_i, acts) in key.history().streets().iter().enumerate() {
        for &kind in acts {
            if state.is_terminal() || state.street().betting().map(|s| s as usize) != Some(street_i)
            {
                return Err(bad("history does not match street boundaries"));
            }
            state
                .apply(kind)
                .map_err(|_| bad("illegal action in history"))?;
        }
    }
    if state.is_terminal() || state.street().betting() != Some(key.street()) {
        return Err(bad(
            "history does not end at a decision on the key's street",
        ));
    }
    let seat = state.to_act().ok_or_else(|| bad("no player to act"))?;
    let pos = if seat == Seat::Zero {
        Position::Button
    } else {
        Position::BigBlind
    };
    if pos != key.position() {
        return Err(bad("position does not match the player to act"));
    }
    Ok(mask_of(state.legal_actions().iter().map(|a| a.kind)))
}

/// An average-strategy artifact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StrategyArtifact {
    /// Metadata.
    pub metadata: ArtifactMetadata,
    /// Entries, sorted by key bytes, unique.
    pub entries: Vec<ArtifactEntry>,
}

fn put_text(b: &mut Vec<u8>, s: &str) {
    b.extend_from_slice(&(s.len() as u16).to_le_bytes());
    b.extend_from_slice(s.as_bytes());
}

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], ArtifactError> {
        let end = self.pos.checked_add(n).ok_or(ArtifactError::Truncated)?;
        let s = self.b.get(self.pos..end).ok_or(ArtifactError::Truncated)?;
        self.pos = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, ArtifactError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, ArtifactError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().expect("2")))
    }
    fn u32(&mut self) -> Result<u32, ArtifactError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("4")))
    }
    fn u64(&mut self) -> Result<u64, ArtifactError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().expect("8")))
    }
    fn text(&mut self, len: usize) -> Result<String, ArtifactError> {
        let s = self.take(len)?;
        String::from_utf8(s.to_vec())
            .map_err(|_| ArtifactError::Invalid("text is not UTF-8".into()))
    }
}

fn num_buckets(abs: &AbstractionConfig, key: &InfoKey) -> u16 {
    abs.buckets(key.street())
        .unwrap_or(NUM_PREFLOP_CLASSES as u16)
}

impl StrategyArtifact {
    /// Build from a table's average strategy. Information sets that never
    /// accumulated average-strategy mass (visited only as the traverser) are
    /// omitted; [`Policy::lookup`] returns `None` for them.
    pub fn from_table(
        table: &StrategyTable,
        metadata: ArtifactMetadata,
    ) -> Result<Self, ArtifactError> {
        if metadata.trainer_id.len() > MAX_TEXT_BYTES || metadata.label.len() > MAX_TEXT_BYTES {
            return Err(ArtifactError::Invalid(
                "trainer id or label too long".into(),
            ));
        }
        let mut entries: Vec<ArtifactEntry> = table
            .iter()
            .filter(|(_, e)| e.strategy_sum.iter().any(|&s| s > 0.0))
            .map(|(k, e)| ArtifactEntry {
                key: k.to_vec(),
                mask: e.mask,
                q: quantize(&e.average_strategy(), e.mask),
            })
            .collect();
        entries.sort_by(|a, b| a.key.cmp(&b.key));
        let a = Self { metadata, entries };
        // Validate what we produce with the same rules as the reader.
        a.validate()?;
        Ok(a)
    }

    /// FNV-1a 64 checksum of the serialized artifact (its last 8 bytes).
    pub fn checksum(&self) -> u64 {
        let b = self.to_bytes();
        u64::from_le_bytes(b[b.len() - 8..].try_into().expect("8"))
    }

    /// Serialize (layout in `docs/STRATEGY_FORMAT.md`).
    pub fn to_bytes(&self) -> Vec<u8> {
        let m = &self.metadata;
        let mut b = Vec::with_capacity(128 + self.entries.len() * 20);
        b.extend_from_slice(ARTIFACT_MAGIC);
        b.extend_from_slice(&ARTIFACT_VERSION.to_le_bytes());
        b.push(RULES_SPEC.len() as u8);
        b.extend_from_slice(RULES_SPEC.as_bytes());
        for v in [
            m.rules.small_blind,
            m.rules.big_blind,
            m.rules.starting_stack,
        ] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&m.abstraction.to_bytes());
        b.extend_from_slice(&m.abstraction.fingerprint().to_le_bytes());
        b.extend_from_slice(&flags_to_bits(m.update, m.weighting).to_le_bytes());
        b.extend_from_slice(&m.seed.to_le_bytes());
        b.extend_from_slice(&m.iterations.to_le_bytes());
        put_text(&mut b, &m.trainer_id);
        put_text(&mut b, &m.label);
        b.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        for e in &self.entries {
            b.push(e.key.len() as u8);
            b.extend_from_slice(&e.key);
            b.push(e.mask);
            for i in 0..NUM_SLOTS {
                if is_legal(e.mask, i) {
                    b.extend_from_slice(&e.q[i].to_le_bytes());
                }
            }
        }
        let sum = fnv1a64(&b);
        b.extend_from_slice(&sum.to_le_bytes());
        b
    }

    /// Decode and fully validate.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ArtifactError> {
        if bytes.len() < 10 {
            return Err(ArtifactError::Truncated);
        }
        if &bytes[..8] != ARTIFACT_MAGIC {
            return Err(ArtifactError::BadMagic);
        }
        let version = u16::from_le_bytes([bytes[8], bytes[9]]);
        if version != ARTIFACT_VERSION {
            return Err(ArtifactError::UnsupportedVersion(version));
        }
        if bytes.len() < 18 {
            return Err(ArtifactError::Truncated);
        }
        let (body, tail) = bytes.split_at(bytes.len() - 8);
        if fnv1a64(body) != u64::from_le_bytes(tail.try_into().expect("8")) {
            return Err(ArtifactError::ChecksumMismatch);
        }
        let mut r = Reader { b: body, pos: 10 };
        let spec_len = usize::from(r.u8()?);
        let spec = r.text(spec_len)?;
        if spec != RULES_SPEC {
            return Err(ArtifactError::UnsupportedRulesSpec(spec));
        }
        let rules = RulesConfig {
            small_blind: r.u32()?,
            big_blind: r.u32()?,
            starting_stack: r.u32()?,
        };
        let abstraction = AbstractionConfig::from_bytes(r.take(CONFIG_BYTES)?)
            .map_err(|e| ArtifactError::Invalid(e.to_string()))?;
        if r.u64()? != abstraction.fingerprint() {
            return Err(ArtifactError::Invalid("abstraction fingerprint".into()));
        }
        let (update, weighting) = flags_from_bits(r.u32()?)
            .ok_or_else(|| ArtifactError::Invalid("unknown flag bits".into()))?;
        let seed = r.u64()?;
        let iterations = r.u64()?;
        let n = usize::from(r.u16()?);
        let trainer_id = r.text(n)?;
        let n = usize::from(r.u16()?);
        let label = r.text(n)?;
        let count = r.u32()? as usize;
        let mut entries = Vec::with_capacity(count.min(1 << 20));
        for _ in 0..count {
            let klen = usize::from(r.u8()?);
            let key = r.take(klen)?.to_vec();
            let mask = r.u8()?;
            if mask == 0 || mask & !FULL_MASK != 0 {
                return Err(ArtifactError::Invalid(
                    "empty or out-of-range legal mask".into(),
                ));
            }
            let mut q = [0u16; NUM_SLOTS];
            for (i, slot) in q.iter_mut().enumerate() {
                if is_legal(mask, i) {
                    *slot = r.u16()?;
                }
            }
            entries.push(ArtifactEntry { key, mask, q });
        }
        if r.pos != body.len() {
            return Err(ArtifactError::TrailingBytes);
        }
        let a = Self {
            metadata: ArtifactMetadata {
                rules,
                abstraction,
                seed,
                iterations,
                update,
                weighting,
                trainer_id,
                label,
            },
            entries,
        };
        a.validate()?;
        Ok(a)
    }

    /// Structural and semantic checks shared by writer and reader.
    pub fn validate(&self) -> Result<(), ArtifactError> {
        let m = &self.metadata;
        m.rules
            .validate()
            .map_err(|e| ArtifactError::Invalid(e.to_string()))?;
        m.abstraction
            .validate()
            .map_err(|e| ArtifactError::Invalid(e.to_string()))?;
        if m.trainer_id.len() > MAX_TEXT_BYTES || m.label.len() > MAX_TEXT_BYTES {
            return Err(ArtifactError::Invalid("text field too long".into()));
        }
        for (i, e) in self.entries.iter().enumerate() {
            if i > 0 {
                match self.entries[i - 1].key.cmp(&e.key) {
                    std::cmp::Ordering::Less => {}
                    std::cmp::Ordering::Equal => return Err(ArtifactError::DuplicateKey),
                    std::cmp::Ordering::Greater => return Err(ArtifactError::UnsortedKeys),
                }
            }
            let key = InfoKey::from_bytes(&e.key)
                .map_err(|err| ArtifactError::Invalid(format!("entry {i}: {err}")))?;
            if key.bucket() >= num_buckets(&m.abstraction, &key) {
                return Err(ArtifactError::Invalid(format!(
                    "entry {i}: bucket out of range"
                )));
            }
            if e.mask == 0 || e.mask & !FULL_MASK != 0 {
                return Err(ArtifactError::Invalid(format!("entry {i}: bad legal mask")));
            }
            let legal = legal_mask_for_key(&m.rules, &key)?;
            if legal != e.mask {
                return Err(ArtifactError::Invalid(format!(
                    "entry {i}: mask {:#08b} is not the legal set {legal:#08b} for {key}",
                    e.mask
                )));
            }
            let mut sum: u32 = 0;
            for s in 0..NUM_SLOTS {
                if !is_legal(e.mask, s) && e.q[s] != 0 {
                    return Err(ArtifactError::Invalid(format!(
                        "entry {i}: illegal-slot mass"
                    )));
                }
                sum += u32::from(e.q[s]);
            }
            if sum != u32::from(PROB_SCALE) {
                return Err(ArtifactError::Invalid(format!(
                    "entry {i}: probabilities sum to {sum}, not 65535"
                )));
            }
        }
        Ok(())
    }
}

/// A loaded, validated policy for one game configuration.
#[derive(Clone, Debug)]
pub struct Policy {
    artifact: StrategyArtifact,
}

impl Policy {
    /// Validate `bytes` and wrap them without compatibility checks. Prefer
    /// [`Policy::load`].
    pub fn from_bytes_unchecked_config(bytes: &[u8]) -> Result<Self, ArtifactError> {
        Ok(Self {
            artifact: StrategyArtifact::from_bytes(bytes)?,
        })
    }

    /// Load for a game played with `rules` and `abstraction`. Fails on a
    /// depth/blind mismatch or a different abstraction.
    pub fn load(
        bytes: &[u8],
        rules: &RulesConfig,
        abstraction: &AbstractionConfig,
    ) -> Result<Self, ArtifactError> {
        let p = Self::from_bytes_unchecked_config(bytes)?;
        p.expect_rules(rules)?;
        p.expect_abstraction(abstraction)?;
        Ok(p)
    }

    /// Error unless the artifact was trained for exactly `rules`.
    pub fn expect_rules(&self, rules: &RulesConfig) -> Result<(), ArtifactError> {
        let found = self.artifact.metadata.rules;
        if found != *rules {
            return Err(ArtifactError::RulesMismatch {
                expected: *rules,
                found,
            });
        }
        Ok(())
    }

    /// Error unless the artifact uses exactly `abstraction`.
    pub fn expect_abstraction(&self, abstraction: &AbstractionConfig) -> Result<(), ArtifactError> {
        let found = self.artifact.metadata.abstraction.fingerprint();
        let expected = abstraction.fingerprint();
        if found != expected || self.artifact.metadata.abstraction != *abstraction {
            return Err(ArtifactError::AbstractionMismatch { expected, found });
        }
        Ok(())
    }

    /// Metadata.
    pub fn metadata(&self) -> &ArtifactMetadata {
        &self.artifact.metadata
    }

    /// Number of stored information sets.
    pub fn len(&self) -> usize {
        self.artifact.entries.len()
    }

    /// True when no entries are stored.
    pub fn is_empty(&self) -> bool {
        self.artifact.entries.is_empty()
    }

    /// Entry for key bytes (binary search).
    pub fn lookup_bytes(&self, key: &[u8]) -> Option<&ArtifactEntry> {
        self.artifact
            .entries
            .binary_search_by(|e| e.key.as_slice().cmp(key))
            .ok()
            .map(|i| &self.artifact.entries[i])
    }

    /// Average-strategy probabilities per slot, or `None` for an unseen key.
    pub fn lookup(&self, key: &InfoKey) -> Option<[f64; NUM_SLOTS]> {
        self.lookup_bytes(&key.to_bytes()).map(ArtifactEntry::probs)
    }

    /// The validated artifact.
    pub fn artifact(&self) -> &StrategyArtifact {
        &self.artifact
    }
}

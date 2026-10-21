//! Training-run configuration files (M-13).
//!
//! A strict parser for the small TOML subset used by `configs/*.toml`, with
//! no external dependency:
//! - `[section]` headers, `key = value` lines, `#` comments (full-line or
//!   after a value), and blank lines.
//! - Values: decimal integers (`_` separators allowed), `true` / `false`, and
//!   double-quoted strings with `\"`, `\\`, `\n` and `\t` escapes.
//! - Unknown sections or keys, duplicate keys, wrong value types and
//!   malformed lines are errors, reported with their 1-based line number.
//!
//! Schema (`*` = required):
//! ```text
//! [rules]        depth_bb*, small_blind (1), big_blind (2)
//!                starting_stack = depth_bb * big_blind
//! [abstraction]  flop_buckets, turn_buckets, river_buckets, flop_samples,
//!                turn_samples (0 = exact), seed  (defaults: AbstractionConfig::default())
//! [train]        seed*, iterations*, checkpoint_every, max_wall_secs,
//!                linear_averaging (false), linear_regrets (false),
//!                regret_matching_plus (false), cache_capacity (65536)
//! [output]       dir ("artifacts"), artifact*, checkpoint*, manifest*, label*
//! ```
//! `iterations` must be in `1..=MAX_CONFIG_ITERATIONS`, and `max_wall_secs`
//! at most 86,400. There is no unbounded run.

use std::collections::BTreeMap;
use std::fmt;

use pokerbot_core::rules::RulesConfig;

use crate::abstraction::postflop::AbstractionConfig;
use crate::regret::{UpdateConfig, Weighting};
use crate::train::TrainConfig;

/// Hard cap on iterations from a config file or the CLI override.
pub const MAX_CONFIG_ITERATIONS: u64 = 100_000_000;
/// Hard cap on `max_wall_secs` (one day).
pub const MAX_WALL_SECS: u64 = 86_400;

/// A parsed scalar value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    /// Integer.
    Int(i64),
    /// Boolean.
    Bool(bool),
    /// String.
    Str(String),
}

/// Configuration error with an optional 1-based line number.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigFileError {
    /// Line number, if the error is tied to a line.
    pub line: Option<usize>,
    /// Description.
    pub message: String,
}

impl fmt::Display for ConfigFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(l) => write!(f, "config line {l}: {}", self.message),
            None => write!(f, "config: {}", self.message),
        }
    }
}

impl std::error::Error for ConfigFileError {}

fn err(line: usize, m: impl Into<String>) -> ConfigFileError {
    ConfigFileError {
        line: Some(line),
        message: m.into(),
    }
}

const SCHEMA: &[(&str, &[&str])] = &[
    ("rules", &["depth_bb", "small_blind", "big_blind"]),
    (
        "abstraction",
        &[
            "flop_buckets",
            "turn_buckets",
            "river_buckets",
            "flop_samples",
            "turn_samples",
            "seed",
        ],
    ),
    (
        "train",
        &[
            "seed",
            "iterations",
            "checkpoint_every",
            "max_wall_secs",
            "linear_averaging",
            "linear_regrets",
            "regret_matching_plus",
            "cache_capacity",
        ],
    ),
    (
        "output",
        &["dir", "artifact", "checkpoint", "manifest", "label"],
    ),
];

fn parse_string(s: &str, line: usize) -> Result<(String, &str), ConfigFileError> {
    // `s` starts after the opening quote.
    let mut out = String::new();
    let mut chars = s.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => return Ok((out, &s[i + 1..])),
            '\\' => match chars.next() {
                Some((_, '"')) => out.push('"'),
                Some((_, '\\')) => out.push('\\'),
                Some((_, 'n')) => out.push('\n'),
                Some((_, 't')) => out.push('\t'),
                _ => return Err(err(line, "invalid escape in string")),
            },
            c => out.push(c),
        }
    }
    Err(err(line, "unterminated string"))
}

fn parse_value(raw: &str, line: usize) -> Result<Value, ConfigFileError> {
    let raw = raw.trim_start();
    let (value, rest) = if let Some(s) = raw.strip_prefix('"') {
        let (v, rest) = parse_string(s, line)?;
        (Value::Str(v), rest)
    } else {
        let end = raw.find('#').unwrap_or(raw.len());
        let tok = raw[..end].trim();
        let v = match tok {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            "" => return Err(err(line, "missing value")),
            t => {
                let digits = t.replace('_', "");
                let ok = !digits.is_empty()
                    && digits
                        .strip_prefix('-')
                        .unwrap_or(&digits)
                        .chars()
                        .all(|c| c.is_ascii_digit())
                    && !t.starts_with('_')
                    && !t.ends_with('_');
                if !ok {
                    return Err(err(line, format!("invalid value {t:?}")));
                }
                Value::Int(
                    digits
                        .parse()
                        .map_err(|_| err(line, format!("integer out of range: {t}")))?,
                )
            }
        };
        (v, &raw[end..])
    };
    let rest = rest.trim();
    if !(rest.is_empty() || rest.starts_with('#')) {
        return Err(err(line, format!("unexpected text after value: {rest:?}")));
    }
    Ok(value)
}

/// Parsed document: `(section, key) → (value, 1-based line)`.
pub type Document = BTreeMap<(String, String), (Value, usize)>;

/// Parse a config document into `(section, key) → (value, line)`.
pub fn parse_document(text: &str) -> Result<Document, ConfigFileError> {
    let mut out = BTreeMap::new();
    let mut section: Option<&'static str> = None;
    for (i, raw) in text.lines().enumerate() {
        let line = i + 1;
        let t = raw.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        if let Some(h) = t.strip_prefix('[') {
            let end = h
                .find(']')
                .ok_or_else(|| err(line, "malformed section header"))?;
            let rest = h[end + 1..].trim();
            if !(rest.is_empty() || rest.starts_with('#')) {
                return Err(err(line, "unexpected text after section header"));
            }
            let name = h[..end].trim();
            section = Some(
                SCHEMA
                    .iter()
                    .find(|(s, _)| *s == name)
                    .map(|(s, _)| *s)
                    .ok_or_else(|| err(line, format!("unknown section [{name}]")))?,
            );
            continue;
        }
        let eq = t
            .find('=')
            .ok_or_else(|| err(line, "expected `key = value`"))?;
        let key = t[..eq].trim();
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(err(line, format!("invalid key {key:?}")));
        }
        let sec = section.ok_or_else(|| err(line, "key outside of any [section]"))?;
        let allowed = SCHEMA
            .iter()
            .find(|(s, _)| *s == sec)
            .map(|(_, k)| *k)
            .unwrap_or(&[]);
        if !allowed.contains(&key) {
            return Err(err(line, format!("unknown key `{key}` in [{sec}]")));
        }
        let v = parse_value(&t[eq + 1..], line)?;
        if out
            .insert((sec.to_string(), key.to_string()), (v, line))
            .is_some()
        {
            return Err(err(line, format!("duplicate key `{key}` in [{sec}]")));
        }
    }
    Ok(out)
}

/// Output locations and label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputConfig {
    /// Output directory.
    pub dir: String,
    /// Artifact file name (within `dir`).
    pub artifact: String,
    /// Checkpoint file name.
    pub checkpoint: String,
    /// Manifest file name.
    pub manifest: String,
    /// Artifact label.
    pub label: String,
}

/// A fully validated run configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunConfig {
    /// Training configuration.
    pub train: TrainConfig,
    /// Outputs.
    pub output: OutputConfig,
}

struct Doc(Document);

impl Doc {
    fn get(&self, s: &str, k: &str) -> Option<&(Value, usize)> {
        self.0.get(&(s.to_string(), k.to_string()))
    }
    fn missing(s: &str, k: &str) -> ConfigFileError {
        ConfigFileError {
            line: None,
            message: format!("missing required key `{k}` in [{s}]"),
        }
    }
    fn int(&self, s: &str, k: &str, lo: i64, hi: i64) -> Result<Option<i64>, ConfigFileError> {
        match self.get(s, k) {
            None => Ok(None),
            Some((Value::Int(v), line)) => {
                if *v < lo || *v > hi {
                    Err(err(*line, format!("`{k}` must be in {lo}..={hi}, got {v}")))
                } else {
                    Ok(Some(*v))
                }
            }
            Some((_, line)) => Err(err(*line, format!("`{k}` must be an integer"))),
        }
    }
    fn req_int(&self, s: &str, k: &str, lo: i64, hi: i64) -> Result<i64, ConfigFileError> {
        self.int(s, k, lo, hi)?.ok_or_else(|| Self::missing(s, k))
    }
    fn boolean(&self, s: &str, k: &str) -> Result<bool, ConfigFileError> {
        match self.get(s, k) {
            None => Ok(false),
            Some((Value::Bool(b), _)) => Ok(*b),
            Some((_, line)) => Err(err(*line, format!("`{k}` must be true or false"))),
        }
    }
    fn string(&self, s: &str, k: &str) -> Result<Option<String>, ConfigFileError> {
        match self.get(s, k) {
            None => Ok(None),
            Some((Value::Str(v), _)) => Ok(Some(v.clone())),
            Some((_, line)) => Err(err(*line, format!("`{k}` must be a quoted string"))),
        }
    }
    fn req_string(&self, s: &str, k: &str) -> Result<String, ConfigFileError> {
        self.string(s, k)?.ok_or_else(|| Self::missing(s, k))
    }
}

fn file_name(s: String, k: &str) -> Result<String, ConfigFileError> {
    if s.is_empty() || s.contains('/') || s.contains('\\') || s == "." || s == ".." {
        return Err(ConfigFileError {
            line: None,
            message: format!("[output] {k} must be a plain file name, got {s:?}"),
        });
    }
    Ok(s)
}

impl RunConfig {
    /// Parse and validate a config document.
    pub fn parse(text: &str) -> Result<Self, ConfigFileError> {
        let d = Doc(parse_document(text)?);
        let u32max = i64::from(u32::MAX);
        let depth = d.req_int("rules", "depth_bb", 1, 1_000_000)? as u32;
        let sb = d.int("rules", "small_blind", 1, 1_000_000)?.unwrap_or(1) as u32;
        let bb = d.int("rules", "big_blind", 1, 1_000_000)?.unwrap_or(2) as u32;
        let stack = depth.checked_mul(bb).ok_or_else(|| ConfigFileError {
            line: None,
            message: "depth_bb * big_blind overflows".into(),
        })?;
        let rules = RulesConfig::new(sb, bb, stack).map_err(|e| ConfigFileError {
            line: None,
            message: e.to_string(),
        })?;

        let mut abs = AbstractionConfig::default();
        let b = |k| d.int("abstraction", k, 1, 1024);
        if let Some(v) = b("flop_buckets")? {
            abs.flop_buckets = v as u16;
        }
        if let Some(v) = b("turn_buckets")? {
            abs.turn_buckets = v as u16;
        }
        if let Some(v) = b("river_buckets")? {
            abs.river_buckets = v as u16;
        }
        if let Some(v) = d.int("abstraction", "flop_samples", 1, 1_000_000)? {
            abs.flop_samples = v as u32;
        }
        if let Some(v) = d.int("abstraction", "turn_samples", 0, 1_000_000)? {
            abs.turn_samples = v as u32;
        }
        if let Some(v) = d.int("abstraction", "seed", 0, i64::MAX)? {
            abs.seed = v as u64;
        }
        abs.validate().map_err(|e| ConfigFileError {
            line: None,
            message: e.to_string(),
        })?;

        let seed = d.req_int("train", "seed", 0, i64::MAX)? as u64;
        let iterations = d.req_int("train", "iterations", 1, MAX_CONFIG_ITERATIONS as i64)? as u64;
        let mut train = TrainConfig::new(rules, abs, seed, iterations);
        train.checkpoint_every = d
            .int("train", "checkpoint_every", 1, MAX_CONFIG_ITERATIONS as i64)?
            .map(|v| v as u64);
        train.max_wall_secs = d
            .int("train", "max_wall_secs", 1, MAX_WALL_SECS as i64)?
            .map(|v| v as u64);
        train.weighting = Weighting {
            linear_averaging: d.boolean("train", "linear_averaging")?,
            linear_regrets: d.boolean("train", "linear_regrets")?,
        };
        train.update = UpdateConfig {
            regret_matching_plus: d.boolean("train", "regret_matching_plus")?,
        };
        if let Some(v) = d.int("train", "cache_capacity", 0, u32max)? {
            train.cache_capacity = v as usize;
        }

        let output = OutputConfig {
            dir: d
                .string("output", "dir")?
                .unwrap_or_else(|| "artifacts".into()),
            artifact: file_name(d.req_string("output", "artifact")?, "artifact")?,
            checkpoint: file_name(d.req_string("output", "checkpoint")?, "checkpoint")?,
            manifest: file_name(d.req_string("output", "manifest")?, "manifest")?,
            label: d.req_string("output", "label")?,
        };
        Ok(Self { train, output })
    }
}

//! Strict parser for evaluation configs (`configs/eval*.toml`).
//!
//! Supported TOML subset: `[section]` headers, `key = value`, `#` comments,
//! blank lines. Values: decimal integers (`_` allowed), `true`/`false`,
//! double-quoted strings (escapes `\"` `\\`), and single-line arrays of
//! strings `["a", "b"]`. Unknown sections/keys, duplicates, wrong types and
//! malformed lines are errors with a 1-based line number.
//!
//! Schema (`*` = required):
//! ```text
//! [policy]  artifact*, expected_checksum ("0x..." hex string), agent_seed (0)
//! [rules]   depth_bb*, small_blind (1), big_blind (2)
//! [eval]    base_seed*, deal_pairs*, opponents* (subset of random, checkcall,
//!           tight, mirror, cfr), mode ("promotion" | "comparison"; default
//!           "promotion"), candidates (subset of cfr, adaptive_cfr; default
//!           ["cfr"]), sanity_random_vs_checkcall (true),
//!           bootstrap_resamples (0 = off), bootstrap_seed (0),
//!           max_actions_per_hand (512)
//! [adaptive] (required iff a candidate is adaptive_cfr) enabled*,
//!           min_samples (20), max_shift_bp (1000), fold_threshold_bp (6000),
//!           station_threshold_bp (6000), seed (0)
//! [gate]    required in promotion mode, forbidden in comparison mode:
//!           min_hands*, require_positive_ci_vs*, max_fallback_rate_bp*
//!           (basis points, 100 = 1%), require_pinned_checksum (true)
//! ```
//!
//! Promotion mode gates the fixed `cfr` candidate. Comparison mode only
//! reports (e.g. fixed vs adaptive) and never yields a verdict.

use std::collections::BTreeMap;
use std::fmt;

use pokerbot_analysis::opponent::AdaptConfig;
use pokerbot_core::rules::RulesConfig;

use crate::gate::GateConfig;
use crate::matchup::{DEFAULT_MAX_ACTIONS, MAX_PAIRS};

/// Known opponent ids. `cfr` is a fixed CFR opponent (for head-to-head
/// against `adaptive_cfr`); `mirror` is the candidate's own kind.
pub const OPPONENTS: &[&str] = &["random", "checkcall", "tight", "mirror", "cfr"];
/// Known candidate kinds.
pub const CANDIDATES: &[&str] = &["cfr", "adaptive_cfr"];

/// What the run is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Gate the fixed CFR candidate (PROMOTE/HOLD).
    Promotion,
    /// Report a comparison only; no verdict.
    Comparison,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Value {
    Int(i64),
    Bool(bool),
    Str(String),
    List(Vec<String>),
}

/// Config error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalConfigError {
    pub line: Option<usize>,
    pub message: String,
}

impl fmt::Display for EvalConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(l) => write!(f, "eval config line {l}: {}", self.message),
            None => write!(f, "eval config: {}", self.message),
        }
    }
}

impl std::error::Error for EvalConfigError {}

fn err(line: usize, m: impl Into<String>) -> EvalConfigError {
    EvalConfigError {
        line: Some(line),
        message: m.into(),
    }
}

fn gerr(m: impl Into<String>) -> EvalConfigError {
    EvalConfigError {
        line: None,
        message: m.into(),
    }
}

const SCHEMA: &[(&str, &[&str])] = &[
    ("policy", &["artifact", "expected_checksum", "agent_seed"]),
    (
        "adaptive",
        &[
            "enabled",
            "min_samples",
            "max_shift_bp",
            "fold_threshold_bp",
            "station_threshold_bp",
            "seed",
        ],
    ),
    ("rules", &["depth_bb", "small_blind", "big_blind"]),
    (
        "eval",
        &[
            "base_seed",
            "deal_pairs",
            "opponents",
            "mode",
            "candidates",
            "sanity_random_vs_checkcall",
            "bootstrap_resamples",
            "bootstrap_seed",
            "max_actions_per_hand",
        ],
    ),
    (
        "gate",
        &[
            "min_hands",
            "require_positive_ci_vs",
            "max_fallback_rate_bp",
            "require_pinned_checksum",
        ],
    ),
];

/// Parses a quoted string starting after the opening quote; returns the
/// string and the remaining text.
fn parse_str(s: &str, line: usize) -> Result<(String, &str), EvalConfigError> {
    let mut out = String::new();
    let mut it = s.char_indices();
    while let Some((i, c)) = it.next() {
        match c {
            '"' => return Ok((out, &s[i + 1..])),
            '\\' => match it.next() {
                Some((_, '"')) => out.push('"'),
                Some((_, '\\')) => out.push('\\'),
                _ => return Err(err(line, "invalid escape in string")),
            },
            c => out.push(c),
        }
    }
    Err(err(line, "unterminated string"))
}

fn end_of_value(rest: &str, line: usize) -> Result<(), EvalConfigError> {
    let r = rest.trim();
    if r.is_empty() || r.starts_with('#') {
        Ok(())
    } else {
        Err(err(line, format!("unexpected text after value: {r:?}")))
    }
}

fn parse_value(raw: &str, line: usize) -> Result<Value, EvalConfigError> {
    let raw = raw.trim_start();
    if let Some(s) = raw.strip_prefix('"') {
        let (v, rest) = parse_str(s, line)?;
        end_of_value(rest, line)?;
        return Ok(Value::Str(v));
    }
    if let Some(mut s) = raw.strip_prefix('[') {
        let mut items = Vec::new();
        loop {
            s = s.trim_start();
            if let Some(rest) = s.strip_prefix(']') {
                end_of_value(rest, line)?;
                return Ok(Value::List(items));
            }
            let Some(q) = s.strip_prefix('"') else {
                return Err(err(line, "arrays may only contain quoted strings"));
            };
            let (v, rest) = parse_str(q, line)?;
            items.push(v);
            s = rest.trim_start();
            if let Some(r) = s.strip_prefix(',') {
                s = r;
            } else if !s.starts_with(']') {
                return Err(err(line, "expected `,` or `]` in array"));
            }
        }
    }
    let end = raw.find('#').unwrap_or(raw.len());
    let tok = raw[..end].trim();
    match tok {
        "true" => Ok(Value::Bool(true)),
        "false" => Ok(Value::Bool(false)),
        "" => Err(err(line, "missing value")),
        t => {
            let digits = t.replace('_', "");
            let body = digits.strip_prefix('-').unwrap_or(&digits);
            if body.is_empty() || !body.chars().all(|c| c.is_ascii_digit()) {
                return Err(err(line, format!("invalid value {t:?}")));
            }
            digits
                .parse::<i64>()
                .map(Value::Int)
                .map_err(|_| err(line, format!("integer out of range: {t}")))
        }
    }
}

type Document = BTreeMap<(String, String), (Value, usize)>;

fn parse_document(text: &str) -> Result<Document, EvalConfigError> {
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
            end_of_value(&h[end + 1..], line)?;
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

struct Doc(Document);

impl Doc {
    fn get(&self, s: &str, k: &str) -> Option<&(Value, usize)> {
        self.0.get(&(s.to_string(), k.to_string()))
    }
    fn missing(s: &str, k: &str) -> EvalConfigError {
        gerr(format!("missing required key `{k}` in [{s}]"))
    }
    fn int(&self, s: &str, k: &str, lo: i64, hi: i64) -> Result<Option<i64>, EvalConfigError> {
        match self.get(s, k) {
            None => Ok(None),
            Some((Value::Int(v), _)) if (lo..=hi).contains(v) => Ok(Some(*v)),
            Some((Value::Int(v), l)) => {
                Err(err(*l, format!("`{k}` must be in {lo}..={hi}, got {v}")))
            }
            Some((_, l)) => Err(err(*l, format!("`{k}` must be an integer"))),
        }
    }
    fn req_int(&self, s: &str, k: &str, lo: i64, hi: i64) -> Result<i64, EvalConfigError> {
        self.int(s, k, lo, hi)?.ok_or_else(|| Self::missing(s, k))
    }
    fn boolean(&self, s: &str, k: &str, default: bool) -> Result<bool, EvalConfigError> {
        match self.get(s, k) {
            None => Ok(default),
            Some((Value::Bool(b), _)) => Ok(*b),
            Some((_, l)) => Err(err(*l, format!("`{k}` must be true or false"))),
        }
    }
    fn string(&self, s: &str, k: &str) -> Result<Option<String>, EvalConfigError> {
        match self.get(s, k) {
            None => Ok(None),
            Some((Value::Str(v), _)) => Ok(Some(v.clone())),
            Some((_, l)) => Err(err(*l, format!("`{k}` must be a quoted string"))),
        }
    }
    fn list(
        &self,
        s: &str,
        k: &str,
        known: &[&str],
    ) -> Result<Option<Vec<String>>, EvalConfigError> {
        match self.get(s, k) {
            None => Ok(None),
            Some((Value::List(v), l)) => {
                for item in v {
                    if !known.contains(&item.as_str()) {
                        return Err(err(
                            *l,
                            format!("unknown entry {item:?} in `{k}`; known: {known:?}"),
                        ));
                    }
                }
                let mut seen = v.clone();
                seen.sort();
                seen.dedup();
                if seen.len() != v.len() {
                    return Err(err(*l, format!("`{k}` has duplicate entries")));
                }
                Ok(Some(v.clone()))
            }
            Some((_, l)) => Err(err(*l, format!("`{k}` must be an array of strings"))),
        }
    }
}

/// Parses `"0x..."` (or decimal) into a u64.
pub fn parse_checksum(s: &str) -> Option<u64> {
    let t = s.trim();
    match t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        Some(h) if !h.is_empty() && h.len() <= 16 => u64::from_str_radix(h, 16).ok(),
        Some(_) => None,
        None => t.parse().ok(),
    }
}

/// A validated evaluation config.
#[derive(Clone, Debug, PartialEq)]
pub struct EvalConfig {
    pub artifact: String,
    pub expected_checksum: Option<u64>,
    pub agent_seed: u64,
    pub rules: RulesConfig,
    pub depth_bb: u32,
    pub base_seed: u64,
    pub deal_pairs: u64,
    pub opponents: Vec<String>,
    pub sanity_random_vs_checkcall: bool,
    pub bootstrap_resamples: u32,
    pub bootstrap_seed: u64,
    pub max_actions_per_hand: u32,
    pub mode: Mode,
    pub candidates: Vec<String>,
    /// Adaptive-agent settings (present iff `adaptive_cfr` is a candidate).
    pub adaptive: Option<AdaptConfig>,
    /// Gate thresholds (promotion mode only).
    pub gate: Option<GateConfig>,
}

impl EvalConfig {
    /// Parse and validate.
    pub fn parse(text: &str) -> Result<Self, EvalConfigError> {
        let d = Doc(parse_document(text)?);
        let artifact = d
            .string("policy", "artifact")?
            .ok_or_else(|| Doc::missing("policy", "artifact"))?;
        if artifact.is_empty() {
            return Err(gerr("[policy] artifact must not be empty"));
        }
        let expected_checksum =
            match d.string("policy", "expected_checksum")? {
                None => None,
                Some(s) => Some(parse_checksum(&s).ok_or_else(|| {
                    gerr(format!("[policy] expected_checksum {s:?} is not a u64"))
                })?),
            };
        let agent_seed = d.int("policy", "agent_seed", 0, i64::MAX)?.unwrap_or(0) as u64;

        let depth = d.req_int("rules", "depth_bb", 1, 1_000_000)? as u32;
        let sb = d.int("rules", "small_blind", 1, 1_000_000)?.unwrap_or(1) as u32;
        let bb = d.int("rules", "big_blind", 1, 1_000_000)?.unwrap_or(2) as u32;
        let stack = depth
            .checked_mul(bb)
            .ok_or_else(|| gerr("depth_bb * big_blind overflows"))?;
        let rules = RulesConfig::new(sb, bb, stack).map_err(|e| gerr(e.to_string()))?;

        let base_seed = d.req_int("eval", "base_seed", 0, i64::MAX)? as u64;
        let deal_pairs = d.req_int("eval", "deal_pairs", 1, MAX_PAIRS as i64)? as u64;
        let opponents = d
            .list("eval", "opponents", OPPONENTS)?
            .ok_or_else(|| Doc::missing("eval", "opponents"))?;
        if opponents.is_empty() {
            return Err(gerr("[eval] opponents must not be empty"));
        }
        let candidates = d
            .list("eval", "candidates", CANDIDATES)?
            .unwrap_or_else(|| vec!["cfr".into()]);
        if candidates.is_empty() {
            return Err(gerr("[eval] candidates must not be empty"));
        }
        let mode = match d.string("eval", "mode")?.as_deref() {
            None | Some("promotion") => Mode::Promotion,
            Some("comparison") => Mode::Comparison,
            Some(m) => {
                return Err(gerr(format!(
                    "[eval] mode must be \"promotion\" or \"comparison\", got {m:?}"
                )));
            }
        };
        let sanity = d.boolean("eval", "sanity_random_vs_checkcall", true)?;
        let bootstrap_resamples = d
            .int("eval", "bootstrap_resamples", 0, 100_000)?
            .unwrap_or(0) as u32;
        let bootstrap_seed = d.int("eval", "bootstrap_seed", 0, i64::MAX)?.unwrap_or(0) as u64;
        let max_actions = d
            .int("eval", "max_actions_per_hand", 1, 100_000)?
            .unwrap_or(i64::from(DEFAULT_MAX_ACTIONS)) as u32;

        let has_adaptive_keys = d.0.keys().any(|(sec, _)| sec == "adaptive");
        let adaptive = if candidates.iter().any(|c| c == "adaptive_cfr") {
            if d.get("adaptive", "enabled").is_none() {
                return Err(gerr(
                    "candidate \"adaptive_cfr\" needs an [adaptive] section with an explicit `enabled`",
                ));
            }
            let def = AdaptConfig::default();
            let bp = |k, dflt: f64| -> Result<f64, EvalConfigError> {
                Ok(d.int("adaptive", k, 0, 10_000)?
                    .map_or(dflt, |v| v as f64 / 10_000.0))
            };
            let a = AdaptConfig {
                enabled: d.boolean("adaptive", "enabled", true)?,
                min_samples: d
                    .int("adaptive", "min_samples", 1, i64::MAX)?
                    .map_or(def.min_samples, |v| v as u64),
                max_shift: bp("max_shift_bp", def.max_shift)?,
                fold_threshold: bp("fold_threshold_bp", def.fold_threshold)?,
                station_threshold: bp("station_threshold_bp", def.station_threshold)?,
                seed: d
                    .int("adaptive", "seed", 0, i64::MAX)?
                    .map_or(def.seed, |v| v as u64),
            };
            a.validate().map_err(|e| gerr(format!("[adaptive] {e}")))?;
            Some(a)
        } else {
            if has_adaptive_keys {
                return Err(gerr(
                    "[adaptive] is set but no candidate is \"adaptive_cfr\"",
                ));
            }
            None
        };

        let has_gate_keys = d.0.keys().any(|(sec, _)| sec == "gate");
        let gate = match mode {
            Mode::Comparison => {
                if has_gate_keys {
                    return Err(gerr(
                        "[gate] is not allowed in comparison mode (a comparison is not a promotion)",
                    ));
                }
                None
            }
            Mode::Promotion => {
                if !candidates.iter().any(|c| c == "cfr") {
                    return Err(gerr("promotion mode needs the \"cfr\" candidate"));
                }
                let min_hands = d.req_int("gate", "min_hands", 1, i64::MAX)? as u64;
                let require = d
                    .list("gate", "require_positive_ci_vs", OPPONENTS)?
                    .ok_or_else(|| Doc::missing("gate", "require_positive_ci_vs"))?;
                for r in &require {
                    if !opponents.contains(r) {
                        return Err(gerr(format!(
                            "[gate] requires {r:?}, which is not in [eval] opponents"
                        )));
                    }
                    if r == "mirror" || r == "cfr" {
                        return Err(gerr(format!(
                            "[gate] cannot require a positive result vs {r:?}"
                        )));
                    }
                }
                let fb = d.req_int("gate", "max_fallback_rate_bp", 0, 10_000)?;
                Some(GateConfig {
                    min_hands,
                    require_positive_ci_vs: require,
                    max_fallback_rate: fb as f64 / 10_000.0,
                    require_pinned_checksum: d.boolean("gate", "require_pinned_checksum", true)?,
                })
            }
        };

        Ok(Self {
            artifact,
            expected_checksum,
            agent_seed,
            rules,
            depth_bb: depth,
            base_seed,
            deal_pairs,
            opponents,
            sanity_random_vs_checkcall: sanity,
            bootstrap_resamples,
            bootstrap_seed,
            max_actions_per_hand: max_actions,
            mode,
            candidates,
            adaptive,
            gate,
        })
    }
}

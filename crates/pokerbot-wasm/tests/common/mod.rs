//! Shared test helpers: a strict minimal JSON parser (validator), policy
//! artifact fixtures, and a seeded random legal player.

#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::AtomicBool;

use pokerbot_cfr::artifact::{ArtifactMetadata, StrategyArtifact};
use pokerbot_cfr::train::{TrainConfig, train};
use pokerbot_core::rng::PokerbotRng;
use pokerbot_wasm::GameSession;

// ------------------------------------------------------------------ JSON

/// A parsed JSON value.
#[derive(Clone, Debug, PartialEq)]
pub enum V {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<V>),
    Obj(Vec<(String, V)>),
}

impl V {
    pub fn get(&self, k: &str) -> &V {
        match self {
            V::Obj(f) => f
                .iter()
                .find(|(n, _)| n == k)
                .map(|(_, v)| v)
                .unwrap_or_else(|| panic!("missing key {k:?} in {self:?}")),
            _ => panic!("not an object: {self:?}"),
        }
    }
    pub fn has(&self, k: &str) -> bool {
        matches!(self, V::Obj(f) if f.iter().any(|(n, _)| n == k))
    }
    pub fn s(&self) -> &str {
        match self {
            V::Str(s) => s,
            _ => panic!("not a string: {self:?}"),
        }
    }
    pub fn n(&self) -> i64 {
        match self {
            V::Num(n) if n.fract() == 0.0 => *n as i64,
            _ => panic!("not an integer: {self:?}"),
        }
    }
    pub fn a(&self) -> &[V] {
        match self {
            V::Arr(a) => a,
            _ => panic!("not an array: {self:?}"),
        }
    }
    pub fn is_null(&self) -> bool {
        matches!(self, V::Null)
    }
}

/// Parses strict RFC 8259 JSON; panics with position on any error.
pub fn parse(text: &str) -> V {
    let mut p = P {
        b: text.as_bytes(),
        i: 0,
    };
    p.ws();
    let v = p.value();
    p.ws();
    assert_eq!(p.i, p.b.len(), "trailing data at {} in {text}", p.i);
    v
}

struct P<'a> {
    b: &'a [u8],
    i: usize,
}

impl P<'_> {
    fn peek(&self) -> u8 {
        *self
            .b
            .get(self.i)
            .unwrap_or_else(|| panic!("unexpected end"))
    }
    fn eat(&mut self, c: u8) {
        assert_eq!(self.peek(), c, "expected {:?} at {}", c as char, self.i);
        self.i += 1;
    }
    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\n' | b'\r' | b'\t') {
            self.i += 1;
        }
    }
    fn lit(&mut self, s: &str, v: V) -> V {
        assert!(
            self.b[self.i..].starts_with(s.as_bytes()),
            "bad literal at {}",
            self.i
        );
        self.i += s.len();
        v
    }
    fn value(&mut self) -> V {
        match self.peek() {
            b'n' => self.lit("null", V::Null),
            b't' => self.lit("true", V::Bool(true)),
            b'f' => self.lit("false", V::Bool(false)),
            b'"' => V::Str(self.string()),
            b'[' => {
                self.eat(b'[');
                let mut a = Vec::new();
                self.ws();
                if self.peek() == b']' {
                    self.i += 1;
                    return V::Arr(a);
                }
                loop {
                    self.ws();
                    a.push(self.value());
                    self.ws();
                    match self.peek() {
                        b',' => self.i += 1,
                        b']' => {
                            self.i += 1;
                            return V::Arr(a);
                        }
                        c => panic!("bad array char {:?} at {}", c as char, self.i),
                    }
                }
            }
            b'{' => {
                self.eat(b'{');
                let mut f: Vec<(String, V)> = Vec::new();
                self.ws();
                if self.peek() == b'}' {
                    self.i += 1;
                    return V::Obj(f);
                }
                loop {
                    self.ws();
                    let k = self.string();
                    assert!(f.iter().all(|(n, _)| *n != k), "duplicate key {k}");
                    self.ws();
                    self.eat(b':');
                    self.ws();
                    let v = self.value();
                    f.push((k, v));
                    self.ws();
                    match self.peek() {
                        b',' => self.i += 1,
                        b'}' => {
                            self.i += 1;
                            return V::Obj(f);
                        }
                        c => panic!("bad object char {:?} at {}", c as char, self.i),
                    }
                }
            }
            _ => self.number(),
        }
    }
    fn number(&mut self) -> V {
        let st = self.i;
        if self.peek() == b'-' {
            self.i += 1;
        }
        let digits = |p: &mut Self| {
            let s = p.i;
            while p.i < p.b.len() && p.b[p.i].is_ascii_digit() {
                p.i += 1;
            }
            assert!(p.i > s, "expected digits at {s}");
        };
        if self.peek() == b'0' {
            self.i += 1;
        } else {
            digits(self);
        }
        if self.i < self.b.len() && self.b[self.i] == b'.' {
            self.i += 1;
            digits(self);
        }
        if self.i < self.b.len() && matches!(self.b[self.i], b'e' | b'E') {
            self.i += 1;
            if matches!(self.peek(), b'+' | b'-') {
                self.i += 1;
            }
            digits(self);
        }
        let s = std::str::from_utf8(&self.b[st..self.i]).unwrap();
        V::Num(s.parse().unwrap())
    }
    fn string(&mut self) -> String {
        self.eat(b'"');
        let mut out = String::new();
        loop {
            let c = self.peek();
            match c {
                b'"' => {
                    self.i += 1;
                    return out;
                }
                b'\\' => {
                    self.i += 1;
                    let e = self.peek();
                    self.i += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let h = std::str::from_utf8(&self.b[self.i..self.i + 4]).unwrap();
                            self.i += 4;
                            let cp = u32::from_str_radix(h, 16).unwrap();
                            out.push(char::from_u32(cp).expect("no surrogates emitted"));
                        }
                        _ => panic!("bad escape at {}", self.i),
                    }
                }
                c if c < 0x20 => panic!("raw control char in string at {}", self.i),
                _ => {
                    // Copy one UTF-8 scalar.
                    let rest = std::str::from_utf8(&self.b[self.i..]).unwrap();
                    let ch = rest.chars().next().unwrap();
                    out.push(ch);
                    self.i += ch.len_utf8();
                }
            }
        }
    }
}

// -------------------------------------------------------------- fixtures

pub const TINY_LABEL: &str = "test: tiny wasm fixture, no strength claim";

/// A small 20BB artifact trained in-process (cached per test binary).
pub fn tiny_artifact() -> &'static [u8] {
    static A: OnceLock<Vec<u8>> = OnceLock::new();
    A.get_or_init(|| {
        let cfg = TrainConfig::tiny(11, 3_000);
        let out = train(&cfg, None, &AtomicBool::new(false), &mut |_| {}).unwrap();
        let meta = ArtifactMetadata::from_trainer(&out.trainer, TINY_LABEL);
        StrategyArtifact::from_table(out.trainer.table(), meta)
            .unwrap()
            .to_bytes()
    })
}

/// `artifacts/smoke_20bb.pokerbot` at the workspace root, if present.
pub fn smoke_artifact_path() -> Option<PathBuf> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../artifacts/smoke_20bb.pokerbot");
    p.is_file().then_some(p)
}

/// The smoke artifact if present, else the tiny one; with a name.
pub fn policy_20bb() -> (&'static str, Vec<u8>) {
    match smoke_artifact_path() {
        Some(p) => ("artifacts/smoke_20bb.pokerbot", std::fs::read(p).unwrap()),
        None => {
            eprintln!("artifacts/smoke_20bb.pokerbot absent: using an in-process tiny artifact");
            ("in-process tiny artifact", tiny_artifact().to_vec())
        }
    }
}

/// Writes `bytes` to a unique temp file and returns its path.
pub fn temp_file(name: &str, bytes: &[u8]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pokerbot-wasm-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join(name);
    std::fs::write(&p, bytes).unwrap();
    p
}

// ---------------------------------------------------------------- driver

/// Picks a uniformly random legal action name from a snapshot.
pub fn random_action(snap: &V, rng: &mut PokerbotRng) -> String {
    let legal = snap.get("legalActions").a();
    assert!(!legal.is_empty(), "human to act but no legal actions");
    legal[rng.gen_range(legal.len() as u64) as usize]
        .get("type")
        .s()
        .to_string()
}

/// Plays the current hand to the end with random legal actions, checking
/// chip conservation at every snapshot; returns all snapshots seen.
pub fn play_hand(s: &mut GameSession, rng: &mut PokerbotRng) -> Vec<String> {
    let total = 2 * parse(&s.snapshot_json()).get("startingStack").n();
    let mut seen = vec![s.snapshot_json()];
    for _ in 0..500 {
        let snap = parse(seen.last().unwrap());
        let chips = snap.get("myStack").n() + snap.get("botStack").n() + snap.get("potChips").n();
        if snap.get("terminal").is_null() {
            assert_eq!(chips, total, "chip conservation mid-hand");
            assert_eq!(
                snap.get("actor").s(),
                "human",
                "bot turns run automatically"
            );
            let a = random_action(&snap, rng);
            s.step(&a).unwrap();
            seen.push(s.snapshot_json());
        } else {
            let p = snap.get("payoff");
            assert_eq!(
                p.get("human").n() + p.get("pokerbot").n(),
                0,
                "zero-sum payoff"
            );
            assert!(snap.get("legalActions").a().is_empty());
            assert!(snap.get("actor").is_null());
            return seen;
        }
    }
    panic!("hand did not finish");
}

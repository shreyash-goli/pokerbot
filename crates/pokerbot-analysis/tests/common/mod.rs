//! Shared helpers for pokerbot-analysis integration tests.
#![allow(dead_code, unused_imports)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use pokerbot_analysis::OpponentLabel;
use pokerbot_core::action::{ActionKind, Seat};
use pokerbot_core::card::{Card, parse_cards};
use pokerbot_core::deck::Deck;
use pokerbot_core::rng::PokerbotRng;
use pokerbot_core::rules::{RulesConfig, StackDepth};
use pokerbot_core::state::{HandConfig, HandState};

pub use ActionKind::{AllIn, Call, Check, Fold, HalfPot, Pot};

pub const HUMAN: Seat = Seat::Zero;
pub const BOT: Seat = Seat::One;

pub fn cfr_label() -> OpponentLabel {
    OpponentLabel::new("CFR policy smoke-20bb", Some(0x0123_4567_89ab_cdef))
}

pub fn config(button: Seat) -> HandConfig {
    HandConfig {
        rules: RulesConfig::standard(StackDepth::Bb100),
        button,
    }
}

/// Deck order for explicit cards: seat 0 hole, seat 1 hole, 5 board cards,
/// then every other card in index order.
pub fn order(button: Seat, seat0: &str, seat1: &str, board: &str) -> Vec<Card> {
    let h = [parse_cards(seat0).unwrap(), parse_cards(seat1).unwrap()];
    let b = parse_cards(board).unwrap();
    let btn = &h[button.index()];
    let bb = &h[button.other().index()];
    let mut o = vec![bb[0], btn[0], bb[1], btn[1]];
    o.extend_from_slice(&b);
    for c in Card::all() {
        if !o.contains(&c) {
            o.push(c);
        }
    }
    o
}

pub fn state_from_order(button: Seat, order: &[Card]) -> HandState {
    HandState::new(config(button), Deck::from_order(order.to_vec()).unwrap()).unwrap()
}

/// Plays `kinds` in order (each must be legal), returning the state.
pub fn play(mut st: HandState, kinds: &[ActionKind]) -> HandState {
    for &k in kinds {
        st.apply(k)
            .unwrap_or_else(|e| panic!("{k:?} rejected: {e}"));
    }
    st
}

pub fn hand(
    button: Seat,
    seat0: &str,
    seat1: &str,
    board: &str,
    kinds: &[ActionKind],
) -> HandState {
    play(
        state_from_order(button, &order(button, seat0, seat1, board)),
        kinds,
    )
}

/// Plays a random legal line (card-independent choices from `rng`).
pub fn play_random(mut st: HandState, rng: &mut PokerbotRng) -> HandState {
    for _ in 0..200 {
        if st.is_terminal() {
            return st;
        }
        let legal = st.legal_actions();
        let a = legal[rng.gen_range(legal.len() as u64) as usize];
        st.apply_legal(&a).unwrap();
    }
    panic!("hand did not terminate");
}

/// Plays a random legal line replaying fixed choice indices.
pub fn play_choices(mut st: HandState, choices: &[usize]) -> HandState {
    for &c in choices {
        let legal = st.legal_actions();
        st.apply_legal(&legal[c]).unwrap();
    }
    st
}

pub fn random_choices(st: &HandState, rng: &mut PokerbotRng) -> Vec<usize> {
    let mut st = st.clone();
    let mut out = Vec::new();
    while !st.is_terminal() {
        let legal = st.legal_actions();
        let c = rng.gen_range(legal.len() as u64) as usize;
        st.apply_legal(&legal[c]).unwrap();
        out.push(c);
    }
    out
}

/// A fresh, unique directory under the system temp dir. Tests never use
/// the real home directory.
pub fn temp_dir(tag: &str) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let d = std::env::temp_dir().join(format!(
        "pokerbot-analysis-test-{}-{}-{tag}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

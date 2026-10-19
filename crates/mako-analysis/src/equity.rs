//! Showdown equity for the coach (M-18), using only `mako_core`'s
//! evaluator.
//!
//! * [`vs_random`] — the hero's equity against a **uniformly random**
//!   opponent hand, from the hero's own cards and the visible board only.
//!   This is decision-time information, but it is *not* equity against the
//!   opponent's actual range.
//! * [`vs_hand`] — equity against a **known** opponent hand. Only meaningful
//!   in hindsight, after the opponent's cards were shown.
//!
//! Equity counts a win as 1 and a tie as ½ over all (or sampled) remaining
//! board runouts; it ignores any future betting. Small cases are enumerated
//! exactly; large ones are sampled with a fixed seed derived from the
//! known cards, so results are reproducible and labelled as estimates.

use std::cmp::Ordering;

use mako_core::card::{Card, CardSet};
use mako_core::eval::evaluate7;
use mako_core::rng::MakoRng;

/// Trials used when [`vs_random`] samples (preflop and flop).
pub const RANDOM_SAMPLES: u32 = 3_000;
/// Trials used when [`vs_hand`] samples (preflop only).
pub const KNOWN_SAMPLES: u32 = 5_000;

/// How an estimate was obtained.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Method {
    /// Every case enumerated.
    Exact,
    /// Monte Carlo with this many trials (seeded, reproducible).
    Sampled { trials: u32 },
}

/// Equity as `points / (2 · trials)`: a win scores 2 points, a tie 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EquityEstimate {
    pub points: u64,
    pub trials: u64,
    pub method: Method,
}

impl EquityEstimate {
    /// Equity in `[0, 1]`, or `None` with no trials.
    pub fn value(&self) -> Option<f64> {
        (self.trials > 0).then(|| self.points as f64 / (2 * self.trials) as f64)
    }

    /// E.g. `54.1% (exact)` or `54.1% (sampled estimate, 3000 trials)`.
    pub fn describe(&self) -> String {
        let pct = match self.value() {
            Some(v) => format!("{:.1}%", 100.0 * v),
            None => return "n/a".to_string(),
        };
        match self.method {
            Method::Exact => format!("{pct} (exact)"),
            Method::Sampled { trials } => format!("{pct} (sampled estimate, {trials} trials)"),
        }
    }
}

fn unseen(known: &[Card]) -> Vec<Card> {
    let set: CardSet = known.iter().copied().collect();
    Card::all().filter(|c| !set.contains(*c)).collect()
}

fn seven(hole: [Card; 2], board: &[Card; 5]) -> [Card; 7] {
    [
        hole[0], hole[1], board[0], board[1], board[2], board[3], board[4],
    ]
}

fn points(hero: [Card; 2], opp: [Card; 2], board: &[Card; 5]) -> u64 {
    match evaluate7(&seven(hero, board)).cmp(&evaluate7(&seven(opp, board))) {
        Ordering::Greater => 2,
        Ordering::Equal => 1,
        Ordering::Less => 0,
    }
}

/// Deterministic seed from the known cards (order-sensitive FNV-1a).
fn seed_of(cards: &[Card]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for c in cards {
        h ^= u64::from(c.index()) + 1;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

fn full_board(board: &[Card], extra: &[Card]) -> [Card; 5] {
    // Every slot is overwritten below; the initial value is a placeholder.
    let mut b = [Card::from_index(0).expect("valid index"); 5];
    b[..board.len()].copy_from_slice(board);
    b[board.len()..].copy_from_slice(&extra[..5 - board.len()]);
    b
}

fn valid(cards: &[Card], board_len: usize) -> bool {
    matches!(board_len, 0 | 3 | 4 | 5) && CardSet::try_from_cards(cards).is_ok()
}

/// Hero equity vs a uniformly random opponent hand. `board` must have 0, 3,
/// 4 or 5 cards; `None` on invalid input (duplicates, bad length).
pub fn vs_random(hole: [Card; 2], board: &[Card]) -> Option<EquityEstimate> {
    let mut known = hole.to_vec();
    known.extend_from_slice(board);
    if !valid(&known, board.len()) {
        return None;
    }
    let rest = unseen(&known);
    let (mut pts, mut trials) = (0u64, 0u64);
    match board.len() {
        5 | 4 => {
            let runouts: Vec<Vec<Card>> = if board.len() == 5 {
                vec![Vec::new()]
            } else {
                rest.iter().map(|&r| vec![r]).collect()
            };
            for extra in runouts {
                let b = if extra.is_empty() {
                    full_board(board, &[])
                } else {
                    full_board(board, &extra)
                };
                for i in 0..rest.len() {
                    if extra.contains(&rest[i]) {
                        continue;
                    }
                    for j in i + 1..rest.len() {
                        if extra.contains(&rest[j]) {
                            continue;
                        }
                        pts += points(hole, [rest[i], rest[j]], &b);
                        trials += 1;
                    }
                }
            }
            Some(EquityEstimate {
                points: pts,
                trials,
                method: Method::Exact,
            })
        }
        _ => Some(sample(hole, None, board, rest, RANDOM_SAMPLES, &known)),
    }
}

/// Hero equity vs the known opponent hand `opp` over the remaining runouts.
/// `None` on invalid input.
pub fn vs_hand(hole: [Card; 2], opp: [Card; 2], board: &[Card]) -> Option<EquityEstimate> {
    let mut known = hole.to_vec();
    known.extend_from_slice(&opp);
    known.extend_from_slice(board);
    if !valid(&known, board.len()) {
        return None;
    }
    let rest = unseen(&known);
    let need = 5 - board.len();
    if board.is_empty() {
        return Some(sample(hole, Some(opp), board, rest, KNOWN_SAMPLES, &known));
    }
    let (mut pts, mut trials) = (0u64, 0u64);
    let mut visit = |extra: &[Card]| {
        pts += points(hole, opp, &full_board(board, extra));
        trials += 1;
    };
    match need {
        0 => visit(&[]),
        1 => rest.iter().for_each(|&r| visit(&[r])),
        _ => {
            for i in 0..rest.len() {
                for j in i + 1..rest.len() {
                    visit(&[rest[i], rest[j]]);
                }
            }
        }
    }
    Some(EquityEstimate {
        points: pts,
        trials,
        method: Method::Exact,
    })
}

fn sample(
    hole: [Card; 2],
    opp: Option<[Card; 2]>,
    board: &[Card],
    mut rest: Vec<Card>,
    trials: u32,
    known: &[Card],
) -> EquityEstimate {
    let mut rng = MakoRng::seed_from_u64(seed_of(known));
    let need = 5 - board.len() + if opp.is_some() { 0 } else { 2 };
    let mut pts = 0u64;
    for _ in 0..trials {
        for k in 0..need {
            let j = k + rng.gen_range((rest.len() - k) as u64) as usize;
            rest.swap(k, j);
        }
        let (o, extra) = match opp {
            Some(o) => (o, &rest[..need]),
            None => ([rest[0], rest[1]], &rest[2..need]),
        };
        pts += points(hole, o, &full_board(board, extra));
    }
    EquityEstimate {
        points: pts,
        trials: u64::from(trials),
        method: Method::Sampled { trials },
    }
}

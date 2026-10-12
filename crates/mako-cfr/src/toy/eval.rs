//! Exact evaluation of Kuhn strategy profiles.
//!
//! Kuhn's tree is tiny (6 deals × 9 terminal histories), so expected value
//! and best-response values are computed exactly by enumeration.
//!
//! # Exploitability definition
//! Let `BR_0(σ_1)` be player 0's best-response value against `σ_1` and
//! `BR_1(σ_0)` player 1's against `σ_0`. Then
//! `NashConv(σ) = BR_0(σ_1) + BR_1(σ_0)` (≥ 0, zero exactly at equilibrium,
//! since the game is zero-sum) and we report
//! **`exploitability(σ) = NashConv(σ) / 2`**, in chips per hand, averaged over
//! both seats.

use super::kuhn::{Card, Deal, History, InfoSetId, NUM_ACTIONS, NUM_DEALS, NUM_INFOSETS};

/// A complete behavioural strategy profile for Kuhn poker: one distribution
/// over `[Pass, Bet]` per information set (indexed by [`InfoSetId`]).
#[derive(Debug, Clone, PartialEq)]
pub struct KuhnStrategy {
    /// `probs[id][a]`: probability of action `a` at information set `id`.
    pub probs: [[f64; NUM_ACTIONS]; NUM_INFOSETS],
}

impl KuhnStrategy {
    /// Uniform random profile.
    pub fn uniform() -> Self {
        Self {
            probs: [[0.5; NUM_ACTIONS]; NUM_INFOSETS],
        }
    }

    /// Distribution at an information set.
    pub fn get(&self, id: InfoSetId) -> [f64; NUM_ACTIONS] {
        self.probs[id.0]
    }

    /// Probability of `Bet` (bet or call) at `card` + `history`.
    pub fn bet_prob(&self, card: Card, history: &str) -> f64 {
        let h = History::parse(history);
        self.probs[InfoSetId::from_parts(card, h.decision_index()).0][1]
    }

    /// One member of the known equilibrium family (see [`crate::toy`] docs).
    ///
    /// # Panics
    /// If `alpha` is outside `[0, 1/3]`.
    pub fn equilibrium(alpha: f64) -> Self {
        assert!(
            (0.0..=1.0 / 3.0 + 1e-15).contains(&alpha),
            "alpha must be in [0, 1/3]"
        );
        let mut s = Self::uniform();
        let mut set = |card: Card, hist: &str, bet: f64| {
            let id = InfoSetId::from_parts(card, History::parse(hist).decision_index());
            s.probs[id.0] = [1.0 - bet, bet];
        };
        use Card::*;
        set(J, "", alpha);
        set(Q, "", 0.0);
        set(K, "", 3.0 * alpha);
        set(J, "pb", 0.0);
        set(Q, "pb", alpha + 1.0 / 3.0);
        set(K, "pb", 1.0);
        set(J, "p", 1.0 / 3.0);
        set(Q, "p", 0.0);
        set(K, "p", 1.0);
        set(J, "b", 0.0);
        set(Q, "b", 1.0 / 3.0);
        set(K, "b", 1.0);
        s
    }
}

/// Probability of each deal.
pub const DEAL_PROB: f64 = 1.0 / NUM_DEALS as f64;

/// Known game value for player 0 at any equilibrium.
pub const GAME_VALUE_P0: f64 = -1.0 / 18.0;

fn ev_rec(s: &KuhnStrategy, deal: Deal, h: History) -> f64 {
    if h.is_terminal() {
        return f64::from(h.terminal_utility_p0(deal.cards));
    }
    let p = h.player_to_act();
    let id = super::kuhn::info_key(&deal.observation(p, h));
    let sigma = s.get(id);
    let mut v = 0.0;
    for a in super::kuhn::Action::ALL {
        let pa = sigma[a.index()];
        if pa != 0.0 {
            v += pa * ev_rec(s, deal, h.child(a));
        }
    }
    v
}

/// Exact expected utility for player 0 when both players follow `s`.
pub fn expected_value_p0(s: &KuhnStrategy) -> f64 {
    Deal::all()
        .iter()
        .map(|&d| DEAL_PROB * ev_rec(s, d, History::root()))
        .sum()
}

/// `w[o]` = chance probability × opponent reach for opponent card `o`.
fn br_rec(s: &KuhnStrategy, br: usize, own: Card, h: History, w: [f64; 3]) -> f64 {
    let deal_for = |opp: Card| {
        let mut cards = [own, opp];
        if br == 1 {
            cards = [opp, own];
        }
        Deal { cards }
    };
    if h.is_terminal() {
        let sign = if br == 0 { 1.0 } else { -1.0 };
        return Card::ALL
            .iter()
            .filter(|&&o| w[o.index()] != 0.0)
            .map(|&o| w[o.index()] * sign * f64::from(h.terminal_utility_p0(deal_for(o).cards)))
            .sum();
    }
    let p = h.player_to_act();
    if p == br {
        // Best responder's information set is (own, h): choose one action
        // for all opponent cards jointly.
        super::kuhn::Action::ALL
            .iter()
            .map(|&a| br_rec(s, br, own, h.child(a), w))
            .fold(f64::NEG_INFINITY, f64::max)
    } else {
        let mut total = 0.0;
        for a in super::kuhn::Action::ALL {
            let mut w2 = [0.0; 3];
            for o in Card::ALL {
                if w[o.index()] != 0.0 {
                    let id = super::kuhn::info_key(&deal_for(o).observation(p, h));
                    w2[o.index()] = w[o.index()] * s.get(id)[a.index()];
                }
            }
            total += br_rec(s, br, own, h.child(a), w2);
        }
        total
    }
}

/// Exact best-response value for player `br` against the other player's
/// strategy in `s` (in chips for `br`).
pub fn best_response_value(s: &KuhnStrategy, br: usize) -> f64 {
    assert!(br < 2);
    Card::ALL
        .iter()
        .map(|&own| {
            let mut w = [DEAL_PROB; 3];
            w[own.index()] = 0.0;
            br_rec(s, br, own, History::root(), w)
        })
        .sum()
}

/// `BR_0(σ_1) + BR_1(σ_0)`.
pub fn nash_conv(s: &KuhnStrategy) -> f64 {
    best_response_value(s, 0) + best_response_value(s, 1)
}

/// `NashConv / 2` (see module docs).
pub fn exploitability(s: &KuhnStrategy) -> f64 {
    nash_conv(s) / 2.0
}

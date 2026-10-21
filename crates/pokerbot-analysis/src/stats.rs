//! Session statistics computed only from public decision history.
//!
//! [`SessionStats`] aggregates [`HandRecord`]s for both players (the human
//! and the opponent). Every input is public: the action list, blinds,
//! button, result and payoffs. Hole cards are never read, so the
//! opponent's unrevealed cards cannot influence any number here.
//!
//! # Definitions
//!
//! For a player `P` and a set of hands `H` (all hands, or only those where
//! `P` held the button / the big blind):
//!
//! * **hands** — `|H|`.
//! * **net chips** — `Σ payoff(P)` over `H` (integer chips). The two
//!   players' net chips always sum to zero.
//! * **BB/100** — `100 · Σ (payoff(P) / big_blind) / |H|`, where each hand's
//!   payoff is converted to big blinds with that hand's own big blind.
//!   `None` when `|H| = 0`.
//! * **preflop decision hands** `D` — hands in `H` where `P` made at least one
//!   voluntary preflop action. A walk (the button folds, so the big blind
//!   never acts) is excluded for the big blind.
//! * **VPIP** — `|{h ∈ D : P made a preflop Call, HalfPot, Pot or AllIn}| / |D|`.
//!   A button limp (calling the big blind) **is** VPIP; the big blind
//!   checking its option is **not** (a check adds no chips); blinds are
//!   forced and never count; a fold is not VPIP.
//! * **PFR** — `|{h ∈ D : P made a preflop HalfPot, Pot or AllIn}| / |D|`.
//!   An open-raise, 3-bet or all-in shove counts; a limp or call does not.
//!   (Every HalfPot/Pot/AllIn record is a real bet or raise: the engine only
//!   offers those when the actor has more chips than the call.)
//! * **bets+raises**, **calls**, **checks**, **folds** — counts of `P`'s
//!   actions of each class on all streets.
//! * **AF (aggression factor)** — `(bets + raises) / calls`; `None` when
//!   `calls = 0`.
//! * **AFq (aggression frequency)** — `(bets + raises) / (bets + raises +
//!   calls + folds)`; checks are excluded. `None` when the denominator is 0.
//! * **showdowns** — hands that ended with [`TerminalReason::Showdown`];
//!   **won** — showdowns with `payoff(P) > 0`; **split** — showdowns with
//!   `payoff(P) = 0`. **W$SD** = `won / showdowns`, `None` when no showdowns.
//!
//! Ratios are returned as `Option<f64>` fractions in `[0, 1]` (AF is
//! unbounded); a zero denominator yields `None` (shown as `n/a`), never NaN.

use pokerbot_core::action::{ActionKind, Seat, Street, TerminalReason};

use crate::history::HandRecord;

/// Which participant of the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    Human,
    Opponent,
}

impl Role {
    pub const ALL: [Role; 2] = [Role::Human, Role::Opponent];

    /// The seat this role occupied in `rec`.
    pub fn seat_in(self, rec: &HandRecord) -> Seat {
        match self {
            Role::Human => rec.human_seat,
            Role::Opponent => rec.human_seat.other(),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Role::Human => "You",
            Role::Opponent => "Opponent",
        }
    }
}

/// Counts over one subset of hands (overall, button-only or BB-only).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SplitStats {
    pub hands: u64,
    pub net_chips: i64,
    /// `Σ payoff / big_blind` (see module docs).
    pub net_big_blinds: f64,
    /// Hands with at least one voluntary preflop action.
    pub preflop_decision_hands: u64,
    pub vpip_hands: u64,
    pub pfr_hands: u64,
}

fn ratio(num: u64, den: u64) -> Option<f64> {
    (den > 0).then(|| num as f64 / den as f64)
}

impl SplitStats {
    /// Big blinds won per 100 hands; `None` with no hands.
    pub fn bb_per_100(&self) -> Option<f64> {
        (self.hands > 0).then(|| 100.0 * self.net_big_blinds / self.hands as f64)
    }

    /// VPIP fraction; `None` with no preflop decisions.
    pub fn vpip(&self) -> Option<f64> {
        ratio(self.vpip_hands, self.preflop_decision_hands)
    }

    /// PFR fraction; `None` with no preflop decisions.
    pub fn pfr(&self) -> Option<f64> {
        ratio(self.pfr_hands, self.preflop_decision_hands)
    }
}

/// All statistics for one player.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerStats {
    pub overall: SplitStats,
    /// Hands where this player held the button (small blind).
    pub button: SplitStats,
    /// Hands where this player posted the big blind.
    pub big_blind: SplitStats,
    pub bets_raises: u64,
    pub calls: u64,
    pub checks: u64,
    pub folds: u64,
    pub showdowns: u64,
    pub showdowns_won: u64,
    pub showdowns_split: u64,
}

impl PlayerStats {
    /// `(bets + raises) / calls`; `None` when there are no calls.
    pub fn aggression_factor(&self) -> Option<f64> {
        ratio(self.bets_raises, self.calls)
    }

    /// `(bets + raises) / (bets + raises + calls + folds)`.
    pub fn aggression_frequency(&self) -> Option<f64> {
        ratio(self.bets_raises, self.bets_raises + self.calls + self.folds)
    }

    /// Showdowns won / showdowns.
    pub fn showdown_win_rate(&self) -> Option<f64> {
        ratio(self.showdowns_won, self.showdowns)
    }

    fn add(&mut self, rec: &HandRecord, seat: Seat) {
        let payoff = rec.payoffs[seat.index()];
        let mut decided = false;
        let mut vpip = false;
        let mut pfr = false;
        for a in rec.actions.iter().filter(|a| a.seat == seat) {
            match a.kind {
                ActionKind::Fold => self.folds += 1,
                ActionKind::Check => self.checks += 1,
                ActionKind::Call => self.calls += 1,
                ActionKind::HalfPot | ActionKind::Pot | ActionKind::AllIn => self.bets_raises += 1,
            }
            if a.street == Street::Preflop {
                decided = true;
                vpip |= a.kind == ActionKind::Call || a.kind.is_aggressive();
                pfr |= a.kind.is_aggressive();
            }
        }
        let bb = f64::from(rec.rules.big_blind.max(1));
        let split = if seat == rec.button {
            &mut self.button
        } else {
            &mut self.big_blind
        };
        for s in [&mut self.overall, split] {
            s.hands += 1;
            s.net_chips += payoff;
            s.net_big_blinds += payoff as f64 / bb;
            s.preflop_decision_hands += u64::from(decided);
            s.vpip_hands += u64::from(vpip);
            s.pfr_hands += u64::from(pfr);
        }
        if rec.reason == TerminalReason::Showdown {
            self.showdowns += 1;
            match payoff.cmp(&0) {
                std::cmp::Ordering::Greater => self.showdowns_won += 1,
                std::cmp::Ordering::Equal => self.showdowns_split += 1,
                std::cmp::Ordering::Less => {}
            }
        }
    }
}

/// Statistics for the whole session, both players.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SessionStats {
    pub human: PlayerStats,
    pub opponent: PlayerStats,
}

impl SessionStats {
    /// Empty statistics.
    pub fn new() -> SessionStats {
        SessionStats::default()
    }

    /// Statistics over `records`, in order.
    pub fn from_records(records: &[HandRecord]) -> SessionStats {
        let mut s = SessionStats::new();
        for r in records {
            s.add(r);
        }
        s
    }

    /// Adds one hand.
    pub fn add(&mut self, rec: &HandRecord) {
        self.human.add(rec, Role::Human.seat_in(rec));
        self.opponent.add(rec, Role::Opponent.seat_in(rec));
    }

    /// The stats of `role`.
    pub fn player(&self, role: Role) -> &PlayerStats {
        match role {
            Role::Human => &self.human,
            Role::Opponent => &self.opponent,
        }
    }

    /// Hands recorded.
    pub fn hands(&self) -> u64 {
        self.human.overall.hands
    }

    /// Plain-text summary, one line per entry, for terminal display.
    pub fn summary_lines(&self) -> Vec<String> {
        let mut out = vec![format!("Hands played: {}", self.hands())];
        for role in Role::ALL {
            let p = self.player(role);
            let o = &p.overall;
            out.push(format!(
                "{:<8} net {:+} chips | {} BB/100 | VPIP {} | PFR {} | AF {} | AFq {}",
                role.name(),
                o.net_chips,
                fmt_num(o.bb_per_100()),
                fmt_pct(o.vpip()),
                fmt_pct(o.pfr()),
                fmt_num(p.aggression_factor()),
                fmt_pct(p.aggression_frequency()),
            ));
            out.push(format!(
                "{:<8} showdowns {} (won {}, split {}, W$SD {})",
                "",
                p.showdowns,
                p.showdowns_won,
                p.showdowns_split,
                fmt_pct(p.showdown_win_rate())
            ));
            for (label, s) in [("button", &p.button), ("big blind", &p.big_blind)] {
                out.push(format!(
                    "{:<8} {label:<9} {} hands, net {:+}, VPIP {}, PFR {}",
                    "",
                    s.hands,
                    s.net_chips,
                    fmt_pct(s.vpip()),
                    fmt_pct(s.pfr())
                ));
            }
        }
        out
    }
}

/// Formats a fraction as a percentage with one decimal, or `n/a`.
pub fn fmt_pct(v: Option<f64>) -> String {
    match v {
        Some(x) if x.is_finite() => format!("{:.1}%", 100.0 * x),
        _ => "n/a".to_string(),
    }
}

/// Formats a number with two decimals, or `n/a`.
pub fn fmt_num(v: Option<f64>) -> String {
    match v {
        Some(x) if x.is_finite() => format!("{x:.2}"),
        _ => "n/a".to_string(),
    }
}

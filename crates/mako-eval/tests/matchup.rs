//! Match runner: pairing, zero-sum, mirror antisymmetry, information boundary.

mod common;

use mako_core::deck::Deck;
use mako_core::observation::PlayerObservation;
use mako_core::rules::{RulesConfig, StackDepth};
use mako_core::state::Seat;
use mako_eval::agent::{Agent, AgentError, CfrAgent, Decision};
use mako_eval::baselines::{CheckCall, RandomLegal, TightRule};
use mako_eval::matchup::{
    DEFAULT_MAX_ACTIONS, DecisionCounts, MatchSpec, deal_seed, play_pair, run_match, sample_legal,
};

const RULES: RulesConfig = RulesConfig::standard(StackDepth::Bb20);

fn spec(pairs: u64, base_seed: u64) -> MatchSpec {
    MatchSpec {
        rules: RULES,
        base_seed,
        pairs,
        max_actions: DEFAULT_MAX_ACTIONS,
    }
}

/// Records every observation it is given, then delegates.
struct Spy<A: Agent> {
    inner: A,
    seen: Vec<PlayerObservation>,
}

impl<A: Agent> Agent for Spy<A> {
    fn label(&self) -> String {
        format!("spy({})", self.inner.label())
    }
    fn act(&mut self, obs: &PlayerObservation) -> Result<Decision, AgentError> {
        self.seen.push(obs.clone());
        self.inner.act(obs)
    }
}

#[test]
fn same_seed_same_deal_with_seats_swapped() {
    let mut a = Spy {
        inner: RandomLegal::new(1),
        seen: vec![],
    };
    let mut b = Spy {
        inner: RandomLegal::new(2),
        seen: vec![],
    };
    let (mut ca, mut cb) = (DecisionCounts::default(), DecisionCounts::default());
    for i in 0..50 {
        let seed = deal_seed(11, i);
        let (pair, [g1, g2]) =
            play_pair(RULES, seed, &mut a, &mut b, &mut ca, &mut cb, 512).unwrap();
        // Game 1: A (seat Zero) on the button; game 2: B (seat One) on it.
        assert_eq!(g1.button, Seat::Zero);
        assert_eq!(g2.button, Seat::One);
        // Each position gets the same cards in both games; the board is identical.
        assert_eq!(g1.hole[0], g2.hole[1], "button cards follow the button");
        assert_eq!(g1.hole[1], g2.hole[0], "BB cards follow the BB");
        assert_eq!(g1.board, g2.board);
        // And they are the deck's dealing order for that seed.
        let d = Deck::from_seed(seed);
        let o = d.order();
        assert_eq!(g1.hole[0], [o[1], o[3]]);
        assert_eq!(g1.hole[1], [o[0], o[2]]);
        assert_eq!(g1.board, [o[4], o[5], o[6], o[7], o[8]]);
        assert_eq!(pair.a_button, g1.payoffs[0]);
        assert_eq!(pair.a_big_blind, g2.payoffs[0]);
    }
    // Distinct deals per index.
    assert_ne!(deal_seed(11, 0), deal_seed(11, 1));
    assert_ne!(deal_seed(11, 0), deal_seed(12, 0));
}

#[test]
fn results_are_zero_sum() {
    let mut a = RandomLegal::new(3);
    let mut b = TightRule;
    let (mut ca, mut cb) = (DecisionCounts::default(), DecisionCounts::default());
    for i in 0..200 {
        let (_, logs) = play_pair(
            RULES,
            deal_seed(5, i),
            &mut a,
            &mut b,
            &mut ca,
            &mut cb,
            512,
        )
        .unwrap();
        for l in logs {
            assert_eq!(l.payoffs[0] + l.payoffs[1], 0);
        }
    }
    let m = run_match(&spec(300, 9), &mut RandomLegal::new(1), &mut CheckCall).unwrap();
    let r = mako_eval::report::summarize_match("checkcall", &m, None);
    assert_eq!(r.a_net_chips + r.b_net_chips, 0);
    assert_eq!(r.hands, 600);
}

#[test]
fn mirror_of_same_strategy_is_exactly_zero() {
    // The harness samples from the reported distribution with streams keyed
    // by (deal, position), so both games of a pair are the same hand with
    // the roles swapped, even though the two agents' own RNG seeds differ.
    let m = run_match(
        &spec(500, 21),
        &mut RandomLegal::new(1),
        &mut RandomLegal::new(999),
    )
    .unwrap();
    assert!(m.pairs.iter().all(|p| p.a_total() == 0));
    let r = mako_eval::report::summarize_match("mirror", &m, Some((200, 1)));
    assert_eq!(r.bb100.mean, 0.0);
    assert!(r.bb100.ci_low <= 0.0 && 0.0 <= r.bb100.ci_high);

    let bytes = common::tiny_artifact();
    let mut x = CfrAgent::from_artifact_bytes(bytes, &RULES, 1).unwrap();
    let mut y = CfrAgent::from_artifact_bytes(bytes, &RULES, 2).unwrap();
    let m = run_match(&spec(100, 21), &mut x, &mut y).unwrap();
    assert!(m.pairs.iter().all(|p| p.a_total() == 0));
}

#[test]
fn different_strategies_are_not_trivially_zero() {
    let m = run_match(&spec(300, 4), &mut TightRule, &mut RandomLegal::new(1)).unwrap();
    assert!(m.pairs.iter().any(|p| p.a_total() != 0));
}

#[test]
fn agents_see_only_their_own_observation() {
    let mut a = Spy {
        inner: RandomLegal::new(1),
        seen: vec![],
    };
    let mut b = Spy {
        inner: CheckCall,
        seen: vec![],
    };
    let (mut ca, mut cb) = (DecisionCounts::default(), DecisionCounts::default());
    let mut logs = vec![];
    for i in 0..100 {
        let (_, l) = play_pair(
            RULES,
            deal_seed(8, i),
            &mut a,
            &mut b,
            &mut ca,
            &mut cb,
            512,
        )
        .unwrap();
        logs.extend(l);
    }
    assert!(!a.seen.is_empty() && !b.seen.is_empty());
    for (spy_seat, seen) in [(Seat::Zero, &a.seen), (Seat::One, &b.seen)] {
        for obs in seen {
            assert_eq!(obs.seat, spy_seat);
            assert!(obs.is_my_turn());
            assert!(obs.terminal.is_none());
            assert!(
                obs.opponent_hole_cards.is_none(),
                "no opponent cards before showdown"
            );
            assert!(matches!(obs.board.len(), 0 | 3 | 4 | 5));
            // Own hole cards are one of the referee's holdings for this seat.
            assert!(
                logs.iter()
                    .any(|l| l.hole[spy_seat.index()] == obs.hole_cards)
            );
        }
    }
    assert_eq!(ca.decisions, a.seen.len() as u64);
    assert_eq!(cb.decisions, b.seen.len() as u64);
}

#[test]
fn sampling_respects_legality_and_mass() {
    use mako_core::rules::ActionKind::*;
    let d = [0.5, 0.0, 0.5, 0.0, 0.0, 0.0];
    assert_eq!(sample_legal(&d, &[Fold, Call], 0.0), Some(Fold));
    assert_eq!(sample_legal(&d, &[Fold, Call], 0.49), Some(Fold));
    assert_eq!(sample_legal(&d, &[Fold, Call], 0.51), Some(Call));
    // Mass on illegal slots is ignored and the rest renormalized.
    assert_eq!(sample_legal(&d, &[Call, Pot], 0.99), Some(Call));
    assert_eq!(sample_legal(&d, &[Check, Pot], 0.3), None);
    assert_eq!(sample_legal(&[f64::NAN; 6], &[Check], 0.3), None);
}

#[test]
fn match_rejects_bad_parameters() {
    assert!(run_match(&spec(0, 1), &mut CheckCall, &mut CheckCall).is_err());
}

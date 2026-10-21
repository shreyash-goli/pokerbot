//! Statistics against hand-computed values.

mod common;

use common::*;
use pokerbot_analysis::stats::{fmt_num, fmt_pct};
use pokerbot_analysis::{HandRecord, Role, SessionStats};
use pokerbot_core::action::{Seat, TerminalReason};
use pokerbot_core::rng::PokerbotRng;
use pokerbot_core::state::HandState;

const DRY: &str = "Kc Qd 9h 4s 3c";

fn rec(n: u64, st: &HandState) -> HandRecord {
    HandRecord::from_state(n, st, HUMAN, cfr_label()).unwrap()
}

/// The five scripted hands; expected values are worked out in the comments.
fn session() -> Vec<HandRecord> {
    let checks = [Check; 6];
    // H1 human button limps, BB checks option, check down; human AA wins pot 4.
    let mut a = vec![Call, Check];
    a.extend_from_slice(&checks);
    let h1 = hand(Seat::Zero, "Ah Ad", "7c 2d", DRY, &a);
    // H2 bot button open-raises pot (to 6), human folds: human -2.
    let h2 = hand(Seat::One, "Ah Ad", "7c 2d", DRY, &[Pot, Fold]);
    // H3 human opens half-pot (to 4), bot 3-bets pot (to 12), human calls;
    // check down, bot AA wins pot 24: human -12.
    let mut a = vec![HalfPot, Pot, Call];
    a.extend_from_slice(&checks);
    let h3 = hand(Seat::Zero, "7c 2d", "Ah Ad", DRY, &a);
    // H4 bot button folds: a walk, human +1, human makes no preflop decision.
    let h4 = hand(Seat::One, "Ah Ad", "7c 2d", DRY, &[Fold]);
    // H5 limp/check to a royal-flush board: split, payoffs 0.
    let mut a = vec![Call, Check];
    a.extend_from_slice(&checks);
    let h5 = hand(Seat::Zero, "7c 2d", "8h 3d", "As Ks Qs Js Ts", &a);
    [h1, h2, h3, h4, h5]
        .iter()
        .enumerate()
        .map(|(i, s)| rec(i as u64 + 1, s))
        .collect()
}

#[test]
fn scripted_payoffs_are_as_computed() {
    let r = session();
    let human: Vec<i64> = r.iter().map(|x| x.payoffs[HUMAN.index()]).collect();
    assert_eq!(human, vec![2, -2, -12, 1, 0]);
    assert_eq!(r[0].reason, TerminalReason::Showdown);
    assert_eq!(r[1].reason, TerminalReason::Fold { winner: BOT });
    assert_eq!(r[4].reason, TerminalReason::Showdown);
}

#[test]
fn overall_stats_match_hand_computed_values() {
    let s = SessionStats::from_records(&session());
    let h = &s.human;
    assert_eq!(s.hands(), 5);
    assert_eq!(h.overall.hands, 5);
    assert_eq!(h.overall.net_chips, -11);
    assert_eq!(h.overall.bb_per_100(), Some(-110.0));
    // Decisions in H1, H2, H3, H5 (H4 is a walk); VPIP H1 limp, H3, H5 limp.
    assert_eq!(h.overall.preflop_decision_hands, 4);
    assert_eq!(h.overall.vpip(), Some(0.75));
    assert_eq!(h.overall.pfr(), Some(0.25));
    assert_eq!((h.bets_raises, h.calls, h.checks, h.folds), (1, 3, 9, 1));
    assert_eq!(h.aggression_factor(), Some(1.0 / 3.0));
    assert_eq!(h.aggression_frequency(), Some(0.2));
    assert_eq!((h.showdowns, h.showdowns_won, h.showdowns_split), (3, 1, 1));
    assert_eq!(h.showdown_win_rate(), Some(1.0 / 3.0));

    let b = &s.opponent;
    assert_eq!(b.overall.net_chips, 11);
    assert_eq!(b.overall.bb_per_100(), Some(110.0));
    // BB checks in H1/H5 are decisions but not VPIP; H4 fold is a decision.
    assert_eq!(b.overall.preflop_decision_hands, 5);
    assert_eq!(b.overall.vpip(), Some(0.4));
    assert_eq!(b.overall.pfr(), Some(0.4));
    assert_eq!((b.bets_raises, b.calls, b.checks, b.folds), (2, 0, 11, 1));
    assert_eq!(
        b.aggression_factor(),
        None,
        "no calls: AF is n/a, not NaN/inf"
    );
    assert_eq!(fmt_num(b.aggression_factor()), "n/a");
    assert_eq!(b.aggression_frequency(), Some(2.0 / 3.0));
    assert_eq!((b.showdowns, b.showdowns_won, b.showdowns_split), (3, 1, 1));
}

#[test]
fn positional_splits() {
    let s = SessionStats::from_records(&session());
    let (hb, hbb) = (&s.human.button, &s.human.big_blind);
    assert_eq!(
        (hb.hands, hb.net_chips, hb.preflop_decision_hands),
        (3, -10, 3)
    );
    assert_eq!((hb.vpip(), hb.pfr()), (Some(1.0), Some(1.0 / 3.0)));
    assert_eq!(
        (hbb.hands, hbb.net_chips, hbb.preflop_decision_hands),
        (2, -1, 1)
    );
    assert_eq!((hbb.vpip(), hbb.pfr()), (Some(0.0), Some(0.0)));
    let (bb_btn, bb_bb) = (&s.opponent.button, &s.opponent.big_blind);
    assert_eq!((bb_btn.hands, bb_btn.net_chips), (2, 1));
    assert_eq!((bb_btn.vpip(), bb_btn.pfr()), (Some(0.5), Some(0.5)));
    assert_eq!((bb_bb.hands, bb_bb.net_chips), (3, 10));
    assert_eq!(
        (bb_bb.vpip(), bb_bb.pfr()),
        (Some(1.0 / 3.0), Some(1.0 / 3.0))
    );
    for p in [&s.human, &s.opponent] {
        assert_eq!(p.button.hands + p.big_blind.hands, p.overall.hands);
        assert_eq!(
            p.button.net_chips + p.big_blind.net_chips,
            p.overall.net_chips
        );
    }
}

#[test]
fn bb_checking_option_is_not_vpip_but_limp_is() {
    let mut a = vec![Call, Check];
    a.extend_from_slice(&[Check; 6]);
    let st = hand(Seat::One, "Ah Ad", "7c 2d", DRY, &a); // human is BB
    let s = SessionStats::from_records(&[rec(1, &st)]);
    assert_eq!(s.human.overall.preflop_decision_hands, 1);
    assert_eq!(s.human.overall.vpip(), Some(0.0), "BB check is not VPIP");
    assert_eq!(s.opponent.overall.vpip(), Some(1.0), "limp is VPIP");
    assert_eq!(s.opponent.overall.pfr(), Some(0.0), "limp is not PFR");
}

#[test]
fn bb_calling_a_raise_is_vpip_and_shove_is_pfr() {
    // Bot button shoves; human BB calls.
    let st = hand(Seat::One, "Ah Ad", "7c 2d", DRY, &[AllIn, Call]);
    let s = SessionStats::from_records(&[rec(1, &st)]);
    assert_eq!(s.human.overall.vpip(), Some(1.0));
    assert_eq!(s.human.overall.pfr(), Some(0.0));
    assert_eq!(s.opponent.overall.pfr(), Some(1.0));
    assert_eq!(s.human.overall.net_chips, 200);
}

#[test]
fn empty_stats_are_none_not_nan() {
    let s = SessionStats::new();
    for role in Role::ALL {
        let p = s.player(role);
        assert_eq!(p.overall.bb_per_100(), None);
        assert_eq!(p.overall.vpip(), None);
        assert_eq!(p.overall.pfr(), None);
        assert_eq!(p.aggression_factor(), None);
        assert_eq!(p.aggression_frequency(), None);
        assert_eq!(p.showdown_win_rate(), None);
    }
    assert_eq!(fmt_pct(None), "n/a");
    for line in s.summary_lines() {
        assert!(!line.contains("NaN") && !line.contains("inf"), "{line}");
    }
}

#[test]
fn zero_sum_net_chips_over_random_hands() {
    let mut rng = PokerbotRng::seed_from_u64(17);
    let mut recs = Vec::new();
    for n in 1..=300u64 {
        let button = Seat::ALL[(n % 2) as usize];
        let st =
            pokerbot_core::state::HandState::from_seed(config(button), rng.next_u64()).unwrap();
        let st = play_random(st, &mut rng);
        recs.push(rec(n, &st));
    }
    let s = SessionStats::from_records(&recs);
    assert_eq!(s.human.overall.net_chips + s.opponent.overall.net_chips, 0);
    assert_eq!(s.human.overall.hands, 300);
    let mut inc = SessionStats::new();
    for r in &recs {
        inc.add(r);
    }
    assert_eq!(inc, s, "incremental equals batch");
}

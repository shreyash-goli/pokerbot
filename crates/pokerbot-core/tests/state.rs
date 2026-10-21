//! Golden scripted hands and state-machine API tests. Expected chip
//! amounts are hand-computed in comments (blinds 1/2; S = starting stack).

use pokerbot_core::card::{Card, parse_cards};
use pokerbot_core::deck::Deck;
use pokerbot_core::eval::HandCategory;
use pokerbot_core::rules::{RulesConfig, StackDepth};
use pokerbot_core::state::{
    ActionError, ActionKind, HandConfig, HandError, HandState, LegalAction, Match, Seat, Street,
    TerminalReason,
};

use ActionKind::*;

const DEPTHS: [StackDepth; 3] = StackDepth::ALL;

fn la(kind: ActionKind, raise_to: u32, chips_to_commit: u32) -> LegalAction {
    LegalAction {
        kind,
        raise_to,
        chips_to_commit,
    }
}

/// Deck order: BB, BTN, BB, BTN hole cards, then the 5 board cards.
fn deck(bb: &str, btn: &str, board: &str) -> Deck {
    let (b, t, bo) = (
        parse_cards(bb).unwrap(),
        parse_cards(btn).unwrap(),
        parse_cards(board).unwrap(),
    );
    Deck::from_order(vec![
        b[0], t[0], b[1], t[1], bo[0], bo[1], bo[2], bo[3], bo[4],
    ])
    .unwrap()
}

fn cfg(depth: StackDepth, button: Seat) -> HandConfig {
    HandConfig {
        rules: RulesConfig::standard(depth),
        button,
    }
}

fn play(st: &mut HandState, actions: &[ActionKind]) {
    for &k in actions {
        let before = st.to_act();
        let out = st
            .apply(k)
            .unwrap_or_else(|e| panic!("{k:?} rejected: {e}"));
        assert_eq!(Some(out.record.seat), before);
    }
}

fn start(depth: StackDepth, button: Seat, bb: &str, btn: &str, board: &str) -> HandState {
    HandState::new(cfg(depth, button), deck(bb, btn, board)).unwrap()
}

fn for_all(mut f: impl FnMut(StackDepth, Seat, u32)) {
    for d in DEPTHS {
        for b in Seat::ALL {
            f(d, b, d.starting_stack());
        }
    }
}

fn finals(st: &HandState, btn: Seat) -> (u32, u32) {
    let f = st.final_stacks().unwrap();
    (f[btn.index()], f[btn.other().index()])
}

#[test]
fn state_initial_deal_and_blinds() {
    for_all(|d, btn, s| {
        let st = start(d, btn, "Ah Ad", "Kh Kd", "2c 7d 9h Js 3c");
        let bb = btn.other();
        assert_eq!(st.hole_cards(bb), parse_cards("Ah Ad").unwrap()[..]);
        assert_eq!(st.hole_cards(btn), parse_cards("Kh Kd").unwrap()[..]);
        assert_eq!(st.street(), Street::Preflop);
        assert!(st.board().is_empty(), "board hidden preflop");
        assert_eq!(st.to_act(), Some(btn), "button acts first preflop");
        assert_eq!((st.stack(btn), st.stack(bb)), (s - 1, s - 2));
        assert_eq!(
            (st.street_contribution(btn), st.street_contribution(bb)),
            (1, 2)
        );
        assert_eq!(st.pot(), 3);
        assert!(st.history().is_empty());
        assert_eq!(st.payoffs(), None);
        assert_eq!(
            st.legal_actions(),
            vec![
                la(Fold, 1, 0),
                la(Call, 2, 1),
                la(HalfPot, 4, 3),
                la(Pot, 6, 5),
                la(AllIn, s, s - 1)
            ]
        );
    });
}

#[test]
fn state_sb_folds_preflop() {
    // Button forfeits its 1-chip SB; BB's uncalled 1 is refunded: -1 / +1.
    for_all(|d, btn, s| {
        let mut st = start(d, btn, "Ah Ad", "Kh Kd", "2c 7d 9h Js 3c");
        play(&mut st, &[Fold]);
        assert!(st.is_terminal());
        assert_eq!(st.street(), Street::Showdown);
        assert_eq!(
            st.terminal_reason(),
            Some(TerminalReason::Fold {
                winner: btn.other()
            })
        );
        assert_eq!(finals(&st, btn), (s - 1, s + 1));
        let mut p = [0i64; 2];
        p[btn.index()] = -1;
        p[btn.other().index()] = 1;
        assert_eq!(st.payoffs(), Some(p));
        assert!(st.showdown_hands().is_none());
        assert!(st.legal_actions().is_empty());
        assert_eq!(st.pot(), 0);
        assert!(
            st.board().is_empty(),
            "no board revealed after a preflop fold"
        );
    });
}

#[test]
fn state_limp_check_down_to_showdown() {
    // Pot 4 (2 each). Button KK beats BB QQ on a dry board: button +2.
    for_all(|d, btn, s| {
        let mut st = start(d, btn, "Qh Qd", "Kh Kd", "2c 7d 9h Js 3c");
        let bb = btn.other();
        play(&mut st, &[Call]);
        assert_eq!(st.to_act(), Some(bb), "BB keeps the option after a limp");
        assert_eq!(st.street(), Street::Preflop);
        assert_eq!(
            st.legal_actions(),
            vec![
                la(Check, 2, 0),
                la(HalfPot, 4, 2),
                la(Pot, 6, 4),
                la(AllIn, s, s - 2)
            ]
        );
        play(&mut st, &[Check]);
        for (street, len) in [(Street::Flop, 3), (Street::Turn, 4), (Street::River, 5)] {
            assert_eq!(st.street(), street);
            assert_eq!(st.board().len(), len);
            assert_eq!(st.to_act(), Some(bb), "BB acts first postflop");
            assert_eq!(st.pot(), 4);
            play(&mut st, &[Check, Check]);
        }
        assert_eq!(st.terminal_reason(), Some(TerminalReason::Showdown));
        assert_eq!(finals(&st, btn), (s + 2, s - 2));
        assert_eq!(st.history().len(), 8);
        let sh = st.showdown_hands().unwrap();
        assert_eq!(sh[btn.index()].rank.category(), HandCategory::Pair);
        assert!(sh[btn.index()].rank > sh[bb.index()].rank);
    });
}

#[test]
fn state_bb_raises_after_limp() {
    // Limp (2 each, pot 4); BB pot-raises to 2 + 4 = 6; button folds: -2 / +2.
    for_all(|d, btn, s| {
        let mut st = start(d, btn, "Qh Qd", "Kh Kd", "2c 7d 9h Js 3c");
        play(&mut st, &[Call]);
        let out = st.apply(Pot).unwrap();
        assert_eq!((out.record.raise_to, out.record.chips_committed), (6, 4));
        assert!(!out.round_closed);
        assert_eq!(
            st.to_act(),
            Some(btn),
            "a raise reopens action to the button"
        );
        assert_eq!(st.last_full_raise(), 4);
        play(&mut st, &[Fold]);
        assert_eq!(finals(&st, btn), (s - 2, s + 2));
    });
}

#[test]
fn state_all_in_preflop_runs_out() {
    // Button shoves S, BB calls; board runs out; button AA beats KK: +S / -S.
    for_all(|d, btn, s| {
        let mut st = start(d, btn, "Kh Kd", "Ah Ad", "2c 7d 9h Js 3c");
        let out = st.apply(AllIn).unwrap();
        assert_eq!(
            (out.record.raise_to, out.record.chips_committed),
            (s, s - 1)
        );
        assert_eq!(st.legal_actions(), vec![la(Fold, 2, 0), la(Call, s, s - 2)]);
        let out = st.apply(Call).unwrap();
        assert!(out.round_closed);
        assert_eq!(out.terminal, Some(TerminalReason::Showdown));
        assert_eq!(st.board().len(), 5, "board ran out automatically");
        assert_eq!(st.history().len(), 2, "no actions during the runout");
        assert_eq!(finals(&st, btn), (2 * s, 0));
    });
}

#[test]
fn state_call_for_less_with_refund() {
    // Unequal stacks: button (Zero) 200, BB (One) 50. Button shoves to 200;
    // BB calls all-in for less (48 more, 50 total). Matched pot 100 to the BB
    // (AA vs KK); button's unmatched 150 is refunded: finals 150 / 100.
    let c = cfg(StackDepth::Bb100, Seat::Zero);
    let mut st =
        HandState::new_with_stacks(c, deck("Ah Ad", "Kh Kd", "2c 7d 9h Js 3c"), [200, 50]).unwrap();
    play(&mut st, &[AllIn]);
    assert_eq!(st.legal_actions(), vec![la(Fold, 2, 0), la(Call, 50, 48)]);
    play(&mut st, &[Call]);
    assert_eq!(st.terminal_reason(), Some(TerminalReason::Showdown));
    assert_eq!(st.final_stacks(), Some([150, 100]));
    assert_eq!(st.payoffs(), Some([-50, 50]));
    assert_eq!(st.total_contribution(Seat::Zero), 200);
    // Uncalled-bet refund on a fold at equal stacks: flop bet then fold.
    let mut st = start(
        StackDepth::Bb50,
        Seat::One,
        "Ah Ad",
        "Kh Kd",
        "2c 7d 9h Js 3c",
    );
    play(&mut st, &[Call, Check, Pot, Fold]); // BB (Zero) bets 4 into 4
    assert_eq!(st.final_stacks(), Some([102, 98]));
}

#[test]
fn state_split_pot_board_plays() {
    // Royal flush on board: both play the board, 2 each returned: 0 / 0.
    for_all(|d, btn, s| {
        let mut st = start(d, btn, "2c 3d", "4h 5c", "As Ks Qs Js Ts");
        play(
            &mut st,
            &[Call, Check, Check, Check, Check, Check, Check, Check],
        );
        assert_eq!(st.payoffs(), Some([0, 0]));
        assert_eq!(finals(&st, btn), (s, s));
        let sh = st.showdown_hands().unwrap();
        assert_eq!(sh[0].rank, sh[1].rank);
        assert_eq!(sh[0].rank.category(), HandCategory::StraightFlush);
    });
}

#[test]
fn state_min_raise_tracking_and_dedup() {
    // 100BB: BTN Pot to 6 (inc 4). BB faces 4, pot 8, pac 12:
    // HalfPot 6+6=12, Pot 6+12=18. BB HalfPot to 12 (inc 6). BTN faces 6,
    // pot 18, pac 24: HalfPot 24, Pot 36.
    let mut st = start(
        StackDepth::Bb100,
        Seat::Zero,
        "Qh Qd",
        "Kh Kd",
        "2c 7d 9h Js 3c",
    );
    play(&mut st, &[Pot]);
    assert_eq!(st.last_full_raise(), 4);
    assert_eq!(
        st.legal_actions(),
        vec![
            la(Fold, 2, 0),
            la(Call, 6, 4),
            la(HalfPot, 12, 10),
            la(Pot, 18, 16),
            la(AllIn, 200, 198)
        ]
    );
    play(&mut st, &[HalfPot]);
    assert_eq!(st.last_full_raise(), 6);
    assert_eq!(
        st.legal_actions(),
        vec![
            la(Fold, 6, 0),
            la(Call, 12, 6),
            la(HalfPot, 24, 18),
            la(Pot, 36, 30),
            la(AllIn, 200, 194)
        ]
    );
    // 20BB: same line, then BTN HalfPot to 24 (inc 12). BB faces 12, pot 36,
    // pac 48: HalfPot 48 and Pot 72 both >= all-in 40, so only AllIn (40/28).
    let mut st = start(
        StackDepth::Bb20,
        Seat::Zero,
        "Qh Qd",
        "Kh Kd",
        "2c 7d 9h Js 3c",
    );
    play(&mut st, &[Pot, HalfPot, HalfPot]);
    assert_eq!(st.last_full_raise(), 12);
    assert_eq!(
        st.legal_actions(),
        vec![la(Fold, 12, 0), la(Call, 24, 12), la(AllIn, 40, 28)]
    );
    // BB shoves (inc 16 >= 12, full); BTN has 16 behind = to_call: call only.
    play(&mut st, &[AllIn]);
    assert_eq!(st.legal_actions(), vec![la(Fold, 24, 0), la(Call, 40, 16)]);
    play(&mut st, &[Call]);
    assert_eq!(finals(&st, Seat::Zero), (80, 0)); // KK beats QQ
    // Short all-in (unequal stacks): BTN 200, BB 9. BTN Pot to 6; BB shoves
    // to 9 (inc 3 < 4): last_full_raise stays 4; BTN may only fold or call 3.
    let c = cfg(StackDepth::Bb100, Seat::Zero);
    let mut st =
        HandState::new_with_stacks(c, deck("Qh Qd", "Kh Kd", "2c 7d 9h Js 3c"), [200, 9]).unwrap();
    play(&mut st, &[Pot]);
    assert_eq!(
        st.legal_actions(),
        vec![la(Fold, 2, 0), la(Call, 6, 4), la(AllIn, 9, 7)]
    );
    play(&mut st, &[AllIn]);
    assert_eq!(st.last_full_raise(), 4);
    assert_eq!(st.legal_actions(), vec![la(Fold, 6, 0), la(Call, 9, 3)]);
    play(&mut st, &[Call]);
    assert_eq!(st.final_stacks(), Some([209, 0]));
}

#[test]
fn state_river_showdown_with_kicker() {
    // Both have a pair of kings; button's Q kicker beats BB's J.
    // Pre: limp/check (2 each). Flop: BB bets 2 (half of 4), BTN calls (4 each).
    // Turn: check/check. River: BB checks, BTN bets pot 8, BB calls (12 each).
    // Button wins 24: +12 / -12.
    for_all(|d, btn, s| {
        let mut st = start(d, btn, "Kh Jd", "Kd Qh", "Kc 7d 4h 2s 9c");
        play(
            &mut st,
            &[Call, Check, HalfPot, Call, Check, Check, Check, Pot, Call],
        );
        assert_eq!(st.history()[2].raise_to, 2);
        assert_eq!(st.history()[7].raise_to, 8);
        assert_eq!(st.terminal_reason(), Some(TerminalReason::Showdown));
        assert_eq!(finals(&st, btn), (s + 12, s - 12));
        let sh = st.showdown_hands().unwrap();
        let b: Vec<String> = sh[btn.index()]
            .best_five
            .iter()
            .map(|c| c.to_string())
            .collect();
        assert_eq!(b.join(" "), "Kd Kc Qh 9c 7d");
    });
}

#[test]
fn state_rejects_illegal_without_mutation() {
    let mut st = start(
        StackDepth::Bb100,
        Seat::Zero,
        "Qh Qd",
        "Kh Kd",
        "2c 7d 9h Js 3c",
    );
    let before = st.clone();
    assert_eq!(st.apply(Check), Err(ActionError::Illegal(Check)));
    assert_eq!(st, before);
    let bogus = la(Pot, 7, 6);
    assert_eq!(st.apply_legal(&bogus), Err(ActionError::Mismatch(bogus)));
    assert_eq!(st, before);
    // apply_legal with an exact entry matches apply.
    let mut a = st.clone();
    let mut b = st.clone();
    let pot = a.legal_actions()[3];
    assert_eq!(a.apply_legal(&pot), b.apply(Pot));
    assert_eq!(a, b);
    play(&mut st, &[Fold]);
    let done = st.clone();
    assert_eq!(st.apply(Call), Err(ActionError::HandOver));
    assert_eq!(st.apply_legal(&la(Call, 2, 1)), Err(ActionError::HandOver));
    assert_eq!(st, done);
}

#[test]
fn state_construction_errors_and_seed() {
    let c = cfg(StackDepth::Bb20, Seat::Zero);
    let mut d = Deck::from_seed(1);
    d.deal_n(45).unwrap();
    assert_eq!(
        HandState::new(c, d),
        Err(HandError::DeckTooSmall { remaining: 7 })
    );
    let bad = HandConfig {
        rules: RulesConfig {
            small_blind: 0,
            big_blind: 2,
            starting_stack: 40,
        },
        button: Seat::Zero,
    };
    assert!(matches!(
        HandState::from_seed(bad, 1),
        Err(HandError::Config(_))
    ));
    assert_eq!(
        HandState::new_with_stacks(c, Deck::from_seed(1), [0, 40]),
        Err(HandError::InvalidStack(0))
    );
    // Seeded deal follows deck.rs order: seed 7 deck starts Qh 9c Kh 8h Js 7s 2c Ts 7d.
    let mut st = HandState::from_seed(c, 7).unwrap();
    let s = |v: [Card; 2]| format!("{} {}", v[0], v[1]);
    assert_eq!(s(st.hole_cards(Seat::One)), "Qh Kh"); // BB (button is Zero)
    assert_eq!(s(st.hole_cards(Seat::Zero)), "9c 8h");
    assert_eq!(st, HandState::from_seed(c, 7).unwrap());
    play(&mut st, &[Call, Check]);
    let b: Vec<String> = st.board().iter().map(|c| c.to_string()).collect();
    assert_eq!(b.join(" "), "Js 7s 2c");
}

#[test]
fn state_blinds_all_in_short_stacks() {
    // S = 1: both blinds all-in immediately (1 each); straight to showdown.
    let c = HandConfig {
        rules: RulesConfig::new(1, 2, 1).unwrap(),
        button: Seat::One,
    };
    let st = HandState::new(c, deck("Ah Ad", "Kh Kd", "2c 7d 9h Js 3c")).unwrap();
    assert!(st.is_terminal());
    assert_eq!(st.board().len(), 5);
    assert_eq!(st.final_stacks(), Some([2, 0])); // BB is Zero with AA
    // S = 2: BB posts 2 all-in; button must fold or call 1.
    let c = HandConfig {
        rules: RulesConfig::new(1, 2, 2).unwrap(),
        button: Seat::Zero,
    };
    let mut st = HandState::new(c, deck("Ah Ad", "Kh Kd", "2c 7d 9h Js 3c")).unwrap();
    assert_eq!(st.legal_actions(), vec![la(Fold, 1, 0), la(Call, 2, 1)]);
    play(&mut st, &[Call]);
    assert_eq!(st.final_stacks(), Some([0, 4]));
}

#[test]
fn state_observation_boundary() {
    let mut st = start(
        StackDepth::Bb100,
        Seat::Zero,
        "Qh Qd",
        "Kh Kd",
        "2c 7d 9h Js 3c",
    );
    let o0 = st.observation(Seat::Zero);
    let o1 = st.observation(Seat::One);
    assert_eq!(o0.hole_cards, st.hole_cards(Seat::Zero));
    assert!(o0.is_my_turn() && !o0.legal_actions.is_empty());
    assert!(!o1.is_my_turn() && o1.legal_actions.is_empty());
    assert_eq!(o0.opponent_hole_cards, None);
    play(&mut st, &[Call, Check, Check, Check, Check, Check, Check]);
    assert_eq!(st.street(), Street::River);
    assert_eq!(
        st.observation(Seat::One).opponent_hole_cards,
        None,
        "not before river closes"
    );
    play(&mut st, &[Check]);
    assert_eq!(
        st.observation(Seat::One).opponent_hole_cards,
        Some(st.hole_cards(Seat::Zero))
    );
    let mut st = start(
        StackDepth::Bb100,
        Seat::Zero,
        "Qh Qd",
        "Kh Kd",
        "2c 7d 9h Js 3c",
    );
    play(&mut st, &[Call, Pot, Fold]);
    for seat in Seat::ALL {
        assert_eq!(
            st.observation(seat).opponent_hole_cards,
            None,
            "never after a fold"
        );
    }
}

#[test]
fn state_match_alternates_button_and_scores() {
    let mut m = Match::new(RulesConfig::standard(StackDepth::Bb50), Seat::One).unwrap();
    let mut buttons = Vec::new();
    for seed in 0..4u64 {
        let mut st = m.start_hand(Deck::from_seed(seed)).unwrap();
        buttons.push(st.button());
        assert_eq!(st.stack(st.button()), 99, "stacks reset each hand");
        assert!(!m.record(&st), "unfinished hand not recorded");
        st.apply(Fold).unwrap();
        assert!(m.record(&st));
    }
    assert_eq!(buttons, [Seat::One, Seat::Zero, Seat::One, Seat::Zero]);
    assert_eq!(m.score(), [0, 0]); // each seat folded its SB twice
    assert_eq!(m.hands_played(), 4);
    let c = cfg(StackDepth::Bb20, Seat::Zero);
    assert_eq!(c.next_hand().button, Seat::One);
}

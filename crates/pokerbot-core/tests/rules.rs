//! M-01 acceptance tests: betting legality, sizing, reopening, settlement and
//! chip-accounting invariants. Expected values are hand-computed in comments
//! (blinds 1/2 unless stated; "pac" = pot after call = pot + to_call).

use pokerbot_core::rng::PokerbotRng;
use pokerbot_core::rules::{
    ActionKind, BettingSnapshot, Chips, HandResult, LegalAction, MAX_STARTING_STACK, Position,
    RulesConfig, RulesError, StackDepth, Street, button_seat, check_conservation, check_zero_sum,
    legal_actions, next_can_raise, post_blinds, resolve_action, settle, split_pot,
    unmatched_refund,
};

use ActionKind::*;

#[allow(clippy::too_many_arguments)]
fn snap(
    actor_street: Chips,
    opp_street: Chips,
    actor_stack: Chips,
    opp_stack: Chips,
    pot: Chips,
    last_full_raise: Chips,
    can_raise: bool,
) -> BettingSnapshot {
    BettingSnapshot {
        actor_street,
        opp_street,
        actor_stack,
        opp_stack,
        pot,
        last_full_raise,
        big_blind: 2,
        can_raise,
    }
}

fn la(kind: ActionKind, raise_to: Chips, chips_to_commit: Chips) -> LegalAction {
    LegalAction {
        kind,
        raise_to,
        chips_to_commit,
    }
}

fn legal(s: &BettingSnapshot) -> Vec<LegalAction> {
    legal_actions(s).unwrap()
}

// ------------------------------------------------------------ config ---

#[test]
fn standard_depths() {
    let stacks: Vec<Chips> = StackDepth::ALL
        .iter()
        .map(|d| RulesConfig::standard(*d).starting_stack)
        .collect();
    assert_eq!(stacks, [40, 100, 200]);
    for d in StackDepth::ALL {
        let c = RulesConfig::standard(d);
        assert_eq!((c.small_blind, c.big_blind), (1, 2));
        c.validate().unwrap();
    }
}

#[test]
fn invalid_config_rejected() {
    assert!(matches!(
        RulesConfig::new(0, 2, 100),
        Err(RulesError::InvalidConfig(_))
    ));
    assert!(matches!(
        RulesConfig::new(3, 2, 100),
        Err(RulesError::InvalidConfig(_))
    ));
    assert!(matches!(
        RulesConfig::new(1, 2, 0),
        Err(RulesError::InvalidConfig(_))
    ));
    assert!(matches!(
        RulesConfig::new(1, 2, MAX_STARTING_STACK + 1),
        Err(RulesError::InvalidConfig(_))
    ));
    assert!(RulesConfig::new(1, 1, 1).is_ok());
    assert!(RulesConfig::new(1, 2, MAX_STARTING_STACK).is_ok());
}

// ------------------------------------------------ positions & streets ---

#[test]
fn blind_order_streets_and_button() {
    assert_eq!(Street::Preflop.first_to_act(), Position::Button);
    for st in [Street::Flop, Street::Turn, Street::River] {
        assert_eq!(st.first_to_act(), Position::BigBlind);
    }
    let lens: Vec<usize> = Street::ALL.iter().map(|s| s.board_len()).collect();
    assert_eq!(lens, [0, 3, 4, 5]);
    let rev: Vec<usize> = Street::ALL.iter().map(|s| s.cards_revealed()).collect();
    assert_eq!(rev, [0, 3, 1, 1]);
    assert_eq!(Street::River.next(), None);
    assert_eq!(Street::Preflop.next(), Some(Street::Flop));
    let seats: Vec<u8> = (0..4).map(|h| button_seat(1, h)).collect();
    assert_eq!(seats, [1, 0, 1, 0]);
    assert_eq!(Position::Button.other(), Position::BigBlind);
}

#[test]
fn blinds_posted_and_short_stack_covered() {
    let c = RulesConfig::standard(StackDepth::Bb100);
    let p = post_blinds(&c, 200, 200);
    assert_eq!((p.button_posted, p.big_blind_posted), (1, 2));
    // Big blind with 1 chip posts it all (all-in); short SB likewise.
    let c = RulesConfig::new(1, 2, 1).unwrap();
    let p = post_blinds(&c, 1, 1);
    assert_eq!((p.button_posted, p.big_blind_posted), (1, 1));
    let c = RulesConfig::new(5, 10, 100).unwrap();
    let p = post_blinds(&c, 3, 7);
    assert_eq!((p.button_posted, p.big_blind_posted), (3, 7));
}

// -------------------------------------------------------- legality ---

#[test]
fn preflop_sb_first_action_all_depths() {
    // SB posted 1, BB 2. to_call = 1, pot = 3, pac = 4.
    // HalfPot: 2 + 4/2 = 4 (commit 3). Pot: 2 + 4 = 6 (commit 5).
    // AllIn: 1 + (S - 1) = S.
    for (s, d) in [
        (40, StackDepth::Bb20),
        (100, StackDepth::Bb50),
        (200, StackDepth::Bb100),
    ] {
        assert_eq!(RulesConfig::standard(d).starting_stack, s);
        let got = legal(&snap(1, 2, s - 1, s - 2, 3, 2, true));
        assert_eq!(
            got,
            vec![
                la(Fold, 1, 0),
                la(Call, 2, 1),
                la(HalfPot, 4, 3),
                la(Pot, 6, 5),
                la(AllIn, s, s - 1),
            ],
            "depth {s}"
        );
    }
}

#[test]
fn bb_option_after_limp() {
    // SB limps to 2. BB: to_call 0, pot 4. Bet half: 2 + 2 = 4, pot: 2 + 4 = 6.
    // BB has not acted (blind is not an action) so can_raise is true.
    let limp = la(Call, 2, 1);
    let sb = snap(1, 2, 199, 198, 3, 2, true);
    let upd = next_can_raise(&sb, &limp, false);
    assert!(upd.opponent_can_raise && !upd.is_full_raise);
    assert_eq!(upd.last_full_raise, 2);
    let got = legal(&snap(2, 2, 198, 198, 4, 2, upd.opponent_can_raise));
    assert_eq!(
        got,
        vec![
            la(Check, 2, 0),
            la(HalfPot, 4, 2),
            la(Pot, 6, 4),
            la(AllIn, 200, 198)
        ]
    );
}

#[test]
fn opening_bet_on_flop() {
    // Pot 12 (6 each), stacks 194. Bets: half 6, pot 12, all-in 194.
    let got = legal(&snap(0, 0, 194, 194, 12, 2, true));
    assert_eq!(
        got,
        vec![
            la(Check, 0, 0),
            la(HalfPot, 6, 6),
            la(Pot, 12, 12),
            la(AllIn, 194, 194)
        ]
    );
}

#[test]
fn raise_facing_a_bet() {
    // Pot 12 before BB bets 12: pot 24, to_call 12, pac 36.
    // HalfPot: 12 + 18 = 30. Pot: 12 + 36 = 48.
    let got = legal(&snap(0, 12, 194, 182, 24, 12, true));
    assert_eq!(
        got,
        vec![
            la(Fold, 0, 0),
            la(Call, 12, 12),
            la(HalfPot, 30, 30),
            la(Pot, 48, 48),
            la(AllIn, 194, 194),
        ]
    );
    // Resulting update: increment 30 - 12 = 18 >= 12, full raise.
    let s = snap(0, 12, 194, 182, 24, 12, true);
    let u = next_can_raise(&s, &la(HalfPot, 30, 30), true);
    assert_eq!(
        (u.increment, u.is_full_raise, u.last_full_raise),
        (18, true, 18)
    );
    assert!(u.opponent_can_raise);
}

#[test]
fn min_raise_bumping() {
    // Synthetic (bumping never binds at 1/2 with pot-after-call sizing,
    // but the rule is enforced). Bet with pot 6, big blind 4:
    // half = 3 < 4 -> bumped to 4; pot = 6.
    let mut s = snap(0, 0, 100, 100, 6, 4, true);
    s.big_blind = 4;
    assert_eq!(
        legal(&s),
        vec![
            la(Check, 0, 0),
            la(HalfPot, 4, 4),
            la(Pot, 6, 6),
            la(AllIn, 100, 100)
        ]
    );
    // Facing 10 with last full raise 30, pot 14, pac 24:
    // half inc 12 -> 30 (to 40); pot inc 24 -> 30 (to 40); equal -> HalfPot dropped.
    let got = legal(&snap(0, 10, 100, 100, 14, 30, true));
    assert_eq!(
        got,
        vec![
            la(Fold, 0, 0),
            la(Call, 10, 10),
            la(Pot, 40, 40),
            la(AllIn, 100, 100)
        ]
    );
    // Pot 2, no bet: half 1 -> 2, pot 2 -> dedup to Pot only.
    let got = legal(&snap(0, 0, 50, 50, 2, 2, true));
    assert_eq!(got, vec![la(Check, 0, 0), la(Pot, 2, 2), la(AllIn, 50, 50)]);
}

#[test]
fn all_in_dedup_when_stack_below_sizes() {
    // 20BB: SB opened pot to 6, BB raised pot to 18. SB: actor 6, opp 18,
    // stack 34, pot 24, to_call 12, pac 36. Half: 18 + 18 = 36 < 40 keep;
    // Pot: 18 + 36 = 54 >= 40 dropped. AllIn 40 (commit 34).
    let got = legal(&snap(6, 18, 34, 22, 24, 12, true));
    assert_eq!(
        got,
        vec![
            la(Fold, 6, 0),
            la(Call, 18, 12),
            la(HalfPot, 36, 30),
            la(AllIn, 40, 34)
        ]
    );
    // BB facing 4-bet to 36: actor 18, stack 22, pot 54, pac 72. Half to 72,
    // pot to 108, both >= 40 -> only AllIn (to 40, commit 22).
    let got = legal(&snap(18, 36, 22, 4, 54, 18, true));
    assert_eq!(
        got,
        vec![la(Fold, 18, 0), la(Call, 36, 18), la(AllIn, 40, 22)]
    );
    // Exactly equal to all-in is dropped too: stack 6, opening bet with pot 12:
    // half 6 == all-in 6 -> dropped; pot 12 -> dropped.
    let got = legal(&snap(0, 0, 6, 100, 12, 2, true));
    assert_eq!(got, vec![la(Check, 0, 0), la(AllIn, 6, 6)]);
}

#[test]
fn call_for_less_and_no_all_in_duplicate() {
    // Facing 50 with 30 behind: call all-in for less, no AllIn entry.
    let got = legal(&snap(0, 50, 30, 100, 60, 50, true));
    assert_eq!(got, vec![la(Fold, 0, 0), la(Call, 30, 30)]);
    // Stack exactly equals to_call: same.
    let got = legal(&snap(0, 50, 50, 100, 60, 50, true));
    assert_eq!(got, vec![la(Fold, 0, 0), la(Call, 50, 50)]);
}

#[test]
fn no_raises_when_opponent_all_in_or_not_reopened() {
    let got = legal(&snap(0, 30, 100, 0, 40, 30, true));
    assert_eq!(got, vec![la(Fold, 0, 0), la(Call, 30, 30)]);
    let got = legal(&snap(10, 15, 90, 50, 45, 10, false));
    assert_eq!(got, vec![la(Fold, 10, 0), la(Call, 15, 5)]);
}

#[test]
fn short_all_in_does_not_reopen() {
    // A bet 10 into 20; B (15 behind) shoves to 15: increment 5 < min 10.
    let b = snap(0, 10, 15, 90, 30, 10, true);
    let got = legal(&b);
    assert_eq!(
        got,
        vec![la(Fold, 0, 0), la(Call, 10, 10), la(AllIn, 15, 15)]
    );
    let u = next_can_raise(&b, &la(AllIn, 15, 15), true);
    assert_eq!(
        (u.increment, u.is_full_raise, u.last_full_raise),
        (5, false, 10)
    );
    assert!(!u.opponent_can_raise, "A already acted: call or fold only");
    // Same short shove when the opponent has not yet acted: reopened.
    let u = next_can_raise(&b, &la(AllIn, 15, 15), false);
    assert!(u.opponent_can_raise);
    // A full all-in raise (increment >= min) reopens.
    let b = snap(0, 10, 25, 90, 30, 10, true);
    let u = next_can_raise(&b, &la(AllIn, 25, 25), true);
    assert!(u.is_full_raise && u.opponent_can_raise);
    assert_eq!(u.last_full_raise, 15);
    // Check/call never raise.
    let u = next_can_raise(&b, &la(Call, 10, 10), true);
    assert_eq!(
        (u.increment, u.is_full_raise, u.last_full_raise),
        (0, false, 10)
    );
}

#[test]
fn illegal_and_inconsistent_inputs_rejected() {
    let s = snap(0, 12, 194, 182, 24, 12, true);
    assert_eq!(
        resolve_action(&s, Check),
        Err(RulesError::IllegalAction(Check))
    );
    assert_eq!(resolve_action(&s, Call).unwrap(), la(Call, 12, 12));
    let s = snap(0, 0, 194, 194, 12, 2, true);
    assert_eq!(
        resolve_action(&s, Fold),
        Err(RulesError::IllegalAction(Fold))
    );
    assert_eq!(
        resolve_action(&s, Call),
        Err(RulesError::IllegalAction(Call))
    );
    // Inconsistent snapshots.
    assert!(matches!(
        legal_actions(&snap(5, 2, 10, 10, 7, 2, true)),
        Err(RulesError::InconsistentSnapshot(_))
    ));
    assert!(matches!(
        legal_actions(&snap(2, 4, 10, 10, 5, 2, true)),
        Err(RulesError::InconsistentSnapshot(_))
    ));
    assert_eq!(
        legal_actions(&snap(0, 4, 0, 10, 8, 2, true)),
        Err(RulesError::ActorAllIn)
    );
    let mut z = snap(0, 0, 10, 10, 4, 2, true);
    z.big_blind = 0;
    assert!(legal_actions(&z).is_err());
    let huge = snap(0, 0, u32::MAX, u32::MAX, 4, 2, true);
    assert!(legal_actions(&huge).is_err());
}

// ------------------------------------------------------- settlement ---

#[test]
fn refund_of_unmatched_chips() {
    assert_eq!(unmatched_refund([200, 40]), [160, 0]);
    assert_eq!(unmatched_refund([40, 40]), [0, 0]);
    // Button all-in 200, BB called all-in for 40, BB wins: contested 80.
    let st = settle(
        [200, 40],
        HandResult::Showdown {
            winner: Some(Position::BigBlind),
        },
    );
    assert_eq!(st.refunds, [160, 0]);
    assert_eq!(st.contested_pot, 80);
    assert_eq!(st.awards, [0, 80]);
    assert_eq!(st.payoffs, [-40, 40]);
    // SB folds preflop: BB's uncalled 1 returned, BB wins 2 -> +1 / -1.
    let st = settle(
        [1, 2],
        HandResult::Fold {
            folder: Position::Button,
        },
    );
    assert_eq!(st.refunds, [0, 1]);
    assert_eq!(st.awards, [0, 2]);
    assert_eq!(st.payoffs, [-1, 1]);
    check_zero_sum(st.payoffs).unwrap();
}

#[test]
fn odd_chip_split() {
    assert_eq!(split_pot(5, 2).unwrap(), vec![3, 2]); // odd chip to first (BB)
    assert_eq!(split_pot(7, 3).unwrap(), vec![3, 2, 2]);
    assert_eq!(split_pot(0, 2).unwrap(), vec![0, 0]);
    assert_eq!(split_pot(5, 0), Err(RulesError::NoWinners));
    for pot in 0..300 {
        for n in 1..6 {
            let v = split_pot(pot, n).unwrap();
            assert_eq!(v.iter().sum::<Chips>(), pot);
            assert!(v.iter().max().unwrap() - v.iter().min().unwrap() <= 1);
        }
    }
    let st = settle([50, 50], HandResult::Showdown { winner: None });
    assert_eq!(st.awards, [50, 50]);
    assert_eq!(st.payoffs, [0, 0]);
    let st = settle([120, 50], HandResult::Showdown { winner: None });
    assert_eq!(
        (st.refunds, st.awards, st.payoffs),
        ([70, 0], [50, 50], [0, 0])
    );
}

#[test]
fn invariant_checkers() {
    check_conservation([190, 200], 10, 200).unwrap();
    assert_eq!(
        check_conservation([190, 200], 11, 200),
        Err(RulesError::ConservationViolated {
            expected: 400,
            actual: 401
        })
    );
    assert_eq!(
        check_zero_sum([3, -2]),
        Err(RulesError::NotZeroSum { sum: 1 })
    );
}

// ---------------------------------------------- randomized hand walk ---

/// Minimal reference betting loop (test-only stand-in for M-04) that plays
/// random legal actions and asserts invariants after every step.
fn play_random_hand(cfg: RulesConfig, rng: &mut PokerbotRng) -> [i64; 2] {
    let s0 = cfg.starting_stack;
    let p = post_blinds(&cfg, s0, s0);
    let mut stacks = [s0 - p.button_posted, s0 - p.big_blind_posted];
    let mut street_c = [p.button_posted, p.big_blind_posted];
    let mut total = street_c;
    let mut pot = p.button_posted + p.big_blind_posted;
    let mut street = Street::Preflop;
    let mut lfr = cfg.big_blind;
    let mut can_raise = [true, true];
    let mut acted = [false, false];
    let mut actor = Street::Preflop.first_to_act().index();
    let result;
    'hand: loop {
        // Is betting possible on this street?
        let closed = stacks[0] == 0 || (stacks[1] == 0 && street_c[actor] >= street_c[1 - actor]);
        if !closed {
            loop {
                let o = 1 - actor;
                let s = BettingSnapshot {
                    actor_street: street_c[actor],
                    opp_street: street_c[o],
                    actor_stack: stacks[actor],
                    opp_stack: stacks[o],
                    pot,
                    last_full_raise: lfr,
                    big_blind: cfg.big_blind,
                    can_raise: can_raise[actor],
                };
                let acts = legal_actions(&s).unwrap();
                // Canonical order, unique kinds, increasing aggressive sizes.
                for w in acts.windows(2) {
                    assert!(w[0].kind < w[1].kind);
                    if w[0].kind.is_aggressive() {
                        assert!(w[0].raise_to < w[1].raise_to);
                    }
                }
                for a in &acts {
                    assert!(a.chips_to_commit <= s.actor_stack);
                    assert_eq!(a.raise_to, s.actor_street + a.chips_to_commit);
                    if a.kind.is_aggressive() {
                        assert!(a.raise_to > s.opp_street);
                        if a.kind != AllIn {
                            assert!(a.raise_to - s.opp_street >= s.min_raise_increment());
                        }
                    }
                }
                let a = acts[rng.gen_range(acts.len() as u64) as usize];
                let upd = next_can_raise(&s, &a, acted[o]);
                stacks[actor] -= a.chips_to_commit;
                street_c[actor] = a.raise_to;
                total[actor] += a.chips_to_commit;
                pot += a.chips_to_commit;
                check_conservation(stacks, pot, s0).unwrap();
                if a.kind == Fold {
                    result = HandResult::Fold {
                        folder: if actor == 0 {
                            Position::Button
                        } else {
                            Position::BigBlind
                        },
                    };
                    break 'hand;
                }
                lfr = upd.last_full_raise;
                can_raise[o] = upd.opponent_can_raise;
                acted[actor] = true;
                let matched = street_c[0] == street_c[1];
                let short_call = a.kind == Call && street_c[actor] < street_c[o];
                if (matched && acted[o]) || short_call {
                    break;
                }
                if matched && stacks[o] == 0 {
                    break;
                }
                actor = o;
            }
        }
        // Street over: run out if anyone is all-in, else next street.
        let next = if stacks[0] == 0 || stacks[1] == 0 {
            None
        } else {
            street.next()
        };
        match next {
            Some(n) => {
                street = n;
                street_c = [0, 0];
                lfr = cfg.big_blind;
                can_raise = [true, true];
                acted = [false, false];
                actor = n.first_to_act().index();
            }
            None => {
                let winner = match rng.gen_range(3) {
                    0 => Some(Position::Button),
                    1 => Some(Position::BigBlind),
                    _ => None,
                };
                result = HandResult::Showdown { winner };
                break 'hand;
            }
        }
    }
    let st = settle(total, result);
    let fin = [0, 1].map(|i| stacks[i] + st.refunds[i] + st.awards[i]);
    check_conservation(fin, 0, s0).unwrap();
    check_zero_sum(st.payoffs).unwrap();
    for i in 0..2 {
        assert_eq!(st.payoffs[i], i64::from(fin[i]) - i64::from(s0));
    }
    st.payoffs
}

#[test]
fn random_legal_hands_conserve_chips() {
    let mut rng = PokerbotRng::seed_from_u64(0xC41F);
    let mut configs: Vec<RulesConfig> = StackDepth::ALL
        .iter()
        .map(|d| RulesConfig::standard(*d))
        .collect();
    configs.push(RulesConfig::new(1, 2, 1).unwrap()); // both blinds all-in short
    configs.push(RulesConfig::new(1, 2, 3).unwrap());
    configs.push(RulesConfig::new(5, 10, 137).unwrap());
    for cfg in configs {
        for _ in 0..1500 {
            let p = play_random_hand(cfg, &mut rng);
            assert_eq!(p[0] + p[1], 0);
        }
    }
}

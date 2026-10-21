//! The rules engine at 20/50/100BB. Hand-computed amounts in
//! comments (blinds 1/2; "pac" = pot after call).

use pokerbot_core::card::parse_cards;
use pokerbot_core::deck::Deck;
use pokerbot_core::rng::PokerbotRng;
use pokerbot_core::rules::{
    ActionKind, Chips, MAX_STARTING_STACK, RulesConfig, RulesError, StackDepth,
};
use pokerbot_core::state::{
    HandConfig, HandError, HandState, LegalAction, Match, Seat, Street, TerminalReason,
};

use ActionKind::*;

fn la(kind: ActionKind, raise_to: Chips, chips_to_commit: Chips) -> LegalAction {
    LegalAction {
        kind,
        raise_to,
        chips_to_commit,
    }
}

fn cfg(depth: StackDepth, button: Seat) -> HandConfig {
    HandConfig {
        rules: RulesConfig::standard(depth),
        button,
    }
}

/// Button Zero holds AA, BB (One) holds KK, dry board.
fn aa_vs_kk(depth: StackDepth) -> HandState {
    let (bb, btn, board) = (
        parse_cards("Kh Kd").unwrap(),
        parse_cards("Ah Ad").unwrap(),
        parse_cards("2c 7d 9h Js 3c").unwrap(),
    );
    let order = vec![
        bb[0], btn[0], bb[1], btn[1], board[0], board[1], board[2], board[3], board[4],
    ];
    HandState::new(cfg(depth, Seat::Zero), Deck::from_order(order).unwrap()).unwrap()
}

#[test]
fn depth_stack_sizes() {
    let got: Vec<(u32, Chips)> = StackDepth::ALL
        .iter()
        .map(|d| (d.big_blinds(), d.starting_stack()))
        .collect();
    assert_eq!(got, [(20, 40), (50, 100), (100, 200)]);
    for d in StackDepth::ALL {
        let c = RulesConfig::standard(d);
        assert_eq!(c.starting_stack, d.starting_stack());
        assert_eq!(c.starting_stack, d.big_blinds() * c.big_blind);
        for button in Seat::ALL {
            let st = HandState::new(cfg(d, button), Deck::from_seed(1)).unwrap();
            assert_eq!(st.starting_stack(Seat::Zero), d.starting_stack());
            assert_eq!(st.starting_stack(Seat::One), d.starting_stack());
            assert_eq!(st.stack(button), d.starting_stack() - 1);
            assert_eq!(st.stack(button.other()), d.starting_stack() - 2);
        }
    }
}

#[test]
fn depth_invalid_configs_rejected() {
    for (sb, bb, s) in [
        (0, 2, 100),
        (3, 2, 100),
        (1, 2, 0),
        (1, 2, MAX_STARTING_STACK + 1),
    ] {
        assert!(matches!(
            RulesConfig::new(sb, bb, s),
            Err(RulesError::InvalidConfig(_))
        ));
        let bad = HandConfig {
            rules: RulesConfig {
                small_blind: sb,
                big_blind: bb,
                starting_stack: s,
            },
            button: Seat::Zero,
        };
        assert!(matches!(
            HandState::from_seed(bad, 1),
            Err(HandError::Config(_))
        ));
        assert!(Match::new(bad.rules, Seat::Zero).is_err());
    }
}

#[test]
fn depth_next_hand_keeps_depth() {
    for d in StackDepth::ALL {
        let c = cfg(d, Seat::Zero);
        let n = c.next_hand();
        assert_eq!(n.rules, c.rules);
        assert_eq!(n.button, Seat::One);
        assert_eq!(n.next_hand(), c);
        let mut m = Match::new(RulesConfig::standard(d), Seat::One).unwrap();
        for i in 0..4u64 {
            let mut st = m.start_hand(Deck::from_seed(i)).unwrap();
            assert_eq!(st.config().rules.starting_stack, d.starting_stack());
            assert_eq!(
                st.stack(st.big_blind()),
                d.starting_stack() - 2,
                "stacks reset"
            );
            st.apply(Fold).unwrap();
            assert!(m.record(&st));
            assert_eq!(m.next_config().rules, RulesConfig::standard(d));
        }
    }
}

#[test]
fn depth_golden_100bb_open_3bet_4bet_jam() {
    let mut st = aa_vs_kk(StackDepth::Bb100);
    // BTN opens pot: pot 3, to_call 1, pac 4 -> to 6.
    st.apply(Pot).unwrap();
    // BB faces 4: pot 8, pac 12: half to 12, pot to 18.
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
    st.apply(Pot).unwrap(); // 3-bet to 18 (inc 12)
    assert_eq!(st.last_full_raise(), 12);
    // BTN faces 12: pot 24, pac 36: half to 36, pot to 54.
    assert_eq!(
        st.legal_actions(),
        vec![
            la(Fold, 6, 0),
            la(Call, 18, 12),
            la(HalfPot, 36, 30),
            la(Pot, 54, 48),
            la(AllIn, 200, 194)
        ]
    );
    st.apply(Pot).unwrap(); // 4-bet to 54 (inc 36)
    assert_eq!(st.last_full_raise(), 36);
    // BB faces 36: pot 72, pac 108: half to 108, pot to 162, both < 200.
    assert_eq!(
        st.legal_actions(),
        vec![
            la(Fold, 18, 0),
            la(Call, 54, 36),
            la(HalfPot, 108, 90),
            la(Pot, 162, 144),
            la(AllIn, 200, 182)
        ]
    );
    st.apply(AllIn).unwrap(); // jam: inc 146, a full raise
    assert_eq!(st.last_full_raise(), 146);
    // BTN has 146 behind = to_call: call only, no AllIn duplicate.
    assert_eq!(
        st.legal_actions(),
        vec![la(Fold, 54, 0), la(Call, 200, 146)]
    );
    st.apply(Call).unwrap();
    assert_eq!(st.terminal_reason(), Some(TerminalReason::Showdown));
    assert_eq!(st.board().len(), 5);
    assert_eq!(st.final_stacks(), Some([400, 0]));
    assert_eq!(st.payoffs(), Some([200, -200]));

    // Same line, BB folds to the 4-bet: BTN wins BB's 18 (+18 / -18).
    let mut st = aa_vs_kk(StackDepth::Bb100);
    for k in [Pot, Pot, Pot, Fold] {
        st.apply(k).unwrap();
    }
    assert_eq!(st.final_stacks(), Some([218, 182]));
}

#[test]
fn depth_golden_50bb_4bet_leaves_only_jam() {
    let mut st = aa_vs_kk(StackDepth::Bb50);
    st.apply(Pot).unwrap(); // open to 6
    st.apply(Pot).unwrap(); // 3-bet to 18
    // BTN: half 36, pot 54 both < 100.
    assert_eq!(
        st.legal_actions(),
        vec![
            la(Fold, 6, 0),
            la(Call, 18, 12),
            la(HalfPot, 36, 30),
            la(Pot, 54, 48),
            la(AllIn, 100, 94)
        ]
    );
    st.apply(Pot).unwrap(); // 4-bet to 54
    // BB (82 behind): half 108 and pot 162 are >= all-in 100 -> only jam.
    assert_eq!(
        st.legal_actions(),
        vec![la(Fold, 18, 0), la(Call, 54, 36), la(AllIn, 100, 82)]
    );
    st.apply(AllIn).unwrap();
    assert_eq!(st.legal_actions(), vec![la(Fold, 54, 0), la(Call, 100, 46)]);
    st.apply(Call).unwrap();
    assert_eq!(st.final_stacks(), Some([200, 0]));

    // Postflop at 50BB: limp, then flop pot bets 4 -> 12 -> 36.
    let mut st = aa_vs_kk(StackDepth::Bb50);
    st.apply(Call).unwrap();
    st.apply(Check).unwrap();
    assert_eq!(st.street(), Street::Flop);
    // BB (One) bets pot 4; BTN raises pot: pot 8, to_call 4, pac 12 -> 4+12=16.
    st.apply(Pot).unwrap();
    assert_eq!(st.legal_actions()[3], la(Pot, 16, 16));
    st.apply(Pot).unwrap();
    // BB faces 12: pot 24, pac 36 -> pot 52 (commit 48) < all-in 98, so kept.
    assert_eq!(st.legal_actions()[3], la(Pot, 52, 48));
}

#[test]
fn depth_pot_raise_chain_length_grows_with_depth() {
    // Raise pot every time until Pot is no longer offered (it would reach
    // all-in). 20BB: 6, 18 (54 >= 40). 50BB: 6, 18, 54 (162 >= 100).
    // 100BB: 6, 18, 54, 162 (486 >= 200).
    let mut chains = Vec::new();
    for d in StackDepth::ALL {
        let mut st = aa_vs_kk(d);
        let mut tos = Vec::new();
        while let Some(p) = st.legal_actions().into_iter().find(|a| a.kind == Pot) {
            st.apply_legal(&p).unwrap();
            tos.push(p.raise_to);
        }
        chains.push(tos);
    }
    assert_eq!(
        chains,
        vec![vec![6, 18], vec![6, 18, 54], vec![6, 18, 54, 162]]
    );
}

/// Aggressive-biased random legal play; returns the max raises on a street.
fn play(st: &mut HandState, rng: &mut PokerbotRng, aggressive: bool) -> usize {
    let total = u64::from(st.starting_stack(Seat::Zero)) + u64::from(st.starting_stack(Seat::One));
    let mut max_raises = 0;
    let mut raises = 0;
    let mut street = st.street();
    while !st.is_terminal() {
        let legal = st.legal_actions();
        assert!(!legal.is_empty());
        let seat = st.to_act().unwrap();
        let mut prev_to = 0;
        for w in legal.windows(2) {
            assert!(w[0].kind < w[1].kind, "canonical order");
        }
        for a in &legal {
            assert!(a.chips_to_commit <= st.stack(seat));
            if a.kind.is_aggressive() {
                assert!(a.raise_to > prev_to, "sized options strictly increase");
                prev_to = a.raise_to;
                if a.kind != AllIn {
                    assert!(a.raise_to < st.street_contribution(seat) + st.stack(seat));
                    let opp = st.street_contribution(seat.other());
                    assert!(a.raise_to - opp >= st.last_full_raise().max(2), "min raise");
                    // Each sized raise at least doubles the street's high bet.
                    assert!(a.raise_to >= 2 * opp.max(1));
                }
            }
        }
        // Aggressive mode: 3/4 of the time take the smallest bet/raise.
        let smallest_raise = legal.iter().copied().find(|a| a.kind.is_aggressive());
        let a = match smallest_raise {
            Some(r) if aggressive && rng.gen_range(4) != 0 => r,
            _ => legal[rng.gen_range(legal.len() as u64) as usize],
        };
        if st.street() != street {
            street = st.street();
            raises = 0;
        }
        if a.kind.is_aggressive() {
            raises += 1;
            max_raises = max_raises.max(raises);
        }
        st.apply_legal(&a).unwrap();
        let stacks: u64 = Seat::ALL.iter().map(|&s| u64::from(st.stack(s))).sum();
        assert_eq!(stacks + u64::from(st.pot()), total, "conservation");
    }
    let p = st.payoffs().unwrap();
    assert_eq!(p[0] + p[1], 0, "zero-sum");
    max_raises
}

#[test]
fn depth_fuzz_all_depths_conservation_and_history_bound() {
    let mut rng = PokerbotRng::seed_from_u64(0xDE97);
    let mut max_len = Vec::new();
    let mut max_raises = Vec::new();
    for d in StackDepth::ALL {
        let s = d.starting_stack();
        // Each bet/raise at least doubles the street's high contribution
        // (>= 2 chips), so a street has at most floor(log2(s)) + 1
        // aggressive actions plus 2 passive ones; 4 streets.
        let per_street = (u32::BITS - s.leading_zeros()) as usize + 2;
        let bound = 4 * per_street;
        let (mut longest, mut most_raises) = (0, 0);
        for i in 0..3_000u64 {
            let button = Seat::ALL[(i % 2) as usize];
            let mut st = HandState::new(cfg(d, button), Deck::shuffled(&mut rng)).unwrap();
            let r = play(&mut st, &mut rng, i % 3 == 0);
            assert!(
                st.history().len() <= bound,
                "{} > {bound}",
                st.history().len()
            );
            longest = longest.max(st.history().len());
            most_raises = most_raises.max(r);
        }
        max_len.push(longest);
        max_raises.push(most_raises);
    }
    println!("depth fuzz: longest history {max_len:?}, most raises on a street {max_raises:?}");
    // Deeper stacks allow longer raise wars.
    assert!(max_raises[0] <= max_raises[1] && max_raises[1] <= max_raises[2]);
    assert!(max_raises[2] > max_raises[0], "{max_raises:?}");
}

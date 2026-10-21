//! M-04 seeded random-legal fuzz: invariants after every action, illegal
//! actions never mutate, determinism, and the hidden-information boundary.

use pokerbot_core::card::{Card, CardSet};
use pokerbot_core::deck::Deck;
use pokerbot_core::rng::PokerbotRng;
use pokerbot_core::rules::{ActionKind, RulesConfig, StackDepth};
use pokerbot_core::state::{HandConfig, HandState, LegalAction, Seat, Street, TerminalReason};

fn initial_total(st: &HandState) -> u64 {
    Seat::ALL
        .iter()
        .map(|&s| u64::from(st.starting_stack(s)))
        .sum()
}

/// Checks every per-state invariant.
fn check_state(st: &HandState) {
    let total = initial_total(st);
    let stacks: u64 = Seat::ALL.iter().map(|&s| u64::from(st.stack(s))).sum();
    assert_eq!(stacks + u64::from(st.pot()), total, "chip conservation");

    let legal = st.legal_actions();
    assert_eq!(
        legal.is_empty(),
        st.is_terminal(),
        "legal empty iff terminal"
    );
    assert_eq!(st.to_act().is_none(), st.is_terminal());
    for a in &legal {
        let seat = st.to_act().unwrap();
        assert!(a.chips_to_commit <= st.stack(seat), "no negative stack");
        assert_eq!(a.raise_to, st.street_contribution(seat) + a.chips_to_commit);
    }

    let board_len = st.board().len();
    match st.street() {
        Street::Preflop => assert_eq!(board_len, 0),
        Street::Flop => assert_eq!(board_len, 3),
        Street::Turn => assert_eq!(board_len, 4),
        Street::River => assert_eq!(board_len, 5),
        Street::Showdown => match st.terminal_reason() {
            Some(TerminalReason::Showdown) => assert_eq!(board_len, 5),
            Some(TerminalReason::Fold { .. }) => assert!([0, 3, 4, 5].contains(&board_len)),
            None => panic!("Showdown street must be terminal"),
        },
    }

    let mut cards: Vec<Card> = Vec::new();
    for s in Seat::ALL {
        cards.extend(st.hole_cards(s));
    }
    cards.extend_from_slice(st.board());
    assert!(CardSet::try_from_cards(&cards).is_ok(), "duplicate card");

    check_history(st);

    if st.is_terminal() {
        let p = st.payoffs().unwrap();
        assert_eq!(p[0] + p[1], 0, "zero-sum");
        let f = st.final_stacks().unwrap();
        for s in Seat::ALL {
            assert_eq!(
                p[s.index()],
                i64::from(f[s.index()]) - i64::from(st.starting_stack(s))
            );
        }
        assert_eq!(st.pot(), 0);
        assert_eq!(
            st.showdown_hands().is_some(),
            st.terminal_reason() == Some(TerminalReason::Showdown)
        );
    } else {
        assert!(st.payoffs().is_none() && st.showdown_hands().is_none());
        for s in Seat::ALL {
            assert!(st.street_contribution(s) <= st.total_contribution(s));
        }
    }
}

/// Recomputes contributions from blinds + history and checks consistency.
fn check_history(st: &HandState) {
    let rules = st.config().rules;
    let btn = st.button();
    let mut total = [0u64; 2];
    let mut street_c = [0u32; 2];
    total[btn.index()] = u64::from(rules.small_blind.min(st.starting_stack(btn)));
    total[btn.other().index()] = u64::from(rules.big_blind.min(st.starting_stack(btn.other())));
    street_c[0] = total[0] as u32;
    street_c[1] = total[1] as u32;
    let mut cur = Street::Preflop;
    let mut last_seat = None;
    for (i, r) in st.history().iter().enumerate() {
        assert!(r.street >= cur, "streets non-decreasing");
        assert!(r.street != Street::Showdown);
        if r.street != cur {
            cur = r.street;
            street_c = [0, 0];
            last_seat = None;
        }
        if let Some(ls) = last_seat {
            assert_ne!(ls, r.seat, "players alternate within a street");
        }
        last_seat = Some(r.seat);
        assert_eq!(
            r.raise_to,
            street_c[r.seat.index()] + r.chips_committed,
            "record {i}"
        );
        if r.kind == ActionKind::Fold {
            assert_eq!(r.chips_committed, 0);
            assert_eq!(i + 1, st.history().len(), "fold ends the hand");
        }
        street_c[r.seat.index()] = r.raise_to;
        total[r.seat.index()] += u64::from(r.chips_committed);
    }
    for s in Seat::ALL {
        assert_eq!(total[s.index()], u64::from(st.total_contribution(s)));
    }
}

/// Every non-legal kind (and a perturbed legal action) is rejected with no
/// mutation.
fn check_rejections(st: &HandState) {
    let legal = st.legal_actions();
    for kind in ActionKind::ALL {
        if legal.iter().any(|a| a.kind == kind) {
            continue;
        }
        let mut c = st.clone();
        assert!(c.apply(kind).is_err());
        assert_eq!(&c, st, "rejected {kind:?} mutated state");
    }
    if let Some(a) = legal.first() {
        let bad = LegalAction {
            raise_to: a.raise_to + 1,
            chips_to_commit: a.chips_to_commit + 1,
            ..*a
        };
        let mut c = st.clone();
        assert!(c.apply_legal(&bad).is_err());
        assert_eq!(&c, st);
    }
}

/// Plays one random legal hand, checking invariants at each step; returns
/// the chosen action indices.
fn play_random(st: &mut HandState, rng: &mut PokerbotRng) -> Vec<usize> {
    let mut choices = Vec::new();
    check_state(st);
    while !st.is_terminal() {
        check_rejections(st);
        let legal = st.legal_actions();
        // Random legal action, biased towards passive play (2/3 of the
        // time re-draw an aggressive pick among passive ones) so that many
        // hands reach the river as well as all-in runouts.
        let mut i = rng.gen_range(legal.len() as u64) as usize;
        let passive: Vec<usize> = (0..legal.len())
            .filter(|&j| !legal[j].kind.is_aggressive() && legal[j].kind != ActionKind::Fold)
            .collect();
        if legal[i].kind.is_aggressive() && !passive.is_empty() && rng.gen_range(3) != 0 {
            i = passive[rng.gen_range(passive.len() as u64) as usize];
        }
        choices.push(i);
        let mut via_kind = st.clone();
        st.apply_legal(&legal[i]).unwrap();
        via_kind.apply(legal[i].kind).unwrap();
        assert_eq!(&via_kind, st, "apply and apply_legal agree");
        check_state(st);
        assert!(choices.len() < 200, "hand did not terminate");
    }
    check_rejections(st);
    choices
}

fn replay(config: HandConfig, seed: u64, choices: &[usize]) -> HandState {
    let mut st = HandState::from_seed(config, seed).unwrap();
    for &i in choices {
        let a = st.legal_actions()[i];
        st.apply_legal(&a).unwrap();
    }
    st
}

#[test]
fn invariants_random_hands_all_depths_and_buttons() {
    let mut rng = PokerbotRng::seed_from_u64(0x1A7A);
    let mut showdowns = 0;
    let mut folds = 0;
    let mut runouts = 0;
    for depth in StackDepth::ALL {
        for button in Seat::ALL {
            let config = HandConfig {
                rules: RulesConfig::standard(depth),
                button,
            };
            for _ in 0..2_000 {
                let seed = rng.next_u64();
                let mut st = HandState::from_seed(config, seed).unwrap();
                let choices = play_random(&mut st, &mut rng);
                match st.terminal_reason() {
                    Some(TerminalReason::Showdown) => {
                        showdowns += 1;
                        if st
                            .history()
                            .last()
                            .is_some_and(|r| r.street != Street::River)
                        {
                            runouts += 1;
                        }
                    }
                    _ => folds += 1,
                }
                assert_eq!(replay(config, seed, &choices), st, "deterministic replay");
            }
        }
    }
    println!(
        "hands: {} showdowns ({runouts} all-in runouts), {folds} folds",
        showdowns
    );
    // The fuzz must exercise every ending.
    assert!(
        showdowns > 500 && folds > 500 && runouts > 100,
        "{showdowns} {folds} {runouts}"
    );
}

#[test]
fn invariants_unequal_and_tiny_stacks() {
    let mut rng = PokerbotRng::seed_from_u64(0x57AC);
    for _ in 0..3_000 {
        let s = rng.gen_range(3) as usize;
        let depth = StackDepth::ALL[s];
        let button = Seat::ALL[rng.gen_range(2) as usize];
        let config = HandConfig {
            rules: RulesConfig::standard(depth),
            button,
        };
        let stacks = [1 + rng.gen_range(250) as u32, 1 + rng.gen_range(250) as u32];
        let mut st = HandState::new_with_stacks(config, Deck::shuffled(&mut rng), stacks).unwrap();
        play_random(&mut st, &mut rng);
    }
}

/// Builds the hand from an explicit 9+ card deck order.
fn from_order(config: HandConfig, order: &[Card]) -> HandState {
    HandState::new(config, Deck::from_order(order.to_vec()).unwrap()).unwrap()
}

#[test]
fn invariants_observation_independent_of_hidden_cards() {
    let mut rng = PokerbotRng::seed_from_u64(0x0B5E);
    let mut compared = 0u64;
    for h in 0..3_000u64 {
        let depth = StackDepth::ALL[(h % 3) as usize];
        let button = Seat::ALL[(h / 3 % 2) as usize];
        let observer = Seat::ALL[(h / 6 % 2) as usize];
        let config = HandConfig {
            rules: RulesConfig::standard(depth),
            button,
        };
        let order = Deck::shuffled(&mut rng).order().to_vec();
        // Deck positions of the opponent's hole cards (BB gets 0 and 2).
        let opp = observer.other();
        let opp_pos = if opp == button { [1, 3] } else { [0, 2] };
        let mut swap_hole = order.clone();
        swap_hole.swap(opp_pos[0], 9);
        swap_hole.swap(opp_pos[1], 10);
        let mut swap_river = order.clone();
        swap_river.swap(8, 11);
        let mut swap_turn_river = order.clone();
        swap_turn_river.swap(7, 12);
        swap_turn_river.swap(8, 13);

        let mut base = from_order(config, &order);
        // (variant, compare while fewer than this many board cards are shown)
        let mut variants = [
            (from_order(config, &swap_hole), 6),
            (from_order(config, &swap_river), 5),
            (from_order(config, &swap_turn_river), 4),
        ];
        loop {
            let ob = base.observation(observer);
            let showdown_terminal = base.terminal_reason() == Some(TerminalReason::Showdown);
            for (v, limit) in &variants {
                // Compare while the swapped cards are still hidden.
                if !showdown_terminal && base.board().len() < *limit {
                    assert_eq!(
                        v.observation(observer),
                        ob,
                        "observation leaked hidden cards"
                    );
                    compared += 1;
                }
            }
            if base.is_terminal() {
                break;
            }
            if base.to_act() == Some(observer) {
                assert_eq!(ob.legal_actions, base.legal_actions());
            } else {
                assert!(ob.legal_actions.is_empty());
            }
            assert!(ob.opponent_hole_cards.is_none());
            let legal = base.legal_actions();
            let a = legal[rng.gen_range(legal.len() as u64) as usize];
            base.apply_legal(&a).unwrap();
            for (v, _) in variants.iter_mut() {
                v.apply_legal(&a).unwrap();
            }
        }
        let ob = base.observation(observer);
        match base.terminal_reason() {
            Some(TerminalReason::Showdown) => {
                assert_eq!(ob.opponent_hole_cards, Some(base.hole_cards(opp)))
            }
            _ => assert_eq!(ob.opponent_hole_cards, None),
        }
    }
    println!("observation comparisons: {compared}");
    assert!(compared > 20_000, "compared {compared}");
}

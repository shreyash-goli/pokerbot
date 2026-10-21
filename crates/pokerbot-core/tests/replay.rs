//! M-06 acceptance tests: event streams, viewer visibility, hand-log
//! replay, text format, and public-state reconstruction.

use pokerbot_core::card::{Card, parse_cards};
use pokerbot_core::deck::Deck;
use pokerbot_core::events::{HandEvent, Viewer};
use pokerbot_core::observation::PlayerObservation;
use pokerbot_core::replay::{HandLog, ReplayError, rebuild_public, replay};
use pokerbot_core::rng::PokerbotRng;
use pokerbot_core::rules::{ActionKind, RulesConfig, StackDepth};
use pokerbot_core::state::{HandConfig, HandState, LegalAction, Seat, TerminalReason};

use ActionKind::*;

fn cfg(depth: StackDepth, button: Seat) -> HandConfig {
    HandConfig {
        rules: RulesConfig::standard(depth),
        button,
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

fn pick(st: &HandState, rng: &mut PokerbotRng) -> LegalAction {
    let legal = st.legal_actions();
    let mut i = rng.gen_range(legal.len() as u64) as usize;
    // Bias towards passive play so many hands reach later streets.
    if legal[i].kind.is_aggressive() && rng.gen_range(3) != 0 {
        if let Some(j) = legal.iter().position(|a| matches!(a.kind, Check | Call)) {
            i = j;
        }
    }
    legal[i]
}

fn hole_events(ev: &[HandEvent]) -> Vec<Seat> {
    ev.iter()
        .filter_map(|e| match e {
            HandEvent::HoleCards { seat, .. } => Some(*seat),
            _ => None,
        })
        .collect()
}

/// At every prefix: event-rebuilt observations/public views equal the
/// state's own, for every viewer.
fn check_prefix(st: &HandState) {
    for seat in Seat::ALL {
        let ev = st.events_for(Viewer::Seat(seat));
        assert_eq!(hole_events(&ev), vec![seat], "seat sees only its own deal");
        let rebuilt = PlayerObservation::from_events(seat, &ev).unwrap();
        assert_eq!(rebuilt, st.observation(seat));
    }
    let spec = st.events_for(Viewer::Spectator);
    assert!(hole_events(&spec).is_empty());
    assert_eq!(rebuild_public(&spec).unwrap(), st.public_view());
    assert_eq!(rebuild_public(&st.events()).unwrap(), st.public_view());
    assert_eq!(st.events_for(Viewer::Omniscient), st.events());
}

#[test]
fn replay_fuzz_log_roundtrip_and_event_rebuild() {
    let mut rng = PokerbotRng::seed_from_u64(0x2E91);
    let (mut showdowns, mut folds) = (0, 0);
    for depth in StackDepth::ALL {
        for button in Seat::ALL {
            let config = cfg(depth, button);
            for _ in 0..700 {
                let seed = rng.next_u64();
                let d = Deck::from_seed(seed);
                let s = config.rules.starting_stack;
                let mut st = HandState::new(config, d.clone()).unwrap();
                check_prefix(&st);
                while !st.is_terminal() {
                    let a = pick(&st, &mut rng);
                    st.apply_legal(&a).unwrap();
                    check_prefix(&st);
                }
                match st.terminal_reason() {
                    Some(TerminalReason::Showdown) => showdowns += 1,
                    _ => folds += 1,
                }
                let log = HandLog::capture(&st, [s, s], &d).unwrap();
                assert_eq!(log, {
                    let mut l = HandLog::from_seed(config, seed).unwrap();
                    l.actions = log.actions.clone();
                    l
                });
                assert_eq!(replay(&log).unwrap(), st);
                let text = log.to_text();
                let parsed = HandLog::parse(&text).unwrap();
                assert_eq!(parsed, log);
                assert_eq!(parsed.to_string(), text, "deterministic text output");
                assert_eq!(parsed.replay().unwrap(), st);
            }
        }
    }
    println!("replay fuzz: {showdowns} showdowns, {folds} folds");
    assert!(showdowns > 300 && folds > 300);
}

#[test]
fn replay_unequal_stacks_roundtrip() {
    let mut rng = PokerbotRng::seed_from_u64(0x77);
    for _ in 0..1_000 {
        let config = cfg(StackDepth::Bb100, Seat::ALL[rng.gen_range(2) as usize]);
        let stacks = [1 + rng.gen_range(300) as u32, 1 + rng.gen_range(300) as u32];
        let d = Deck::shuffled(&mut rng);
        let mut st = HandState::new_with_stacks(config, d.clone(), stacks).unwrap();
        check_prefix(&st);
        while !st.is_terminal() {
            let a = pick(&st, &mut rng);
            st.apply_legal(&a).unwrap();
            check_prefix(&st);
        }
        let log = HandLog::capture(&st, stacks, &d).unwrap();
        assert_eq!(
            HandLog::parse(&log.to_text()).unwrap().replay().unwrap(),
            st
        );
    }
}

#[test]
fn replay_seat_stream_hides_opponent_and_undealt_cards() {
    let mut rng = PokerbotRng::seed_from_u64(0x41DE);
    let mut compared = 0;
    for h in 0..1_500u64 {
        let config = cfg(
            StackDepth::ALL[(h % 3) as usize],
            Seat::ALL[(h / 3 % 2) as usize],
        );
        let observer = Seat::ALL[(h / 6 % 2) as usize];
        let opp = observer.other();
        let order = Deck::shuffled(&mut rng).order().to_vec();
        let opp_pos = if opp == config.button { [1, 3] } else { [0, 2] };
        let mut hole = order.clone();
        hole.swap(opp_pos[0], 9);
        hole.swap(opp_pos[1], 10);
        let mut river = order.clone();
        river.swap(8, 11);
        let mut turn_river = order.clone();
        turn_river.swap(7, 12);
        turn_river.swap(8, 13);
        let mk =
            |o: &Vec<Card>| HandState::new(config, Deck::from_order(o.clone()).unwrap()).unwrap();
        let mut base = mk(&order);
        // (variant, compare while fewer than this many board cards are shown)
        let mut variants = [(mk(&hole), 6), (mk(&river), 5), (mk(&turn_river), 4)];
        loop {
            let showdown = base.terminal_reason() == Some(TerminalReason::Showdown);
            let ev = base.events_for(Viewer::Seat(observer));
            let spec = base.events_for(Viewer::Spectator);
            for (v, limit) in &variants {
                if !showdown && base.board().len() < *limit {
                    assert_eq!(
                        v.events_for(Viewer::Seat(observer)),
                        ev,
                        "seat stream leaked"
                    );
                    assert_eq!(
                        v.events_for(Viewer::Spectator),
                        spec,
                        "spectator stream leaked"
                    );
                    compared += 1;
                }
            }
            if !showdown {
                assert!(!ev.iter().any(|e| matches!(e, HandEvent::Showdown { .. })));
                for e in &ev {
                    if let HandEvent::BoardDealt { cards, .. } = e {
                        assert!(
                            cards.iter().all(|c| base.board().contains(c)),
                            "undealt card"
                        );
                    }
                }
            }
            if base.is_terminal() {
                break;
            }
            let a = pick(&base, &mut rng);
            base.apply_legal(&a).unwrap();
            for (v, _) in variants.iter_mut() {
                v.apply_legal(&a).unwrap();
            }
        }
    }
    assert!(compared > 10_000, "compared {compared}");
}

#[test]
fn replay_fold_never_reveals_and_showdown_reveals_both() {
    let d = deck("Qh Qd", "Kh Kd", "2c 7d 9h Js 3c");
    let mut st = HandState::new(cfg(StackDepth::Bb100, Seat::Zero), d.clone()).unwrap();
    for k in [Call, Check, Pot, Fold] {
        st.apply(k).unwrap();
    }
    for seat in Seat::ALL {
        let ev = st.events_for(Viewer::Seat(seat));
        assert_eq!(hole_events(&ev), vec![seat]);
        assert!(!ev.iter().any(|e| matches!(e, HandEvent::Showdown { .. })));
        assert_eq!(
            PlayerObservation::from_events(seat, &ev)
                .unwrap()
                .opponent_hole_cards,
            None
        );
    }
    let spec = st.events_for(Viewer::Spectator);
    assert!(hole_events(&spec).is_empty());
    assert!(matches!(
        spec.last(),
        Some(HandEvent::HandEnded {
            reason: TerminalReason::Fold { winner: Seat::One },
            ..
        })
    ));

    let mut st = HandState::new(cfg(StackDepth::Bb100, Seat::Zero), d).unwrap();
    st.apply(AllIn).unwrap();
    st.apply(Call).unwrap();
    let spec = st.events_for(Viewer::Spectator);
    let shown = spec
        .iter()
        .find_map(|e| match e {
            HandEvent::Showdown { hands } => Some(*hands),
            _ => None,
        })
        .expect("showdown event");
    assert_eq!(shown[0].hole_cards, st.hole_cards(Seat::Zero));
    assert_eq!(shown[1].hole_cards, st.hole_cards(Seat::One));
    let boards: usize = spec
        .iter()
        .filter(|e| matches!(e, HandEvent::BoardDealt { .. }))
        .count();
    assert_eq!(boards, 3, "flop, turn, river dealt during the runout");
    let obs =
        PlayerObservation::from_events(Seat::One, &st.events_for(Viewer::Seat(Seat::One))).unwrap();
    assert_eq!(obs.opponent_hole_cards, Some(st.hole_cards(Seat::Zero)));
}

#[test]
fn replay_events_deterministic_and_ordered() {
    let config = cfg(StackDepth::Bb50, Seat::One);
    let mut a = HandState::from_seed(config, 99).unwrap();
    let mut b = HandState::from_seed(config, 99).unwrap();
    for k in [Call, Check, Check, Check] {
        a.apply(k).unwrap();
        b.apply(k).unwrap();
    }
    assert_eq!(a.events(), b.events());
    let ev = a.events();
    assert!(matches!(
        ev[0],
        HandEvent::HandStarted {
            blinds_posted: [2, 1],
            ..
        }
    ));
    assert!(matches!(
        ev[1],
        HandEvent::HoleCards {
            seat: Seat::Zero,
            ..
        }
    )); // BB dealt first
    assert!(matches!(
        ev[2],
        HandEvent::HoleCards {
            seat: Seat::One,
            ..
        }
    ));
    // Call, Check, Flop, Check, Check, Turn.
    assert!(matches!(
        ev[5],
        HandEvent::BoardDealt {
            street: pokerbot_core::state::Street::Flop,
            ..
        }
    ));
    assert!(matches!(
        ev[8],
        HandEvent::BoardDealt {
            street: pokerbot_core::state::Street::Turn,
            ..
        }
    ));
    assert_eq!(ev.len(), 9);
    assert_eq!(
        HandLog::from_seed(config, 99).unwrap().to_text(),
        HandLog::from_seed(config, 99).unwrap().to_text()
    );
}

// ------------------------------------------------------- text format ---

fn sample_log() -> (HandLog, HandState) {
    let d = deck("Kh Jd", "Kd Qh", "Kc 7d 4h 2s 9c");
    let mut st = HandState::new(cfg(StackDepth::Bb20, Seat::Zero), d.clone()).unwrap();
    for k in [Call, Check, HalfPot, Call, Check, Check, Check, Pot, Call] {
        st.apply(k).unwrap();
    }
    (HandLog::capture(&st, [40, 40], &d).unwrap(), st)
}

#[test]
fn replay_text_format_golden() {
    let (log, st) = sample_log();
    let text = log.to_text();
    assert_eq!(
        text,
        "pokerbot-hand-log v1\nrules 1 2 40\nbutton 0\nstacks 40 40\n\
         cards Kh Kd Jd Qh Kc 7d 4h 2s 9c\n\
         action call 2 1\naction check 2 0\naction halfpot 2 2\naction call 2 2\n\
         action check 0 0\naction check 0 0\naction check 0 0\naction pot 8 8\naction call 8 8\nend\n"
    );
    assert_eq!(text.parse::<HandLog>().unwrap().replay().unwrap(), st);
}

fn parse_err(text: &str) -> ReplayError {
    HandLog::parse(text).unwrap_err()
}

#[test]
fn replay_text_rejects_malformed_and_unsupported() {
    let (log, _) = sample_log();
    let good = log.to_text();
    assert_eq!(
        parse_err(&good.replace("v1", "v2")),
        ReplayError::UnsupportedVersion("v2".into())
    );
    let malformed = [
        String::new(),
        good.replace("pokerbot-hand-log v1", "poker log"),
        good.replace("rules 1 2 40", "rules 1 2"),
        good.replace("rules 1 2 40", "rules 1 2 +40"),
        good.replace("rules 1 2 40", "rules 1 2 99999999999"),
        good.replace("button 0", "button 2"),
        good.replace("stacks 40 40", "stacks 40 -1"),
        good.replace("cards Kh", "cards Zz"),
        good.replace(" 9c\n", "\n"),
        good.replace("action pot 8 8", "action bet 8 8"),
        good.replace("action pot 8 8", "action pot 8"),
        good.replace("action pot 8 8", "action  pot 8 8"),
        good.replace("end\n", ""),
        format!("{good}action call 2 1\n"),
        good.replace("\nend", "\n\nend"),
    ];
    for t in &malformed {
        assert!(
            matches!(HandLog::parse(t), Err(ReplayError::Parse { .. })),
            "accepted:\n{t}"
        );
    }
    // Duplicate card, invalid config, invalid stack.
    assert_eq!(
        parse_err(&good.replace("cards Kh Kd", "cards Kh Kh")),
        ReplayError::DuplicateCard("Kh".parse().unwrap())
    );
    assert!(matches!(
        parse_err(&good.replace("rules 1 2 40", "rules 0 2 40")),
        ReplayError::Hand(_)
    ));
    assert!(matches!(
        parse_err(&good.replace("stacks 40 40", "stacks 0 40")),
        ReplayError::Hand(_)
    ));
}

#[test]
fn replay_rejects_tampered_logs() {
    let (log, _) = sample_log();
    let good = log.to_text();
    let tampered = [
        good.replace("action pot 8 8", "action pot 9 9"), // wrong amount
        good.replace("action halfpot 2 2", "action check 0 0"), // then "call 2 2" facing no bet
        good.replace("action call 2 1\n", "action fold 1 0\naction call 2 1\n"), // action after fold
        good.replace("action call 8 8\n", "action call 8 8\naction check 0 0\n"), // after end of hand
        good.replace("action call 2 1", "action check 1 0"), // check facing a bet
    ];
    for t in &tampered {
        let log = HandLog::parse(t).unwrap();
        assert!(
            matches!(log.replay(), Err(ReplayError::Action { .. })),
            "replayed:\n{t}"
        );
    }
    // A captured log that does not reproduce the state is rejected.
    let (_, st) = sample_log();
    assert_eq!(
        HandLog::capture(&st, [40, 40], &Deck::from_seed(1)),
        Err(ReplayError::Diverged)
    );
}

#[test]
fn replay_rejects_tampered_event_streams() {
    let (_, st) = sample_log();
    let ev = st.events_for(Viewer::Seat(Seat::Zero));
    assert!(rebuild_public(&ev).is_ok());
    let mut cases: Vec<Vec<HandEvent>> = Vec::new();
    // Tampered action amount.
    let mut t = ev.clone();
    for e in t.iter_mut() {
        match e {
            HandEvent::Action(r) if r.kind == Pot => r.raise_to += 1,
            _ => {}
        }
    }
    cases.push(t);
    // Missing flop.
    cases.push(
        ev.iter()
            .filter(|e| {
                !matches!(
                    e,
                    HandEvent::BoardDealt {
                        street: pokerbot_core::state::Street::Flop,
                        ..
                    }
                )
            })
            .cloned()
            .collect(),
    );
    // Board card duplicating a hole card.
    let mut t = ev.clone();
    for e in t.iter_mut() {
        if let HandEvent::BoardDealt {
            street: pokerbot_core::state::Street::River,
            cards,
        } = e
        {
            cards[0] = st.hole_cards(Seat::Zero)[0];
        }
    }
    cases.push(t);
    // Wrong payoffs.
    let mut t = ev.clone();
    if let Some(HandEvent::HandEnded { payoffs, .. }) = t.last_mut() {
        payoffs.swap(0, 1);
    }
    cases.push(t);
    // Showdown rank swapped between players.
    let mut t = ev.clone();
    for e in t.iter_mut() {
        if let HandEvent::Showdown { hands } = e {
            let r = hands[0].rank;
            hands[0].rank = hands[1].rank;
            hands[1].rank = r;
        }
    }
    cases.push(t);
    // Event before HandStarted / duplicated start / missing end.
    cases.push(ev[1..].to_vec());
    cases.push([vec![ev[0].clone()], ev.clone()].concat());
    cases.push(ev[..ev.len() - 1].to_vec());
    // Extra event after HandEnded.
    cases.push([ev.clone(), vec![ev[0].clone()]].concat());
    for (i, c) in cases.iter().enumerate() {
        assert!(rebuild_public(c).is_err(), "tampered stream {i} accepted");
    }
    // Observation from an omniscient stream is refused (contains opponent cards).
    assert!(PlayerObservation::from_events(Seat::Zero, &st.events()).is_err());
    // Seat stream without own hole cards is refused.
    assert!(PlayerObservation::from_events(Seat::Zero, &st.events_for(Viewer::Spectator)).is_err());
}

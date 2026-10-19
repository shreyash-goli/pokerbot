//! M-18 coach: pot odds, policy mixes at the human's own observation,
//! decision-time vs hindsight separation, and no hidden-card leakage.

mod common;

use std::cell::RefCell;

use common::*;
use mako_analysis::HandRecord;
use mako_analysis::coach::{
    self, CAVEAT, Hindsight, LineKind, MixProvider, MixSource, PolicyMix, Section,
};
use mako_analysis::equity::{self, Method};
use mako_core::action::{ActionKind, Seat, TerminalReason};
use mako_core::card::parse_cards;
use mako_core::deck::Deck;
use mako_core::observation::PlayerObservation;
use mako_core::rng::MakoRng;
use mako_core::state::HandState;

const DRY: &str = "Kc Qd 9h 4s 3c";

fn rec(st: &HandState) -> HandRecord {
    HandRecord::from_state(1, st, HUMAN, cfr_label()).unwrap()
}

/// A deterministic provider that reads only the observation and records
/// every observation it was asked about.
#[derive(Default)]
struct Probe {
    seen: RefCell<Vec<PlayerObservation>>,
    fallback: bool,
}

impl Probe {
    fn mix_for(obs: &PlayerObservation) -> [f64; 6] {
        // Weights from public fields only; normalised over legal actions.
        let mut w = [0.0; 6];
        for (i, a) in obs.legal_actions.iter().enumerate() {
            w[a.kind.index()] = 1.0 + ((obs.pot as usize + i * 7 + obs.history.len()) % 5) as f64;
        }
        let t: f64 = w.iter().sum();
        w.map(|x| x / t)
    }
}

impl MixProvider for Probe {
    fn label(&self) -> String {
        "probe policy".into()
    }
    fn checksum(&self) -> Option<u64> {
        Some(0xabc)
    }
    fn mix(&self, obs: &PlayerObservation) -> Result<PolicyMix, String> {
        self.seen.borrow_mut().push(obs.clone());
        Ok(PolicyMix {
            probs: Probe::mix_for(obs),
            source: if self.fallback {
                MixSource::Fallback("unseen infoset".into())
            } else {
                MixSource::Policy
            },
        })
    }
}

/// Whether `card` appears as a whole token (cards are printed like `Th`).
fn has_card(text: &str, card: &str) -> bool {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .any(|w| w == card)
}

fn text(lines: &[coach::CoachLine]) -> String {
    lines
        .iter()
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn coach_pot_odds_hand_computed() {
    // Hand: human button. Preflop human calls 1 into 3 (1/4); bot raises
    // half pot (4 -> to 4? BB raises to 4 total? see below); verify each.
    let st = hand(
        Seat::One,
        "Ah Ad",
        "7c 2d",
        DRY,
        &[Pot, Call, Check, HalfPot, Call, Check, Check, Check, Check],
    );
    // Human is BB: faces a pot-sized open to 6: pot 6+2 = 8, to call 4 -> 4/12 = 1/3.
    let r = coach::review(&rec(&st), None).unwrap();
    let d = &r.decisions;
    assert_eq!((d[0].pot, d[0].to_call), (8, 4));
    assert_eq!(d[0].pot_odds.unwrap().fraction(), (1, 3));
    // Flop: human checks first (nothing to call).
    assert_eq!(d[1].to_call, 0);
    assert!(d[1].pot_odds.is_none());
    // Bot bets half pot of 12 -> 6; human faces 6 into 18: 6/24 = 1/4.
    assert_eq!((d[2].pot, d[2].to_call), (18, 6));
    assert_eq!(d[2].pot_odds.unwrap().fraction(), (1, 4));
    assert!((d[2].pot_odds.unwrap().value() - 0.25).abs() < 1e-12);
    let t = text(&coach::render(&r));
    assert!(t.contains("Pot odds: call 4 into 8 = 1/3 = 33.3%"), "{t}");
    assert!(t.contains("Pot odds: n/a (nothing to call)."));
    // Button open-limp: call 1 into 3 = 1/4.
    let st = hand(
        Seat::Zero,
        "Ah Ad",
        "7c 2d",
        DRY,
        &[Call, Check, Check, Check, Check, Check, Check, Check],
    );
    let r = coach::review(&rec(&st), None).unwrap();
    assert_eq!(r.decisions[0].pot_odds.unwrap().fraction(), (1, 4));
}

#[test]
fn coach_mix_is_queried_at_the_exact_live_observation() {
    let mut rng = MakoRng::seed_from_u64(77);
    for h in 0..60u64 {
        let button = Seat::ALL[(h % 2) as usize];
        let mut st = HandState::from_seed(config(button), rng.next_u64()).unwrap();
        let mut live = Vec::new();
        while let Some(a) = st.to_act() {
            if a == HUMAN {
                live.push(st.observation(HUMAN));
            }
            let legal = st.legal_actions();
            st.apply_legal(&legal[rng.gen_range(legal.len() as u64) as usize])
                .unwrap();
        }
        let probe = Probe::default();
        let r = coach::review(&rec(&st), Some(&probe)).unwrap();
        // The provider saw exactly the human's live observations...
        assert_eq!(*probe.seen.borrow(), live, "hand {h}");
        // ...and the review reports exactly what it returned.
        for (d, obs) in r.decisions.iter().zip(&live) {
            let m = d.mix.clone().unwrap().unwrap();
            assert_eq!(m.probs, Probe::mix_for(obs));
            assert_eq!(d.legal, obs.legal_actions);
            assert_eq!(d.board, obs.board);
        }
    }
}

#[test]
fn coach_fallback_is_flagged() {
    let st = hand(Seat::Zero, "Ah Ad", "7c 2d", DRY, &[Pot, Fold]);
    let probe = Probe {
        fallback: true,
        ..Probe::default()
    };
    let lines = coach::render(&coach::review(&rec(&st), Some(&probe)).unwrap());
    let fb: Vec<_> = lines
        .iter()
        .filter(|l| l.text.contains("FALLBACK"))
        .collect();
    assert!(
        fb.iter()
            .any(|l| l.kind == LineKind::Warn && l.section == Section::DecisionTime)
    );
    assert!(text(&lines).contains("had no trained policy entry"));
}

#[test]
fn coach_without_policy_still_shows_odds_and_decisions() {
    let st = hand(Seat::One, "Ah Ad", "7c 2d", DRY, &[Pot, Fold]);
    let t = text(&coach::render(&coach::review(&rec(&st), None).unwrap()));
    assert!(t.contains("No trained policy is available"));
    assert!(t.contains("Pot odds: call 4 into 8"));
    assert!(t.contains("You chose: fold."));
    assert!(t.contains(CAVEAT));
}

fn decision_time(lines: &[coach::CoachLine]) -> Vec<coach::CoachLine> {
    lines
        .iter()
        .filter(|l| matches!(l.section, Section::Header | Section::DecisionTime))
        .cloned()
        .collect()
}

#[test]
fn coach_decision_time_is_independent_of_hidden_bot_cards() {
    let mut rng = MakoRng::seed_from_u64(0xC0AC);
    let (mut folds, mut showdowns) = (0, 0);
    for h in 0..160u64 {
        let button = Seat::ALL[(h % 2) as usize];
        let base = Deck::shuffled(&mut rng).order().to_vec();
        let bot_pos = if BOT == button { [1, 3] } else { [0, 2] };
        let mut swapped = base.clone();
        swapped.swap(bot_pos[0], 30);
        swapped.swap(bot_pos[1], 31);
        let a0 = state_from_order(button, &base);
        let choices = random_choices(&a0, &mut rng);
        let a = play_choices(a0, &choices);
        let b = play_choices(state_from_order(button, &swapped), &choices);
        let probe = Probe::default();
        let la = coach::render(&coach::review(&rec(&a), Some(&probe)).unwrap());
        let lb = coach::render(&coach::review(&rec(&b), Some(&probe)).unwrap());
        assert_eq!(decision_time(&la), decision_time(&lb), "hand {h}");
        if a.terminal_reason() == Some(TerminalReason::Showdown) {
            showdowns += 1;
        } else {
            folds += 1;
            assert_eq!(text(&la).as_bytes(), text(&lb).as_bytes(), "fold hand {h}");
            for c in a.hole_cards(BOT) {
                assert!(
                    !has_card(&text(&la), &c.to_string()),
                    "hand {h}: {c} leaked"
                );
            }
        }
    }
    assert!(
        folds > 40 && showdowns > 10,
        "{folds} folds, {showdowns} showdowns"
    );
}

#[test]
fn coach_hindsight_only_after_showdown_and_labelled() {
    let fold = hand(Seat::Zero, "Ah Ad", "7c 2d", DRY, &[Pot, Fold]);
    let r = coach::review(&rec(&fold), None).unwrap();
    assert_eq!(r.hindsight, Hindsight::NotShown);
    let t = text(&coach::render(&r));
    assert!(t.contains("Hindsight (not available at decision time):"));
    assert!(t.contains("were not shown"));
    assert!(!has_card(&t, "7c") && !has_card(&t, "2d"));

    let mut acts = vec![Call, Check];
    acts.extend([Check; 6]);
    let sd = hand(Seat::Zero, "Ah Ad", "7c 2d", DRY, &acts);
    let r = coach::review(&rec(&sd), None).unwrap();
    let Hindsight::Revealed {
        opponent_cards,
        streets,
    } = &r.hindsight
    else {
        panic!("expected revealed");
    };
    assert_eq!(opponent_cards.map(|c| c.to_string()), ["7c", "2d"]);
    assert_eq!(streets.len(), 4);
    assert_eq!(streets[3].equity.unwrap().value(), Some(1.0));
    let lines = coach::render(&r);
    for l in &lines {
        if has_card(&l.text, "7c") {
            assert_eq!(l.section, Section::Hindsight, "{}", l.text);
        }
    }
}

#[test]
fn coach_makes_no_exact_ev_or_regret_claims() {
    let mut rng = MakoRng::seed_from_u64(5);
    let probe = Probe::default();
    for h in 0..40u64 {
        let st = play_random(
            HandState::from_seed(config(Seat::ALL[(h % 2) as usize]), h).unwrap(),
            &mut rng,
        );
        let t = text(&coach::render(
            &coach::review(&rec(&st), Some(&probe)).unwrap(),
        ))
        .to_lowercase();
        for bad in [
            "ev =",
            "ev:",
            "expected value =",
            "expected value:",
            "regret =",
            "regret:",
            "optimal",
            "best action",
            "mistake",
        ] {
            assert!(!t.contains(bad), "hand {h} contains {bad:?}:\n{t}");
        }
        assert!(t.contains("no expected value and no regret"));
    }
}

#[test]
fn coach_equity_values() {
    let c = |s: &str| {
        let v = parse_cards(s).unwrap();
        [v[0], v[1]]
    };
    let board = parse_cards(DRY).unwrap();
    let e = equity::vs_hand(c("Ah Ad"), c("7c 2d"), &board).unwrap();
    assert_eq!((e.points, e.trials, e.method), (2, 1, Method::Exact));
    let royal = parse_cards("As Ks Qs Js Ts").unwrap();
    assert_eq!(
        equity::vs_hand(c("7c 2d"), c("8h 3d"), &royal)
            .unwrap()
            .value(),
        Some(0.5)
    );
    let r = equity::vs_random(c("Ah Ad"), &board).unwrap();
    assert_eq!((r.trials, r.method), (990, Method::Exact));
    let t = equity::vs_random(c("Ah Ad"), &board[..4]).unwrap();
    assert_eq!((t.trials, t.method), (46 * 990, Method::Exact));
    let flop = equity::vs_hand(c("Ah Ad"), c("7c 2d"), &board[..3]).unwrap();
    assert_eq!(flop.trials, 990);
    let pre = equity::vs_random(c("Ah Ad"), &[]).unwrap();
    assert!(matches!(pre.method, Method::Sampled { trials: 3000 }));
    let v = pre.value().unwrap();
    assert!((0.80..0.90).contains(&v), "AA vs random preflop {v}");
    assert_eq!(pre, equity::vs_random(c("Ah Ad"), &[]).unwrap(), "seeded");
    let ah = c("Ah Ad")[0];
    assert!(equity::vs_random([ah, ah], &[]).is_none());
    assert!(equity::vs_random(c("Ah Ad"), &board[..2]).is_none());
}

#[test]
fn coach_replay_frames_follow_the_hand() {
    let st = hand(
        Seat::One,
        "Ah Ad",
        "7c 2d",
        DRY,
        &[Pot, Call, Check, HalfPot, Fold],
    );
    let (frames, last) = coach::replay_frames(&rec(&st)).unwrap();
    assert_eq!(frames.len(), 5);
    assert_eq!(frames[0].action.kind, ActionKind::Pot);
    assert_eq!((frames[0].pot, frames[0].to_call), (3, 1));
    assert_eq!(frames[2].board.len(), 3);
    assert_eq!(last.terminal, st.terminal_reason());
    assert_eq!(last.opponent_hole_cards, None);
}

//! M-20: public-only opponent profile and the conservative adaptive agent.

mod common;

use common::*;
use mako_analysis::HandRecord;
use mako_analysis::opponent::{
    AdaptConfig, AdaptiveAgent, MAX_SHIFT_LIMIT, OpponentProfile, ProfileError, adjust,
};
use mako_cfr::infer::{Agent, BaselineAgent, Decision};
use mako_core::action::{ActionKind, Seat, Street};
use mako_core::deck::Deck;
use mako_core::events::Viewer;
use mako_core::rng::MakoRng;
use mako_core::state::HandState;

const DRY: &str = "Kc Qd 9h 4s 3c";

fn bot_view(st: &HandState) -> Vec<mako_core::events::HandEvent> {
    st.events_for(Viewer::Seat(BOT))
}

/// Profile with `n` copies of "human limps, then folds to a flop bet".
fn folder_profile(n: usize) -> OpponentProfile {
    let mut p = OpponentProfile::new();
    for _ in 0..n {
        let st = hand(
            Seat::Zero,
            "Ah Ad",
            "7c 2d",
            DRY,
            &[Call, Check, HalfPot, Fold],
        );
        p.observe_events(HUMAN, &bot_view(&st)).unwrap();
    }
    p
}

#[test]
fn opponent_profile_hand_computed() {
    let mut p = OpponentProfile::new();
    // H1: human button limps, bot checks; flop bot bets, human folds.
    let h1 = hand(
        Seat::Zero,
        "Ah Ad",
        "7c 2d",
        DRY,
        &[Call, Check, HalfPot, Fold],
    );
    // H2: bot button opens pot, human calls; flop human checks, bot bets,
    // human calls; checked down to showdown.
    let h2 = hand(
        Seat::One,
        "Ah Ad",
        "7c 2d",
        DRY,
        &[Pot, Call, Check, HalfPot, Call, Check, Check, Check, Check],
    );
    // H3: human button opens half pot, bot folds.
    let h3 = hand(Seat::Zero, "Ah Ad", "7c 2d", DRY, &[HalfPot, Fold]);
    for st in [&h1, &h2, &h3] {
        p.observe_events(HUMAN, &bot_view(st)).unwrap();
    }
    assert_eq!(p.hands, 3);
    assert_eq!((p.vpip.hits, p.vpip.samples), (3, 3));
    assert_eq!((p.pfr.hits, p.pfr.samples), (1, 3));
    assert_eq!(
        (
            p.fold_to_aggression[0].hits,
            p.fold_to_aggression[0].samples
        ),
        (0, 1)
    );
    assert_eq!(
        (
            p.fold_to_aggression[1].hits,
            p.fold_to_aggression[1].samples
        ),
        (1, 2)
    );
    assert_eq!(
        (
            p.call_vs_aggression[0].hits,
            p.call_vs_aggression[0].samples
        ),
        (1, 1)
    );
    assert_eq!(
        (
            p.call_vs_aggression[1].hits,
            p.call_vs_aggression[1].samples
        ),
        (1, 2)
    );
    assert_eq!(p.fold_to_aggression[2].value(), None, "no turn samples");
    // Aggression: call, fold, call, call, half-pot -> 1/5 (checks excluded).
    assert_eq!((p.aggression.hits, p.aggression.samples), (1, 5));
    assert_eq!((p.showdown.hits, p.showdown.samples), (1, 3));
    let d = p.diagnostics().join("\n");
    assert!(
        d.contains("3 hand(s)") && d.contains("flop 50.0% (n=2)"),
        "{d}"
    );
    assert!(d.contains("turn n/a (n=0)"), "{d}");
    // The same hands via the human's saved records give the same profile.
    let mut q = OpponentProfile::new();
    for st in [&h1, &h2, &h3] {
        q.observe_record(&HandRecord::from_state(1, st, HUMAN, cfr_label()).unwrap());
    }
    assert_eq!(p, q);
}

#[test]
fn opponent_profile_is_public_only() {
    let st = hand(
        Seat::Zero,
        "Ah Ad",
        "7c 2d",
        DRY,
        &[Call, Check, HalfPot, Fold],
    );
    let mut p = OpponentProfile::new();
    for v in [Viewer::Omniscient, Viewer::Seat(HUMAN)] {
        assert_eq!(
            p.observe_events(HUMAN, &st.events_for(v)),
            Err(ProfileError::HiddenInformation),
            "{v:?}"
        );
    }
    assert_eq!(p, OpponentProfile::new(), "refused streams change nothing");

    // Swapping both players' hidden cards (same actions) never changes it.
    let mut rng = MakoRng::seed_from_u64(0x9F);
    let (mut a, mut b) = (OpponentProfile::new(), OpponentProfile::new());
    for h in 0..300u64 {
        let button = Seat::ALL[(h % 2) as usize];
        let order = Deck::shuffled(&mut rng).order().to_vec();
        let mut swapped = order.clone();
        for (i, j) in [(0, 40), (1, 41), (2, 42), (3, 43)] {
            swapped.swap(i, j);
        }
        let s0 = state_from_order(button, &order);
        let choices = random_choices(&s0, &mut rng);
        let x = play_choices(s0, &choices);
        let y = play_choices(state_from_order(button, &swapped), &choices);
        a.observe_events(HUMAN, &bot_view(&x)).unwrap();
        b.observe_events(HUMAN, &bot_view(&y)).unwrap();
    }
    assert_eq!(a, b);
    assert_eq!(a.hands, 300);
}

#[test]
fn opponent_reset_clears_profile() {
    let mut p = folder_profile(5);
    assert_eq!(p.hands, 5);
    p.reset();
    assert_eq!(p, OpponentProfile::new());
    let mut agent = AdaptiveAgent::new(
        Box::new(BaselineAgent::new(1)),
        HUMAN,
        AdaptConfig {
            enabled: true,
            ..AdaptConfig::default()
        },
    )
    .unwrap();
    let st = hand(
        Seat::Zero,
        "Ah Ad",
        "7c 2d",
        DRY,
        &[Call, Check, HalfPot, Fold],
    );
    agent.observe_hand(&bot_view(&st)).unwrap();
    assert_eq!(agent.profile().hands, 1);
    agent.reset_profile();
    assert_eq!(agent.profile().hands, 0);
    assert!(agent.diagnostics()[0].contains("not evaluated as stronger"));
}

/// Plays `hands` random hands with `bot` in seat BOT and a seeded random
/// human, returning every bot decision. The bot's profile is updated after
/// each hand when it is adaptive (`observe`).
fn play_match(
    bot: &mut dyn Agent,
    observe: &mut dyn FnMut(&HandState),
    hands: u64,
    deck_seed: u64,
    human_cards_swap: bool,
) -> Vec<Decision> {
    let mut decks = MakoRng::seed_from_u64(deck_seed);
    let mut human = MakoRng::seed_from_u64(deck_seed ^ 0xFFFF);
    let mut out = Vec::new();
    for h in 0..hands {
        let button = Seat::ALL[(h % 2) as usize];
        let mut order = Deck::shuffled(&mut decks).order().to_vec();
        if human_cards_swap {
            let pos = if HUMAN == button { [1, 3] } else { [0, 2] };
            order.swap(pos[0], 44);
            order.swap(pos[1], 45);
        }
        let mut st = state_from_order(button, &order);
        while let Some(a) = st.to_act() {
            let obs = st.observation(a);
            let act = if a == BOT {
                let d = bot.act(&obs).unwrap();
                out.push(d);
                *obs.legal_actions
                    .iter()
                    .find(|l| l.kind == d.action)
                    .unwrap()
            } else {
                obs.legal_actions[human.gen_range(obs.legal_actions.len() as u64) as usize]
            };
            st.apply_legal(&act).unwrap();
        }
        observe(&st);
    }
    out
}

#[test]
fn opponent_disabled_or_undersampled_is_bit_identical() {
    for enabled in [false, true] {
        let mut base = BaselineAgent::new(42);
        let plain = play_match(&mut base, &mut |_| {}, 60, 7, false);
        let mut ad = AdaptiveAgent::new(
            Box::new(BaselineAgent::new(42)),
            HUMAN,
            AdaptConfig {
                enabled,
                min_samples: 1_000_000, // never reached
                ..AdaptConfig::default()
            },
        )
        .unwrap();
        let wrapped = play_match(&mut ad, &mut |_| {}, 60, 7, false);
        assert_eq!(plain, wrapped, "enabled={enabled}");
        assert_eq!(ad.adapt_stats().adapted, 0);
    }
}

#[test]
fn opponent_undersampled_profile_with_updates_is_bit_identical() {
    // Profile updated every hand, but min_samples is never reached.
    let mut base = BaselineAgent::new(5);
    let plain = play_match(&mut base, &mut |_| {}, 40, 9, false);
    let mut ad = AdaptiveAgent::new(
        Box::new(BaselineAgent::new(5)),
        HUMAN,
        AdaptConfig {
            enabled: true,
            min_samples: 500,
            ..AdaptConfig::default()
        },
    )
    .unwrap();
    let mut seen = Vec::new();
    let mut decisions = Vec::new();
    let mut decks = MakoRng::seed_from_u64(9);
    let mut human = MakoRng::seed_from_u64(9 ^ 0xFFFF);
    for h in 0..40u64 {
        let button = Seat::ALL[(h % 2) as usize];
        let order = Deck::shuffled(&mut decks).order().to_vec();
        let mut st = state_from_order(button, &order);
        while let Some(a) = st.to_act() {
            let obs = st.observation(a);
            let act = if a == BOT {
                let d = ad.act(&obs).unwrap();
                decisions.push(d);
                *obs.legal_actions
                    .iter()
                    .find(|l| l.kind == d.action)
                    .unwrap()
            } else {
                obs.legal_actions[human.gen_range(obs.legal_actions.len() as u64) as usize]
            };
            st.apply_legal(&act).unwrap();
        }
        ad.observe_hand(&bot_view(&st)).unwrap();
        seen.push(ad.profile().hands);
    }
    assert_eq!(plain, decisions);
    assert_eq!(*seen.last().unwrap(), 40);
}

fn enabled_cfg(seed: u64) -> AdaptConfig {
    AdaptConfig {
        enabled: true,
        min_samples: 10,
        max_shift: 0.10,
        seed,
        ..AdaptConfig::default()
    }
}

#[test]
fn opponent_shift_is_bounded_legal_and_normalised() {
    let profile = folder_profile(30);
    let cfg = enabled_cfg(1);
    let mut rng = MakoRng::seed_from_u64(3);
    let mut adapted = 0;
    for h in 0..400u64 {
        let button = Seat::ALL[(h % 2) as usize];
        let mut st = HandState::from_seed(config(button), rng.next_u64()).unwrap();
        while let Some(a) = st.to_act() {
            let obs = st.observation(a);
            if a == BOT {
                let base = BaselineAgent::strategy(&obs).unwrap();
                if let Some(adj) = adjust(&base, &obs, &profile, &cfg) {
                    adapted += 1;
                    assert_eq!(obs.street, Street::Flop, "only flop has samples");
                    let sum: f64 = adj.distribution.iter().sum();
                    assert!((sum - 1.0).abs() < 1e-9);
                    for (i, &p) in adj.distribution.iter().enumerate() {
                        let legal = obs.legal_actions.iter().any(|l| l.kind.index() == i);
                        assert!(legal || p == 0.0, "illegal slot {i} has mass");
                        assert!(p >= 0.0);
                    }
                    let tv: f64 = adj
                        .distribution
                        .iter()
                        .zip(&base)
                        .map(|(x, y)| (x - y).abs())
                        .sum::<f64>()
                        / 2.0;
                    assert!(tv <= cfg.max_shift + 1e-12, "moved {tv}");
                    assert!(adj.moved <= cfg.max_shift + 1e-12);
                    assert!(
                        adj.distribution[ActionKind::Check.index()]
                            < base[ActionKind::Check.index()]
                    );
                }
            }
            let legal = st.legal_actions();
            st.apply_legal(&legal[rng.gen_range(legal.len() as u64) as usize])
                .unwrap();
        }
    }
    assert!(adapted > 20, "only {adapted} adapted decisions");
    assert!(
        AdaptConfig {
            max_shift: MAX_SHIFT_LIMIT + 0.01,
            ..AdaptConfig::default()
        }
        .validate()
        .is_err()
    );
}

fn adaptive(seed: u64, profile_hands: usize) -> AdaptiveAgent<BaselineAgent> {
    let mut ad =
        AdaptiveAgent::new(Box::new(BaselineAgent::new(seed)), HUMAN, enabled_cfg(seed)).unwrap();
    for _ in 0..profile_hands {
        let st = hand(
            Seat::Zero,
            "Ah Ad",
            "7c 2d",
            DRY,
            &[Call, Check, HalfPot, Fold],
        );
        ad.observe_hand(&bot_view(&st)).unwrap();
    }
    ad
}

#[test]
fn opponent_enabled_is_deterministic_per_seed_and_counts_adaptations() {
    let mut a = adaptive(11, 30);
    let mut b = adaptive(11, 30);
    let da = play_match(&mut a, &mut |_| {}, 80, 4, false);
    let db = play_match(&mut b, &mut |_| {}, 80, 4, false);
    assert_eq!(da, db);
    assert!(a.adapt_stats().adapted > 0);
    assert!(
        a.label()
            .contains("adaptive (experimental; not evaluated as stronger)")
    );
    let mut c = adaptive(12, 30);
    let dc = play_match(&mut c, &mut |_| {}, 80, 4, false);
    assert_ne!(da, dc, "different seed, different stream");
}

#[test]
fn opponent_adaptation_never_reads_human_hole_cards() {
    // Same deck except the human's hole cards; the human's random choices
    // are card-independent; the profile is updated after every hand.
    let run = |swap: bool| {
        let mut ad = adaptive(21, 30);
        let mut profiles = Vec::new();
        let mut decks = MakoRng::seed_from_u64(31);
        let mut human = MakoRng::seed_from_u64(32);
        let mut decisions = Vec::new();
        for h in 0..120u64 {
            let button = Seat::ALL[(h % 2) as usize];
            let mut order = Deck::shuffled(&mut decks).order().to_vec();
            if swap {
                let pos = if HUMAN == button { [1, 3] } else { [0, 2] };
                order.swap(pos[0], 46);
                order.swap(pos[1], 47);
            }
            let mut st = state_from_order(button, &order);
            while let Some(a) = st.to_act() {
                let obs = st.observation(a);
                let act = if a == BOT {
                    let d = ad.act(&obs).unwrap();
                    decisions.push((h, d));
                    *obs.legal_actions
                        .iter()
                        .find(|l| l.kind == d.action)
                        .unwrap()
                } else {
                    obs.legal_actions[human.gen_range(obs.legal_actions.len() as u64) as usize]
                };
                st.apply_legal(&act).unwrap();
            }
            ad.observe_hand(&bot_view(&st)).unwrap();
            profiles.push(ad.profile().clone());
        }
        (decisions, profiles, ad.adapt_stats())
    };
    let (da, pa, sa) = run(false);
    let (db, pb, sb) = run(true);
    assert_eq!(da, db);
    assert_eq!(pa, pb);
    assert_eq!(sa, sb);
    assert!(sa.adapted > 0);
}

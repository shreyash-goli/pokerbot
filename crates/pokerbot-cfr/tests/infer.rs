//! M-14 policy-inference tests. Prefixed `infer_` so
//! `cargo test -p pokerbot-cfr infer` selects them.

use std::sync::atomic::AtomicBool;

use pokerbot_cfr::abstraction::key::InfoKey;
use pokerbot_cfr::abstraction::postflop::{AbstractionConfig, EquityAbstraction};
use pokerbot_cfr::artifact::{ArtifactError, ArtifactMetadata, Policy, StrategyArtifact};
use pokerbot_cfr::infer::{
    Agent, AgentError, BASELINE_LABEL, BaselineAgent, CfrAgent, DecisionSource, FallbackReason,
};
use pokerbot_cfr::infoset::StrategyTable;
use pokerbot_cfr::train::{TrainConfig, train};
use pokerbot_cfr::traverse::{Trainer, TrainerConfig};
use pokerbot_core::action::Seat;
use pokerbot_core::card::Card;
use pokerbot_core::deck::Deck;
use pokerbot_core::observation::PlayerObservation;
use pokerbot_core::rng::PokerbotRng;
use pokerbot_core::rules::{ActionKind, RulesConfig, StackDepth};
use pokerbot_core::state::{HandConfig, HandState};

const LABEL: &str = "test: tiny infer fixture, no strength claim";

/// A small artifact trained in-process with the public train API.
fn tiny_artifact(seed: u64, iterations: u64) -> Vec<u8> {
    let cfg = TrainConfig::tiny(seed, iterations);
    let out = train(&cfg, None, &AtomicBool::new(false), &mut |_| {}).unwrap();
    let meta = ArtifactMetadata::from_trainer(&out.trainer, LABEL);
    StrategyArtifact::from_table(out.trainer.table(), meta)
        .unwrap()
        .to_bytes()
}

/// An artifact with no entries: every decision must fall back.
fn empty_artifact() -> Vec<u8> {
    let t = Trainer::new(TrainerConfig::tiny(3)).unwrap();
    let meta = ArtifactMetadata::from_trainer(&t, "empty");
    StrategyArtifact::from_table(&StrategyTable::new(), meta)
        .unwrap()
        .to_bytes()
}

fn rules20() -> RulesConfig {
    RulesConfig::standard(StackDepth::Bb20)
}

fn config(i: u64) -> HandConfig {
    HandConfig {
        rules: rules20(),
        button: if i % 2 == 0 { Seat::Zero } else { Seat::One },
    }
}

fn check_distribution(obs: &PlayerObservation, dist: &[f64; 6]) {
    let legal: Vec<ActionKind> = obs.legal_actions.iter().map(|a| a.kind).collect();
    for k in ActionKind::ALL {
        let p = dist[k.index()];
        assert!(p.is_finite() && p >= 0.0);
        if !legal.contains(&k) {
            assert_eq!(p, 0.0, "mass on illegal {k:?}");
        }
    }
    let sum: f64 = dist.iter().sum();
    assert!((sum - 1.0).abs() < 1e-9, "sum {sum}");
}

#[test]
fn infer_loads_with_stored_abstraction_and_rejects_wrong_depth() {
    let bytes = tiny_artifact(1, 300);
    // The tiny abstraction is not the default one: loading with the default
    // would be an abstraction mismatch, so the agent must use the stored one.
    let default_load = Policy::load(&bytes, &rules20(), &AbstractionConfig::default());
    assert!(matches!(
        default_load,
        Err(ArtifactError::AbstractionMismatch { .. })
    ));
    let agent = CfrAgent::from_artifact_bytes(&bytes, &rules20(), 0).unwrap();
    let m = agent.metadata();
    assert_eq!(m.iterations, 300);
    assert_eq!(m.seed, 1);
    assert_eq!(m.label, LABEL);
    assert_eq!(m.rules, rules20());
    assert_eq!(m.abstraction, TrainerConfig::tiny(1).abstraction);
    assert_eq!(m.fingerprint, m.abstraction.fingerprint());
    assert_eq!(
        m.checksum,
        u64::from_le_bytes(bytes[bytes.len() - 8..].try_into().unwrap())
    );
    assert_eq!(m.depth_bb(), 20);
    assert!(agent.label().contains("20BB"), "{}", agent.label());
    assert!(agent.label().contains(LABEL));

    for depth in [StackDepth::Bb50, StackDepth::Bb100] {
        let e =
            CfrAgent::from_artifact_bytes(&bytes, &RulesConfig::standard(depth), 0).unwrap_err();
        assert!(
            matches!(e, AgentError::Artifact(ArtifactError::RulesMismatch { .. })),
            "{e:?}"
        );
    }
    let mut corrupt = bytes.clone();
    let mid = corrupt.len() / 2;
    corrupt[mid] ^= 0x55;
    assert!(matches!(
        CfrAgent::from_artifact_bytes(&corrupt, &rules20(), 0),
        Err(AgentError::Artifact(ArtifactError::ChecksumMismatch))
    ));
    assert!(CfrAgent::from_artifact_bytes(&bytes[..bytes.len() - 1], &rules20(), 0).is_err());
}

/// Plays `hands` hands with one agent per seat; checks every decision.
fn play(agents: &mut [CfrAgent; 2], hands: u64, deck_seed: u64) -> Vec<(Seat, ActionKind)> {
    let mut log = Vec::new();
    for i in 0..hands {
        let mut h = HandState::from_seed(config(i), deck_seed + i).unwrap();
        while let Some(seat) = h.to_act() {
            let obs = h.observation(seat);
            let (sdist, ssrc) = agents[seat.index()].strategy(&obs).unwrap();
            let d = agents[seat.index()].act(&obs).unwrap();
            assert_eq!(d.distribution, sdist, "strategy() matches act()");
            assert_eq!(d.source, ssrc);
            check_distribution(&obs, &d.distribution);
            assert!(
                obs.legal_actions.iter().any(|a| a.kind == d.action),
                "illegal {:?}",
                d.action
            );
            assert!(
                d.distribution[d.action.index()] > 0.0,
                "sampled a 0-prob slot"
            );
            if let DecisionSource::Fallback(r) = d.source {
                assert_eq!(r, FallbackReason::UnseenInfoset);
                let n = obs.legal_actions.len() as f64;
                for a in &obs.legal_actions {
                    assert_eq!(d.distribution[a.kind.index()], 1.0 / n, "uniform fallback");
                }
            }
            h.apply(d.action).unwrap();
            log.push((seat, d.action));
        }
    }
    log
}

#[test]
fn infer_only_legal_actions_and_counts() {
    let bytes = tiny_artifact(2, 400);
    let mut agents = [
        CfrAgent::from_artifact_bytes(&bytes, &rules20(), 10).unwrap(),
        CfrAgent::from_artifact_bytes(&bytes, &rules20(), 11).unwrap(),
    ];
    let log = play(&mut agents, 400, 7_000);
    let s = [agents[0].stats(), agents[1].stats()];
    let decisions = s[0].decisions + s[1].decisions;
    assert_eq!(decisions, log.len() as u64);
    for st in s {
        assert_eq!(st.policy_hits + st.fallbacks, st.decisions);
    }
    let hits = s[0].policy_hits + s[1].policy_hits;
    let falls = s[0].fallbacks + s[1].fallbacks;
    assert!(hits > 0, "policy used");
    assert!(falls > 0, "a 400-iteration table leaves unseen infosets");
    assert_eq!(agents[0].infer_stats(), Some(s[0]));
    eprintln!("{decisions} decisions: {hits} policy, {falls} fallback");
}

#[test]
fn infer_fallback_for_unseen_is_explicit_and_counted() {
    let bytes = empty_artifact();
    let mut agent = CfrAgent::from_artifact_bytes(&bytes, &rules20(), 5).unwrap();
    assert_eq!(agent.metadata().infosets, 0);
    let mut n = 0;
    for i in 0..30 {
        let mut h = HandState::from_seed(config(i), 300 + i).unwrap();
        while let Some(seat) = h.to_act() {
            let d = agent.act(&h.observation(seat)).unwrap();
            assert_eq!(
                d.source,
                DecisionSource::Fallback(FallbackReason::UnseenInfoset)
            );
            h.apply(d.action).unwrap();
            n += 1;
        }
    }
    let s = agent.stats();
    assert_eq!((s.decisions, s.policy_hits, s.fallbacks), (n, 0, n));
}

#[test]
fn infer_strategy_equals_artifact_lookup() {
    let bytes = tiny_artifact(4, 400);
    let agent = CfrAgent::from_artifact_bytes(&bytes, &rules20(), 0).unwrap();
    let policy = Policy::load(&bytes, &rules20(), &agent.metadata().abstraction).unwrap();
    let abs = EquityAbstraction::new(agent.metadata().abstraction).unwrap();
    let mut rng = PokerbotRng::seed_from_u64(1);
    let (mut seen, mut unseen) = (0, 0);
    for i in 0..200 {
        let mut h = HandState::from_seed(config(i), 900 + i).unwrap();
        while let Some(seat) = h.to_act() {
            let obs = h.observation(seat);
            let key = InfoKey::from_observation(&obs, &abs).unwrap();
            let (dist, src) = agent.strategy(&obs).unwrap();
            match policy.lookup(&key) {
                Some(p) => {
                    assert_eq!(src, DecisionSource::Policy);
                    assert_eq!(dist, p);
                    seen += 1;
                }
                None => {
                    assert_eq!(src, DecisionSource::Fallback(FallbackReason::UnseenInfoset));
                    unseen += 1;
                }
            }
            let legal = h.legal_actions();
            h.apply_legal(&legal[rng.gen_range(legal.len() as u64) as usize])
                .unwrap();
        }
    }
    assert!(seen > 0 && unseen > 0, "seen {seen} unseen {unseen}");
    assert_eq!(agent.stats().decisions, 0, "strategy() does not count");
}

#[test]
fn infer_deterministic_per_seed() {
    let bytes = tiny_artifact(6, 300);
    let mk = |s: u64| {
        [
            CfrAgent::from_artifact_bytes(&bytes, &rules20(), s).unwrap(),
            CfrAgent::from_artifact_bytes(&bytes, &rules20(), s + 1).unwrap(),
        ]
    };
    let a = play(&mut mk(1), 80, 50);
    let b = play(&mut mk(1), 80, 50);
    assert_eq!(a, b);
    let c = play(&mut mk(1000), 80, 50);
    assert_ne!(a, c, "different sampling seed changes play");
}

#[test]
fn infer_errors_off_turn_and_terminal() {
    let bytes = tiny_artifact(7, 100);
    let mut agent = CfrAgent::from_artifact_bytes(&bytes, &rules20(), 0).unwrap();
    let mut h = HandState::from_seed(config(0), 1).unwrap();
    let waiting = h.to_act().unwrap().other();
    assert_eq!(
        agent.act(&h.observation(waiting)).unwrap_err(),
        AgentError::NotToAct
    );
    h.apply(ActionKind::Fold).unwrap();
    assert_eq!(
        agent.act(&h.observation(Seat::Zero)).unwrap_err(),
        AgentError::Terminal
    );
    let mut forged = HandState::from_seed(config(0), 1)
        .unwrap()
        .observation(Seat::Zero);
    forged.legal_actions.retain(|a| a.kind != ActionKind::AllIn);
    assert!(matches!(
        agent.strategy(&forged),
        Err(AgentError::MaskMismatch { .. }) | Ok((_, DecisionSource::Fallback(_)))
    ));
    assert_eq!(agent.stats().decisions, 0);
}

/// Deck positions: BB hole 0,2; button hole 1,3; board 4..9.
fn own_positions(seat: Seat, button: Seat) -> [usize; 2] {
    if seat == button { [1, 3] } else { [0, 2] }
}

/// Re-shuffles the opponent's hole cards, the board beyond `visible`, and the
/// undealt deck; keeps `seat`'s own cards and the visible board.
fn reshuffle_hidden(
    order: &[Card],
    seat: Seat,
    button: Seat,
    visible: usize,
    rng: &mut PokerbotRng,
) -> Vec<Card> {
    let fixed: Vec<usize> = own_positions(seat, button)
        .into_iter()
        .chain(4..4 + visible)
        .collect();
    let free: Vec<usize> = (0..order.len()).filter(|p| !fixed.contains(p)).collect();
    let mut out = order.to_vec();
    for j in (1..free.len()).rev() {
        let r = rng.gen_range(j as u64 + 1) as usize;
        out.swap(free[j], free[r]);
    }
    out
}

#[test]
fn infer_hidden_opponent_cards_do_not_change_decisions() {
    let bytes = tiny_artifact(8, 400);
    let mut rng = PokerbotRng::seed_from_u64(77);
    let (mut compared, mut opp_changed) = (0, 0);
    for i in 0..60u64 {
        let cfg = config(i);
        let order = Deck::from_seed(4_000 + i).order().to_vec();
        // A fixed public action sequence (legality never depends on cards).
        let mut h = HandState::new(cfg, Deck::from_order(order.clone()).unwrap()).unwrap();
        let mut kinds = Vec::new();
        while !h.is_terminal() {
            let legal = h.legal_actions();
            let mut k = legal[rng.gen_range(legal.len() as u64) as usize].kind;
            if k.is_aggressive() && rng.gen_range(2) == 0 {
                k = legal[0].kind;
                if k == ActionKind::Fold {
                    k = ActionKind::Call;
                }
            }
            h.apply(k).unwrap();
            kinds.push(k);
        }
        for seat in Seat::ALL {
            for visible in [0usize, 3, 4, 5] {
                let swapped = reshuffle_hidden(&order, seat, cfg.button, visible, &mut rng);
                let mut a = HandState::new(cfg, Deck::from_order(order.clone()).unwrap()).unwrap();
                let mut b = HandState::new(cfg, Deck::from_order(swapped).unwrap()).unwrap();
                let mut agent_a = CfrAgent::from_artifact_bytes(&bytes, &rules20(), i).unwrap();
                let mut agent_b = CfrAgent::from_artifact_bytes(&bytes, &rules20(), i).unwrap();
                if a.hole_cards(seat.other()) != b.hole_cards(seat.other()) {
                    opp_changed += 1;
                }
                for &k in &kinds {
                    if a.to_act() == Some(seat) && a.board().len() <= visible {
                        let oa = a.observation(seat);
                        let ob = b.observation(seat);
                        assert_eq!(agent_a.strategy(&oa), agent_b.strategy(&ob));
                        assert_eq!(agent_a.act(&oa), agent_b.act(&ob), "hand {i}");
                        compared += 1;
                    }
                    a.apply(k).unwrap();
                    b.apply(k).unwrap();
                }
            }
        }
    }
    assert!(compared > 300, "only {compared} comparisons");
    assert!(
        opp_changed > 400,
        "swaps changed opponent cards only {opp_changed} times"
    );
    eprintln!("{compared} hidden-info decision comparisons");
}

#[test]
fn infer_baseline_agent_is_legal_and_labelled() {
    let mut agents = [BaselineAgent::new(1), BaselineAgent::new(2)];
    for i in 0..300 {
        let mut h = HandState::from_seed(config(i), 60 + i).unwrap();
        while let Some(seat) = h.to_act() {
            let obs = h.observation(seat);
            let d = agents[seat.index()].act(&obs).unwrap();
            assert_eq!(d.source, DecisionSource::Baseline);
            check_distribution(&obs, &d.distribution);
            assert_eq!(BaselineAgent::strategy(&obs).unwrap(), d.distribution);
            h.apply(d.action).unwrap();
        }
    }
    assert_eq!(agents[0].label(), BASELINE_LABEL);
    assert!(BASELINE_LABEL.contains("untrained"));
    let s = agents[0].infer_stats().unwrap();
    assert!(s.decisions > 0);
    assert_eq!((s.policy_hits, s.fallbacks), (0, 0));
}

//! Fixed-vs-adaptive comparator: information boundary, per-hand
//! observation, determinism when disabled, reported distribution, config.

mod common;

use pokerbot_analysis::opponent::{AdaptConfig, AdaptStats, AdaptiveAgent};
use pokerbot_core::events::HandEvent;
use pokerbot_core::observation::PlayerObservation;
use pokerbot_core::rules::{RulesConfig, StackDepth};
use pokerbot_core::state::Seat;
use pokerbot_eval::agent::{Agent, AgentError, CfrAgent, Decision, InferStats};
use pokerbot_eval::baselines::{AlwaysFold, CheckCall, RandomLegal};
use pokerbot_eval::config::{EvalConfig, Mode};
use pokerbot_eval::evaluate::{from_bytes, run_evaluation};
use pokerbot_eval::matchup::{MatchSpec, Participant, Plain, run_match, run_match_with};

const RULES: RulesConfig = RulesConfig::standard(StackDepth::Bb20);

fn spec(pairs: u64) -> MatchSpec {
    MatchSpec {
        rules: RULES,
        base_seed: 31,
        pairs,
        max_actions: 512,
    }
}

fn cfr(seed: u64) -> CfrAgent {
    CfrAgent::from_artifact_bytes(common::tiny_artifact(), &RULES, seed).unwrap()
}

fn adapt(enabled: bool) -> AdaptConfig {
    AdaptConfig {
        enabled,
        seed: 5,
        ..AdaptConfig::default()
    }
}

/// Records observations at decisions (inner agent) and the finished-hand
/// events delivered by the harness, and checks every adapted decision
/// reports the shifted distribution.
struct Probe {
    inner: AdaptiveAgent<CfrAgent>,
    seat: Seat,
    seen: Vec<PlayerObservation>,
    delivered: Vec<Vec<HandEvent>>,
    adapted_checked: u64,
}

impl Agent for Probe {
    fn label(&self) -> String {
        Agent::label(&self.inner)
    }
    fn act(&mut self, obs: &PlayerObservation) -> Result<Decision, AgentError> {
        self.seen.push(obs.clone());
        let (base, _) = self.inner.base().strategy(obs)?;
        let d = self.inner.act(obs)?;
        if self.inner.last_adapted() {
            assert_ne!(
                d.distribution, base,
                "adapted decision must report the shifted distribution"
            );
            assert!(d.distribution[d.action.index()] > 0.0);
            assert!((d.distribution.iter().sum::<f64>() - 1.0).abs() < 1e-9);
            self.adapted_checked += 1;
        } else {
            assert_eq!(d.distribution, base);
        }
        Ok(d)
    }
    fn infer_stats(&self) -> Option<InferStats> {
        Agent::infer_stats(&self.inner)
    }
}

impl Participant for Probe {
    fn agent(&mut self) -> &mut dyn Agent {
        self
    }
    fn label(&self) -> String {
        Agent::label(&self.inner)
    }
    fn infer_stats(&self) -> Option<InferStats> {
        Agent::infer_stats(&self.inner)
    }
    fn wants_events(&self) -> bool {
        true
    }
    fn hand_finished(&mut self, events: &[HandEvent]) -> Result<(), String> {
        self.delivered.push(events.to_vec());
        self.inner.observe_hand(events).map_err(|e| e.to_string())
    }
    fn adapt_stats(&self) -> Option<AdaptStats> {
        Some(self.inner.adapt_stats())
    }
}

fn probe(seat: Seat, enabled: bool) -> Probe {
    Probe {
        inner: AdaptiveAgent::new(Box::new(cfr(1)), seat.other(), adapt(enabled)).unwrap(),
        seat,
        seen: vec![],
        delivered: vec![],
        adapted_checked: 0,
    }
}

#[test]
fn adaptive_sees_only_its_own_observation_and_events() {
    let mut p = probe(Seat::Zero, true);
    let mut opp = RandomLegal::new(3);
    let m = run_match_with(&spec(60), &mut p, &mut Plain(&mut opp)).unwrap();
    assert!(!p.seen.is_empty());
    for obs in &p.seen {
        assert_eq!(obs.seat, p.seat);
        assert!(obs.is_my_turn());
        assert!(obs.opponent_hole_cards.is_none());
    }
    assert_eq!(p.delivered.len() as u64, m.hands());
    for ev in &p.delivered {
        let own = ev
            .iter()
            .filter(|e| matches!(e, HandEvent::HoleCards { seat, .. } if *seat == p.seat))
            .count();
        let other = ev
            .iter()
            .filter(|e| matches!(e, HandEvent::HoleCards { seat, .. } if *seat != p.seat))
            .count();
        assert_eq!((own, other), (1, 0), "own hole cards only");
        assert!(matches!(ev.last(), Some(HandEvent::HandEnded { .. })));
    }
}

#[test]
fn harness_calls_observe_hand_after_every_hand() {
    let mut a = AdaptiveAgent::new(Box::new(cfr(1)), Seat::One, adapt(true)).unwrap();
    let mut opp = CheckCall;
    let m = run_match_with(&spec(75), &mut a, &mut Plain(&mut opp)).unwrap();
    assert_eq!(m.delivered_a, 150);
    assert_eq!(m.delivered_b, 0);
    assert_eq!(a.profile().hands, 150, "profile saw every hand");
    // And as seat One.
    let mut b = AdaptiveAgent::new(Box::new(cfr(1)), Seat::Zero, adapt(true)).unwrap();
    let mut r = RandomLegal::new(1);
    let m = run_match_with(&spec(40), &mut Plain(&mut r), &mut b).unwrap();
    assert_eq!(m.delivered_b, 80);
    assert_eq!(b.profile().hands, 80);
}

#[test]
fn disabled_adaptive_is_identical_to_fixed() {
    for opp_seed in [1u64, 2] {
        let mut fixed = cfr(1);
        let mut o1 = RandomLegal::new(opp_seed);
        let f = run_match(&spec(150), &mut fixed, &mut o1).unwrap();
        let mut off = AdaptiveAgent::new(Box::new(cfr(1)), Seat::One, adapt(false)).unwrap();
        let mut o2 = RandomLegal::new(opp_seed);
        let a = run_match_with(&spec(150), &mut off, &mut Plain(&mut o2)).unwrap();
        assert_eq!(f.pairs, a.pairs);
        assert_eq!(f.counts_a, a.counts_a);
        assert_eq!(a.adapt_a.unwrap().adapted, 0);
        assert_eq!(a.delivered_a, 300, "disabled agents still observe");
    }
}

#[test]
fn adaptation_fires_and_reports_shifted_distribution() {
    // An always-folder's fold-to-aggression is 100%, so the over-folder rule
    // fires once it has min_samples observations.
    let mut p = probe(Seat::Zero, true);
    let mut f = AlwaysFold;
    let m = run_match_with(&spec(200), &mut p, &mut Plain(&mut f)).unwrap();
    let st = m.adapt_a.unwrap();
    assert!(st.adapted > 0, "{st:?}");
    assert_eq!(p.adapted_checked, st.adapted);
}

#[test]
fn adaptive_mirror_is_exactly_zero() {
    // Pair-boundary delivery keeps both agents' profiles identical (mirrored),
    // so mirror pairs stay exactly antisymmetric even while adapting.
    let mut a = AdaptiveAgent::new(Box::new(cfr(1)), Seat::One, adapt(true)).unwrap();
    let mut b = AdaptiveAgent::new(Box::new(cfr(2)), Seat::Zero, adapt(true)).unwrap();
    let m = run_match_with(&spec(150), &mut a, &mut b).unwrap();
    assert!(m.pairs.iter().all(|p| p.a_total() == 0));
    assert_eq!(m.adapt_a.unwrap().adapted, m.adapt_b.unwrap().adapted);
}

const CMP: &str = r#"
[policy]
artifact = "x.pokerbot"
[rules]
depth_bb = 20
[eval]
mode = "comparison"
candidates = ["cfr", "adaptive_cfr"]
opponents = ["random", "checkcall", "cfr", "mirror"]
base_seed = 999
deal_pairs = 40
sanity_random_vs_checkcall = false
[adaptive]
enabled = ENABLED
"#;

#[test]
fn comparison_config_parsing() {
    let c = EvalConfig::parse(&CMP.replace("ENABLED", "true")).unwrap();
    assert_eq!(c.mode, Mode::Comparison);
    assert_eq!(c.candidates, ["cfr", "adaptive_cfr"]);
    assert!(c.gate.is_none());
    let a = c.adaptive.unwrap();
    assert!(a.enabled);
    assert_eq!(a.min_samples, AdaptConfig::default().min_samples);

    let locked =
        std::fs::read_to_string(common::workspace_root().join("configs/eval_adaptive.toml"))
            .unwrap();
    let l = EvalConfig::parse(&locked).unwrap();
    assert_eq!(l.mode, Mode::Comparison);
    assert!(l.adaptive.unwrap().enabled);
    assert!(l.expected_checksum.is_some());
    assert_ne!(l.base_seed, 20_261_115);

    let base = CMP.replace("ENABLED", "true");
    let bad = [
        base.replace("[adaptive]\nenabled = true\n", ""), // adaptive candidate, no section
        base.replace(
            "sanity_random_vs_checkcall = false",
            "sanity_random_vs_checkcall = false\n[gate]\nmin_hands = 1",
        ),
        base.replace("\"adaptive_cfr\"]", "\"smart_cfr\"]"),
        base.replace("mode = \"comparison\"", "mode = \"tournament\""),
        base.replace("enabled = true", "enabled = true\nmax_shift_bp = 5000"), // > MAX_SHIFT_LIMIT
        base.replace("enabled = true", "enabled = true\nfold_threshold_bp = 0"),
        base.replace(
            "candidates = [\"cfr\", \"adaptive_cfr\"]",
            "candidates = [\"cfr\"]",
        ), // [adaptive] unused
        base.replace("mode = \"comparison\"", "mode = \"promotion\""), // promotion needs [gate]
    ];
    for (i, t) in bad.iter().enumerate() {
        assert!(EvalConfig::parse(t).is_err(), "case {i} should fail:\n{t}");
    }
    // Promotion mode without the fixed candidate is refused.
    let promo = format!(
        "{}\n[gate]\nmin_hands = 1\nrequire_positive_ci_vs = [\"random\"]\nmax_fallback_rate_bp = 100\n",
        base.replace("mode = \"comparison\"", "mode = \"promotion\"")
            .replace("[\"cfr\", \"adaptive_cfr\"]", "[\"adaptive_cfr\"]")
    );
    assert!(EvalConfig::parse(&promo).is_err());
}

#[test]
fn disabled_comparison_differences_are_exactly_zero() {
    let cfg = EvalConfig::parse(&CMP.replace("ENABLED", "false")).unwrap();
    let art = from_bytes("tiny".into(), common::tiny_artifact().to_vec(), &RULES).unwrap();
    let r = run_evaluation(&cfg, &art, None).unwrap();
    assert!(r.gate.is_none());
    // cfr: random, checkcall, mirror; adaptive: random, checkcall, cfr, mirror.
    assert_eq!(r.matchups.len(), 7);
    assert_eq!(r.comparisons.len(), 2);
    for c in &r.comparisons {
        assert_eq!(c.fixed_minus_adaptive_bb100.mean, 0.0, "{}", c.opponent_id);
        assert_eq!(c.fixed_minus_adaptive_bb100.sd, 0.0);
    }
    let h2h = r
        .matchups
        .iter()
        .find(|m| m.candidate_id == "adaptive_cfr" && m.opponent_id == "cfr")
        .unwrap();
    assert_eq!(h2h.a_net_chips, 0, "disabled adaptive vs fixed is a mirror");

    let on = EvalConfig::parse(&CMP.replace("ENABLED", "true")).unwrap();
    let r = run_evaluation(&on, &art, None).unwrap();
    let text = pokerbot_eval::output::render_text(&r);
    assert!(text.contains(
        "Adaptive mode is experimental; this comparison does not establish general strength"
    ));
    assert!(text.contains("not a promotion"));
    assert!(!text.contains("promotion gate:"));
    let json = pokerbot_eval::output::render_json(&r);
    assert!(json.contains("\"mode\":\"comparison\"") && json.contains("\"gate\":null"));
    for m in r
        .matchups
        .iter()
        .filter(|m| m.candidate_id == "adaptive_cfr")
    {
        assert_eq!(m.delivered_a, m.hands);
    }
}

//! M-22 native parity: the browser session deals the same cards and the
//! bot makes the same decisions as the native terminal game for the same
//! session seed and the same human actions.

mod common;

use common::policy_20bb;
use mako_cfr::infer::{Agent, BaselineAgent, CfrAgent};
// Only the CLI's stable items are used, so CLI refactors cannot break this.
use mako_cli::game;
use mako_core::action::{ActionKind, ActionRecord, LegalAction};
use mako_core::observation::PlayerObservation;
use mako_core::rng::MakoRng;
use mako_core::rules::{RulesConfig, StackDepth};
use mako_core::state::Match;
use mako_wasm::seeds;
use mako_wasm::session::action_name;
use mako_wasm::{BOT, GameSession, HUMAN};

#[test]
fn seats_and_seed_derivation_match_cli() {
    assert_eq!(HUMAN, game::HUMAN);
    assert_eq!(BOT, game::BOT);
    for session in [0u64, 1, 7, 42, 20261115, u64::MAX, 0xDEAD_BEEF_F00D] {
        for i in 0..64 {
            assert_eq!(
                seeds::hand_deck(session, i).order(),
                game::hand_deck(session, i).order(),
                "deck {session}/{i}"
            );
            for domain in [seeds::DOMAIN_DECK, seeds::DOMAIN_BOT, 0, 1, u64::MAX] {
                assert_eq!(
                    seeds::derive_seed(session, domain, i),
                    game::derive_seed(session, domain, i)
                );
            }
        }
        assert_eq!(
            seeds::bot_seed(session),
            game::derive_seed(session, seeds::DOMAIN_BOT, 0)
        );
    }
}

/// The CLI keeps its bot-RNG domain private; guard it at the source level
/// so a change there is caught here.
#[test]
fn cli_bot_seed_domain_matches_source() {
    let src = include_str!("../../mako-cli/src/game.rs");
    assert!(src.contains("const DOMAIN_BOT: u64 = 0x0042_4F54;"));
    assert!(src.contains("const DOMAIN_DECK: u64 = 0x4445_434B;"));
    assert!(src.contains("derive_seed(session, DOMAIN_BOT, 0)"));
    assert_eq!(seeds::DOMAIN_BOT, 0x0042_4F54);
    assert_eq!(seeds::DOMAIN_DECK, 0x4445_434B);
}

/// Scripted human: uniform random legal, or check/call.
#[derive(Clone, Copy, Debug)]
enum Policy {
    Random,
    CheckCall,
}

struct Scripted {
    policy: Policy,
    rng: MakoRng,
}

impl Scripted {
    fn new(policy: Policy, seed: u64) -> Self {
        Scripted {
            policy,
            rng: MakoRng::seed_from_u64(seed ^ 0x5EED),
        }
    }
    fn choose(&mut self, obs: &PlayerObservation) -> LegalAction {
        let l = &obs.legal_actions;
        match self.policy {
            Policy::Random => l[self.rng.gen_range(l.len() as u64) as usize],
            Policy::CheckCall => *l
                .iter()
                .find(|a| a.kind == ActionKind::Check)
                .or_else(|| l.iter().find(|a| a.kind == ActionKind::Call))
                .unwrap_or(&l[0]),
        }
    }
}

/// Per hand: the full action history and the payoff.
type Transcript = Vec<(Vec<ActionRecord>, [i64; 2])>;

/// The CLI session loop's semantics (`mako_cli::game::run_session` at
/// ad6ea07) rebuilt from library pieces: one `Match` with the button
/// starting at the human and alternating, `game::hand_deck(seed, i)` per
/// hand, one agent seeded from the session seed for the whole session that
/// sees only `observation(BOT)`, actions applied via `apply_legal`.
fn native_reference(seed: u64, hands: u64, bot: &mut dyn Agent, policy: Policy) -> Transcript {
    let mut me = Scripted::new(policy, seed);
    let mut m = Match::new(RulesConfig::standard(StackDepth::Bb20), game::HUMAN).unwrap();
    let mut out = Vec::new();
    for i in 0..hands {
        let mut h = m.start_hand(game::hand_deck(seed, i)).unwrap();
        while let Some(a) = h.to_act() {
            let obs = h.observation(a);
            let act = if a == game::BOT {
                let k = bot.act(&obs).unwrap().action;
                *obs.legal_actions.iter().find(|l| l.kind == k).unwrap()
            } else {
                me.choose(&obs)
            };
            h.apply_legal(&act).unwrap();
        }
        out.push((h.history().to_vec(), h.payoffs().unwrap()));
        m.record(&h);
    }
    out
}

/// The browser session driven by the same scripted human.
fn wasm_session(seed: u64, hands: u64, s: &mut GameSession, policy: Policy) -> Transcript {
    let mut me = Scripted::new(policy, seed);
    let mut out = Vec::new();
    for i in 0..hands {
        if i > 0 {
            s.next_hand().unwrap();
        }
        while !s.is_hand_over() {
            let obs = s.observation();
            let a = me.choose(&obs);
            s.step(action_name(a.kind)).unwrap();
        }
        let obs = s.observation();
        out.push((obs.history.clone(), obs.payoffs.unwrap()));
    }
    out
}

#[test]
fn cfr_decisions_match_native_loop() {
    let (name, bytes) = policy_20bb();
    let rules = RulesConfig::standard(StackDepth::Bb20);
    let mut actions = 0;
    for seed in [1u64, 7, 2026] {
        for policy in [Policy::Random, Policy::CheckCall] {
            let mut native = CfrAgent::from_artifact_bytes(
                &bytes,
                &rules,
                game::derive_seed(seed, seeds::DOMAIN_BOT, 0),
            )
            .unwrap();
            let want = native_reference(seed, 20, &mut native, policy);
            let mut s = GameSession::new(20, seed, Some(bytes.clone()), "cfr").unwrap();
            let got = wasm_session(seed, 20, &mut s, policy);
            assert_eq!(got, want, "seed {seed} {policy:?}");
            actions += want.iter().map(|(h, _)| h.len()).sum::<usize>();
        }
    }
    eprintln!("cfr parity ({name}): {actions} identical actions over 120 hands");
}

#[test]
fn baseline_decisions_match_native_loop() {
    for seed in 0..10u64 {
        let mut native = BaselineAgent::new(game::derive_seed(seed, seeds::DOMAIN_BOT, 0));
        let want = native_reference(seed, 15, &mut native, Policy::Random);
        let mut s = GameSession::new(20, seed, None, "baseline").unwrap();
        assert_eq!(wasm_session(seed, 15, &mut s, Policy::Random), want);
    }
}

/// FNV-1a 64 over the JSON a scripted check/call client sees: every
/// snapshot after each call plus each hand's history. The Node smoke
/// (`smoke/smoke.mjs`) computes the same digest through the wasm build.
pub fn check_call_digest(s: &mut GameSession, hands: u64) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |t: &str| {
        for b in t.bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
    };
    for i in 0..hands {
        if i > 0 {
            s.next_hand().unwrap();
        }
        eat(&s.snapshot_json());
        while !s.is_hand_over() {
            let l = s.observation().legal_actions;
            let a = l
                .iter()
                .find(|a| a.kind == ActionKind::Check)
                .or_else(|| l.iter().find(|a| a.kind == ActionKind::Call))
                .unwrap_or(&l[0]);
            s.step(action_name(a.kind)).unwrap();
            eat(&s.snapshot_json());
        }
        eat(&s.history_json());
    }
    h
}

/// Expected baseline digest for seed 7, 20 hands (no artifact needed);
/// `smoke/smoke.mjs` asserts the same constant from the wasm build.
pub const BASELINE_DIGEST_SEED7: u64 = 0x1392_4bcf_7385_351b;

#[test]
fn cross_platform_digest() {
    let mut s = GameSession::new(20, 7, None, "baseline").unwrap();
    let d = check_call_digest(&mut s, 20);
    eprintln!("native baseline digest seed 7 x20: {d:#018x}");
    if let Some(p) = common::smoke_artifact_path() {
        let mut c = GameSession::new(20, 7, Some(std::fs::read(p).unwrap()), "cfr").unwrap();
        eprintln!(
            "native cfr(smoke_20bb) digest seed 7 x20: {:#018x}",
            check_call_digest(&mut c, 20)
        );
    }
    assert_eq!(d, BASELINE_DIGEST_SEED7);
}

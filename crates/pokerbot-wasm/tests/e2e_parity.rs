//! M-24 direct native <-> wasm parity. The REAL CLI session loop
//! (`pokerbot_cli::game::run_session_with_history`, the code the `pokerbot` binary
//! runs) and the browser `GameSession` are driven with the same session
//! seed and the CLI's own scripted human (`AutoPlayer`). Every hand must
//! have identical hole cards, actions (bot decisions included), boards,
//! showdown reveals and payoffs, with the CFR artifact and the baseline.

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use common::policy_20bb;
use pokerbot_cli::args::{AutoPolicy, BotKind, HistoryChoice};
use pokerbot_cli::game::{self, AutoPlayer, HumanSeat, SessionConfig};
use pokerbot_cli::opponent;
use pokerbot_cli::ui::Ui;
use pokerbot_core::action::{ActionRecord, TerminalReason};
use pokerbot_core::card::Card;
use pokerbot_core::rules::StackDepth;
use pokerbot_wasm::session::action_name;
use pokerbot_wasm::{BOT, GameSession, HUMAN, seeds};

/// One hand as both sides must agree on it.
#[derive(Debug, PartialEq, Eq)]
struct Hand {
    human_cards: [Card; 2],
    actions: Vec<ActionRecord>,
    board: Vec<Card>,
    shown: Option<[Card; 2]>,
    payoffs: [i64; 2],
}

/// The CLI session loop, exactly as `main` wires it (no history file).
fn native(
    seed: u64,
    hands: u64,
    kind: BotKind,
    policy: Option<&Path>,
    auto: AutoPolicy,
) -> Vec<Hand> {
    let mut opp = opponent::select(kind, policy, StackDepth::Bb20, game::bot_seed(seed)).unwrap();
    let (mut recorder, notices) = pokerbot_cli::history::open(&HistoryChoice::Disabled, || None);
    let cfg = SessionConfig {
        seed,
        hands,
        depth: StackDepth::Bb20,
        ui: Ui::plain(),
        replay: String::new(),
        pace: Duration::ZERO,
        notices,
    };
    let human: HumanSeat<&[u8]> = HumanSeat::Auto(AutoPlayer::new(auto, seed));
    let mut sink = Vec::new();
    let summary =
        game::run_session_with_history(&cfg, human, &mut opp, &mut recorder, &mut sink).unwrap();
    assert_eq!(summary.hands_completed, hands);
    recorder
        .records()
        .iter()
        .map(|r| Hand {
            human_cards: r.human_cards,
            actions: r.actions.clone(),
            board: r.board.clone(),
            shown: r.opponent_cards,
            payoffs: r.payoffs,
        })
        .collect()
}

/// The browser session driven by the same scripted human.
fn wasm(seed: u64, hands: u64, mut s: GameSession, auto: AutoPolicy) -> Vec<Hand> {
    let mut me = AutoPlayer::new(auto, seed);
    let mut out = Vec::new();
    for i in 0..hands {
        if i > 0 {
            s.next_hand().unwrap();
        }
        while !s.is_hand_over() {
            let a = me.choose(&s.observation()).expect("a legal action");
            s.step(action_name(a.kind)).unwrap();
        }
        let o = s.observation();
        assert_eq!(o.seat, HUMAN);
        if o.terminal != Some(TerminalReason::Showdown) {
            assert_eq!(o.opponent_hole_cards, None, "no reveal after a fold");
        }
        out.push(Hand {
            human_cards: o.hole_cards,
            actions: o.history.clone(),
            board: o.board.clone(),
            shown: o.opponent_hole_cards,
            payoffs: o.payoffs.unwrap(),
        });
    }
    out
}

fn policy_file() -> PathBuf {
    let (_, bytes) = policy_20bb();
    let p = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("e2e-parity-{}.pokerbot", std::process::id()));
    std::fs::write(&p, bytes).unwrap();
    p
}

#[test]
fn e2e_parity_cli_loop_vs_wasm_cfr() {
    let path = policy_file();
    let bytes = std::fs::read(&path).unwrap();
    let mut bot_actions = 0;
    for seed in [3u64, 19, 20261124] {
        for auto in [AutoPolicy::CheckCall, AutoPolicy::Random] {
            let want = native(seed, 20, BotKind::Cfr, Some(&path), auto);
            let s = GameSession::new(20, seed, Some(bytes.clone()), "cfr").unwrap();
            let got = wasm(seed, 20, s, auto);
            assert_eq!(got, want, "seed {seed} {auto:?}");
            bot_actions += want
                .iter()
                .flat_map(|h| &h.actions)
                .filter(|a| a.seat == BOT)
                .count();
        }
    }
    eprintln!("cfr: {bot_actions} identical bot decisions over 120 hands");
    assert!(bot_actions > 100);
}

#[test]
fn e2e_parity_cli_loop_vs_wasm_baseline() {
    for seed in [0u64, 5, 77] {
        for auto in [AutoPolicy::CheckCall, AutoPolicy::Random] {
            let want = native(seed, 20, BotKind::Baseline, None, auto);
            let s = GameSession::new(20, seed, None, "baseline").unwrap();
            assert_eq!(wasm(seed, 20, s, auto), want, "seed {seed} {auto:?}");
        }
    }
}

/// Replaces the former `include_str!` source match: the CLI's seed domains
/// are public constants now, so equality is checked by value.
#[test]
fn e2e_parity_seed_domains_equal_cli_constants() {
    assert_eq!(seeds::DOMAIN_DECK, game::DOMAIN_DECK);
    assert_eq!(seeds::DOMAIN_BOT, game::DOMAIN_BOT);
    for s in [0u64, 7, u64::MAX] {
        assert_eq!(seeds::bot_seed(s), game::bot_seed(s));
        assert_eq!(
            seeds::bot_seed(s),
            game::derive_seed(s, game::DOMAIN_BOT, 0)
        );
    }
}

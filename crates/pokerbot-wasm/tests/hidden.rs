//! Hidden-information tests: nothing returned to JavaScript reveals
//! the bot's hole cards before a showdown terminal, nor after a fold.

mod common;

use common::{parse, random_action, tiny_artifact};
use pokerbot_cfr::infer::{Agent, BaselineAgent, InferStats};
use pokerbot_core::action::{Seat, TerminalReason};
use pokerbot_core::card::Card;
use pokerbot_core::deck::Deck;
use pokerbot_core::events::Viewer;
use pokerbot_core::rng::PokerbotRng;
use pokerbot_core::rules::{RulesConfig, StackDepth};
use pokerbot_core::state::{HandConfig, HandState};
use pokerbot_wasm::encode::{self, BotInfo, SessionInfo};
use pokerbot_wasm::seeds::hand_deck;
use pokerbot_wasm::{BOT, BotKind, GameSession, HUMAN};

/// The bot's hole cards for hand `i` of a session, computed independently
/// from the seed (button: human on even hands).
fn bot_cards(seed: u64, i: u64, rules: RulesConfig) -> [Card; 2] {
    let button = if i % 2 == 0 { HUMAN } else { BOT };
    HandState::new(HandConfig { rules, button }, hand_deck(seed, i))
        .unwrap()
        .hole_cards(BOT)
}

fn quoted(c: Card) -> String {
    format!("\"{c}\"")
}

fn assert_hidden(text: &str, bot: [Card; 2], ctx: &str) {
    for c in bot {
        assert!(
            !text.contains(&quoted(c)),
            "bot card {c} leaked ({ctx}): {text}"
        );
    }
    assert!(!text.contains("botCards"), "botCards present ({ctx})");
}

#[test]
fn bot_cards_never_leak_before_showdown_or_after_fold() {
    let rules = RulesConfig::standard(StackDepth::Bb20);
    let (mut showdowns, mut folds, mut snaps) = (0, 0, 0);
    for seed in 0..12u64 {
        for (bot, policy) in [("baseline", None), ("cfr", Some(tiny_artifact().to_vec()))] {
            let mut s = GameSession::new(20, seed, policy, bot).unwrap();
            let mut rng = PokerbotRng::seed_from_u64(seed ^ 0xABCD);
            for i in 0..15u64 {
                let bc = bot_cards(seed, i, rules);
                loop {
                    let text = s.snapshot_json();
                    let hist = s.history_json();
                    snaps += 1;
                    let v = parse(&text);
                    let term = v.get("terminal");
                    if term.is_null() {
                        assert_hidden(&text, bc, "mid-hand snapshot");
                        assert_hidden(&hist, bc, "mid-hand history");
                        s.step(&random_action(&v, &mut rng)).unwrap();
                        continue;
                    }
                    match term.get("reason").s() {
                        "fold" => {
                            folds += 1;
                            assert_hidden(&text, bc, "after fold");
                            assert_hidden(&hist, bc, "history after fold");
                        }
                        "showdown" => {
                            showdowns += 1;
                            let shown: Vec<&str> =
                                v.get("botCards").a().iter().map(|c| c.s()).collect();
                            assert_eq!(shown, bc.map(|c| c.to_string()));
                            assert_eq!(v.get("board").a().len(), 5);
                        }
                        r => panic!("unknown terminal {r}"),
                    }
                    break;
                }
                s.next_hand().unwrap();
            }
        }
    }
    assert!(
        showdowns > 20 && folds > 20,
        "{showdowns} showdowns, {folds} folds"
    );
    eprintln!("checked {snaps} snapshots: {showdowns} showdowns, {folds} folds");
}

/// Same deck except the bot's two hole cards.
fn swapped_decks(seed: u64, button: Seat) -> (Deck, Deck) {
    let a = Deck::from_seed(seed);
    let mut order = a.order().to_vec();
    // HandState deals BB, Button, BB, Button from the top.
    let (p, q) = if button == BOT { (1, 3) } else { (0, 2) };
    order.swap(p, 20);
    order.swap(q, 21);
    (a, Deck::from_order(order).unwrap())
}

fn info() -> SessionInfo {
    SessionInfo {
        hand_number: 1,
        seed: 0,
        depth_bb: 20,
        starting_stack: 40,
        hands_completed: 0,
        score: [0, 0],
        poisoned: false,
    }
}

#[test]
fn json_is_identical_when_only_bot_hidden_cards_differ() {
    let rules = RulesConfig::standard(StackDepth::Bb20);
    let bot = BotInfo {
        kind: BotKind::Baseline,
        label: "baseline".into(),
        metadata: None,
        stats_at_hand_end: InferStats::default(),
    };
    let mut compared = 0;
    for seed in 0..300u64 {
        let button = if seed % 2 == 0 { HUMAN } else { BOT };
        let (da, db) = swapped_decks(seed, button);
        let cfg = HandConfig { rules, button };
        let mut a = HandState::new(cfg, da).unwrap();
        let mut b = HandState::new(cfg, db).unwrap();
        assert_ne!(a.hole_cards(BOT), b.hole_cards(BOT));
        assert_eq!(a.hole_cards(HUMAN), b.hole_cards(HUMAN));
        assert_eq!(a.board(), b.board());
        let mut agent = BaselineAgent::new(seed);
        let mut rng = PokerbotRng::seed_from_u64(seed);
        loop {
            let oa = a.observation(HUMAN);
            let ob = b.observation(HUMAN);
            if oa.terminal == Some(TerminalReason::Showdown) {
                break; // cards legitimately revealed; payoffs may differ
            }
            let ja = encode::public_state(&oa, &info(), &bot).encode();
            let jb = encode::public_state(&ob, &info(), &bot).encode();
            assert_eq!(ja, jb, "snapshot depends on the bot's hidden cards");
            let ea = encode::events(&a.events_for(Viewer::Seat(HUMAN)), HUMAN).encode();
            let eb = encode::events(&b.events_for(Viewer::Seat(HUMAN)), HUMAN).encode();
            assert_eq!(ea, eb, "history depends on the bot's hidden cards");
            compared += 1;
            let Some(actor) = a.to_act() else { break };
            let la = a.observation(actor).legal_actions;
            assert_eq!(la, b.observation(actor).legal_actions);
            let act = if actor == BOT {
                // Baseline choices depend only on the legal set.
                let k = agent.act(&a.observation(BOT)).unwrap().action;
                *la.iter().find(|l| l.kind == k).unwrap()
            } else {
                la[rng.gen_range(la.len() as u64) as usize]
            };
            a.apply_legal(&act).unwrap();
            b.apply_legal(&act).unwrap();
        }
    }
    assert!(compared > 600, "{compared}");
}

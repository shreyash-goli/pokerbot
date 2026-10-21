//! End-to-end: a full seeded `pokerbot` session saved with `--history`
//! replays exactly through the engine. Every saved hand is rebuilt from the
//! session seed's deck and the recorded actions; results, payoffs, revealed
//! cards and the record itself must match, and the transcript's final score
//! must equal the sum of the saved payoffs. Children run with a sandboxed
//! POKERBOT_HOME/HOME (tests/common).

mod common;

use std::path::{Path, PathBuf};

use common::*;
use pokerbot_analysis::history::{self, HandRecord};
use pokerbot_cli::game::{self, BOT, HUMAN};
use pokerbot_core::action::{LegalAction, TerminalReason};
use pokerbot_core::replay::HandLog;
use pokerbot_core::state::{HandConfig, HandState};

fn history_file(name: &str) -> PathBuf {
    let p = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("e2e-replay-{}-{name}.txt", std::process::id()));
    let _ = std::fs::remove_file(&p);
    p
}

/// Runs a session and returns (transcript, saved records).
fn session(name: &str, seed: u64, hands: u64, extra: &[&str]) -> (String, Vec<HandRecord>) {
    let h = history_file(name);
    let (seed_s, hands_s) = (seed.to_string(), hands.to_string());
    let mut args = vec![
        "--seed",
        &seed_s,
        "--hands",
        &hands_s,
        "--no-color",
        "--fast",
        "--history",
        h.to_str().unwrap(),
    ];
    args.extend_from_slice(extra);
    let o = pokerbot(&args, "");
    let out = stdout(&o);
    assert!(o.status.success(), "{}\n{out}", stderr(&o));
    let text = std::fs::read_to_string(&h).expect("history written");
    let records = history::parse(&text).expect("history parses");
    let _ = std::fs::remove_file(&h);
    (out, records)
}

/// Rebuilds hand `i` from the seed and the recorded actions; asserts that
/// it reproduces the record exactly.
fn replay_record(seed: u64, i: u64, rec: &HandRecord) -> HandState {
    assert_eq!(rec.hand_number, i + 1);
    assert_eq!(rec.human_seat, HUMAN);
    let config = HandConfig {
        rules: rec.rules,
        button: rec.button,
    };
    let deck = game::hand_deck(seed, i);
    let mut st = HandState::new_with_stacks(config, deck.clone(), rec.starting_stacks).unwrap();
    for a in &rec.actions {
        assert_eq!(Some(a.seat), st.to_act(), "hand {i}");
        let la = LegalAction {
            kind: a.kind,
            raise_to: a.raise_to,
            chips_to_commit: a.chips_committed,
        };
        st.apply_legal(&la)
            .unwrap_or_else(|e| panic!("hand {i}: {e}"));
    }
    assert!(st.is_terminal(), "hand {i} ends where the record ends");
    assert_eq!(st.history(), &rec.actions[..]);
    assert_eq!(st.terminal_reason(), Some(rec.reason));
    assert_eq!(st.payoffs(), Some(rec.payoffs));
    assert_eq!(st.final_stacks(), Some(rec.final_stacks));
    assert_eq!(st.board(), &rec.board[..]);
    assert_eq!(st.hole_cards(HUMAN), rec.human_cards);
    match rec.reason {
        TerminalReason::Showdown => assert_eq!(rec.opponent_cards, Some(st.hole_cards(BOT))),
        TerminalReason::Fold { .. } => assert_eq!(rec.opponent_cards, None),
    }
    // The record is exactly what the engine derives from the human's view.
    let again = HandRecord::from_state(rec.hand_number, &st, HUMAN, rec.opponent.clone()).unwrap();
    assert_eq!(&again, rec);
    // And a pokerbot_core HandLog of the same hand replays to the same state.
    let log = HandLog::capture(&st, rec.starting_stacks, &deck).unwrap();
    assert_eq!(
        HandLog::parse(&log.to_text()).unwrap().replay().unwrap(),
        st
    );
    st
}

fn check_session(name: &str, seed: u64, hands: u64, extra: &[&str]) -> usize {
    let (out, records) = session(name, seed, hands, extra);
    assert_eq!(records.len() as u64, hands);
    let mut sum = [0i64; 2];
    let mut showdowns = 0;
    for (i, rec) in records.iter().enumerate() {
        let st = replay_record(seed, i as u64, rec);
        sum[0] += st.payoffs().unwrap()[0];
        sum[1] += st.payoffs().unwrap()[1];
        showdowns += usize::from(rec.reason == TerminalReason::Showdown);
    }
    let (you, bot) = final_score(&out);
    assert_eq!(
        (you, bot),
        (sum[HUMAN.index()], sum[BOT.index()]),
        "transcript final score equals the saved payoffs"
    );
    assert_eq!(you + bot, 0);
    showdowns
}

#[test]
fn e2e_replay_baseline_sessions() {
    let mut showdowns = 0;
    for (seed, auto) in [(11u64, "random"), (12, "checkcall")] {
        showdowns += check_session(
            &format!("base{seed}"),
            seed,
            30,
            &["--bot", "baseline", "--auto", auto],
        );
    }
    showdowns += check_session(
        "base50",
        13,
        20,
        &["--bot", "baseline", "--depth", "50", "--auto", "random"],
    );
    assert!(showdowns > 0, "some hands reached showdown");
}

#[test]
fn e2e_replay_cfr_session() {
    let p = tiny_artifact_path().to_str().unwrap().to_string();
    for (seed, auto) in [(21u64, "random"), (22, "checkcall")] {
        check_session(
            &format!("cfr{seed}"),
            seed,
            25,
            &["--bot", "cfr", "--strategy", &p, "--auto", auto],
        );
    }
}

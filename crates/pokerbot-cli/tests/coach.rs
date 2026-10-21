//! End-to-end: the between-hands `analysis` and `replay` commands.
//! Every run goes through `common::pokerbot_env`, which sandboxes `POKERBOT_HOME`
//! and `HOME` per child and adds `--no-history` unless a test picks one.

mod common;

use std::path::Path;

use common::*;
use pokerbot_cfr::infer::CfrAgent;
use pokerbot_cli::game::{HUMAN, hand_deck};
use pokerbot_cli::ui::term::{strip_ansi, visible_width};
use pokerbot_core::rules::{RulesConfig, StackDepth};
use pokerbot_core::state::{HandConfig, HandState};

fn has_card(text: &str, card: &str) -> bool {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .any(|w| w == card)
}

fn flat(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Bot cards of hand 0 (human on the button), without brackets.
fn bot_cards_hand0(seed: u64) -> Vec<String> {
    let (_, bot) = hole_cards(seed, 0);
    bot.iter()
        .map(|c| c.trim_matches(['[', ']']).to_string())
        .collect()
}

#[test]
fn analysis_and_replay_after_a_fold_hand_with_baseline() {
    // Hand 1: the human (button) folds at once; then analysis, a full
    // replay, and quit at the next prompt (hand 2 is never dealt).
    let o = pokerbot(
        &["--seed", "21", "--hands", "2", "--bot", "baseline"],
        "f\nanalysis\nreplay\n\nq\n",
    );
    assert!(o.status.success(), "{}", stderr(&o));
    let out = stdout(&o);
    let t = flat(&out);
    assert!(t.contains("Analysis of hand 1"), "{t}");
    assert!(t.contains("At decision time (only what you could see when acting):"));
    assert!(t.contains("Pot odds: call 1 into 3 = 1/4 = 25.0%"));
    assert!(t.contains("You chose: fold."));
    assert!(t.contains("No trained policy is available"));
    assert!(t.contains("Hindsight (not available at decision time):"));
    assert!(t.contains("were not shown"));
    assert!(t.contains("Replay of hand 1: 1 action(s)"));
    assert!(t.contains("Step 1/1"));
    for c in bot_cards_hand0(21) {
        assert!(!has_card(&out, &c), "bot card {c} leaked:\n{out}");
    }
}

#[test]
fn bot_cards_never_appear_in_analysis_of_fold_hands() {
    for seed in 0..16u64 {
        let s = seed.to_string();
        let o = pokerbot(
            &["--seed", &s, "--hands", "2", "--bot", "baseline"],
            "f\na\nr\n\nq\n",
        );
        let out = stdout(&o);
        assert!(out.contains("Analysis of hand 1"), "seed {seed}");
        for c in bot_cards_hand0(seed) {
            assert!(!has_card(&out, &c), "seed {seed}: bot card {c} leaked");
        }
        let low = out.to_lowercase();
        for bad in [
            "ev =",
            "ev:",
            "expected value =",
            "regret =",
            "regret:",
            "optimal",
        ] {
            assert!(!low.contains(bad), "seed {seed}: {bad:?}");
        }
    }
}

#[test]
fn policy_mix_matches_strategy_for_the_human_observation() {
    let art = tiny_artifact_path().to_str().unwrap();
    let seed = 13u64;
    let o = pokerbot(
        &[
            "--seed",
            "13",
            "--hands",
            "2",
            "--bot",
            "cfr",
            "--strategy",
            art,
        ],
        "f\na\nq\n",
    );
    assert!(o.status.success(), "{}", stderr(&o));
    let t = flat(&stdout(&o));
    // Recompute the human's first observation and the policy's strategy.
    let rules = RulesConfig::standard(StackDepth::Bb20);
    let st = HandState::new(
        HandConfig {
            rules,
            button: HUMAN,
        },
        hand_deck(seed, 0),
    )
    .unwrap();
    let obs = st.observation(HUMAN);
    let agent = CfrAgent::from_artifact_bytes(tiny_artifact_bytes(), &rules, 0).unwrap();
    let (probs, source) = agent.strategy(&obs).unwrap();
    let parts: Vec<String> = obs
        .legal_actions
        .iter()
        .map(|a| {
            format!(
                "{} {:.1}%",
                pokerbot_analysis::coach::kind_label(a.kind),
                100.0 * probs[a.kind.index()]
            )
        })
        .collect();
    let expect = parts.join(" | ");
    assert!(t.contains(&expect), "missing {expect:?} in {t}");
    let checksum = format!("{:#018x}", agent.metadata().checksum);
    assert!(t.contains(&checksum), "{t}");
    match source {
        pokerbot_cfr::infer::DecisionSource::Policy => assert!(t.contains("Policy mix:")),
        _ => assert!(t.contains("Policy FALLBACK")),
    }
}

#[test]
fn fallback_decisions_are_flagged_with_a_tiny_policy() {
    // The tiny artifact has few infosets; find a seed whose first human
    // decision is unseen and check the flag.
    let rules = RulesConfig::standard(StackDepth::Bb20);
    let agent = CfrAgent::from_artifact_bytes(tiny_artifact_bytes(), &rules, 0).unwrap();
    let seed = (0..400u64)
        .find(|&s| {
            let st = HandState::new(
                HandConfig {
                    rules,
                    button: HUMAN,
                },
                hand_deck(s, 0),
            )
            .unwrap();
            !matches!(
                agent.strategy(&st.observation(HUMAN)).unwrap().1,
                pokerbot_cfr::infer::DecisionSource::Policy
            )
        })
        .expect("some unseen preflop spot");
    let s = seed.to_string();
    let art = tiny_artifact_path().to_str().unwrap();
    let o = pokerbot(
        &[
            "--seed",
            &s,
            "--hands",
            "2",
            "--bot",
            "cfr",
            "--strategy",
            art,
        ],
        "f\na\nq\n",
    );
    let t = flat(&stdout(&o));
    assert!(t.contains("Policy FALLBACK (unseen infoset)"), "{t}");
    assert!(t.contains("had no trained policy entry"));
}

#[test]
fn replay_of_a_saved_hand_from_history() {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("pokerbot-coach-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let p = d.join("h.txt");
    let ps = p.to_str().unwrap();
    let home = d.to_str().unwrap();
    let env = [("POKERBOT_HOME", home), ("HOME", home)];
    let o = pokerbot_env(
        &[
            "--seed",
            "3",
            "--hands",
            "2",
            "--bot",
            "baseline",
            "--history",
            ps,
        ],
        "f\nq\n",
        &env,
    );
    assert!(o.status.success());
    let o = pokerbot_env(
        &[
            "--seed",
            "4",
            "--hands",
            "2",
            "--bot",
            "baseline",
            "--history",
            ps,
        ],
        "f\nreplay 1\n\nanalysis 1\nreplay 99\nq\n",
        &env,
    );
    let t = flat(&stdout(&o));
    assert!(t.contains("Replay of hand 1: 1 action(s)"), "{t}");
    assert!(t.contains("Analysis of hand 1"));
    assert!(
        t.contains("No saved hand 99; saved hands are numbered 1 to 2."),
        "{t}"
    );
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn replay_can_be_stopped_and_eof_ends_the_session() {
    // A one-action hand still prompts after its frame; 'q' stops there.
    let o = pokerbot(
        &["--seed", "8", "--hands", "2", "--bot", "baseline"],
        "f\nr\nq\nq\n",
    );
    let t = flat(&stdout(&o));
    assert!(t.contains("Replay of hand 1: 1 action(s)"), "{t}");
    assert!(t.contains("Replay stopped."), "{t}");
    assert!(t.contains("You quit the session."), "{t}");
    // End of input inside the replay ends the session cleanly.
    let o = pokerbot(
        &["--seed", "8", "--hands", "2", "--bot", "baseline"],
        "f\nr\n",
    );
    assert!(o.status.success());
    assert!(stdout(&o).contains("Input closed"));
}

#[test]
fn analysis_fits_narrow_width_and_help_lists_commands() {
    let art = tiny_artifact_path().to_str().unwrap();
    let o = pokerbot(
        &[
            "--seed",
            "13",
            "--hands",
            "2",
            "--bot",
            "cfr",
            "--strategy",
            art,
            "--width",
            "48",
        ],
        "f\nhelp\na\nr\n\nq\n",
    );
    let out = stdout(&o);
    for l in out.lines() {
        // Session-header paths (artifact location) may be long tokens; the
        // coach and replay output must fit.
        if l.contains(art) || l.contains("pokerbot-cli-") {
            continue;
        }
        assert!(visible_width(&strip_ansi(l)) <= 48, "too wide: {l:?}");
    }
    let t = flat(&out);
    assert!(t.contains("analysis (a)"), "{t}");
    assert!(t.contains("replay (r)"), "{t}");
}

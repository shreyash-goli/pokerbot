//! M-17 end-to-end: the binary records completed hands to a history file,
//! statistics persist across runs, corrupt files are quarantined, reset is
//! explicit, `--no-history` writes nothing, and the saved file never holds
//! the bot's unrevealed hole cards.
//!
//! Every run passes `--history <temp path>` / `--no-history`, or points
//! `POKERBOT_HOME` and `HOME` (on the child process only) at a temp directory.

mod common;

use std::path::{Path, PathBuf};

use common::{pokerbot_env, stdout};
use pokerbot_analysis::history::{self, HandRecord};
use pokerbot_analysis::{SessionStats, store::HISTORY_FILE_NAME};
use pokerbot_cli::game::{BOT, HUMAN, hand_deck};
use pokerbot_core::action::TerminalReason;
use pokerbot_core::rules::{RulesConfig, StackDepth};
use pokerbot_core::state::{HandConfig, HandState};

fn dir(tag: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("pokerbot-cli-history-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Runs `pokerbot --bot baseline` with `args`, `POKERBOT_HOME`/`HOME` set to
/// `home` on the child only.
fn run(home: &Path, args: &[&str], stdin: &str) -> std::process::Output {
    let mut a = vec!["--bot", "baseline"];
    a.extend_from_slice(args);
    let h = home.to_str().unwrap();
    pokerbot_env(&a, stdin, &[("POKERBOT_HOME", h), ("HOME", h)])
}

fn load(path: &Path) -> Vec<HandRecord> {
    history::parse(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn flat(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn stats_persist_across_processes() {
    let d = dir("persist");
    let p = d.join("h.txt");
    let ps = p.to_str().unwrap();
    let o = run(
        &d,
        &[
            "--seed",
            "5",
            "--hands",
            "5",
            "--auto",
            "random",
            "--history",
            ps,
        ],
        "",
    );
    assert!(o.status.success());
    let t = flat(&stdout(&o));
    assert!(t.contains("no saved hands at"), "{t}");
    assert!(t.contains("Lifetime: 5 hand(s)"), "{t}");
    assert_eq!(load(&p).len(), 5);

    let o = run(
        &d,
        &[
            "--seed",
            "6",
            "--hands",
            "5",
            "--auto",
            "random",
            "--history",
            ps,
        ],
        "",
    );
    let t = flat(&stdout(&o));
    assert!(t.contains("5 saved hand(s) loaded"), "{t}");
    assert!(t.contains("Lifetime: 10 hand(s)"), "{t}");
    let recs = load(&p);
    assert_eq!(recs.len(), 10);
    assert_eq!(
        recs.iter().map(|r| r.hand_number).collect::<Vec<_>>(),
        (1..=10).collect::<Vec<_>>()
    );

    // Third run: the `stats` command shows lifetime 10 and session 0, with
    // the same numbers the library computes from the file.
    let o = run(
        &d,
        &["--seed", "7", "--hands", "1", "--history", ps],
        "stats\nquit\n",
    );
    let t = flat(&stdout(&o));
    assert!(t.contains("This session: 0 hand(s)"), "{t}");
    assert!(t.contains("Lifetime (saved history): 10 hand(s)"), "{t}");
    let s = SessionStats::from_records(&recs);
    let net = s.human.overall.net_chips;
    let want = format!("net {}{net} chips", if net > 0 { "+" } else { "" });
    assert!(t.contains(&want), "missing {want:?} in {t}");
    assert!(t.contains("VPIP"), "{t}");
    // The quit hand is neither scored nor saved.
    assert_eq!(load(&p).len(), 10);
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn corrupt_history_is_quarantined_and_play_continues() {
    let d = dir("corrupt");
    let p = d.join("h.txt");
    let junk = "pokerbot-history v1\nhand 1\nthis is not a record\n";
    std::fs::write(&p, junk).unwrap();
    let o = run(
        &d,
        &[
            "--seed",
            "3",
            "--hands",
            "2",
            "--auto",
            "checkcall",
            "--history",
            p.to_str().unwrap(),
        ],
        "",
    );
    assert!(o.status.success());
    let t = flat(&stdout(&o));
    assert!(t.contains("History WARNING"), "{t}");
    assert!(t.contains("h.txt.corrupt-1"), "{t}");
    assert!(t.contains("Session complete."));
    assert_eq!(
        std::fs::read_to_string(d.join("h.txt.corrupt-1")).unwrap(),
        junk
    );
    assert_eq!(load(&p).len(), 2);
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn reset_stats_is_explicit_and_refuses_foreign_files() {
    let d = dir("reset");
    let p = d.join("h.txt");
    let ps = p.to_str().unwrap();
    run(
        &d,
        &[
            "--seed",
            "1",
            "--hands",
            "2",
            "--auto",
            "random",
            "--history",
            ps,
        ],
        "",
    );
    assert!(p.exists());
    let o = run(&d, &["--reset-stats", "--history", ps], "");
    assert!(o.status.success());
    assert!(stdout(&o).contains(&format!("Deleted hand history {ps}.")));
    assert!(!p.exists());
    let o = run(&d, &["--reset-stats", "--history", ps], "");
    assert!(o.status.success());
    assert!(stdout(&o).contains("nothing deleted"));

    let foreign = d.join("notes.txt");
    std::fs::write(&foreign, "keep").unwrap();
    let o = run(
        &d,
        &["--reset-stats", "--history", foreign.to_str().unwrap()],
        "",
    );
    assert!(!o.status.success());
    assert!(String::from_utf8_lossy(&o.stderr).contains("not a Pokerbot history"));
    assert_eq!(std::fs::read_to_string(&foreign).unwrap(), "keep");
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn no_history_writes_nothing() {
    let d = dir("nohist");
    let o = run(
        &d,
        &[
            "--seed",
            "2",
            "--hands",
            "3",
            "--auto",
            "random",
            "--no-history",
        ],
        "",
    );
    assert!(o.status.success());
    let t = flat(&stdout(&o));
    assert!(t.contains("not saved (--no-history)"), "{t}");
    assert!(t.contains("Lifetime: 3 hand(s)"), "{t}");
    assert_eq!(std::fs::read_dir(&d).unwrap().count(), 0, "nothing written");
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn default_path_honours_pokerbot_home_on_the_child_only() {
    let d = dir("pokerbothome");
    let o = run(&d, &["--seed", "4", "--hands", "3", "--auto", "random"], "");
    assert!(o.status.success());
    assert_eq!(load(&d.join(HISTORY_FILE_NAME)).len(), 3);
    std::fs::remove_dir_all(d).unwrap();
}

/// Bot hole cards for hand `i` (0-based) at 20BB, recomputed from the seed.
fn bot_cards(seed: u64, i: u64) -> [String; 2] {
    let config = HandConfig {
        rules: RulesConfig::standard(StackDepth::Bb20),
        button: if i % 2 == 0 { HUMAN } else { BOT },
    };
    HandState::new(config, hand_deck(seed, i))
        .unwrap()
        .hole_cards(BOT)
        .map(|c| c.to_string())
}

#[test]
fn saved_history_never_contains_unrevealed_bot_cards() {
    let d = dir("leak");
    let p = d.join("h.txt");
    let seed = 11;
    let o = run(
        &d,
        &[
            "--seed",
            "11",
            "--hands",
            "40",
            "--auto",
            "random",
            "--history",
            p.to_str().unwrap(),
        ],
        "",
    );
    assert!(o.status.success());
    let text = std::fs::read_to_string(&p).unwrap();
    let blocks: Vec<&str> = text.split("\nhand ").skip(1).collect();
    let recs = load(&p);
    assert_eq!(recs.len(), 40);
    let mut folds = 0;
    for (i, (r, block)) in recs.iter().zip(&blocks).enumerate() {
        let bot = bot_cards(seed, i as u64);
        match r.reason {
            TerminalReason::Showdown => {
                let shown = r.opponent_cards.unwrap().map(|c| c.to_string());
                assert_eq!(shown, bot, "hand {i}: shown cards are the bot's");
            }
            TerminalReason::Fold { .. } => {
                folds += 1;
                assert_eq!(r.opponent_cards, None);
                for c in &bot {
                    assert!(
                        !block.split_whitespace().any(|w| w == c),
                        "hand {i}: bot card {c} saved after a fold:\n{block}"
                    );
                }
            }
        }
    }
    assert!(folds >= 5, "only {folds} fold hands");
    std::fs::remove_dir_all(d).unwrap();
}

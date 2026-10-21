//! End-to-end: `--adapt` is off by default; when on it is labelled,
//! conservative, inspectable (`profile`), switchable (`adapt off`) and
//! resettable (`reset-profile`). All runs use `common::pokerbot`, which
//! sandboxes POKERBOT_HOME/HOME per child and adds `--no-history`.

mod common;

use common::*;

fn flat(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn bot_actions(out: &str) -> Vec<&str> {
    out.lines()
        .filter(|l| l.trim_start().starts_with("> Bot"))
        .collect()
}

#[test]
fn adapt_is_off_by_default() {
    let o = pokerbot(
        &[
            "--seed", "5", "--hands", "6", "--bot", "baseline", "--auto", "random",
        ],
        "",
    );
    let t = stdout(&o);
    assert!(o.status.success());
    let low = t.to_lowercase();
    assert!(
        !low.contains("adaptive"),
        "default mentions adaptation:\n{t}"
    );
    assert!(
        !t.contains("--adapt"),
        "replay command must not add --adapt"
    );
    // Deterministic: same command, same transcript.
    let o2 = pokerbot(
        &[
            "--seed", "5", "--hands", "6", "--bot", "baseline", "--auto", "random",
        ],
        "",
    );
    assert_eq!(t, stdout(&o2));
}

#[test]
fn adapt_is_labelled_and_conservative_before_min_samples() {
    let args = [
        "--seed", "9", "--hands", "6", "--bot", "baseline", "--auto", "random",
    ];
    let plain = stdout(&pokerbot(&args, ""));
    let mut a = args.to_vec();
    a.push("--adapt");
    let o = pokerbot(&a, "");
    assert!(o.status.success(), "{}", stderr(&o));
    let adapted = stdout(&o);
    let t = flat(&adapted);
    assert!(
        t.contains("ADAPTIVE mode: adaptive (experimental; not evaluated as stronger)"),
        "{t}"
    );
    assert!(
        t.contains("+ adaptive (experimental; not evaluated as stronger)"),
        "{t}"
    );
    assert!(t.contains("--adapt"), "replay command includes --adapt");
    // Fewer than 20 samples of any statistic: the bot plays exactly as
    // without adaptation.
    assert_eq!(bot_actions(&plain), bot_actions(&adapted));
    assert!(t.contains("0 of"), "{t}");
    assert_no_leak(&adapted, 9);
}

#[test]
fn profile_reset_and_off_commands() {
    let o = pokerbot(
        &[
            "--seed", "5", "--hands", "3", "--bot", "baseline", "--adapt",
        ],
        "f\nprofile\nreset-profile\nadapt\nadapt off\nadapt on\nq\n",
    );
    assert!(o.status.success(), "{}", stderr(&o));
    let t = flat(&stdout(&o));
    assert!(
        t.contains("Profile of your play from public actions only (no cards): 1 hand(s)"),
        "{t}"
    );
    assert!(t.contains("VPIP 0.0% (n=1)"), "{t}");
    assert!(t.contains("Profile cleared"), "{t}");
    assert!(
        t.contains("Profile of your play from public actions only (no cards): 0 hand(s)"),
        "{t}"
    );
    assert!(
        t.contains("Adaptation OFF. The fixed trained policy plays from now on."),
        "{t}"
    );
    assert!(t.contains("Adaptation ON."), "{t}");
}

#[test]
fn profile_commands_without_adapt_say_so() {
    let o = pokerbot(
        &["--seed", "5", "--hands", "2", "--bot", "baseline"],
        "f\nprofile\nadapt off\nq\n",
    );
    let t = flat(&stdout(&o));
    assert_eq!(
        t.matches("Adaptation is not active. Start pokerbot with --adapt")
            .count(),
        2,
        "{t}"
    );
}

#[test]
fn long_adaptive_session_is_legal_zero_sum_and_leak_free() {
    let o = pokerbot(
        &[
            "--seed", "77", "--hands", "60", "--bot", "baseline", "--auto", "random", "--adapt",
        ],
        "",
    );
    assert!(o.status.success(), "{}", stderr(&o));
    let out = stdout(&o);
    let (you, bot) = final_score(&out);
    assert_eq!(you + bot, 0);
    assert_no_leak(&out, 77);
}

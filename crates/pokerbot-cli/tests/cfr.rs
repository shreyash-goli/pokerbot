//! M-14: the `pokerbot` binary playing a trained CFR artifact, and explicit
//! handling of missing / wrong-depth / corrupt policies.

mod common;

use common::*;
use pokerbot_cfr::infer::BASELINE_LABEL;

fn path_str() -> &'static str {
    tiny_artifact_path().to_str().unwrap()
}

#[test]
fn cfr_full_session_with_banner_and_stats() {
    let o = pokerbot(
        &[
            "--bot",
            "cfr",
            "--strategy",
            path_str(),
            "--seed",
            "7",
            "--hands",
            "4",
            "--no-color",
            "--auto",
            "checkcall",
        ],
        "",
    );
    let out = stdout(&o);
    assert!(o.status.success(), "{}\n{out}", stderr(&o));
    let bytes = tiny_artifact_bytes();
    let checksum = u64::from_le_bytes(bytes[bytes.len() - 8..].try_into().unwrap());
    assert!(
        out.contains("Opponent: CFR policy -- cfr 20BB (400 it)"),
        "{out}"
    );
    assert!(out.contains(TINY_LABEL));
    assert!(out.contains(&format!("checksum {checksum:#018x}")), "{out}");
    assert!(out.contains("trained 400 iterations"));
    assert!(out.contains("trainer seed 31"));
    assert!(out.contains("not competitive, not GTO / Nash"));
    assert!(
        !out.contains(BASELINE_LABEL),
        "no baseline in a cfr session"
    );
    assert!(out.contains("Session complete."));
    assert!(out.contains("Final: 4 hand(s) played."));
    assert!(out.contains("Bot decisions in completed hands:"));
    assert_no_leak(&out, 7);
    let (you, bot) = final_score(&out);
    assert_eq!(you + bot, 0);
}

#[test]
fn cfr_stats_report_policy_and_fallback_counts() {
    // Tiny tables leave many unseen infosets, so both counters move.
    let o = pokerbot(
        &[
            "--bot",
            "cfr",
            "--strategy",
            path_str(),
            "--seed",
            "3",
            "--hands",
            "25",
            "--no-color",
            "--auto",
            "random",
        ],
        "",
    );
    let out = stdout(&o);
    assert!(o.status.success());
    let last = out
        .lines()
        .rfind(|l| l.contains("Bot decisions in completed hands:"))
        .unwrap();
    let nums: Vec<u64> = last
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|w| w.parse().ok())
        .collect();
    let (total, hits, falls) = (nums[0], nums[1], nums[2]);
    assert_eq!(hits + falls, total, "{last}");
    assert!(hits > 0 && falls > 0, "{last}");
}

#[test]
fn cfr_piped_stdin_with_stats_and_quit() {
    let o = pokerbot(
        &[
            "--bot",
            "cfr",
            "--strategy",
            path_str(),
            "--seed",
            "7",
            "--hands",
            "3",
            "--no-color",
        ],
        "x\nstats\nc\nc\nc\nc\n\nc\nquit\n",
    );
    let out = stdout(&o);
    assert!(o.status.success(), "{out}");
    assert!(out.contains("Invalid input"));
    assert!(
        out.contains("Bot decisions in completed hands: 0 total"),
        "{out}"
    );
    assert!(out.contains("You quit the session.") || out.contains("Input closed"));
    assert_no_leak(&out, 7);
}

#[test]
fn cfr_same_seed_same_transcript() {
    let run = |seed: &str| {
        stdout(&pokerbot(
            &[
                "--bot",
                "cfr",
                "--strategy",
                path_str(),
                "--seed",
                seed,
                "--hands",
                "6",
                "--no-color",
                "--auto",
                "random",
            ],
            "",
        ))
    };
    let a = run("11");
    assert_eq!(a, run("11"));
    assert_ne!(a, run("12"));
}

#[test]
fn cfr_never_leaks_over_many_seeds() {
    let mut showdowns = 0;
    for seed in 0..12u64 {
        let s = seed.to_string();
        let policy = if seed % 2 == 0 { "checkcall" } else { "random" };
        let o = pokerbot(
            &[
                "--bot",
                "cfr",
                "--strategy",
                path_str(),
                "--seed",
                &s,
                "--hands",
                "5",
                "--no-color",
                "--auto",
                policy,
            ],
            "",
        );
        let out = stdout(&o);
        assert!(o.status.success());
        showdowns += assert_no_leak(&out, seed);
        let (you, bot) = final_score(&out);
        assert_eq!(you + bot, 0);
    }
    assert!(showdowns > 0);
}

#[test]
fn missing_artifact_cfr_exits_2_auto_warns_loudly() {
    let missing = "/nonexistent/pokerbot-test/none.pokerbot";
    let o = pokerbot(
        &[
            "--bot",
            "cfr",
            "--strategy",
            missing,
            "--seed",
            "1",
            "--auto",
            "checkcall",
        ],
        "",
    );
    assert_eq!(o.status.code(), Some(2));
    let err = stderr(&o);
    assert!(err.contains("no trained policy for 20BB"), "{err}");
    assert!(err.contains("missing"), "{err}");
    assert!(err.contains(missing));
    assert!(o.stdout.is_empty(), "no substitute game");

    let o = pokerbot(
        &[
            "--strategy",
            missing,
            "--seed",
            "1",
            "--hands",
            "2",
            "--no-color",
            "--auto",
            "checkcall",
        ],
        "",
    );
    let out = stdout(&o);
    assert!(o.status.success(), "auto plays on");
    assert!(out.contains("!! No trained policy for 20BB"), "{out}");
    assert!(out.contains("missing"));
    assert!(out.contains("UNTRAINED baseline"));
    assert!(out.contains("scripts/train_smoke.sh"));
    assert!(out.contains(&format!("Opponent: {BASELINE_LABEL}")));
    assert!(!out.contains("CFR policy"));
}

#[test]
fn auto_uses_a_valid_artifact() {
    let o = pokerbot(
        &[
            "--bot",
            "auto",
            "--strategy",
            path_str(),
            "--seed",
            "1",
            "--hands",
            "1",
            "--no-color",
            "--auto",
            "checkcall",
        ],
        "",
    );
    let out = stdout(&o);
    assert!(o.status.success());
    assert!(out.contains("Opponent: CFR policy"), "{out}");
    assert!(!out.contains("UNTRAINED"));
}

#[test]
fn wrong_depth_artifact_is_rejected() {
    let o = pokerbot(
        &[
            "--bot",
            "cfr",
            "--depth",
            "50",
            "--strategy",
            path_str(),
            "--seed",
            "1",
            "--auto",
            "checkcall",
        ],
        "",
    );
    assert_eq!(o.status.code(), Some(2));
    let err = stderr(&o);
    assert!(err.contains("no trained policy for 50BB"), "{err}");
    assert!(err.contains("wrong depth"), "{err}");
    assert!(o.stdout.is_empty());

    let o = pokerbot(
        &[
            "--depth",
            "100",
            "--strategy",
            path_str(),
            "--seed",
            "1",
            "--hands",
            "1",
            "--no-color",
            "--auto",
            "checkcall",
        ],
        "",
    );
    let out = stdout(&o);
    assert!(o.status.success());
    assert!(out.contains("No trained policy for 100BB"), "{out}");
    assert!(out.contains("wrong depth"));
    assert!(out.contains("UNTRAINED baseline"));
}

#[test]
fn corrupt_artifact_is_rejected() {
    let mut bytes = tiny_artifact_bytes().to_vec();
    let mid = bytes.len() / 2;
    bytes[mid] ^= 0xA5;
    let p = write_tmp("corrupt.pokerbot", &bytes);
    let o = pokerbot(
        &[
            "--bot",
            "cfr",
            "--strategy",
            p.to_str().unwrap(),
            "--auto",
            "checkcall",
        ],
        "",
    );
    assert_eq!(o.status.code(), Some(2));
    let err = stderr(&o);
    assert!(err.contains("corrupt or invalid"), "{err}");
    assert!(err.contains("checksum"), "{err}");

    let p = write_tmp("garbage.pokerbot", b"definitely not a strategy");
    let o = pokerbot(
        &[
            "--bot",
            "cfr",
            "--strategy",
            p.to_str().unwrap(),
            "--auto",
            "checkcall",
        ],
        "",
    );
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("bad magic"), "{}", stderr(&o));
}

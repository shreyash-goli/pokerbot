//! M-22 session API tests: full hands, input validation, errors, JSON shape.

mod common;

use common::{V, parse, play_hand, policy_20bb, tiny_artifact};
use pokerbot_core::rng::PokerbotRng;
use pokerbot_wasm::GameSession;
use pokerbot_wasm::json::{Json, escape};

fn code(r: Result<GameSession, pokerbot_wasm::SessionError>) -> &'static str {
    match r {
        Ok(_) => panic!("expected an error"),
        Err(e) => e.code(),
    }
}

#[test]
fn full_seeded_hands_with_cfr_policy() {
    let (name, bytes) = policy_20bb();
    let mut s = GameSession::new(20, 7, Some(bytes), "cfr").unwrap();
    let info = parse(&s.strategy_info_json());
    assert_eq!(info.get("kind").s(), "cfr");
    assert_eq!(info.get("policy").get("depthBB").n(), 20);
    assert_eq!(info.get("trainedInBrowser"), &V::Bool(false));
    assert!(info.get("disclaimer").s().contains("NOT GTO"));
    let mut rng = PokerbotRng::seed_from_u64(1);
    let mut net = 0;
    let mut buttons = Vec::new();
    for h in 1..=30 {
        let first = parse(&s.snapshot_json());
        assert_eq!(first.get("handNumber").n(), h);
        buttons.push(first.get("button").s().to_string());
        let seen = play_hand(&mut s, &mut rng);
        let last = parse(seen.last().unwrap());
        net += last.get("payoff").get("human").n();
        let sess = last.get("session");
        assert_eq!(sess.get("handsCompleted").n(), h);
        assert_eq!(sess.get("score").get("human").n(), net);
        assert_eq!(sess.get("score").get("pokerbot").n(), -net);
        s.next_hand().unwrap();
    }
    assert_eq!(&buttons[..4], ["human", "pokerbot", "human", "pokerbot"]);
    let bot = parse(&s.snapshot_json()).get("bot").clone();
    let st = bot.get("statsAtLastHandEnd");
    assert!(st.get("decisions").n() > 0);
    assert_eq!(
        st.get("decisions").n(),
        st.get("policyHits").n() + st.get("fallbacks").n()
    );
    assert!(bot.get("artifactChecksum").s().starts_with("0x"));
    eprintln!(
        "cfr ({name}): 30 hands, human net {net}, bot decisions {} (policy {}, fallback {})",
        st.get("decisions").n(),
        st.get("policyHits").n(),
        st.get("fallbacks").n()
    );
}

#[test]
fn full_seeded_hands_with_baseline_all_depths() {
    for depth in [20u32, 50, 100] {
        let mut s = GameSession::new(depth, 3, None, "baseline").unwrap();
        let mut rng = PokerbotRng::seed_from_u64(depth.into());
        for _ in 0..25 {
            play_hand(&mut s, &mut rng);
            s.next_hand().unwrap();
        }
        let snap = parse(&s.snapshot_json());
        assert_eq!(snap.get("depthBB").n(), i64::from(depth));
        assert_eq!(snap.get("startingStack").n(), i64::from(depth) * 2);
        assert_eq!(snap.get("bot").get("kind").s(), "baseline");
        assert!(snap.get("bot").get("artifactChecksum").is_null());
        assert!(parse(&s.strategy_info_json()).get("policy").is_null());
    }
}

#[test]
fn same_seed_same_session() {
    let run = || {
        let mut s = GameSession::new(20, 99, Some(tiny_artifact().to_vec()), "cfr").unwrap();
        let mut rng = PokerbotRng::seed_from_u64(5);
        let mut all = Vec::new();
        for _ in 0..10 {
            all.extend(play_hand(&mut s, &mut rng));
            all.push(s.history_json());
            s.next_hand().unwrap();
        }
        all
    };
    assert_eq!(run(), run());
}

#[test]
fn reset_equals_fresh_session() {
    let bytes = tiny_artifact().to_vec();
    let mut a = GameSession::new(20, 1, Some(bytes.clone()), "cfr").unwrap();
    let mut rng = PokerbotRng::seed_from_u64(2);
    for _ in 0..3 {
        play_hand(&mut a, &mut rng);
        a.next_hand().unwrap();
    }
    a.reset(42).unwrap();
    let mut b = GameSession::new(20, 42, Some(bytes), "cfr").unwrap();
    let (mut r1, mut r2) = (PokerbotRng::seed_from_u64(9), PokerbotRng::seed_from_u64(9));
    for _ in 0..5 {
        assert_eq!(play_hand(&mut a, &mut r1), play_hand(&mut b, &mut r2));
        a.next_hand().unwrap();
        b.next_hand().unwrap();
    }
    assert_eq!(parse(&a.snapshot_json()).get("seed").s(), "42");
}

#[test]
fn invalid_actions_are_rejected_without_mutation() {
    let mut s = GameSession::new(20, 7, None, "baseline").unwrap();
    let mut rng = PokerbotRng::seed_from_u64(3);
    let garbage = [
        "",
        " ",
        "FOLD",
        "Fold",
        " fold",
        "fold ",
        "raise",
        "bet",
        "x",
        "c",
        "1",
        "allin",
        "half-pot",
        "half_pot\0",
        "{\"type\":\"fold\"}",
        "\u{2028}",
        "♠",
    ];
    let mut checked_illegal = 0;
    for _ in 0..40 {
        loop {
            let before = (s.snapshot_json(), s.history_json(), s.strategy_info_json());
            let snap = parse(&before.0);
            if !snap.get("terminal").is_null() {
                // Hand over: step must fail without changes.
                let e = s.step("fold").unwrap_err();
                assert_eq!(e.code(), "hand_over");
                assert_eq!(before.0, s.snapshot_json());
                break;
            }
            for g in garbage {
                let e = s.step(g).unwrap_err();
                assert_eq!(e.code(), "unknown_action", "{g:?}");
                assert!(e.to_string().starts_with("unknown_action: "));
            }
            let legal: Vec<&str> = snap
                .get("legalActions")
                .a()
                .iter()
                .map(|a| a.get("type").s())
                .collect();
            for name in ["fold", "check", "call", "half_pot", "pot", "all_in"] {
                if !legal.contains(&name) {
                    let e = s.step(name).unwrap_err();
                    assert_eq!(e.code(), "illegal_action", "{name}");
                    checked_illegal += 1;
                }
            }
            let e = s.next_hand().unwrap_err();
            assert_eq!(e.code(), "hand_in_progress");
            assert_eq!(
                before,
                (s.snapshot_json(), s.history_json(), s.strategy_info_json()),
                "rejected input changed the session"
            );
            let a = common::random_action(&snap, &mut rng);
            s.step(&a).unwrap();
        }
        s.next_hand().unwrap();
    }
    assert!(checked_illegal > 40, "{checked_illegal}");
}

#[test]
fn unknown_action_echo_is_truncated() {
    let mut s = GameSession::new(20, 7, None, "baseline").unwrap();
    let long = "x".repeat(10_000);
    let msg = s.step(&long).unwrap_err().to_string();
    assert!(msg.starts_with("unknown_action: "));
    assert!(msg.len() < 200, "{} bytes", msg.len());
    assert!(msg.contains(&format!("{}…", "x".repeat(64))));
    assert!(!msg.contains(&"x".repeat(65)));
    let wide = "♠".repeat(100); // multi-byte: cut on a char boundary
    assert!(
        s.step(&wide)
            .unwrap_err()
            .to_string()
            .contains(&format!("{}…", "♠".repeat(64)))
    );
    assert_eq!(pokerbot_wasm::session::echo_input("fold!"), "fold!");
}

#[test]
fn bad_construction_is_an_explicit_error() {
    assert_eq!(code(GameSession::new(20, 1, None, "cfr")), "policy_missing");
    for bad in [&b""[..], b"PKBTSTRT", b"definitely not an artifact"] {
        assert_eq!(
            code(GameSession::new(20, 1, Some(bad.to_vec()), "cfr")),
            "policy_invalid"
        );
    }
    let good = tiny_artifact();
    let truncated = good[..good.len() / 2].to_vec();
    assert_eq!(
        code(GameSession::new(20, 1, Some(truncated), "cfr")),
        "policy_invalid"
    );
    let mut flipped = good.to_vec();
    let mid = flipped.len() / 2;
    flipped[mid] ^= 0x40;
    assert_eq!(
        code(GameSession::new(20, 1, Some(flipped), "cfr")),
        "policy_invalid"
    );
    for depth in [50, 100] {
        assert_eq!(
            code(GameSession::new(depth, 1, Some(good.to_vec()), "cfr")),
            "policy_rules_mismatch"
        );
    }
    for depth in [0, 10, 21, 200] {
        assert_eq!(
            code(GameSession::new(depth, 1, None, "baseline")),
            "invalid_depth"
        );
    }
    for bot in ["", "CFR", "gto", "auto"] {
        assert_eq!(code(GameSession::new(20, 1, None, bot)), "invalid_bot");
    }
    // Explicit baseline ignores policy bytes and says it is the baseline.
    let s = GameSession::new(20, 1, Some(b"junk".to_vec()), "baseline").unwrap();
    assert_eq!(parse(&s.strategy_info_json()).get("kind").s(), "baseline");
}

#[test]
fn json_is_well_formed_and_has_contract_fields() {
    let mut s = GameSession::new(20, 11, Some(tiny_artifact().to_vec()), "cfr").unwrap();
    let mut rng = PokerbotRng::seed_from_u64(4);
    for _ in 0..30 {
        for snap in play_hand(&mut s, &mut rng) {
            let v = parse(&snap);
            for k in [
                "schema",
                "handNumber",
                "street",
                "board",
                "potChips",
                "actor",
                "myCards",
                "myStack",
                "botStack",
                "legalActions",
                "actionLog",
                "terminal",
                "payoff",
                "session",
                "bot",
            ] {
                assert!(v.has(k), "missing {k}");
            }
            assert_eq!(v.get("schema").s(), "pokerbot.public_state.v1");
            assert_eq!(v.get("myCards").a().len(), 2);
            for a in v.get("legalActions").a() {
                a.get("chipsToCommit").n();
                a.get("raiseTo").n();
            }
            for r in v.get("actionLog").a() {
                assert!(matches!(r.get("actor").s(), "human" | "pokerbot"));
            }
        }
        let h = parse(&s.history_json());
        let evs = h.get("events").a();
        assert_eq!(evs[0].get("type").s(), "hand_started");
        assert_eq!(evs.last().unwrap().get("type").s(), "hand_ended");
        s.next_hand().unwrap();
    }
    parse(&s.strategy_info_json());
}

#[test]
fn json_escaping_round_trips() {
    let nasty = "q\"b\\s/n\nr\rt\tz\u{0}\u{1}\u{1f}é♠\u{2028}\u{2029}😀";
    let v = Json::Obj(vec![
        ("s", Json::str(nasty)),
        (
            "a",
            Json::Arr(vec![Json::Int(i64::MIN + 1), Json::Int(0), Json::Null]),
        ),
    ]);
    let p = parse(&v.encode());
    assert_eq!(p.get("s").s(), nasty);
    assert_eq!(parse(&escape(nasty)), V::Str(nasty.into()));
}

/// Prints native artifact-load and per-decision times (informational; run
/// with `--release --nocapture` for meaningful numbers).
#[test]
fn native_inference_timing_report() {
    let (name, bytes) = policy_20bb();
    let t0 = std::time::Instant::now();
    let mut s = GameSession::new(20, 5, Some(bytes), "cfr").unwrap();
    let load = t0.elapsed();
    let mut rng = PokerbotRng::seed_from_u64(6);
    let t1 = std::time::Instant::now();
    let hands = 100;
    for _ in 0..hands {
        play_hand(&mut s, &mut rng);
        s.next_hand().unwrap();
    }
    let play = t1.elapsed();
    let d = parse(&s.snapshot_json())
        .get("bot")
        .get("statsAtLastHandEnd")
        .get("decisions")
        .n();
    assert!(d > 0);
    eprintln!(
        "timing ({name}): load {:.1} ms; {hands} hands, {d} bot decisions, {:.3} ms per \
         decision (includes engine + JSON snapshots)",
        load.as_secs_f64() * 1e3,
        play.as_secs_f64() * 1e3 / d as f64
    );
}

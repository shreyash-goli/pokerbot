//! M-21 difficulty modes. Every test is named `difficulty_*`, so
//! `cargo test -p mako-cli difficulty` selects them.

mod common;

use common::*;
use mako_cfr::abstraction::key::InfoKey;
use mako_cfr::abstraction::postflop::EquityAbstraction;
use mako_cfr::infer::{Agent, CfrAgent, DIFFICULTY_NOTE, DecisionSource, Difficulty};
use mako_core::action::{ActionKind, Seat};
use mako_core::observation::PlayerObservation;
use mako_core::rng::MakoRng;
use mako_core::rules::{RulesConfig, StackDepth};
use mako_core::state::{HandConfig, HandState};

fn rules() -> RulesConfig {
    RulesConfig::standard(StackDepth::Bb20)
}

fn agent(seed: u64, d: Difficulty) -> CfrAgent {
    CfrAgent::from_artifact_bytes(tiny_artifact_bytes(), &rules(), seed)
        .unwrap()
        .with_difficulty(d)
}

fn config(i: u64) -> HandConfig {
    HandConfig {
        rules: rules(),
        button: if i % 2 == 0 { Seat::Zero } else { Seat::One },
    }
}

fn legal_mask(obs: &PlayerObservation) -> u8 {
    obs.legal_actions
        .iter()
        .fold(0, |m, a| m | (1 << a.kind.index()))
}

/// Plays `hands` hands with one agent per seat, checking every decision;
/// returns the action log.
fn play(agents: &mut [CfrAgent; 2], hands: u64, deck_seed: u64) -> Vec<(Seat, ActionKind)> {
    let mut log = Vec::new();
    for i in 0..hands {
        let mut h = HandState::from_seed(config(i), deck_seed + i).unwrap();
        while let Some(seat) = h.to_act() {
            let obs = h.observation(seat);
            let a = &mut agents[seat.index()];
            let (fixed, _) = a.policy_strategy(&obs).unwrap();
            let (planned, _) = a.strategy(&obs).unwrap();
            let d = a.act(&obs).unwrap();
            assert_eq!(d.distribution, planned, "act() samples from strategy()");
            let mask = legal_mask(&obs);
            let sum: f64 = d.distribution.iter().sum();
            assert!((sum - 1.0).abs() < 1e-9, "sum {sum}");
            for k in ActionKind::ALL {
                let p = d.distribution[k.index()];
                assert!(p.is_finite() && p >= 0.0);
                if mask & (1 << k.index()) == 0 {
                    assert_eq!(p, 0.0, "mass on illegal {k:?}");
                }
            }
            assert!(obs.legal_actions.iter().any(|l| l.kind == d.action));
            assert!(d.distribution[d.action.index()] > 0.0);
            // The reported distribution is the documented transform of the
            // fixed policy mix (fallbacks are uniform and stay uniform).
            let expect = match d.source {
                DecisionSource::Policy => a.difficulty().transform(&fixed, mask),
                _ => fixed,
            };
            for i in 0..6 {
                assert!((d.distribution[i] - expect[i]).abs() < 1e-12);
            }
            h.apply(d.action).unwrap();
            log.push((seat, d.action));
        }
    }
    log
}

#[test]
fn difficulty_modes_are_legal_only_and_normalized() {
    for d in Difficulty::ALL {
        let mut agents = [agent(1, d), agent(2, d)];
        let log = play(&mut agents, 150, 10_000);
        let s = agents[0].stats();
        assert!(s.policy_hits > 0 && s.fallbacks > 0, "{d:?}: {s:?}");
        assert!(log.len() > 300);
    }
}

#[test]
fn difficulty_modes_are_deterministic_per_seed() {
    for d in Difficulty::ALL {
        let a = play(&mut [agent(5, d), agent(6, d)], 60, 200);
        let b = play(&mut [agent(5, d), agent(6, d)], 60, 200);
        assert_eq!(a, b, "{d:?}");
        let c = play(&mut [agent(500, d), agent(600, d)], 60, 200);
        assert_ne!(a, c, "{d:?}: seed changes sampling");
    }
}

/// The M-14 sampling algorithm, re-implemented here: integer weights
/// `q` of the stored entry (uniform for unseen keys), one `gen_range` draw.
fn reference_act(
    bytes: &[u8],
    abs: &EquityAbstraction,
    rng: &mut MakoRng,
    obs: &PlayerObservation,
) -> ActionKind {
    let policy = mako_cfr::artifact::Policy::load(bytes, &rules(), abs.config()).unwrap();
    let mask = legal_mask(obs);
    let key = InfoKey::from_observation(obs, abs).unwrap();
    let weights: [u64; 6] = match policy.lookup_bytes(&key.to_bytes()) {
        Some(e) => std::array::from_fn(|i| u64::from(e.q[i])),
        None => std::array::from_fn(|i| u64::from(mask & (1 << i) != 0)),
    };
    let mut pick = rng.gen_range(weights.iter().sum());
    for (i, &w) in weights.iter().enumerate() {
        if pick < w {
            return ActionKind::ALL[i];
        }
        pick -= w;
    }
    unreachable!()
}

#[test]
fn difficulty_challenging_is_bit_identical_to_the_m14_agent() {
    let bytes = tiny_artifact_bytes();
    let base = CfrAgent::from_artifact_bytes(bytes, &rules(), 77).unwrap();
    let abs = EquityAbstraction::new(base.metadata().abstraction).unwrap();
    let mut default_agent = CfrAgent::from_artifact_bytes(bytes, &rules(), 77).unwrap();
    let mut challenging = agent(77, Difficulty::Challenging);
    assert_eq!(
        default_agent.difficulty(),
        Difficulty::Challenging,
        "default"
    );
    assert_eq!(
        default_agent.label(),
        challenging.label(),
        "label unchanged"
    );
    let mut rng = MakoRng::seed_from_u64(77);
    let mut n = 0;
    for i in 0..80 {
        let mut h = HandState::from_seed(config(i), 3_000 + i).unwrap();
        while let Some(seat) = h.to_act() {
            let obs = h.observation(seat);
            let a = default_agent.act(&obs).unwrap();
            let b = challenging.act(&obs).unwrap();
            assert_eq!(a, b, "identical Decision, distribution bits included");
            assert_eq!(a.action, reference_act(bytes, &abs, &mut rng, &obs));
            assert_eq!(
                challenging.strategy(&obs).unwrap(),
                challenging.policy_strategy(&obs).unwrap()
            );
            h.apply(a.action).unwrap();
            n += 1;
        }
    }
    assert!(n > 200);
    assert_eq!(default_agent.stats(), challenging.stats());
}

fn l1_to_uniform(p: &[f64; 6], mask: u8) -> f64 {
    let n = f64::from(mask.count_ones());
    (0..6)
        .filter(|i| mask & (1 << i) != 0)
        .map(|i| (p[i] - 1.0 / n).abs())
        .sum()
}

fn entropy(p: &[f64; 6]) -> f64 {
    p.iter().filter(|&&x| x > 0.0).map(|&x| -x * x.ln()).sum()
}

/// Fixed scripted observations where the policy has a trained entry.
fn scripted_policy_observations() -> Vec<PlayerObservation> {
    let probe = agent(0, Difficulty::Challenging);
    let mut rng = MakoRng::seed_from_u64(2024);
    let mut out = Vec::new();
    for i in 0..120 {
        let mut h = HandState::from_seed(config(i), 50_000 + i).unwrap();
        while let Some(seat) = h.to_act() {
            let obs = h.observation(seat);
            if probe.strategy(&obs).unwrap().1 == DecisionSource::Policy
                && obs.legal_actions.len() > 1
            {
                out.push(obs);
            }
            let legal = h.legal_actions();
            h.apply_legal(&legal[rng.gen_range(legal.len() as u64) as usize])
                .unwrap();
        }
    }
    out
}

#[test]
fn difficulty_behaviour_is_differentiated_on_scripted_observations() {
    let obs = scripted_policy_observations();
    assert!(obs.len() > 100, "{}", obs.len());
    let agents: Vec<CfrAgent> = Difficulty::ALL.iter().map(|&d| agent(0, d)).collect();
    let mut mean_l1 = [0.0; 3];
    let mut mean_h = [0.0; 3];
    for o in &obs {
        let mask = legal_mask(o);
        let dists: Vec<[f64; 6]> = agents.iter().map(|a| a.strategy(o).unwrap().0).collect();
        let l1: Vec<f64> = dists.iter().map(|p| l1_to_uniform(p, mask)).collect();
        // Exact: distance to uniform scales by (1 - lambda).
        assert!(
            (l1[0] - 0.4 * l1[2]).abs() < 1e-9,
            "training = 0.4 x challenging"
        );
        assert!(
            (l1[1] - 0.75 * l1[2]).abs() < 1e-9,
            "balanced = 0.75 x challenging"
        );
        for k in 0..3 {
            mean_l1[k] += l1[k] / obs.len() as f64;
            mean_h[k] += entropy(&dists[k]) / obs.len() as f64;
        }
    }
    // Ordering of [training, balanced, challenging].
    assert!(
        mean_l1[0] < mean_l1[1] && mean_l1[1] < mean_l1[2],
        "{mean_l1:?}"
    );
    assert!(mean_h[0] > mean_h[1] && mean_h[1] > mean_h[2], "{mean_h:?}");
    eprintln!("mean L1 to uniform {mean_l1:?}, mean entropy {mean_h:?}");
}

#[test]
fn difficulty_empirical_frequencies_differ_significantly() {
    // The scripted spot where the trained mix is furthest from uniform.
    let obs = scripted_policy_observations();
    let probe = agent(0, Difficulty::Challenging);
    let spot = obs
        .iter()
        .max_by(|a, b| {
            let da = l1_to_uniform(&probe.strategy(a).unwrap().0, legal_mask(a));
            let db = l1_to_uniform(&probe.strategy(b).unwrap().0, legal_mask(b));
            da.total_cmp(&db)
        })
        .unwrap();
    let (fixed, _) = probe.strategy(spot).unwrap();
    let top = (0..6)
        .max_by(|&a, &b| fixed[a].total_cmp(&fixed[b]))
        .unwrap();
    const N: u64 = 20_000;
    let mut freq = [0.0; 3];
    for (k, d) in Difficulty::ALL.into_iter().enumerate() {
        let mut a = agent(31 + k as u64, d);
        let expected = a.strategy(spot).unwrap().0[top];
        let hits = (0..N)
            .filter(|_| a.act(spot).unwrap().action.index() == top)
            .count() as f64;
        freq[k] = hits / N as f64;
        // Each mode samples its own documented distribution (6 sigma).
        let sigma = (expected * (1.0 - expected) / N as f64).sqrt();
        assert!(
            (freq[k] - expected).abs() < 6.0 * sigma + 1e-9,
            "{d:?}: {} vs {expected}",
            freq[k]
        );
    }
    // Neighbouring modes differ by far more than sampling noise.
    for (x, y) in [(0, 1), (1, 2)] {
        let p = (freq[x] + freq[y]) / 2.0;
        let se = (2.0 * p * (1.0 - p) / N as f64).sqrt();
        assert!(freq[y] - freq[x] > 6.0 * se, "{freq:?} not separated");
    }
    eprintln!("top-action frequency [training, balanced, challenging] = {freq:?}");
}

fn run(difficulty: Option<&str>, extra: &[&str]) -> std::process::Output {
    let mut args = vec![
        "--bot",
        "cfr",
        "--strategy",
        tiny_artifact_path().to_str().unwrap(),
        "--seed",
        "9",
        "--hands",
        "6",
        "--width",
        "200",
    ];
    if let Some(d) = difficulty {
        args.extend(["--difficulty", d]);
    }
    args.extend_from_slice(extra);
    mako(&args, "")
}

/// The header's `Replay:` command with its shell continuations (` \\` +
/// newline + indent, see `ui::screens::shell_wrap`) joined back into one
/// line. Where the wrap falls depends on the artifact path's length (e.g. a
/// clean clone's longer temp dir), so assertions use the joined command.
fn replay_command(out: &str) -> String {
    let mut cmd = String::new();
    let mut lines = out.lines().skip_while(|l| !l.starts_with("Replay: "));
    for l in lines.by_ref() {
        let part = l.trim_start();
        match part.strip_suffix(" \\") {
            Some(p) => {
                cmd.push_str(p);
                cmd.push(' ');
            }
            None => {
                cmd.push_str(part);
                break;
            }
        }
    }
    assert!(cmd.starts_with("Replay: mako "), "no Replay line:\n{out}");
    cmd.push(' ');
    cmd
}

#[test]
fn difficulty_binary_runs_show_honest_labels() {
    let mut transcripts = Vec::new();
    for d in Difficulty::ALL {
        let o = run(Some(d.name()), &["--auto", "random"]);
        let out = stdout(&o);
        assert!(o.status.success(), "{}", stderr(&o));
        // Wrapped lines: compare with whitespace collapsed.
        let flat = out.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            flat.contains(&format!("Difficulty: {}", d.description())),
            "{out}"
        );
        assert!(flat.contains(DIFFICULTY_NOTE), "{out}");
        assert!(
            flat.contains(&format!("Difficulty {}.", d.description())),
            "stats line"
        );
        assert!(
            replay_command(&out).contains(&format!(" --difficulty {} ", d.name())),
            "replay hint: {}",
            replay_command(&out)
        );
        assert!(out.contains("Session complete."));
        if d != Difficulty::Challenging {
            assert!(
                flat.contains(&format!("| difficulty {}", d.description())),
                "label"
            );
        }
        assert!(!out.contains("stronger") || out.contains("not evaluated as stronger"));
        assert_eq!(
            out,
            stdout(&run(Some(d.name()), &["--auto", "random"])),
            "reproducible"
        );
        transcripts.push(out);
    }
    // Default is challenging.
    assert_eq!(stdout(&run(None, &["--auto", "random"])), transcripts[2]);
    assert_ne!(transcripts[0], transcripts[2]);
    assert_ne!(transcripts[1], transcripts[2]);
}

#[test]
fn difficulty_invalid_values_exit_2() {
    for bad in ["easy", "", "CHALLENGING!", "2"] {
        let o = mako(
            &[
                "--difficulty",
                bad,
                "--auto",
                "checkcall",
                "--bot",
                "baseline",
            ],
            "",
        );
        assert_eq!(o.status.code(), Some(2), "{bad:?}");
        assert!(stderr(&o).contains("--difficulty"), "{}", stderr(&o));
        assert!(o.stdout.is_empty());
    }
    let o = mako(
        &["--difficulty", "training", "--difficulty", "balanced"],
        "",
    );
    assert_eq!(o.status.code(), Some(2));
}

#[test]
fn difficulty_leaves_the_baseline_selectable_and_unaffected() {
    let base = [
        "--bot", "baseline", "--seed", "4", "--hands", "5", "--auto", "random", "--width", "200",
    ];
    let plain = stdout(&mako(&base, ""));
    let mut with = base.to_vec();
    with.extend(["--difficulty", "training"]);
    let o = mako(&with, "");
    assert!(o.status.success());
    let out = stdout(&o);
    assert!(
        out.contains("--difficulty training does not apply: the untrained baseline is unaffected.")
    );
    assert!(out.contains("baseline (untrained, random legal)"));
    let strip = |s: &str| {
        s.lines()
            .filter(|l| !l.contains("does not apply"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert_eq!(strip(&out), strip(&plain), "same game, same replay hint");
}

#[test]
fn difficulty_composes_with_adapt_and_coach_shows_fixed_policy() {
    // The human folds at once (the button faces the big blind), then
    // reviews the hand and asks for help.
    let script = "f\nanalysis\nhelp\nstats\nquit\n";
    let o = mako(
        &[
            "--bot",
            "cfr",
            "--strategy",
            tiny_artifact_path().to_str().unwrap(),
            "--seed",
            "9",
            "--hands",
            "3",
            "--width",
            "200",
            "--difficulty",
            "balanced",
            "--adapt",
        ],
        script,
    );
    let out = stdout(&o);
    assert!(o.status.success(), "{}", stderr(&o));
    assert!(
        out.contains("Order: trained policy -> difficulty blend (balanced) -> adaptive shift."),
        "{out}"
    );
    assert!(
        out.contains("of all decisions adapted by --adapt, not pure policy"),
        "{out}"
    );
    assert!(out.contains("fixed trained policy (not adapted, challenging); the opponent plays the balanced blend instead"), "{out}");
    assert!(
        out.contains("--adapt only"),
        "adaptive commands listed with --adapt"
    );
    assert!(
        replay_command(&out).contains(" --difficulty balanced --adapt"),
        "replay hint: {}",
        replay_command(&out)
    );

    let o = mako(
        &[
            "--bot",
            "cfr",
            "--strategy",
            tiny_artifact_path().to_str().unwrap(),
            "--seed",
            "9",
            "--hands",
            "3",
            "--width",
            "200",
        ],
        script,
    );
    let out = stdout(&o);
    assert!(o.status.success());
    assert!(
        out.contains("trained policy (fixed, unadapted, challenging)"),
        "{out}"
    );
    assert!(
        !out.contains("--adapt only"),
        "adaptive commands hidden without --adapt"
    );
    assert!(!out.contains("adapted by --adapt"));
}

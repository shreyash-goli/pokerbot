//! Text and JSON rendering of an [`EvalReport`]. Both are deterministic:
//! no timestamps or timings (the CLI prints wall-clock time separately).

use std::fmt::Write as _;

use crate::config::Mode;
use crate::evaluate::{COMPARISON_DISCLAIMER, Comparison, EvalReport};
use crate::gate::{ChecksumStatus, DISCLAIMER};
use crate::report::MatchupReport;
use crate::stats::Summary;

fn ci(s: &Summary) -> String {
    if s.n < 2 {
        return format!("{:+.2} (CI undefined, n={})", s.mean, s.n);
    }
    format!(
        "{:+8.2} ± {:6.2}  [{:+.2}, {:+.2}]",
        s.mean,
        s.half_width(),
        s.ci_low,
        s.ci_high
    )
}

fn pct(n: u64, d: u64) -> String {
    if d == 0 {
        "n/a".into()
    } else {
        format!("{:.2}%", n as f64 / d as f64 * 100.0)
    }
}

fn matchup_text(out: &mut String, m: &MatchupReport) {
    let _ = writeln!(out, "  {}  vs  {}", m.label_a, m.label_b);
    let _ = writeln!(
        out,
        "    hands {} ({} seat-swapped deal pairs), showdowns {}",
        m.hands, m.pairs, m.showdowns
    );
    let _ = writeln!(out, "    BB/100 (A, paired)   {}", ci(&m.bb100));
    if let Some((l, h)) = m.bootstrap_bb100 {
        let _ = writeln!(out, "    bootstrap 95% CI     [{l:+.2}, {h:+.2}]");
    }
    let _ = writeln!(out, "    A on button          {}", ci(&m.button_bb100));
    let _ = writeln!(out, "    A on big blind       {}", ci(&m.big_blind_bb100));
    let _ = writeln!(
        out,
        "    net chips A {:+} / B {:+}",
        m.a_net_chips, m.b_net_chips
    );
    for (side, c, inf) in [("A", m.counts_a, m.infer_a), ("B", m.counts_b, m.infer_b)] {
        if let Some(s) = inf {
            let _ = writeln!(
                out,
                "    {side} fallback rate      {} of {} decisions (unseen info sets; agent InferStats: {} hits / {} fallbacks)",
                pct(c.fallbacks, c.decisions),
                c.decisions,
                s.policy_hits,
                s.fallbacks
            );
        }
    }
    for (side, ad, n) in [
        ("A", m.adapt_a, m.delivered_a),
        ("B", m.adapt_b, m.delivered_b),
    ] {
        if let Some(a) = ad {
            let avg = if a.adapted > 0 {
                format!("{:.4}", a.mass_moved / a.adapted as f64)
            } else {
                "n/a".into()
            };
            let _ = writeln!(
                out,
                "    {side} adaptation        {} of {} decisions shifted ({}); avg mass moved {avg}; {n} hands observed",
                a.adapted,
                a.decisions,
                pct(a.adapted, a.decisions)
            );
        }
    }
}

fn comparison_text(out: &mut String, c: &Comparison) {
    let _ = writeln!(
        out,
        "  vs {:<10} fixed − adaptive BB/100 {}  ({} paired deals)",
        c.opponent_id,
        ci(&c.fixed_minus_adaptive_bb100),
        c.pairs
    );
    if let Some((l, h)) = c.bootstrap {
        let _ = writeln!(out, "               bootstrap 95% CI [{l:+.2}, {h:+.2}]");
    }
}

/// Human-readable report.
pub fn render_text(r: &EvalReport) -> String {
    let mut out = String::new();
    let a = &r.artifact;
    let _ = writeln!(out, "mako-eval: paired-seed, seat-swapped evaluation");
    let _ = writeln!(
        out,
        "artifact   {} ({} bytes, {} info sets, {} iterations, training seed {})",
        a.path, a.size_bytes, a.infosets, a.iterations, a.training_seed
    );
    let _ = writeln!(out, "label      {}", a.label);
    let _ = writeln!(out, "checksum   {:#018x}", a.checksum);
    let _ = writeln!(
        out,
        "game       {} BB, blinds {}/{}, stacks reset every hand",
        r.depth_bb, r.rules.small_blind, r.rules.big_blind
    );
    let _ = writeln!(
        out,
        "eval seed  {} ({} deal pairs per matchup = {} hands); 95% CI = mean ± 1.96·sd/√n over pairs",
        r.base_seed,
        r.pairs,
        2 * r.pairs
    );
    let _ = writeln!(out, "\ncandidate matchups (A = candidate):");
    for m in &r.matchups {
        matchup_text(&mut out, m);
    }
    if !r.sanity.is_empty() {
        let _ = writeln!(out, "\nsanity matchups (baseline vs baseline):");
        for m in &r.sanity {
            matchup_text(&mut out, m);
        }
    }
    let c = r.candidate_counts;
    let _ = writeln!(
        out,
        "\ncandidate fallback (unseen info set) rate: {} of {} decisions",
        pct(c.fallbacks, c.decisions),
        c.decisions
    );
    if !r.comparisons.is_empty() {
        let _ = writeln!(
            out,
            "\nfixed vs adaptive (same deals; positive = fixed did better):"
        );
        for c in &r.comparisons {
            comparison_text(&mut out, c);
        }
    }
    match &r.gate {
        Some(g) => {
            let _ = writeln!(out, "\npromotion gate: {}", g.verdict.as_str());
            for ch in &g.checks {
                let _ = writeln!(
                    out,
                    "  [{}] {}: {}",
                    if ch.passed { "pass" } else { "FAIL" },
                    ch.name,
                    ch.detail
                );
            }
        }
        None => {
            let _ = writeln!(
                out,
                "\ncomparison mode: this is a COMPARISON, not a promotion; no verdict is given."
            );
        }
    }
    if r.mode == Mode::Comparison {
        let _ = writeln!(out, "\n{COMPARISON_DISCLAIMER}");
    }
    let _ = writeln!(out, "\n{DISCLAIMER}");
    out
}

// ------------------------------------------------------------------ JSON

fn jstr(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for ch in s.chars() {
        match ch {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\t' => o.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(o, "\\u{:04x}", c as u32);
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn jnum(x: f64) -> String {
    if x.is_finite() {
        format!("{x:.6}")
    } else {
        "null".into()
    }
}

fn jsummary(s: &Summary) -> String {
    format!(
        "{{\"n\":{},\"mean\":{},\"sd\":{},\"se\":{},\"ci95_low\":{},\"ci95_high\":{}}}",
        s.n,
        jnum(s.mean),
        jnum(s.sd),
        jnum(s.se),
        jnum(s.ci_low),
        jnum(s.ci_high)
    )
}

fn jmatchup(m: &MatchupReport) -> String {
    let infer = |s: Option<crate::agent::InferStats>| match s {
        Some(s) => format!(
            "{{\"decisions\":{},\"policy_hits\":{},\"fallbacks\":{}}}",
            s.decisions, s.policy_hits, s.fallbacks
        ),
        None => "null".into(),
    };
    let adapt = |a: Option<mako_analysis::opponent::AdaptStats>| match a {
        Some(a) => format!(
            "{{\"decisions\":{},\"adapted\":{},\"mass_moved\":{}}}",
            a.decisions,
            a.adapted,
            jnum(a.mass_moved)
        ),
        None => "null".into(),
    };
    let boot = match m.bootstrap_bb100 {
        Some((l, h)) => format!("[{},{}]", jnum(l), jnum(h)),
        None => "null".into(),
    };
    format!(
        "{{\"candidate\":{},\"opponent\":{},\"label_a\":{},\"label_b\":{},\"pairs\":{},\"hands\":{},\"showdowns\":{},\
\"bb100\":{},\"bootstrap_ci95\":{},\"a_button_bb100\":{},\"a_big_blind_bb100\":{},\
\"a_net_chips\":{},\"b_net_chips\":{},\
\"a_decisions\":{},\"a_fallbacks\":{},\"b_decisions\":{},\"b_fallbacks\":{},\
\"a_infer_stats\":{},\"b_infer_stats\":{},\
\"a_adapt_stats\":{},\"b_adapt_stats\":{},\"a_hands_observed\":{},\"b_hands_observed\":{}}}",
        jstr(&m.candidate_id),
        jstr(&m.opponent_id),
        jstr(&m.label_a),
        jstr(&m.label_b),
        m.pairs,
        m.hands,
        m.showdowns,
        jsummary(&m.bb100),
        boot,
        jsummary(&m.button_bb100),
        jsummary(&m.big_blind_bb100),
        m.a_net_chips,
        m.b_net_chips,
        m.counts_a.decisions,
        m.counts_a.fallbacks,
        m.counts_b.decisions,
        m.counts_b.fallbacks,
        infer(m.infer_a),
        infer(m.infer_b),
        adapt(m.adapt_a),
        adapt(m.adapt_b),
        m.delivered_a,
        m.delivered_b
    )
}

/// Machine-readable report.
pub fn render_json(r: &EvalReport) -> String {
    let a = &r.artifact;
    let (status, pinned) = match r.checksum {
        ChecksumStatus::Matches(e) => ("matches", Some(e)),
        ChecksumStatus::Mismatch { expected, .. } => ("mismatch", Some(expected)),
        ChecksumStatus::NotPinned(_) => ("not_pinned", None),
    };
    let list = |v: &[MatchupReport]| v.iter().map(jmatchup).collect::<Vec<_>>().join(",");
    let gate = match &r.gate {
        Some(g) => {
            let checks = g
                .checks
                .iter()
                .map(|c| {
                    format!(
                        "{{\"name\":{},\"passed\":{},\"detail\":{}}}",
                        jstr(&c.name),
                        c.passed,
                        jstr(&c.detail)
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            format!(
                "{{\"verdict\":{},\"checks\":[{checks}]}}",
                jstr(g.verdict.as_str())
            )
        }
        None => "null".into(),
    };
    let comparisons = r
        .comparisons
        .iter()
        .map(|c| {
            format!(
                "{{\"opponent\":{},\"pairs\":{},\"fixed_minus_adaptive_bb100\":{},\"bootstrap_ci95\":{}}}",
                jstr(&c.opponent_id),
                c.pairs,
                jsummary(&c.fixed_minus_adaptive_bb100),
                c.bootstrap
                    .map_or("null".into(), |(l, h)| format!("[{},{}]", jnum(l), jnum(h)))
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let mode = match r.mode {
        Mode::Promotion => "promotion",
        Mode::Comparison => "comparison",
    };
    let comparison_note = match r.mode {
        Mode::Comparison => jstr(COMPARISON_DISCLAIMER),
        Mode::Promotion => "null".into(),
    };
    format!(
        "{{\"format\":\"mako-eval-report v1\",\
\"artifact\":{{\"path\":{},\"bytes\":{},\"checksum\":{},\"checksum_status\":{},\"pinned_checksum\":{},\
\"training_seed\":{},\"abstraction_seed\":{},\"iterations\":{},\"infosets\":{},\"label\":{},\"trainer_id\":{}}},\
\"rules\":{{\"depth_bb\":{},\"small_blind\":{},\"big_blind\":{},\"starting_stack\":{}}},\
\"base_seed\":{},\"pairs_per_matchup\":{},\"candidate\":{},\
\"matchups\":[{}],\"sanity\":[{}],\
\"candidate_decisions\":{},\"candidate_fallbacks\":{},\
\"candidate_infer_stats\":{{\"decisions\":{},\"policy_hits\":{},\"fallbacks\":{}}},\
\"mode\":{},\"gate\":{},\"comparisons\":[{}],\"comparison_note\":{},\
\"disclaimer\":{}}}\n",
        jstr(&a.path),
        a.size_bytes,
        jstr(&format!("{:#018x}", a.checksum)),
        jstr(status),
        pinned.map_or("null".into(), |p| jstr(&format!("{p:#018x}"))),
        a.training_seed,
        a.abstraction_seed,
        a.iterations,
        a.infosets,
        jstr(&a.label),
        jstr(&a.trainer_id),
        r.depth_bb,
        r.rules.small_blind,
        r.rules.big_blind,
        r.rules.starting_stack,
        r.base_seed,
        r.pairs,
        jstr(&r.candidate_label),
        list(&r.matchups),
        list(&r.sanity),
        r.candidate_counts.decisions,
        r.candidate_counts.fallbacks,
        r.candidate_infer.decisions,
        r.candidate_infer.policy_hits,
        r.candidate_infer.fallbacks,
        jstr(mode),
        gate,
        comparisons,
        comparison_note,
        jstr(DISCLAIMER)
    )
}

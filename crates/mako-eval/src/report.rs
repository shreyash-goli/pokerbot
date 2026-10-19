//! Per-matchup statistics derived from raw match results.

use mako_analysis::opponent::AdaptStats;

use crate::agent::InferStats;
use crate::matchup::{DecisionCounts, MatchResult};
use crate::stats::{Summary, Z95, bootstrap_ci, summarize};

/// Statistics for one matchup, from agent A's point of view.
#[derive(Clone, Debug, PartialEq)]
pub struct MatchupReport {
    /// Candidate kind (`cfr` or `adaptive_cfr`) playing as A.
    pub candidate_id: String,
    /// Short opponent id (`random`, `checkcall`, `tight`, `mirror`, ...).
    pub opponent_id: String,
    pub label_a: String,
    pub label_b: String,
    pub big_blind: u32,
    pub base_seed: u64,
    pub pairs: u64,
    pub hands: u64,
    pub showdowns: u64,
    /// A's BB/100 over all hands; CI from the paired per-deal results.
    pub bb100: Summary,
    /// Seeded percentile-bootstrap CI of `bb100.mean`.
    pub bootstrap_bb100: Option<(f64, f64)>,
    /// A's BB/100 when A held the button (moves first preflop).
    pub button_bb100: Summary,
    /// A's BB/100 when A held the big blind (moves first postflop).
    pub big_blind_bb100: Summary,
    /// Net chips over the whole match.
    pub a_net_chips: i64,
    pub b_net_chips: i64,
    pub counts_a: DecisionCounts,
    pub counts_b: DecisionCounts,
    pub infer_a: Option<InferStats>,
    pub infer_b: Option<InferStats>,
    pub adapt_a: Option<AdaptStats>,
    pub adapt_b: Option<AdaptStats>,
    /// Finished hands delivered to each side's opponent model.
    pub delivered_a: u64,
    pub delivered_b: u64,
}

/// Build the report. `bootstrap = (resamples, seed)`.
pub fn summarize_match(
    opponent_id: &str,
    m: &MatchResult,
    bootstrap: Option<(u32, u64)>,
) -> MatchupReport {
    let bb = m.rules.big_blind;
    let per_pair: Vec<f64> = m.pairs.iter().map(|p| p.a_total() as f64).collect();
    let btn: Vec<f64> = m.pairs.iter().map(|p| p.a_button as f64).collect();
    let bbp: Vec<f64> = m.pairs.iter().map(|p| p.a_big_blind as f64).collect();
    // A pair is two hands: chips/hand = pair total / 2.
    let pair_scale = 100.0 / f64::from(bb) / 2.0;
    let hand_scale = 100.0 / f64::from(bb);
    let a_net: i64 = m.pairs.iter().map(|p| p.a_total()).sum();
    MatchupReport {
        candidate_id: "cfr".into(),
        opponent_id: opponent_id.to_string(),
        label_a: m.label_a.clone(),
        label_b: m.label_b.clone(),
        big_blind: bb,
        base_seed: m.base_seed,
        pairs: m.pairs.len() as u64,
        hands: m.hands(),
        showdowns: m.showdowns,
        bb100: summarize(&per_pair, Z95).scaled(pair_scale),
        bootstrap_bb100: bootstrap.and_then(|(b, seed)| {
            bootstrap_ci(&per_pair, b, seed).map(|(l, h)| (l * pair_scale, h * pair_scale))
        }),
        button_bb100: summarize(&btn, Z95).scaled(hand_scale),
        big_blind_bb100: summarize(&bbp, Z95).scaled(hand_scale),
        a_net_chips: a_net,
        b_net_chips: -a_net,
        counts_a: m.counts_a,
        counts_b: m.counts_b,
        infer_a: m.infer_a,
        infer_b: m.infer_b,
        adapt_a: m.adapt_a,
        adapt_b: m.adapt_b,
        delivered_a: m.delivered_a,
        delivered_b: m.delivered_b,
    }
}

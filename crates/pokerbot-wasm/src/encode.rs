//! JSON encodings of what the human player may see.
//!
//! [`public_state`] takes a [`PlayerObservation`] (never a `HandState`), so
//! the only private cards it can serialize are the observer's own, plus
//! the opponent's cards that the engine itself puts in
//! `opponent_hole_cards` at a showdown terminal. As a second guard, those
//! are emitted only when `terminal == Showdown`.
//!
//! [`events`] takes an event list already filtered with
//! `events_for(Viewer::Seat(me))` and additionally drops any `HoleCards`
//! event for another seat.

use pokerbot_cfr::infer::{AgentMetadata, InferStats};
use pokerbot_core::action::{ActionRecord, Seat, Street, TerminalReason};
use pokerbot_core::card::Card;
use pokerbot_core::eval::evaluate;
use pokerbot_core::events::HandEvent;
use pokerbot_core::observation::PlayerObservation;
use pokerbot_core::rules::Chips;

use crate::json::Json;
use crate::session::{BotKind, action_name};

/// Wire schema id of [`public_state`].
pub const PUBLIC_STATE_SCHEMA: &str = "pokerbot.public_state.v1";
/// Wire schema id of [`events`].
pub const HISTORY_SCHEMA: &str = "pokerbot.history.v1";
/// Wire schema id of [`strategy_info`].
pub const STRATEGY_INFO_SCHEMA: &str = "pokerbot.strategy_info.v1";

/// Shown with every CFR policy.
pub const CFR_DISCLAIMER: &str = "Pokerbot plays the average strategy of a from-scratch \
external-sampling MCCFR run over a coarse abstraction (six bet sizes, bucketed cards). \
It approximates an equilibrium of that abstract game only: it is NOT GTO or Nash, and a \
smoke-trained policy is weak. Situations missing from the policy are played uniformly \
at random over the legal actions (counted as fallbacks).";
/// Shown with the baseline.
pub const BASELINE_DISCLAIMER: &str = "The untrained baseline picks a weighted-random \
legal action. It is NOT a trained CFR policy and has no strategy.";
/// Browser-security caveat shown with every session.
pub const CLIENT_SIDE_NOTE: &str = "Entertainment only, no real money. The game runs \
entirely in your browser: snapshots never include the bot's unrevealed cards, but \
client-side code cannot hide state from a determined user.";

/// Session-level facts for a snapshot.
#[derive(Clone, Copy, Debug)]
pub struct SessionInfo {
    pub hand_number: u64,
    pub seed: u64,
    pub depth_bb: u32,
    pub starting_stack: Chips,
    pub hands_completed: u64,
    pub score: [i64; 2],
    pub poisoned: bool,
}

/// What the client may know about the bot.
#[derive(Clone, Debug)]
pub struct BotInfo<'a> {
    pub kind: BotKind,
    pub label: String,
    pub metadata: Option<&'a AgentMetadata>,
    /// Counters as of the last completed hand (never mid-hand).
    pub stats_at_hand_end: InferStats,
}

fn who(seat: Seat, me: Seat) -> &'static str {
    if seat == me { "human" } else { "pokerbot" }
}

fn street_name(s: Street) -> &'static str {
    match s {
        Street::Preflop => "preflop",
        Street::Flop => "flop",
        Street::Turn => "turn",
        Street::River => "river",
        Street::Showdown => "showdown",
    }
}

fn card(c: Card) -> Json {
    Json::Str(c.to_string())
}

fn cards(cs: &[Card]) -> Json {
    Json::Arr(cs.iter().map(|&c| card(c)).collect())
}

fn hex64(n: u64) -> Json {
    Json::Str(format!("{n:#018x}"))
}

/// u64 as a decimal string (may exceed 2^53).
fn dec64(n: u64) -> Json {
    Json::Str(n.to_string())
}

fn u64_num(n: u64) -> Json {
    // Counters stay far below 2^53; saturate rather than wrap if not.
    Json::Int(i64::try_from(n.min((1 << 53) - 1)).unwrap_or(0))
}

/// `{human, pokerbot}` from a seat-indexed pair.
fn per_seat<T: Copy>(v: [T; 2], me: Seat, f: impl Fn(T) -> Json) -> Json {
    Json::Obj(vec![
        ("human", f(v[me.index()])),
        ("pokerbot", f(v[me.other().index()])),
    ])
}

fn action_record(r: &ActionRecord, me: Seat) -> Json {
    Json::Obj(vec![
        ("actor", Json::str(who(r.seat, me))),
        ("street", Json::str(street_name(r.street))),
        ("type", Json::str(action_name(r.kind))),
        ("raiseTo", Json::int(r.raise_to)),
        ("chipsCommitted", Json::int(r.chips_committed)),
    ])
}

fn terminal(t: Option<TerminalReason>, me: Seat) -> Json {
    match t {
        None => Json::Null,
        Some(TerminalReason::Fold { winner }) => Json::Obj(vec![
            ("reason", Json::str("fold")),
            ("winner", Json::str(who(winner, me))),
        ]),
        Some(TerminalReason::Showdown) => Json::Obj(vec![("reason", Json::str("showdown"))]),
    }
}

fn hand_description(hole: [Card; 2], board: &[Card]) -> Json {
    let mut all = hole.to_vec();
    all.extend_from_slice(board);
    if all.len() < 5 {
        return Json::Null;
    }
    Json::Str(evaluate(&all).to_string())
}

/// The `PublicState` object for `obs.seat` (the human).
pub fn public_state(obs: &PlayerObservation, s: &SessionInfo, bot: &BotInfo<'_>) -> Json {
    let me = obs.seat;
    let opp = me.other();
    let showdown = obs.terminal == Some(TerminalReason::Showdown);
    let revealed = obs.opponent_hole_cards.filter(|_| showdown);

    let mut f: Vec<(&'static str, Json)> = vec![
        ("schema", Json::str(PUBLIC_STATE_SCHEMA)),
        ("handNumber", u64_num(s.hand_number)),
        ("seed", dec64(s.seed)),
        ("depthBB", Json::int(s.depth_bb)),
        ("startingStack", Json::int(s.starting_stack)),
        ("smallBlind", Json::int(obs.small_blind)),
        ("bigBlind", Json::int(obs.big_blind)),
        ("button", Json::str(who(obs.button, me))),
        ("street", Json::str(street_name(obs.street))),
        ("board", cards(&obs.board)),
        ("potChips", Json::int(obs.pot)),
        ("actor", Json::opt(obs.to_act, |a| Json::str(who(a, me)))),
        ("myCards", cards(&obs.hole_cards)),
        ("myStack", Json::int(obs.stacks[me.index()])),
        ("botStack", Json::int(obs.stacks[opp.index()])),
        (
            "myStreetChips",
            Json::int(obs.street_contributions[me.index()]),
        ),
        (
            "botStreetChips",
            Json::int(obs.street_contributions[opp.index()]),
        ),
        (
            "legalActions",
            Json::Arr(
                obs.legal_actions
                    .iter()
                    .map(|a| {
                        Json::Obj(vec![
                            ("type", Json::str(action_name(a.kind))),
                            ("chipsToCommit", Json::int(a.chips_to_commit)),
                            ("raiseTo", Json::int(a.raise_to)),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "actionLog",
            Json::Arr(obs.history.iter().map(|r| action_record(r, me)).collect()),
        ),
        ("terminal", terminal(obs.terminal, me)),
        (
            "payoff",
            Json::opt(obs.payoffs, |p| per_seat(p, me, Json::Int)),
        ),
    ];
    if let Some(bc) = revealed {
        f.push(("botCards", cards(&bc)));
        f.push((
            "showdown",
            Json::Obj(vec![
                ("myHand", hand_description(obs.hole_cards, &obs.board)),
                ("botHand", hand_description(bc, &obs.board)),
            ]),
        ));
    }
    f.push((
        "session",
        Json::Obj(vec![
            ("handsCompleted", u64_num(s.hands_completed)),
            ("score", per_seat(s.score, me, Json::Int)),
            ("poisoned", Json::Bool(s.poisoned)),
        ]),
    ));
    f.push(("bot", bot_summary(bot)));
    f.push(("note", Json::str(CLIENT_SIDE_NOTE)));
    Json::Obj(f)
}

fn disclaimer(kind: BotKind) -> &'static str {
    match kind {
        BotKind::Cfr => CFR_DISCLAIMER,
        BotKind::Baseline => BASELINE_DISCLAIMER,
    }
}

fn stats(st: InferStats) -> Json {
    Json::Obj(vec![
        ("decisions", u64_num(st.decisions)),
        ("policyHits", u64_num(st.policy_hits)),
        ("fallbacks", u64_num(st.fallbacks)),
    ])
}

fn bot_summary(bot: &BotInfo<'_>) -> Json {
    Json::Obj(vec![
        ("kind", Json::str(bot.kind.name())),
        ("label", Json::str(bot.label.clone())),
        (
            "artifactChecksum",
            Json::opt(bot.metadata, |m| hex64(m.checksum)),
        ),
        ("notGto", Json::Bool(true)),
        ("disclaimer", Json::str(disclaimer(bot.kind))),
        ("statsAtLastHandEnd", stats(bot.stats_at_hand_end)),
    ])
}

/// Policy metadata only: never strategy contents.
pub fn strategy_info(bot: &BotInfo<'_>) -> Json {
    let policy = Json::opt(bot.metadata, |m| {
        let a = &m.abstraction;
        Json::Obj(vec![
            ("checksum", hex64(m.checksum)),
            ("label", Json::str(m.label.clone())),
            ("trainerId", Json::str(m.trainer_id.clone())),
            ("trainerSeed", dec64(m.seed)),
            ("iterations", u64_num(m.iterations)),
            ("infosets", u64_num(m.infosets as u64)),
            ("depthBB", Json::int(m.depth_bb())),
            ("smallBlind", Json::int(m.rules.small_blind)),
            ("bigBlind", Json::int(m.rules.big_blind)),
            ("startingStack", Json::int(m.rules.starting_stack)),
            ("abstractionFingerprint", hex64(m.fingerprint)),
            (
                "abstraction",
                Json::Obj(vec![
                    ("version", Json::int(a.version)),
                    ("flopBuckets", Json::int(a.flop_buckets)),
                    ("turnBuckets", Json::int(a.turn_buckets)),
                    ("riverBuckets", Json::int(a.river_buckets)),
                    ("flopSamples", Json::int(a.flop_samples)),
                    ("turnSamples", Json::int(a.turn_samples)),
                ]),
            ),
        ])
    });
    Json::Obj(vec![
        ("schema", Json::str(STRATEGY_INFO_SCHEMA)),
        ("kind", Json::str(bot.kind.name())),
        ("label", Json::str(bot.label.clone())),
        ("notGto", Json::Bool(true)),
        ("disclaimer", Json::str(disclaimer(bot.kind))),
        ("trainedInBrowser", Json::Bool(false)),
        ("policy", policy),
        ("statsAtLastHandEnd", stats(bot.stats_at_hand_end)),
    ])
}

fn event(e: &HandEvent, me: Seat) -> Option<Json> {
    Some(match e {
        HandEvent::HandStarted {
            rules,
            button,
            starting_stacks,
            blinds_posted,
        } => Json::Obj(vec![
            ("type", Json::str("hand_started")),
            ("button", Json::str(who(*button, me))),
            ("smallBlind", Json::int(rules.small_blind)),
            ("bigBlind", Json::int(rules.big_blind)),
            ("startingStacks", per_seat(*starting_stacks, me, Json::int)),
            ("blindsPosted", per_seat(*blinds_posted, me, Json::int)),
        ]),
        HandEvent::HoleCards { seat, cards: cs } => {
            if *seat != me {
                return None; // never another seat's private cards
            }
            Json::Obj(vec![
                ("type", Json::str("hole_cards")),
                ("seat", Json::str(who(*seat, me))),
                ("cards", cards(cs)),
            ])
        }
        HandEvent::Action(r) => Json::Obj(vec![
            ("type", Json::str("action")),
            ("actor", Json::str(who(r.seat, me))),
            ("street", Json::str(street_name(r.street))),
            ("action", Json::str(action_name(r.kind))),
            ("raiseTo", Json::int(r.raise_to)),
            ("chipsCommitted", Json::int(r.chips_committed)),
        ]),
        HandEvent::BoardDealt { street, cards: cs } => Json::Obj(vec![
            ("type", Json::str("board_dealt")),
            ("street", Json::str(street_name(*street))),
            ("cards", cards(cs)),
        ]),
        HandEvent::Showdown { hands } => Json::Obj(vec![
            ("type", Json::str("showdown")),
            (
                "hands",
                Json::Arr(
                    hands
                        .iter()
                        .map(|h| {
                            Json::Obj(vec![
                                ("seat", Json::str(who(h.seat, me))),
                                ("holeCards", cards(&h.hole_cards)),
                                ("bestFive", cards(&h.best_five)),
                                ("hand", Json::Str(h.rank.to_string())),
                            ])
                        })
                        .collect(),
                ),
            ),
        ]),
        HandEvent::HandEnded {
            reason,
            payoffs,
            final_stacks,
        } => Json::Obj(vec![
            ("type", Json::str("hand_ended")),
            ("terminal", terminal(Some(*reason), me)),
            ("payoff", per_seat(*payoffs, me, Json::Int)),
            ("finalStacks", per_seat(*final_stacks, me, Json::int)),
        ]),
    })
}

/// `{schema, events}` for a viewer-filtered event list.
pub fn events(evs: &[HandEvent], me: Seat) -> Json {
    Json::Obj(vec![
        ("schema", Json::str(HISTORY_SCHEMA)),
        (
            "events",
            Json::Arr(evs.iter().filter_map(|e| event(e, me)).collect()),
        ),
    ])
}

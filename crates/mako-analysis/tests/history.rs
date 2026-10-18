//! M-17 hand records: information boundary, text round trip, strict parser.

mod common;

use common::*;
use mako_analysis::history::{self, HISTORY_HEADER_V1};
use mako_analysis::{HandRecord, HistoryError, OpponentLabel, SessionStats};
use mako_core::action::{Seat, TerminalReason};
use mako_core::card::Card;
use mako_core::deck::Deck;
use mako_core::events::Viewer;
use mako_core::rng::MakoRng;
use mako_core::state::HandState;

const DRY: &str = "Kc Qd 9h 4s 3c";

fn rec(n: u64, st: &HandState) -> HandRecord {
    HandRecord::from_state(n, st, HUMAN, cfr_label()).unwrap()
}

fn sample_records() -> Vec<HandRecord> {
    let mut rng = MakoRng::seed_from_u64(99);
    let mut out = Vec::new();
    for n in 1..=60u64 {
        let button = Seat::ALL[(n % 2) as usize];
        let st = HandState::from_seed(config(button), rng.next_u64()).unwrap();
        let st = play_random(st, &mut rng);
        let label = if n % 3 == 0 {
            OpponentLabel::new("baseline", None)
        } else {
            cfr_label()
        };
        out.push(HandRecord::from_state(n, &st, HUMAN, label).unwrap());
    }
    out
}

#[test]
fn record_contains_only_human_knowledge_after_fold() {
    let st = hand(Seat::One, "Ah Ad", "7c 2d", DRY, &[Pot, Fold]);
    let r = rec(1, &st);
    assert_eq!(
        r.human_cards,
        [st.hole_cards(HUMAN)[0], st.hole_cards(HUMAN)[1]]
    );
    assert_eq!(r.opponent_cards, None);
    assert!(r.board.is_empty(), "no undealt board cards stored");
    let text = history::to_text(std::slice::from_ref(&r));
    for c in st.hole_cards(BOT) {
        assert!(
            !text.contains(&c.to_string()),
            "bot card {c} leaked:\n{text}"
        );
    }
    assert!(text.contains("shown -\n"));
    assert!(text.contains("board -\n"));
}

#[test]
fn showdown_record_contains_shown_cards() {
    let mut a = vec![Call, Check];
    a.extend_from_slice(&[Check; 6]);
    let st = hand(Seat::Zero, "Ah Ad", "7c 2d", DRY, &a);
    let r = rec(1, &st);
    assert_eq!(r.opponent_cards, Some(st.hole_cards(BOT)));
    assert_eq!(r.board.len(), 5);
    assert_eq!(r.reason, TerminalReason::Showdown);
}

#[test]
fn omniscient_or_wrong_seat_stream_is_refused() {
    let st = hand(Seat::One, "Ah Ad", "7c 2d", DRY, &[Pot, Fold]);
    for v in [Viewer::Omniscient, Viewer::Seat(BOT)] {
        let e = HandRecord::from_events(1, HUMAN, &st.events_for(v), cfr_label());
        assert_eq!(e, Err(HistoryError::HiddenInformation), "{v:?}");
    }
    let live = state_from_order(Seat::Zero, &order(Seat::Zero, "Ah Ad", "7c 2d", DRY));
    assert_eq!(
        HandRecord::from_state(1, &live, HUMAN, cfr_label()),
        Err(HistoryError::NotTerminal)
    );
}

/// Swapping the bot's hidden cards (with cards that never appear) must not
/// change the record, its text or the stats for any hand ending in a fold.
#[test]
fn hidden_card_swap_leaves_record_and_stats_identical() {
    let mut rng = MakoRng::seed_from_u64(0x5EED);
    let mut compared = 0;
    for h in 0..400u64 {
        let button = Seat::ALL[(h % 2) as usize];
        let base_order = Deck::shuffled(&mut rng).order().to_vec();
        let bot_pos = if BOT == button { [1, 3] } else { [0, 2] };
        let mut swapped = base_order.clone();
        swapped.swap(bot_pos[0], 20);
        swapped.swap(bot_pos[1], 21);
        let base = state_from_order(button, &base_order);
        let choices = random_choices(&base, &mut rng);
        let a = play_choices(base, &choices);
        let b = play_choices(state_from_order(button, &swapped), &choices);
        assert_ne!(a.hole_cards(BOT), b.hole_cards(BOT));
        if a.terminal_reason() == Some(TerminalReason::Showdown) {
            continue; // cards are legitimately revealed
        }
        let (ra, rb) = (rec(h + 1, &a), rec(h + 1, &b));
        assert_eq!(ra, rb);
        let (ta, tb) = (
            history::to_text(std::slice::from_ref(&ra)),
            history::to_text(std::slice::from_ref(&rb)),
        );
        assert_eq!(ta.as_bytes(), tb.as_bytes());
        for c in a.hole_cards(BOT).into_iter().chain(b.hole_cards(BOT)) {
            assert!(!ta.contains(&format!(" {c}")), "hidden {c} in record");
        }
        assert_eq!(
            SessionStats::from_records(&[ra]),
            SessionStats::from_records(&[rb])
        );
        compared += 1;
    }
    assert!(compared > 150, "only {compared} fold hands compared");
}

#[test]
fn text_round_trip() {
    let recs = sample_records();
    let text = history::to_text(&recs);
    assert!(text.starts_with("mako-history v1\nhand 1\n"));
    let back = history::parse(&text).unwrap();
    assert_eq!(back, recs);
    assert_eq!(history::to_text(&back), text, "serialisation is stable");
    assert_eq!(
        history::parse(&format!("{HISTORY_HEADER_V1}\n")).unwrap(),
        vec![]
    );
}

#[test]
fn exact_block_format() {
    let st = hand(Seat::One, "Ah Ad", "7c 2d", DRY, &[Pot, Fold]);
    let text = history::to_text(&[rec(1, &st)]);
    assert_eq!(
        text,
        "mako-history v1\nhand 1\nrules 1 2 200\nbutton 1\nhuman 0\nstacks 200 200\n\
         opponent 0123456789abcdef CFR policy smoke-20bb\nhole Ah Ad\nshown -\nboard -\n\
         action 1 preflop pot 6 5\naction 0 preflop fold 2 0\nresult fold 1\n\
         payoffs -2 2\nfinal 198 202\nend\n"
    );
}

#[test]
fn label_is_sanitised() {
    let l = OpponentLabel::new("  evil\nlabel\r\x07 ", None);
    assert_eq!(l.name, "evil label");
    assert_eq!(OpponentLabel::new("\n", None).name, "unknown");
    let long = "x".repeat(500);
    assert_eq!(OpponentLabel::new(&long, None).name.len(), 200);
}

fn assert_parse_err(text: &str) {
    assert!(history::parse(text).is_err(), "accepted:\n{text}");
}

#[test]
fn strict_parser_rejects_corruption() {
    let recs = sample_records();
    let good = history::to_text(&recs[..3]);
    assert!(history::parse(&good).is_ok());
    // Version and header.
    assert_eq!(
        history::parse(&good.replacen("v1", "v2", 1)),
        Err(HistoryError::UnsupportedVersion("v2".into()))
    );
    assert_parse_err(&good.replacen("mako-history", "mako-hist", 1));
    assert_parse_err("");
    // Truncation at every line boundary and mid-line.
    let lines: Vec<&str> = good.lines().collect();
    for cut in 1..lines.len() {
        let prefix = lines[..cut].join("\n") + "\n";
        let complete = lines[cut - 1] == "end" || cut == 1;
        assert_eq!(history::parse(&prefix).is_ok(), complete, "cut {cut}");
    }
    assert_parse_err(&good[..good.len() - 1]);
    // Field-level corruption.
    for (from, to) in [
        ("payoffs ", "payoffs 1"),
        ("final ", "final 9"),
        ("hand 2", "hand 1"),
        ("hand 1", "hand 01"),
        ("rules 1 2", "rules 1 3"),
        ("button ", "button 2"),
        ("human 0", "human x"),
        ("hole ", "hole Zz "),
        ("action ", "action  "),
        ("end\n", "end \n"),
        ("result", "reslut"),
        ("opponent 0123", "opponent 0X23"),
    ] {
        let bad = good.replacen(from, to, 1);
        assert_ne!(bad, good, "{from}");
        assert_parse_err(&bad);
    }
    assert_parse_err(&format!("{good}garbage\n"));
    assert_parse_err(&format!("{good}\n"));
    assert_parse_err(&good.replace('\n', "\r\n"));
}

#[test]
fn parser_rejects_semantically_invalid_records() {
    let st = hand(Seat::One, "Ah Ad", "7c 2d", DRY, &[Pot, Fold]);
    let good = history::to_text(&[rec(1, &st)]);
    for (from, to) in [
        ("payoffs -2 2", "payoffs -3 3"), // wrong result
        ("action 1 preflop pot 6 5", "action 1 preflop pot 8 7"), // illegal size
        ("shown -", "shown 7c 2d"),       // shown cards after a fold
        ("board -", "board 2c 3c 4c"),    // board that was never dealt
        ("hole Ah Ad", "hole Ah Ah"),     // duplicate card
        ("result fold 1", "result showdown"),
    ] {
        let bad = good.replacen(from, to, 1);
        assert_ne!(bad, good);
        assert!(
            matches!(history::parse(&bad), Err(HistoryError::Invalid { .. })),
            "{from} -> {to}"
        );
    }
}

#[test]
fn random_mutation_fuzz_never_panics() {
    let good = history::to_text(&sample_records()[..8]);
    let bytes = good.as_bytes();
    let alphabet = b" \n-0123456789abcdefhkqjtsAKQJT:x\x00\xff";
    let mut rng = MakoRng::seed_from_u64(0xF022);
    let mut rejected = 0;
    for _ in 0..3_000 {
        let mut m = bytes.to_vec();
        for _ in 0..=rng.gen_range(3) {
            let i = rng.gen_range(m.len() as u64) as usize;
            match rng.gen_range(4) {
                0 => m[i] = alphabet[rng.gen_range(alphabet.len() as u64) as usize],
                1 => {
                    m.remove(i);
                }
                2 => m.insert(i, alphabet[rng.gen_range(alphabet.len() as u64) as usize]),
                _ => m.truncate(i),
            }
            if m.is_empty() {
                break;
            }
        }
        let text = String::from_utf8_lossy(&m);
        if let Ok(recs) = history::parse(&text) {
            // Anything accepted must be self-consistent and round-trip.
            for r in &recs {
                r.validate().unwrap();
            }
            assert_eq!(history::parse(&history::to_text(&recs)).unwrap(), recs);
        } else {
            rejected += 1;
        }
    }
    assert!(rejected > 2_500, "only {rejected} mutations rejected");
}

#[test]
fn board_cards_only_as_revealed() {
    // Fold on the flop: exactly 3 board cards recorded.
    let st = hand(
        Seat::Zero,
        "Ah Ad",
        "7c 2d",
        DRY,
        &[Call, Check, HalfPot, Fold],
    );
    let r = rec(1, &st);
    assert_eq!(r.board.len(), 3);
    let turn: Card = "4s".parse().unwrap();
    assert!(!history::to_text(&[r]).contains(&turn.to_string()));
}

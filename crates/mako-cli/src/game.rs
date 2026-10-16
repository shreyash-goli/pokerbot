//! The session loop: N hands of human vs bot.
//!
//! The engine (`HandState`) is the only thing that changes game state. The
//! human path reads a line, parses it against the observation's legal list
//! and submits the chosen `LegalAction` with `apply_legal`; the bot path
//! does the same with the bot's choice. The bot is a
//! [`mako_cfr::infer::Agent`] that receives only `observation(BOT)`; display
//! is built only from `observation(HUMAN)`.

use std::io::{self, BufRead, Write};

use mako_core::action::{ActionKind, LegalAction, Seat};
use mako_core::deck::Deck;
use mako_core::observation::PlayerObservation;
use mako_core::rng::{MakoRng, splitmix64_next};
use mako_core::rules::{RulesConfig, StackDepth};
use mako_core::state::{HandState, Match};

use crate::args::AutoPolicy;
use crate::input::{self, BetweenHands, InHand};
use crate::opponent::{Opponent, OpponentKind};
use crate::render::{self, Names, Style};
use mako_cfr::infer::InferStats;

/// The human always sits in seat Zero (and has the button on hand 1).
pub const HUMAN: Seat = Seat::Zero;
/// The bot sits in seat One.
pub const BOT: Seat = Seat::One;

const DOMAIN_DECK: u64 = 0x4445_434B; // "DECK"
const DOMAIN_BOT: u64 = 0x0042_4F54; // "BOT"
const DOMAIN_AUTO: u64 = 0x4155_544F; // "AUTO"

/// Derives an independent sub-seed from the session seed.
pub fn derive_seed(session: u64, domain: u64, index: u64) -> u64 {
    let mut s = session ^ domain.rotate_left(32);
    let a = splitmix64_next(&mut s);
    let mut t = a ^ index.wrapping_mul(0xD1B5_4A32_D192_ED03);
    splitmix64_next(&mut t)
}

/// Deck seed for hand `index` (0-based) of a session.
pub fn hand_seed(session: u64, index: u64) -> u64 {
    derive_seed(session, DOMAIN_DECK, index)
}

/// The deck for hand `index` (0-based).
pub fn hand_deck(session: u64, index: u64) -> Deck {
    Deck::from_seed(hand_seed(session, index))
}

/// Seed for the opponent agent's sampling RNG (baseline or CFR).
pub fn bot_seed(session: u64) -> u64 {
    derive_seed(session, DOMAIN_BOT, 0)
}

/// Session parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionConfig {
    pub seed: u64,
    pub hands: u64,
    pub depth: StackDepth,
    pub style: Style,
}

/// Who plays the human seat.
pub enum HumanSeat<R: BufRead> {
    /// Lines typed (or piped) on stdin.
    Lines(R),
    /// A built-in scripted policy; no input is read.
    Auto(AutoPlayer),
}

/// Scripted player for `--auto`.
#[derive(Clone, Debug)]
pub struct AutoPlayer {
    policy: AutoPolicy,
    rng: MakoRng,
}

impl AutoPlayer {
    pub fn new(policy: AutoPolicy, session_seed: u64) -> AutoPlayer {
        AutoPlayer {
            policy,
            rng: MakoRng::seed_from_u64(derive_seed(session_seed, DOMAIN_AUTO, 0)),
        }
    }

    /// Chooses from `obs.legal_actions` only.
    pub fn choose(&mut self, obs: &PlayerObservation) -> Option<LegalAction> {
        let legal = &obs.legal_actions;
        match self.policy {
            AutoPolicy::CheckCall => legal
                .iter()
                .find(|a| a.kind == ActionKind::Check)
                .or_else(|| legal.iter().find(|a| a.kind == ActionKind::Call))
                .or(legal.first())
                .copied(),
            AutoPolicy::Random => {
                if legal.is_empty() {
                    None
                } else {
                    Some(legal[self.rng.gen_range(legal.len() as u64) as usize])
                }
            }
        }
    }
}

/// How the session ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndReason {
    /// All requested hands were played.
    Completed,
    /// The player typed quit.
    Quit,
    /// Standard input reached end of file.
    InputClosed,
}

/// Outcome of a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionSummary {
    pub hands_completed: u64,
    /// Net chips per seat (indexed by `Seat::index`); always sums to zero.
    pub score: [i64; 2],
    pub end: EndReason,
    /// A hand was in progress when the player quit (not scored).
    pub abandoned_hand: bool,
}

/// Result of asking the human for a decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HumanChoice {
    Act(LegalAction),
    Quit,
    InputClosed,
}

fn help_text() -> String {
    "  Commands: type the key shown before an action (f fold, c check/call, k check,\n\
     \x20 h half-pot bet/raise, p pot bet/raise, a all-in) or its menu number.\n\
     \x20 'stats' shows the session score, 'help' this text, 'quit' ends the session\n\
     \x20 (an unfinished hand is abandoned and not scored).\n"
        .to_string()
}

/// Session score plus the opponent's decision sources. `bot` is a snapshot
/// taken at the end of the last completed hand, so it never reflects
/// decisions of the hand in progress (whether the bot's current spot was in
/// the policy depends on its private cards).
fn stats_text(m: &Match, names: Names, opponent: &Opponent, bot: Option<InferStats>) -> String {
    let s = m.score();
    let mut t = format!(
        "  Session: {} hand(s) completed. {} {} / {} {}\n",
        m.hands_played(),
        names.of(HUMAN),
        render::chips_delta(s[HUMAN.index()]),
        names.of(BOT),
        render::chips_delta(s[BOT.index()]),
    );
    match (opponent.kind, bot) {
        (OpponentKind::Cfr, Some(b)) => t.push_str(&format!(
            "  Bot decisions in completed hands: {} total, {} from policy, {} explicit \
             fallback (uniform legal, unseen infoset)\n",
            b.decisions, b.policy_hits, b.fallbacks
        )),
        (OpponentKind::Cfr, None) => {}
        (_, b) => t.push_str(&format!(
            "  Bot: {} ({} decisions; no trained policy in use)\n",
            opponent.agent.label(),
            b.map_or(0, |b| b.decisions)
        )),
    }
    t
}

fn read_line<R: BufRead>(input: &mut R) -> io::Result<Option<String>> {
    let mut line = String::new();
    match input.read_line(&mut line)? {
        0 => Ok(None),
        _ => Ok(Some(line)),
    }
}

/// Prompts until a legal action, quit, or EOF. Takes only the observation,
/// so it cannot alter the game state; invalid input just re-prompts.
pub fn read_decision<R: BufRead, W: Write>(
    obs: &PlayerObservation,
    stats: &str,
    style: Style,
    input: &mut R,
    out: &mut W,
) -> io::Result<HumanChoice> {
    loop {
        write!(out, "  {} ", style.bold("Your action>"))?;
        out.flush()?;
        let Some(line) = read_line(input)? else {
            writeln!(out)?;
            return Ok(HumanChoice::InputClosed);
        };
        match input::parse_in_hand(&line, &obs.legal_actions) {
            InHand::Act(a) => {
                writeln!(out)?;
                return Ok(HumanChoice::Act(a));
            }
            InHand::Quit => {
                writeln!(out)?;
                return Ok(HumanChoice::Quit);
            }
            InHand::Help => {
                writeln!(out)?;
                write!(out, "{}", help_text())?;
            }
            InHand::Stats => {
                writeln!(out)?;
                write!(out, "{stats}")?;
            }
            InHand::Invalid(why) => {
                writeln!(out)?;
                writeln!(out, "  Invalid input ({why}): {:?}", line.trim())?;
            }
        }
        writeln!(out, "  Options: {}", render::action_menu(obs, style))?;
    }
}

fn io_err(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}

/// Plays a session, writing the transcript to `out`.
pub fn run_session<R: BufRead, W: Write>(
    cfg: &SessionConfig,
    mut human: HumanSeat<R>,
    opponent: &mut Opponent,
    out: &mut W,
) -> io::Result<SessionSummary> {
    let style = cfg.style;
    let names = Names { human: HUMAN };
    let rules = RulesConfig::standard(cfg.depth);
    let mut m = Match::new(rules, HUMAN).map_err(io_err)?;
    let mut bot_snapshot = opponent.agent.infer_stats();

    writeln!(
        out,
        "{}",
        style.bold(&format!(
            "MAKO {} - heads-up no-limit hold'em",
            crate_version()
        ))
    )?;
    for line in &opponent.banner {
        writeln!(out, "{line}")?;
    }
    if let HumanSeat::Auto(p) = &human {
        writeln!(out, "Your seat: scripted --auto {}", p.policy.name())?;
    }
    writeln!(
        out,
        "Session seed {} | {} hand(s) | {} BB deep ({} chips) | blinds {}/{}",
        cfg.seed,
        cfg.hands,
        cfg.depth.big_blinds(),
        rules.starting_stack,
        rules.small_blind,
        rules.big_blind
    )?;
    writeln!(
        out,
        "Replay: mako --seed {} --hands {} --depth {}\n",
        cfg.seed,
        cfg.hands,
        cfg.depth.big_blinds()
    )?;

    let mut end = EndReason::Completed;
    let mut abandoned = false;

    'session: for i in 0..cfg.hands {
        let mut hand: HandState = m.start_hand(hand_deck(cfg.seed, i)).map_err(io_err)?;
        let button = hand.observation(HUMAN).button;
        writeln!(
            out,
            "{}",
            style.accent(&format!(
                "======== Hand {} of {} | button: {} ========",
                i + 1,
                cfg.hands,
                names.of(button)
            ))
        )?;
        let first = hand.observation(HUMAN);
        writeln!(
            out,
            "  Blinds posted: {} {} (SB), {} {} (BB). Your cards: {}",
            names.of(button),
            first.street_contributions[button.index()],
            names.of(button.other()),
            first.street_contributions[button.other().index()],
            style.cards(&first.hole_cards)
        )?;

        let mut steps = 0u32;
        while let Some(actor) = hand.to_act() {
            steps += 1;
            if steps > 1000 {
                return Err(io_err(
                    "hand exceeded 1000 actions; engine invariant broken",
                ));
            }
            let obs = hand.observation(actor);
            let before_street = obs.street;
            let street_max = obs.street_contributions.iter().copied().max().unwrap_or(0);
            let action = if actor == BOT {
                let d = opponent.agent.act(&obs).map_err(io_err)?;
                *obs.legal_actions
                    .iter()
                    .find(|a| a.kind == d.action)
                    .ok_or_else(|| io_err("bot chose an action that is not legal"))?
            } else {
                writeln!(out, "{}", render::table(&obs, names, style).trim_end())?;
                writeln!(out, "  Options: {}", render::action_menu(&obs, style))?;
                match &mut human {
                    HumanSeat::Auto(p) => {
                        let a = p
                            .choose(&obs)
                            .ok_or_else(|| io_err("no legal action on player's turn"))?;
                        writeln!(out, "  {} {}", style.bold("Your action>"), key_word(a))?;
                        a
                    }
                    HumanSeat::Lines(r) => {
                        let stats = stats_text(&m, names, opponent, bot_snapshot);
                        match read_decision(&obs, &stats, style, r, out)? {
                            HumanChoice::Act(a) => a,
                            HumanChoice::Quit => {
                                end = EndReason::Quit;
                                abandoned = true;
                                break 'session;
                            }
                            HumanChoice::InputClosed => {
                                end = EndReason::InputClosed;
                                abandoned = true;
                                break 'session;
                            }
                        }
                    }
                }
            };
            let outcome = hand.apply_legal(&action).map_err(io_err)?;
            writeln!(
                out,
                "  > {} {}",
                names.of(actor),
                render::describe_record(&outcome.record, street_max, actor == HUMAN)
            )?;
            let now = hand.observation(HUMAN);
            if now.street != before_street && now.terminal.is_none() {
                writeln!(
                    out,
                    "  --- {} {} | pot {} ---",
                    render::street_name(now.street),
                    style.board(&now.board),
                    now.pot
                )?;
            }
        }

        let final_obs = hand.observation(HUMAN);
        write!(out, "{}", render::result(&final_obs, names, style))?;
        if let Some(stacks) = hand.final_stacks() {
            debug_assert_eq!(
                u64::from(stacks[0]) + u64::from(stacks[1]),
                2 * u64::from(rules.starting_stack)
            );
        }
        m.record(&hand);
        bot_snapshot = opponent.agent.infer_stats();
        write!(out, "{}", stats_text(&m, names, opponent, bot_snapshot))?;
        writeln!(out)?;

        if i + 1 < cfg.hands {
            if let HumanSeat::Lines(r) = &mut human {
                loop {
                    write!(out, "  [Enter] next hand, 'stats', or 'quit'> ")?;
                    out.flush()?;
                    let Some(line) = read_line(r)? else {
                        writeln!(out)?;
                        end = EndReason::InputClosed;
                        break 'session;
                    };
                    writeln!(out)?;
                    match input::parse_between(&line) {
                        BetweenHands::Continue => break,
                        BetweenHands::Quit => {
                            end = EndReason::Quit;
                            break 'session;
                        }
                        BetweenHands::Stats => {
                            write!(out, "{}", stats_text(&m, names, opponent, bot_snapshot))?
                        }
                        BetweenHands::Help => {
                            writeln!(out, "  Press Enter (or 'c') to deal, 'quit' to stop.")?
                        }
                        BetweenHands::Invalid => {
                            writeln!(out, "  Invalid input: {:?}", line.trim())?
                        }
                    }
                }
            }
        }
    }

    let score = m.score();
    match end {
        EndReason::Completed => writeln!(out, "Session complete.")?,
        EndReason::Quit => writeln!(out, "You quit the session.")?,
        EndReason::InputClosed => writeln!(out, "Input closed; ending the session.")?,
    }
    if abandoned {
        writeln!(out, "The unfinished hand was abandoned and is not scored.")?;
    }
    writeln!(
        out,
        "Final: {} hand(s) played. You {} / Bot {} chips (vs {}).",
        m.hands_played(),
        render::chips_delta(score[HUMAN.index()]),
        render::chips_delta(score[BOT.index()]),
        opponent.agent.label()
    )?;
    out.flush()?;
    Ok(SessionSummary {
        hands_completed: m.hands_played(),
        score,
        end,
        abandoned_hand: abandoned,
    })
}

fn key_word(a: LegalAction) -> char {
    input::key(a.kind)
}

/// The crate version string.
pub const fn crate_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn cfg(seed: u64, hands: u64) -> SessionConfig {
        SessionConfig {
            seed,
            hands,
            depth: StackDepth::Bb20,
            style: Style { color: false },
        }
    }

    fn baseline(seed: u64) -> Opponent {
        crate::opponent::select(
            crate::args::BotKind::Baseline,
            None,
            StackDepth::Bb20,
            bot_seed(seed),
        )
        .unwrap()
    }

    fn run(seed: u64, hands: u64, human: HumanSeat<Cursor<Vec<u8>>>) -> (SessionSummary, String) {
        let mut out = Vec::new();
        let s = run_session(&cfg(seed, hands), human, &mut baseline(seed), &mut out).unwrap();
        (s, String::from_utf8(out).unwrap())
    }

    fn lines(s: &str) -> HumanSeat<Cursor<Vec<u8>>> {
        HumanSeat::Lines(Cursor::new(s.as_bytes().to_vec()))
    }

    #[test]
    fn auto_session_completes_and_conserves_chips() {
        for seed in 0..40 {
            for policy in [AutoPolicy::CheckCall, AutoPolicy::Random] {
                let (s, t) = run(seed, 12, HumanSeat::Auto(AutoPlayer::new(policy, seed)));
                assert_eq!(s.end, EndReason::Completed);
                assert_eq!(s.hands_completed, 12);
                assert_eq!(s.score[0] + s.score[1], 0, "zero-sum session score");
                assert!(t.contains("Session complete."));
                assert!(t.contains("untrained"));
            }
        }
    }

    #[test]
    fn session_score_equals_sum_of_engine_payoffs() {
        // Replays the same hands through the engine directly and checks the
        // session score matches the per-hand payoffs.
        let seed = 21;
        let (s, _) = run(
            seed,
            8,
            HumanSeat::Auto(AutoPlayer::new(AutoPolicy::Random, seed)),
        );
        let mut bot = mako_cfr::infer::BaselineAgent::new(bot_seed(seed));
        let mut me = AutoPlayer::new(AutoPolicy::Random, seed);
        let mut m = Match::new(RulesConfig::standard(StackDepth::Bb20), HUMAN).unwrap();
        let mut total = [0i64; 2];
        for i in 0..8 {
            let mut h = m.start_hand(hand_deck(seed, i)).unwrap();
            while let Some(a) = h.to_act() {
                let obs = h.observation(a);
                let act = if a == BOT {
                    use mako_cfr::infer::Agent;
                    let k = bot.act(&obs).unwrap().action;
                    *obs.legal_actions.iter().find(|l| l.kind == k).unwrap()
                } else {
                    me.choose(&obs).unwrap()
                };
                h.apply_legal(&act).unwrap();
            }
            let p = h.payoffs().unwrap();
            let fs = h.final_stacks().unwrap();
            assert_eq!(fs[0] + fs[1], 80);
            total[0] += p[0];
            total[1] += p[1];
            m.record(&h);
        }
        assert_eq!(s.score, total);
    }

    #[test]
    fn same_seed_same_transcript() {
        let script = "x\nc\nhelp\nc\nc\nc\n\nc\nc\nc\nc\n\nc\nc\nc\nc\n";
        let (a, ta) = run(7, 3, lines(script));
        let (b, tb) = run(7, 3, lines(script));
        assert_eq!(a, b);
        assert_eq!(ta, tb);
        let (_, auto_a) = run(
            7,
            5,
            HumanSeat::Auto(AutoPlayer::new(AutoPolicy::Random, 7)),
        );
        let (_, auto_b) = run(
            7,
            5,
            HumanSeat::Auto(AutoPlayer::new(AutoPolicy::Random, 7)),
        );
        assert_eq!(auto_a, auto_b);
        let (_, other) = run(
            8,
            5,
            HumanSeat::Auto(AutoPlayer::new(AutoPolicy::Random, 8)),
        );
        assert_ne!(auto_a, other);
    }

    #[test]
    fn invalid_input_does_not_touch_state() {
        let m = Match::new(RulesConfig::standard(StackDepth::Bb20), HUMAN).unwrap();
        let hand = m.start_hand(hand_deck(7, 0)).unwrap();
        let snapshot = hand.clone();
        let obs = hand.observation(HUMAN);
        let mut input = Cursor::new(b"zzz\n\n9\nk\nstats\nhelp\n".to_vec());
        let mut out = Vec::new();
        let stats = stats_text(&m, Names { human: HUMAN }, &baseline(7), None);
        let d = read_decision(&obs, &stats, Style { color: false }, &mut input, &mut out).unwrap();
        assert_eq!(d, HumanChoice::InputClosed);
        assert_eq!(hand, snapshot);
        let t = String::from_utf8(out).unwrap();
        assert_eq!(t.matches("Invalid input").count(), 4, "{t}");
        assert!(t.contains("Commands:"));
        assert!(t.contains("Session: 0 hand(s)"));
    }

    #[test]
    fn quit_mid_hand_and_eof() {
        let (s, t) = run(7, 3, lines("x\nquit\n"));
        assert_eq!(s.end, EndReason::Quit);
        assert!(s.abandoned_hand);
        assert_eq!(s.hands_completed, 0);
        assert_eq!(s.score, [0, 0]);
        assert!(t.contains("abandoned"));

        let (s, t) = run(7, 3, lines(""));
        assert_eq!(s.end, EndReason::InputClosed);
        assert!(t.contains("Input closed"));
    }

    #[test]
    fn quit_between_hands() {
        // Check/call until the first hand ends, then quit at the prompt.
        let mut script = String::new();
        for _ in 0..12 {
            script.push_str("c\n");
        }
        let (s, _) = run(7, 3, lines(&format!("{script}quit\n")));
        assert!(s.hands_completed >= 1);
        assert_eq!(s.score[0] + s.score[1], 0);
    }

    #[test]
    fn derived_seeds_differ() {
        assert_ne!(hand_seed(7, 0), hand_seed(7, 1));
        assert_ne!(hand_seed(7, 0), hand_seed(8, 0));
        assert_ne!(hand_seed(7, 0), bot_seed(7));
        assert_eq!(hand_seed(7, 2), hand_seed(7, 2));
    }

    #[test]
    fn button_alternates() {
        let (_, t) = run(
            3,
            4,
            HumanSeat::Auto(AutoPlayer::new(AutoPolicy::CheckCall, 3)),
        );
        let buttons: Vec<&str> = t
            .lines()
            .filter_map(|l| l.split("button: ").nth(1))
            .map(|r| r.split(' ').next().unwrap())
            .collect();
        assert_eq!(buttons, ["You", "Bot", "You", "Bot"]);
    }
}

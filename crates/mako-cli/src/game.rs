//! The session loop: N hands of human vs bot.
//!
//! The engine (`HandState`) is the only thing that changes game state. The
//! human path reads a line, parses it against the observation's legal list
//! and submits the chosen `LegalAction` with `apply_legal`; the bot path
//! does the same with the bot's choice. The bot is a
//! [`mako_cfr::infer::Agent`] that receives only `observation(BOT)`; display
//! is built only from `observation(HUMAN)`.
//!
//! Hand history (M-17): every **completed** hand is recorded with
//! [`SessionRecorder::record_hand`] (which keeps only what the human seat may
//! know) and saved right away. A hand abandoned by `quit` or end of input is
//! neither scored nor recorded. A failed save prints a warning and play
//! continues; the hand stays in memory and is written with the next save.

use std::io::{self, BufRead, Write};

use mako_core::action::{ActionKind, LegalAction, Seat};
use mako_core::deck::Deck;
use mako_core::observation::PlayerObservation;
use mako_core::rng::{MakoRng, splitmix64_next};
use mako_core::rules::{RulesConfig, StackDepth};
use mako_core::state::{HandState, Match};

use mako_analysis::{SessionRecorder, SessionStats as AnalysisStats};

use crate::args::AutoPolicy;
use crate::input::{self, BetweenHands, InHand};
use crate::opponent::{Opponent, OpponentKind};
use crate::ui::stats::{self, BotSources, HistoryView, SessionStats};
use crate::ui::table::{self, TableInfo};
use crate::ui::{Names, Role, Ui, chips_delta, help, screens, term};
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionConfig {
    pub seed: u64,
    pub hands: u64,
    pub depth: StackDepth,
    pub ui: Ui,
    /// Command line that replays the session (shown in the header).
    pub replay: String,
    /// Pause before each bot action and street (zero = none). Only set for
    /// interactive TTY play without `--fast`.
    pub pace: std::time::Duration,
    /// Extra lines for the session header (e.g. history load notices).
    pub notices: Vec<String>,
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

/// Session and lifetime analysis statistics from the recorder.
/// `session_start` is the number of records loaded before this session.
fn history_view(rec: &SessionRecorder, session_start: usize) -> HistoryView {
    let session = rec.records().get(session_start..).unwrap_or(&[]);
    HistoryView {
        session: AnalysisStats::from_records(session),
        lifetime: *rec.stats(),
        storage: match rec.store() {
            Some(s) => format!("saved to {}", s.path().display()),
            None => "not saved this session".to_string(),
        },
    }
}

fn session_stats(
    m: &Match,
    opponent: &Opponent,
    bot: Option<InferStats>,
    bb: u32,
    history: Option<HistoryView>,
) -> SessionStats {
    let s = m.score();
    let decisions = bot.map_or(0, |b| b.decisions);
    SessionStats {
        hands_completed: m.hands_played(),
        you: s[HUMAN.index()],
        bot: s[BOT.index()],
        big_blind: bb,
        opponent: opponent.agent.label(),
        sources: match (opponent.kind, bot) {
            (OpponentKind::Cfr, Some(b)) => BotSources::Policy {
                decisions: b.decisions,
                policy: b.policy_hits,
                fallback: b.fallbacks,
            },
            _ => BotSources::Untrained { decisions },
        },
        history,
    }
}

/// Greedy word wrap that never cuts a word: a file path longer than the
/// width gets a line of its own so it stays complete and copyable.
pub fn wrap_whole_words(text: &str, width: usize, indent: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let used = if line.is_empty() {
            0
        } else {
            line.chars().count() + 1
        };
        if !line.is_empty() && used + word.chars().count() > width {
            out.push(std::mem::take(&mut line));
        }
        if line.is_empty() {
            line.push_str(if out.is_empty() { "" } else { indent });
        } else {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}

fn write_lines<W: Write>(out: &mut W, lines: &[String]) -> io::Result<()> {
    for l in lines {
        writeln!(out, "{l}")?;
    }
    Ok(())
}

fn read_line<R: BufRead>(input: &mut R) -> io::Result<Option<String>> {
    let mut line = String::new();
    match input.read_line(&mut line)? {
        0 => Ok(None),
        _ => Ok(Some(line)),
    }
}

/// What the in-hand prompt can show besides the menu.
pub struct PromptContext<'a> {
    /// Rendered stats panel.
    pub stats: &'a [String],
    /// Rendered help.
    pub help: &'a [String],
}

/// Prompts until a legal action, quit, or EOF. Takes only the observation,
/// so it cannot alter the game state; invalid input just re-prompts.
pub fn read_decision<R: BufRead, W: Write>(
    obs: &PlayerObservation,
    ctx: &PromptContext<'_>,
    ui: &Ui,
    input: &mut R,
    out: &mut W,
) -> io::Result<HumanChoice> {
    loop {
        write!(out, "{}", screens::action_prompt(ui))?;
        out.flush()?;
        let Some(line) = read_line(input)? else {
            writeln!(out)?;
            return Ok(HumanChoice::InputClosed);
        };
        writeln!(out)?;
        match input::parse_in_hand(&line, &obs.legal_actions) {
            InHand::Act(a) => return Ok(HumanChoice::Act(a)),
            InHand::Quit => return Ok(HumanChoice::Quit),
            InHand::Help => write_lines(out, ctx.help)?,
            InHand::Stats => write_lines(out, ctx.stats)?,
            InHand::Invalid(why) => {
                let msg = format!("  Invalid input ({why}): {:?}", line.trim());
                writeln!(out, "{}", ui.paint(Role::Warn, &term::fit(&msg, ui.width)))?;
            }
        }
        write_lines(out, &table::menu(obs, ui))?;
    }
}

fn io_err(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}

/// Plays a session without persistent history (an in-memory recorder).
pub fn run_session<R: BufRead, W: Write>(
    cfg: &SessionConfig,
    human: HumanSeat<R>,
    opponent: &mut Opponent,
    out: &mut W,
) -> io::Result<SessionSummary> {
    run_session_with_history(cfg, human, opponent, &mut SessionRecorder::ephemeral(), out)
}

/// Plays a session, writing the transcript to `out` and recording every
/// completed hand in `history` (saved after each hand).
pub fn run_session_with_history<R: BufRead, W: Write>(
    cfg: &SessionConfig,
    mut human: HumanSeat<R>,
    opponent: &mut Opponent,
    history: &mut SessionRecorder,
    out: &mut W,
) -> io::Result<SessionSummary> {
    let ui = cfg.ui;
    let names = Names { human: HUMAN };
    let rules = RulesConfig::standard(cfg.depth);
    let bb = rules.big_blind;
    let mut m = Match::new(rules, HUMAN).map_err(io_err)?;
    let mut bot_snapshot = opponent.agent.infer_stats();
    let label = opponent.agent.label();
    let history_label = opponent.history_label();
    let session_start = history.records().len();
    let pause = |out: &mut W| -> io::Result<()> {
        if !cfg.pace.is_zero() {
            out.flush()?;
            std::thread::sleep(cfg.pace);
        }
        Ok(())
    };

    let dot = ui.frame().dot;
    let mut setup = vec![format!(
        "Session seed {} {dot} {} hand(s) {dot} {} BB deep ({} chips) {dot} blinds {}/{}",
        cfg.seed,
        cfg.hands,
        cfg.depth.big_blinds(),
        rules.starting_stack,
        rules.small_blind,
        rules.big_blind
    )];
    if let HumanSeat::Auto(p) = &human {
        setup.push(format!("Your seat: scripted --auto {}", p.policy.name()));
    }
    setup.push(format!("Replay: {}", cfg.replay));
    write_lines(
        out,
        &screens::session_header(&ui, crate_version(), &opponent.banner, &setup),
    )?;
    for n in &cfg.notices {
        let role = if n.contains("WARNING") {
            Role::Warn
        } else {
            Role::Dim
        };
        for l in wrap_whole_words(n, ui.width, "  ") {
            writeln!(out, "{}", ui.paint(role, &l))?;
        }
    }
    writeln!(out)?;

    let mut end = EndReason::Completed;
    let mut abandoned = false;

    'session: for i in 0..cfg.hands {
        let mut hand: HandState = m.start_hand(hand_deck(cfg.seed, i)).map_err(io_err)?;
        let info = TableInfo {
            hand: i + 1,
            hands: cfg.hands,
            opponent: &label,
            names,
        };
        write_lines(
            out,
            &screens::hand_header(&ui, &hand.observation(HUMAN), i + 1, cfg.hands, names),
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
            let action = if actor == BOT {
                pause(out)?;
                let d = opponent.agent.act(&obs).map_err(io_err)?;
                *obs.legal_actions
                    .iter()
                    .find(|a| a.kind == d.action)
                    .ok_or_else(|| io_err("bot chose an action that is not legal"))?
            } else {
                write_lines(out, &table::table_screen(&obs, &info, &ui))?;
                write_lines(out, &table::menu(&obs, &ui))?;
                match &mut human {
                    HumanSeat::Auto(p) => {
                        let a = p
                            .choose(&obs)
                            .ok_or_else(|| io_err("no legal action on player's turn"))?;
                        writeln!(out, "{}{}", screens::action_prompt(&ui), input::key(a.kind))?;
                        a
                    }
                    HumanSeat::Lines(r) => {
                        let view = history_view(history, session_start);
                        let stats = stats::render_full(
                            &session_stats(&m, opponent, bot_snapshot, bb, Some(view)),
                            &ui,
                        );
                        let help = help::in_hand(&ui, &label);
                        let ctx = PromptContext {
                            stats: &stats,
                            help: &help,
                        };
                        match read_decision(&obs, &ctx, &ui, r, out)? {
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
                "{}",
                screens::action_event(&ui, &outcome.record, &obs, names)
            )?;
            let now = hand.observation(HUMAN);
            if now.street != before_street && now.terminal.is_none() {
                pause(out)?;
                writeln!(out, "{}", screens::street_banner(&ui, &now))?;
            }
        }

        write_lines(out, &screens::result(&ui, &hand.observation(HUMAN), names))?;
        if let Some(stacks) = hand.final_stacks() {
            debug_assert_eq!(
                u64::from(stacks[0]) + u64::from(stacks[1]),
                2 * u64::from(rules.starting_stack)
            );
        }
        m.record(&hand);
        let saved = match history.record_hand(&hand, HUMAN, &history_label) {
            Ok(_) => history.save().map_err(|e| e.to_string()),
            Err(e) => Err(format!("could not record the hand: {e}")),
        };
        if let Err(e) = saved {
            let msg = format!("  Warning: hand history not saved ({e}); play continues.");
            write_lines(
                out,
                &term::wrap(&msg, ui.width, "", "    ")
                    .into_iter()
                    .map(|l| ui.paint(Role::Warn, &l))
                    .collect::<Vec<_>>(),
            )?;
        }
        bot_snapshot = opponent.agent.infer_stats();
        let view = history_view(history, session_start);
        write_lines(
            out,
            &stats::render(
                &session_stats(&m, opponent, bot_snapshot, bb, Some(view)),
                &ui,
            ),
        )?;
        writeln!(out)?;

        if i + 1 < cfg.hands {
            if let HumanSeat::Lines(r) = &mut human {
                loop {
                    write!(out, "{}", screens::between_prompt(&ui))?;
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
                            let view = history_view(history, session_start);
                            write_lines(
                                out,
                                &stats::render_full(
                                    &session_stats(&m, opponent, bot_snapshot, bb, Some(view)),
                                    &ui,
                                ),
                            )?
                        }
                        BetweenHands::Help => write_lines(out, &help::between_hands(&ui, &label))?,
                        BetweenHands::Invalid => {
                            let msg = format!("  Invalid input: {:?}", line.trim());
                            writeln!(out, "{}", ui.paint(Role::Warn, &term::fit(&msg, ui.width)))?
                        }
                    }
                }
            }
        }
    }

    let score = m.score();
    writeln!(out, "{}", ui.rule(""))?;
    match end {
        EndReason::Completed => writeln!(out, "Session complete.")?,
        EndReason::Quit => writeln!(out, "You quit the session.")?,
        EndReason::InputClosed => writeln!(out, "Input closed; ending the session.")?,
    }
    if abandoned {
        write_lines(
            out,
            &term::wrap(
                "The unfinished hand was abandoned and is not scored or saved.",
                ui.width,
                "",
                "    ",
            ),
        )?;
    }
    let final_line = format!(
        "Final: {} hand(s) played. You {} / Bot {} chips.",
        m.hands_played(),
        chips_delta(score[HUMAN.index()]),
        chips_delta(score[BOT.index()]),
    );
    write_lines(out, &term::wrap(&final_line, ui.width, "", "    "))?;
    write_lines(
        out,
        &term::wrap(&format!("Opponent: {label}"), ui.width, "", "    "),
    )?;
    out.flush()?;
    Ok(SessionSummary {
        hands_completed: m.hands_played(),
        score,
        end,
        abandoned_hand: abandoned,
    })
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
            ui: Ui::plain(),
            replay: format!("mako --seed {seed}"),
            pace: std::time::Duration::ZERO,
            notices: Vec::new(),
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
        let stats = stats::render(
            &session_stats(&m, &baseline(7), None, 2, None),
            &Ui::plain(),
        );
        let help = help::in_hand(&Ui::plain(), "baseline");
        let ctx = PromptContext {
            stats: &stats,
            help: &help,
        };
        let d = read_decision(&obs, &ctx, &Ui::plain(), &mut input, &mut out).unwrap();
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
    fn whole_word_wrap_keeps_paths() {
        let path = format!("/{}", "x".repeat(100));
        let lines = wrap_whole_words(&format!("moved to {path} ok now"), 40, "  ");
        assert!(lines.iter().any(|l| l.trim() == path), "{lines:?}");
        assert_eq!(lines.join(" ").split_whitespace().count(), 5);
        assert!(
            lines
                .iter()
                .filter(|l| !l.contains(&path))
                .all(|l| l.len() <= 40)
        );
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

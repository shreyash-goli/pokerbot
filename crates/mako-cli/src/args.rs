//! Command-line parsing (no external dependencies).

use std::fmt;

use mako_cfr::infer::Difficulty;
use mako_core::rules::StackDepth;

use crate::ui::term::{ColorChoice, ColorMode, GlyphChoice, Glyphs, MAX_WIDTH, MIN_WIDTH};

/// Default number of hands per session.
pub const DEFAULT_HANDS: u64 = 10;
/// Upper bound on `--hands`, so a typo cannot start an effectively endless
/// session.
pub const MAX_HANDS: u64 = 100_000;

/// Which bot plays the opponent seat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BotKind {
    /// CFR policy if a valid artifact for the depth loads; otherwise the
    /// untrained baseline, announced with a loud banner.
    Auto,
    /// CFR policy; a missing or invalid artifact is a hard error.
    Cfr,
    /// Seeded weighted-random legal bot (untrained).
    Baseline,
}

/// Scripted policy for the human seat (`--auto`), for tests and demos.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AutoPolicy {
    /// Check when possible, otherwise call.
    CheckCall,
    /// Uniformly random legal action (seeded from the session seed).
    Random,
}

impl AutoPolicy {
    /// Name as accepted on the command line.
    pub const fn name(self) -> &'static str {
        match self {
            AutoPolicy::CheckCall => "checkcall",
            AutoPolicy::Random => "random",
        }
    }
}

/// Where the hand history (M-17) is kept.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum HistoryChoice {
    /// `$MAKO_HOME/session-history.txt`, else `~/.mako/session-history.txt`.
    #[default]
    Default,
    /// `--history <path>`.
    Path(String),
    /// `--no-history`: in memory only, nothing is written.
    Disabled,
}

/// A fully parsed session configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    /// `None` means "derive from the clock" (done in `main`).
    pub seed: Option<u64>,
    pub hands: u64,
    pub depth: StackDepth,
    pub bot: BotKind,
    /// `--color` / `--no-color`.
    pub color: ColorChoice,
    /// `--glyphs`.
    pub glyphs: GlyphChoice,
    /// `--width`; `None` means `COLUMNS` or 80.
    pub width: Option<usize>,
    /// `--fast`: no pauses between bot actions.
    pub fast: bool,
    pub auto: Option<AutoPolicy>,
    /// `--strategy`; `None` means `artifacts/smoke_<depth>bb.mako`.
    pub strategy: Option<String>,
    /// `--history <path>` / `--no-history`.
    pub history: HistoryChoice,
    /// `--reset-stats`: delete the saved history file and exit.
    pub reset_stats: bool,
    /// `--adapt`: experimental adaptive opponent (M-20). Off by default.
    pub adapt: bool,
    /// `--difficulty` (M-21): how the CFR policy's mix is blended with
    /// uniform-random legal play. Default challenging (policy as-is).
    pub difficulty: Difficulty,
}

impl Options {
    /// The bot names as accepted by `--bot`.
    pub const fn bot_name(&self) -> &'static str {
        match self.bot {
            BotKind::Auto => "auto",
            BotKind::Cfr => "cfr",
            BotKind::Baseline => "baseline",
        }
    }

    /// A command line that replays this session with `seed` (same deck,
    /// bot and scripted-player streams). `strategy` is the artifact path
    /// actually used (omitted for the baseline).
    pub fn replay_command(&self, seed: u64, strategy: &str) -> String {
        let mut c = format!(
            "mako --seed {seed} --hands {} --depth {} --bot {}",
            self.hands,
            self.depth.big_blinds(),
            self.bot_name()
        );
        if self.bot != BotKind::Baseline {
            if strategy.contains(char::is_whitespace) {
                c.push_str(&format!(" --strategy '{strategy}'"));
            } else {
                c.push_str(&format!(" --strategy {strategy}"));
            }
            c.push_str(&format!(" --difficulty {}", self.difficulty.name()));
        }
        if let Some(a) = self.auto {
            c.push_str(&format!(" --auto {}", a.name()));
        }
        if self.adapt {
            c.push_str(" --adapt");
        }
        c
    }
}

impl Default for Options {
    fn default() -> Self {
        Options {
            seed: None,
            hands: DEFAULT_HANDS,
            depth: StackDepth::Bb20,
            bot: BotKind::Auto,
            color: ColorChoice::Auto,
            glyphs: GlyphChoice::Auto,
            width: None,
            fast: false,
            auto: None,
            strategy: None,
            history: HistoryChoice::Default,
            reset_stats: false,
            adapt: false,
            difficulty: Difficulty::Challenging,
        }
    }
}

/// What the command line asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Help,
    Version,
    Play(Options),
}

/// A command-line error. Displayed to the user; exit status 2.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArgError {
    UnknownFlag(String),
    MissingValue(&'static str),
    InvalidValue {
        flag: &'static str,
        value: String,
    },
    UnknownBot(String),
    UnknownAuto(String),
    /// The same option given twice (or `--color` with `--no-color`).
    Repeated(&'static str),
    /// Two options that cannot be combined.
    Conflict(&'static str, &'static str),
}

impl fmt::Display for ArgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ArgError::UnknownFlag(s) => write!(f, "unknown argument '{s}' (see --help)"),
            ArgError::MissingValue(flag) => write!(f, "{flag} requires a value"),
            ArgError::InvalidValue { flag, value } => {
                let hint = match *flag {
                    "--depth" => " (expected 20, 50 or 100)",
                    "--hands" => " (expected 1..=100000)",
                    "--width" => " (expected 48..=240)",
                    "--color" => " (expected auto, never, 16, 256 or truecolor)",
                    "--glyphs" => " (expected auto, unicode or ascii)",
                    "--difficulty" => " (expected training, balanced or challenging)",
                    "--seed" => " (expected an unsigned 64-bit integer)",
                    _ => "",
                };
                write!(f, "invalid value '{value}' for {flag}{hint}")
            }
            ArgError::UnknownBot(name) => {
                write!(f, "unknown bot '{name}' (available: auto, cfr, baseline)")
            }
            ArgError::Repeated(flag) => write!(
                f,
                "{flag} given more than once (each option may appear once; \
                 --no-color counts as --color, --no-history as --history)"
            ),
            ArgError::Conflict(a, b) => write!(f, "{a} cannot be combined with {b}"),
            ArgError::UnknownAuto(name) => {
                write!(
                    f,
                    "unknown --auto policy '{name}' (available: checkcall, random)"
                )
            }
        }
    }
}

impl std::error::Error for ArgError {}

/// Usage text for `--help`.
pub const USAGE: &str = "\
mako - heads-up no-limit hold'em in the terminal (human vs bot)

USAGE:
    mako [OPTIONS]

OPTIONS:
    --seed <u64>          Session seed (default: from the clock; always printed)
    --hands <n>           Number of hands to play (default 10, max 100000)
    --depth <20|50|100>   Starting stack in big blinds at 1/2 blinds (default 20)
    --bot <auto|cfr|baseline>
                          Opponent (default auto). cfr: trained policy from
                          --strategy, error if missing/invalid. baseline:
                          untrained, random legal. auto: cfr if the artifact
                          loads for this depth, else baseline with a warning.
    --strategy <path>     Policy artifact (default artifacts/smoke_<depth>bb.mako)
    --auto <policy>       Play the human seat with a scripted policy
                          (checkcall | random); reads no input
    --color <auto|never|16|256|truecolor>
                          Colour depth (default auto: NO_COLOR, non-TTY or
                          TERM=dumb -> never; COLORTERM=truecolor -> truecolor;
                          TERM=*256color -> 256; else 16)
    --no-color            Same as --color never
    --glyphs <auto|unicode|ascii>
                          Suit symbols and box frames, or ASCII (default auto:
                          unicode if LC_ALL/LC_CTYPE/LANG says UTF-8)
    --width <48..240>     Layout width (default: COLUMNS, else 80; >= 100 uses
                          the wide two-pane table)
    --fast                No pauses between bot actions (pauses only ever
                          happen in interactive TTY play)
    --history <path>      Hand history file (default $MAKO_HOME/session-history.txt,
                          else ~/.mako/session-history.txt). Every completed hand
                          is saved; quit/abandoned hands are not. Stats are
                          computed from public actions only.
    --no-history          Keep stats in memory only; write nothing
    --reset-stats         Delete the saved hand history (only a Mako history
                          file), print what was deleted, and exit
    --difficulty <training|balanced|challenging>
                          CFR bot only (default challenging = trained policy
                          as-is). balanced = 75% policy + 25% uniform-random
                          legal; training = 40% policy + 60% uniform-random
                          legal. Blends of a smoke policy; relative strength
                          not benchmarked. The baseline is unaffected
    --adapt               EXPERIMENTAL, off by default: the bot conservatively
                          adapts to your public tendencies (never your cards);
                          not evaluated as stronger. See 'profile' between hands
    -h, --help            Show this help
    -V, --version         Show the version

Each option may be given at most once.

IN-HAND COMMANDS:
    action keys shown in the menu (f c k h p a), menu numbers, or the
    words fold/check/call/half/pot/allin; help (?), stats (s), quit (q)
BETWEEN HANDS:
    Enter / continue (c), stats (s), help (?), quit (q)
";

fn take_value<'a, I: Iterator<Item = &'a str>>(
    flag: &'static str,
    inline: Option<&'a str>,
    rest: &mut I,
) -> Result<&'a str, ArgError> {
    match inline {
        Some(v) => Ok(v),
        None => rest.next().ok_or(ArgError::MissingValue(flag)),
    }
}

fn invalid(flag: &'static str, value: &str) -> ArgError {
    ArgError::InvalidValue {
        flag,
        value: value.to_string(),
    }
}

/// Parses arguments (excluding the program name). Accepts `--flag value`
/// and `--flag=value`.
pub fn parse<S: AsRef<str>>(args: &[S]) -> Result<Command, ArgError> {
    let mut opts = Options::default();
    let mut seen: Vec<&'static str> = Vec::new();
    let mut once = |flag: &'static str| {
        if seen.contains(&flag) {
            Err(ArgError::Repeated(flag))
        } else {
            seen.push(flag);
            Ok(())
        }
    };
    let mut it = args.iter().map(AsRef::as_ref);
    while let Some(arg) = it.next() {
        let (name, inline) = match arg.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n, Some(v)),
            _ => (arg, None),
        };
        match name {
            "--seed" | "--hands" | "--depth" | "--bot" | "--strategy" | "--auto" | "--width"
            | "--glyphs" | "--fast" | "--reset-stats" | "--adapt" | "--difficulty" => {
                once(match name {
                    "--seed" => "--seed",
                    "--hands" => "--hands",
                    "--depth" => "--depth",
                    "--bot" => "--bot",
                    "--strategy" => "--strategy",
                    "--auto" => "--auto",
                    "--width" => "--width",
                    "--glyphs" => "--glyphs",
                    "--reset-stats" => "--reset-stats",
                    "--adapt" => "--adapt",
                    "--difficulty" => "--difficulty",
                    _ => "--fast",
                })?
            }
            "--history" | "--no-history" => once("--history")?,
            "--color" | "--colour" | "--no-color" | "--no-colour" => once("--color")?,
            _ => {}
        }
        match name {
            "-h" | "--help" => return Ok(Command::Help),
            "-V" | "--version" => return Ok(Command::Version),
            "--no-color" | "--no-colour" => opts.color = ColorChoice::Force(ColorMode::None),
            "--color" | "--colour" => {
                let v = take_value("--color", inline, &mut it)?;
                opts.color = match v.to_ascii_lowercase().as_str() {
                    "auto" => ColorChoice::Auto,
                    "never" | "none" => ColorChoice::Force(ColorMode::None),
                    "16" => ColorChoice::Force(ColorMode::Ansi16),
                    "256" => ColorChoice::Force(ColorMode::Ansi256),
                    "truecolor" | "24bit" => ColorChoice::Force(ColorMode::TrueColor),
                    _ => return Err(invalid("--color", v)),
                };
            }
            "--glyphs" => {
                let v = take_value("--glyphs", inline, &mut it)?;
                opts.glyphs = match v.to_ascii_lowercase().as_str() {
                    "auto" => GlyphChoice::Auto,
                    "unicode" => GlyphChoice::Force(Glyphs::Unicode),
                    "ascii" => GlyphChoice::Force(Glyphs::Ascii),
                    _ => return Err(invalid("--glyphs", v)),
                };
            }
            "--width" => {
                let v = take_value("--width", inline, &mut it)?;
                let w: usize = v.parse().map_err(|_| invalid("--width", v))?;
                if !(MIN_WIDTH..=MAX_WIDTH).contains(&w) {
                    return Err(invalid("--width", v));
                }
                opts.width = Some(w);
            }
            "--fast" => opts.fast = true,
            "--reset-stats" => opts.reset_stats = true,
            "--adapt" => opts.adapt = true,
            "--difficulty" => {
                let v = take_value("--difficulty", inline, &mut it)?;
                opts.difficulty = Difficulty::parse(v).ok_or_else(|| invalid("--difficulty", v))?;
            }
            "--no-history" => opts.history = HistoryChoice::Disabled,
            "--history" => {
                let v = take_value("--history", inline, &mut it)?;
                if v.is_empty() {
                    return Err(invalid("--history", v));
                }
                opts.history = HistoryChoice::Path(v.to_string());
            }
            "--seed" => {
                let v = take_value("--seed", inline, &mut it)?;
                opts.seed = Some(v.parse().map_err(|_| invalid("--seed", v))?);
            }
            "--hands" => {
                let v = take_value("--hands", inline, &mut it)?;
                let n: u64 = v.parse().map_err(|_| invalid("--hands", v))?;
                if n == 0 || n > MAX_HANDS {
                    return Err(invalid("--hands", v));
                }
                opts.hands = n;
            }
            "--depth" => {
                let v = take_value("--depth", inline, &mut it)?;
                opts.depth = match v.trim_end_matches(['b', 'B']) {
                    "20" => StackDepth::Bb20,
                    "50" => StackDepth::Bb50,
                    "100" => StackDepth::Bb100,
                    _ => return Err(invalid("--depth", v)),
                };
            }
            "--bot" => {
                let v = take_value("--bot", inline, &mut it)?;
                opts.bot = match v.to_ascii_lowercase().as_str() {
                    "auto" => BotKind::Auto,
                    "cfr" => BotKind::Cfr,
                    "baseline" => BotKind::Baseline,
                    _ => return Err(ArgError::UnknownBot(v.to_string())),
                };
            }
            "--strategy" => {
                let v = take_value("--strategy", inline, &mut it)?;
                if v.is_empty() {
                    return Err(invalid("--strategy", v));
                }
                opts.strategy = Some(v.to_string());
            }
            "--auto" => {
                let v = take_value("--auto", inline, &mut it)?;
                opts.auto = Some(match v.to_ascii_lowercase().as_str() {
                    "checkcall" | "check-call" => AutoPolicy::CheckCall,
                    "random" => AutoPolicy::Random,
                    _ => return Err(ArgError::UnknownAuto(v.to_string())),
                });
            }
            _ => return Err(ArgError::UnknownFlag(arg.to_string())),
        }
    }
    if opts.reset_stats && opts.history == HistoryChoice::Disabled {
        return Err(ArgError::Conflict("--reset-stats", "--no-history"));
    }
    Ok(Command::Play(opts))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn play(args: &[&str]) -> Options {
        match parse(args) {
            Ok(Command::Play(o)) => o,
            other => panic!("expected Play, got {other:?}"),
        }
    }

    #[test]
    fn defaults() {
        let o = play(&[]);
        assert_eq!(o, Options::default());
        assert_eq!(o.depth, StackDepth::Bb20);
        assert_eq!(o.hands, DEFAULT_HANDS);
        assert_eq!(o.seed, None);
        assert_eq!(o.bot, BotKind::Auto);
        assert_eq!(o.strategy, None);
    }

    #[test]
    fn full_flag_set() {
        let o = play(&[
            "--seed",
            "7",
            "--hands",
            "3",
            "--bot",
            "baseline",
            "--depth",
            "100",
            "--no-color",
            "--auto",
            "checkcall",
        ]);
        assert_eq!(o.seed, Some(7));
        assert_eq!(o.hands, 3);
        assert_eq!(o.depth, StackDepth::Bb100);
        assert_eq!(o.bot, BotKind::Baseline);
        assert_eq!(o.color, ColorChoice::Force(ColorMode::None));
        assert_eq!(o.auto, Some(AutoPolicy::CheckCall));
    }

    #[test]
    fn equals_syntax_and_depths() {
        let o = play(&["--seed=18446744073709551615", "--depth=50", "--auto=random"]);
        assert_eq!(o.seed, Some(u64::MAX));
        assert_eq!(o.depth, StackDepth::Bb50);
        assert_eq!(o.auto, Some(AutoPolicy::Random));
        assert_eq!(play(&["--depth", "20"]).depth, StackDepth::Bb20);
    }

    #[test]
    fn bot_kinds_and_strategy() {
        assert_eq!(play(&["--bot", "cfr"]).bot, BotKind::Cfr);
        assert_eq!(play(&["--bot", "AUTO"]).bot, BotKind::Auto);
        assert_eq!(play(&["--bot=baseline"]).bot, BotKind::Baseline);
        let o = play(&["--strategy", "x/y.mako"]);
        assert_eq!(o.strategy.as_deref(), Some("x/y.mako"));
        assert_eq!(
            play(&["--strategy=z.mako"]).strategy.as_deref(),
            Some("z.mako")
        );
        assert_eq!(
            parse(&["--strategy"]),
            Err(ArgError::MissingValue("--strategy"))
        );
        assert!(parse(&["--strategy="]).is_err());
    }

    #[test]
    fn rejects_bad_input() {
        assert!(matches!(
            parse(&["--bot", "gto"]),
            Err(ArgError::UnknownBot(_))
        ));
        assert!(matches!(
            parse(&["--depth", "30"]),
            Err(ArgError::InvalidValue {
                flag: "--depth",
                ..
            })
        ));
        assert!(matches!(
            parse(&["--hands", "0"]),
            Err(ArgError::InvalidValue {
                flag: "--hands",
                ..
            })
        ));
        assert!(matches!(
            parse(&["--hands", "100001"]),
            Err(ArgError::InvalidValue { .. })
        ));
        assert!(matches!(
            parse(&["--seed", "-1"]),
            Err(ArgError::InvalidValue { flag: "--seed", .. })
        ));
        assert_eq!(parse(&["--seed"]), Err(ArgError::MissingValue("--seed")));
        assert!(matches!(
            parse(&["--frobnicate"]),
            Err(ArgError::UnknownFlag(_))
        ));
        assert!(matches!(
            parse(&["--auto", "bluff"]),
            Err(ArgError::UnknownAuto(_))
        ));
    }

    #[test]
    fn ui_flags() {
        let o = play(&[
            "--color", "256", "--glyphs", "unicode", "--width", "120", "--fast",
        ]);
        assert_eq!(o.color, ColorChoice::Force(ColorMode::Ansi256));
        assert_eq!(o.glyphs, GlyphChoice::Force(Glyphs::Unicode));
        assert_eq!(o.width, Some(120));
        assert!(o.fast);
        assert_eq!(
            play(&["--color=truecolor"]).color,
            ColorChoice::Force(ColorMode::TrueColor)
        );
        assert_eq!(
            play(&["--color", "never"]).color,
            ColorChoice::Force(ColorMode::None)
        );
        assert!(matches!(
            parse(&["--width", "47"]),
            Err(ArgError::InvalidValue { .. })
        ));
        assert!(matches!(
            parse(&["--width", "241"]),
            Err(ArgError::InvalidValue { .. })
        ));
        assert!(matches!(
            parse(&["--color", "pink"]),
            Err(ArgError::InvalidValue { .. })
        ));
        assert!(matches!(
            parse(&["--glyphs", "emoji"]),
            Err(ArgError::InvalidValue { .. })
        ));
    }

    #[test]
    fn repeated_flags_are_rejected() {
        assert_eq!(
            parse(&["--seed", "1", "--seed", "2"]),
            Err(ArgError::Repeated("--seed"))
        );
        assert_eq!(
            parse(&["--hands=3", "--hands", "4"]),
            Err(ArgError::Repeated("--hands"))
        );
        assert_eq!(
            parse(&["--no-color", "--color", "256"]),
            Err(ArgError::Repeated("--color"))
        );
        assert_eq!(
            parse(&["--fast", "--fast"]),
            Err(ArgError::Repeated("--fast"))
        );
        let msg = ArgError::Repeated("--bot").to_string();
        assert!(msg.contains("more than once"), "{msg}");
    }

    #[test]
    fn difficulty_flag_parses_rejects_and_replays() {
        assert_eq!(play(&[]).difficulty, Difficulty::Challenging);
        for d in Difficulty::ALL {
            let o = play(&["--difficulty", d.name(), "--bot", "cfr"]);
            assert_eq!(o.difficulty, d);
            assert!(
                o.replay_command(1, "p.mako")
                    .contains(&format!("--difficulty {}", d.name()))
            );
        }
        assert_eq!(
            play(&["--difficulty=Training"]).difficulty,
            Difficulty::Training
        );
        assert!(matches!(
            parse(&["--difficulty", "easy"]),
            Err(ArgError::InvalidValue {
                flag: "--difficulty",
                ..
            })
        ));
        assert_eq!(
            parse(&["--difficulty"]),
            Err(ArgError::MissingValue("--difficulty"))
        );
        assert_eq!(
            parse(&["--difficulty", "training", "--difficulty", "balanced"]),
            Err(ArgError::Repeated("--difficulty"))
        );
        let b = play(&["--bot", "baseline", "--difficulty", "training"]);
        assert!(
            !b.replay_command(1, "x").contains("--difficulty"),
            "baseline is unaffected"
        );
    }

    #[test]
    fn replay_command_includes_bot_strategy_auto() {
        let o = play(&[
            "--hands",
            "3",
            "--bot",
            "cfr",
            "--auto",
            "checkcall",
            "--depth",
            "50",
        ]);
        assert_eq!(
            o.replay_command(7, "artifacts/smoke_50bb.mako"),
            "mako --seed 7 --hands 3 --depth 50 --bot cfr --strategy artifacts/smoke_50bb.mako --difficulty challenging --auto checkcall"
        );
        let b = play(&["--bot", "baseline"]);
        assert_eq!(
            b.replay_command(1, "ignored"),
            "mako --seed 1 --hands 10 --depth 20 --bot baseline"
        );
        assert!(
            play(&[])
                .replay_command(1, "a b.mako")
                .ends_with("--strategy 'a b.mako' --difficulty challenging")
        );
    }

    #[test]
    fn history_flags() {
        assert_eq!(play(&[]).history, HistoryChoice::Default);
        assert_eq!(
            play(&["--history", "/tmp/x.txt"]).history,
            HistoryChoice::Path("/tmp/x.txt".into())
        );
        assert_eq!(
            play(&["--history=h.txt"]).history,
            HistoryChoice::Path("h.txt".into())
        );
        assert_eq!(play(&["--no-history"]).history, HistoryChoice::Disabled);
        let o = play(&["--reset-stats", "--history", "h.txt"]);
        assert!(o.reset_stats);
        assert_eq!(
            parse(&["--history", "a", "--no-history"]),
            Err(ArgError::Repeated("--history"))
        );
        assert_eq!(
            parse(&["--reset-stats", "--no-history"]),
            Err(ArgError::Conflict("--reset-stats", "--no-history"))
        );
        assert_eq!(
            parse(&["--history"]),
            Err(ArgError::MissingValue("--history"))
        );
        assert!(parse(&["--history="]).is_err());
        assert!(USAGE.contains("--reset-stats") && USAGE.contains("--no-history"));
        assert!(!play(&[]).adapt, "adaptation is off by default");
        assert!(play(&["--adapt"]).adapt);
        assert_eq!(
            parse(&["--adapt", "--adapt"]),
            Err(ArgError::Repeated("--adapt"))
        );
        assert!(
            play(&["--adapt", "--seed", "1"])
                .replay_command(1, "x")
                .ends_with(" --adapt")
        );
    }

    #[test]
    fn help_and_version() {
        assert_eq!(parse(&["--help"]), Ok(Command::Help));
        assert_eq!(parse(&["--seed", "3", "-h"]), Ok(Command::Help));
        assert_eq!(parse(&["--version"]), Ok(Command::Version));
    }
}

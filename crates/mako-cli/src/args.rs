//! Command-line parsing (no external dependencies).

use std::fmt;

use mako_core::rules::StackDepth;

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

/// A fully parsed session configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    /// `None` means "derive from the clock" (done in `main`).
    pub seed: Option<u64>,
    pub hands: u64,
    pub depth: StackDepth,
    pub bot: BotKind,
    pub no_color: bool,
    pub auto: Option<AutoPolicy>,
    /// `--strategy`; `None` means `artifacts/smoke_<depth>bb.mako`.
    pub strategy: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            seed: None,
            hands: DEFAULT_HANDS,
            depth: StackDepth::Bb20,
            bot: BotKind::Auto,
            no_color: false,
            auto: None,
            strategy: None,
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
    InvalidValue { flag: &'static str, value: String },
    UnknownBot(String),
    UnknownAuto(String),
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
                    "--seed" => " (expected an unsigned 64-bit integer)",
                    _ => "",
                };
                write!(f, "invalid value '{value}' for {flag}{hint}")
            }
            ArgError::UnknownBot(name) => {
                write!(f, "unknown bot '{name}' (available: auto, cfr, baseline)")
            }
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
    --no-color            Disable ANSI colors (NO_COLOR is also honoured)
    -h, --help            Show this help
    -V, --version         Show the version

IN-HAND COMMANDS:
    action keys shown in the menu (f c k h p a) or menu numbers,
    help, stats, quit
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
    let mut it = args.iter().map(AsRef::as_ref);
    while let Some(arg) = it.next() {
        let (name, inline) = match arg.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n, Some(v)),
            _ => (arg, None),
        };
        match name {
            "-h" | "--help" => return Ok(Command::Help),
            "-V" | "--version" => return Ok(Command::Version),
            "--no-color" | "--no-colour" => opts.no_color = true,
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
        assert!(o.no_color);
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
    fn help_and_version() {
        assert_eq!(parse(&["--help"]), Ok(Command::Help));
        assert_eq!(parse(&["--seed", "3", "-h"]), Ok(Command::Help));
        assert_eq!(parse(&["--version"]), Ok(Command::Version));
    }
}

//! `mako` — heads-up no-limit hold'em in the terminal.

use std::io::{self, IsTerminal};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use mako_cli::args::{self, BotKind, Command};
use mako_cli::game::{self, AutoPlayer, HumanSeat, SessionConfig};
use mako_cli::render::Style;

fn clock_seed() -> u64 {
    let d = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    d.as_secs() ^ u64::from(d.subsec_nanos()).rotate_left(32)
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let opts = match args::parse(&argv) {
        Ok(Command::Help) => {
            print!("{}", args::USAGE);
            return ExitCode::SUCCESS;
        }
        Ok(Command::Version) => {
            println!("mako {}", game::crate_version());
            return ExitCode::SUCCESS;
        }
        Ok(Command::Play(o)) => o,
        Err(e) => {
            eprintln!("mako: error: {e}");
            return ExitCode::from(2);
        }
    };
    match opts.bot {
        BotKind::Baseline => {}
    }

    let no_color_env = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
    let style = Style {
        color: !opts.no_color && !no_color_env && io::stdout().is_terminal(),
    };
    let cfg = SessionConfig {
        seed: opts.seed.unwrap_or_else(clock_seed),
        hands: opts.hands,
        depth: opts.depth,
        style,
    };
    let human = match opts.auto {
        Some(p) => HumanSeat::Auto(AutoPlayer::new(p, cfg.seed)),
        None => HumanSeat::Lines(io::stdin().lock()),
    };
    let mut out = io::stdout().lock();
    match game::run_session(&cfg, human, &mut out) {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mako: error: {e}");
            ExitCode::FAILURE
        }
    }
}

//! `mako` — heads-up no-limit hold'em in the terminal.

use std::io::{self, IsTerminal};
use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mako_cli::args::{self, Command};
use mako_cli::game::{self, AutoPlayer, HumanSeat, SessionConfig};
use mako_cli::ui::{Ui, term};
use mako_cli::{history, opponent};

/// Pause before bot actions and new streets in interactive TTY play.
const PACE: Duration = Duration::from_millis(350);

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
    if opts.reset_stats {
        let (msg, ok) = history::reset(&opts.history, history::default_path);
        return if ok {
            println!("{msg}");
            ExitCode::SUCCESS
        } else {
            eprintln!("mako: error: {msg}");
            ExitCode::FAILURE
        };
    }
    let env = |k: &str| std::env::var(k).ok();
    let stdout_tty = io::stdout().is_terminal();
    let ui = Ui {
        color: term::detect_color(opts.color, &env, stdout_tty),
        glyphs: term::detect_glyphs(opts.glyphs, &env),
        width: term::detect_width(opts.width, &env),
    };
    let seed = opts.seed.unwrap_or_else(clock_seed);
    let mut opp = match opponent::select_with(
        opts.bot,
        opts.strategy.as_deref().map(Path::new),
        opts.depth,
        game::bot_seed(seed),
        opts.difficulty,
    ) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("mako: error: {e}");
            return ExitCode::from(2);
        }
    };
    if opts.adapt {
        if let Err(e) = opp.enable_adaptive(game::HUMAN, game::adapt_seed(seed)) {
            eprintln!("mako: error: {e}");
            return ExitCode::from(2);
        }
    }
    let (mut recorder, notices) = history::open(&opts.history, history::default_path);
    let interactive = opts.auto.is_none() && stdout_tty && io::stdin().is_terminal();
    let cfg = SessionConfig {
        seed,
        hands: opts.hands,
        depth: opts.depth,
        ui,
        replay: opts.replay_command(seed, &opp.strategy_path.display().to_string()),
        pace: if interactive && !opts.fast {
            PACE
        } else {
            Duration::ZERO
        },
        notices,
    };
    let human = match opts.auto {
        Some(p) => HumanSeat::Auto(AutoPlayer::new(p, cfg.seed)),
        None => HumanSeat::Lines(io::stdin().lock()),
    };
    let mut out = io::stdout().lock();
    match game::run_session_with_history(&cfg, human, &mut opp, &mut recorder, &mut out) {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) if e.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("mako: error: {e}");
            ExitCode::FAILURE
        }
    }
}

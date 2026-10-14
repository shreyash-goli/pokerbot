//! Bounded, cancellable, resumable training loop (M-11).
//!
//! [`train`] runs the [`Trainer`] until `config.iterations` total iterations
//! are reached, the `cancel` flag is set, or the optional wall-clock budget
//! runs out. Cancellation and the time limit are checked between
//! iterations. There is no unbounded mode: `iterations` is a required
//! constructor argument and is capped at [`MAX_ITERATIONS`].
//!
//! Checkpoints are emitted as [`TrainEvent::Checkpoint`] every
//! `checkpoint_every` iterations, and always once when the loop stops. The
//! caller decides where to put them (e.g. [`crate::checkpoint::save_atomic`]
//! on native). That keeps this module free of file I/O and wasm-safe.
//! Resuming from a checkpoint continues bit-identically (same table digest,
//! RNG state, counters) to an uninterrupted run.

use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};

use mako_core::rules::RulesConfig;

use crate::abstraction::postflop::AbstractionConfig;
use crate::checkpoint::{Checkpoint, CheckpointError};
use crate::regret::{UpdateConfig, Weighting};
use crate::traverse::{Trainer, TrainerConfig};

/// Hard ceiling on `TrainConfig::iterations`.
pub const MAX_ITERATIONS: u64 = 1 << 40;

/// Training-run configuration. There is no `Default`: the iteration target
/// must always be given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrainConfig {
    /// Game rules.
    pub rules: RulesConfig,
    /// Card abstraction.
    pub abstraction: AbstractionConfig,
    /// Update flags (RM+).
    pub update: UpdateConfig,
    /// Linear weighting flags.
    pub weighting: Weighting,
    /// Trainer seed.
    pub seed: u64,
    /// Total iteration target (including iterations already in a resumed
    /// checkpoint).
    pub iterations: u64,
    /// Emit a checkpoint every this many iterations.
    pub checkpoint_every: Option<u64>,
    /// Stop after this many wall-clock seconds (native only).
    pub max_wall_secs: Option<u64>,
    /// Equity cache capacity; does not affect results.
    pub cache_capacity: usize,
}

impl TrainConfig {
    /// Config with plain ES-MCCFR flags, no checkpoints, no time limit.
    pub fn new(
        rules: RulesConfig,
        abstraction: AbstractionConfig,
        seed: u64,
        iterations: u64,
    ) -> Self {
        Self {
            rules,
            abstraction,
            update: UpdateConfig::default(),
            weighting: Weighting::default(),
            seed,
            iterations,
            checkpoint_every: None,
            max_wall_secs: None,
            cache_capacity: 1 << 16,
        }
    }

    /// Same settings as [`TrainerConfig::tiny`] with an explicit target.
    pub fn tiny(seed: u64, iterations: u64) -> Self {
        let t = TrainerConfig::tiny(seed);
        Self::new(t.rules, t.abstraction, seed, iterations)
    }

    /// The trainer configuration this run uses.
    pub fn trainer_config(&self) -> TrainerConfig {
        TrainerConfig {
            rules: self.rules,
            abstraction: self.abstraction,
            seed: self.seed,
            cache_capacity: self.cache_capacity,
            update: self.update,
            weighting: self.weighting,
            verify_keys: false,
        }
    }
}

/// Why the loop stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    /// Reached `iterations`.
    Completed,
    /// `cancel` was set.
    Cancelled,
    /// `max_wall_secs` elapsed.
    WallClock,
}

/// Events delivered to the caller.
#[derive(Debug)]
pub enum TrainEvent<'a> {
    /// Progress after a checkpoint interval or at the end.
    Progress {
        /// Completed iterations.
        iteration: u64,
        /// Target.
        target: u64,
    },
    /// A checkpoint the caller may persist.
    Checkpoint(&'a Checkpoint),
}

/// Result of [`train`].
#[derive(Debug)]
pub struct TrainOutcome {
    /// The trainer at the stop point.
    pub trainer: Trainer,
    /// Why training stopped.
    pub reason: StopReason,
    /// Iterations run in this call.
    pub iterations_run: u64,
}

/// Training errors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrainError {
    /// Invalid configuration.
    Config(String),
    /// Checkpoint incompatible with the config.
    Checkpoint(CheckpointError),
}

impl fmt::Display for TrainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TrainError::Config(m) => write!(f, "invalid training config: {m}"),
            TrainError::Checkpoint(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for TrainError {}

struct Clock {
    #[cfg(not(target_arch = "wasm32"))]
    deadline: Option<std::time::Instant>,
}

impl Clock {
    fn new(max_wall_secs: Option<u64>) -> Result<Self, TrainError> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            Ok(Self {
                deadline: max_wall_secs
                    .map(|s| std::time::Instant::now() + std::time::Duration::from_secs(s)),
            })
        }
        #[cfg(target_arch = "wasm32")]
        {
            if max_wall_secs.is_some() {
                return Err(TrainError::Config(
                    "max_wall_secs is not supported on wasm32".into(),
                ));
            }
            Ok(Self {})
        }
    }

    fn expired(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.deadline
                .is_some_and(|d| std::time::Instant::now() >= d)
        }
        #[cfg(target_arch = "wasm32")]
        {
            false
        }
    }
}

/// Run (or resume) training. See the module docs.
pub fn train(
    config: &TrainConfig,
    resume: Option<Checkpoint>,
    cancel: &AtomicBool,
    on_event: &mut dyn FnMut(TrainEvent<'_>),
) -> Result<TrainOutcome, TrainError> {
    if config.iterations == 0 || config.iterations > MAX_ITERATIONS {
        return Err(TrainError::Config(format!(
            "iterations must be in 1..={MAX_ITERATIONS}"
        )));
    }
    if config.checkpoint_every == Some(0) {
        return Err(TrainError::Config(
            "checkpoint_every must be positive".into(),
        ));
    }
    let clock = Clock::new(config.max_wall_secs)?;
    let tc = config.trainer_config();
    let mut trainer = match resume {
        Some(cp) => cp.into_trainer(tc).map_err(TrainError::Checkpoint)?,
        None => Trainer::new(tc).map_err(|e| TrainError::Config(e.to_string()))?,
    };
    if trainer.iteration() > config.iterations {
        return Err(TrainError::Config(
            "checkpoint is already past the iteration target".into(),
        ));
    }
    let start = trainer.iteration();
    let reason = loop {
        if trainer.iteration() >= config.iterations {
            break StopReason::Completed;
        }
        if cancel.load(Ordering::Relaxed) {
            break StopReason::Cancelled;
        }
        if clock.expired() {
            break StopReason::WallClock;
        }
        trainer.run_iterations(1);
        if let Some(every) = config.checkpoint_every {
            let it = trainer.iteration();
            if it % every == 0 && it < config.iterations {
                on_event(TrainEvent::Progress {
                    iteration: it,
                    target: config.iterations,
                });
                on_event(TrainEvent::Checkpoint(&Checkpoint::from_trainer(&trainer)));
            }
        }
    };
    on_event(TrainEvent::Progress {
        iteration: trainer.iteration(),
        target: config.iterations,
    });
    on_event(TrainEvent::Checkpoint(&Checkpoint::from_trainer(&trainer)));
    let iterations_run = trainer.iteration() - start;
    Ok(TrainOutcome {
        trainer,
        reason,
        iterations_run,
    })
}

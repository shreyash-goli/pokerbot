//! The pure-Rust game session behind the browser API.
//!
//! [`GameSession`] owns the authoritative `HandState` and the bot agent.
//! Everything it returns to JavaScript is built by [`crate::encode`] from
//! `observation(HUMAN)` or `events_for(Viewer::Seat(HUMAN))`; the full
//! state is never serialized. The bot receives only `observation(BOT)`.
//!
//! The API takes no cards from the caller: the only inputs are a depth, a
//! seed, an optional policy artifact (validated by
//! `CfrAgent::from_artifact_bytes`), a bot kind, and action names checked
//! against the engine's legal list.

use std::fmt;

use pokerbot_cfr::artifact::ArtifactError;
use pokerbot_cfr::infer::{Agent, AgentError, AgentMetadata, BaselineAgent, CfrAgent, InferStats};
use pokerbot_core::action::{ActionKind, Seat};
use pokerbot_core::events::Viewer;
use pokerbot_core::observation::PlayerObservation;
use pokerbot_core::rules::{RulesConfig, StackDepth};
use pokerbot_core::state::{HandState, Match};

use crate::encode::{self, BotInfo, SessionInfo};
use crate::seeds::{bot_seed, hand_deck};

/// The human always sits in seat Zero (button on hand 1, then alternating).
pub const HUMAN: Seat = Seat::Zero;
/// The bot sits in seat One.
pub const BOT: Seat = Seat::One;

/// Upper bound on actions in one hand; exceeding it means a broken engine.
const MAX_ACTIONS_PER_HAND: u32 = 1000;

/// Which agent plays the bot seat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BotKind {
    /// A trained CFR policy artifact (required; never substituted).
    Cfr,
    /// The untrained random-legal baseline, requested explicitly.
    Baseline,
}

impl BotKind {
    /// Parses `"cfr"` or `"baseline"` (exact, lowercase).
    pub fn parse(s: &str) -> Result<BotKind, SessionError> {
        match s {
            "cfr" => Ok(BotKind::Cfr),
            "baseline" => Ok(BotKind::Baseline),
            _ => Err(SessionError::InvalidBot(s.to_string())),
        }
    }

    /// The wire name.
    pub const fn name(self) -> &'static str {
        match self {
            BotKind::Cfr => "cfr",
            BotKind::Baseline => "baseline",
        }
    }
}

/// Wire name of an action class.
pub const fn action_name(kind: ActionKind) -> &'static str {
    match kind {
        ActionKind::Fold => "fold",
        ActionKind::Check => "check",
        ActionKind::Call => "call",
        ActionKind::HalfPot => "half_pot",
        ActionKind::Pot => "pot",
        ActionKind::AllIn => "all_in",
    }
}

/// Parses an exact wire action name.
pub fn parse_action(s: &str) -> Option<ActionKind> {
    ActionKind::ALL.into_iter().find(|&k| action_name(k) == s)
}

/// Longest caller input echoed back in an error, in characters.
pub const MAX_ECHO_CHARS: usize = 64;

/// `s` cut to [`MAX_ECHO_CHARS`] characters (with `…` if cut), so errors
/// never echo arbitrarily large input.
pub fn echo_input(s: &str) -> String {
    let mut it = s.chars();
    let head: String = it.by_ref().take(MAX_ECHO_CHARS).collect();
    if it.next().is_some() {
        head + "…"
    } else {
        head
    }
}

/// Depth in big blinds → [`StackDepth`]; only 20, 50 and 100 exist.
pub fn parse_depth(bb: u32) -> Result<StackDepth, SessionError> {
    StackDepth::ALL
        .into_iter()
        .find(|d| d.big_blinds() == bb)
        .ok_or(SessionError::InvalidDepth(bb))
}

/// Every failure of the session API. No failure mutates game state, except
/// [`SessionError::Bot`]/[`SessionError::Engine`] during a bot turn, which
/// poison the session (later calls return [`SessionError::Poisoned`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionError {
    /// Depth other than 20/50/100 BB.
    InvalidDepth(u32),
    /// Bot kind other than `"cfr"` / `"baseline"`.
    InvalidBot(String),
    /// `"cfr"` was requested without policy bytes.
    PolicyMissing,
    /// The artifact was trained for other rules (depth or blinds).
    PolicyRulesMismatch(String),
    /// The artifact is corrupt, truncated, or otherwise invalid.
    PolicyInvalid(String),
    /// A seed that is not a `BigInt` in `0..2^64` (JS API only).
    InvalidSeed(String),
    /// Not one of the six wire action names (echo truncated, see
    /// [`echo_input`]).
    UnknownAction(String),
    /// A known action that is not legal now; `legal` lists what is.
    IllegalAction {
        action: String,
        legal: Vec<&'static str>,
    },
    /// The hand is over; call `next_hand`.
    HandOver,
    /// `next_hand` while the current hand is still being played.
    HandInProgress,
    /// It is not the human's turn (cannot happen through the public API).
    NotYourTurn,
    /// The bot agent failed (session poisoned).
    Bot(String),
    /// The engine rejected an action or hand setup (session poisoned).
    Engine(String),
    /// An earlier bot/engine failure left the session unusable.
    Poisoned,
}

impl SessionError {
    /// Stable machine-readable code (the prefix of the JS error message).
    pub const fn code(&self) -> &'static str {
        match self {
            SessionError::InvalidDepth(_) => "invalid_depth",
            SessionError::InvalidBot(_) => "invalid_bot",
            SessionError::PolicyMissing => "policy_missing",
            SessionError::PolicyRulesMismatch(_) => "policy_rules_mismatch",
            SessionError::PolicyInvalid(_) => "policy_invalid",
            SessionError::InvalidSeed(_) => "invalid_seed",
            SessionError::UnknownAction(_) => "unknown_action",
            SessionError::IllegalAction { .. } => "illegal_action",
            SessionError::HandOver => "hand_over",
            SessionError::HandInProgress => "hand_in_progress",
            SessionError::NotYourTurn => "not_your_turn",
            SessionError::Bot(_) => "bot_error",
            SessionError::Engine(_) => "engine_error",
            SessionError::Poisoned => "session_poisoned",
        }
    }
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: ", self.code())?;
        match self {
            SessionError::InvalidDepth(bb) => {
                write!(f, "depth {bb} BB is not supported (use 20, 50 or 100)")
            }
            SessionError::InvalidBot(s) => {
                write!(
                    f,
                    "bot {s:?} is not supported (use \"cfr\" or \"baseline\")"
                )
            }
            SessionError::PolicyMissing => write!(
                f,
                "bot \"cfr\" needs policy artifact bytes; none were given \
                 (pass a .pokerbot file, or choose bot \"baseline\" explicitly)"
            ),
            SessionError::PolicyRulesMismatch(e) => {
                write!(f, "policy artifact is for other rules: {e}")
            }
            SessionError::PolicyInvalid(e) => write!(f, "policy artifact rejected: {e}"),
            SessionError::InvalidSeed(s) => write!(
                f,
                "seed must be a BigInt in 0..2^64 (e.g. 7n or BigInt(7)); got {s}"
            ),
            SessionError::UnknownAction(s) => write!(
                f,
                "{s:?} is not an action (use fold, check, call, half_pot, pot, all_in)"
            ),
            SessionError::IllegalAction { action, legal } => {
                write!(f, "{action} is not legal now; legal: {}", legal.join(", "))
            }
            SessionError::HandOver => write!(f, "the hand is over; call nextHand()"),
            SessionError::HandInProgress => {
                write!(f, "the current hand is not finished")
            }
            SessionError::NotYourTurn => write!(f, "it is not the human player's turn"),
            SessionError::Bot(e) => write!(f, "bot failed: {e}"),
            SessionError::Engine(e) => write!(f, "engine failed: {e}"),
            SessionError::Poisoned => {
                write!(f, "an earlier internal error ended this session; reset it")
            }
        }
    }
}

impl std::error::Error for SessionError {}

fn load_cfr(bytes: &[u8], rules: &RulesConfig, seed: u64) -> Result<CfrAgent, SessionError> {
    CfrAgent::from_artifact_bytes(bytes, rules, seed).map_err(|e| match e {
        AgentError::Artifact(ArtifactError::RulesMismatch { .. }) => {
            SessionError::PolicyRulesMismatch(e.to_string())
        }
        _ => SessionError::PolicyInvalid(e.to_string()),
    })
}

/// The bot seat's agent.
enum BotAgent {
    Cfr(Box<CfrAgent>),
    Baseline(BaselineAgent),
}

impl BotAgent {
    fn agent(&mut self) -> &mut dyn Agent {
        match self {
            BotAgent::Cfr(a) => a.as_mut(),
            BotAgent::Baseline(a) => a,
        }
    }

    fn label(&self) -> String {
        match self {
            BotAgent::Cfr(a) => a.label(),
            BotAgent::Baseline(a) => a.label(),
        }
    }

    fn metadata(&self) -> Option<&AgentMetadata> {
        match self {
            BotAgent::Cfr(a) => Some(a.metadata()),
            BotAgent::Baseline(_) => None,
        }
    }

    fn stats(&self) -> InferStats {
        match self {
            BotAgent::Cfr(a) => a.stats(),
            BotAgent::Baseline(a) => a.infer_stats().unwrap_or_default(),
        }
    }
}

/// A human-vs-bot session: seeded hands, alternating button, running score.
pub struct GameSession {
    depth: StackDepth,
    rules: RulesConfig,
    seed: u64,
    kind: BotKind,
    /// Kept so `reset` can rebuild the agent with a fresh RNG.
    policy_bytes: Option<Vec<u8>>,
    bot: BotAgent,
    matchup: Match,
    hand: HandState,
    /// 0-based index of the current hand.
    hand_index: u64,
    /// Whether the current (terminal) hand was added to the score.
    recorded: bool,
    /// Bot counters as of the end of the last completed hand. Counters of
    /// the hand in progress are withheld: whether the bot's current spot is
    /// in the policy depends on its private cards.
    bot_stats_at_hand_end: InferStats,
    poisoned: bool,
}

impl fmt::Debug for GameSession {
    /// Deliberately opaque: the full state holds the bot's hole cards.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GameSession")
            .field("depth_bb", &self.depth.big_blinds())
            .field("hand_number", &(self.hand_index + 1))
            .finish_non_exhaustive()
    }
}

impl GameSession {
    /// Starts a session and deals hand 1 (the bot acts first if it must).
    /// `policy` is required for [`BotKind::Cfr`] and ignored for
    /// [`BotKind::Baseline`].
    pub fn new(
        depth_bb: u32,
        seed: u64,
        policy: Option<Vec<u8>>,
        bot: &str,
    ) -> Result<GameSession, SessionError> {
        let depth = parse_depth(depth_bb)?;
        let kind = BotKind::parse(bot)?;
        let rules = RulesConfig::standard(depth);
        let policy_bytes = match kind {
            BotKind::Cfr => Some(policy.ok_or(SessionError::PolicyMissing)?),
            BotKind::Baseline => None,
        };
        let agent = Self::make_bot(kind, policy_bytes.as_deref(), &rules, seed)?;
        Self::start(depth, rules, seed, kind, policy_bytes, agent)
    }

    fn make_bot(
        kind: BotKind,
        bytes: Option<&[u8]>,
        rules: &RulesConfig,
        seed: u64,
    ) -> Result<BotAgent, SessionError> {
        Ok(match kind {
            BotKind::Cfr => {
                let bytes = bytes.ok_or(SessionError::PolicyMissing)?;
                BotAgent::Cfr(Box::new(load_cfr(bytes, rules, bot_seed(seed))?))
            }
            BotKind::Baseline => BotAgent::Baseline(BaselineAgent::new(bot_seed(seed))),
        })
    }

    fn start(
        depth: StackDepth,
        rules: RulesConfig,
        seed: u64,
        kind: BotKind,
        policy_bytes: Option<Vec<u8>>,
        bot: BotAgent,
    ) -> Result<GameSession, SessionError> {
        let matchup = Match::new(rules, HUMAN).map_err(|e| SessionError::Engine(e.to_string()))?;
        let hand = matchup
            .start_hand(hand_deck(seed, 0))
            .map_err(|e| SessionError::Engine(e.to_string()))?;
        let mut s = GameSession {
            depth,
            rules,
            seed,
            kind,
            policy_bytes,
            bot,
            matchup,
            hand,
            hand_index: 0,
            recorded: false,
            bot_stats_at_hand_end: InferStats::default(),
            poisoned: false,
        };
        s.advance()?;
        Ok(s)
    }

    /// Restarts with a new seed (same depth, bot and policy): score, hand
    /// counter and bot RNG all reset, exactly like a fresh session.
    pub fn reset(&mut self, seed: u64) -> Result<(), SessionError> {
        let bot = Self::make_bot(self.kind, self.policy_bytes.as_deref(), &self.rules, seed)?;
        let fresh = Self::start(
            self.depth,
            self.rules,
            seed,
            self.kind,
            self.policy_bytes.clone(),
            bot,
        )?;
        *self = fresh;
        Ok(())
    }

    /// Applies the human's action (a wire name) and then plays bot turns
    /// until the human must act again or the hand ends. Any error before
    /// the human's action is applied leaves the session untouched.
    pub fn step(&mut self, action: &str) -> Result<(), SessionError> {
        if self.poisoned {
            return Err(SessionError::Poisoned);
        }
        if self.hand.is_terminal() {
            return Err(SessionError::HandOver);
        }
        if self.hand.to_act() != Some(HUMAN) {
            return Err(SessionError::NotYourTurn);
        }
        let kind =
            parse_action(action).ok_or_else(|| SessionError::UnknownAction(echo_input(action)))?;
        let obs = self.hand.observation(HUMAN);
        let legal = obs
            .legal_actions
            .iter()
            .find(|a| a.kind == kind)
            .copied()
            .ok_or_else(|| SessionError::IllegalAction {
                action: action.to_string(),
                legal: obs
                    .legal_actions
                    .iter()
                    .map(|a| action_name(a.kind))
                    .collect(),
            })?;
        if let Err(e) = self.hand.apply_legal(&legal) {
            // Unreachable for an action taken from the legal list.
            self.poisoned = true;
            return Err(SessionError::Engine(e.to_string()));
        }
        self.advance()
    }

    /// Deals the next hand (stacks reset to the configured depth, button
    /// moves). Fails with [`SessionError::HandInProgress`] mid-hand.
    pub fn next_hand(&mut self) -> Result<(), SessionError> {
        if self.poisoned {
            return Err(SessionError::Poisoned);
        }
        if !self.hand.is_terminal() {
            return Err(SessionError::HandInProgress);
        }
        let next = self.hand_index + 1;
        let hand = self
            .matchup
            .start_hand(hand_deck(self.seed, next))
            .map_err(|e| SessionError::Engine(e.to_string()))?;
        self.hand = hand;
        self.hand_index = next;
        self.recorded = false;
        self.advance()
    }

    /// Plays bot turns until the human acts or the hand ends, then scores a
    /// finished hand once.
    fn advance(&mut self) -> Result<(), SessionError> {
        let r = self.play_bot_turns();
        if r.is_err() {
            self.poisoned = true;
        }
        r?;
        if self.hand.is_terminal() && !self.recorded {
            self.recorded = self.matchup.record(&self.hand);
            self.bot_stats_at_hand_end = self.bot.stats();
        }
        Ok(())
    }

    fn play_bot_turns(&mut self) -> Result<(), SessionError> {
        let mut steps = 0u32;
        while self.hand.to_act() == Some(BOT) {
            steps += 1;
            if steps > MAX_ACTIONS_PER_HAND
                || self.hand.history().len() as u32 > MAX_ACTIONS_PER_HAND
            {
                return Err(SessionError::Engine(
                    "hand exceeded the action cap; engine invariant broken".into(),
                ));
            }
            // The bot sees only its own observation.
            let obs = self.hand.observation(BOT);
            let d = self
                .bot
                .agent()
                .act(&obs)
                .map_err(|e| SessionError::Bot(e.to_string()))?;
            let legal = *obs
                .legal_actions
                .iter()
                .find(|a| a.kind == d.action)
                .ok_or_else(|| SessionError::Bot("bot chose an illegal action".into()))?;
            self.hand
                .apply_legal(&legal)
                .map_err(|e| SessionError::Engine(e.to_string()))?;
        }
        Ok(())
    }

    /// The human seat's observation (the only view of the hand that leaves
    /// this type).
    pub fn observation(&self) -> PlayerObservation {
        self.hand.observation(HUMAN)
    }

    /// 1-based number of the current hand.
    pub fn hand_number(&self) -> u64 {
        self.hand_index + 1
    }

    /// Completed hands.
    pub fn hands_completed(&self) -> u64 {
        self.matchup.hands_played()
    }

    /// Net chips per seat over completed hands (zero-sum).
    pub fn score(&self) -> [i64; 2] {
        self.matchup.score()
    }

    /// The session seed.
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// The bot kind.
    pub fn bot_kind(&self) -> BotKind {
        self.kind
    }

    /// Whether the current hand is over.
    pub fn is_hand_over(&self) -> bool {
        self.hand.is_terminal()
    }

    fn bot_info(&self) -> BotInfo<'_> {
        BotInfo {
            kind: self.kind,
            label: self.bot.label(),
            metadata: self.bot.metadata(),
            stats_at_hand_end: self.bot_stats_at_hand_end,
        }
    }

    fn session_info(&self) -> SessionInfo {
        SessionInfo {
            hand_number: self.hand_number(),
            seed: self.seed,
            depth_bb: self.depth.big_blinds(),
            starting_stack: self.rules.starting_stack,
            hands_completed: self.hands_completed(),
            score: self.score(),
            poisoned: self.poisoned,
        }
    }

    /// `PublicState` JSON, built from `observation(HUMAN)` only.
    pub fn snapshot_json(&self) -> String {
        encode::public_state(&self.observation(), &self.session_info(), &self.bot_info()).encode()
    }

    /// The current hand's events as the human may see them, as JSON.
    pub fn history_json(&self) -> String {
        encode::events(&self.hand.events_for(Viewer::Seat(HUMAN)), HUMAN).encode()
    }

    /// Policy metadata JSON (no strategy contents).
    pub fn strategy_info_json(&self) -> String {
        encode::strategy_info(&self.bot_info()).encode()
    }
}

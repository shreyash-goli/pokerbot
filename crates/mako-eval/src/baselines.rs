//! Rule-based baseline agents. None of them is trained; all are labelled as
//! rule bots. Each decides from its own [`PlayerObservation`] only.
//!
//! * [`RandomLegal`]: uniform over the legal action classes.
//! * [`CheckCall`]: passive; checks when possible, otherwise calls. Never
//!   folds or raises (a call for less than the full amount is a call).
//! * [`TightRule`]: a simple fixed hand-strength rule (premium preflop
//!   hands raise, medium hands call, others check/fold; postflop it bets made
//!   hands of two pair or better and calls with a pair).
//! * [`AlwaysShove`] / [`AlwaysFold`]: degenerate agents for gate tests.

use mako_core::card::Rank;
use mako_core::eval::{HandCategory, evaluate};
use mako_core::observation::PlayerObservation;
use mako_core::rng::MakoRng;
use mako_core::rules::ActionKind;

use crate::agent::{Agent, AgentError, Decision, DecisionSource};

fn legal_kinds(obs: &PlayerObservation) -> Result<Vec<ActionKind>, AgentError> {
    if obs.terminal.is_some() {
        return Err(AgentError::Terminal);
    }
    if !obs.is_my_turn() {
        return Err(AgentError::NotToAct);
    }
    if obs.legal_actions.is_empty() {
        return Err(AgentError::NoLegalActions);
    }
    Ok(obs.legal_actions.iter().map(|a| a.kind).collect())
}

/// One-hot decision on the first legal kind of `preference`.
fn first_legal(obs: &PlayerObservation, preference: &[ActionKind]) -> Result<Decision, AgentError> {
    let legal = legal_kinds(obs)?;
    let kind = preference
        .iter()
        .copied()
        .find(|k| legal.contains(k))
        .ok_or(AgentError::NoLegalActions)?;
    let mut distribution = [0.0; 6];
    distribution[kind.index()] = 1.0;
    Ok(Decision {
        action: kind,
        distribution,
        source: DecisionSource::Baseline,
    })
}

/// Uniform over legal action classes.
#[derive(Clone, Debug)]
pub struct RandomLegal {
    rng: MakoRng,
}

impl RandomLegal {
    /// `seed` drives only this agent's own `action` draw; the harness
    /// samples from the distribution with its own stream.
    pub fn new(seed: u64) -> Self {
        Self {
            rng: MakoRng::seed_from_u64(seed),
        }
    }
}

impl Agent for RandomLegal {
    fn label(&self) -> String {
        "random-legal (rule bot)".into()
    }

    fn act(&mut self, obs: &PlayerObservation) -> Result<Decision, AgentError> {
        let legal = legal_kinds(obs)?;
        let p = 1.0 / legal.len() as f64;
        let mut distribution = [0.0; 6];
        for k in &legal {
            distribution[k.index()] = p;
        }
        let action = legal[self.rng.gen_range(legal.len() as u64) as usize];
        Ok(Decision {
            action,
            distribution,
            source: DecisionSource::Baseline,
        })
    }
}

/// Check if possible, else call. Never folds or raises.
#[derive(Clone, Copy, Debug, Default)]
pub struct CheckCall;

impl Agent for CheckCall {
    fn label(&self) -> String {
        "check-call (rule bot)".into()
    }

    fn act(&mut self, obs: &PlayerObservation) -> Result<Decision, AgentError> {
        first_legal(obs, &[ActionKind::Check, ActionKind::Call])
    }
}

/// Simple fixed hand-strength rule bot.
#[derive(Clone, Copy, Debug, Default)]
pub struct TightRule;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tier {
    Strong,
    Medium,
    Weak,
}

fn preflop_tier(obs: &PlayerObservation) -> Tier {
    let [a, b] = obs.hole_cards;
    let (hi, lo) = if a.rank() >= b.rank() {
        (a.rank(), b.rank())
    } else {
        (b.rank(), a.rank())
    };
    let pair = hi == lo;
    if (pair && hi >= Rank::Eight) || (hi == Rank::Ace && lo >= Rank::Jack) {
        Tier::Strong
    } else if pair || hi == Rank::Ace || lo >= Rank::Ten {
        Tier::Medium
    } else {
        Tier::Weak
    }
}

fn postflop_tier(obs: &PlayerObservation) -> Tier {
    let mut cards = obs.hole_cards.to_vec();
    cards.extend_from_slice(&obs.board);
    match evaluate(&cards).category() {
        c if c >= HandCategory::TwoPair => Tier::Strong,
        HandCategory::Pair => Tier::Medium,
        _ => Tier::Weak,
    }
}

impl Agent for TightRule {
    fn label(&self) -> String {
        "tight (rule bot)".into()
    }

    fn act(&mut self, obs: &PlayerObservation) -> Result<Decision, AgentError> {
        use ActionKind::*;
        let tier = if obs.board.len() >= 3 {
            postflop_tier(obs)
        } else {
            preflop_tier(obs)
        };
        match tier {
            Tier::Strong => first_legal(obs, &[Pot, HalfPot, AllIn, Call, Check]),
            Tier::Medium => first_legal(obs, &[Check, Call]),
            Tier::Weak => first_legal(obs, &[Check, Fold]),
        }
    }
}

/// Always goes all-in when it can (else calls/checks). Gate tests only.
#[derive(Clone, Copy, Debug, Default)]
pub struct AlwaysShove;

impl Agent for AlwaysShove {
    fn label(&self) -> String {
        "always-shove (test bot)".into()
    }

    fn act(&mut self, obs: &PlayerObservation) -> Result<Decision, AgentError> {
        use ActionKind::*;
        first_legal(obs, &[AllIn, Call, Check])
    }
}

/// Folds whenever folding is legal, else checks. Gate tests only.
#[derive(Clone, Copy, Debug, Default)]
pub struct AlwaysFold;

impl Agent for AlwaysFold {
    fn label(&self) -> String {
        "always-fold (test bot)".into()
    }

    fn act(&mut self, obs: &PlayerObservation) -> Result<Decision, AgentError> {
        use ActionKind::*;
        first_legal(obs, &[Fold, Check, Call])
    }
}

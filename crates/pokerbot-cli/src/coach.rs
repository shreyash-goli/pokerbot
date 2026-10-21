//! Bridges the trained policy into the post-hand coach.
//!
//! [`PolicyCoach`] implements [`pokerbot_analysis::coach::MixProvider`] with
//! [`CfrAgent::strategy`], which reads only the observation it is given
//! (the human's own) and neither samples nor counts. The coach uses its own
//! copy of the artifact, so reviewing a hand never disturbs the opponent's
//! RNG or decision counters.

use pokerbot_analysis::coach::{MixProvider, MixSource, PolicyMix};
use pokerbot_cfr::infer::{Agent, CfrAgent, DecisionSource, Difficulty};
use pokerbot_core::observation::PlayerObservation;

/// "What the Pokerbot policy would play in your seat." Always the FIXED
/// trained mix ([`CfrAgent::policy_strategy`]): never the difficulty blend
/// or the adaptive shift the opponent may be playing.
pub struct PolicyCoach<'a> {
    pub agent: &'a CfrAgent,
    /// Whether the opponent is adapting (`--adapt`).
    pub adapting: bool,
    /// The difficulty the opponent plays at.
    pub difficulty: Difficulty,
}

impl<'a> PolicyCoach<'a> {
    pub fn new(agent: &'a CfrAgent, adapting: bool, difficulty: Difficulty) -> Self {
        PolicyCoach {
            agent,
            adapting,
            difficulty,
        }
    }

    /// The honest label of the coach's mix.
    pub fn mix_label(&self) -> String {
        let mut head = if self.adapting {
            "fixed trained policy (not adapted, challenging)".to_string()
        } else {
            "trained policy (fixed, unadapted, challenging)".to_string()
        };
        if self.difficulty != Difficulty::Challenging {
            head.push_str(&format!(
                "; the opponent plays the {} blend instead",
                self.difficulty.name()
            ));
        }
        format!("{head} -- {}", self.agent.label())
    }
}

impl MixProvider for PolicyCoach<'_> {
    fn label(&self) -> String {
        self.mix_label()
    }

    fn checksum(&self) -> Option<u64> {
        Some(self.agent.metadata().checksum)
    }

    fn mix(&self, obs: &PlayerObservation) -> Result<PolicyMix, String> {
        let (probs, source) = self.agent.policy_strategy(obs).map_err(|e| e.to_string())?;
        Ok(PolicyMix {
            probs,
            source: match source {
                DecisionSource::Policy => MixSource::Policy,
                DecisionSource::Fallback(r) => MixSource::Fallback(r.to_string()),
                DecisionSource::Baseline => MixSource::Fallback("untrained baseline".into()),
            },
        })
    }
}

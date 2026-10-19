//! Bridges the trained policy into the post-hand coach (M-18).
//!
//! [`PolicyCoach`] implements [`mako_analysis::coach::MixProvider`] with
//! [`CfrAgent::strategy`], which reads only the observation it is given
//! (the human's own) and neither samples nor counts. The coach uses its own
//! copy of the artifact, so reviewing a hand never disturbs the opponent's
//! RNG or decision counters.

use mako_analysis::coach::{MixProvider, MixSource, PolicyMix};
use mako_cfr::infer::{Agent, CfrAgent, DecisionSource};
use mako_core::observation::PlayerObservation;

/// "What the Mako policy would play in your seat."
pub struct PolicyCoach<'a>(pub &'a CfrAgent);

impl MixProvider for PolicyCoach<'_> {
    fn label(&self) -> String {
        self.0.label()
    }

    fn checksum(&self) -> Option<u64> {
        Some(self.0.metadata().checksum)
    }

    fn mix(&self, obs: &PlayerObservation) -> Result<PolicyMix, String> {
        let (probs, source) = self.0.strategy(obs).map_err(|e| e.to_string())?;
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

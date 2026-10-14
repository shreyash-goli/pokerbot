//! Bots for the opponent seat.
//!
//! A [`Bot`] decides from a [`PlayerObservation`] only — the strict
//! hidden-information boundary. It never receives the `HandState`, so it
//! cannot see the other seat's hole cards or the undealt board. The trained
//! CFR policy (M-14) will implement the same trait.

use mako_core::action::{ActionKind, LegalAction};
use mako_core::observation::PlayerObservation;
use mako_core::rng::MakoRng;

/// Label shown wherever the baseline bot is mentioned.
pub const BASELINE_LABEL: &str = "baseline (untrained, random legal)";

/// An opponent policy.
pub trait Bot {
    /// Human-readable label, displayed to the player.
    fn label(&self) -> &str;

    /// Chooses one of `obs.legal_actions`. Returns `None` only if the list is
    /// empty (i.e. it is not this seat's turn).
    fn choose(&mut self, obs: &PlayerObservation) -> Option<LegalAction>;
}

/// Seeded weighted-random choice over the legal actions. Not trained and
/// not strategic in any sense: it exists so the game is playable before a
/// CFR policy exists.
#[derive(Clone, Debug)]
pub struct BaselineBot {
    rng: MakoRng,
}

impl BaselineBot {
    /// A bot whose decisions are fully determined by `seed`.
    pub fn new(seed: u64) -> BaselineBot {
        BaselineBot {
            rng: MakoRng::seed_from_u64(seed),
        }
    }

    /// Integer selection weight per action class. Passive actions are
    /// favoured so hands do not end almost immediately.
    pub const fn weight(kind: ActionKind) -> u64 {
        match kind {
            ActionKind::Fold => 2,
            ActionKind::Check => 6,
            ActionKind::Call => 5,
            ActionKind::HalfPot => 3,
            ActionKind::Pot => 2,
            ActionKind::AllIn => 1,
        }
    }
}

impl Bot for BaselineBot {
    fn label(&self) -> &str {
        BASELINE_LABEL
    }

    fn choose(&mut self, obs: &PlayerObservation) -> Option<LegalAction> {
        let legal = &obs.legal_actions;
        let total: u64 = legal.iter().map(|a| Self::weight(a.kind)).sum();
        if total == 0 {
            return None;
        }
        let mut pick = self.rng.gen_range(total);
        for a in legal {
            let w = Self::weight(a.kind);
            if pick < w {
                return Some(*a);
            }
            pick -= w;
        }
        unreachable!("pick < total by construction")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mako_core::action::Seat;
    use mako_core::rules::{RulesConfig, StackDepth};
    use mako_core::state::{HandConfig, HandState};

    #[test]
    fn only_legal_actions_over_many_seeded_hands() {
        let mut bots = [BaselineBot::new(11), BaselineBot::new(12)];
        let mut decisions = 0u64;
        let mut kinds_seen = [false; 6];
        for (i, depth) in (0..1500u64).zip(StackDepth::ALL.iter().cycle()) {
            let config = HandConfig {
                rules: RulesConfig::standard(*depth),
                button: if i % 2 == 0 { Seat::Zero } else { Seat::One },
            };
            let mut hand = HandState::from_seed(config, 1000 + i).unwrap();
            let mut steps = 0;
            while let Some(seat) = hand.to_act() {
                let obs = hand.observation(seat);
                let action = bots[seat.index()]
                    .choose(&obs)
                    .expect("legal list non-empty");
                assert!(obs.legal_actions.contains(&action));
                assert_eq!(hand.legal_actions(), obs.legal_actions);
                hand.apply_legal(&action)
                    .expect("baseline chose a legal action");
                kinds_seen[action.kind.index()] = true;
                decisions += 1;
                steps += 1;
                assert!(steps < 200, "hand did not terminate");
            }
            let p = hand.payoffs().unwrap();
            assert_eq!(p[0] + p[1], 0);
        }
        assert!(decisions > 3000);
        assert!(
            kinds_seen.iter().all(|&s| s),
            "every class sampled at least once"
        );
    }

    #[test]
    fn same_seed_same_choices() {
        let config = HandConfig {
            rules: RulesConfig::standard(StackDepth::Bb20),
            button: Seat::Zero,
        };
        let hand = HandState::from_seed(config, 5).unwrap();
        let obs = hand.observation(hand.to_act().unwrap());
        let mut a = BaselineBot::new(99);
        let mut b = BaselineBot::new(99);
        for _ in 0..100 {
            assert_eq!(a.choose(&obs), b.choose(&obs));
        }
    }

    #[test]
    fn no_choice_when_not_to_act() {
        let config = HandConfig {
            rules: RulesConfig::standard(StackDepth::Bb20),
            button: Seat::Zero,
        };
        let hand = HandState::from_seed(config, 5).unwrap();
        let waiting = hand.to_act().unwrap().other();
        assert_eq!(BaselineBot::new(1).choose(&hand.observation(waiting)), None);
    }

    #[test]
    fn label_says_untrained() {
        assert!(BaselineBot::new(0).label().contains("untrained"));
    }
}

//! Vanilla CFR and external-sampling MCCFR on Kuhn poker.
//!
//! - [`VanillaCfr`]: Zinkevich et al. (2007). Each iteration fixes the current
//!   regret-matching profile, enumerates all chance outcomes and actions, and
//!   updates both players simultaneously with counterfactual regrets weighted
//!   by opponent (and chance) reach. Fully deterministic.
//! - [`ExternalSamplingMccfr`]: Lanctot et al. (2009). Each iteration runs one
//!   traversal per player: chance and opponent actions are sampled, all
//!   traverser actions are enumerated, and sampled counterfactual regrets are
//!   added at traverser nodes. The average strategy is accumulated at
//!   opponent nodes ("simple" averaging from the paper). Deterministic for a
//!   fixed seed via [`SplitMix64`].
//!
//! Both support [`AverageWeighting::Linear`], which weights the iteration-`t`
//! contribution to the average strategy by `t` (Brown & Sandholm, 2019).

use super::eval::KuhnStrategy;
use super::kuhn::{Action, Card, Deal, History, NUM_ACTIONS, NUM_INFOSETS, info_key};
use super::regret::RegretNode;
use super::rng::SplitMix64;

/// How iterations are weighted in the average strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AverageWeighting {
    /// Every iteration weighted equally (classic CFR).
    #[default]
    Uniform,
    /// Iteration `t` weighted by `t` (linear averaging).
    Linear,
}

impl AverageWeighting {
    fn weight(self, t: u64) -> f64 {
        match self {
            AverageWeighting::Uniform => 1.0,
            AverageWeighting::Linear => t as f64,
        }
    }
}

fn new_nodes() -> Vec<RegretNode> {
    (0..NUM_INFOSETS)
        .map(|_| RegretNode::new(NUM_ACTIONS))
        .collect()
}

fn average_of(nodes: &[RegretNode]) -> KuhnStrategy {
    let mut s = KuhnStrategy::uniform();
    for (i, n) in nodes.iter().enumerate() {
        n.average_strategy(&mut s.probs[i]);
    }
    s
}

fn current_of(nodes: &[RegretNode]) -> KuhnStrategy {
    let mut s = KuhnStrategy::uniform();
    for (i, n) in nodes.iter().enumerate() {
        n.current_strategy(&mut s.probs[i]);
    }
    s
}

/// Vanilla tabular CFR with full chance enumeration.
#[derive(Debug, Clone)]
pub struct VanillaCfr {
    nodes: Vec<RegretNode>,
    iterations: u64,
    weighting: AverageWeighting,
}

impl VanillaCfr {
    /// New solver with zeroed tables.
    pub fn new(weighting: AverageWeighting) -> Self {
        Self {
            nodes: new_nodes(),
            iterations: 0,
            weighting,
        }
    }

    /// Completed iterations.
    pub fn iterations(&self) -> u64 {
        self.iterations
    }

    /// Per-information-set accumulators (indexed by `InfoSetId`).
    pub fn nodes(&self) -> &[RegretNode] {
        &self.nodes
    }

    /// Run `n` further iterations.
    pub fn run(&mut self, n: u64) {
        for _ in 0..n {
            self.iterate();
        }
    }

    /// One simultaneous-update CFR iteration.
    pub fn iterate(&mut self) {
        self.iterations += 1;
        let current = current_of(&self.nodes);
        let w = self.weighting.weight(self.iterations);
        let chance = 1.0 / Deal::all().len() as f64;
        for deal in Deal::all() {
            self.traverse(deal, History::root(), [1.0, 1.0], chance, &current, w);
        }
    }

    /// Returns utility for player 0.
    fn traverse(
        &mut self,
        deal: Deal,
        h: History,
        reach: [f64; 2],
        chance: f64,
        current: &KuhnStrategy,
        w: f64,
    ) -> f64 {
        if h.is_terminal() {
            return f64::from(h.terminal_utility_p0(deal.cards));
        }
        let p = h.player_to_act();
        let id = info_key(&deal.observation(p, h));
        let sigma = current.get(id);
        let mut child_u = [0.0; NUM_ACTIONS];
        let mut node_u = 0.0;
        for a in Action::ALL {
            let mut r = reach;
            r[p] *= sigma[a.index()];
            let u = self.traverse(deal, h.child(a), r, chance, current, w);
            child_u[a.index()] = u;
            node_u += sigma[a.index()] * u;
        }
        let sign = if p == 0 { 1.0 } else { -1.0 };
        let cf_reach = chance * reach[1 - p];
        let mut regrets = [0.0; NUM_ACTIONS];
        for a in 0..NUM_ACTIONS {
            regrets[a] = cf_reach * sign * (child_u[a] - node_u);
        }
        let node = &mut self.nodes[id.0];
        node.add_regrets(&regrets);
        node.accumulate_strategy(&sigma, w * chance * reach[p]);
        node_u
    }

    /// Normalized average strategy (the CFR output that converges).
    pub fn average_strategy(&self) -> KuhnStrategy {
        average_of(&self.nodes)
    }

    /// Current regret-matching strategy.
    pub fn current_strategy(&self) -> KuhnStrategy {
        current_of(&self.nodes)
    }
}

/// External-sampling Monte Carlo CFR.
#[derive(Debug, Clone)]
pub struct ExternalSamplingMccfr {
    nodes: Vec<RegretNode>,
    iterations: u64,
    weighting: AverageWeighting,
    rng: SplitMix64,
}

impl ExternalSamplingMccfr {
    /// New solver seeded with `seed`.
    pub fn new(seed: u64, weighting: AverageWeighting) -> Self {
        Self {
            nodes: new_nodes(),
            iterations: 0,
            weighting,
            rng: SplitMix64::new(seed),
        }
    }

    /// Completed iterations.
    pub fn iterations(&self) -> u64 {
        self.iterations
    }

    /// Per-information-set accumulators (indexed by `InfoSetId`).
    pub fn nodes(&self) -> &[RegretNode] {
        &self.nodes
    }

    /// Run `n` further iterations.
    pub fn run(&mut self, n: u64) {
        for _ in 0..n {
            self.iterate();
        }
    }

    fn sample_deal(&mut self) -> Deal {
        let c0 = Card::ALL[self.rng.below(3) as usize];
        let others: Vec<Card> = Card::ALL.into_iter().filter(|&c| c != c0).collect();
        let c1 = others[self.rng.below(2) as usize];
        Deal { cards: [c0, c1] }
    }

    /// One iteration: a sampled traversal for each player in turn.
    pub fn iterate(&mut self) {
        self.iterations += 1;
        for traverser in 0..2 {
            let deal = self.sample_deal();
            self.traverse(deal, History::root(), traverser);
        }
    }

    /// Returns sampled utility for `traverser`.
    fn traverse(&mut self, deal: Deal, h: History, traverser: usize) -> f64 {
        if h.is_terminal() {
            let u0 = f64::from(h.terminal_utility_p0(deal.cards));
            return if traverser == 0 { u0 } else { -u0 };
        }
        let p = h.player_to_act();
        // Key from the acting player's observation only.
        let id = info_key(&deal.observation(p, h));
        let mut sigma = [0.0; NUM_ACTIONS];
        self.nodes[id.0].current_strategy(&mut sigma);
        if p == traverser {
            let mut child_u = [0.0; NUM_ACTIONS];
            let mut node_u = 0.0;
            for a in Action::ALL {
                let u = self.traverse(deal, h.child(a), traverser);
                child_u[a.index()] = u;
                node_u += sigma[a.index()] * u;
            }
            let mut regrets = [0.0; NUM_ACTIONS];
            for a in 0..NUM_ACTIONS {
                regrets[a] = child_u[a] - node_u;
            }
            self.nodes[id.0].add_regrets(&regrets);
            node_u
        } else {
            let w = self.weighting.weight(self.iterations);
            self.nodes[id.0].accumulate_strategy(&sigma, w);
            let a = Action::ALL[self.rng.sample_index(&sigma)];
            self.traverse(deal, h.child(a), traverser)
        }
    }

    /// Normalized average strategy.
    pub fn average_strategy(&self) -> KuhnStrategy {
        average_of(&self.nodes)
    }

    /// Current regret-matching strategy.
    pub fn current_strategy(&self) -> KuhnStrategy {
        current_of(&self.nodes)
    }
}

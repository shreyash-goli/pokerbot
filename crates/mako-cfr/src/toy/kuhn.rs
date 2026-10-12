//! Kuhn poker rules (Kuhn, 1950).
//!
//! Three cards J < Q < K, two players, each antes 1 chip, each is dealt one
//! private card. Player 0 acts first. Actions are `Pass` (check, or fold when
//! facing a bet) and `Bet` (bet 1, or call when facing a bet).
//!
//! Histories (`p` = pass, `b` = bet):
//! - non-terminal: `""` (P0), `"p"` (P1), `"b"` (P1), `"pb"` (P0)
//! - terminal: `"pp"`, `"bp"`, `"bb"`, `"pbp"`, `"pbb"`
//!
//! Chip amounts are integers; utilities are zero-sum (`u1 = -u0`).
//!
//! # Information boundary
//! An information-set key is a function of a [`KuhnObservation`] only (the
//! acting player's own card and the public betting history). The opponent's
//! card is never an input to [`info_key`].

use std::fmt;

/// A Kuhn card. Ordering is rank order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Card {
    /// Jack (lowest).
    J = 0,
    /// Queen.
    Q = 1,
    /// King (highest).
    K = 2,
}

impl Card {
    /// All cards in rank order.
    pub const ALL: [Card; 3] = [Card::J, Card::Q, Card::K];

    /// Rank index 0..3.
    pub fn index(self) -> usize {
        self as usize
    }

    /// Single-character label.
    pub fn label(self) -> char {
        match self {
            Card::J => 'J',
            Card::Q => 'Q',
            Card::K => 'K',
        }
    }
}

/// A Kuhn action. Index 0 = pass (check/fold), index 1 = bet (bet/call).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    /// Check, or fold when facing a bet.
    Pass = 0,
    /// Bet 1, or call when facing a bet.
    Bet = 1,
}

impl Action {
    /// Both actions, indexed by [`Action::index`]. Every non-terminal Kuhn
    /// node has exactly these two legal actions.
    pub const ALL: [Action; 2] = [Action::Pass, Action::Bet];

    /// Action index (0 or 1).
    pub fn index(self) -> usize {
        self as usize
    }

    fn label(self) -> char {
        match self {
            Action::Pass => 'p',
            Action::Bet => 'b',
        }
    }
}

/// Number of legal actions at each decision node.
pub const NUM_ACTIONS: usize = 2;
/// Number of non-terminal public histories.
pub const NUM_HISTORIES: usize = 4;
/// Number of information sets (3 cards × 4 decision histories).
pub const NUM_INFOSETS: usize = 3 * NUM_HISTORIES;
/// Number of equally likely deals (ordered pairs of distinct cards).
pub const NUM_DEALS: usize = 6;

/// Public betting history (at most 3 actions).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct History {
    actions: [Option<Action>; 3],
    len: usize,
}

impl History {
    /// The empty (root) history.
    pub fn root() -> Self {
        Self::default()
    }

    /// Build a history from a string of `p`/`b`. Panics on invalid input.
    pub fn parse(s: &str) -> Self {
        let mut h = Self::root();
        for c in s.chars() {
            let a = match c {
                'p' => Action::Pass,
                'b' => Action::Bet,
                _ => panic!("invalid Kuhn action char {c:?}"),
            };
            h = h.child(a);
        }
        h
    }

    /// Number of actions taken.
    pub fn len(&self) -> usize {
        self.len
    }

    /// True for the root history.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    fn get(&self, i: usize) -> Action {
        self.actions[i].expect("index within len")
    }

    /// Append an action. Panics if the history is terminal.
    pub fn child(&self, a: Action) -> Self {
        assert!(!self.is_terminal(), "cannot act at terminal history {self}");
        let mut h = *self;
        h.actions[h.len] = Some(a);
        h.len += 1;
        h
    }

    /// Whether the hand is over.
    pub fn is_terminal(&self) -> bool {
        use Action::*;
        match self.len {
            0 | 1 => false,
            2 => !(self.get(0) == Pass && self.get(1) == Bet),
            _ => true,
        }
    }

    /// Player to act at a non-terminal history (0 or 1).
    pub fn player_to_act(&self) -> usize {
        debug_assert!(!self.is_terminal());
        self.len % 2
    }

    /// Index of a non-terminal history: `""`=0, `"p"`=1, `"b"`=2, `"pb"`=3.
    pub fn decision_index(&self) -> usize {
        use Action::*;
        assert!(
            !self.is_terminal(),
            "terminal history has no decision index"
        );
        match self.len {
            0 => 0,
            1 => match self.get(0) {
                Pass => 1,
                Bet => 2,
            },
            _ => 3,
        }
    }

    /// Terminal utility for player 0 (integer chips), given both cards.
    ///
    /// Pot contributions: ante 1 each, plus 1 for each bet/call.
    pub fn terminal_utility_p0(&self, cards: [Card; 2]) -> i32 {
        use Action::*;
        assert!(
            self.is_terminal(),
            "utility requested at non-terminal {self}"
        );
        let showdown = |stake: i32| {
            if cards[0] > cards[1] { stake } else { -stake }
        };
        match (self.len, self.get(0), self.get(1)) {
            (2, Pass, Pass) => showdown(1),
            (2, Bet, Pass) => 1, // P1 folds to bet
            (2, Bet, Bet) => showdown(2),
            (3, Pass, Bet) => match self.get(2) {
                Pass => -1, // P0 folds to bet
                Bet => showdown(2),
            },
            _ => unreachable!("not a terminal Kuhn history"),
        }
    }
}

impl fmt::Display for History {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for i in 0..self.len {
            write!(f, "{}", self.get(i).label())?;
        }
        Ok(())
    }
}

/// A full deal: `cards[p]` is player `p`'s private card. Only the game
/// engine / chance enumeration sees both entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Deal {
    /// Private cards indexed by player.
    pub cards: [Card; 2],
}

impl Deal {
    /// All six equally likely deals.
    pub fn all() -> [Deal; NUM_DEALS] {
        use Card::*;
        [
            Deal { cards: [J, Q] },
            Deal { cards: [J, K] },
            Deal { cards: [Q, J] },
            Deal { cards: [Q, K] },
            Deal { cards: [K, J] },
            Deal { cards: [K, Q] },
        ]
    }

    /// The observation available to `player` at `history`. Excludes the
    /// opponent's card.
    pub fn observation(&self, player: usize, history: History) -> KuhnObservation {
        KuhnObservation {
            own_card: self.cards[player],
            history,
        }
    }
}

/// What the acting player legally knows: own card and public history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KuhnObservation {
    /// The acting player's private card.
    pub own_card: Card,
    /// Public betting history.
    pub history: History,
}

/// Compact information-set identifier in `0..NUM_INFOSETS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct InfoSetId(pub usize);

impl InfoSetId {
    /// Construct from card and decision-history index.
    pub fn from_parts(card: Card, history_index: usize) -> Self {
        assert!(history_index < NUM_HISTORIES);
        InfoSetId(card.index() * NUM_HISTORIES + history_index)
    }

    /// All information sets in index order.
    pub fn all() -> impl Iterator<Item = InfoSetId> {
        (0..NUM_INFOSETS).map(InfoSetId)
    }

    /// Card component.
    pub fn card(self) -> Card {
        Card::ALL[self.0 / NUM_HISTORIES]
    }

    /// Public history component.
    pub fn history(self) -> History {
        History::parse(["", "p", "b", "pb"][self.0 % NUM_HISTORIES])
    }

    /// Player who acts at this information set.
    pub fn player(self) -> usize {
        self.history().player_to_act()
    }
}

impl fmt::Display for InfoSetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.card().label(), self.history())
    }
}

/// Information-set key from an observation (own card + public history only).
///
/// The signature deliberately cannot accept the opponent's card.
pub fn info_key(obs: &KuhnObservation) -> InfoSetId {
    InfoSetId::from_parts(obs.own_card, obs.history.decision_index())
}

/// Human-readable string key, e.g. `"Q:pb"`.
pub fn info_key_string(obs: &KuhnObservation) -> String {
    info_key(obs).to_string()
}

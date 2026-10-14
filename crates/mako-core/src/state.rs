//! The authoritative heads-up hand state machine.
//!
//! [`HandState`] owns the full (hidden-information) state of one hand: both
//! players' hole cards, the pre-determined board, stacks, contributions and
//! history. All transitions go through [`HandState::apply`] /
//! [`HandState::apply_legal`], which only accept actions present in
//! [`HandState::legal_actions`]; a rejected action leaves the state
//! unchanged. Player-facing code must use [`HandState::observation`]
//! (see [`crate::observation`]), never the full-state accessors.
//!
//! Betting follows `docs/GAME_RULES.md`; legality and sizing come from
//! [`crate::rules`], hand strength from [`crate::eval`].

use crate::card::Card;
use crate::deck::Deck;
use crate::eval::{HandRank, best_five, evaluate7};
use crate::rules::{
    self, BettingSnapshot, Chips, HandResult, MAX_STARTING_STACK, Position, RulesConfig,
};

pub use crate::action::{
    ActionError, ActionKind, ActionOutcome, ActionRecord, HandError, LegalAction, Seat, Street,
    TerminalReason, next_button,
};

/// Configuration for one hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HandConfig {
    pub rules: RulesConfig,
    /// Seat holding the button (= small blind).
    pub button: Seat,
}

impl HandConfig {
    /// Config for the next hand: same rules, button moved to the other seat.
    pub const fn next_hand(&self) -> HandConfig {
        HandConfig {
            rules: self.rules,
            button: next_button(self.button),
        }
    }
}

/// A player's showdown result (only produced for hands that reached
/// showdown).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShowdownHand {
    pub seat: Seat,
    pub hole_cards: [Card; 2],
    pub best_five: [Card; 5],
    pub rank: HandRank,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Finished {
    reason: TerminalReason,
    /// Net chip change per seat.
    payoffs: [i64; 2],
    /// Hand ranks per seat (showdown only).
    ranks: Option<[HandRank; 2]>,
}

/// Full state of one hand. Cheap to clone (fixed arrays plus a short
/// history vector); contains no RNG.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HandState {
    config: HandConfig,
    initial_stacks: [Chips; 2],
    /// Hole cards per seat — hidden information.
    hole: [[Card; 2]; 2],
    /// The whole board, fixed at deal time; only `revealed` cards are public.
    board: [Card; 5],
    revealed: u8,
    street: Street,
    stacks: [Chips; 2],
    street_c: [Chips; 2],
    total_c: [Chips; 2],
    last_full_raise: Chips,
    can_raise: [bool; 2],
    acted: [bool; 2],
    to_act: Option<Seat>,
    history: Vec<ActionRecord>,
    finished: Option<Finished>,
}

impl HandState {
    /// Starts a hand: validates the config, posts blinds, and deals the next
    /// nine cards of `deck` as BB, Button, BB, Button, then the five board
    /// cards (revealed street by street).
    pub fn new(config: HandConfig, deck: Deck) -> Result<HandState, HandError> {
        let s = config.rules.starting_stack;
        HandState::new_with_stacks(config, deck, [s, s])
    }

    /// [`HandState::new`] with a fresh deck shuffled from `seed`.
    pub fn from_seed(config: HandConfig, seed: u64) -> Result<HandState, HandError> {
        HandState::new(config, Deck::from_seed(seed))
    }

    /// Like [`HandState::new`] but with explicit per-seat starting stacks
    /// (indexed by [`Seat::index`]). Normal play always uses equal stacks
    /// (the configured depth); unequal stacks exist for tests and what-if
    /// analysis, and make call-for-less/refund paths reachable.
    pub fn new_with_stacks(
        config: HandConfig,
        mut deck: Deck,
        stacks: [Chips; 2],
    ) -> Result<HandState, HandError> {
        config.rules.validate().map_err(HandError::Config)?;
        for &st in &stacks {
            if st == 0 || st > MAX_STARTING_STACK {
                return Err(HandError::InvalidStack(st));
            }
        }
        let remaining = deck.remaining();
        let d = deck
            .deal_n(9)
            .ok_or(HandError::DeckTooSmall { remaining })?;
        let btn = config.button;
        let bb = btn.other();
        let mut hole = [[d[0]; 2]; 2];
        hole[bb.index()] = [d[0], d[2]];
        hole[btn.index()] = [d[1], d[3]];
        let board = [d[4], d[5], d[6], d[7], d[8]];

        let posted = rules::post_blinds(&config.rules, stacks[btn.index()], stacks[bb.index()]);
        let mut street_c = [0; 2];
        street_c[btn.index()] = posted.button_posted;
        street_c[bb.index()] = posted.big_blind_posted;
        let mut st = HandState {
            config,
            initial_stacks: stacks,
            hole,
            board,
            revealed: 0,
            street: Street::Preflop,
            stacks: [stacks[0] - street_c[0], stacks[1] - street_c[1]],
            street_c,
            total_c: street_c,
            last_full_raise: config.rules.big_blind,
            can_raise: [true; 2],
            acted: [false; 2],
            to_act: None,
            history: Vec::new(),
            finished: None,
        };
        st.begin_round(btn);
        Ok(st)
    }

    // ------------------------------------------------------------ queries

    /// Hand configuration.
    pub fn config(&self) -> &HandConfig {
        &self.config
    }

    /// Seat holding the button.
    pub fn button(&self) -> Seat {
        self.config.button
    }

    /// Seat posting the big blind.
    pub fn big_blind(&self) -> Seat {
        self.config.button.other()
    }

    /// Current street; `Showdown` for every terminal state.
    pub fn street(&self) -> Street {
        self.street
    }

    /// Whether the hand is over.
    pub fn is_terminal(&self) -> bool {
        self.finished.is_some()
    }

    /// How the hand ended, if it has.
    pub fn terminal_reason(&self) -> Option<TerminalReason> {
        self.finished.map(|f| f.reason)
    }

    /// Seat to act, or `None` when terminal.
    pub fn to_act(&self) -> Option<Seat> {
        self.to_act
    }

    /// Chips in the pot (0 after settlement).
    pub fn pot(&self) -> Chips {
        if self.finished.is_some() {
            0
        } else {
            self.total_c[0] + self.total_c[1]
        }
    }

    /// Chips behind for `seat` (the final stack once terminal).
    pub fn stack(&self, seat: Seat) -> Chips {
        self.stacks[seat.index()]
    }

    /// Starting stack of `seat` for this hand.
    pub fn starting_stack(&self, seat: Seat) -> Chips {
        self.initial_stacks[seat.index()]
    }

    /// `seat`'s contribution on the current street.
    pub fn street_contribution(&self, seat: Seat) -> Chips {
        self.street_c[seat.index()]
    }

    /// `seat`'s total contribution this hand (before any refund).
    pub fn total_contribution(&self, seat: Seat) -> Chips {
        self.total_c[seat.index()]
    }

    /// Size of the last full bet/raise increment on the current street.
    pub fn last_full_raise(&self) -> Chips {
        self.last_full_raise
    }

    /// Revealed board cards only (0, 3, 4 or 5).
    pub fn board(&self) -> &[Card] {
        &self.board[..usize::from(self.revealed)]
    }

    /// **Full-state accessor — hidden information.** Hole cards of `seat`.
    /// Not for player-facing code; use [`HandState::observation`].
    pub fn hole_cards(&self, seat: Seat) -> [Card; 2] {
        self.hole[seat.index()]
    }

    /// Public action history (voluntary actions only; blinds are implied).
    pub fn history(&self) -> &[ActionRecord] {
        &self.history
    }

    /// Net chip change per seat versus its starting stack, once terminal.
    /// Always sums to zero.
    pub fn payoffs(&self) -> Option<[i64; 2]> {
        self.finished.map(|f| f.payoffs)
    }

    /// Final stacks once terminal.
    pub fn final_stacks(&self) -> Option<[Chips; 2]> {
        self.finished.map(|_| self.stacks)
    }

    /// Both players' best five and rank, only if the hand reached showdown
    /// (never after a fold).
    pub fn showdown_hands(&self) -> Option<[ShowdownHand; 2]> {
        let ranks = self.finished?.ranks?;
        Some(Seat::ALL.map(|seat| {
            let seven = self.seven(seat);
            ShowdownHand {
                seat,
                hole_cards: self.hole[seat.index()],
                best_five: best_five(&seven),
                rank: ranks[seat.index()],
            }
        }))
    }

    /// The rules snapshot for the seat to act (`None` when terminal).
    pub fn betting_snapshot(&self) -> Option<BettingSnapshot> {
        let a = self.to_act?;
        let (ai, oi) = (a.index(), a.other().index());
        Some(BettingSnapshot {
            actor_street: self.street_c[ai],
            opp_street: self.street_c[oi],
            actor_stack: self.stacks[ai],
            opp_stack: self.stacks[oi],
            pot: self.pot(),
            last_full_raise: self.last_full_raise,
            big_blind: self.config.rules.big_blind,
            can_raise: self.can_raise[ai],
        })
    }

    /// Legal actions for the seat to act, in canonical order; empty when
    /// terminal.
    pub fn legal_actions(&self) -> Vec<LegalAction> {
        match self.betting_snapshot() {
            Some(s) => rules::legal_actions(&s).expect("state machine keeps snapshots consistent"),
            None => Vec::new(),
        }
    }

    // -------------------------------------------------------- transitions

    /// Applies the legal action of class `kind` (amount resolved from
    /// [`HandState::legal_actions`]). On error nothing changes.
    pub fn apply(&mut self, kind: ActionKind) -> Result<ActionOutcome, ActionError> {
        if self.is_terminal() {
            return Err(ActionError::HandOver);
        }
        let act = self
            .legal_actions()
            .into_iter()
            .find(|a| a.kind == kind)
            .ok_or(ActionError::Illegal(kind))?;
        Ok(self.commit(act))
    }

    /// Applies an exact [`LegalAction`]; it must equal an entry of
    /// [`HandState::legal_actions`]. On error nothing changes.
    pub fn apply_legal(&mut self, action: &LegalAction) -> Result<ActionOutcome, ActionError> {
        if self.is_terminal() {
            return Err(ActionError::HandOver);
        }
        if !self.legal_actions().contains(action) {
            return Err(ActionError::Mismatch(*action));
        }
        Ok(self.commit(*action))
    }

    /// Performs a validated action.
    fn commit(&mut self, act: LegalAction) -> ActionOutcome {
        let a = self.to_act.expect("non-terminal state has an actor");
        let o = a.other();
        let snap = self.betting_snapshot().expect("actor exists");
        let record = ActionRecord {
            seat: a,
            street: self.street,
            kind: act.kind,
            raise_to: act.raise_to,
            chips_committed: act.chips_to_commit,
        };
        self.history.push(record);

        if act.kind == ActionKind::Fold {
            self.to_act = None;
            self.finish(HandResult::Fold {
                folder: self.position(a),
            });
            return self.outcome(record, true);
        }

        let (ai, oi) = (a.index(), o.index());
        self.stacks[ai] -= act.chips_to_commit;
        self.street_c[ai] = act.raise_to;
        self.total_c[ai] += act.chips_to_commit;
        let upd = rules::next_can_raise(&snap, &act, self.acted[oi]);
        self.last_full_raise = upd.last_full_raise;
        self.can_raise[oi] = upd.opponent_can_raise;
        self.acted[ai] = true;

        if self.needs_action(o) {
            self.to_act = Some(o);
            self.outcome(record, false)
        } else {
            self.to_act = None;
            self.close_round();
            self.outcome(record, true)
        }
    }

    fn outcome(&self, record: ActionRecord, round_closed: bool) -> ActionOutcome {
        ActionOutcome {
            record,
            round_closed,
            street: self.street,
            terminal: self.terminal_reason(),
        }
    }

    fn position(&self, seat: Seat) -> Position {
        if seat == self.config.button {
            Position::Button
        } else {
            Position::BigBlind
        }
    }

    /// Whether `seat` must still act in the current round.
    fn needs_action(&self, seat: Seat) -> bool {
        let (i, o) = (seat.index(), seat.other().index());
        if self.stacks[i] == 0 || self.street_c[i] > self.street_c[o] {
            return false;
        }
        self.street_c[i] < self.street_c[o] || (!self.acted[i] && self.stacks[o] > 0)
    }

    /// Picks the first actor of a round, or closes it immediately if nobody
    /// can act.
    fn begin_round(&mut self, first: Seat) {
        self.to_act = if self.needs_action(first) {
            Some(first)
        } else if self.needs_action(first.other()) {
            Some(first.other())
        } else {
            None
        };
        if self.to_act.is_none() {
            self.close_round();
        }
    }

    /// Ends the current betting round: next street, or runout + showdown.
    fn close_round(&mut self) {
        let all_in = self.stacks[0] == 0 || self.stacks[1] == 0;
        let next = match self.street.betting() {
            Some(s) if !all_in => s.next(),
            _ => None,
        };
        match next {
            Some(n) => {
                self.street = Street::from_betting(n);
                self.revealed = n.board_len() as u8;
                self.street_c = [0; 2];
                self.last_full_raise = self.config.rules.big_blind;
                self.can_raise = [true; 2];
                self.acted = [false; 2];
                let first = match n.first_to_act() {
                    Position::Button => self.config.button,
                    Position::BigBlind => self.big_blind(),
                };
                self.begin_round(first);
            }
            None => {
                // River closed or no more betting possible: run out the board.
                self.revealed = 5;
                let r = Seat::ALL.map(|s| evaluate7(&self.seven(s)));
                let winner = match r[0].cmp(&r[1]) {
                    std::cmp::Ordering::Greater => Some(self.position(Seat::Zero)),
                    std::cmp::Ordering::Less => Some(self.position(Seat::One)),
                    std::cmp::Ordering::Equal => None,
                };
                self.finish(HandResult::Showdown { winner });
                if let Some(f) = self.finished.as_mut() {
                    f.ranks = Some(r);
                }
            }
        }
    }

    /// Settles chips and marks the hand terminal.
    fn finish(&mut self, result: HandResult) {
        let btn = self.config.button.index();
        let bb = self.big_blind().index();
        let st = rules::settle([self.total_c[btn], self.total_c[bb]], result);
        for (seat_i, pos_i) in [(btn, 0), (bb, 1)] {
            self.stacks[seat_i] += st.refunds[pos_i] + st.awards[pos_i];
        }
        let payoffs = [0, 1].map(|i| i64::from(self.stacks[i]) - i64::from(self.initial_stacks[i]));
        let reason = match result {
            HandResult::Fold { folder } => TerminalReason::Fold {
                winner: if folder == Position::Button {
                    self.big_blind()
                } else {
                    self.config.button
                },
            },
            HandResult::Showdown { .. } => TerminalReason::Showdown,
        };
        self.street = Street::Showdown;
        self.to_act = None;
        self.street_c = [0; 2];
        self.finished = Some(Finished {
            reason,
            payoffs,
            ranks: None,
        });
    }

    fn seven(&self, seat: Seat) -> [Card; 7] {
        let h = self.hole[seat.index()];
        let b = self.board;
        [h[0], h[1], b[0], b[1], b[2], b[3], b[4]]
    }
}

/// A sequence of hands between two seats: alternates the button and keeps
/// a running score. Stacks reset to the configured depth every hand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    rules: RulesConfig,
    next_button: Seat,
    hands_played: u64,
    score: [i64; 2],
}

impl Match {
    /// New match; `first_button` holds the button on hand 0.
    pub fn new(rules: RulesConfig, first_button: Seat) -> Result<Match, HandError> {
        rules.validate().map_err(HandError::Config)?;
        Ok(Match {
            rules,
            next_button: first_button,
            hands_played: 0,
            score: [0; 2],
        })
    }

    /// Config for the next hand.
    pub fn next_config(&self) -> HandConfig {
        HandConfig {
            rules: self.rules,
            button: self.next_button,
        }
    }

    /// Starts the next hand with the given deck.
    pub fn start_hand(&self, deck: Deck) -> Result<HandState, HandError> {
        HandState::new(self.next_config(), deck)
    }

    /// Records a finished hand's payoffs and moves the button. Returns
    /// `false` (and records nothing) if the hand is not terminal.
    pub fn record(&mut self, hand: &HandState) -> bool {
        let Some(p) = hand.payoffs() else {
            return false;
        };
        self.score[0] += p[0];
        self.score[1] += p[1];
        self.hands_played += 1;
        self.next_button = next_button(self.next_button);
        true
    }

    /// Cumulative chips won per seat.
    pub fn score(&self) -> [i64; 2] {
        self.score
    }

    /// Hands recorded so far.
    pub fn hands_played(&self) -> u64 {
        self.hands_played
    }
}

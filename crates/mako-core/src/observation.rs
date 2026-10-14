//! Player observations: the strict imperfect-information boundary.
//!
//! A [`PlayerObservation`] is an owned snapshot of exactly what one seat may
//! know: its own hole cards, the revealed board, public chip counts and the
//! public action history. The opponent's hole cards are included **only**
//! when the hand ended at showdown (never after a fold, never before the
//! river closes). Observations hold no reference to [`HandState`], so
//! player-facing code (CLI, bots, browser) can be handed one without access
//! to hidden state.

use crate::action::{ActionRecord, LegalAction, Seat, Street, TerminalReason};
use crate::card::Card;
use crate::rules::Chips;
use crate::state::HandState;

/// What `seat` can legally know about the hand.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PlayerObservation {
    pub seat: Seat,
    pub button: Seat,
    pub street: Street,
    pub hole_cards: [Card; 2],
    /// Revealed board cards only.
    pub board: Vec<Card>,
    pub pot: Chips,
    /// Chips behind, indexed by [`Seat::index`].
    pub stacks: [Chips; 2],
    /// Current-street contributions, indexed by [`Seat::index`].
    pub street_contributions: [Chips; 2],
    /// Blind sizes are public.
    pub small_blind: Chips,
    pub big_blind: Chips,
    pub history: Vec<ActionRecord>,
    pub to_act: Option<Seat>,
    /// Legal actions, non-empty only when `seat` is to act.
    pub legal_actions: Vec<LegalAction>,
    pub terminal: Option<TerminalReason>,
    /// Net result per seat once terminal (public: derivable from the
    /// history and, at showdown, the revealed cards).
    pub payoffs: Option<[i64; 2]>,
    /// Opponent's hole cards, `Some` only at a showdown-terminal state.
    pub opponent_hole_cards: Option<[Card; 2]>,
}

impl PlayerObservation {
    /// Whether this seat must act now.
    pub fn is_my_turn(&self) -> bool {
        self.to_act == Some(self.seat)
    }
}

impl HandState {
    /// The observation for `seat`. This is the only state view player-facing
    /// code should use.
    pub fn observation(&self, seat: Seat) -> PlayerObservation {
        let opp = seat.other();
        let terminal = self.terminal_reason();
        PlayerObservation {
            seat,
            button: self.button(),
            street: self.street(),
            hole_cards: self.hole_cards(seat),
            board: self.board().to_vec(),
            pot: self.pot(),
            stacks: Seat::ALL.map(|s| self.stack(s)),
            street_contributions: Seat::ALL.map(|s| self.street_contribution(s)),
            small_blind: self.config().rules.small_blind,
            big_blind: self.config().rules.big_blind,
            history: self.history().to_vec(),
            to_act: self.to_act(),
            legal_actions: if self.to_act() == Some(seat) {
                self.legal_actions()
            } else {
                Vec::new()
            },
            terminal,
            payoffs: self.payoffs(),
            opponent_hole_cards: (terminal == Some(TerminalReason::Showdown))
                .then(|| self.hole_cards(opp)),
        }
    }
}

//! Replayable hand event stream with per-viewer visibility.
//!
//! [`HandState::events`] derives the complete ordered event stream of a
//! hand (so far) as a pure function of the state: start, private hole-card
//! deals, actions, board deals, showdown and result. [`HandState::events_for`]
//! filters it for a [`Viewer`]:
//!
//! * [`Viewer::Seat`] — a player: sees its **own** [`HandEvent::HoleCards`]
//!   only; the opponent's cards appear only inside a legitimate
//!   [`HandEvent::Showdown`].
//! * [`Viewer::Spectator`] — sees no hole cards until showdown.
//! * [`Viewer::Omniscient`] — **not player-facing.** Sees every event,
//!   including both players' hole cards from the start. Only for post-hand
//!   analysis, logs and debugging; never feed it to a live bot, coach or UI.
//!
//! Undealt board cards never appear in any stream: board events are emitted
//! only for cards already revealed by the state.

use crate::action::{ActionRecord, Seat, Street, TerminalReason};
use crate::card::Card;
use crate::rules::{self, Chips, RulesConfig};
use crate::state::{HandState, ShowdownHand};

/// Who is looking at the event stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Viewer {
    /// A player in the hand.
    Seat(Seat),
    /// A third party with only public information.
    Spectator,
    /// Full information. **Not player-facing**: analysis and logs only.
    Omniscient,
}

/// One event in a hand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HandEvent {
    /// Hand configuration and blinds (always first).
    HandStarted {
        rules: RulesConfig,
        button: Seat,
        /// Indexed by [`Seat::index`].
        starting_stacks: [Chips; 2],
        /// Blind actually posted by each seat, indexed by [`Seat::index`].
        blinds_posted: [Chips; 2],
    },
    /// Private: `seat`'s hole cards. Visible to that seat (and Omniscient).
    HoleCards { seat: Seat, cards: [Card; 2] },
    /// A voluntary action.
    Action(ActionRecord),
    /// Board cards revealed at the start of `street` (3 on the flop, 1 on
    /// turn and river).
    BoardDealt { street: Street, cards: Vec<Card> },
    /// Both hands revealed at showdown, indexed by [`Seat::index`].
    Showdown { hands: [ShowdownHand; 2] },
    /// The hand is over. Arrays are indexed by [`Seat::index`].
    HandEnded {
        reason: TerminalReason,
        payoffs: [i64; 2],
        final_stacks: [Chips; 2],
    },
}

impl HandEvent {
    /// Whether `viewer` may see this event.
    pub fn visible_to(&self, viewer: Viewer) -> bool {
        match (self, viewer) {
            (_, Viewer::Omniscient) => true,
            (HandEvent::HoleCards { seat, .. }, Viewer::Seat(v)) => *seat == v,
            (HandEvent::HoleCards { .. }, Viewer::Spectator) => false,
            _ => true,
        }
    }
}

/// Board slice revealed on `street`.
fn street_cards(board: &[Card], street: Street) -> Vec<Card> {
    match street {
        Street::Flop => board[0..3].to_vec(),
        Street::Turn => board[3..4].to_vec(),
        Street::River => board[4..5].to_vec(),
        Street::Preflop | Street::Showdown => Vec::new(),
    }
}

/// Number of board cards visible on `street` (Showdown counts as 5).
fn board_len(street: Street) -> usize {
    street.betting().map_or(5, |s| s.board_len())
}

impl HandState {
    /// The complete event stream so far (omniscient: includes both players'
    /// hole cards). Use [`HandState::events_for`] for anything player-facing.
    pub fn events(&self) -> Vec<HandEvent> {
        let btn = self.button();
        let bb = btn.other();
        let start = Seat::ALL.map(|s| self.starting_stack(s));
        let posted =
            rules::post_blinds(&self.config().rules, start[btn.index()], start[bb.index()]);
        let mut blinds = [0; 2];
        blinds[btn.index()] = posted.button_posted;
        blinds[bb.index()] = posted.big_blind_posted;

        let mut ev = vec![HandEvent::HandStarted {
            rules: self.config().rules,
            button: btn,
            starting_stacks: start,
            blinds_posted: blinds,
        }];
        // Deal order: BB first, then button.
        for seat in [bb, btn] {
            ev.push(HandEvent::HoleCards {
                seat,
                cards: self.hole_cards(seat),
            });
        }

        let board = self.board();
        let mut shown = 0usize;
        let mut emit_board_to = |target: usize, ev: &mut Vec<HandEvent>| {
            for street in [Street::Flop, Street::Turn, Street::River] {
                let len = board_len(street);
                if len > shown && len <= target {
                    ev.push(HandEvent::BoardDealt {
                        street,
                        cards: street_cards(board, street),
                    });
                    shown = len;
                }
            }
        };
        for r in self.history() {
            emit_board_to(board_len(r.street), &mut ev);
            ev.push(HandEvent::Action(*r));
        }
        emit_board_to(board.len(), &mut ev);

        if let Some(hands) = self.showdown_hands() {
            ev.push(HandEvent::Showdown { hands });
        }
        if let (Some(reason), Some(payoffs), Some(final_stacks)) =
            (self.terminal_reason(), self.payoffs(), self.final_stacks())
        {
            ev.push(HandEvent::HandEnded {
                reason,
                payoffs,
                final_stacks,
            });
        }
        ev
    }

    /// The event stream filtered for `viewer` (see [`Viewer`]).
    pub fn events_for(&self, viewer: Viewer) -> Vec<HandEvent> {
        self.events()
            .into_iter()
            .filter(|e| e.visible_to(viewer))
            .collect()
    }
}

//! Heads-up no-limit hold'em betting rules and chip accounting.
//!
//! Pure, integer-only functions implementing `docs/GAME_RULES.md`. This
//! module deliberately does not depend on cards, decks or RNG; the hand
//! state machine (M-04) owns the mutable state and calls these helpers.
//!
//! All amounts are whole chips ([`Chips`] = `u32`). Bets and raises are
//! expressed as **raise-to** amounts: the actor's total contribution on the
//! current street after acting. Internally sums are computed in `u64` so no
//! intermediate can overflow.

use std::fmt;

/// Whole chips.
pub type Chips = u32;

/// Default small blind.
pub const DEFAULT_SMALL_BLIND: Chips = 1;
/// Default big blind.
pub const DEFAULT_BIG_BLIND: Chips = 2;
/// Largest allowed starting stack. Keeps `4 * stack` inside `u32`.
pub const MAX_STARTING_STACK: Chips = u32::MAX / 4;

/// Errors from rules helpers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RulesError {
    /// A [`RulesConfig`] field combination is not allowed.
    InvalidConfig(&'static str),
    /// A [`BettingSnapshot`] is internally inconsistent.
    InconsistentSnapshot(&'static str),
    /// The actor has no chips behind and cannot act.
    ActorAllIn,
    /// The requested action class is not in the legal list.
    IllegalAction(ActionKind),
    /// A pot split was requested with zero winners.
    NoWinners,
    /// Stacks plus pot differ from the chips put into play.
    ConservationViolated { expected: u64, actual: u64 },
    /// Per-hand payoffs do not sum to zero.
    NotZeroSum { sum: i64 },
    /// A hand contribution exceeds [`MAX_STARTING_STACK`] (would overflow).
    ContributionOutOfRange(Chips),
}

impl fmt::Display for RulesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RulesError::InvalidConfig(m) => write!(f, "invalid rules config: {m}"),
            RulesError::InconsistentSnapshot(m) => write!(f, "inconsistent betting snapshot: {m}"),
            RulesError::ActorAllIn => write!(f, "actor is all-in and cannot act"),
            RulesError::IllegalAction(k) => write!(f, "illegal action {k:?}"),
            RulesError::NoWinners => write!(f, "pot split with zero winners"),
            RulesError::ConservationViolated { expected, actual } => {
                write!(
                    f,
                    "chip conservation violated: expected {expected}, found {actual}"
                )
            }
            RulesError::NotZeroSum { sum } => write!(f, "payoffs sum to {sum}, not zero"),
            RulesError::ContributionOutOfRange(c) => {
                write!(f, "hand contribution {c} exceeds the maximum stack")
            }
        }
    }
}

impl std::error::Error for RulesError {}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Supported starting stack depths, in big blinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StackDepth {
    /// 20 big blinds (40 chips at 1/2).
    Bb20,
    /// 50 big blinds (100 chips at 1/2).
    Bb50,
    /// 100 big blinds (200 chips at 1/2).
    Bb100,
}

impl StackDepth {
    /// All depths, shallow to deep.
    pub const ALL: [StackDepth; 3] = [StackDepth::Bb20, StackDepth::Bb50, StackDepth::Bb100];

    /// Depth in big blinds.
    pub const fn big_blinds(self) -> Chips {
        match self {
            StackDepth::Bb20 => 20,
            StackDepth::Bb50 => 50,
            StackDepth::Bb100 => 100,
        }
    }

    /// Starting stack in chips at the default 1/2 blinds (40/100/200).
    pub const fn starting_stack(self) -> Chips {
        self.big_blinds() * DEFAULT_BIG_BLIND
    }
}

/// Blind and stack configuration. Stacks reset to `starting_stack` for each
/// seat at the start of every hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RulesConfig {
    pub small_blind: Chips,
    pub big_blind: Chips,
    pub starting_stack: Chips,
}

impl RulesConfig {
    /// Builds and validates a config.
    pub fn new(
        small_blind: Chips,
        big_blind: Chips,
        starting_stack: Chips,
    ) -> Result<RulesConfig, RulesError> {
        let c = RulesConfig {
            small_blind,
            big_blind,
            starting_stack,
        };
        c.validate()?;
        Ok(c)
    }

    /// Standard 1/2 blinds at the given depth.
    pub const fn standard(depth: StackDepth) -> RulesConfig {
        RulesConfig {
            small_blind: DEFAULT_SMALL_BLIND,
            big_blind: DEFAULT_BIG_BLIND,
            starting_stack: depth.starting_stack(),
        }
    }

    /// Checks: `1 <= small_blind <= big_blind`, `1 <= starting_stack <=
    /// MAX_STARTING_STACK`. A stack smaller than a blind is permitted (the
    /// blind is then posted all-in), though the shipped depths never do it.
    pub fn validate(&self) -> Result<(), RulesError> {
        if self.small_blind == 0 {
            return Err(RulesError::InvalidConfig("small blind must be at least 1"));
        }
        if self.big_blind < self.small_blind {
            return Err(RulesError::InvalidConfig(
                "big blind must be at least the small blind",
            ));
        }
        if self.starting_stack == 0 {
            return Err(RulesError::InvalidConfig("starting stack must be positive"));
        }
        if self.starting_stack > MAX_STARTING_STACK {
            return Err(RulesError::InvalidConfig("starting stack too large"));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Positions, streets, blinds
// ---------------------------------------------------------------------------

/// Heads-up position. The button is also the small blind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Position {
    /// Button / small blind: acts first preflop, last postflop.
    Button,
    /// Big blind: acts last preflop, first postflop.
    BigBlind,
}

impl Position {
    /// Array index used by settlement helpers: Button = 0, BigBlind = 1.
    pub const fn index(self) -> usize {
        match self {
            Position::Button => 0,
            Position::BigBlind => 1,
        }
    }

    /// The other position.
    pub const fn other(self) -> Position {
        match self {
            Position::Button => Position::BigBlind,
            Position::BigBlind => Position::Button,
        }
    }
}

/// Seat index (0 or 1) holding the button on hand number `hand_index`,
/// given the seat that held it on hand 0. The button alternates every hand.
pub const fn button_seat(first_button_seat: u8, hand_index: u64) -> u8 {
    ((first_button_seat as u64 + hand_index) % 2) as u8
}

/// Betting rounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Street {
    Preflop,
    Flop,
    Turn,
    River,
}

impl Street {
    /// All streets in order.
    pub const ALL: [Street; 4] = [Street::Preflop, Street::Flop, Street::Turn, Street::River];

    /// Who acts first on this street.
    pub const fn first_to_act(self) -> Position {
        match self {
            Street::Preflop => Position::Button,
            _ => Position::BigBlind,
        }
    }

    /// Board cards revealed when this street begins (0, 3, 1, 1).
    pub const fn cards_revealed(self) -> usize {
        match self {
            Street::Preflop => 0,
            Street::Flop => 3,
            Street::Turn | Street::River => 1,
        }
    }

    /// Total board cards visible during this street (0, 3, 4, 5).
    pub const fn board_len(self) -> usize {
        match self {
            Street::Preflop => 0,
            Street::Flop => 3,
            Street::Turn => 4,
            Street::River => 5,
        }
    }

    /// The following street, or `None` after the river (showdown).
    pub const fn next(self) -> Option<Street> {
        match self {
            Street::Preflop => Some(Street::Flop),
            Street::Flop => Some(Street::Turn),
            Street::Turn => Some(Street::River),
            Street::River => None,
        }
    }
}

/// Blinds actually posted at the start of a hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlindPosting {
    /// Chips posted by the button (small blind), `min(sb, button_stack)`.
    pub button_posted: Chips,
    /// Chips posted by the big blind, `min(bb, bb_stack)`.
    pub big_blind_posted: Chips,
}

/// Blind amounts each player posts. A player whose stack is smaller than
/// their blind posts the whole stack and is all-in.
pub fn post_blinds(config: &RulesConfig, button_stack: Chips, bb_stack: Chips) -> BlindPosting {
    BlindPosting {
        button_posted: config.small_blind.min(button_stack),
        big_blind_posted: config.big_blind.min(bb_stack),
    }
}

// ---------------------------------------------------------------------------
// Actions and legality
// ---------------------------------------------------------------------------

/// The six abstract action classes, in canonical order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ActionKind {
    Fold,
    Check,
    Call,
    HalfPot,
    Pot,
    AllIn,
}

impl ActionKind {
    /// All classes in canonical order.
    pub const ALL: [ActionKind; 6] = [
        ActionKind::Fold,
        ActionKind::Check,
        ActionKind::Call,
        ActionKind::HalfPot,
        ActionKind::Pot,
        ActionKind::AllIn,
    ];

    /// Canonical index `0..6`.
    pub const fn index(self) -> usize {
        self as usize
    }

    /// Whether this class bets or raises (puts in more than a call).
    pub const fn is_aggressive(self) -> bool {
        matches!(
            self,
            ActionKind::HalfPot | ActionKind::Pot | ActionKind::AllIn
        )
    }
}

/// A concrete legal action.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LegalAction {
    pub kind: ActionKind,
    /// Actor's total street contribution after the action. For `Fold` this
    /// is the unchanged current contribution.
    pub raise_to: Chips,
    /// Chips moved from the actor's stack into the pot by this action.
    pub chips_to_commit: Chips,
}

/// Everything the legality function needs about the current decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BettingSnapshot {
    /// Actor's contribution on the current street (blinds count preflop).
    pub actor_street: Chips,
    /// Opponent's contribution on the current street.
    pub opp_street: Chips,
    /// Actor's chips behind.
    pub actor_stack: Chips,
    /// Opponent's chips behind.
    pub opp_stack: Chips,
    /// All chips committed this hand by both players, current street
    /// included.
    pub pot: Chips,
    /// Size of the last full bet/raise increment on this street. At the
    /// start of every street (and preflop, after blinds) it is the big
    /// blind.
    pub last_full_raise: Chips,
    /// Configured big blind (floor for the minimum raise).
    pub big_blind: Chips,
    /// Whether the actor may bet/raise (false after a short all-in raise
    /// facing a player who already acted; see [`next_can_raise`]).
    pub can_raise: bool,
}

impl BettingSnapshot {
    /// Chips needed to call.
    pub const fn to_call(&self) -> Chips {
        self.opp_street.saturating_sub(self.actor_street)
    }

    /// Minimum bet/raise increment: `max(last_full_raise, big_blind)`.
    pub fn min_raise_increment(&self) -> Chips {
        self.last_full_raise.max(self.big_blind)
    }

    /// Actor's all-in raise-to amount.
    pub fn all_in_to(&self) -> Chips {
        // Fits: validate() ensures pot >= actor_street and the u64 sum of
        // pot + stacks is bounded by config limits in practice; use
        // saturating math as a belt-and-braces guard.
        self.actor_street.saturating_add(self.actor_stack)
    }

    /// Checks internal consistency.
    pub fn validate(&self) -> Result<(), RulesError> {
        if self.big_blind == 0 {
            return Err(RulesError::InconsistentSnapshot("big blind is zero"));
        }
        if self.opp_street < self.actor_street {
            return Err(RulesError::InconsistentSnapshot(
                "actor already leads the street; it is not the actor's turn",
            ));
        }
        if u64::from(self.pot) < u64::from(self.actor_street) + u64::from(self.opp_street) {
            return Err(RulesError::InconsistentSnapshot(
                "pot smaller than current street contributions",
            ));
        }
        let total = u64::from(self.pot) + u64::from(self.actor_stack) + u64::from(self.opp_stack);
        if total > u64::from(MAX_STARTING_STACK) * 2 {
            return Err(RulesError::InconsistentSnapshot("chip totals out of range"));
        }
        if self.actor_stack == 0 {
            return Err(RulesError::ActorAllIn);
        }
        Ok(())
    }
}

/// Legal actions in canonical order (Fold, Check, Call, HalfPot, Pot,
/// AllIn), after sizing, min-raise bumping and deduplication. See
/// `docs/GAME_RULES.md` §5 for the full specification.
pub fn legal_actions(s: &BettingSnapshot) -> Result<Vec<LegalAction>, RulesError> {
    s.validate()?;
    let to_call = s.to_call();
    let mut out = Vec::with_capacity(6);

    if to_call > 0 {
        out.push(LegalAction {
            kind: ActionKind::Fold,
            raise_to: s.actor_street,
            chips_to_commit: 0,
        });
        let pay = to_call.min(s.actor_stack);
        out.push(LegalAction {
            kind: ActionKind::Call,
            raise_to: s.actor_street + pay,
            chips_to_commit: pay,
        });
    } else {
        out.push(LegalAction {
            kind: ActionKind::Check,
            raise_to: s.actor_street,
            chips_to_commit: 0,
        });
    }

    // Raising requires: permission, an opponent with chips behind, and
    // more chips than the call.
    let may_raise = s.can_raise && s.opp_stack > 0 && s.actor_stack > to_call;
    if may_raise {
        let all_in_to = s.all_in_to();
        let min_inc = s.min_raise_increment();
        let pot_after_call = u64::from(s.pot) + u64::from(to_call);
        let sized = |increment: u64| -> Option<Chips> {
            let inc = increment.max(u64::from(min_inc));
            let to = u64::from(s.opp_street) + inc;
            (to < u64::from(all_in_to)).then_some(to as Chips)
        };
        let half_to = sized(pot_after_call / 2);
        let pot_to = sized(pot_after_call);
        let mk = |kind, to: Chips| LegalAction {
            kind,
            raise_to: to,
            chips_to_commit: to - s.actor_street,
        };
        if let Some(h) = half_to.filter(|&h| Some(h) != pot_to) {
            out.push(mk(ActionKind::HalfPot, h));
        }
        if let Some(p) = pot_to {
            out.push(mk(ActionKind::Pot, p));
        }
        out.push(mk(ActionKind::AllIn, all_in_to));
    }
    Ok(out)
}

/// Resolves a requested action class against the legal list; any class not
/// present is rejected without side effects.
pub fn resolve_action(s: &BettingSnapshot, kind: ActionKind) -> Result<LegalAction, RulesError> {
    legal_actions(s)?
        .into_iter()
        .find(|a| a.kind == kind)
        .ok_or(RulesError::IllegalAction(kind))
}

/// Effect of an action on the street's raise bookkeeping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RaiseUpdate {
    /// `raise_to - opp_street` if the action bet/raised, else 0.
    pub increment: Chips,
    /// Whether the increment is at least a full minimum raise.
    pub is_full_raise: bool,
    /// New `last_full_raise` for the street (changes only on a full raise).
    pub last_full_raise: Chips,
    /// New `can_raise` flag for the opponent, who acts next.
    pub opponent_can_raise: bool,
}

/// Raise bookkeeping after `action` is taken from snapshot `s`.
///
/// `opponent_has_acted` is whether the opponent has voluntarily acted on
/// this street already (posting a blind is **not** an action).
///
/// Rule: the opponent may raise next iff they have not yet acted this
/// street, or the action was a full bet/raise. A short all-in raise
/// therefore does not reopen betting for an opponent who already acted.
pub fn next_can_raise(
    s: &BettingSnapshot,
    action: &LegalAction,
    opponent_has_acted: bool,
) -> RaiseUpdate {
    let increment = action.raise_to.saturating_sub(s.opp_street);
    let is_full_raise = increment > 0 && increment >= s.min_raise_increment();
    RaiseUpdate {
        increment,
        is_full_raise,
        last_full_raise: if is_full_raise {
            increment
        } else {
            s.last_full_raise
        },
        opponent_can_raise: !opponent_has_acted || is_full_raise,
    }
}

// ---------------------------------------------------------------------------
// Settlement and invariants
// ---------------------------------------------------------------------------

/// Uncalled chips returned before payout, indexed by [`Position::index`].
/// Given each player's total hand contribution, the larger contributor gets
/// back the excess over the smaller one.
pub fn unmatched_refund(contributions: [Chips; 2]) -> [Chips; 2] {
    let [a, b] = contributions;
    if a > b { [a - b, 0] } else { [0, b - a] }
}

/// Splits `pot` among `n_winners` without losing chips. Every share is
/// `pot / n`; the `pot % n` odd chips go one each to the first winners in
/// the supplied order. Callers order winners by odd-chip priority (the big
/// blind first; see `docs/GAME_RULES.md` §7).
pub fn split_pot(pot: Chips, n_winners: usize) -> Result<Vec<Chips>, RulesError> {
    if n_winners == 0 {
        return Err(RulesError::NoWinners);
    }
    let n = n_winners as u64;
    let base = (u64::from(pot) / n) as Chips;
    let odd = (u64::from(pot) % n) as usize;
    Ok((0..n_winners)
        .map(|i| base + Chips::from(i < odd))
        .collect())
}

/// How a hand ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandResult {
    /// The given position folded; the other wins everything.
    Fold { folder: Position },
    /// Showdown with a winner, or `None` for a tie (split pot).
    Showdown { winner: Option<Position> },
}

/// Final chip movement for a hand, indexed by [`Position::index`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settlement {
    /// Uncalled chips returned to their owner.
    pub refunds: [Chips; 2],
    /// The contested (matched) pot, `2 * min(contributions)`.
    pub contested_pot: Chips,
    /// Share of the contested pot awarded to each position.
    pub awards: [Chips; 2],
    /// Net result: `refund + award - contribution`. Sums to zero.
    pub payoffs: [i64; 2],
}

/// Settles a hand from total contributions (indexed by position).
///
/// Refunds unmatched chips first, then awards the contested pot. On a tie
/// the contested pot is split with the odd chip (if any) to the big blind;
/// heads-up with no antes the contested pot is always even, so in practice
/// ties split exactly.
///
/// # Panics
/// Panics if a contribution exceeds [`MAX_STARTING_STACK`]; use
/// [`try_settle`] to get an error instead.
pub fn settle(contributions: [Chips; 2], result: HandResult) -> Settlement {
    try_settle(contributions, result).expect("contribution within MAX_STARTING_STACK")
}

/// Checked [`settle`]: rejects any contribution above
/// [`MAX_STARTING_STACK`], which guarantees `2 * min(contributions)` and
/// every award fit in [`Chips`]. Intermediate sums use `u64`/`i64`.
pub fn try_settle(contributions: [Chips; 2], result: HandResult) -> Result<Settlement, RulesError> {
    if let Some(&c) = contributions.iter().find(|&&c| c > MAX_STARTING_STACK) {
        return Err(RulesError::ContributionOutOfRange(c));
    }
    let refunds = unmatched_refund(contributions);
    let matched = u64::from(contributions[0].min(contributions[1]));
    // <= 2 * (u32::MAX / 4) < u32::MAX, so the conversion cannot fail.
    let contested_pot = Chips::try_from(2 * matched).expect("bounded by MAX_STARTING_STACK");
    let mut awards = [0; 2];
    match result {
        HandResult::Fold { folder } => awards[folder.other().index()] = contested_pot,
        HandResult::Showdown { winner: Some(w) } => awards[w.index()] = contested_pot,
        HandResult::Showdown { winner: None } => {
            // Order: BigBlind first (odd-chip priority), then Button.
            let shares = split_pot(contested_pot, 2).expect("two winners");
            awards[Position::BigBlind.index()] = shares[0];
            awards[Position::Button.index()] = shares[1];
        }
    }
    let payoffs =
        [0, 1].map(|i| i64::from(refunds[i]) + i64::from(awards[i]) - i64::from(contributions[i]));
    Ok(Settlement {
        refunds,
        contested_pot,
        awards,
        payoffs,
    })
}

/// Checks `stacks[0] + stacks[1] + pot == 2 * starting_stack`.
pub fn check_conservation(
    stacks: [Chips; 2],
    pot: Chips,
    starting_stack: Chips,
) -> Result<(), RulesError> {
    let expected = 2 * u64::from(starting_stack);
    let actual = u64::from(stacks[0]) + u64::from(stacks[1]) + u64::from(pot);
    if expected == actual {
        Ok(())
    } else {
        Err(RulesError::ConservationViolated { expected, actual })
    }
}

/// Checks that per-hand payoffs sum to zero.
pub fn check_zero_sum(payoffs: [i64; 2]) -> Result<(), RulesError> {
    let sum = payoffs[0] + payoffs[1];
    if sum == 0 {
        Ok(())
    } else {
        Err(RulesError::NotZeroSum { sum })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settle_rejects_overflowing_contributions() {
        let big = MAX_STARTING_STACK + 1;
        let r = HandResult::Showdown { winner: None };
        assert_eq!(
            try_settle([big, big], r),
            Err(RulesError::ContributionOutOfRange(big))
        );
        assert_eq!(
            try_settle([u32::MAX, 5], r),
            Err(RulesError::ContributionOutOfRange(u32::MAX))
        );
        // Largest allowed contributions settle without overflow.
        let m = MAX_STARTING_STACK;
        let st = try_settle([m, m], r).unwrap();
        assert_eq!(st.contested_pot, 2 * m);
        assert_eq!(st.awards, [m, m]);
        assert_eq!(st.payoffs, [0, 0]);
        let st = try_settle(
            [m, m],
            HandResult::Fold {
                folder: Position::Button,
            },
        )
        .unwrap();
        assert_eq!(st.payoffs, [-i64::from(m), i64::from(m)]);
    }

    #[test]
    #[should_panic(expected = "MAX_STARTING_STACK")]
    fn settle_panics_on_overflowing_contributions() {
        settle([u32::MAX, u32::MAX], HandResult::Showdown { winner: None });
    }
}

//! Parsing of a typed line into a command. Pure: it only reads the legal
//! action list, so it cannot change game state.

use mako_core::action::{ActionKind, LegalAction};

/// Menu key for an action class. Check and call share `c` (they are never
/// both legal); `k` is accepted as an alias for check.
pub const fn key(kind: ActionKind) -> char {
    match kind {
        ActionKind::Fold => 'f',
        ActionKind::Check | ActionKind::Call => 'c',
        ActionKind::HalfPot => 'h',
        ActionKind::Pot => 'p',
        ActionKind::AllIn => 'a',
    }
}

/// A parsed in-hand command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InHand {
    Act(LegalAction),
    Help,
    Stats,
    Quit,
    /// Rejected input, with the reason to show before re-prompting.
    Invalid(&'static str),
}

fn kind_for_word(word: &str) -> Option<&'static [ActionKind]> {
    use ActionKind::*;
    Some(match word {
        "f" | "fold" => &[Fold],
        "c" => &[Check, Call],
        "k" | "check" => &[Check],
        "call" => &[Call],
        "h" | "half" | "halfpot" | "half-pot" => &[HalfPot],
        "p" | "pot" => &[Pot],
        "a" | "allin" | "all-in" | "shove" => &[AllIn],
        _ => return None,
    })
}

/// Parses one line typed during the player's turn.
pub fn parse_in_hand(line: &str, legal: &[LegalAction]) -> InHand {
    let word = line.trim().to_ascii_lowercase();
    match word.as_str() {
        "" => return InHand::Invalid("type an action key (or 'help')"),
        "?" | "help" => return InHand::Help,
        "s" | "stats" | "score" => return InHand::Stats,
        "q" | "quit" | "exit" => return InHand::Quit,
        _ => {}
    }
    if let Ok(n) = word.parse::<usize>() {
        return match n.checked_sub(1).and_then(|i| legal.get(i)) {
            Some(a) => InHand::Act(*a),
            None => InHand::Invalid("no action with that number"),
        };
    }
    match kind_for_word(&word) {
        Some(kinds) => legal
            .iter()
            .find(|a| kinds.contains(&a.kind))
            .map_or(InHand::Invalid("that action is not legal right now"), |a| {
                InHand::Act(*a)
            }),
        None => InHand::Invalid("unrecognised input"),
    }
}

/// A parsed between-hands command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BetweenHands {
    Continue,
    Stats,
    Help,
    Quit,
    Invalid,
}

/// Parses one line typed between hands.
pub fn parse_between(line: &str) -> BetweenHands {
    match line.trim().to_ascii_lowercase().as_str() {
        "" | "c" | "n" | "y" | "next" | "continue" | "deal" => BetweenHands::Continue,
        "s" | "stats" | "score" => BetweenHands::Stats,
        "?" | "help" => BetweenHands::Help,
        "q" | "quit" | "exit" => BetweenHands::Quit,
        _ => BetweenHands::Invalid,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn la(kind: ActionKind, to: u32, commit: u32) -> LegalAction {
        LegalAction {
            kind,
            raise_to: to,
            chips_to_commit: commit,
        }
    }

    fn facing_bet() -> Vec<LegalAction> {
        vec![
            la(ActionKind::Fold, 1, 0),
            la(ActionKind::Call, 2, 1),
            la(ActionKind::HalfPot, 4, 3),
            la(ActionKind::Pot, 6, 5),
            la(ActionKind::AllIn, 40, 39),
        ]
    }

    #[test]
    fn keys_words_and_numbers() {
        let l = facing_bet();
        assert_eq!(parse_in_hand("f", &l), InHand::Act(l[0]));
        assert_eq!(parse_in_hand(" C \n", &l), InHand::Act(l[1]));
        assert_eq!(parse_in_hand("call", &l), InHand::Act(l[1]));
        assert_eq!(parse_in_hand("h", &l), InHand::Act(l[2]));
        assert_eq!(parse_in_hand("p", &l), InHand::Act(l[3]));
        assert_eq!(parse_in_hand("all-in", &l), InHand::Act(l[4]));
        assert_eq!(parse_in_hand("1", &l), InHand::Act(l[0]));
        assert_eq!(parse_in_hand("5", &l), InHand::Act(l[4]));
    }

    #[test]
    fn check_aliases() {
        let l = vec![la(ActionKind::Check, 0, 0), la(ActionKind::AllIn, 30, 30)];
        assert_eq!(parse_in_hand("c", &l), InHand::Act(l[0]));
        assert_eq!(parse_in_hand("k", &l), InHand::Act(l[0]));
        assert_eq!(parse_in_hand("check", &l), InHand::Act(l[0]));
    }

    #[test]
    fn illegal_or_garbage_is_rejected() {
        let l = facing_bet();
        for bad in [
            "x", "k", "check", "6", "0", "-1", "", "   ", "zzz", "raise 7", "ff",
        ] {
            assert!(
                matches!(parse_in_hand(bad, &l), InHand::Invalid(_)),
                "{bad:?} should be invalid"
            );
        }
        let no_raise = vec![la(ActionKind::Fold, 0, 0), la(ActionKind::Call, 10, 8)];
        assert!(matches!(parse_in_hand("p", &no_raise), InHand::Invalid(_)));
        assert!(matches!(parse_in_hand("a", &no_raise), InHand::Invalid(_)));
        assert!(matches!(parse_in_hand("c", &[]), InHand::Invalid(_)));
    }

    #[test]
    fn meta_commands() {
        let l = facing_bet();
        assert_eq!(parse_in_hand("help", &l), InHand::Help);
        assert_eq!(parse_in_hand("?", &l), InHand::Help);
        assert_eq!(parse_in_hand("stats", &l), InHand::Stats);
        assert_eq!(parse_in_hand("QUIT", &l), InHand::Quit);
        assert_eq!(parse_in_hand("q", &l), InHand::Quit);
    }

    #[test]
    fn between_hands() {
        assert_eq!(parse_between(""), BetweenHands::Continue);
        assert_eq!(parse_between("c"), BetweenHands::Continue);
        assert_eq!(parse_between("quit"), BetweenHands::Quit);
        assert_eq!(parse_between("stats"), BetweenHands::Stats);
        assert_eq!(parse_between("help"), BetweenHands::Help);
        assert_eq!(parse_between("blah"), BetweenHands::Invalid);
    }
}

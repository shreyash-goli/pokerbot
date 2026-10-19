//! The retro terminal UI (M-15).
//!
//! Everything shown during a hand is rendered from the human seat's
//! `PlayerObservation`, the opponent's public label and session statistics;
//! no `HandState` reaches this module. Screens are pure functions returning
//! lines, so they can be golden-tested (`tests/snapshots/`).
//!
//! * [`term`]: colour/glyph/width detection and ANSI-aware helpers
//! * [`theme`]: the cream/sepia palette per colour depth
//! * [`table`]: the table, action log and legal-action menu
//! * [`screens`]: session/hand headers, street banners, results, prompts
//! * [`stats`]: session statistics panel (plain input struct)
//! * [`help`]: command help, listing exactly the working commands
//! * [`coach`]: post-hand analysis and replay viewer (M-18)

pub mod coach;
pub mod help;
pub mod screens;
pub mod stats;
pub mod table;
pub mod term;
pub mod theme;

use mako_core::action::Seat;
use mako_core::card::{Card, Suit};

pub use term::{ColorMode, Glyphs};
pub use theme::Role;

/// Marker line printed when a hand legitimately reaches showdown. Nothing
/// before it in a hand's output may contain the opponent's hole cards.
pub const SHOWDOWN_MARKER: &str = "*** SHOWDOWN ***";

/// Rendering context: colour depth, glyph set and width.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Ui {
    pub color: ColorMode,
    pub glyphs: Glyphs,
    /// Target width in cells; no rendered line is wider.
    pub width: usize,
}

impl Ui {
    /// Plain ASCII, no colour, 80 columns.
    pub const fn plain() -> Ui {
        Ui {
            color: ColorMode::None,
            glyphs: Glyphs::Ascii,
            width: term::DEFAULT_WIDTH,
        }
    }

    /// Whether the two-pane layout is used.
    pub const fn is_wide(&self) -> bool {
        self.width >= term::WIDE_FROM
    }

    pub fn paint(&self, role: Role, text: &str) -> String {
        theme::paint(self.color, role, text)
    }

    /// Card text without styling: `[A♥]` or `[Ah]`.
    pub fn card_text(&self, c: Card) -> String {
        match self.glyphs {
            Glyphs::Ascii => format!("[{c}]"),
            Glyphs::Unicode => {
                let suit = match c.suit() {
                    Suit::Spades => '\u{2660}',
                    Suit::Hearts => '\u{2665}',
                    Suit::Diamonds => '\u{2666}',
                    Suit::Clubs => '\u{2663}',
                };
                format!("[{}{suit}]", c.rank().to_char())
            }
        }
    }

    /// A face-up card.
    pub fn card(&self, c: Card) -> String {
        let role = match c.suit() {
            Suit::Hearts | Suit::Diamonds => Role::RedCard,
            Suit::Spades | Suit::Clubs => Role::BlackCard,
        };
        self.paint(role, &self.card_text(c))
    }

    /// Several face-up cards.
    pub fn cards(&self, cs: &[Card]) -> String {
        cs.iter().map(|&c| self.card(c)).collect()
    }

    /// Text of one face-down card: `[▓▓]` or `[##]`.
    pub fn back_text(&self) -> &'static str {
        match self.glyphs {
            Glyphs::Unicode => "[\u{2593}\u{2593}]",
            Glyphs::Ascii => "[##]",
        }
    }

    /// Two face-down cards (the opponent's hand before showdown).
    pub fn backs(&self) -> String {
        let one = self.paint(Role::Back, self.back_text());
        format!("{one}{one}")
    }

    /// An empty board slot: `[··]` or `[--]`.
    pub fn slot(&self) -> String {
        let t = match self.glyphs {
            Glyphs::Unicode => "[\u{b7}\u{b7}]",
            Glyphs::Ascii => "[--]",
        };
        self.paint(Role::Slot, t)
    }

    /// Board: revealed cards then empty slots up to five.
    pub fn board(&self, revealed: &[Card]) -> String {
        let mut s = self.cards(revealed);
        for _ in revealed.len()..5 {
            s.push_str(&self.slot());
        }
        s
    }

    /// Frame glyphs.
    pub const fn frame(&self) -> Frame {
        match self.glyphs {
            Glyphs::Unicode => Frame {
                tl: '\u{2554}',
                tr: '\u{2557}',
                bl: '\u{255a}',
                br: '\u{255d}',
                h: '\u{2550}',
                v: '\u{2551}',
                split: '\u{2502}',
                rule: '\u{2500}',
                dot: '\u{b7}',
                arrow: '\u{25b8}',
            },
            Glyphs::Ascii => Frame {
                tl: '+',
                tr: '+',
                bl: '+',
                br: '+',
                h: '=',
                v: '|',
                split: '|',
                rule: '-',
                dot: '|',
                arrow: '>',
            },
        }
    }

    /// A full-width horizontal rule with an optional caption.
    pub fn rule(&self, caption: &str) -> String {
        let f = self.frame();
        let r2: String = std::iter::repeat_n(f.rule, 2).collect();
        let head = if caption.is_empty() {
            String::new()
        } else {
            format!("{r2} {caption} ")
        };
        let rest = self.width.saturating_sub(term::visible_width(&head));
        let line: String = std::iter::repeat_n(f.rule, rest).collect();
        self.paint(
            Role::Frame,
            &term::fit(&format!("{head}{line}"), self.width),
        )
    }
}

/// Box-drawing characters of a glyph set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub tl: char,
    pub tr: char,
    pub bl: char,
    pub br: char,
    pub h: char,
    pub v: char,
    pub split: char,
    pub rule: char,
    pub dot: char,
    pub arrow: char,
}

/// Seat display names from the human's point of view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Names {
    pub human: Seat,
}

impl Names {
    pub fn of(&self, seat: Seat) -> &'static str {
        if seat == self.human { "You" } else { "Bot" }
    }
}

/// Signed chip amount, e.g. `+3`, `-12`, `0`.
pub fn chips_delta(v: i64) -> String {
    if v > 0 {
        format!("+{v}")
    } else {
        v.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mako_core::card::Rank;

    #[test]
    fn card_glyphs() {
        let ah = Card::new(Rank::Ace, Suit::Hearts);
        let ts = Card::new(Rank::Ten, Suit::Spades);
        let mut ui = Ui::plain();
        assert_eq!(ui.card(ah), "[Ah]");
        assert_eq!(ui.cards(&[ah, ts]), "[Ah][Ts]");
        assert_eq!(ui.backs(), "[##][##]");
        assert_eq!(ui.board(&[ah]), "[Ah][--][--][--][--]");
        ui.glyphs = Glyphs::Unicode;
        assert_eq!(ui.card(ah), "[A\u{2665}]");
        assert_eq!(ui.card(ts), "[T\u{2660}]");
        assert_eq!(term::visible_width(&ui.board(&[])), 20);
        ui.color = ColorMode::Ansi256;
        assert!(ui.card(ah).contains("[A\u{2665}]") && ui.card(ah).contains("\x1b["));
    }

    #[test]
    fn rules_fill_width() {
        for glyphs in [Glyphs::Ascii, Glyphs::Unicode] {
            for width in [48, 80, 133] {
                let ui = Ui {
                    color: ColorMode::TrueColor,
                    glyphs,
                    width,
                };
                assert_eq!(term::visible_width(&ui.rule("Hand 1 of 3")), width);
                assert_eq!(term::visible_width(&ui.rule("")), width);
            }
        }
    }
}

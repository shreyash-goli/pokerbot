//! The warm cream/sepia palette, expressed per colour depth.
//!
//! Only foreground colours are used for text and frames, so the UI reads
//! on light and dark terminals alike; card faces get a cream background so
//! they look like little paper cards. With [`ColorMode::None`] nothing is
//! emitted and every meaning is carried by text (brackets, words, `!!`).

use super::term::ColorMode;

/// What a piece of text is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    /// Table frame and rules.
    Frame,
    /// Title text.
    Title,
    /// Secondary text.
    Dim,
    /// Menu keys and highlights.
    Accent,
    /// Opponent label.
    Label,
    /// Face-up heart or diamond.
    RedCard,
    /// Face-up spade or club.
    BlackCard,
    /// Face-down card.
    Back,
    /// Empty board slot.
    Slot,
    /// Warnings (untrained fallback, errors).
    Warn,
    /// Chips won.
    Win,
    /// Chips lost.
    Lose,
    /// Pot and important numbers.
    Strong,
}

/// SGR parameters for a role in a colour mode (`None` = unstyled).
pub fn sgr(mode: ColorMode, role: Role) -> Option<&'static str> {
    use Role::*;
    Some(match mode {
        ColorMode::None => return None,
        ColorMode::TrueColor => match role {
            Frame => "38;2;176;137;84",
            Title => "1;38;2;236;201;140",
            Dim => "38;2;150;130;105",
            Accent => "1;38;2;232;170;80",
            Label => "38;2;214;196;160",
            RedCard => "1;38;2;165;42;42;48;2;242;230;201",
            BlackCard => "1;38;2;40;30;20;48;2;242;230;201",
            Back => "38;2;214;170;110;48;2;120;72;40",
            Slot => "38;2;120;102;80",
            Warn => "1;38;2;230;120;60",
            Win => "38;2;160;190;110",
            Lose => "38;2;210;110;90",
            Strong => "1;38;2;242;230;201",
        },
        ColorMode::Ansi256 => match role {
            Frame => "38;5;137",
            Title => "1;38;5;222",
            Dim => "38;5;101",
            Accent => "1;38;5;179",
            Label => "38;5;187",
            RedCard => "1;38;5;124;48;5;230",
            BlackCard => "1;38;5;235;48;5;230",
            Back => "38;5;180;48;5;94",
            Slot => "38;5;95",
            Warn => "1;38;5;166",
            Win => "38;5;107",
            Lose => "38;5;167",
            Strong => "1;38;5;230",
        },
        ColorMode::Ansi16 => match role {
            Frame => "33",
            Title => "1;93",
            Dim => "2",
            Accent => "1;33",
            Label => "37",
            RedCard => "1;31;47",
            BlackCard => "1;30;47",
            Back => "30;43",
            Slot => "2",
            Warn => "1;31",
            Win => "32",
            Lose => "31",
            Strong => "1;97",
        },
    })
}

/// Wraps `text` in the role's colour, or returns it unchanged.
pub fn paint(mode: ColorMode, role: Role, text: &str) -> String {
    match sgr(mode, role) {
        Some(code) if !text.is_empty() => format!("\x1b[{code}m{text}\x1b[0m"),
        _ => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_mode_emits_nothing() {
        assert_eq!(paint(ColorMode::None, Role::Warn, "!!"), "!!");
        for mode in [ColorMode::Ansi16, ColorMode::Ansi256, ColorMode::TrueColor] {
            let p = paint(mode, Role::RedCard, "[Ah]");
            assert!(p.starts_with("\x1b[") && p.ends_with("\x1b[0m"), "{p:?}");
            assert!(p.contains("[Ah]"));
        }
        assert!(paint(ColorMode::Ansi256, Role::Frame, "x").contains("38;5;"));
        assert!(paint(ColorMode::TrueColor, Role::Frame, "x").contains("38;2;"));
    }
}

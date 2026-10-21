//! Terminal capabilities (colour depth, glyph set, width), detected from the
//! environment without dependencies, plus ANSI-aware width helpers.

/// Colour depth actually used for output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColorMode {
    /// Plain text, no escape sequences at all.
    None,
    /// The 16 basic ANSI colours.
    Ansi16,
    /// The xterm 256-colour palette.
    Ansi256,
    /// 24-bit colour.
    TrueColor,
}

impl ColorMode {
    /// Name used by `--color` and in docs.
    pub const fn name(self) -> &'static str {
        match self {
            ColorMode::None => "never",
            ColorMode::Ansi16 => "16",
            ColorMode::Ansi256 => "256",
            ColorMode::TrueColor => "truecolor",
        }
    }
}

/// `--color` / `--no-color` request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColorChoice {
    /// Detect from the environment.
    Auto,
    /// Force a mode (even when not a TTY).
    Force(ColorMode),
}

/// Card and frame glyphs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Glyphs {
    /// Suit symbols ♠♥♦♣ and box-drawing frames.
    Unicode,
    /// Suit letters (`As`, `Kh`) and `+-|` frames.
    Ascii,
}

/// `--glyphs` request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GlyphChoice {
    /// Unicode when the locale says UTF-8, otherwise ASCII.
    Auto,
    Force(Glyphs),
}

/// Narrowest supported width; `--width` below it is rejected and a smaller
/// `COLUMNS` is raised to it.
pub const MIN_WIDTH: usize = 48;
/// Widest layout; larger widths are capped.
pub const MAX_WIDTH: usize = 240;
/// Width when neither `--width` nor `COLUMNS` is given.
pub const DEFAULT_WIDTH: usize = 80;
/// From this width on the wide (two-pane) table layout is used.
pub const WIDE_FROM: usize = 100;

/// Colour mode for `choice`. Auto: none if `NO_COLOR` is set (non-empty),
/// stdout is not a TTY, or `TERM=dumb`; truecolor if `COLORTERM` is
/// `truecolor`/`24bit`; 256 if `TERM` contains `256color`; otherwise 16.
pub fn detect_color(
    choice: ColorChoice,
    env: &dyn Fn(&str) -> Option<String>,
    stdout_is_tty: bool,
) -> ColorMode {
    if let ColorChoice::Force(m) = choice {
        return m;
    }
    if env("NO_COLOR").is_some_and(|v| !v.is_empty()) || !stdout_is_tty {
        return ColorMode::None;
    }
    let term = env("TERM").unwrap_or_default().to_ascii_lowercase();
    if term == "dumb" {
        return ColorMode::None;
    }
    let colorterm = env("COLORTERM").unwrap_or_default().to_ascii_lowercase();
    if colorterm == "truecolor" || colorterm == "24bit" {
        ColorMode::TrueColor
    } else if term.contains("256color") {
        ColorMode::Ansi256
    } else {
        ColorMode::Ansi16
    }
}

/// Glyph set for `choice`. Auto: Unicode if the first non-empty of
/// `LC_ALL`, `LC_CTYPE`, `LANG` names UTF-8, otherwise ASCII.
pub fn detect_glyphs(choice: GlyphChoice, env: &dyn Fn(&str) -> Option<String>) -> Glyphs {
    if let GlyphChoice::Force(g) = choice {
        return g;
    }
    let locale = ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .filter_map(|k| env(k))
        .find(|v| !v.is_empty())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if locale.contains("utf-8") || locale.contains("utf8") {
        Glyphs::Unicode
    } else {
        Glyphs::Ascii
    }
}

/// Width: `--width` if given, else `COLUMNS`, else [`DEFAULT_WIDTH`];
/// clamped to `MIN_WIDTH..=MAX_WIDTH`.
pub fn detect_width(flag: Option<usize>, env: &dyn Fn(&str) -> Option<String>) -> usize {
    flag.or_else(|| env("COLUMNS").and_then(|v| v.trim().parse().ok()))
        .unwrap_or(DEFAULT_WIDTH)
        .clamp(MIN_WIDTH, MAX_WIDTH)
}

/// Removes ANSI CSI escape sequences (`ESC [ ... final`).
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for d in chars.by_ref() {
                if ('\x40'..='\x7e').contains(&d) {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Visible width in terminal cells. Every glyph this UI emits (ASCII,
/// box drawing, suit symbols, shade blocks) is one cell wide.
pub fn visible_width(s: &str) -> usize {
    strip_ansi(s).chars().count()
}

/// Pads `s` with spaces to `width` visible cells (no-op if wider).
pub fn pad(s: &str, width: usize) -> String {
    let w = visible_width(s);
    if w >= width {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(width - w))
    }
}

/// Truncates `s` to at most `width` visible cells, keeping escape
/// sequences intact and marking the cut with `~`. Appends a reset if `s`
/// contained escapes.
pub fn fit(s: &str, width: usize) -> String {
    if visible_width(s) <= width {
        return s.to_string();
    }
    let keep = width.saturating_sub(1);
    let mut out = String::new();
    let mut seen = 0;
    let mut had_escape = false;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' && chars.peek() == Some(&'[') {
            had_escape = true;
            out.push(c);
            out.push(chars.next().expect("peeked"));
            for d in chars.by_ref() {
                out.push(d);
                if ('\x40'..='\x7e').contains(&d) {
                    break;
                }
            }
            continue;
        }
        if seen == keep {
            break;
        }
        out.push(c);
        seen += 1;
    }
    if had_escape {
        out.push_str("\x1b[0m");
    }
    if width > 0 {
        out.push('~');
    }
    out
}

/// Greedy word wrap of plain text to `width`, with `indent` before the
/// first line and `cont` before continuation lines. Words longer than the
/// line are cut by [`fit`].
pub fn wrap(text: &str, width: usize, indent: &str, cont: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = indent.to_string();
    let mut empty = true;
    for word in text.split_whitespace() {
        let need = visible_width(&line) + usize::from(!empty) + visible_width(word);
        if !empty && need > width {
            lines.push(fit(&line, width));
            line = cont.to_string();
            empty = true;
        }
        if !empty {
            line.push(' ');
        }
        line.push_str(word);
        empty = false;
    }
    if !empty || lines.is_empty() {
        lines.push(fit(&line, width));
    }
    lines
}

/// Lays out pre-styled items left to right separated by `gap`, wrapping
/// to `width`; continuation lines start with `cont`.
pub fn flow(items: &[String], width: usize, first: &str, cont: &str, gap: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = first.to_string();
    let mut empty = true;
    for item in items {
        let need =
            visible_width(&line) + if empty { 0 } else { visible_width(gap) } + visible_width(item);
        if !empty && need > width {
            lines.push(fit(&line, width));
            line = cont.to_string();
            empty = true;
        }
        if !empty {
            line.push_str(gap);
        }
        line.push_str(item);
        empty = false;
    }
    lines.push(fit(&line, width));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn color_detection() {
        let auto = ColorChoice::Auto;
        assert_eq!(detect_color(auto, &env_of(&[]), false), ColorMode::None);
        assert_eq!(detect_color(auto, &env_of(&[]), true), ColorMode::Ansi16);
        assert_eq!(
            detect_color(auto, &env_of(&[("TERM", "xterm-256color")]), true),
            ColorMode::Ansi256
        );
        assert_eq!(
            detect_color(
                auto,
                &env_of(&[("TERM", "xterm-256color"), ("COLORTERM", "truecolor")]),
                true
            ),
            ColorMode::TrueColor
        );
        assert_eq!(
            detect_color(auto, &env_of(&[("COLORTERM", "24bit")]), true),
            ColorMode::TrueColor
        );
        assert_eq!(
            detect_color(
                auto,
                &env_of(&[("NO_COLOR", "1"), ("COLORTERM", "truecolor")]),
                true
            ),
            ColorMode::None
        );
        assert_eq!(
            detect_color(auto, &env_of(&[("NO_COLOR", "")]), true),
            ColorMode::Ansi16,
            "empty NO_COLOR is ignored"
        );
        assert_eq!(
            detect_color(auto, &env_of(&[("TERM", "dumb")]), true),
            ColorMode::None
        );
        assert_eq!(
            detect_color(ColorChoice::Force(ColorMode::Ansi256), &env_of(&[]), false),
            ColorMode::Ansi256
        );
    }

    #[test]
    fn glyph_detection() {
        let auto = GlyphChoice::Auto;
        assert_eq!(detect_glyphs(auto, &env_of(&[])), Glyphs::Ascii);
        assert_eq!(
            detect_glyphs(auto, &env_of(&[("LANG", "en_US.UTF-8")])),
            Glyphs::Unicode
        );
        assert_eq!(
            detect_glyphs(auto, &env_of(&[("LC_ALL", "C"), ("LANG", "en_US.UTF-8")])),
            Glyphs::Ascii,
            "LC_ALL wins"
        );
        assert_eq!(
            detect_glyphs(auto, &env_of(&[("LC_ALL", ""), ("LC_CTYPE", "C.utf8")])),
            Glyphs::Unicode
        );
        assert_eq!(
            detect_glyphs(
                GlyphChoice::Force(Glyphs::Ascii),
                &env_of(&[("LANG", "x.UTF-8")])
            ),
            Glyphs::Ascii
        );
    }

    #[test]
    fn width_detection() {
        assert_eq!(detect_width(None, &env_of(&[])), 80);
        assert_eq!(detect_width(None, &env_of(&[("COLUMNS", "132")])), 132);
        assert_eq!(detect_width(None, &env_of(&[("COLUMNS", "20")])), MIN_WIDTH);
        assert_eq!(
            detect_width(None, &env_of(&[("COLUMNS", "9999")])),
            MAX_WIDTH
        );
        assert_eq!(detect_width(None, &env_of(&[("COLUMNS", "wide")])), 80);
        assert_eq!(detect_width(Some(100), &env_of(&[("COLUMNS", "60")])), 100);
    }

    #[test]
    fn ansi_helpers() {
        let s = "\x1b[1;31m[A\u{2665}]\x1b[0m ok";
        assert_eq!(strip_ansi(s), "[A\u{2665}] ok");
        assert_eq!(visible_width(s), 7);
        assert_eq!(visible_width(&pad(s, 10)), 10);
        let f = fit(s, 5);
        assert_eq!(visible_width(&f), 5);
        assert!(f.ends_with('~'));
        assert!(f.contains("\x1b[0m"));
        assert_eq!(fit("abc", 5), "abc");
        assert_eq!(fit("abcdef", 4), "abc~");
    }

    #[test]
    fn wrapping_respects_width() {
        let text = "the quick brown fox jumps over the lazy dog again and again";
        for w in [10, 17, 30] {
            for l in wrap(text, w, "  ", "    ") {
                assert!(visible_width(&l) <= w, "{l:?} > {w}");
            }
        }
        let items: Vec<String> = (0..9).map(|i| format!("[{i}] item")).collect();
        let lines = flow(&items, 30, "Opts: ", "      ", "  ");
        assert!(lines.len() > 1);
        assert!(lines.iter().all(|l| visible_width(l) <= 30));
        assert!(lines[1].starts_with("      "));
    }
}

//! Text for a terminal. Every string the human (non-JSON) views print
//! that comes from a file or from an argument (a theme, an entity, an
//! event name, a title, a quote, a path, a key, a corpus id, an error's
//! text) goes through [`safe`] on its way out, so no byte of an archive
//! file reaches the terminal as a control sequence (a colour change, a
//! cursor move, a rewritten title bar, a faked line).
//!
//! JSON output does not come here: `serde_json` writes every control
//! character below U+0020 as a `\u00XX` escape, and a JSON consumer is
//! not a terminal.

use std::borrow::Cow;

/// What stands in for a character a terminal could act on.
const REPLACEMENT: char = '\u{FFFD}';

/// Is `c` one a terminal may act on instead of showing? The C0 controls
/// but the tab (ESC, CR, LF, BEL, backspace … U+0000 to U+001F), DEL
/// (U+007F), the C1 controls (U+0080 to U+009F, CSI among them), and the
/// line and paragraph separators (U+2028, U+2029).
fn acts(c: char) -> bool {
    (c.is_control() && c != '\t') || matches!(c, '\u{2028}' | '\u{2029}')
}

/// `s` for a terminal: each character of [`acts`] replaced by U+FFFD,
/// one for one, so widths and alignment stay; everything else as it is.
/// A line break is replaced too: a string from a file is one line of
/// output at most (see [`safe_lines`] for text that is several lines by
/// design).
pub fn safe(s: &str) -> Cow<'_, str> {
    if s.chars().any(acts) {
        Cow::Owned(
            s.chars()
                .map(|c| if acts(c) { REPLACEMENT } else { c })
                .collect(),
        )
    } else {
        Cow::Borrowed(s)
    }
}

/// [`safe`] on each line of a text that is several lines by design (a
/// citation, a registry entry, an error with its causes): the line feeds
/// stay, every line between them goes through [`safe`].
pub fn safe_lines(s: &str) -> String {
    s.split('\n').map(safe).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_are_replaced_one_for_one_and_everything_else_is_kept() {
        // Untouched, and not copied.
        for keep in [
            "",
            "plain",
            "a\tb",
            "naïve — “quoted” 日本語 🙂",
            "a\u{a0}b\u{200b}c",
        ] {
            assert!(
                matches!(safe(keep), Cow::Borrowed(s) if s == keep),
                "{keep:?}"
            );
        }
        assert_eq!(safe("\u{1b}[31mred\u{1b}[0m"), "\u{fffd}[31mred\u{fffd}[0m");
        assert_eq!(safe("a\nb\rc"), "a\u{fffd}b\u{fffd}c");
        assert_eq!(safe("\u{7}\u{8}\u{0}\u{7f}"), "\u{fffd}".repeat(4));
        // C1: CSI (U+009B), OSC (U+009D), NEL (U+0085).
        assert_eq!(
            safe("\u{9b}2J\u{9d}0;t\u{85}"),
            "\u{fffd}2J\u{fffd}0;t\u{fffd}"
        );
        assert_eq!(safe("a\u{2028}b\u{2029}c"), "a\u{fffd}b\u{fffd}c");
        // Every character of the ranges, and the neighbours kept.
        for c in (0u32..=0x1f).chain(0x7f..=0x9f).chain([0x2028, 0x2029]) {
            let c = char::from_u32(c).unwrap();
            let want = if c == '\t' { "\t" } else { "\u{fffd}" };
            assert_eq!(safe(&c.to_string()), want, "{:x}", c as u32);
        }
        for c in [' ', '~', '\u{a0}', '\u{2027}', '\u{202a}'] {
            assert_eq!(safe(&c.to_string()), c.to_string());
        }
        // One for one: the width of a column does not move.
        assert_eq!(safe("\u{1b}x\u{9b}").chars().count(), 3);
    }

    #[test]
    fn several_lines_keep_their_line_feeds_and_nothing_else() {
        assert_eq!(safe_lines("a\nb\n"), "a\nb\n");
        assert_eq!(
            safe_lines("one\u{1b}[2J\r\ntwo\u{2028}\n"),
            "one\u{fffd}[2J\u{fffd}\ntwo\u{fffd}\n"
        );
    }
}

//! Text for a terminal. Every string the human (non-JSON) views print
//! that comes from a file or from an argument (a theme, an entity, an
//! event name, a title, a quote, a path, a key, a corpus id, an error's
//! text) goes through [`safe`] on its way out, so no byte of an archive
//! file reaches the terminal as a control sequence (a colour change, a
//! cursor move, a rewritten title bar, a faked line) or as a character
//! that reorders or hides the text around it.
//!
//! The rule for line breaks: a string from a file or an argument is ONE
//! line of output at most. A line break in the output is always one the
//! program wrote itself, between strings that each went through [`safe`]:
//! the rows of a view, the lines of an error of several lines
//! ([`error_text`]). The two texts that are documents and not messages, a
//! citation and a registry entry, keep their own line feeds
//! ([`safe_lines`]).
//!
//! JSON output does not come here: `serde_json` writes every control
//! character below U+0020 as a `\u00XX` escape, and a JSON consumer is
//! not a terminal.

use std::borrow::Cow;

/// What stands in for a character a terminal could act on.
const REPLACEMENT: char = '\u{FFFD}';

/// Is `c` one a terminal may act on instead of showing, or one that is
/// not shown and changes how the text around it reads?
///
/// - The C0 controls but the tab (ESC, CR, LF, BEL, backspace … U+0000 to
///   U+001F), DEL (U+007F), the C1 controls (U+0080 to U+009F, CSI among
///   them), and the line and paragraph separators (U+2028, U+2029).
/// - The bidirectional controls, which reorder what is shown: the Arabic
///   letter mark (U+061C), the left-to-right and right-to-left marks
///   (U+200E, U+200F), the embeddings and overrides (U+202A to U+202E)
///   and the isolates (in U+2066 to U+2069).
/// - The invisible format characters: the zero width space (U+200B), the
///   word joiner and the invisible operators and deprecated format
///   characters after it (U+2060 to U+2069), the byte order mark / zero
///   width no-break space (U+FEFF), and the tag block (U+E0000 to
///   U+E007F).
///
/// Kept on purpose: the zero width joiner (U+200D) and non-joiner
/// (U+200C). Emoji sequences are built with the joiner (a family, a
/// profession, a flag with a rainbow), and Persian and several Indic
/// scripts are spelt with both; replacing them would break ordinary
/// text, and neither moves or reorders anything.
fn acts(c: char) -> bool {
    (c.is_control() && c != '\t')
        || matches!(
            c,
            '\u{2028}' | '\u{2029}'
                | '\u{061C}'
                | '\u{200B}'
                | '\u{200E}' | '\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{2069}'
                | '\u{FEFF}'
                | '\u{E0000}'..='\u{E007F}'
        )
}

/// `s` for a terminal: each character of [`acts`] replaced by U+FFFD,
/// one for one, so widths and alignment stay; everything else as it is.
/// A line break is replaced too: a string from a file or an argument is
/// one line of output at most.
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

/// [`safe`] on each line of a text that is a document of several lines: a
/// citation (a quote keeps its paragraphs) and a registry entry (`corpora
/// show`). The line feeds stay, every line between them goes through
/// [`safe`]. Not for a message: an error or a note is one line, or goes
/// through [`error_text`].
pub fn safe_lines(s: &str) -> String {
    s.split('\n').map(safe).collect::<Vec<_>>().join("\n")
}

/// An error for stderr: its causes, outermost first, joined by `: ` (as
/// `{:#}` writes them), each cause ONE line through [`safe`], whatever a
/// path, a key or a value named in it holds. The one error that is
/// several lines, [`scout_corpus::ErrorLines`] (the refusals of `scout
/// events seed`, a line per fault), is written a line per line of its
/// own, each through [`safe`]: the only line breaks are the ones the
/// program put between its lines.
pub fn error_text(e: &anyhow::Error) -> String {
    e.chain()
        .map(
            |cause| match cause.downcast_ref::<scout_corpus::ErrorLines>() {
                Some(lines) => lines
                    .0
                    .iter()
                    .map(|l| safe(l).into_owned())
                    .collect::<Vec<_>>()
                    .join("\n"),
                None => safe(&cause.to_string()).into_owned(),
            },
        )
        .collect::<Vec<_>>()
        .join(": ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const FFFD: &str = "\u{fffd}";

    #[test]
    fn controls_are_replaced_one_for_one_and_everything_else_is_kept() {
        // Untouched, and not copied.
        for keep in [
            "",
            "plain",
            "a\tb",
            "naïve — “quoted” 日本語 🙂",
            "a\u{a0}b",
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
            let want = if c == '\t' { "\t" } else { FFFD };
            assert_eq!(safe(&c.to_string()), want, "{:x}", c as u32);
        }
        for c in [' ', '~', '\u{a0}', '\u{2027}'] {
            assert_eq!(safe(&c.to_string()), c.to_string());
        }
        // One for one: the width of a column does not move.
        assert_eq!(safe("\u{1b}x\u{9b}").chars().count(), 3);
    }

    /// Every character of `range` is replaced, and the character before
    /// and after the range is kept.
    fn replaced(range: std::ops::RangeInclusive<u32>, what: &str) {
        for n in range.clone() {
            let c = char::from_u32(n).unwrap();
            assert_eq!(
                safe(&format!("a{c}b")),
                format!("a{FFFD}b"),
                "{what}: U+{n:04X}"
            );
        }
        for n in [range.start() - 1, range.end() + 1] {
            if matches!(n, 0x200C | 0x200D) {
                continue; // kept on purpose, see below
            }
            let Some(c) = char::from_u32(n) else { continue };
            if !acts(c) {
                assert_eq!(
                    safe(&c.to_string()),
                    c.to_string(),
                    "{what}: U+{n:04X} is kept"
                );
            }
        }
    }

    #[test]
    fn the_arabic_letter_mark_is_replaced() {
        replaced(0x061C..=0x061C, "ALM");
        assert_eq!(safe("\u{061B}\u{061D}"), "\u{061B}\u{061D}");
    }

    #[test]
    fn the_zero_width_space_and_the_direction_marks_are_replaced() {
        replaced(0x200B..=0x200B, "ZWSP");
        replaced(0x200E..=0x200F, "LRM, RLM");
        assert_eq!(safe("\u{200A}\u{2010}"), "\u{200A}\u{2010}");
    }

    #[test]
    fn the_bidirectional_embeddings_and_overrides_are_replaced() {
        replaced(0x202A..=0x202E, "LRE, RLE, PDF, LRO, RLO");
        // The classic spoof: a name shown reversed.
        assert_eq!(
            safe("invoice\u{202E}fdp.exe"),
            format!("invoice{FFFD}fdp.exe")
        );
        assert_eq!(safe("\u{202F}"), "\u{202F}");
    }

    #[test]
    fn the_word_joiner_the_invisible_operators_and_the_isolates_are_replaced() {
        replaced(0x2060..=0x2069, "WJ … PDI");
        assert_eq!(safe("\u{205F}\u{206A}"), "\u{205F}\u{206A}");
    }

    #[test]
    fn the_byte_order_mark_is_replaced() {
        replaced(0xFEFF..=0xFEFF, "BOM");
        assert_eq!(safe("\u{FEFF}{\"key\""), format!("{FFFD}{{\"key\""));
    }

    #[test]
    fn the_tag_block_is_replaced() {
        replaced(0xE0000..=0xE007F, "tags");
        // Text hidden in tag characters after a visible word.
        let hidden: String = "ok"
            .chars()
            .chain(
                "rm".chars()
                    .map(|c| char::from_u32(0xE0000 + c as u32).unwrap()),
            )
            .collect();
        assert_eq!(safe(&hidden), format!("ok{FFFD}{FFFD}"));
    }

    /// U+200D and U+200C are NOT replaced: emoji sequences and several
    /// scripts are written with them.
    #[test]
    fn ordinary_text_in_any_script_and_joined_emoji_are_unchanged() {
        for keep in [
            "Příliš žluťoučký kůň úpěl ďábelské ódy",
            "النص العربي مع أرقام ١٢٣ وعلامات، ترقيم؟",
            "עברית מימין לשמאל",
            // Persian with the zero width non-joiner (U+200C).
            "می\u{200C}خواهم",
            // Devanagari with the joiner (U+200D).
            "क्\u{200D}ष",
            // A family, a profession, a flag: joined with U+200D, with
            // variation selectors and skin tones.
            "👨\u{200D}👩\u{200D}👧\u{200D}👦",
            "👩🏽\u{200D}💻",
            "🏳\u{FE0F}\u{200D}🌈",
            "❤\u{FE0F} 🇨🇿 1\u{FE0F}\u{20E3}",
            "e\u{301} combining, ½, “quotes”, a\u{a0}b, soft\u{ad}hyphen",
        ] {
            assert!(
                matches!(safe(keep), Cow::Borrowed(s) if s == keep),
                "{keep:?}"
            );
        }
        assert!(!acts('\u{200C}') && !acts('\u{200D}'));
    }

    #[test]
    fn several_lines_keep_their_line_feeds_and_nothing_else() {
        assert_eq!(safe_lines("a\nb\n"), "a\nb\n");
        assert_eq!(
            safe_lines("one\u{1b}[2J\r\ntwo\u{2028}\n"),
            "one\u{fffd}[2J\u{fffd}\ntwo\u{fffd}\n"
        );
    }

    #[test]
    fn an_error_is_one_line_whatever_it_names() {
        let e = anyhow::anyhow!("missing\nFAKE-LINE: no such folder\r\u{1b}[2J");
        assert_eq!(
            error_text(&e),
            format!("missing{FFFD}FAKE-LINE: no such folder{FFFD}{FFFD}[2J")
        );
        // Every cause of a chain, joined as `{:#}` joins them.
        let e = e.context("read a\nb").context("build");
        assert_eq!(
            error_text(&e),
            format!("build: read a{FFFD}b: missing{FFFD}FAKE-LINE: no such folder{FFFD}{FFFD}[2J")
        );
        assert!(!error_text(&e).contains('\n'));
        let plain = anyhow::anyhow!("x: y").context("z");
        assert_eq!(error_text(&plain), format!("{plain:#}"));
    }

    #[test]
    fn an_error_of_several_lines_breaks_only_between_its_own_lines() {
        let lines = scout_corpus::ErrorLines(vec![
            "/out/a\nFAKE.md: cannot read the record (bad\u{202E} front)".into(),
            "nothing was written".into(),
        ]);
        let want = format!(
            "/out/a{FFFD}FAKE.md: cannot read the record (bad{FFFD} front)\nnothing was written"
        );
        assert_eq!(error_text(&anyhow::Error::new(lines.clone())), want);
        let wrapped = anyhow::Error::new(lines).context("seed\n");
        assert_eq!(error_text(&wrapped), format!("seed{FFFD}: {want}"));
    }
}

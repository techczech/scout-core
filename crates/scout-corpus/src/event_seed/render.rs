//! The bytes of a record file. Every value is written as a double-quoted
//! YAML scalar on one line, so no name can be read as anything but text:
//! not as a number, a null, a tag, an anchor, a comment or a second
//! document. An empty single value is written as nothing after the key and
//! an empty list as `[]`.

use crate::events::EventRecord;

/// `s` as a double-quoted YAML scalar. Printable ASCII and printable
/// non-ASCII characters stand as they are; `"`, `\`, every control
/// character, every whitespace character but the plain space (line and
/// paragraph separators among them) and the byte-order mark are escaped.
fn quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ' '..='~' => out.push(c),
            c if c >= '\u{a1}'
                && !c.is_whitespace()
                && !c.is_control()
                && !matches!(c, '\u{feff}' | '\u{fffe}' | '\u{ffff}') =>
            {
                out.push(c)
            }
            c => match c as u32 {
                n @ 0..=0xff => out.push_str(&format!("\\x{n:02X}")),
                n @ 0x100..=0xffff => out.push_str(&format!("\\u{n:04X}")),
                n => out.push_str(&format!("\\U{n:08X}")),
            },
        }
    }
    out.push('"');
    out
}

fn single(out: &mut String, key: &str, value: Option<&str>) {
    match value {
        Some(v) => out.push_str(&format!("{key}: {}\n", quoted(v))),
        None => out.push_str(&format!("{key}:\n")),
    }
}

fn list(out: &mut String, key: &str, values: &[String]) {
    if values.is_empty() {
        out.push_str(&format!("{key}: []\n"));
        return;
    }
    out.push_str(&format!("{key}:\n"));
    for v in values {
        out.push_str(&format!("  - {}\n", quoted(v)));
    }
}

/// The file text of `record`: the frontmatter in the spec's field order,
/// a blank line, then the description. What the reader gives back for it
/// equals `record` whenever each of its values is trimmed and not empty
/// (the seed checks that for every record before it writes any).
pub fn render_record(record: &EventRecord) -> String {
    let mut out = String::from("---\n");
    single(&mut out, "id", Some(&record.id));
    single(&mut out, "name", Some(&record.name));
    list(&mut out, "aliases", &record.aliases);
    single(&mut out, "start", record.start.as_deref());
    single(&mut out, "end", record.end.as_deref());
    list(&mut out, "members", &record.members);
    list(&mut out, "themes", &record.themes);
    single(&mut out, "origin", record.origin.as_deref());
    single(&mut out, "edited", record.edited.as_deref());
    single(&mut out, "merged_into", record.merged_into.as_deref());
    out.push_str("---\n\n");
    if !record.description.is_empty() {
        out.push_str(&record.description);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::parse_bytes;

    fn record(name: &str) -> EventRecord {
        EventRecord {
            id: "2026-09-x".into(),
            name: name.into(),
            aliases: vec![name.into(), format!("also {name}")],
            start: Some("2026-09-01".into()),
            end: Some("2026-09-30".into()),
            members: vec!["2026-09/E01".into()],
            themes: vec![name.into()],
            origin: Some("model".into()),
            edited: None,
            merged_into: None,
            description: format!("About {name}."),
        }
    }

    fn round_trips(r: &EventRecord) {
        let text = render_record(r);
        let back = parse_bytes(text.as_bytes(), &r.id).unwrap_or_else(|e| panic!("{text:?}: {e}"));
        assert_eq!(&back, r, "{text:?}");
    }

    #[test]
    fn the_layout_is_the_spec_frontmatter_with_empty_fields_written_empty() {
        let mut r = record("Gemini launch");
        r.aliases.clear();
        r.themes.clear();
        assert_eq!(
            render_record(&r),
            "---\nid: \"2026-09-x\"\nname: \"Gemini launch\"\naliases: []\nstart: \"2026-09-01\"\nend: \"2026-09-30\"\nmembers:\n  - \"2026-09/E01\"\nthemes: []\norigin: \"model\"\nedited:\nmerged_into:\n---\n\nAbout Gemini launch.\n"
        );
        r.description.clear();
        assert!(render_record(&r).ends_with("merged_into:\n---\n\n"));
        round_trips(&r);
    }

    #[test]
    fn awkward_names_come_back_from_the_reader_as_written() {
        let long = "long ".repeat(4000);
        for name in [
            "He said: \"it's #1\" — naïve café, 100% [sic] & more",
            "'single' and \"double\" and \\back\\slash\\",
            "key: value",
            "trailing colon:",
            "# not a comment",
            "a #hash in the middle",
            "- leading dash",
            "-",
            "---",
            "--- text",
            "...",
            "!tagged",
            "!!binary YQ==",
            "!!str x",
            "&anchor",
            "*alias",
            "? question",
            "| block",
            "> folded",
            "@at",
            "`tick`",
            "%directive",
            "[flow, list]",
            "{flow: map}",
            ", comma",
            "line one\nline two",
            "cr\r\nlf",
            "tab\there",
            "nul\u{0}inside",
            "bell\u{7}esc\u{1b}del\u{7f}",
            "next\u{85}line",
            "nbsp\u{a0}inside",
            "line\u{2028}sep\u{2029}para",
            "bom\u{feff}inside",
            "\u{feff}",
            "non\u{fffe}char\u{ffff}",
            "zero\u{200b}width\u{200d}joiner",
            "rtl \u{202e}override",
            "emoji 🎉 👩‍👩‍👧 🇨🇿",
            "日本語",
            "Ünïcode — alias",
            "combining e\u{301}",
            "astral 𝒜 \u{10ffff}",
            "123",
            "-1.5e3",
            "0x1F",
            "0o17",
            ".inf",
            ".NaN",
            "true",
            "False",
            "yes",
            "NO",
            "on",
            "null",
            "Null",
            "NULL",
            "~",
            "2026-10-11",
            "2026-10-11T12:00:00Z",
            "12:30:45",
            "<<",
            "=",
            "\\n not a newline",
            "\\x41 \\u0041 not escapes",
            "\"",
            "\\",
            "'",
            "a  two  spaces",
            long.trim(),
        ] {
            round_trips(&record(name));
        }
    }

    /// A small deterministic generator (xorshift): the same strings on
    /// every run.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn char(&mut self) -> char {
            const SPECIAL: &[char] = &[
                '"', '\\', '\'', ':', '#', '-', '!', '&', '*', '\n', '\r', '\t', ' ', '[', ']',
                '{', '}', ',', '|', '>', '%', '@', '`', '?', '~', '\u{0}', '\u{85}', '\u{a0}',
                '\u{2028}', '\u{2029}', '\u{feff}', '.', '0', 'n', 'u', 'l',
            ];
            loop {
                let n = self.next();
                let c = match n % 5 {
                    0 => Some(SPECIAL[(n >> 8) as usize % SPECIAL.len()]),
                    1 => char::from_u32((n >> 8) as u32 % 0x80),
                    2 => char::from_u32((n >> 8) as u32 % 0x3000),
                    3 => char::from_u32((n >> 8) as u32 % 0x1_0000),
                    _ => char::from_u32((n >> 8) as u32 % 0x11_0000),
                };
                if let Some(c) = c {
                    return c;
                }
            }
        }

        fn string(&mut self) -> String {
            let len = 1 + self.next() as usize % 24;
            (0..len).map(|_| self.char()).collect()
        }
    }

    #[test]
    fn any_trimmed_text_comes_back_from_the_reader_as_written() {
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let mut tried = 0;
        while tried < 20_000 {
            let name = rng.string().trim().to_string();
            let alias = rng.string().trim().to_string();
            if name.is_empty() || alias.is_empty() {
                continue;
            }
            tried += 1;
            let mut r = record(&name);
            r.aliases = vec![alias];
            r.description = rng.string().trim().to_string();
            round_trips(&r);
        }
    }
}

//! One record file: its bytes to an [`EventRecord`], or the reason the
//! reader skips it. The only implementation of the record format: the
//! index build reads every record through [`parse_bytes`], and `scout
//! events seed` passes every record it is about to write through it.
//!
//! Nothing here recurses over record content. The frontmatter is read by
//! pulling parser events one at a time ([`read_flat`]) and the pull stops
//! at the first value that nests too deep.

use super::EventRecord;
use std::collections::BTreeMap;
use yaml_rust2::parser::{Event, Parser};
use yaml_rust2::scanner::TScalarStyle;

/// Largest record file read, in bytes; a larger one is skipped.
pub const MAX_RECORD_BYTES: u64 = 1 << 20;
pub(super) const TOO_LARGE: &str = "larger than 1 MiB";
/// Deepest YAML nesting read in a record's frontmatter: the mapping (1),
/// a list value (2) and one level inside an ignored value (3). A record
/// that nests deeper is skipped.
pub const MAX_YAML_DEPTH: usize = 3;

/// The record id a file of the records folder must carry: its name
/// without `.md` (every trailing `.md`). `None` for a name that is not a
/// record file (no `.md` ending, or a leading `.`); such a file is not
/// looked at.
pub fn record_stem(file_name: &str) -> Option<&str> {
    (file_name.ends_with(".md") && !file_name.starts_with('.'))
        .then(|| file_name.trim_end_matches(".md"))
}

/// The record in a file's bytes, or why the reader skips it: the size cap,
/// UTF-8 and [`parse_record`] (which looks for control characters in the
/// whole file before it reads any of it). Every record the index reads has passed
/// through here, and so has every record `scout events seed` writes.
pub fn parse_bytes(bytes: &[u8], stem: &str) -> std::result::Result<EventRecord, String> {
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err(TOO_LARGE.to_string());
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => parse_record(text, stem),
        Err(_) => Err("not valid UTF-8".to_string()),
    }
}

/// A frontmatter value, as far as a record needs it.
#[derive(Debug, PartialEq, Eq)]
enum Val {
    Null,
    Str(String),
    List(Vec<String>),
    /// A mapping, or a list holding something other than scalars.
    Other,
}

/// Collects the top-level mapping of one YAML document: scalar and
/// scalar-list values by key. Aliases are refused (they can expand without
/// bound) and nothing below the second level is kept.
#[derive(Default)]
struct Flat {
    depth: usize,
    /// A value opened below [`MAX_YAML_DEPTH`]; reading stops there.
    too_deep: bool,
    top_is_map: bool,
    key: Option<String>,
    list: Option<Vec<String>>,
    other: bool,
    map: BTreeMap<String, Val>,
    err: Option<String>,
}

impl Flat {
    fn open(&mut self, sequence: bool) {
        if self.depth >= MAX_YAML_DEPTH {
            self.too_deep = true;
            return;
        }
        match self.depth {
            0 => self.top_is_map = !sequence,
            1 if self.key.is_none() => self.fail("a key is not a plain name"),
            1 if sequence => self.list = Some(Vec::new()),
            _ => self.other = true,
        }
        self.depth += 1;
    }

    fn close(&mut self) {
        self.depth = self.depth.saturating_sub(1);
        if self.depth == 1 {
            let v = match (self.other, self.list.take()) {
                (false, Some(l)) => Val::List(l),
                _ => Val::Other,
            };
            self.other = false;
            self.put(v);
        }
    }

    fn put(&mut self, v: Val) {
        if let Some(k) = self.key.take() {
            if self.map.insert(k.clone(), v).is_some() {
                self.fail(&format!("key {k:?} appears twice"));
            }
        }
    }

    fn fail(&mut self, why: &str) {
        if self.err.is_none() {
            self.err = Some(why.to_string());
        }
    }
}

fn is_null(v: &str, style: TScalarStyle) -> bool {
    style == TScalarStyle::Plain && matches!(v, "" | "~" | "null" | "Null" | "NULL")
}

impl Flat {
    fn on_event(&mut self, ev: Event) {
        match ev {
            Event::Alias(_) => self.fail("YAML aliases are not read"),
            Event::MappingStart(..) => self.open(false),
            Event::SequenceStart(..) => self.open(true),
            Event::MappingEnd | Event::SequenceEnd => self.close(),
            Event::Scalar(v, style, ..) => match self.depth {
                1 if self.key.is_none() => self.key = Some(v),
                1 if is_null(&v, style) => self.put(Val::Null),
                1 => self.put(Val::Str(v)),
                2 if !is_null(&v, style) => {
                    if let Some(l) = self.list.as_mut() {
                        l.push(v);
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
}

/// Why a record holding one of these characters is skipped.
pub(super) const CONTROL: &str = "control character in record";

/// A character no record file may hold anywhere: a C0 control other than
/// TAB, LF and CR, DEL, a C1 control (U+0080 to U+009F), the line and
/// paragraph separators U+2028 and U+2029, and U+FEFF. (CR is judged by
/// what follows it, in [`has_control`]; one leading U+FEFF is allowed
/// there.)
pub fn is_record_control(c: char) -> bool {
    matches!(c,
        '\0'..='\u{8}'
        | '\u{b}'
        | '\u{c}'
        | '\u{e}'..='\u{1f}'
        | '\u{7f}'..='\u{9f}'
        | '\u{2028}'
        | '\u{2029}'
        | '\u{feff}')
}

/// Does `text`, a whole record file, hold a character the reader refuses:
/// one of [`is_record_control`], or a CR that is not followed by LF? One
/// byte-order mark at the very start is not counted (it is cut off before
/// the frontmatter is looked for, as it always was); any other is.
///
/// After this check a line ends at LF and nowhere else, for `str::lines`
/// and for the YAML scanner alike (YAML also breaks a line at a bare CR
/// and stops reading at NUL), so the two cannot disagree about where a
/// line, a `...` marker or the input ends.
fn has_control(text: &str) -> bool {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let b = text.as_bytes();
    let bare_cr = (0..b.len()).any(|i| b[i] == b'\r' && b.get(i + 1) != Some(&b'\n'));
    bare_cr || text.chars().any(is_record_control)
}

/// Is there a document end marker in `front`: a line that is `...`, alone
/// or followed by whitespace? YAML reads such a line as the end of the
/// document wherever it stands (content of a block scalar is indented, so
/// an indented `...` is not one). Lines are `str::lines`'s, which are the
/// YAML scanner's once [`has_control`] has passed the file.
fn has_document_end(front: &str) -> bool {
    front.lines().any(|l| {
        l.strip_prefix("...")
            .is_some_and(|rest| rest.chars().next().is_none_or(char::is_whitespace))
    })
}

/// Read `front`, which must be exactly one YAML document, into a [`Flat`].
///
/// The whole frontmatter is read, to the end of the YAML stream: an error
/// anywhere in it, a second document (`--- x` on a line of its own) or a
/// document end marker (`...`, which would let anything stand after it)
/// refuses the record. So no part of a frontmatter block goes unchecked.
///
/// The parser is driven one event at a time (`Parser::next_token`, a state
/// machine over explicit stacks) and never through `Parser::load`, which
/// calls itself once per nesting level and overflows the stack on a deeply
/// nested value. The pull stops at the first value below
/// [`MAX_YAML_DEPTH`], so the rest of such a record is not parsed. (The
/// scanner may look ahead of the parser; it has no recursion and refuses
/// more than 255 open flow brackets itself, "recursion limit exceeded".)
fn read_flat(front: &str) -> std::result::Result<Flat, String> {
    if has_document_end(front) {
        return Err("bad YAML (a document end marker `...` in the frontmatter)".into());
    }
    let mut flat = Flat::default();
    let mut parser = Parser::new_from_str(front);
    let mut documents = 0;
    loop {
        let (ev, _) = parser.next_token().map_err(|e| format!("bad YAML ({e})"))?;
        match ev {
            Event::StreamEnd => return Ok(flat),
            Event::DocumentStart => {
                documents += 1;
                if documents > 1 {
                    return Err("bad YAML (more than one document in the frontmatter)".into());
                }
            }
            ev => flat.on_event(ev),
        }
        if flat.too_deep {
            return Err(format!(
                "bad YAML (nested deeper than {MAX_YAML_DEPTH} levels)"
            ));
        }
    }
}

/// Split `text` into its frontmatter block and body.
fn split_frontmatter(text: &str) -> Option<(&str, &str)> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let rest = text
        .strip_prefix("---\r\n")
        .or_else(|| text.strip_prefix("---\n"))?;
    let mut at = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            return Some((&rest[..at], &rest[at + line.len()..]));
        }
        at += line.len();
    }
    None
}

/// Parse one record file; `stem` is its file name without `.md`. The
/// whole file, body included, is refused when it holds a control character
/// ([`has_control`]); that is looked at first, before the frontmatter is
/// found or any YAML is read.
pub fn parse_record(text: &str, stem: &str) -> std::result::Result<EventRecord, String> {
    if has_control(text) {
        return Err(CONTROL.into());
    }
    let (front, body) = split_frontmatter(text).ok_or("no frontmatter")?;
    let flat = read_flat(front)?;
    if let Some(e) = flat.err {
        return Err(format!("bad YAML ({e})"));
    }
    if !flat.top_is_map {
        return Err("frontmatter is not a mapping".into());
    }
    let mut map = flat.map;
    let mut scalar = |k: &str| match map.remove(k) {
        None | Some(Val::Null) => Ok(None),
        Some(Val::Str(s)) if s.trim().is_empty() => Ok(None),
        Some(Val::Str(s)) => Ok(Some(s.trim().to_string())),
        Some(_) => Err(format!("{k} is not a single value")),
    };
    let id = scalar("id")?.ok_or("no id")?;
    if id != stem {
        return Err(format!("id {id:?} does not match the file name"));
    }
    let name = scalar("name")?.unwrap_or_else(|| id.clone());
    let (start, end) = (scalar("start")?, scalar("end")?);
    let (origin, edited) = (scalar("origin")?, scalar("edited")?);
    let merged_into = scalar("merged_into")?;
    let mut list = |k: &str| match map.remove(k) {
        None | Some(Val::Null) => Ok(Vec::new()),
        Some(Val::List(l)) => Ok(l
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()),
        Some(_) => Err(format!("{k} is not a list")),
    };
    Ok(EventRecord {
        aliases: list("aliases")?,
        members: list("members")?,
        themes: list("themes")?,
        id,
        name,
        start,
        end,
        origin,
        edited,
        merged_into,
        description: body.trim().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_values_are_read_as_yaml() {
        let text = "---\nid: 2026-09-a\nname: \"He said: \\\"it's\\\" \\u00e9\"\naliases:\n  - 'O''Reilly: x'\n  - plain # comment\nstart: 2026-09-01\nend:\nmembers: [2026-09/E01, \"2026-10/E04\"]\nthemes: []\norigin: model\nedited: ~\nmerged_into: null\nextra: {nested: [1, 2]}\n---\n\nBody: line.\n---\nmore\n";
        let r = parse_record(text, "2026-09-a").unwrap();
        assert_eq!(r.name, "He said: \"it's\" é");
        assert_eq!(r.aliases, vec!["O'Reilly: x", "plain"]);
        assert_eq!(r.start.as_deref(), Some("2026-09-01"));
        assert_eq!((r.end, r.edited, r.merged_into), (None, None, None));
        assert_eq!(r.members, vec!["2026-09/E01", "2026-10/E04"]);
        assert_eq!(r.description, "Body: line.\n---\nmore");
        // A quoted "null" is a name, not an empty field.
        let r = parse_record("---\nid: x\nname: 'null'\n---\n", "x").unwrap();
        assert_eq!(r.name, "null");
        // No name: the id stands in.
        assert_eq!(parse_record("---\nid: x\n---\n", "x").unwrap().name, "x");
    }

    #[test]
    fn malformed_records_are_refused_with_a_reason() {
        for (text, why) in [
            ("no frontmatter\n", "no frontmatter"),
            ("---\nid: x\n", "no frontmatter"),
            ("---\nid: [x\n---\n", "bad YAML"),
            ("---\n- id\n---\n", "not a mapping"),
            ("---\nname: X\n---\n", "no id"),
            ("---\nid: y\n---\n", "does not match the file name"),
            ("---\nid: X\n---\n", "does not match the file name"),
            (
                "---\nid: x\nmembers: 2026-09/E01\n---\n",
                "members is not a list",
            ),
            ("---\nid: x\nmembers: [[a]]\n---\n", "members is not a list"),
            ("---\nid: x\nname: [a]\n---\n", "name is not a single value"),
            ("---\nid: x\nid: x\n---\n", "appears twice"),
            (
                "---\na: &a [x]\nid: x\nmembers: *a\n---\n",
                "aliases are not read",
            ),
            ("---\n[a]: b\nid: x\n---\n", "not a plain name"),
        ] {
            let e = parse_record(text, "x").unwrap_err();
            assert!(e.contains(why), "{text:?}: {e}");
        }
    }

    #[test]
    fn the_whole_frontmatter_is_one_yaml_document_read_to_its_end() {
        for (front, why) in [
            // The review's case: a valid head, `...`, a broken tail.
            (
                "id: x\nmembers: [2026-09/E01]\n...\nname: [unterminated\n",
                "document end marker",
            ),
            ("id: x\n...\nid: y\n", "document end marker"),
            ("id: x\n...\ngarbage: [\n", "document end marker"),
            ("id: x\n...\n", "document end marker"),
            ("id: x\n... \n", "document end marker"),
            ("id: x\n...\t# done\n", "document end marker"),
            ("...\nid: x\n", "document end marker"),
            ("id: x\n--- second\n", "more than one document"),
            ("id: x\n--- {id: x}\n", "more than one document"),
            ("id: x\n--- [unterminated\n", "bad YAML"),
            ("id: x\nname: fine\nmembers: [a\n", "bad YAML"),
        ] {
            let text = format!("---\n{front}---\nbody\n");
            let e = parse_record(&text, "x").unwrap_err();
            assert!(e.contains(why), "{front:?}: {e}");
        }
        // Not markers: dots inside a value, indented in a block scalar, a
        // longer run of dots, and any line of the body.
        for front in [
            "id: x\nname: wait...\n",
            "id: x\nname: \"...\"\n",
            "id: x\nnote: |\n  one\n  ...\n  two\n",
            "id: x\nname: >\n  ....\n",
        ] {
            let text = format!("---\n{front}---\n...\n--- more\n");
            assert!(parse_record(&text, "x").is_ok(), "{front:?}");
        }
    }

    #[test]
    fn a_control_character_anywhere_in_the_file_refuses_the_record() {
        // The review's two cases: the YAML scanner stops at NUL, and it
        // ends a line at a bare CR where `str::lines` does not.
        for text in [
            "---\nid: x\nmembers: [2026-09/E01]\n\x00garbage: [\n---\n",
            "---\nid: x\rmembers: [2026-09/E01]\r...\n---\n",
            "---\nid: x\rname: y\n---\n",
            "---\nid: x\n---\nbody\rmore\n",
            "---\nid: x\n---\nbody\r",
            "---\nid: x\r\r\n---\n",
            "\r---\nid: x\n---\n",
            "---\nid: x\n---\nbody\u{0}\n",
            "---\nid: x\nname: a\u{1}b\n---\n",
            "---\nid: x\nname: a\u{b}b\n---\n",
            "---\nid: x\nname: a\u{c}b\n---\n",
            "---\nid: x\nname: a\u{1b}b\n---\n",
            "---\nid: x\nname: a\u{1f}b\n---\n",
            "---\nid: x\nname: a\u{7f}b\n---\n",
            "---\nid: x\nname: a\u{80}b\n---\n",
            "---\nid: x\nname: a\u{85}b\n---\n",
            "---\nid: x\nname: a\u{9f}b\n---\n",
            "---\nid: x\nname: a\u{2028}b\n---\n",
            "---\nid: x\nname: a\u{2029}b\n---\n",
            "---\nid: x\nname: \"a\u{2028}b\"\n---\n",
            "---\nid: x\nname: a\u{feff}b\n---\n",
            "---\nid: x\n---\nbody \u{feff}\n",
            "\u{feff}\u{feff}---\nid: x\n---\n",
            "---\nid: x\n# a comment \u{7}\n---\n",
        ] {
            let e = parse_record(text, "x").unwrap_err();
            assert_eq!(e, CONTROL, "{text:?}");
            assert_eq!(parse_bytes(text.as_bytes(), "x").unwrap_err(), CONTROL);
        }
        // Read: TAB, LF, CR before LF, one leading byte-order mark, the
        // characters next to the refused ranges, and any of them written
        // as an escape in a double-quoted value.
        for text in [
            "---\nid: x\nname: a\tb\n---\nbody\tmore\n",
            "---\r\nid: x\r\nname: y\r\n---\r\nbody\r\n",
            "\u{feff}---\nid: x\n---\n",
            "---\nid: x\nname: a\u{a0}b ~ \u{2027}\u{202a}\u{200b}\u{fffd}\n---\n",
        ] {
            assert!(parse_record(text, "x").is_ok(), "{text:?}");
        }
        let escaped = "---\nid: x\nname: \"a\\0b\\rc\\x7Fd\\x85e\\u2028f\\uFEFFg\"\n---\n";
        let r = parse_record(escaped, "x").unwrap();
        assert_eq!(r.name, "a\u{0}b\rc\u{7f}d\u{85}e\u{2028}f\u{feff}g");
    }

    /// Records nested 100,000 deep, in every shape that fits the size cap.
    fn deep_records() -> Vec<(&'static str, String)> {
        let n = 100_000;
        vec![
            (
                "block lists",
                format!("---\nid: x\nmembers:\n  {}a\n---\n", "- ".repeat(n)),
            ),
            (
                "flow lists",
                format!("---\nid: x\nmembers: {}\n---\n", "[".repeat(n)),
            ),
            (
                "closed flow lists",
                format!(
                    "---\nid: x\nextra: {}{}\n---\n",
                    "[".repeat(n),
                    "]".repeat(n)
                ),
            ),
            (
                "flow mappings",
                format!("---\nid: x\nextra: {}\n---\n", "{a: ".repeat(n)),
            ),
            (
                "lists of mappings",
                format!("---\nid: x\nextra:\n  {}a\n---\n", "- ? ".repeat(n)),
            ),
            ("at the top", format!("---\n{}a\n---\n", "- ".repeat(n))),
        ]
    }

    #[test]
    fn a_record_nested_100_000_deep_is_refused_without_deep_recursion() {
        // A 64 KiB stack: one frame per nesting level would not fit.
        let run = std::thread::Builder::new()
            .stack_size(64 * 1024)
            .spawn(|| {
                for (shape, text) in deep_records() {
                    assert!(text.len() as u64 <= MAX_RECORD_BYTES, "{shape}");
                    let e = parse_record(&text, "x").unwrap_err();
                    // Bare `[[[[…` is stopped by the scanner's own cap of
                    // 255 open brackets, reached while it looks ahead.
                    let why = if shape.contains("flow lists") {
                        "recursion limit exceeded"
                    } else {
                        "nested deeper than 3 levels"
                    };
                    assert!(e.contains(why), "{shape}: {e}");
                }
            })
            .unwrap();
        run.join().unwrap();
    }

    #[test]
    fn nesting_is_read_to_three_levels_and_no_deeper() {
        // Level 3 inside a key that is not a record field: ignored.
        let ok = "---\nid: x\nextra: {nested: [1, 2]}\nmore:\n  - [a, b]\n---\n";
        assert!(parse_record(ok, "x").is_ok());
        // Level 3 inside a record field: the field is wrongly typed.
        let e = parse_record("---\nid: x\nmembers: [[a]]\n---\n", "x").unwrap_err();
        assert!(e.contains("members is not a list"), "{e}");
        // Level 4 anywhere: the record is refused.
        for deep in [
            "---\nid: x\nextra: {a: {b: {c: 1}}}\n---\n",
            "---\nid: x\nextra: [[[a]]]\n---\n",
            "---\nid: x\nextra:\n  - - - a\n---\n",
        ] {
            let e = parse_record(deep, "x").unwrap_err();
            assert!(e.contains("nested deeper than 3 levels"), "{deep:?}: {e}");
        }
    }
}

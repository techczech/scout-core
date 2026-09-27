//! Index-time normalisation with an offset map back to the original text.
//!
//! Normalisation feeds counting and matching only (invariant 2). Every
//! normalised byte remembers the original byte span it came from, so a match
//! found in the normalised copy can be quoted from the ORIGINAL text.
//!
//! Stages, in order:
//! 1. drop lines matching the corpus boilerplate regexes (matched against the
//!    original line, without its newline);
//! 2. HTML entity decode, repeated so double-encoded `&amp;amp;` decodes fully;
//! 3. strip HTML tags and comments;
//! 4. strip Markdown link targets (`[text](url)` keeps `text`) and bare URLs;
//! 5. unify curly and straight apostrophes and quotes;
//! 6. Unicode NFC.

use regex::Regex;
use std::sync::OnceLock;
use unicode_normalization::char::canonical_combining_class;
use unicode_normalization::UnicodeNormalization;

/// Normalised text plus, for each normalised byte, the original byte span it
/// came from. Invariant: `starts.len() == ends.len() == text.len()`.
#[derive(Debug, Clone)]
pub struct Mapped {
    pub text: String,
    starts: Vec<usize>,
    ends: Vec<usize>,
    original_len: usize,
}

impl Mapped {
    /// The identity mapping over `s`.
    pub fn identity(s: &str) -> Mapped {
        Mapped {
            text: s.to_string(),
            starts: (0..s.len()).collect(),
            ends: (1..=s.len()).collect(),
            original_len: s.len(),
        }
    }

    /// Map a normalised byte range `[start, end)` to the original byte range.
    /// Both ends of the result fall on original char boundaries.
    pub fn original_range(&self, start: usize, end: usize) -> (usize, usize) {
        let s = self.original_offset(start);
        if end <= start {
            return (s, s);
        }
        (s, self.ends[end - 1])
    }

    /// Map a normalised byte offset to the original byte offset it came from.
    /// `text.len()` maps to the original length.
    pub fn original_offset(&self, pos: usize) -> usize {
        if pos >= self.starts.len() {
            self.original_len
        } else {
            self.starts[pos]
        }
    }
}

struct Builder<'a> {
    src: &'a Mapped,
    out: String,
    starts: Vec<usize>,
    ends: Vec<usize>,
}

impl<'a> Builder<'a> {
    fn new(src: &'a Mapped) -> Self {
        Builder {
            src,
            out: String::with_capacity(src.text.len()),
            starts: Vec::with_capacity(src.text.len()),
            ends: Vec::with_capacity(src.text.len()),
        }
    }

    /// Copy `src.text[start..end]` verbatim, keeping the fine-grained map.
    fn copy(&mut self, start: usize, end: usize) {
        if end <= start {
            return;
        }
        self.out.push_str(&self.src.text[start..end]);
        self.starts.extend_from_slice(&self.src.starts[start..end]);
        self.ends.extend_from_slice(&self.src.ends[start..end]);
    }

    /// Emit `s` as the replacement of `src.text[start..end]`.
    fn replace(&mut self, s: &str, start: usize, end: usize) {
        if s.is_empty() {
            return;
        }
        let (os, oe) = if end > start {
            (self.src.starts[start], self.src.ends[end - 1])
        } else {
            let p = self.src.original_offset(start);
            (p, p)
        };
        self.out.push_str(s);
        for _ in 0..s.len() {
            self.starts.push(os);
            self.ends.push(oe);
        }
    }

    fn finish(self) -> Mapped {
        Mapped {
            text: self.out,
            starts: self.starts,
            ends: self.ends,
            original_len: self.src.original_len,
        }
    }
}

/// Apply `f` to every match of `re`; unmatched text is copied verbatim.
/// `f` receives the builder and the captures and emits the replacement.
fn regex_stage(
    src: &Mapped,
    re: &Regex,
    mut f: impl FnMut(&mut Builder, &regex::Captures),
) -> Mapped {
    let mut b = Builder::new(src);
    let mut last = 0;
    for caps in re.captures_iter(&src.text) {
        let m = caps.get(0).unwrap();
        b.copy(last, m.start());
        f(&mut b, &caps);
        last = m.end();
    }
    b.copy(last, src.text.len());
    b.finish()
}

fn drop_boilerplate(src: &Mapped, rules: &[Regex]) -> Mapped {
    if rules.is_empty() {
        return src.clone();
    }
    let mut b = Builder::new(src);
    let mut pos = 0;
    for line in src.text.split_inclusive('\n') {
        let end = pos + line.len();
        let bare = line.strip_suffix('\n').unwrap_or(line);
        let bare = bare.strip_suffix('\r').unwrap_or(bare);
        if !rules.iter().any(|r| r.is_match(bare)) {
            b.copy(pos, end);
        }
        pos = end;
    }
    b.finish()
}

fn entity_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"&(#[0-9]{1,7}|#[xX][0-9a-fA-F]{1,6}|[A-Za-z][A-Za-z0-9]{1,31});").unwrap()
    })
}

fn named_entity(name: &str) -> Option<&'static str> {
    Some(match name {
        "amp" | "AMP" => "&",
        "lt" | "LT" => "<",
        "gt" | "GT" => ">",
        "quot" | "QUOT" => "\"",
        "apos" => "'",
        "nbsp" | "ensp" | "emsp" | "thinsp" => " ",
        "shy" | "zwj" | "zwnj" | "lrm" | "rlm" => "",
        "ndash" => "\u{2013}",
        "mdash" => "\u{2014}",
        "hellip" => "\u{2026}",
        "lsquo" => "\u{2018}",
        "rsquo" => "\u{2019}",
        "sbquo" => "\u{201A}",
        "ldquo" => "\u{201C}",
        "rdquo" => "\u{201D}",
        "bdquo" => "\u{201E}",
        "laquo" => "\u{00AB}",
        "raquo" => "\u{00BB}",
        "lsaquo" => "\u{2039}",
        "rsaquo" => "\u{203A}",
        "copy" => "\u{00A9}",
        "reg" => "\u{00AE}",
        "trade" => "\u{2122}",
        "middot" => "\u{00B7}",
        "bull" => "\u{2022}",
        "times" => "\u{00D7}",
        "divide" => "\u{00F7}",
        "deg" => "\u{00B0}",
        "euro" => "\u{20AC}",
        "pound" => "\u{00A3}",
        "sect" => "\u{00A7}",
        "para" => "\u{00B6}",
        "prime" => "\u{2032}",
        "Prime" => "\u{2033}",
        "rarr" => "\u{2192}",
        "larr" => "\u{2190}",
        "aacute" => "á",
        "Aacute" => "Á",
        "eacute" => "é",
        "Eacute" => "É",
        "iacute" => "í",
        "Iacute" => "Í",
        "oacute" => "ó",
        "Oacute" => "Ó",
        "uacute" => "ú",
        "Uacute" => "Ú",
        "yacute" => "ý",
        "Yacute" => "Ý",
        "ecaron" => "ě",
        "Ecaron" => "Ě",
        "scaron" => "š",
        "Scaron" => "Š",
        "ccaron" => "č",
        "Ccaron" => "Č",
        "rcaron" => "ř",
        "Rcaron" => "Ř",
        "zcaron" => "ž",
        "Zcaron" => "Ž",
        "uring" => "ů",
        "Uring" => "Ů",
        "auml" => "ä",
        "ouml" => "ö",
        "uuml" => "ü",
        "Auml" => "Ä",
        "Ouml" => "Ö",
        "Uuml" => "Ü",
        "szlig" => "ß",
        "egrave" => "è",
        "agrave" => "à",
        "ccedil" => "ç",
        _ => return None,
    })
}

fn decode_entity(body: &str) -> Option<String> {
    if let Some(num) = body.strip_prefix('#') {
        let cp = if let Some(hex) = num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
            u32::from_str_radix(hex, 16).ok()?
        } else {
            num.parse::<u32>().ok()?
        };
        let c = char::from_u32(cp)?;
        if c == '\u{a0}' {
            return Some(" ".into());
        }
        return Some(c.to_string());
    }
    named_entity(body).map(|s| s.to_string())
}

fn decode_entities_once(src: &Mapped) -> (Mapped, bool) {
    let mut changed = false;
    let out = regex_stage(src, entity_re(), |b, caps| {
        let m = caps.get(0).unwrap();
        match decode_entity(&caps[1]) {
            Some(s) => {
                changed = true;
                b.replace(&s, m.start(), m.end());
            }
            None => b.copy(m.start(), m.end()),
        }
    });
    (out, changed)
}

fn tag_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?s)<!--.*?-->|</?[A-Za-z][A-Za-z0-9:-]*(?:\s[^<>]*)?/?>|<[A-Za-z][A-Za-z0-9+.-]*:[^<>\s]*>").unwrap())
}

fn md_link_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"!?\[([^\[\]\n]*)\]\(\s*<?[^()\s<>]*(?:\([^()\s]*\)[^()\s<>]*)*>?(?:\s+(?:"[^"\n]*"|'[^'\n]*'))?\s*\)"#).unwrap())
}

fn url_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\b(?:https?://|www\.)[^\s<>()\[\]]+").unwrap())
}

fn unify_quote(c: char) -> Option<char> {
    match c {
        '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' | '\u{2032}' | '\u{02BC}' => Some('\''),
        '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{201F}' | '\u{2033}' => Some('"'),
        _ => None,
    }
}

/// Unify curly and straight apostrophes and double quotes, keeping the map.
pub fn unify_quotes(src: &Mapped) -> Mapped {
    let mut b = Builder::new(src);
    let mut last = 0;
    for (i, c) in src.text.char_indices() {
        if let Some(r) = unify_quote(c) {
            b.copy(last, i);
            let mut buf = [0u8; 4];
            b.replace(r.encode_utf8(&mut buf), i, i + c.len_utf8());
            last = i + c.len_utf8();
        }
    }
    b.copy(last, src.text.len());
    b.finish()
}

/// Unify quotes in a plain string (no map). Used by query parsing and later
/// by verify_quote.
pub fn unify_quotes_str(s: &str) -> String {
    s.chars().map(|c| unify_quote(c).unwrap_or(c)).collect()
}

fn nfc(src: &Mapped) -> Mapped {
    if unicode_normalization::is_nfc(&src.text) {
        return src.clone();
    }
    let mut b = Builder::new(src);
    let text = &src.text;
    let mut iter = text.char_indices().peekable();
    while let Some((gs, c)) = iter.next() {
        let mut ge = gs + c.len_utf8();
        while let Some(&(i, n)) = iter.peek() {
            if canonical_combining_class(n) != 0 {
                ge = i + n.len_utf8();
                iter.next();
            } else {
                break;
            }
        }
        let group = &text[gs..ge];
        if unicode_normalization::is_nfc(group) {
            b.copy(gs, ge);
        } else {
            let composed: String = group.nfc().collect();
            b.replace(&composed, gs, ge);
        }
    }
    b.finish()
}

/// Compile boilerplate rules. Invalid regexes are reported as errors.
pub fn compile_rules(rules: &[String]) -> anyhow::Result<Vec<Regex>> {
    rules
        .iter()
        .map(|r| Regex::new(r).map_err(|e| anyhow::anyhow!("bad boilerplate regex {r:?}: {e}")))
        .collect()
}

/// Normalise `original` with the corpus boilerplate `rules`.
pub fn normalize(original: &str, rules: &[Regex]) -> Mapped {
    let mut m = Mapped::identity(original);
    m = drop_boilerplate(&m, rules);
    for _ in 0..8 {
        let (next, changed) = decode_entities_once(&m);
        m = next;
        if !changed {
            break;
        }
    }
    m = regex_stage(&m, tag_re(), |b, caps| {
        let g = caps.get(0).unwrap();
        b.replace(" ", g.start(), g.end());
    });
    m = regex_stage(&m, md_link_re(), |b, caps| {
        let t = caps.get(1).unwrap();
        b.copy(t.start(), t.end());
    });
    m = regex_stage(&m, url_re(), |_, _| {});
    m = unify_quotes(&m);
    nfc(&m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_maps_bytes() {
        let m = Mapped::identity("příklad");
        assert_eq!(m.original_range(0, m.text.len()), (0, "příklad".len()));
    }

    #[test]
    fn replacement_maps_to_whole_source_span() {
        let orig = "a &amp; b";
        let m = normalize(orig, &[]);
        assert_eq!(m.text, "a & b");
        let (s, e) = m.original_range(2, 3);
        assert_eq!(&orig[s..e], "&amp;");
    }
}

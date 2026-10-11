//! The tokenizer of the `@scout/query` grammar: the normative rules are in
//! `docs/cli-json.md` ("Query grammar"); `packages/scout-query/src/scan.ts`
//! is the same algorithm line for line. No regular expression and no
//! built-in whitespace class is used here, so the two cannot drift.
//!
//! The query is split left to right into tokens separated by [`WS`]. A
//! construct is recognised ONLY at a token start (the start of the input or
//! right after whitespace):
//!
//! a. phrase: an opening quote (`"`, `“`, `„`) up to the next closing quote
//!    (any double-quote variant), optionally after one `-`; `\` before a
//!    double-quote variant or another `\` escapes it;
//! b. regex: `/body/flags` followed by whitespace or the end;
//! c. field: `name:value` with a known name; the value is quoted (as a
//!    phrase) or runs to the next whitespace;
//! d. otherwise a term: everything to the next whitespace.
//!
//! Whatever follows a closed phrase or quoted value without whitespace is a
//! plain term (rule d), never a construct.

/// The whitespace of the grammar: Unicode `White_Space`, listed.
pub const WS: &[char] = &[
    '\u{0009}', '\u{000A}', '\u{000B}', '\u{000C}', '\u{000D}', '\u{0020}', '\u{0085}', '\u{00A0}',
    '\u{1680}', '\u{2000}', '\u{2001}', '\u{2002}', '\u{2003}', '\u{2004}', '\u{2005}', '\u{2006}',
    '\u{2007}', '\u{2008}', '\u{2009}', '\u{200A}', '\u{2028}', '\u{2029}', '\u{202F}', '\u{205F}',
    '\u{3000}',
];

/// Double-quote variants: any of them closes a phrase and may be escaped.
pub const DOUBLE_QUOTES: &[char] = &['"', '\u{201C}', '\u{201D}', '\u{201E}', '\u{201F}'];

/// The quotes that open a phrase or a quoted value: `"`, `“`, `„`.
pub const OPEN_QUOTES: &[char] = &['"', '\u{201C}', '\u{201E}'];

pub fn is_ws(c: char) -> bool {
    WS.contains(&c)
}

fn is_dq(c: char) -> bool {
    DOUBLE_QUOTES.contains(&c)
}

/// `s` without leading and trailing [`WS`].
pub fn trim_ws(s: &str) -> &str {
    s.trim_matches(is_ws)
}

/// The [`WS`]-separated pieces of `s`.
pub fn split_ws(s: &str) -> impl Iterator<Item = &str> {
    s.split(is_ws).filter(|p| !p.is_empty())
}

/// One token of a query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    /// A quoted phrase (text unescaped, quotes removed); `negated` for `-"…"`.
    Phrase { negated: bool, text: String },
    /// A plain term as typed, a leading `-` included.
    Term(String),
    /// `/source/flags`.
    Regex { source: String, flags: String },
    /// A known field, its name lowercased; the value unquoted and unescaped.
    Field { name: String, value: String },
}

/// A quoted run starting at `chars[at]` (an opening quote): its unescaped
/// text and the index after the closing quote; `None` when it never closes.
fn read_quoted(chars: &[char], at: usize) -> Option<(String, usize)> {
    let mut out = String::new();
    let mut i = at + 1;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() && (chars[i + 1] == '\\' || is_dq(chars[i + 1])) {
            out.push(chars[i + 1]);
            i += 2;
        } else if is_dq(c) {
            return Some((out, i + 1));
        } else {
            out.push(c);
            i += 1;
        }
    }
    None
}

/// A regex token starting at `chars[at]` (a `/`): source, flags and the
/// index after it; `None` unless it closes, has a body and ends the token.
fn read_regex(chars: &[char], at: usize) -> Option<(String, String, usize)> {
    let mut source = String::new();
    let mut i = at + 1;
    loop {
        let c = *chars.get(i)?;
        if c == '\\' {
            source.push(c);
            source.push(*chars.get(i + 1)?);
            i += 2;
        } else if c == '/' {
            break;
        } else {
            source.push(c);
            i += 1;
        }
    }
    if source.is_empty() {
        return None;
    }
    i += 1;
    let mut flags = String::new();
    while i < chars.len() && chars[i].is_ascii_lowercase() {
        flags.push(chars[i]);
        i += 1;
    }
    (i == chars.len() || is_ws(chars[i])).then_some((source, flags, i))
}

/// Split `raw` into tokens; `is_field` says which lowercased names are
/// fields.
pub fn tokenize(raw: &str, is_field: impl Fn(&str) -> bool) -> Vec<Token> {
    let chars: Vec<char> = raw.chars().collect();
    let n = chars.len();
    let mut out = Vec::new();
    let mut i = 0;
    // After a closed quote: the rest of the run is a plain term.
    let rest_as_term = |i: usize, out: &mut Vec<Token>| -> usize {
        let mut j = i;
        while j < n && !is_ws(chars[j]) {
            j += 1;
        }
        if j > i {
            out.push(Token::Term(chars[i..j].iter().collect()));
        }
        j
    };
    while i < n {
        if is_ws(chars[i]) {
            i += 1;
            continue;
        }
        let mut end = i;
        while end < n && !is_ws(chars[end]) {
            end += 1;
        }
        // a. phrase, optionally negated
        let negated = chars[i] == '-' && i + 1 < n;
        let q = if negated { i + 1 } else { i };
        if OPEN_QUOTES.contains(&chars[q]) {
            if let Some((text, after)) = read_quoted(&chars, q) {
                out.push(Token::Phrase { negated, text });
                i = rest_as_term(after, &mut out);
                continue;
            }
        }
        // b. regex
        if chars[i] == '/' {
            if let Some((source, flags, after)) = read_regex(&chars, i) {
                out.push(Token::Regex { source, flags });
                i = after;
                continue;
            }
        }
        // c. field
        if let Some(colon) = (i..end).find(|&k| chars[k] == ':') {
            let name: String = chars[i..colon]
                .iter()
                .map(|c| c.to_ascii_lowercase())
                .collect();
            if is_field(&name) {
                let v = colon + 1;
                if v < n && OPEN_QUOTES.contains(&chars[v]) {
                    if let Some((value, after)) = read_quoted(&chars, v) {
                        out.push(Token::Field { name, value });
                        i = rest_as_term(after, &mut out);
                        continue;
                    }
                }
                out.push(Token::Field {
                    name,
                    value: chars[v..end].iter().collect(),
                });
                i = end;
                continue;
            }
        }
        // d. term
        out.push(Token::Term(chars[i..end].iter().collect()));
        i = end;
    }
    out
}

/// Write a field value so that the tokenizer reads it back unchanged: bare
/// when it is non-empty and holds no [`WS`], no double-quote variant and no
/// backslash; else in `"…"` with every double-quote variant and backslash
/// escaped by `\`.
pub fn quote_value(v: &str) -> String {
    if !v.is_empty() && !v.chars().any(|c| is_ws(c) || is_dq(c) || c == '\\') {
        return v.to_string();
    }
    let mut out = String::with_capacity(v.len() + 2);
    out.push('"');
    for c in v.chars() {
        if c == '\\' || is_dq(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

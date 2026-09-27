//! Rust port of the `@scout/query` search grammar (`packages/scout-query`).
//!
//! [`parse_search`] and [`date_range`] reproduce the TypeScript functions
//! `parseSearch` and `dateRange` exactly; both implementations are held to
//! ONE shared fixture file, `packages/scout-query/fixtures/grammar-cases.json`
//! (see `tests/fixtures.rs` and `packages/scout-query/grammar.test.ts`).
//!
//! Grammar: `OR` / `|`, `AND`, `-exclude`, `"phrase"`, `prefix*`,
//! `/regex/flags`, and field tokens `au: ti: ty: tag: co: after: before: y:`
//! (with their long forms and aliases), `in: lang: genre: source: zo: i:`
//! and the type shortcuts (`tw:`, `bo:`, `art:` …).
//!
//! [`parse_query`] adds what an engine needs beyond the TS output: the
//! positive free text as an OR of AND-groups of typed terms (FTS5 precedence:
//! AND binds tighter than OR), and the negated terms with their form.

use chrono::{Duration, NaiveDate};
use regex::{Captures, Regex};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::OnceLock;

mod stopwords;
pub use stopwords::{is_stopword, without_stopwords, STOPWORDS};

/// A `/regex/flags` token, as written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegexFilter {
    pub source: String,
    pub flags: String,
}

/// The parsed query, field for field the TS `ParsedQuery`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedQuery {
    pub fts: String,
    pub has_positive: bool,
    pub positive_terms: Vec<String>,
    pub negatives: Vec<String>,
    pub regexes: Vec<RegexFilter>,
    pub author: Option<String>,
    pub title: Option<String>,
    #[serde(rename = "type")]
    pub type_: Option<String>,
    pub tag: Option<String>,
    pub color: Option<String>,
    pub date: Option<String>,
    pub after: Option<String>,
    pub before: Option<String>,
    pub favorite: bool,
    pub zotero: bool,
    pub has_image: bool,
    /// `in:` corpus id(s), comma-separated.
    #[serde(rename = "in")]
    pub in_: Option<String>,
    pub lang: Option<String>,
    pub genre: Option<String>,
    /// `source:` as typed.
    pub source: Option<String>,
}

/// A half-open date range: `start` inclusive, `end` exclusive (ISO dates).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DateRange {
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub start: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub end: Option<String>,
}

/// How a free-text term matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TermForm {
    /// A bare word (`metaphor`).
    Word,
    /// `metaph*`: the text is the prefix without the star.
    Prefix,
    /// `"generative metaphor"`: the text is the phrase without quotes.
    Phrase,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FreeTerm {
    pub text: String,
    pub form: TermForm,
}

/// [`ParsedQuery`] plus the structure an engine executes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Query {
    pub parsed: ParsedQuery,
    /// The positive free text: an OR of AND-groups. Adjacent terms are
    /// ANDed; an explicit `OR` / `|` starts a new group; `AND` is the
    /// default and only documents intent. Dangling `OR`s are dropped.
    /// Unlike the TS `fts` string, three or more bare words are NOT turned
    /// into an OR (the corpus engine keeps T1's AND for plain text).
    pub any_of: Vec<Vec<FreeTerm>>,
    /// Negated terms (`-x`), in order.
    pub not: Vec<FreeTerm>,
}

impl Query {
    pub fn has_positive(&self) -> bool {
        self.any_of.iter().any(|g| !g.is_empty())
    }
}

const TYPE_SHORTCUTS: &[(&str, &str)] = &[
    ("art", "articles"),
    ("article", "articles"),
    ("articles", "articles"),
    ("bo", "books"),
    ("boo", "books"),
    ("book", "books"),
    ("books", "books"),
    ("tw", "tweets"),
    ("tweet", "tweets"),
    ("tweets", "tweets"),
    ("pdf", "pdfs"),
    ("pdfs", "pdfs"),
    ("pod", "podcasts"),
    ("podcast", "podcasts"),
    ("podcasts", "podcasts"),
    ("sup", "supplementals"),
    ("supplemental", "supplementals"),
    ("supplementals", "supplementals"),
];

/// Field keys in the TS `FILTER_TOKENS` order (the alternation order
/// matters for which key a regex match picks).
const FILTER_TOKENS: &[&str] = &[
    "author", "au", "title", "ti", "type", "ty", "tag", "date", "d", "year", "y", "after", "since",
    "from", "before", "until", "to", "i", "zo", "source", "color", "colour", "co", "in", "lang",
    "genre",
];

const OR_THRESHOLD: usize = 3;

fn token_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        let mut keys: Vec<&str> = FILTER_TOKENS.to_vec();
        keys.extend(TYPE_SHORTCUTS.iter().map(|(k, _)| *k));
        // JS `\b` without the `u` flag is an ASCII word boundary.
        Regex::new(&format!(
            r#"(?i)(?-u:\b)({}):("[^"]+"|\S*)"#,
            keys.join("|")
        ))
        .expect("token regex")
    })
}

fn regex_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"/((?:\\.|[^/\\])+)/([a-z]*)").expect("regex regex"))
}

fn free_token_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#""[^"]*"|\S+"#).expect("free token regex"))
}

fn word_star_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[\p{L}\p{N}_]+\*$").expect("prefix regex"))
}

fn word_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[\p{L}\p{N}_]+$").expect("word regex"))
}

fn shortcut(key: &str) -> Option<&'static str> {
    TYPE_SHORTCUTS
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, v)| *v)
}

/// The canonical field a key writes (TS `canonical`).
fn canonical(key: &str) -> String {
    match key {
        "au" => "author",
        "ti" => "title",
        "ty" => "type",
        "co" | "colour" => "color",
        "d" | "year" | "y" => "date",
        "since" | "from" => "after",
        "until" | "to" => "before",
        other => other,
    }
    .to_string()
}

fn set_field(p: &mut ParsedQuery, field: &str, value: String) {
    let slot = match field {
        "author" => &mut p.author,
        "title" => &mut p.title,
        "type" => &mut p.type_,
        "tag" => &mut p.tag,
        "color" => &mut p.color,
        "date" => &mut p.date,
        "after" => &mut p.after,
        "before" => &mut p.before,
        "in" => &mut p.in_,
        "lang" => &mut p.lang,
        "genre" => &mut p.genre,
        _ => return,
    };
    *slot = Some(value);
}

/// Remove one leading and one trailing `"` (TS `replace(/^"|"$/g, "")`).
fn strip_quotes(v: &str) -> String {
    let v = v.strip_prefix('"').unwrap_or(v);
    v.strip_suffix('"').unwrap_or(v).to_string()
}

fn esc_fts(term: &str) -> String {
    term.replace('"', "")
}

struct FreeText {
    fts: String,
    has_positive: bool,
    positive_terms: Vec<String>,
    negatives: Vec<String>,
    any_of: Vec<Vec<FreeTerm>>,
    not: Vec<FreeTerm>,
}

enum Seq {
    Term(String),
    Or,
    And,
}

fn build_free_text(text: &str, partial: bool) -> FreeText {
    let tokens: Vec<&str> = free_token_re()
        .find_iter(text)
        .map(|m| m.as_str())
        .collect();
    let mut positives: Vec<String> = Vec::new();
    let mut positive_terms: Vec<String> = Vec::new();
    let mut negatives: Vec<String> = Vec::new();
    let mut seq: Vec<Seq> = Vec::new();
    let mut explicit_op = false;
    let mut any_of: Vec<Vec<FreeTerm>> = vec![vec![]];
    let mut not: Vec<FreeTerm> = Vec::new();

    let is_op_word =
        |t: &str| t == "|" || t.eq_ignore_ascii_case("and") || t.eq_ignore_ascii_case("or");
    let has_content = tokens
        .iter()
        .filter(|t| !t.starts_with('"') && !is_op_word(t))
        .any(|t| !is_stopword(t.strip_prefix('-').unwrap_or(t)));

    for raw in &tokens {
        let upper = raw.to_uppercase();
        if *raw == "|" || upper == "OR" {
            explicit_op = true;
            seq.push(Seq::Or);
            any_of.push(vec![]);
            continue;
        }
        if upper == "AND" {
            explicit_op = true;
            seq.push(Seq::And);
            continue;
        }
        let mut tok: &str = raw;
        let mut negate = false;
        if tok.starts_with('-') && tok.len() > 1 {
            negate = true;
            tok = &tok[1..];
        }
        let quoted = tok.starts_with('"') && tok.ends_with('"');
        let is_bare = !quoted && !tok.ends_with('*');
        if has_content && is_bare && is_stopword(tok) {
            continue;
        }
        let (fts_term, clean_term, form) =
            if tok.starts_with('"') && tok.ends_with('"') && tok.len() >= 2 {
                let clean = esc_fts(&tok[1..tok.len() - 1]).trim().to_string();
                if clean.is_empty() {
                    continue;
                }
                (format!("\"{clean}\""), clean, TermForm::Phrase)
            } else if word_star_re().is_match(tok) {
                (
                    tok.to_string(),
                    tok[..tok.len() - 1].to_string(),
                    TermForm::Prefix,
                )
            } else {
                let clean = esc_fts(tok).trim().to_string();
                if clean.is_empty() {
                    continue;
                }
                let fts = if partial && word_re().is_match(&clean) {
                    format!("{clean}*")
                } else {
                    format!("\"{clean}\"")
                };
                (fts, clean, TermForm::Word)
            };
        if negate {
            not.push(FreeTerm {
                text: clean_term.clone(),
                form,
            });
            negatives.push(clean_term);
            continue;
        }
        any_of.last_mut().expect("group").push(FreeTerm {
            text: clean_term.clone(),
            form,
        });
        positives.push(fts_term.clone());
        positive_terms.push(clean_term);
        seq.push(Seq::Term(fts_term));
    }
    any_of.retain(|g| !g.is_empty());

    let has_positive = !positives.is_empty();
    if !has_positive {
        return FreeText {
            fts: String::new(),
            has_positive,
            positive_terms,
            negatives,
            any_of,
            not,
        };
    }
    let pos = if explicit_op {
        let parts: Vec<&str> = seq
            .iter()
            .filter_map(|s| match s {
                Seq::Or => Some("OR"),
                Seq::And => None,
                Seq::Term(t) => Some(t.as_str()),
            })
            .collect();
        let n = parts.len();
        parts
            .iter()
            .enumerate()
            .filter(|(i, part)| {
                **part != "OR"
                    || (*i > 0 && *i < n - 1 && parts[i - 1] != "OR" && parts[i + 1] != "OR")
            })
            .map(|(_, p)| *p)
            .collect::<Vec<_>>()
            .join(" ")
    } else if positives.len() >= OR_THRESHOLD {
        positives.join(" OR ")
    } else {
        positives.join(" ")
    };
    let mut fts = if negatives.is_empty() {
        pos
    } else {
        format!("({pos})")
    };
    for n in &negatives {
        fts.push_str(&format!(" NOT \"{}\"", esc_fts(n)));
    }
    FreeText {
        fts,
        has_positive,
        positive_terms,
        negatives,
        any_of,
        not,
    }
}

/// Parse a query, returning the TS-identical [`ParsedQuery`] and the
/// structured free text. `partial` is the TS partial (prefix) mode; it only
/// changes the `fts` string.
pub fn parse_with(raw: &str, partial: bool) -> Query {
    let mut parsed = ParsedQuery::default();
    let rest = regex_re().replace_all(raw, |c: &Captures| {
        parsed.regexes.push(RegexFilter {
            source: c[1].to_string(),
            flags: c[2].to_string(),
        });
        " ".to_string()
    });
    let mut shortcut_text: Vec<String> = Vec::new();
    let rest = token_re()
        .replace_all(&rest, |c: &Captures| {
            let key = c[1].to_ascii_lowercase();
            let value = strip_quotes(&c[2]);
            match key.as_str() {
                "i" => parsed.has_image = true,
                "zo" => parsed.zotero = true,
                "source" => {
                    if value.to_lowercase().contains("zotero") {
                        parsed.zotero = true;
                    }
                    parsed.source = Some(value);
                }
                _ => match shortcut(&key) {
                    Some(t) => {
                        parsed.type_ = Some(t.to_string());
                        if !value.is_empty() {
                            shortcut_text.push(value);
                        }
                    }
                    None => set_field(&mut parsed, &canonical(&key), value),
                },
            }
            " ".to_string()
        })
        .into_owned();
    let mut joined = vec![rest];
    joined.extend(shortcut_text);
    let free_text = joined
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let free = build_free_text(&free_text, partial);
    parsed.fts = free.fts;
    parsed.has_positive = free.has_positive;
    parsed.positive_terms = free.positive_terms;
    parsed.negatives = free.negatives;
    Query {
        parsed,
        any_of: free.any_of,
        not: free.not,
    }
}

/// TS `parseSearch(raw, partial)`.
pub fn parse_search(raw: &str, partial: bool) -> ParsedQuery {
    parse_with(raw, partial).parsed
}

/// [`parse_with`] in whole-word mode: what the corpus engine runs.
pub fn parse_query(raw: &str) -> Query {
    parse_with(raw, false)
}

fn ymd(y: i32, m: u32, d: u32) -> String {
    format!("{y:04}-{m:02}-{d:02}")
}

fn all_digits(s: &str, n: usize) -> bool {
    s.len() == n && s.bytes().all(|b| b.is_ascii_digit())
}

/// TS `dateRange`: a `y:` / `date:` value (`2016`, `2010-2015`, `2016-`,
/// `-2016`, `2016-05`, `2016-06-23`) as a half-open range. Like JS `Date`,
/// a day past the month's end (1–31) rolls into the next month.
pub fn date_range(value: &str) -> Option<DateRange> {
    let clean = value.trim();
    let parts: Vec<&str> = clean.split('-').collect();
    let range = |start: Option<String>, end: Option<String>| Some(DateRange { start, end });
    match parts.as_slice() {
        [y] if all_digits(y, 4) => {
            let y: i32 = y.parse().ok()?;
            range(Some(ymd(y, 1, 1)), Some(ymd(y + 1, 1, 1)))
        }
        [a, ""] if all_digits(a, 4) => range(Some(format!("{a}-01-01")), None),
        ["", b] if all_digits(b, 4) => {
            let b: i32 = b.parse().ok()?;
            range(None, Some(ymd(b + 1, 1, 1)))
        }
        [a, b] if all_digits(a, 4) && all_digits(b, 4) => {
            let (a, b): (i32, i32) = (a.parse().ok()?, b.parse().ok()?);
            if b < a {
                return None;
            }
            range(Some(ymd(a, 1, 1)), Some(ymd(b + 1, 1, 1)))
        }
        [y, m] if all_digits(y, 4) && all_digits(m, 2) => {
            let (y, m): (i32, u32) = (y.parse().ok()?, m.parse().ok()?);
            if !(1..=12).contains(&m) {
                return None;
            }
            let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
            range(Some(ymd(y, m, 1)), Some(ymd(ny, nm, 1)))
        }
        [y, m, d] if all_digits(y, 4) && all_digits(m, 2) && all_digits(d, 2) => {
            let (yi, mi, di): (i32, u32, u32) = (y.parse().ok()?, m.parse().ok()?, d.parse().ok()?);
            if !(1..=12).contains(&mi) || !(1..=31).contains(&di) {
                return None;
            }
            let day = NaiveDate::from_ymd_opt(yi, mi, 1)? + Duration::days(di as i64 - 1);
            let next = day + Duration::days(1);
            range(
                Some(clean.to_string()),
                Some(next.format("%Y-%m-%d").to_string()),
            )
        }
        _ => None,
    }
}

/// The distinct type shortcuts and their canonical (plural) types.
pub fn type_shortcuts() -> HashMap<&'static str, &'static str> {
    TYPE_SHORTCUTS.iter().copied().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn groups(q: &str) -> Vec<Vec<String>> {
        parse_query(q)
            .any_of
            .into_iter()
            .map(|g| g.into_iter().map(|t| t.text).collect())
            .collect()
    }

    #[test]
    fn any_of_is_or_of_and_groups() {
        assert_eq!(
            groups("generative metaphor theory"),
            vec![vec!["generative", "metaphor", "theory"]]
        );
        assert_eq!(
            groups("alpha beta OR gamma"),
            vec![vec!["alpha", "beta"], vec!["gamma"]]
        );
        assert_eq!(
            groups("alpha | beta AND gamma"),
            vec![vec!["alpha"], vec!["beta", "gamma"]]
        );
        assert_eq!(groups("OR alpha OR OR"), vec![vec!["alpha"]]);
        assert!(groups("-simile").is_empty());
    }

    #[test]
    fn terms_carry_their_form() {
        let q = parse_query("metaph* \"dead metaphor\" -simile* theory");
        let forms: Vec<TermForm> = q.any_of[0].iter().map(|t| t.form).collect();
        assert_eq!(
            forms,
            vec![TermForm::Prefix, TermForm::Phrase, TermForm::Word]
        );
        assert_eq!(
            q.not,
            vec![FreeTerm {
                text: "simile".into(),
                form: TermForm::Prefix
            }]
        );
    }
}

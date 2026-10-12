//! Rust port of the `@scout/query` search grammar (`packages/scout-query`).
//!
//! [`parse_search`] and [`date_range`] reproduce the TypeScript functions
//! `parseSearch` and `dateRange` exactly; both implementations are held to
//! ONE shared fixture file, `packages/scout-query/fixtures/grammar-cases.json`
//! (see `tests/fixtures.rs` and `packages/scout-query/grammar.test.ts`).
//!
//! Grammar: `OR` / `|`, `AND`, `-exclude`, `"phrase"`, `prefix*`,
//! `/regex/flags`, and field tokens `au: ti: ty: tag: co: after: before: y:`
//! (with their long forms and aliases), `in: lang: genre: source: zo: i:`,
//! the topic-layer fields `theme: entity: event: ai: month:` and the type
//! shortcuts (`tw:`, `bo:`, `art:` …). A repeated field keeps its last
//! value; `theme: entity: event: month:` also list every value given
//! (`themes`, `entities`, `events`, `months`), which is what the corpus
//! engine reads.
//!
//! [`parse_query`] adds what an engine needs beyond the TS output: the
//! positive free text as an OR of AND-groups of typed terms (FTS5 precedence:
//! AND binds tighter than OR), and the negated terms with their form.

use chrono::{Duration, NaiveDate};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::OnceLock;

pub mod scan;
mod stopwords;
pub use scan::{is_ws, quote_value, Token, DOUBLE_QUOTES, OPEN_QUOTES, WS};
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
    /// Topic layer (scout-corpus): `theme:` name, exact, case-insensitive.
    pub theme: Option<String>,
    /// `entity:` normalised name, exact, case-insensitive.
    pub entity: Option<String>,
    /// `event:` id such as `2026-09/E21`.
    pub event: Option<String>,
    /// `ai:` as typed (`true` / `false`).
    pub ai: Option<String>,
    /// `month:` the topic layer month, `YYYY-MM` or `YYYY`.
    pub month: Option<String>,
    /// Every `theme:` value, in query order. The four topic fields below
    /// may be repeated and every term counts: `theme:`, `entity:` and
    /// `event:` terms are ANDed, `month:` terms ORed (scout-corpus). The
    /// scalar fields above keep the LAST value, as every other field does.
    pub themes: Vec<String>,
    /// Every `entity:` value, in query order.
    pub entities: Vec<String>,
    /// Every `event:` value, in query order.
    pub events: Vec<String>,
    /// Every `month:` value, in query order.
    pub months: Vec<String>,
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
    "genre", "theme", "entity", "event", "ai", "month",
];

const OR_THRESHOLD: usize = 3;

fn is_field_name(name: &str) -> bool {
    FILTER_TOKENS.contains(&name) || TYPE_SHORTCUTS.iter().any(|(k, _)| *k == name)
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
    // The repeatable fields keep every value; the scalar keeps the last.
    match field {
        "theme" => p.themes.push(value.clone()),
        "entity" => p.entities.push(value.clone()),
        "event" => p.events.push(value.clone()),
        "month" => p.months.push(value.clone()),
        _ => {}
    }
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
        "theme" => &mut p.theme,
        "entity" => &mut p.entity,
        "event" => &mut p.event,
        "ai" => &mut p.ai,
        "month" => &mut p.month,
        _ => return,
    };
    *slot = Some(value);
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

/// A free-text token: a phrase, or a plain term as typed.
enum Free {
    Phrase { negated: bool, text: String },
    Term(String),
}

fn build_free_text(tokens: &[Free], partial: bool) -> FreeText {
    let mut positives: Vec<String> = Vec::new();
    let mut positive_terms: Vec<String> = Vec::new();
    let mut negatives: Vec<String> = Vec::new();
    let mut seq: Vec<Seq> = Vec::new();
    let mut explicit_op = false;
    let mut any_of: Vec<Vec<FreeTerm>> = vec![vec![]];
    let mut not: Vec<FreeTerm> = Vec::new();

    let is_or = |t: &str| t == "|" || t.eq_ignore_ascii_case("or");
    let is_and = |t: &str| t.eq_ignore_ascii_case("and");
    // Bare stopwords are dropped unless every bare word is one.
    let has_content = tokens.iter().any(|t| match t {
        Free::Term(t) if !is_or(t) && !is_and(t) => !is_stopword(t.strip_prefix('-').unwrap_or(t)),
        _ => false,
    });

    for token in tokens {
        let (fts_term, clean_term, form, negate) = match token {
            Free::Phrase { negated, text } => {
                let clean = scan::trim_ws(&esc_fts(text)).to_string();
                if clean.is_empty() {
                    continue;
                }
                (format!("\"{clean}\""), clean, TermForm::Phrase, *negated)
            }
            Free::Term(raw) => {
                if is_or(raw) {
                    explicit_op = true;
                    seq.push(Seq::Or);
                    any_of.push(vec![]);
                    continue;
                }
                if is_and(raw) {
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
                if has_content && !tok.ends_with('*') && is_stopword(tok) {
                    continue;
                }
                if word_star_re().is_match(tok) {
                    (
                        tok.to_string(),
                        tok[..tok.len() - 1].to_string(),
                        TermForm::Prefix,
                        negate,
                    )
                } else {
                    let clean = scan::trim_ws(&esc_fts(tok)).to_string();
                    if clean.is_empty() {
                        continue;
                    }
                    let fts = if partial && word_re().is_match(&clean) {
                        format!("{clean}*")
                    } else {
                        format!("\"{clean}\"")
                    };
                    (fts, clean, TermForm::Word, negate)
                }
            }
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
    let mut free: Vec<Free> = Vec::new();
    let mut shortcut_text: Vec<String> = Vec::new();
    for token in scan::tokenize(raw, is_field_name) {
        match token {
            Token::Phrase { negated, text } => free.push(Free::Phrase { negated, text }),
            Token::Term(t) => free.push(Free::Term(t)),
            Token::Regex { source, flags } => parsed.regexes.push(RegexFilter { source, flags }),
            Token::Field { name, value } => match name.as_str() {
                "i" => parsed.has_image = true,
                "zo" => parsed.zotero = true,
                "source" => {
                    if value.to_lowercase().contains("zotero") {
                        parsed.zotero = true;
                    }
                    parsed.source = Some(value);
                }
                _ => match shortcut(&name) {
                    Some(t) => {
                        parsed.type_ = Some(t.to_string());
                        shortcut_text.push(value);
                    }
                    None => set_field(&mut parsed, &canonical(&name), value),
                },
            },
        }
    }
    // A type shortcut's value is free text, after everything else
    // (`tw:karpathy` = tweets + the word karpathy).
    for v in &shortcut_text {
        free.extend(scan::split_ws(v).map(|p| Free::Term(p.to_string())));
    }
    let free = build_free_text(&free, partial);
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
    let clean = scan::trim_ws(value);
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
    fn repeated_topic_fields_keep_every_value_and_the_rest_keep_the_last() {
        let p = parse_search(
            "theme:A proof theme:\"B c\" entity:X ENTITY:Y event:e1 event:e2 month:2026 month:2026-02 in:one",
            false,
        );
        assert_eq!(p.themes, ["A", "B c"]);
        assert_eq!(p.entities, ["X", "Y"]);
        assert_eq!(p.events, ["e1", "e2"]);
        assert_eq!(p.months, ["2026", "2026-02"]);
        // The scalar is the last value, as it always was.
        assert_eq!(p.theme.as_deref(), Some("B c"));
        assert_eq!(p.entity.as_deref(), Some("Y"));
        assert_eq!(p.event.as_deref(), Some("e2"));
        assert_eq!(p.month.as_deref(), Some("2026-02"));
        assert_eq!(p.in_.as_deref(), Some("one"));
        assert_eq!(p.positive_terms, ["proof"]);

        // One term: the list holds it; none: the list is empty.
        let p = parse_search("theme:A", false);
        assert_eq!(
            (p.themes.as_slice(), p.theme.as_deref()),
            (&["A".to_string()][..], Some("A"))
        );
        assert!(p.entities.is_empty() && p.events.is_empty() && p.months.is_empty());
        // An empty value is a value of the list (the engine refuses it).
        assert_eq!(parse_search("theme: theme:A", false).themes, ["", "A"]);

        // Every other field: the last value, and no list.
        let p = parse_search(
            "au:a au:b ti:a ti:b ty:a ty:b tag:a tag:b co:a co:b y:2001 y:2002 after:2001 after:2002 \
             before:2001 before:2002 in:a in:b lang:a lang:b genre:a genre:b source:a source:b ai:true ai:false",
            false,
        );
        for (name, v) in [
            ("au", &p.author),
            ("ti", &p.title),
            ("ty", &p.type_),
            ("tag", &p.tag),
            ("co", &p.color),
            ("in", &p.in_),
            ("lang", &p.lang),
            ("genre", &p.genre),
            ("source", &p.source),
        ] {
            assert_eq!(v.as_deref(), Some("b"), "{name}");
        }
        assert_eq!(p.date.as_deref(), Some("2002"));
        assert_eq!(p.after.as_deref(), Some("2002"));
        assert_eq!(p.before.as_deref(), Some("2002"));
        assert_eq!(p.ai.as_deref(), Some("false"));
        assert!(p.themes.is_empty() && p.months.is_empty());
        // A field-shaped text in a phrase or behind `-` is no field.
        let p = parse_search("\"theme:A\" -theme:B theme:C", false);
        assert_eq!(p.themes, ["C"]);
        assert_eq!(p.negatives, ["theme:B"]);
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

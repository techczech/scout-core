//! Search with the `@scout/query` grammar (T2), over passages, grouped by
//! document. Quotes are original text.
//!
//! - Plain text keeps T1's behaviour: every bare word is prefix-matched and
//!   the words are ANDed (also for three or more words, unlike Highlight
//!   Scout's OR-by-coverage); bare stopwords are dropped unless every term is
//!   one; `"quoted phrases"` match consecutive tokens exactly.
//! - `OR` / `|` separates AND-groups (AND binds tighter); `AND` is the
//!   default; `-x` excludes passages with a token starting `x`; `x*` is an
//!   explicit prefix; `/regex/i` must match the passage's original text.
//! - Document fields: `in: after: before: y: lang: genre: source: zo: ty:
//!   au: ti:` ([`crate::filter::apply_query_fields`]). Passage fields:
//!   `tag:` (a highlight's tags or a document's topics) and `co:` (a
//!   highlight's colour).
//! - A query with no positive term but with filters or a regex lists every
//!   passage that passes them (score 0).
//! - Title- and topic-aware: a passage matches an AND-group when it holds at
//!   least one of the group's terms and every other term is in the passage
//!   or in its document's title, topics or summary. Its document score is
//!   its BM25 score times the share of the group's terms the passage itself
//!   holds (so document fields weigh less than passage text), plus the title
//!   score when the title matches the whole query. A document shows the
//!   passages covering the most terms first, ties broken by score.

use crate::cite::{self, CiteFormat, CiteSource};
use crate::filter::{DocFilter, DocInfo};
use crate::normalize;
use crate::registry::CorpusConfig;
use crate::stopwords::is_stopword;
use crate::tokenize::{tokenize, words, IdentityLemmatizer};
use anyhow::{anyhow, bail, Result};
use rusqlite::{params, Connection};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct SearchRequest {
    pub query: String,
    /// Maximum documents returned.
    pub limit: usize,
    /// Maximum passage hits kept per document.
    pub hits_per_doc: usize,
    /// Quote the whole passage instead of the hit sentence.
    pub whole_passage: bool,
}

impl SearchRequest {
    pub fn new(query: impl Into<String>) -> Self {
        SearchRequest {
            query: query.into(),
            limit: 20,
            hits_per_doc: 3,
            whole_passage: false,
        }
    }
}

/// One query term over index tokens: a prefix-matched word or an exact
/// phrase of tokens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "tokens", rename_all = "snake_case")]
pub enum Term {
    Prefix(String),
    Phrase(Vec<String>),
}

impl Term {
    fn fts(&self) -> String {
        match self {
            Term::Prefix(t) => format!("\"{}\"*", t.replace('"', "\"\"")),
            Term::Phrase(ts) => format!("\"{}\"", ts.join(" ").replace('"', "\"\"")),
        }
    }

    fn from_free(t: &scout_query::FreeTerm) -> Vec<Term> {
        let toks = words(&normalize::normalize(&t.text, &[]).text);
        match t.form {
            scout_query::TermForm::Phrase if !toks.is_empty() => vec![Term::Phrase(toks)],
            scout_query::TermForm::Phrase => vec![],
            // A bare word or `x*`: each token prefix-matched (a hyphenated
            // word gives several, ANDed), as in T1.
            _ => toks.into_iter().map(Term::Prefix).collect(),
        }
    }

    fn is_stop(&self) -> bool {
        matches!(self, Term::Prefix(t) if is_stopword(t))
    }
}

/// A query compiled for the index: positive AND-groups joined by OR, the
/// excluded terms, and the filters.
#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    pub any_of: Vec<Vec<Term>>,
    pub not: Vec<Term>,
    #[serde(rename = "filters")]
    pub filter: DocFilter,
    /// `in:` corpus ids.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub corpora: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub regexes: Vec<String>,
    /// Grammar fields this engine cannot apply (`i:`), named so a caller
    /// can say so.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ignored: Vec<String>,
    #[serde(skip)]
    compiled: Vec<regex::Regex>,
}

impl Plan {
    pub fn parse(query: &str) -> Result<Plan> {
        let q = scout_query::parse_query(&normalize::unify_quotes_str(query));
        let mut any_of: Vec<Vec<Term>> = q
            .any_of
            .iter()
            .map(|g| {
                let mut terms: Vec<Term> = Vec::new();
                for t in g.iter().flat_map(Term::from_free) {
                    if !terms.contains(&t) {
                        terms.push(t);
                    }
                }
                terms
            })
            .collect();
        // T1: bare stopwords go unless every positive term is one.
        if any_of.iter().flatten().any(|t| !t.is_stop()) {
            for g in any_of.iter_mut() {
                g.retain(|t| !t.is_stop());
            }
        }
        any_of.retain(|g| !g.is_empty());
        let not = q.not.iter().flat_map(Term::from_free).collect();
        let p = &q.parsed;
        let mut filter = DocFilter::default();
        let corpora = crate::filter::apply_query_fields(p, &mut filter)?;
        let mut compiled = Vec::new();
        let mut regexes = Vec::new();
        for r in &p.regexes {
            let mut b = regex::RegexBuilder::new(&r.source);
            b.case_insensitive(r.flags.contains('i'))
                .multi_line(r.flags.contains('m'))
                .dot_matches_new_line(r.flags.contains('s'));
            compiled.push(
                b.build()
                    .map_err(|e| anyhow!("bad regex /{}/{}: {e}", r.source, r.flags))?,
            );
            regexes.push(format!("/{}/{}", r.source, r.flags));
        }
        let mut ignored = Vec::new();
        if p.has_image {
            ignored.push("i:".to_string());
        }
        let nonempty = |v: &Option<String>| v.clone().filter(|v| !v.trim().is_empty());
        Ok(Plan {
            any_of,
            not,
            filter,
            corpora,
            tag: nonempty(&p.tag),
            color: nonempty(&p.color),
            regexes,
            ignored,
            compiled,
        })
    }

    /// Every positive term, in order (for locating the quote).
    pub fn terms(&self) -> Vec<Term> {
        let mut out: Vec<Term> = Vec::new();
        for t in self.any_of.iter().flatten() {
            if !out.contains(t) {
                out.push(t.clone());
            }
        }
        out
    }

    pub fn has_positive(&self) -> bool {
        !self.any_of.is_empty()
    }

    /// Anything that selects passages without a positive term.
    fn has_selector(&self) -> bool {
        !self.filter.is_empty()
            || self.tag.is_some()
            || self.color.is_some()
            || !self.compiled.is_empty()
            || !self.not.is_empty()
    }

    /// Every positive term of every group ORed, repeats kept, so a row's
    /// BM25 score equals its score under [`Plan::positive_fts`] whenever it
    /// matches that too; plus the exclusions.
    fn candidate_fts(&self) -> String {
        let pos = self
            .any_of
            .iter()
            .flatten()
            .map(Term::fts)
            .collect::<Vec<_>>()
            .join(" OR ");
        if self.not.is_empty() {
            return pos;
        }
        let mut out = format!("({pos})");
        for n in &self.not {
            out.push_str(&format!(" NOT {}", n.fts()));
        }
        out
    }

    /// The best share of an AND-group's terms that a passage holds itself,
    /// over the groups it satisfies (every term in the passage or in the
    /// document fields, at least one in the passage); `None` when it
    /// satisfies none. `in_passage` and `in_fields` answer per term of
    /// [`Plan::terms`].
    fn coverage(
        &self,
        terms: &[Term],
        in_passage: impl Fn(usize) -> bool,
        in_fields: impl Fn(usize) -> bool,
    ) -> Option<f64> {
        let mut best: Option<f64> = None;
        for g in &self.any_of {
            let idx: Vec<usize> = g
                .iter()
                .filter_map(|t| terms.iter().position(|x| x == t))
                .collect();
            let held = idx.iter().filter(|&&i| in_passage(i)).count();
            if held == 0 || !idx.iter().all(|&i| in_passage(i) || in_fields(i)) {
                continue;
            }
            let share = held as f64 / idx.len() as f64;
            if best.is_none_or(|b| share > b) {
                best = Some(share);
            }
        }
        best
    }

    fn positive_fts(&self) -> String {
        self.any_of
            .iter()
            .map(|g| {
                let inner = g.iter().map(Term::fts).collect::<Vec<_>>().join(" AND ");
                if self.any_of.len() > 1 && g.len() > 1 {
                    format!("({inner})")
                } else {
                    inner
                }
            })
            .collect::<Vec<_>>()
            .join(" OR ")
    }

    /// The FTS5 expression for the positive part and the exclusions.
    pub fn fts(&self) -> String {
        let pos = self.positive_fts();
        if self.not.is_empty() {
            return pos;
        }
        let mut out = format!("({pos})");
        for n in &self.not {
            out.push_str(&format!(" NOT {}", n.fts()));
        }
        out
    }

    fn passage_ok(
        &self,
        doc: &DocInfo,
        tags: &[String],
        color: Option<&str>,
        original: &str,
    ) -> bool {
        if let Some(t) = &self.tag {
            let hit = |s: &String| s.eq_ignore_ascii_case(t.trim());
            if !tags.iter().any(hit) && !doc.topics.iter().any(hit) {
                return false;
            }
        }
        if let Some(c) = &self.color {
            if !color.is_some_and(|x| x.eq_ignore_ascii_case(c.trim())) {
                return false;
            }
        }
        self.compiled.iter().all(|r| r.is_match(original))
    }
}

/// The positive terms of a query (T1's name for them).
pub fn parse_terms(query: &str) -> Vec<Term> {
    Plan::parse(query).map(|p| p.terms()).unwrap_or_default()
}

/// The corpus ids a query names with `in:` (empty when it names none).
pub fn query_corpora(query: &str) -> Result<Vec<String>> {
    Ok(Plan::parse(query)?.corpora)
}

#[derive(Debug, Clone, Serialize)]
pub struct Citation {
    pub markdown: String,
    pub plain: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PassageHit {
    /// `<corpus>:<doc key>:<line_start>`, stable across rebuilds (the id
    /// `scout cite` takes).
    pub passage_id: String,
    pub line_start: usize,
    pub line_end: usize,
    /// The source line the quote starts on.
    pub line: usize,
    /// Original text: the hit sentence, or the whole passage with `--passage`.
    pub quote: String,
    pub score: f64,
    /// Query terms this passage lacks that its document's title, topics or
    /// summary supply.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub field_terms: Vec<Term>,
    pub link: Option<String>,
    /// Highlights: the highlight's tags, colour and highlight date.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub saved_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocResult {
    pub corpus: String,
    /// The document key: the source path relative to the corpus root, plus
    /// `#<tweet id>` for a tweet.
    pub rel_path: String,
    /// The absolute source file.
    pub path: String,
    pub title: String,
    pub author: Option<String>,
    /// The public URL (frontmatter, a tweet's x.com URL, a work's `url`).
    pub public_url: Option<String>,
    pub date: Option<String>,
    pub date_display: Option<String>,
    /// `published`, or `saved` when a highlight's work has no publication
    /// date and `date` is the day it was first highlighted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_source: Option<String>,
    pub genre: Option<String>,
    pub lang: Option<String>,
    /// The document type (`tweet`, `article`, `book` …).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// The source system (`readwise`, `x`, `zotero`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// The shown passage's score times the share of the query terms it holds
    /// itself, plus the title score when the title matches.
    pub score: f64,
    /// `score` divided by the best score in the same corpus: the merge key
    /// across corpora (BM25 scores of different corpora do not compare).
    pub rank: f64,
    pub title_match: bool,
    pub passage_count: usize,
    pub hits: Vec<PassageHit>,
    pub citation: Citation,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchResults {
    pub schema_version: u32,
    pub query: String,
    pub terms: Vec<Term>,
    /// The compiled query: AND-groups, exclusions and filters.
    pub plan: Plan,
    pub corpora: Vec<String>,
    pub total_documents: usize,
    pub total_passages: usize,
    pub results: Vec<DocResult>,
}

fn round6(x: f64) -> f64 {
    (x * 1e6).round() / 1e6
}

/// A located quote inside a passage's original text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located {
    /// Byte range in the passage original.
    pub start: usize,
    pub end: usize,
    /// Byte offset (in the original) of the first matched token inside it.
    pub first_match: usize,
}

/// Find the sentence of `original` that matches the most distinct terms
/// (earliest on ties). Matching runs on the normalised copy; the returned
/// range is in the ORIGINAL text.
pub fn locate(
    original: &str,
    rules: &[regex::Regex],
    terms: &[Term],
    whole_passage: bool,
) -> Located {
    let m = normalize::normalize(original, rules);
    let toks = tokenize(&m.text, &IdentityLemmatizer, None);
    // (term index, original start offset)
    let mut matches: Vec<(usize, usize)> = Vec::new();
    for (ti, term) in terms.iter().enumerate() {
        match term {
            Term::Prefix(p) => {
                for t in &toks {
                    if t.text.starts_with(p.as_str()) {
                        matches.push((ti, m.original_offset(t.start)));
                    }
                }
            }
            Term::Phrase(ps) => {
                if ps.is_empty() || toks.len() < ps.len() {
                    continue;
                }
                for i in 0..=toks.len() - ps.len() {
                    if ps.iter().enumerate().all(|(k, p)| toks[i + k].text == *p) {
                        matches.push((ti, m.original_offset(toks[i].start)));
                    }
                }
            }
        }
    }
    let first_any = matches.iter().map(|(_, o)| *o).min();
    if whole_passage {
        let s = cite::skip_block_marker(original, 0, original.len());
        let e = original.trim_end().len().max(s);
        return Located {
            start: s,
            end: e,
            first_match: first_any.unwrap_or(s).max(s),
        };
    }
    let spans = cite::sentence_spans(original);
    let mut best: Option<(usize, usize, usize, usize)> = None; // (count, start, end, first)
    for (s, e) in &spans {
        let inside: Vec<&(usize, usize)> = matches
            .iter()
            .filter(|(_, o)| *o >= *s && *o < *e)
            .collect();
        let mut distinct: Vec<usize> = inside.iter().map(|(t, _)| *t).collect();
        distinct.sort();
        distinct.dedup();
        let first = inside.iter().map(|(_, o)| *o).min().unwrap_or(*s);
        let cand = (distinct.len(), *s, *e, first);
        match best {
            None => best = Some(cand),
            Some(b) if cand.0 > b.0 => best = Some(cand),
            _ => {}
        }
    }
    match best {
        Some((_, s, e, first)) => {
            let s2 = cite::skip_block_marker(original, s, e);
            Located {
                start: s2,
                end: e,
                first_match: first.max(s2),
            }
        }
        None => Located {
            start: 0,
            end: original.len(),
            first_match: 0,
        },
    }
}

struct Row {
    id: i64,
    rel_path: String,
    line_start: i64,
    score: f64,
    /// Share of its AND-group's terms the passage holds itself (1 when it
    /// needs no document field).
    cover: f64,
    /// Indexes into the plan's terms supplied by document fields.
    field_terms: Vec<usize>,
}

/// The rowids of an FTS table matching one term.
fn fts_rowids(conn: &Connection, table: &str, term: &Term) -> Result<BTreeSet<i64>> {
    let mut st =
        conn.prepare_cached(&format!("SELECT rowid FROM {table} WHERE {table} MATCH ?1"))?;
    let ids = st
        .query_map([term.fts()], |r| r.get::<_, i64>(0))?
        .collect::<rusqlite::Result<BTreeSet<i64>>>()?;
    Ok(ids)
}

/// The documents whose title, topics or summary hold one term (none on an
/// index built before the fields table existed).
fn field_docs(conn: &Connection, term: &Term) -> Result<BTreeSet<String>> {
    let mut st =
        conn.prepare_cached("SELECT DISTINCT rel_path FROM fields_fts WHERE fields_fts MATCH ?1")?;
    let docs = st
        .query_map([term.fts()], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<BTreeSet<String>>>()?;
    Ok(docs)
}

fn has_table(conn: &Connection, name: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE name = ?1",
        [name],
        |r| r.get::<_, i64>(0),
    )? > 0)
}

/// The sentence holding the first match of a regex (regex-only queries).
fn locate_regex(original: &str, re: &regex::Regex, whole_passage: bool) -> Located {
    let at = re.find(original).map(|m| m.start()).unwrap_or(0);
    let spans = cite::sentence_spans(original);
    let span = if whole_passage {
        None
    } else {
        spans.iter().find(|(s, e)| at >= *s && at < *e).copied()
    };
    let (s, e) = span.unwrap_or((0, original.trim_end().len()));
    let s2 = cite::skip_block_marker(original, s, e);
    Located {
        start: s2,
        end: e,
        first_match: at.max(s2),
    }
}

/// The citation metadata of a document.
fn cite_source<'a>(cfg: &CorpusConfig, d: &'a DocInfo, link: Option<&'a str>) -> CiteSource<'a> {
    CiteSource {
        style: cfg.cite_style(),
        author: d.author.as_deref(),
        title: &d.title,
        date: d.date.as_deref(),
        link,
        public_url: d.public_url.as_deref(),
        handle: d.handle.as_deref(),
        date_source: d.date_source.as_deref(),
    }
}

/// Both citation forms of an original-text quote from a document.
pub fn citation(cfg: &CorpusConfig, d: &DocInfo, quote: &str, link: Option<&str>) -> Citation {
    let src = cite_source(cfg, d, link);
    Citation {
        markdown: cite::render_citation(quote, &src, CiteFormat::Markdown),
        plain: cite::render_citation(quote, &src, CiteFormat::Plain),
    }
}

/// Search one corpus's index.
pub fn search_corpus(
    conn: &Connection,
    cfg: &CorpusConfig,
    req: &SearchRequest,
) -> Result<SearchResults> {
    let plan = Plan::parse(&req.query)?;
    let terms = plan.terms();
    let mut res = SearchResults {
        schema_version: SCHEMA_VERSION,
        query: req.query.clone(),
        terms: terms.clone(),
        plan: plan.clone(),
        corpora: vec![cfg.id.clone()],
        total_documents: 0,
        total_passages: 0,
        results: vec![],
    };
    if !plan.has_positive() && !plan.has_selector() {
        return Ok(res);
    }
    let docs = crate::filter::filtered_docs(conn, &plan.filter)?;
    let need_passage = plan.tag.is_some() || plan.color.is_some() || !plan.compiled.is_empty();
    let cols =
        "p.id, p.rel_path, p.line_start, p.tags, p.color, CASE WHEN ?2 THEN p.original ELSE '' END";
    let mut rows: Vec<Row> = Vec::new();
    let mut keep = |row: Row, tags: String, color: Option<String>, original: String| {
        let Some(doc) = docs.get(&row.rel_path) else {
            return;
        };
        if need_passage {
            let tags: Vec<String> = serde_json::from_str(&tags).unwrap_or_default();
            if !plan.passage_ok(doc, &tags, color.as_deref(), &original) {
                return;
            }
        }
        rows.push(row);
    };
    if plan.has_positive() {
        // Which passages hold each term, and which documents' fields do.
        let in_pass: Vec<BTreeSet<i64>> = terms
            .iter()
            .map(|t| fts_rowids(conn, "passages_fts", t))
            .collect::<Result<_>>()?;
        let in_fields: Vec<BTreeSet<String>> = if has_table(conn, "fields_fts")? {
            terms
                .iter()
                .map(|t| field_docs(conn, t))
                .collect::<Result<_>>()?
        } else {
            vec![BTreeSet::new(); terms.len()]
        };
        let mut st = conn.prepare(&format!(
            "SELECT {cols}, bm25(passages_fts) FROM passages_fts JOIN passages p ON p.id = passages_fts.rowid
             WHERE passages_fts MATCH ?1"
        ))?;
        let mut q = st.query(params![plan.candidate_fts(), need_passage])?;
        while let Some(r) = q.next()? {
            let id: i64 = r.get(0)?;
            let rel: String = r.get(1)?;
            let holds = |i: usize| in_pass[i].contains(&id);
            let supplied = |i: usize| in_fields[i].contains(&rel);
            let Some(cover) = plan.coverage(&terms, holds, supplied) else {
                continue;
            };
            let field_terms = (0..terms.len())
                .filter(|&i| !holds(i) && supplied(i))
                .collect();
            keep(
                Row {
                    id,
                    rel_path: rel,
                    line_start: r.get(2)?,
                    score: round6(-r.get::<_, f64>(6)?),
                    cover,
                    field_terms,
                },
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            );
        }
    } else {
        // Filters or a regex alone: every passage that passes them.
        let mut excluded: BTreeSet<i64> = BTreeSet::new();
        if !plan.not.is_empty() {
            let any = plan
                .not
                .iter()
                .map(Term::fts)
                .collect::<Vec<_>>()
                .join(" OR ");
            let mut st =
                conn.prepare("SELECT rowid FROM passages_fts WHERE passages_fts MATCH ?1")?;
            for id in st.query_map([any], |r| r.get::<_, i64>(0))? {
                excluded.insert(id?);
            }
        }
        let mut st = conn.prepare(&format!(
            "SELECT {cols} FROM passages p WHERE ?1 = ?1 ORDER BY p.rel_path, p.line_start"
        ))?;
        let mut q = st.query(params![0, need_passage])?;
        while let Some(r) = q.next()? {
            let id: i64 = r.get(0)?;
            if excluded.contains(&id) {
                continue;
            }
            keep(
                Row {
                    id,
                    rel_path: r.get(1)?,
                    line_start: r.get(2)?,
                    score: 0.0,
                    cover: 1.0,
                    field_terms: vec![],
                },
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            );
        }
    }
    res.total_passages = rows.len();

    // Group by document; order passages by coverage desc, score desc, line.
    let mut by_doc: BTreeMap<String, Vec<Row>> = BTreeMap::new();
    for r in rows {
        by_doc.entry(r.rel_path.clone()).or_default().push(r);
    }
    let mut groups: Vec<(String, Vec<Row>)> = by_doc.into_iter().collect();
    for (_, ps) in groups.iter_mut() {
        ps.sort_by(|a, b| {
            b.cover
                .total_cmp(&a.cover)
                .then(b.score.total_cmp(&a.score))
                .then(a.line_start.cmp(&b.line_start))
        });
    }
    // A document whose title matches the positive query gets its title's
    // score added to its best passage score.
    let mut title_scores: BTreeMap<String, f64> = BTreeMap::new();
    if plan.has_positive() {
        let mut tq = conn.prepare(
            "SELECT rel_path, bm25(titles_fts) FROM titles_fts WHERE titles_fts MATCH ?1",
        )?;
        for row in tq.query_map([plan.positive_fts()], |r| {
            Ok((r.get::<_, String>(0)?, round6(-r.get::<_, f64>(1)?)))
        })? {
            let (k, v) = row?;
            title_scores.insert(k, v);
        }
    }
    let doc_score = |rel: &str, ps: &[Row]| {
        round6(ps[0].score * ps[0].cover + title_scores.get(rel).copied().unwrap_or(0.0))
    };
    groups.sort_by(|a, b| {
        doc_score(&b.0, &b.1)
            .total_cmp(&doc_score(&a.0, &a.1))
            .then(a.0.cmp(&b.0))
    });
    res.total_documents = groups.len();
    let top = groups
        .first()
        .map(|(r, ps)| doc_score(r, ps))
        .unwrap_or(0.0);

    let rules = normalize::compile_rules(&cfg.boilerplate)?;
    let mut get_pass = conn
        .prepare("SELECT line_end, original, tags, color, saved_at FROM passages WHERE id = ?1")?;
    for (rel, ps) in groups.into_iter().take(req.limit) {
        let d = &docs[&rel];
        let abs = cfg.source_path(&rel);
        let mut hits = Vec::new();
        for p in ps.iter().take(req.hits_per_doc.max(1)) {
            let (line_end, original, tags, color, saved_at): (
                i64,
                String,
                String,
                Option<String>,
                Option<String>,
            ) = get_pass.query_row(params![p.id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })?;
            let loc = match (terms.is_empty(), plan.compiled.first()) {
                (true, Some(re)) => locate_regex(&original, re, req.whole_passage),
                _ => locate(&original, &rules, &terms, req.whole_passage),
            };
            let line = p.line_start as usize + original[..loc.first_match].matches('\n').count();
            hits.push(PassageHit {
                passage_id: format!("{}:{}:{}", cfg.id, rel, p.line_start),
                line_start: p.line_start as usize,
                line_end: line_end as usize,
                line,
                quote: original[loc.start..loc.end].to_string(),
                score: p.score,
                field_terms: p.field_terms.iter().map(|&i| terms[i].clone()).collect(),
                link: cfg.link_for(&rel, line),
                tags: serde_json::from_str(&tags).unwrap_or_default(),
                color,
                saved_at,
            });
        }
        let best = &hits[0];
        let citation = citation(cfg, d, &best.quote, best.link.as_deref());
        let score = doc_score(&rel, &ps);
        res.results.push(DocResult {
            corpus: cfg.id.clone(),
            rel_path: rel.clone(),
            path: abs.display().to_string(),
            date_display: d.date.as_deref().and_then(cite::format_date),
            title: d.title.clone(),
            author: d.author.clone(),
            public_url: d.public_url.clone(),
            date: d.date.clone(),
            date_source: d.date_source.clone(),
            genre: d.genre.clone(),
            lang: d.lang.clone(),
            kind: d.kind.clone(),
            source: d.source.clone(),
            score,
            rank: if top > 0.0 { round6(score / top) } else { 0.0 },
            title_match: title_scores.contains_key(&rel),
            passage_count: ps.len(),
            hits,
            citation,
        });
    }
    Ok(res)
}

/// Merge per-corpus results: rank desc (score / the corpus's best score),
/// then score desc, corpus, path. With one corpus this is score order.
pub fn merge(query: &str, parts: Vec<SearchResults>, limit: usize) -> Result<SearchResults> {
    let plan = Plan::parse(query)?;
    let mut out = SearchResults {
        schema_version: SCHEMA_VERSION,
        query: query.to_string(),
        terms: plan.terms(),
        plan,
        corpora: vec![],
        total_documents: 0,
        total_passages: 0,
        results: vec![],
    };
    for p in parts {
        out.corpora.extend(p.corpora);
        out.total_documents += p.total_documents;
        out.total_passages += p.total_passages;
        out.results.extend(p.results);
    }
    out.results.sort_by(|a, b| {
        b.rank
            .total_cmp(&a.rank)
            .then(b.score.total_cmp(&a.score))
            .then(a.corpus.cmp(&b.corpus))
            .then(a.rel_path.cmp(&b.rel_path))
    });
    out.results.truncate(limit);
    Ok(out)
}

/// Helper for tests and callers holding a path.
pub fn quote_is_original(abs_path: &Path, hit: &PassageHit) -> Result<bool> {
    let text = crate::markdown::read_source(abs_path)?;
    let lines: Vec<&str> = text
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect();
    let block = lines[hit.line_start - 1..hit.line_end].join("\n");
    Ok(block.contains(&hit.quote))
}

/// A parsed passage id: `<corpus>:<doc key>:<line_start>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PassageId {
    pub corpus: String,
    pub doc_key: String,
    pub line_start: usize,
}

impl std::str::FromStr for PassageId {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        let bad = || anyhow!("bad passage id {s:?}: expected <corpus>:<path>:<line>");
        let (corpus, rest) = s.trim().split_once(':').ok_or_else(bad)?;
        let (key, line) = rest.rsplit_once(':').ok_or_else(bad)?;
        if corpus.is_empty() || key.is_empty() {
            return Err(bad());
        }
        Ok(PassageId {
            corpus: corpus.to_string(),
            doc_key: key.to_string(),
            line_start: line.parse().map_err(|_| bad())?,
        })
    }
}

/// Raised when a passage id names no indexed passage.
#[derive(Debug)]
pub struct PassageNotFound(pub String);

impl std::fmt::Display for PassageNotFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "no passage {:?} in the index", self.0)
    }
}
impl std::error::Error for PassageNotFound {}

/// `scout cite`: one passage, quoted whole, with both citation forms.
#[derive(Debug, Clone, Serialize)]
pub struct CitedPassage {
    pub schema_version: u32,
    pub passage_id: String,
    pub corpus: String,
    pub rel_path: String,
    pub path: String,
    pub line_start: usize,
    pub line_end: usize,
    /// The passage's original text (block markers at its start skipped).
    pub quote: String,
    pub title: String,
    pub author: Option<String>,
    pub date: Option<String>,
    pub date_display: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_source: Option<String>,
    pub link: Option<String>,
    pub public_url: Option<String>,
    pub citation: Citation,
}

/// Look up a passage by id in its corpus's index and cite it.
pub fn cite_passage(conn: &Connection, cfg: &CorpusConfig, id: &PassageId) -> Result<CitedPassage> {
    if id.corpus != cfg.id {
        bail!(
            "passage {:?} belongs to corpus {:?}, not {:?}",
            id.doc_key,
            id.corpus,
            cfg.id
        );
    }
    let pid = format!("{}:{}:{}", id.corpus, id.doc_key, id.line_start);
    let row: Option<(i64, String)> = conn
        .query_row(
            "SELECT line_end, original FROM passages WHERE rel_path = ?1 AND line_start = ?2",
            params![id.doc_key, id.line_start as i64],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            e => Err(e),
        })?;
    let Some((line_end, original)) = row else {
        return Err(PassageNotFound(pid).into());
    };
    let docs = crate::filter::load_docs(conn)?;
    let d = docs
        .get(&id.doc_key)
        .ok_or_else(|| PassageNotFound(pid.clone()))?;
    let s = cite::skip_block_marker(&original, 0, original.len());
    let quote = original[s..].trim_end().to_string();
    let line = id.line_start + original[..s].matches('\n').count();
    let link = cfg.link_for(&id.doc_key, line);
    let citation = citation(cfg, d, &quote, link.as_deref());
    Ok(CitedPassage {
        schema_version: SCHEMA_VERSION,
        passage_id: pid,
        corpus: cfg.id.clone(),
        rel_path: id.doc_key.clone(),
        path: cfg.source_path(&id.doc_key).display().to_string(),
        line_start: id.line_start,
        line_end: line_end as usize,
        quote,
        title: d.title.clone(),
        author: d.author.clone(),
        date: d.date.clone(),
        date_display: d.date.as_deref().and_then(cite::format_date),
        date_source: d.date_source.clone(),
        link,
        public_url: d.public_url.clone(),
        citation,
    })
}

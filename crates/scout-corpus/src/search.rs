//! Plain search (T1): words are prefix-matched and ANDed; `"quoted phrases"`
//! match consecutive tokens. Bare stopwords are dropped unless every word is
//! one. Hits are passages grouped by document; quotes are original text.

use crate::cite::{self, CiteFormat, CiteSource};
use crate::normalize;
use crate::registry::CorpusConfig;
use crate::stopwords::is_stopword;
use crate::tokenize::{tokenize, words, IdentityLemmatizer};
use anyhow::Result;
use rusqlite::{params, Connection};
use serde::Serialize;
use std::collections::BTreeMap;
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

/// One query term: a prefix-matched word or an exact phrase of tokens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "tokens", rename_all = "snake_case")]
pub enum Term {
    Prefix(String),
    Phrase(Vec<String>),
}

impl Term {
    fn fts(&self) -> String {
        match self {
            Term::Prefix(t) => format!("\"{t}\"*"),
            Term::Phrase(ts) => format!("\"{}\"", ts.join(" ")),
        }
    }
}

/// Parse a plain query into terms.
pub fn parse_terms(query: &str) -> Vec<Term> {
    let q = normalize::unify_quotes_str(query);
    let mut raw: Vec<(bool, String)> = Vec::new();
    let mut rest = q.as_str();
    while let Some(i) = rest.find('"') {
        raw.push((false, rest[..i].to_string()));
        let after = &rest[i + 1..];
        match after.find('"') {
            Some(j) => {
                raw.push((true, after[..j].to_string()));
                rest = &after[j + 1..];
            }
            None => {
                rest = after;
            }
        }
    }
    raw.push((false, rest.to_string()));
    let mut terms = Vec::new();
    let mut bare = Vec::new();
    for (quoted, s) in raw {
        let toks = words(&s);
        if quoted {
            if !toks.is_empty() {
                terms.push(Term::Phrase(toks));
            }
        } else {
            bare.extend(toks);
        }
    }
    let has_content = bare.iter().any(|t| !is_stopword(t)) || !terms.is_empty();
    for t in bare {
        if has_content && is_stopword(&t) {
            continue;
        }
        let term = Term::Prefix(t);
        if !terms.contains(&term) {
            terms.push(term);
        }
    }
    terms
}

#[derive(Debug, Clone, Serialize)]
pub struct Citation {
    pub markdown: String,
    pub plain: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PassageHit {
    /// `<corpus>:<rel_path>:<line_start>`, stable across rebuilds.
    pub passage_id: String,
    pub line_start: usize,
    pub line_end: usize,
    /// The source line the quote starts on.
    pub line: usize,
    /// Original text: the hit sentence, or the whole passage with `--passage`.
    pub quote: String,
    pub score: f64,
    pub link: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocResult {
    pub corpus: String,
    pub rel_path: String,
    pub path: String,
    pub title: String,
    pub author: Option<String>,
    pub date: Option<String>,
    pub date_display: Option<String>,
    pub genre: Option<String>,
    pub lang: Option<String>,
    /// Best passage score, plus the title score when the title matches.
    pub score: f64,
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
    rel_path: String,
    line_start: i64,
    score: f64,
}

struct DocRow {
    title: String,
    date: Option<String>,
    genre: Option<String>,
    lang: Option<String>,
    author: Option<String>,
}

/// Search one corpus's index.
pub fn search_corpus(
    conn: &Connection,
    cfg: &CorpusConfig,
    req: &SearchRequest,
) -> Result<SearchResults> {
    let terms = parse_terms(&req.query);
    let mut res = SearchResults {
        schema_version: SCHEMA_VERSION,
        query: req.query.clone(),
        terms: terms.clone(),
        corpora: vec![cfg.id.clone()],
        total_documents: 0,
        total_passages: 0,
        results: vec![],
    };
    if terms.is_empty() {
        return Ok(res);
    }
    let fts = terms
        .iter()
        .map(|t| t.fts())
        .collect::<Vec<_>>()
        .join(" AND ");
    let mut st = conn.prepare(
        "SELECT p.rel_path, p.line_start, bm25(passages_fts) AS s
         FROM passages_fts JOIN passages p ON p.id = passages_fts.rowid
         WHERE passages_fts MATCH ?1",
    )?;
    let rows: Vec<Row> = st
        .query_map([&fts], |r| {
            Ok(Row {
                rel_path: r.get(0)?,
                line_start: r.get(1)?,
                score: round6(-r.get::<_, f64>(2)?),
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    res.total_passages = rows.len();

    // Group by document; order passages by score desc, then line.
    let mut by_doc: BTreeMap<String, Vec<Row>> = BTreeMap::new();
    for r in rows {
        by_doc.entry(r.rel_path.clone()).or_default().push(r);
    }
    let mut docs: Vec<(String, Vec<Row>)> = by_doc.into_iter().collect();
    for (_, ps) in docs.iter_mut() {
        ps.sort_by(|a, b| {
            b.score
                .total_cmp(&a.score)
                .then(a.line_start.cmp(&b.line_start))
        });
    }
    // A document whose title matches every term gets its title's score added
    // to its best passage score.
    let mut title_scores: BTreeMap<String, f64> = BTreeMap::new();
    {
        let mut tq = conn.prepare(
            "SELECT rel_path, bm25(titles_fts) FROM titles_fts WHERE titles_fts MATCH ?1",
        )?;
        for row in tq.query_map([&fts], |r| {
            Ok((r.get::<_, String>(0)?, round6(-r.get::<_, f64>(1)?)))
        })? {
            let (k, v) = row?;
            title_scores.insert(k, v);
        }
    }
    let doc_score =
        |rel: &str, ps: &[Row]| round6(ps[0].score + title_scores.get(rel).copied().unwrap_or(0.0));
    docs.sort_by(|a, b| {
        doc_score(&b.0, &b.1)
            .total_cmp(&doc_score(&a.0, &a.1))
            .then(a.0.cmp(&b.0))
    });
    res.total_documents = docs.len();

    let rules = normalize::compile_rules(&cfg.boilerplate)?;
    let root = cfg.root();
    let mut get_doc =
        conn.prepare("SELECT title, date, genre, lang, author FROM documents WHERE rel_path = ?1")?;
    let mut get_pass = conn.prepare(
        "SELECT line_end, original FROM passages WHERE rel_path = ?1 AND line_start = ?2",
    )?;
    for (rel, ps) in docs.into_iter().take(req.limit) {
        let d: DocRow = get_doc.query_row([&rel], |r| {
            Ok(DocRow {
                title: r.get(0)?,
                date: r.get(1)?,
                genre: r.get(2)?,
                lang: r.get(3)?,
                author: r.get(4)?,
            })
        })?;
        let abs = root.join(&rel);
        let mut hits = Vec::new();
        for p in ps.iter().take(req.hits_per_doc.max(1)) {
            let (line_end, original): (i64, String) =
                get_pass.query_row(params![rel, p.line_start], |r| Ok((r.get(0)?, r.get(1)?)))?;
            let loc = locate(&original, &rules, &terms, req.whole_passage);
            let line = p.line_start as usize + original[..loc.first_match].matches('\n').count();
            hits.push(PassageHit {
                passage_id: format!("{}:{}:{}", cfg.id, rel, p.line_start),
                line_start: p.line_start as usize,
                line_end: line_end as usize,
                line,
                quote: original[loc.start..loc.end].to_string(),
                score: p.score,
                link: cfg
                    .link
                    .as_deref()
                    .map(|t| cite::render_link(t, &abs, line)),
            });
        }
        let best = &hits[0];
        let src = CiteSource {
            author: d.author.as_deref(),
            title: &d.title,
            date: d.date.as_deref(),
            link: best.link.as_deref(),
        };
        let citation = Citation {
            markdown: cite::render_citation(&best.quote, &src, CiteFormat::Markdown),
            plain: cite::render_citation(&best.quote, &src, CiteFormat::Plain),
        };
        res.results.push(DocResult {
            corpus: cfg.id.clone(),
            rel_path: rel.clone(),
            path: abs.display().to_string(),
            date_display: d.date.as_deref().and_then(cite::format_date),
            title: d.title,
            author: d.author,
            date: d.date,
            genre: d.genre,
            lang: d.lang,
            score: doc_score(&rel, &ps),
            title_match: title_scores.contains_key(&rel),
            passage_count: ps.len(),
            hits,
            citation,
        });
    }
    Ok(res)
}

/// Merge per-corpus results: score desc, then corpus, then path.
pub fn merge(query: &str, parts: Vec<SearchResults>, limit: usize) -> SearchResults {
    let mut out = SearchResults {
        schema_version: SCHEMA_VERSION,
        query: query.to_string(),
        terms: parse_terms(query),
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
        b.score
            .total_cmp(&a.score)
            .then(a.corpus.cmp(&b.corpus))
            .then(a.rel_path.cmp(&b.rel_path))
    });
    out.results.truncate(limit);
    out
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

//! Concordance views: KWIC, distribution and the word profile.
//!
//! All three are computed from ONE occurrence finder ([`find`]), so for the
//! same term and filters the KWIC line count, the profile frequency and the
//! sum of the distribution buckets are the same number by construction
//! (invariant 3). The n-gram view counts independently (a full token scan);
//! the tests check that its n=1 count agrees.
//!
//! Matching: every position where the term's tokens occur consecutively in a
//! passage's token stream (exact tokens, no prefix; overlapping phrase matches
//! each count, as n-gram positions do). The FTS index only preselects
//! candidate passages; the count comes from re-tokenising them.
//!
//! Rendering: left / node / right are slices of the ORIGINAL passage text,
//! found by mapping token spans of the normalised copy back through the
//! offset map (invariant 2). `left + node + right` is one contiguous original
//! substring.

use crate::cite;
use crate::filter::{DocFilter, DocInfo};
use crate::normalize;
use crate::tokenize::{tokenize, words, IdentityLemmatizer};
use crate::Corpus;
use anyhow::{anyhow, bail, Result};
use rusqlite::params;
use serde::Serialize;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

pub const SCHEMA_VERSION: u32 = 1;

/// Sort keys reach this many tokens either side, independent of `width`.
const KEY_SPAN: usize = 9;

/// The term as tokens: normalised and tokenised exactly as passages are.
pub fn node_tokens(term: &str) -> Vec<String> {
    words(&normalize::normalize(term, &[]).text)
}

/// KWIC sort order. Every order ends with the tie-break corpus, path, line,
/// byte offset (invariant 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KwicSort {
    /// `Lk`: the k-th token left of the node, then k+1, … outward.
    Left(usize),
    /// `Rk`: the k-th token right of the node, then k+1, … outward.
    Right(usize),
    /// Document date ascending; undated last.
    Date,
    /// Corpus, path, line.
    Source,
}

impl FromStr for KwicSort {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        let t = s.trim().to_ascii_lowercase();
        match t.as_str() {
            "date" => return Ok(KwicSort::Date),
            "source" => return Ok(KwicSort::Source),
            _ => {}
        }
        let (side, n) = t.split_at(1.min(t.len()));
        let k: usize = n
            .parse()
            .ok()
            .filter(|k| (1..=KEY_SPAN).contains(k))
            .ok_or_else(|| {
                anyhow!("bad sort {s:?}: use L1..L{KEY_SPAN}, R1..R{KEY_SPAN}, date or source")
            })?;
        match side {
            "l" => Ok(KwicSort::Left(k)),
            "r" => Ok(KwicSort::Right(k)),
            _ => bail!("bad sort {s:?}: use L1..L{KEY_SPAN}, R1..R{KEY_SPAN}, date or source"),
        }
    }
}

impl std::fmt::Display for KwicSort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KwicSort::Left(k) => write!(f, "L{k}"),
            KwicSort::Right(k) => write!(f, "R{k}"),
            KwicSort::Date => f.write_str("date"),
            KwicSort::Source => f.write_str("source"),
        }
    }
}

/// One concordance line. Every text field is original source text.
#[derive(Debug, Clone, Serialize)]
pub struct KwicLine {
    pub corpus: String,
    /// `<corpus>:<rel_path>:<passage line_start>` (the id `scout cite` takes).
    pub passage_id: String,
    pub rel_path: String,
    pub path: String,
    pub title: String,
    pub date: Option<String>,
    pub date_display: Option<String>,
    pub genre: Option<String>,
    pub lang: Option<String>,
    /// 1-based source line of the node.
    pub line: usize,
    pub left: String,
    pub node: String,
    pub right: String,
    /// Byte offset of `node` inside the passage's original text.
    pub node_offset: usize,
    pub link: Option<String>,
    #[serde(skip)]
    left_keys: Vec<String>,
    #[serde(skip)]
    right_keys: Vec<String>,
    #[serde(skip)]
    date_floor: Option<String>,
}

/// Everything [`find`] returns: the matched lines (unsorted) and the filtered
/// documents they were counted over (the denominators).
pub struct Found {
    pub node: Vec<String>,
    pub lines: Vec<KwicLine>,
    /// Per corpus id: its documents passing the filter.
    pub docs: BTreeMap<String, BTreeMap<String, DocInfo>>,
}

impl Found {
    pub fn total_tokens(&self) -> i64 {
        self.docs
            .values()
            .flat_map(|d| d.values())
            .map(|d| d.tokens)
            .sum()
    }

    pub fn pieces(&self) -> usize {
        self.lines
            .iter()
            .map(|l| (l.corpus.as_str(), l.rel_path.as_str()))
            .collect::<BTreeSet<_>>()
            .len()
    }
}

fn fts_phrase(node: &[String]) -> String {
    format!("\"{}\"", node.join(" ").replace('"', "\"\""))
}

/// Every occurrence of `term` in `corpora` under `filter`, with `width`
/// tokens of original-text context either side.
pub fn find(corpora: &[Corpus], term: &str, filter: &DocFilter, width: usize) -> Result<Found> {
    filter.validate()?;
    let node = node_tokens(term);
    if node.is_empty() {
        bail!("empty term {term:?}: it has no word tokens");
    }
    let mut found = Found {
        node: node.clone(),
        lines: Vec::new(),
        docs: BTreeMap::new(),
    };
    let fts = fts_phrase(&node);
    for c in corpora {
        let conn = crate::index::open_existing(&c.config, c.index_path())?;
        let docs = crate::filter::filtered_docs(&conn, filter)?;
        let rules = normalize::compile_rules(&c.config.boilerplate)?;
        let root = c.config.root();
        let mut st = conn.prepare(
            "SELECT p.rel_path, p.line_start, p.original
             FROM passages_fts JOIN passages p ON p.id = passages_fts.rowid
             WHERE passages_fts MATCH ?1
             ORDER BY p.rel_path, p.line_start",
        )?;
        let rows = st.query_map(params![fts], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        for row in rows {
            let (rel, line_start, original) = row?;
            let Some(doc) = docs.get(&rel) else { continue };
            let m = normalize::normalize(&original, &rules);
            let toks = tokenize(&m.text, &IdentityLemmatizer, None);
            if toks.len() < node.len() {
                continue;
            }
            // Original byte span of each token.
            let spans: Vec<(usize, usize)> = toks
                .iter()
                .map(|t| m.original_range(t.start, t.end))
                .collect();
            let abs = root.join(&rel);
            for i in 0..=toks.len() - node.len() {
                if !node.iter().enumerate().all(|(k, w)| toks[i + k].text == *w) {
                    continue;
                }
                let last = i + node.len() - 1;
                let ns = spans[i].0;
                let ne = spans[last].1.max(ns);
                let ls = spans[i.saturating_sub(width)].0.min(ns);
                let re = spans[(last + width).min(toks.len() - 1)].1.max(ne);
                let line = line_start as usize + original[..ns].matches('\n').count();
                let left_keys = (1..=KEY_SPAN)
                    .map_while(|k| i.checked_sub(k).map(|j| toks[j].text.clone()))
                    .collect();
                let right_keys = (1..=KEY_SPAN)
                    .map_while(|k| toks.get(last + k).map(|t| t.text.clone()))
                    .collect();
                found.lines.push(KwicLine {
                    corpus: c.id().to_string(),
                    passage_id: format!("{}:{}:{}", c.id(), rel, line_start),
                    rel_path: rel.clone(),
                    path: abs.display().to_string(),
                    title: doc.title.clone(),
                    date: doc.date.clone(),
                    date_display: doc.date.as_deref().and_then(cite::format_date),
                    genre: doc.genre.clone(),
                    lang: doc.lang.clone(),
                    line,
                    left: original[ls..ns].to_string(),
                    node: original[ns..ne].to_string(),
                    right: original[ne..re].to_string(),
                    node_offset: ns,
                    link: c
                        .config
                        .link
                        .as_deref()
                        .map(|t| cite::render_link(t, &abs, line)),
                    left_keys,
                    right_keys,
                    date_floor: doc.date_floor(),
                });
            }
        }
        found.docs.insert(c.id().to_string(), docs);
    }
    Ok(found)
}

fn cmp_keys(a: &[String], b: &[String], from: usize) -> Ordering {
    for k in from..=KEY_SPAN {
        let x = a.get(k - 1).map(String::as_str).unwrap_or("");
        let y = b.get(k - 1).map(String::as_str).unwrap_or("");
        match x.cmp(y) {
            Ordering::Equal => continue,
            o => return o,
        }
    }
    Ordering::Equal
}

fn cmp_source(a: &KwicLine, b: &KwicLine) -> Ordering {
    a.corpus
        .cmp(&b.corpus)
        .then_with(|| a.rel_path.cmp(&b.rel_path))
        .then(a.line.cmp(&b.line))
        .then(a.node_offset.cmp(&b.node_offset))
}

/// Sort lines in place; every order ends with the source tie-break.
pub fn sort_lines(lines: &mut [KwicLine], sort: KwicSort) {
    lines.sort_by(|a, b| {
        let primary = match sort {
            KwicSort::Left(k) => cmp_keys(&a.left_keys, &b.left_keys, k),
            KwicSort::Right(k) => cmp_keys(&a.right_keys, &b.right_keys, k),
            KwicSort::Date => match (&a.date_floor, &b.date_floor) {
                (Some(x), Some(y)) => x.cmp(y),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => Ordering::Equal,
            },
            KwicSort::Source => Ordering::Equal,
        };
        primary.then_with(|| cmp_source(a, b))
    });
}

// ---------- KWIC ----------

#[derive(Debug, Clone)]
pub struct KwicRequest {
    pub term: String,
    pub width: usize,
    pub sort: KwicSort,
    pub limit: usize,
    pub filter: DocFilter,
}

impl KwicRequest {
    pub fn new(term: impl Into<String>) -> Self {
        KwicRequest {
            term: term.into(),
            width: 8,
            sort: KwicSort::Right(1),
            limit: 200,
            filter: DocFilter::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct KwicResults {
    pub schema_version: u32,
    pub term: String,
    pub node_tokens: Vec<String>,
    pub corpora: Vec<String>,
    pub filters: DocFilter,
    pub width: usize,
    pub sort: String,
    /// All matching lines, before `limit`.
    pub total: usize,
    /// Distinct documents among all matching lines.
    pub pieces: usize,
    pub returned: usize,
    pub lines: Vec<KwicLine>,
}

pub fn kwic(corpora: &[Corpus], req: &KwicRequest) -> Result<KwicResults> {
    let mut f = find(corpora, &req.term, &req.filter, req.width)?;
    sort_lines(&mut f.lines, req.sort);
    let total = f.lines.len();
    let pieces = f.pieces();
    f.lines.truncate(req.limit);
    Ok(KwicResults {
        schema_version: SCHEMA_VERSION,
        term: req.term.clone(),
        node_tokens: f.node.clone(),
        corpora: corpora.iter().map(|c| c.id().to_string()).collect(),
        filters: req.filter.clone(),
        width: req.width,
        sort: req.sort.to_string(),
        total,
        pieces,
        returned: f.lines.len(),
        lines: f.lines,
    })
}

// ---------- distribution ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DistBy {
    Year,
    Corpus,
    Genre,
    Lang,
}

impl FromStr for DistBy {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        Ok(match s.trim().to_ascii_lowercase().as_str() {
            "year" => DistBy::Year,
            "corpus" => DistBy::Corpus,
            "genre" => DistBy::Genre,
            "lang" => DistBy::Lang,
            _ => bail!("bad --by {s:?}: use year, corpus, genre or lang"),
        })
    }
}

/// The bucket key used when a document has no value for the dimension.
pub const UNKNOWN: &str = "unknown";

fn bucket_key(by: DistBy, corpus: &str, d: &DocInfo) -> String {
    let or_unknown = |v: &Option<String>| {
        v.as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| UNKNOWN.into())
    };
    match by {
        DistBy::Year => d
            .year()
            .map(|y| y.to_string())
            .unwrap_or_else(|| UNKNOWN.into()),
        DistBy::Corpus => corpus.to_string(),
        DistBy::Genre => or_unknown(&d.genre),
        DistBy::Lang => or_unknown(&d.lang),
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DistBucket {
    pub key: String,
    pub hits: usize,
    /// Distinct documents with at least one hit.
    pub pieces: usize,
    /// Tokens in all filtered documents of this bucket (the denominator).
    pub tokens: i64,
    pub per_million: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Distribution {
    pub schema_version: u32,
    pub term: String,
    pub node_tokens: Vec<String>,
    pub corpora: Vec<String>,
    pub filters: DocFilter,
    pub by: DistBy,
    /// Equals the KWIC total and the sum of `buckets[].hits`.
    pub total: usize,
    pub pieces: usize,
    /// Every bucket holding filtered documents, including zero-hit ones.
    /// `year`: chronological, `unknown` last. Otherwise hits desc, key asc.
    pub buckets: Vec<DistBucket>,
}

fn per_million(hits: usize, tokens: i64) -> f64 {
    if tokens <= 0 {
        return 0.0;
    }
    ((hits as f64) * 1e6 / tokens as f64 * 1000.0).round() / 1000.0
}

/// Per bucket: hits, the (corpus, path) documents with hits, tokens.
type BucketAcc = (usize, BTreeSet<(String, String)>, i64);

fn buckets_of(f: &Found, by: DistBy) -> Vec<DistBucket> {
    let mut acc: BTreeMap<String, BucketAcc> = BTreeMap::new();
    for (corpus, docs) in &f.docs {
        for d in docs.values() {
            acc.entry(bucket_key(by, corpus, d)).or_default().2 += d.tokens;
        }
    }
    for l in &f.lines {
        let d = &f.docs[&l.corpus][&l.rel_path];
        let e = acc.entry(bucket_key(by, &l.corpus, d)).or_default();
        e.0 += 1;
        e.1.insert((l.corpus.clone(), l.rel_path.clone()));
    }
    let mut out: Vec<DistBucket> = acc
        .into_iter()
        .map(|(key, (hits, docs, tokens))| DistBucket {
            key,
            hits,
            pieces: docs.len(),
            tokens,
            per_million: per_million(hits, tokens),
        })
        .collect();
    match by {
        DistBy::Year => out.sort_by(|a, b| {
            (a.key == UNKNOWN)
                .cmp(&(b.key == UNKNOWN))
                .then_with(|| a.key.cmp(&b.key))
        }),
        _ => out.sort_by(|a, b| b.hits.cmp(&a.hits).then_with(|| a.key.cmp(&b.key))),
    }
    out
}

pub fn distribution(
    corpora: &[Corpus],
    term: &str,
    by: DistBy,
    filter: &DocFilter,
) -> Result<Distribution> {
    let f = find(corpora, term, filter, 0)?;
    Ok(Distribution {
        schema_version: SCHEMA_VERSION,
        term: term.to_string(),
        node_tokens: f.node.clone(),
        corpora: corpora.iter().map(|c| c.id().to_string()).collect(),
        filters: filter.clone(),
        by,
        total: f.lines.len(),
        pieces: f.pieces(),
        buckets: buckets_of(&f, by),
    })
}

// ---------- profile ----------

#[derive(Debug, Clone, Serialize)]
pub struct FirstUse {
    pub corpus: String,
    pub passage_id: String,
    pub rel_path: String,
    pub title: String,
    pub date: Option<String>,
    pub date_display: Option<String>,
    pub line: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocFreq {
    pub corpus: String,
    pub rel_path: String,
    pub title: String,
    pub date: Option<String>,
    pub hits: usize,
    pub tokens: i64,
    pub per_million: f64,
}

/// A profile section whose content arrives in a later ticket.
#[derive(Debug, Clone, Serialize)]
pub struct PendingSection {
    pub status: &'static str,
    pub arrives_in: &'static str,
    pub items: Vec<serde_json::Value>,
}

impl PendingSection {
    fn t4() -> Self {
        PendingSection {
            status: "pending",
            arrives_in: "T4",
            items: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Profile {
    pub schema_version: u32,
    pub word: String,
    pub node_tokens: Vec<String>,
    pub corpora: Vec<String>,
    pub filters: DocFilter,
    /// Equals the KWIC total for the same word and filters.
    pub frequency: usize,
    pub total_tokens: i64,
    pub per_million: f64,
    pub pieces: usize,
    /// The earliest dated occurrence (date, then corpus, path, line).
    pub first_used: Option<FirstUse>,
    /// Per-year distribution; the same buckets as `scout dist --by year`.
    pub by_year: Vec<DistBucket>,
    /// Top documents by relative frequency (per million), then hits, then
    /// corpus and path.
    pub top_documents: Vec<DocFreq>,
    /// Plain logDice list (T4).
    pub collocates: PendingSection,
    /// Top n-grams containing the word (T4).
    pub ngrams: PendingSection,
}

pub const PROFILE_TOP_DOCUMENTS: usize = 10;

pub fn profile(corpora: &[Corpus], word: &str, filter: &DocFilter) -> Result<Profile> {
    let mut f = find(corpora, word, filter, 0)?;
    sort_lines(&mut f.lines, KwicSort::Date);
    let total_tokens = f.total_tokens();
    let first_used = f
        .lines
        .iter()
        .find(|l| l.date_floor.is_some())
        .map(|l| FirstUse {
            corpus: l.corpus.clone(),
            passage_id: l.passage_id.clone(),
            rel_path: l.rel_path.clone(),
            title: l.title.clone(),
            date: l.date.clone(),
            date_display: l.date_display.clone(),
            line: l.line,
        });
    let mut per_doc: BTreeMap<(String, String), usize> = BTreeMap::new();
    for l in &f.lines {
        *per_doc
            .entry((l.corpus.clone(), l.rel_path.clone()))
            .or_default() += 1;
    }
    let mut top: Vec<DocFreq> = per_doc
        .into_iter()
        .map(|((corpus, rel), hits)| {
            let d = &f.docs[&corpus][&rel];
            DocFreq {
                title: d.title.clone(),
                date: d.date.clone(),
                tokens: d.tokens,
                per_million: per_million(hits, d.tokens),
                hits,
                corpus,
                rel_path: rel,
            }
        })
        .collect();
    top.sort_by(|a, b| {
        b.per_million
            .total_cmp(&a.per_million)
            .then(b.hits.cmp(&a.hits))
            .then_with(|| a.corpus.cmp(&b.corpus))
            .then_with(|| a.rel_path.cmp(&b.rel_path))
    });
    top.truncate(PROFILE_TOP_DOCUMENTS);
    Ok(Profile {
        schema_version: SCHEMA_VERSION,
        word: word.to_string(),
        node_tokens: f.node.clone(),
        corpora: corpora.iter().map(|c| c.id().to_string()).collect(),
        filters: filter.clone(),
        frequency: f.lines.len(),
        total_tokens,
        per_million: per_million(f.lines.len(), total_tokens),
        pieces: f.pieces(),
        first_used,
        by_year: buckets_of(&f, DistBy::Year),
        top_documents: top,
        collocates: PendingSection::t4(),
        ngrams: PendingSection::t4(),
    })
}

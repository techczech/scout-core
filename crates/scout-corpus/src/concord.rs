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
//! substring. Only the lines a view returns are rendered: counting runs over
//! the stored token streams, so `kwic --limit 200` renders 200 passages, not
//! every hit.

use crate::cite;
use crate::filter::{DocFilter, DocInfo};
use crate::normalize;
use crate::tokenize::{tokenize, words, IdentityLemmatizer};
use crate::Corpus;
use anyhow::{anyhow, bail, Result};
use rusqlite::params;
use serde::Serialize;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, HashMap};
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
}

/// A collocate restriction for KWIC: keep only lines where the single token
/// `word` occurs within `window` tokens left or right of the node, inside the
/// passage (the node's own tokens do not count). This is exactly the
/// co-occurrence a collocate row counts, so a row's count equals the lines of
/// its concordance link (invariant 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Near {
    pub word: String,
    pub window: usize,
}

/// One indexed passage holding at least one hit, as interned token ids.
pub(crate) struct PassageRec {
    /// Index into [`Hits::corpora`].
    pub corpus: usize,
    pub rowid: i64,
    pub rel: String,
    pub line_start: i64,
    pub toks: Vec<u32>,
    pub(crate) date_floor: Option<String>,
}

/// One node occurrence: passage index and the node's first token index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Hit {
    pub p: u32,
    pub i: u32,
}

/// Every occurrence of a term, found over the stored token streams. Text is
/// rendered from the original only for the lines a view returns
/// ([`Hits::lines`]), so counting views never touch the original text.
pub struct Hits {
    pub node: Vec<String>,
    pub(crate) corpora: Vec<Corpus>,
    pub(crate) vocab: Vec<String>,
    pub(crate) ids: HashMap<String, u32>,
    pub(crate) passages: Vec<PassageRec>,
    /// In source order (corpus as given, then path, line, offset).
    pub(crate) hits: Vec<Hit>,
    /// Per corpus id: its documents passing the filter (the denominators).
    pub docs: BTreeMap<String, BTreeMap<String, DocInfo>>,
}

impl Hits {
    pub fn len(&self) -> usize {
        self.hits.len()
    }

    pub fn is_empty(&self) -> bool {
        self.hits.is_empty()
    }

    pub fn total_tokens(&self) -> i64 {
        self.docs
            .values()
            .flat_map(|d| d.values())
            .map(|d| d.tokens)
            .sum()
    }

    pub fn corpus_ids(&self) -> Vec<String> {
        self.corpora.iter().map(|c| c.id().to_string()).collect()
    }

    pub(crate) fn corpus_id(&self, h: Hit) -> &str {
        self.corpora[self.passages[h.p as usize].corpus].id()
    }

    pub(crate) fn doc(&self, h: Hit) -> &DocInfo {
        let rec = &self.passages[h.p as usize];
        &self.docs[self.corpora[rec.corpus].id()][&rec.rel]
    }

    pub fn pieces(&self) -> usize {
        let mut seen: BTreeSet<(usize, &str)> = BTreeSet::new();
        for h in &self.hits {
            let rec = &self.passages[h.p as usize];
            seen.insert((rec.corpus, rec.rel.as_str()));
        }
        seen.len()
    }

    /// The token ids within `w` tokens left and right of the node.
    pub(crate) fn window(&self, h: Hit, w: usize) -> (&[u32], &[u32]) {
        let toks = &self.passages[h.p as usize].toks;
        let i = h.i as usize;
        let last = i + self.node.len(); // one past the node
        let ls = i.saturating_sub(w);
        let re = (last + w).min(toks.len());
        (&toks[ls..i], &toks[last..re])
    }

    fn tok(&self, rec: &PassageRec, j: Option<usize>) -> &str {
        j.and_then(|j| rec.toks.get(j))
            .map(|&id| self.vocab[id as usize].as_str())
            .unwrap_or("")
    }

    fn cmp_side(&self, a: Hit, b: Hit, left: bool, from: usize) -> Ordering {
        let (ra, rb) = (&self.passages[a.p as usize], &self.passages[b.p as usize]);
        let n = self.node.len();
        for k in from..=KEY_SPAN {
            let pos = |h: Hit| {
                if left {
                    (h.i as usize).checked_sub(k)
                } else {
                    Some(h.i as usize + n - 1 + k)
                }
            };
            match self.tok(ra, pos(a)).cmp(self.tok(rb, pos(b))) {
                Ordering::Equal => continue,
                o => return o,
            }
        }
        Ordering::Equal
    }

    /// Corpus id, path, passage line, token index: the same order as corpus,
    /// path, node line, byte offset.
    pub(crate) fn cmp_source(&self, a: Hit, b: Hit) -> Ordering {
        let (ra, rb) = (&self.passages[a.p as usize], &self.passages[b.p as usize]);
        self.corpora[ra.corpus]
            .id()
            .cmp(self.corpora[rb.corpus].id())
            .then_with(|| ra.rel.cmp(&rb.rel))
            .then(ra.line_start.cmp(&rb.line_start))
            .then(a.i.cmp(&b.i))
    }

    pub(crate) fn cmp_date(&self, a: Hit, b: Hit) -> Ordering {
        let (x, y) = (
            &self.passages[a.p as usize].date_floor,
            &self.passages[b.p as usize].date_floor,
        );
        match (x, y) {
            (Some(x), Some(y)) => x.cmp(y),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        }
    }

    /// The hits in `sort` order; every order ends with the source tie-break.
    pub(crate) fn sorted(&self, sort: KwicSort) -> Vec<Hit> {
        let mut v = self.hits.clone();
        v.sort_by(|&a, &b| {
            let primary = match sort {
                KwicSort::Left(k) => self.cmp_side(a, b, true, k),
                KwicSort::Right(k) => self.cmp_side(a, b, false, k),
                KwicSort::Date => self.cmp_date(a, b),
                KwicSort::Source => Ordering::Equal,
            };
            primary.then_with(|| self.cmp_source(a, b))
        });
        v
    }

    /// Render `hits` as concordance lines from the ORIGINAL passage text,
    /// with `width` tokens either side (invariant 2).
    pub(crate) fn lines(&self, hits: &[Hit], width: usize) -> Result<Vec<KwicLine>> {
        // Row ids restart in every corpus's index, so a passage is named by
        // (corpus, row id), never by row id alone.
        struct Rendered {
            key: (usize, i64),
            original: String,
            spans: Vec<(usize, usize)>,
        }
        let mut conns: BTreeMap<usize, (rusqlite::Connection, Vec<regex::Regex>)> = BTreeMap::new();
        let mut cache: Option<Rendered> = None;
        let mut out = Vec::with_capacity(hits.len());
        for &h in hits {
            let rec = &self.passages[h.p as usize];
            let c = &self.corpora[rec.corpus];
            let key = (rec.corpus, rec.rowid);
            if cache.as_ref().map(|r| r.key) != Some(key) {
                if let std::collections::btree_map::Entry::Vacant(e) = conns.entry(rec.corpus) {
                    let conn = crate::index::open_existing(&c.config, c.index_path())?;
                    let rules = normalize::compile_rules(&c.config.boilerplate)?;
                    e.insert((conn, rules));
                }
                let (conn, rules) = &conns[&rec.corpus];
                let original: String = conn.query_row(
                    "SELECT original FROM passages WHERE id = ?1",
                    params![rec.rowid],
                    |r| r.get(0),
                )?;
                let m = normalize::normalize(&original, rules);
                let toks = tokenize(&m.text, &IdentityLemmatizer, None);
                if toks.len() != rec.toks.len() {
                    bail!(
                        "index for {:?} is out of date ({}); run `scout index build {} --force`",
                        c.id(),
                        rec.rel,
                        c.id()
                    );
                }
                let spans = toks
                    .iter()
                    .map(|t| m.original_range(t.start, t.end))
                    .collect();
                cache = Some(Rendered {
                    key,
                    original,
                    spans,
                });
            }
            let r = cache.as_ref().unwrap();
            let (original, spans) = (&r.original, &r.spans);
            let i = h.i as usize;
            let last = i + self.node.len() - 1;
            let ns = spans[i].0;
            let ne = spans[last].1.max(ns);
            let ls = extend_left(original, spans[i.saturating_sub(width)].0.min(ns));
            let re = extend_right(
                original,
                spans[(last + width).min(spans.len() - 1)].1.max(ne),
            );
            let line = rec.line_start as usize + original[..ns].matches('\n').count();
            let doc = self.doc(h);
            let abs = c.config.source_path(&rec.rel);
            out.push(KwicLine {
                corpus: c.id().to_string(),
                passage_id: format!("{}:{}:{}", c.id(), rec.rel, rec.line_start),
                rel_path: rec.rel.clone(),
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
                link: c.config.link_for(&rec.rel, line),
            });
        }
        Ok(out)
    }
}

/// Punctuation glued to the context edge belongs to the context: extend `end`
/// over the characters up to the next whitespace (or the passage end) when
/// they are all non-alphanumeric, so "(building scaffolding)" keeps its ")".
/// A run that reaches into letters (a stripped URL, say) is left out.
fn extend_right(text: &str, end: usize) -> usize {
    let rest = &text[end..];
    let run = rest.find(char::is_whitespace).unwrap_or(rest.len());
    if run > 0 && !rest[..run].chars().any(char::is_alphanumeric) {
        end + run
    } else {
        end
    }
}

/// The left-hand mirror of [`extend_right`]: an opening "(" or "“" glued to
/// the first context token is kept.
fn extend_left(text: &str, start: usize) -> usize {
    let before = &text[..start];
    let from = before
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace())
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);
    if from < start && !before[from..].chars().any(char::is_alphanumeric) {
        from
    } else {
        start
    }
}

fn fts_phrase(node: &[String]) -> String {
    format!("\"{}\"", node.join(" ").replace('"', "\"\""))
}

/// Every occurrence of `term` in `corpora` under `filter`, optionally only
/// those with `near` inside the window.
///
/// Matching runs over the stored token streams (the same tokens the n-gram
/// view counts); the FTS index only preselects candidate passages.
pub fn find(
    corpora: &[Corpus],
    term: &str,
    filter: &DocFilter,
    near: Option<&Near>,
) -> Result<Hits> {
    filter.validate()?;
    let node = node_tokens(term);
    if node.is_empty() {
        bail!("empty term {term:?}: it has no word tokens");
    }
    let near_tok = match near {
        None => None,
        Some(n) => {
            let t = node_tokens(&n.word);
            if t.len() != 1 {
                bail!("--near {:?}: give exactly one word", n.word);
            }
            Some((t[0].clone(), n.window))
        }
    };
    let mut ids: HashMap<String, u32> = HashMap::new();
    let mut vocab: Vec<String> = Vec::new();
    fn intern(t: &str, ids: &mut HashMap<String, u32>, vocab: &mut Vec<String>) -> u32 {
        if let Some(&id) = ids.get(t) {
            return id;
        }
        let id = vocab.len() as u32;
        ids.insert(t.to_string(), id);
        vocab.push(t.to_string());
        id
    }
    let node_ids: Vec<u32> = node
        .iter()
        .map(|t| intern(t, &mut ids, &mut vocab))
        .collect();
    let near_id = near_tok
        .as_ref()
        .map(|(t, w)| (intern(t, &mut ids, &mut vocab), *w));
    let mut out = Hits {
        node: node.clone(),
        corpora: corpora.to_vec(),
        vocab: Vec::new(),
        ids: HashMap::new(),
        passages: Vec::new(),
        hits: Vec::new(),
        docs: BTreeMap::new(),
    };
    let fts = fts_phrase(&node);
    let n = node.len();
    for (ci, c) in corpora.iter().enumerate() {
        let conn = crate::index::open_existing(&c.config, c.index_path())?;
        let docs = crate::filter::filtered_docs(&conn, filter)?;
        let mut st = conn.prepare(
            "SELECT p.id, p.rel_path, p.line_start, passages_fts.tokens
             FROM passages_fts JOIN passages p ON p.id = passages_fts.rowid
             WHERE passages_fts MATCH ?1
             ORDER BY p.rel_path, p.line_start",
        )?;
        let mut rows = st.query(params![fts])?;
        while let Some(r) = rows.next()? {
            let rel: String = r.get(1)?;
            let Some(doc) = docs.get(&rel) else { continue };
            let text: String = r.get(3)?;
            let toks: Vec<u32> = text
                .split(' ')
                .filter(|t| !t.is_empty())
                .map(|t| intern(t, &mut ids, &mut vocab))
                .collect();
            if toks.len() < n {
                continue;
            }
            let p = out.passages.len() as u32;
            let before = out.hits.len();
            for i in 0..=toks.len() - n {
                if toks[i..i + n] != node_ids[..] {
                    continue;
                }
                if let Some((nid, w)) = near_id {
                    let ls = i.saturating_sub(w);
                    let re = (i + n + w).min(toks.len());
                    if !toks[ls..i].contains(&nid) && !toks[i + n..re].contains(&nid) {
                        continue;
                    }
                }
                out.hits.push(Hit { p, i: i as u32 });
            }
            if out.hits.len() > before {
                out.passages.push(PassageRec {
                    corpus: ci,
                    rowid: r.get(0)?,
                    rel,
                    line_start: r.get(2)?,
                    toks,
                    date_floor: doc.date_floor(),
                });
            }
        }
        out.docs.insert(c.id().to_string(), docs);
    }
    out.vocab = vocab;
    out.ids = ids;
    Ok(out)
}

// ---------- KWIC ----------

#[derive(Debug, Clone)]
pub struct KwicRequest {
    pub term: String,
    pub width: usize,
    pub sort: KwicSort,
    pub limit: usize,
    pub filter: DocFilter,
    /// Keep only lines with this collocate in the window (a collocate row's
    /// concordance link).
    pub near: Option<Near>,
}

impl KwicRequest {
    pub fn new(term: impl Into<String>) -> Self {
        KwicRequest {
            term: term.into(),
            width: 8,
            sort: KwicSort::Right(1),
            limit: 200,
            filter: DocFilter::default(),
            near: None,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub near: Option<Near>,
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
    let f = find(corpora, &req.term, &req.filter, req.near.as_ref())?;
    let mut order = f.sorted(req.sort);
    order.truncate(req.limit);
    let lines = f.lines(&order, req.width)?;
    Ok(KwicResults {
        schema_version: SCHEMA_VERSION,
        term: req.term.clone(),
        node_tokens: f.node.clone(),
        corpora: f.corpus_ids(),
        filters: req.filter.clone(),
        near: req.near.clone(),
        width: req.width,
        sort: req.sort.to_string(),
        total: f.len(),
        pieces: f.pieces(),
        returned: lines.len(),
        lines,
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

pub(crate) fn per_million(hits: usize, tokens: i64) -> f64 {
    if tokens <= 0 {
        return 0.0;
    }
    ((hits as f64) * 1e6 / tokens as f64 * 1000.0).round() / 1000.0
}

/// Per bucket: hits, the (corpus, path) documents with hits, tokens.
type BucketAcc = (usize, BTreeSet<(String, String)>, i64);

pub(crate) fn buckets_of(f: &Hits, by: DistBy) -> Vec<DistBucket> {
    let mut acc: BTreeMap<String, BucketAcc> = BTreeMap::new();
    for (corpus, docs) in &f.docs {
        for d in docs.values() {
            acc.entry(bucket_key(by, corpus, d)).or_default().2 += d.tokens;
        }
    }
    for &h in &f.hits {
        let (corpus, d) = (f.corpus_id(h), f.doc(h));
        let e = acc.entry(bucket_key(by, corpus, d)).or_default();
        e.0 += 1;
        e.1.insert((corpus.to_string(), d.rel_path.clone()));
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
    let f = find(corpora, term, filter, None)?;
    Ok(Distribution {
        schema_version: SCHEMA_VERSION,
        term: term.to_string(),
        node_tokens: f.node.clone(),
        corpora: f.corpus_ids(),
        filters: filter.clone(),
        by,
        total: f.len(),
        pieces: f.pieces(),
        buckets: buckets_of(&f, by),
    })
}

// ---------- profile ----------

/// The profile lives in [`crate::profile`]; re-exported here, where T3 put it.
pub use crate::profile::{
    profile, profile_with, DocFreq, FirstUse, Profile, ProfileCollocates, ProfileNgrams,
    ProfileOptions, PROFILE_SCHEMA_VERSION, PROFILE_TOP_DOCUMENTS,
};

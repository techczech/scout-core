//! "More like these": rank passages by tf-idf cosine similarity to the
//! centroid of 1–10 seed passages.
//!
//! - Terms are the index's passage tokens (the stream KWIC and n-grams use)
//!   minus stopwords and tokens with no letter.
//! - Weight = (1 + ln tf) × idf, idf = ln((1 + N) / (1 + df)) + 1, where N
//!   and df count passages across the searched corpora (df from the FTS
//!   vocabulary, so nothing new is stored in the index).
//! - Each seed vector is L2-normalised; the centroid is their mean. A
//!   passage's score is its cosine with the centroid.
//! - A candidate shares at least two distinct terms with the seeds.
//! - Each suggestion names its 3 highest-contributing shared terms and the
//!   seed it is most similar to.
//! - Computed at query time; every float sum runs in a fixed order, so the
//!   output is byte-identical across runs. Ties break by passage id.

use crate::cite;
use crate::filter::{load_docs, DocInfo};
use crate::search::{citation, Citation, PassageId, PassageNotFound};
use crate::stopwords::is_stopword;
use crate::Corpus;
use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};

pub const SCHEMA_VERSION: u32 = 1;
pub const MAX_SEEDS: usize = 10;
const SHARED_TERMS: usize = 3;
/// A candidate must share at least this many distinct terms with the seeds:
/// one shared rare word makes a two-word heading outrank real neighbours.
const MIN_SHARED: usize = 2;

#[derive(Debug, Clone)]
pub struct SimilarRequest {
    pub top: usize,
    pub exclude_seeds: bool,
}

/// A seed as the results name it.
#[derive(Debug, Clone, Serialize)]
pub struct Seed {
    pub passage_id: String,
    pub corpus: String,
    pub rel_path: String,
    pub title: String,
    pub date: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Suggestion {
    pub passage_id: String,
    pub corpus: String,
    pub rel_path: String,
    pub path: String,
    pub line_start: usize,
    pub line_end: usize,
    /// The passage's original text (block markers at its start skipped).
    pub quote: String,
    /// Cosine similarity with the seeds' centroid, 0–1.
    pub score: f64,
    /// The (up to) 3 terms shared with the seeds that add most to the score.
    pub shared_terms: Vec<String>,
    /// The passage id of the seed this passage is most similar to.
    pub closest_seed: String,
    pub title: String,
    pub author: Option<String>,
    pub date: Option<String>,
    pub date_display: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date_source: Option<String>,
    pub genre: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub link: Option<String>,
    pub public_url: Option<String>,
    pub citation: Citation,
}

#[derive(Debug, Clone, Serialize)]
pub struct SimilarResults {
    pub schema_version: u32,
    pub seeds: Vec<Seed>,
    pub corpora: Vec<String>,
    /// Passages sharing at least two terms with the seeds (before `top`).
    pub total_passages: usize,
    pub results: Vec<Suggestion>,
}

fn round6(x: f64) -> f64 {
    (x * 1e6).round() / 1e6
}

/// FxHash (rustc's hasher): the scan hashes every token of every passage,
/// and SipHash dominated the query time. Lookups only; nothing iterates a
/// map built with it where order would matter.
#[derive(Default, Clone, Copy)]
struct Fx(u64);

impl std::hash::Hasher for Fx {
    fn write(&mut self, bytes: &[u8]) {
        const K: u64 = 0x51_7c_c1_b7_27_22_0a_95;
        for chunk in bytes.chunks(8) {
            let mut b = [0u8; 8];
            b[..chunk.len()].copy_from_slice(chunk);
            self.0 = (self.0.rotate_left(5) ^ u64::from_le_bytes(b)).wrapping_mul(K);
        }
    }
    fn write_u8(&mut self, i: u8) {
        self.write(&[i]);
    }
    fn finish(&self) -> u64 {
        self.0
    }
}

type FxMap<K, V> = HashMap<K, V, std::hash::BuildHasherDefault<Fx>>;

/// A term counts when it is not a stopword and holds a letter.
fn is_term(t: &str) -> bool {
    !t.is_empty() && !is_stopword(t) && t.chars().any(char::is_alphabetic)
}

/// A weighted passage vector: (term, weight) in term order.
type Vector<'a> = Vec<(&'a str, f64)>;

/// Passage counts over the searched corpora: N and df per term.
struct Idf {
    n: f64,
    df: FxMap<String, u64>,
}

impl Idf {
    /// N and df from each index's FTS vocabulary (one thread per corpus).
    fn load(corpora: &[Corpus]) -> Result<Idf> {
        let parts = std::thread::scope(|sc| {
            let hs: Vec<_> = corpora
                .iter()
                .map(|c| sc.spawn(move || Idf::load_one(c)))
                .collect();
            hs.into_iter()
                .map(|h| h.join().expect("similar: idf thread panicked"))
                .collect::<Result<Vec<_>>>()
        })?;
        let mut n = 0u64;
        let mut df: FxMap<String, u64> = FxMap::default();
        for (pn, pdf) in parts {
            n += pn;
            for (t, d) in pdf {
                *df.entry(t).or_insert(0) += d;
            }
        }
        Ok(Idf { n: n as f64, df })
    }

    fn load_one(c: &Corpus) -> Result<(u64, Vec<(String, u64)>)> {
        let conn = crate::index::open_existing(&c.config, c.index_path())?;
        let n = conn.query_row("SELECT COUNT(*) FROM passages", [], |r| r.get::<_, i64>(0))?;
        conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS temp.similar_vocab
             USING fts5vocab(main, passages_fts, row);",
        )?;
        let mut st = conn.prepare("SELECT term, doc FROM temp.similar_vocab")?;
        let mut rows = st.query([])?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            let term = r.get_ref(0)?.as_str()?;
            if is_term(term) {
                out.push((term.to_string(), r.get::<_, i64>(1)? as u64));
            }
        }
        Ok((n as u64, out))
    }

    fn idf(&self, term: &str) -> f64 {
        let df = self.df.get(term).copied().unwrap_or(0) as f64;
        ((1.0 + self.n) / (1.0 + df)).ln() + 1.0
    }

    /// The weighted vector of a token string and its L2 norm.
    fn vector<'a>(&self, tokens: &'a str) -> (Vector<'a>, f64) {
        let mut ts: Vec<&str> = tokens.split(' ').filter(|t| is_term(t)).collect();
        ts.sort_unstable();
        let mut v = Vec::new();
        let mut sq = 0.0;
        let mut i = 0;
        while i < ts.len() {
            let mut j = i + 1;
            while j < ts.len() && ts[j] == ts[i] {
                j += 1;
            }
            let w = (1.0 + ((j - i) as f64).ln()) * self.idf(ts[i]);
            sq += w * w;
            v.push((ts[i], w));
            i = j;
        }
        (v, sq.sqrt())
    }
}

struct SeedVec {
    id: String,
    /// L2-normalised weights.
    v: FxMap<String, f64>,
}

fn seed_tokens(conn: &Connection, id: &PassageId, pid: &str) -> Result<String> {
    conn.query_row(
        "SELECT f.tokens FROM passages p JOIN passages_fts f ON f.rowid = p.id
         WHERE p.rel_path = ?1 AND p.line_start = ?2",
        params![id.doc_key, id.line_start as i64],
        |r| r.get(0),
    )
    .optional()?
    .ok_or_else(|| PassageNotFound(pid.to_string()).into())
}

/// `a · b`, summed in `b`'s (term) order so the result is reproducible.
fn dot(a: &FxMap<String, f64>, b: &Vector) -> f64 {
    b.iter().filter_map(|(t, w)| a.get(*t).map(|x| x * w)).sum()
}

/// The passage scan of one corpus against the centroid.
struct Scan<'a> {
    idf: &'a Idf,
    centroid: &'a FxMap<String, f64>,
    c_norm: f64,
    /// Passage ids to leave out (the seeds, with `exclude_seeds`).
    exclude: &'a [&'a str],
}

impl Scan<'_> {
    fn corpus(&self, ci: usize, c: &Corpus) -> Result<Vec<Scored>> {
        let conn = crate::index::open_existing(&c.config, c.index_path())?;
        let mut st = conn.prepare(
            "SELECT p.rel_path, p.line_start, p.line_end, f.tokens, p.id
             FROM passages p JOIN passages_fts f ON f.rowid = p.id",
        )?;
        let mut rows = st.query([])?;
        let mut out = Vec::new();
        while let Some(r) = rows.next()? {
            let tokens = r.get_ref(3)?.as_str()?;
            // Cheap pre-check before building the vector: two token hits.
            if tokens
                .split(' ')
                .filter(|t| self.centroid.contains_key(*t))
                .nth(MIN_SHARED - 1)
                .is_none()
            {
                continue;
            }
            let rel_path = r.get_ref(0)?.as_str()?;
            let line_start = r.get::<_, i64>(1)? as usize;
            let pid = format!("{}:{}:{}", c.id(), rel_path, line_start);
            if self.exclude.contains(&pid.as_str()) {
                continue;
            }
            let (v, p_norm) = self.idf.vector(tokens);
            if p_norm == 0.0
                || v.iter()
                    .filter(|(t, _)| self.centroid.contains_key(*t))
                    .count()
                    < MIN_SHARED
            {
                continue;
            }
            let score = round6(dot(self.centroid, &v) / (self.c_norm * p_norm));
            if score <= 0.0 {
                continue;
            }
            out.push(Scored {
                score,
                pid,
                corpus: ci,
                rel_path: rel_path.to_string(),
                line_start,
                line_end: r.get::<_, i64>(2)? as usize,
                rowid: r.get(4)?,
            });
        }
        Ok(out)
    }
}

/// One scored passage before it is dressed for output.
struct Scored {
    score: f64,
    pid: String,
    corpus: usize,
    rel_path: String,
    line_start: usize,
    line_end: usize,
    rowid: i64,
}

/// The "why" of a passage vector: its shared terms by contribution to the
/// score (then by term), and its closest seed (the first on a tie).
fn explain(
    v: &Vector,
    p_norm: f64,
    centroid: &FxMap<String, f64>,
    seeds: &[SeedVec],
) -> (Vec<String>, String) {
    let mut contrib: Vec<(f64, &str)> = v
        .iter()
        .filter_map(|(t, w)| centroid.get(*t).map(|cw| (cw * w, *t)))
        .collect();
    contrib.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(b.1)));
    let shared = contrib
        .iter()
        .take(SHARED_TERMS)
        .map(|(_, t)| t.to_string())
        .collect();
    let mut closest = (f64::NEG_INFINITY, 0usize);
    for (i, s) in seeds.iter().enumerate() {
        let sim = round6(dot(&s.v, v) / p_norm);
        if sim > closest.0 {
            closest = (sim, i);
        }
    }
    (shared, seeds[closest.1].id.clone())
}

/// Passages of `corpora` ranked by similarity to `seeds`, each seed paired
/// with the corpus whose index holds it.
pub fn similar(
    seeds: &[(Corpus, PassageId)],
    corpora: &[Corpus],
    req: &SimilarRequest,
) -> Result<SimilarResults> {
    if seeds.is_empty() {
        bail!("give at least one seed passage id");
    }
    if seeds.len() > MAX_SEEDS {
        bail!("at most {MAX_SEEDS} seed passages (got {})", seeds.len());
    }
    let idf = Idf::load(corpora)?;

    // Seeds: their vectors and how the results name them.
    let mut seed_vecs = Vec::new();
    let mut seed_info = Vec::new();
    for (c, id) in seeds {
        let pid = format!("{}:{}:{}", id.corpus, id.doc_key, id.line_start);
        if seed_vecs.iter().any(|s: &SeedVec| s.id == pid) {
            continue;
        }
        let conn = crate::index::open_existing(&c.config, c.index_path())?;
        let tokens = seed_tokens(&conn, id, &pid)?;
        let docs = load_docs(&conn)?;
        let d = docs
            .get(&id.doc_key)
            .ok_or_else(|| PassageNotFound(pid.clone()))?;
        let (v, norm) = idf.vector(&tokens);
        let v = v
            .into_iter()
            .map(|(t, w)| (t.to_string(), if norm > 0.0 { w / norm } else { 0.0 }))
            .collect();
        seed_info.push(Seed {
            passage_id: pid.clone(),
            corpus: id.corpus.clone(),
            rel_path: id.doc_key.clone(),
            title: d.title.clone(),
            date: d.date.clone(),
        });
        seed_vecs.push(SeedVec { id: pid, v });
    }
    let k = seed_vecs.len() as f64;
    let mut centroid: BTreeMap<String, f64> = BTreeMap::new();
    for s in &seed_vecs {
        for (t, w) in &s.v {
            *centroid.entry(t.clone()).or_insert(0.0) += w / k;
        }
    }
    let c_norm = centroid.values().map(|w| w * w).sum::<f64>().sqrt();

    let centroid: FxMap<String, f64> = centroid.into_iter().collect();
    let seed_ids: Vec<&str> = seed_vecs.iter().map(|s| s.id.as_str()).collect();
    let mut scored: Vec<Scored> = Vec::new();
    if c_norm > 0.0 {
        let scan = Scan {
            idf: &idf,
            centroid: &centroid,
            c_norm,
            exclude: if req.exclude_seeds { &seed_ids } else { &[] },
        };
        // One thread per corpus; each opens its own read-only connection.
        let parts = std::thread::scope(|sc| {
            let handles: Vec<_> = corpora
                .iter()
                .enumerate()
                .map(|(ci, c)| {
                    let scan = &scan;
                    sc.spawn(move || scan.corpus(ci, c))
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().expect("similar: scan thread panicked"))
                .collect::<Result<Vec<_>>>()
        })?;
        scored = parts.into_iter().flatten().collect();
    }
    let total_passages = scored.len();
    scored.sort_by(|a, b| b.score.total_cmp(&a.score).then_with(|| a.pid.cmp(&b.pid)));
    scored.truncate(req.top);

    // Open each corpus with results once, for its documents and the text.
    let mut open: BTreeMap<usize, (Connection, BTreeMap<String, DocInfo>)> = BTreeMap::new();
    let mut results = Vec::with_capacity(scored.len());
    for s in scored {
        let c = &corpora[s.corpus];
        let (conn, docs) = match open.entry(s.corpus) {
            std::collections::btree_map::Entry::Occupied(o) => o.into_mut(),
            std::collections::btree_map::Entry::Vacant(v) => {
                let conn = crate::index::open_existing(&c.config, c.index_path())?;
                let docs = load_docs(&conn)?;
                v.insert((conn, docs))
            }
        };
        let Some(d) = docs.get(&s.rel_path) else {
            continue;
        };
        let (original, tokens): (String, String) = conn.query_row(
            "SELECT p.original, f.tokens FROM passages p
             JOIN passages_fts f ON f.rowid = p.id WHERE p.id = ?1",
            [s.rowid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let (v, p_norm) = idf.vector(&tokens);
        let (shared_terms, closest_seed) = explain(&v, p_norm, &centroid, &seed_vecs);
        let start = cite::skip_block_marker(&original, 0, original.len());
        let quote = original[start..].trim_end().to_string();
        let line = s.line_start + original[..start].matches('\n').count();
        let link = c.config.link_for(&s.rel_path, line);
        results.push(Suggestion {
            citation: citation(&c.config, d, &quote, link.as_deref()),
            passage_id: s.pid,
            corpus: c.id().to_string(),
            path: c.config.source_path(&s.rel_path).display().to_string(),
            rel_path: s.rel_path,
            line_start: s.line_start,
            line_end: s.line_end,
            quote,
            score: s.score,
            shared_terms,
            closest_seed,
            title: d.title.clone(),
            author: d.author.clone(),
            date: d.date.clone(),
            date_display: d.date.as_deref().and_then(cite::format_date),
            date_source: d.date_source.clone(),
            genre: d.genre.clone(),
            kind: d.kind.clone(),
            source: d.source.clone(),
            link,
            public_url: crate::search::public_url(&c.config, d),
        });
    }
    Ok(SimilarResults {
        schema_version: SCHEMA_VERSION,
        seeds: seed_info,
        corpora: corpora.iter().map(|c| c.id().to_string()).collect(),
        total_passages,
        results,
    })
}

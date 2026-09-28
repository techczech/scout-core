//! Semantic and hybrid search over any corpus (ticket 07).
//!
//! - Local only: an [`Embedder`] runs in-process; nothing leaves the machine.
//! - One vector per passage, in `<corpus>.vectors.sqlite` beside the FTS
//!   index ([`store`]); built by `scout index build --semantic`, then kept
//!   up to date by every index build that has an embedder.
//! - Semantic: passages ranked by cosine to the query, grouped by document
//!   (a document scores its best passage). Hits quote the whole passage's
//!   original text (invariant 2); ties break by passage id (invariant 6).
//! - Hybrid: the full-text ranking and the semantic ranking of documents,
//!   fused by reciprocal rank (score = Σ 1 / (60 + rank)), so a document
//!   holding the words and meaning the same thing comes first. Its hits are
//!   the full-text hits, then the semantic ones.
//! - Query fields (`in:`, `y:`, `lang:`, `tag:`, `-x`, `/re/` …) filter
//!   semantic hits as they filter full-text ones; only the free words are
//!   embedded.

pub mod embed;
pub mod store;

pub use embed::{Embedder, HashEmbedder};
pub use store::{Freshness, VectorReport, Vectors};

use crate::normalize;
use crate::registry::CorpusConfig;
use crate::search::{self, doc_result, locate, passage_hit, PassageRow, Plan, SearchRequest};
use crate::search::{DocResult, SearchResults};
use anyhow::Result;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Reciprocal-rank-fusion constant (the usual 60).
pub const RRF_K: f64 = 60.0;
/// Documents each ranking contributes to a hybrid fusion.
pub const HYBRID_POOL: usize = 100;

/// How `search` ranks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchMode {
    /// Hybrid for a corpus with current vectors (when the engine has an
    /// embedder), else full text.
    #[default]
    Auto,
    Fts,
    Semantic,
    Hybrid,
}

impl std::str::FromStr for SearchMode {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        Ok(match s {
            "auto" => SearchMode::Auto,
            "fts" => SearchMode::Fts,
            "semantic" => SearchMode::Semantic,
            "hybrid" => SearchMode::Hybrid,
            _ => anyhow::bail!("unknown search mode {s:?} (auto, fts, semantic, hybrid)"),
        })
    }
}

/// The free words of a query, as typed (stopwords kept: they carry meaning
/// for an embedding model), without fields, exclusions or regexes.
pub fn query_text(query: &str) -> String {
    let q = scout_query::parse_query(&normalize::unify_quotes_str(query));
    q.any_of
        .iter()
        .flatten()
        .map(|t| t.text.trim())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn round6(x: f64) -> f64 {
    (x * 1e6).round() / 1e6
}

/// Semantic search of one corpus: the top `req.limit` documents by their
/// best passage's cosine to `qvec`.
pub fn semantic_corpus(
    conn: &Connection,
    cfg: &CorpusConfig,
    req: &SearchRequest,
    vectors: &Vectors,
    qvec: &[f32],
) -> Result<SearchResults> {
    let plan = Plan::parse(&req.query)?;
    let docs = crate::filter::filtered_docs(conn, &plan.filter)?;
    let mut excluded: BTreeSet<i64> = BTreeSet::new();
    for t in &plan.not {
        excluded.extend(search::fts_rowids(conn, "passages_fts", t)?);
    }
    let rules = normalize::compile_rules(&cfg.boilerplate)?;
    let mut get = conn.prepare_cached(
        "SELECT rel_path, line_start, line_end, original, tags, color, saved_at FROM passages WHERE id = ?1",
    )?;
    // Walk the ranking until `limit` documents are full or the next passage
    // would open one more.
    let per_doc = req.hits_per_doc.max(1);
    let mut order: Vec<String> = Vec::new();
    let mut by_doc: HashMap<String, Vec<(f64, crate::search::PassageHit)>> = HashMap::new();
    let mut kept = 0usize;
    for (score, id) in vectors.rank(qvec) {
        if excluded.contains(&id) {
            continue;
        }
        type R = (
            String,
            i64,
            i64,
            String,
            String,
            Option<String>,
            Option<String>,
        );
        let row: Option<R> = match get.query_row(params![id], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
            ))
        }) {
            Ok(r) => Some(r),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(e) => return Err(e.into()),
        };
        let Some((rel, line_start, line_end, original, tags, color, saved_at)) = row else {
            continue;
        };
        let Some(doc) = docs.get(&rel) else { continue };
        let tag_list: Vec<String> = serde_json::from_str(&tags).unwrap_or_default();
        if !plan.passage_ok(doc, &tag_list, color.as_deref(), &original) {
            continue;
        }
        if !by_doc.contains_key(&rel) {
            if order.len() == req.limit {
                break;
            }
            order.push(rel.clone());
        }
        let hits = by_doc.entry(rel.clone()).or_default();
        kept += 1;
        if hits.len() >= per_doc {
            continue;
        }
        let loc = locate(&original, &rules, &[], true);
        let s = round6(score as f64);
        let mut hit = passage_hit(
            cfg,
            &rel,
            PassageRow {
                line_start,
                line_end,
                original: &original,
                tags: &tags,
                color,
                saved_at,
            },
            &loc,
            s,
            vec![],
        );
        hit.semantic_score = Some(s);
        hits.push((s, hit));
    }
    let top = order
        .first()
        .and_then(|r| by_doc.get(r))
        .map(|h| h[0].0)
        .unwrap_or(0.0);
    let mut results = Vec::new();
    for rel in &order {
        let hits = by_doc.remove(rel).unwrap_or_default();
        let score = hits[0].0;
        let n = hits.len();
        let hits = hits.into_iter().map(|(_, h)| h).collect();
        results.push(doc_result(cfg, rel, &docs[rel], hits, score, top, false, n));
    }
    Ok(SearchResults {
        schema_version: search::SCHEMA_VERSION,
        query: req.query.clone(),
        terms: plan.terms(),
        plan,
        corpora: vec![cfg.id.clone()],
        total_documents: results.len(),
        total_passages: kept,
        results,
        mode: Some("semantic".into()),
    })
}

/// Hybrid: fuse a full-text and a semantic ranking of one corpus's
/// documents by reciprocal rank, keeping the top `limit`.
pub fn fuse(
    fts: SearchResults,
    sem: SearchResults,
    limit: usize,
    hits_per_doc: usize,
) -> SearchResults {
    let mut rrf: BTreeMap<String, f64> = BTreeMap::new();
    for (i, d) in fts.results.iter().enumerate() {
        *rrf.entry(d.rel_path.clone()).or_default() += 1.0 / (RRF_K + (i + 1) as f64);
    }
    for (i, d) in sem.results.iter().enumerate() {
        *rrf.entry(d.rel_path.clone()).or_default() += 1.0 / (RRF_K + (i + 1) as f64);
    }
    let sem_only_total = sem
        .results
        .iter()
        .filter(|d| !fts.results.iter().any(|f| f.rel_path == d.rel_path))
        .count();
    let mut sem_docs: HashMap<String, DocResult> = sem
        .results
        .into_iter()
        .map(|d| (d.rel_path.clone(), d))
        .collect();
    let mut merged: Vec<DocResult> = Vec::new();
    for mut d in fts.results {
        if let Some(s) = sem_docs.remove(&d.rel_path) {
            let sem_scores: HashMap<&str, f64> = s
                .hits
                .iter()
                .filter_map(|h| Some((h.passage_id.as_str(), h.semantic_score?)))
                .collect();
            for h in d.hits.iter_mut() {
                h.semantic_score = sem_scores.get(h.passage_id.as_str()).copied();
            }
            let have: BTreeSet<String> = d.hits.iter().map(|h| h.passage_id.clone()).collect();
            let extra: Vec<_> = s
                .hits
                .into_iter()
                .filter(|h| !have.contains(&h.passage_id))
                .collect();
            d.hits.extend(extra);
            d.hits.truncate(hits_per_doc.max(1));
        }
        merged.push(d);
    }
    merged.extend(sem_docs.into_values());
    for d in merged.iter_mut() {
        d.score = round6(rrf[&d.rel_path]);
    }
    merged.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then(a.rel_path.cmp(&b.rel_path))
    });
    merged.truncate(limit);
    let top = merged.first().map(|d| d.score).unwrap_or(0.0);
    for d in merged.iter_mut() {
        d.rank = if top > 0.0 {
            round6(d.score / top)
        } else {
            0.0
        };
    }
    SearchResults {
        total_documents: fts.total_documents + sem_only_total,
        total_passages: fts.total_passages,
        results: merged,
        mode: Some("hybrid".into()),
        ..fts
    }
}

/// Hybrid search of one corpus.
pub fn hybrid_corpus(
    conn: &Connection,
    cfg: &CorpusConfig,
    req: &SearchRequest,
    vectors: &Vectors,
    qvec: &[f32],
) -> Result<SearchResults> {
    let mut pool = req.clone();
    pool.limit = HYBRID_POOL.max(req.limit);
    let fts = search::search_corpus(conn, cfg, &pool)?;
    let sem = semantic_corpus(conn, cfg, &pool, vectors, qvec)?;
    Ok(fuse(fts, sem, req.limit, req.hits_per_doc))
}

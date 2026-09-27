//! The word profile (J5-B): frequency, per million, pieces, first use,
//! per-year distribution, top documents, collocates and n-grams.
//!
//! Every count comes from the one occurrence finder ([`concord::find`]), so
//! `frequency` equals the KWIC total; the collocates reuse the same hits.

use crate::concord::{buckets_of, find, per_million, DistBucket, DistBy};
use crate::filter::DocFilter;
use crate::Corpus;
use anyhow::Result;
use serde::Serialize;
use std::collections::BTreeMap;

/// `scout profile` changed shape in T4 (sections filled, `arrives_in` gone,
/// `top_documents` gated by `min_hits`).
pub const PROFILE_SCHEMA_VERSION: u32 = 2;

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

/// The profile's collocates: a plain logDice list.
#[derive(Debug, Clone, Serialize)]
pub struct ProfileCollocates {
    pub status: &'static str,
    pub window: usize,
    pub score: crate::colloc::Score,
    pub min_freq: u64,
    pub keep_stopwords: bool,
    pub items: Vec<crate::colloc::CollocateRow>,
}

/// The profile's n-grams: the top n-grams containing the word.
#[derive(Debug, Clone, Serialize)]
pub struct ProfileNgrams {
    pub status: &'static str,
    pub n_min: usize,
    pub n_max: usize,
    pub items: Vec<crate::ngrams::Ngram>,
}

#[derive(Debug, Clone)]
pub struct ProfileOptions {
    /// A document needs at least this many hits to be a top document.
    pub min_hits: usize,
    pub top_documents: usize,
    pub collocates_top: usize,
    pub ngrams_top: usize,
}

impl Default for ProfileOptions {
    fn default() -> Self {
        ProfileOptions {
            min_hits: 3,
            top_documents: PROFILE_TOP_DOCUMENTS,
            collocates_top: 15,
            ngrams_top: 10,
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
    /// The `min_hits` gate on `top_documents`.
    pub top_documents_min_hits: usize,
    /// Top documents with at least `min_hits` hits, by relative frequency
    /// (per million), then hits, then corpus and path.
    pub top_documents: Vec<DocFreq>,
    /// Top collocates by logDice (window 5, min 5, stopwords dropped).
    pub collocates: ProfileCollocates,
    /// Top 3–5-grams containing the word.
    pub ngrams: ProfileNgrams,
}

pub const PROFILE_TOP_DOCUMENTS: usize = 10;

pub fn profile(corpora: &[Corpus], word: &str, filter: &DocFilter) -> Result<Profile> {
    profile_with(corpora, word, filter, &ProfileOptions::default())
}

pub fn profile_with(
    corpora: &[Corpus],
    word: &str,
    filter: &DocFilter,
    opts: &ProfileOptions,
) -> Result<Profile> {
    let f = find(corpora, word, filter, None)?;
    let total_tokens = f.total_tokens();
    let first = f
        .hits
        .iter()
        .copied()
        .filter(|h| f.passages[h.p as usize].date_floor.is_some())
        .min_by(|&a, &b| f.cmp_date(a, b).then_with(|| f.cmp_source(a, b)));
    let first_used = match first {
        None => None,
        Some(h) => f.lines(&[h], 0)?.pop().map(|l| FirstUse {
            corpus: l.corpus,
            passage_id: l.passage_id,
            rel_path: l.rel_path,
            title: l.title,
            date: l.date,
            date_display: l.date_display,
            line: l.line,
        }),
    };
    let mut per_doc: BTreeMap<(String, String), usize> = BTreeMap::new();
    for &h in &f.hits {
        *per_doc
            .entry((f.corpus_id(h).to_string(), f.doc(h).rel_path.clone()))
            .or_default() += 1;
    }
    let mut top: Vec<DocFreq> = per_doc
        .into_iter()
        .filter(|(_, hits)| *hits >= opts.min_hits)
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
    top.truncate(opts.top_documents);
    let frequency = f.len();
    let pieces = f.pieces();
    let by_year = buckets_of(&f, DistBy::Year);
    let node = f.node.clone();
    let corpus_ids = f.corpus_ids();

    let mut creq = crate::colloc::CollocRequest::new(word);
    creq.filter = filter.clone();
    creq.top = opts.collocates_top;
    let coll = crate::colloc::from_hits(f, &creq)?;
    let nreq = crate::ngrams::NgramRequest {
        n_min: 3,
        n_max: 5,
        filter: filter.clone(),
        top: opts.ngrams_top,
        containing: Some(node.clone()),
        ..crate::ngrams::NgramRequest::default()
    };
    let grams = if node.len() <= nreq.n_max {
        crate::ngrams::ngrams(corpora, &nreq)?.grams
    } else {
        vec![]
    };
    Ok(Profile {
        schema_version: PROFILE_SCHEMA_VERSION,
        word: word.to_string(),
        node_tokens: node,
        corpora: corpus_ids,
        filters: filter.clone(),
        frequency,
        total_tokens,
        per_million: per_million(frequency, total_tokens),
        pieces,
        first_used,
        by_year,
        top_documents_min_hits: opts.min_hits,
        top_documents: top,
        collocates: ProfileCollocates {
            status: "ok",
            window: creq.window,
            score: creq.score,
            min_freq: creq.min_freq,
            keep_stopwords: creq.keep_stopwords,
            items: coll.rows,
        },
        ngrams: ProfileNgrams {
            status: "ok",
            n_min: nreq.n_min,
            n_max: nreq.n_max,
            items: grams,
        },
    })
}

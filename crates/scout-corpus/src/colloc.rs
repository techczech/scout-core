//! Collocates (J5): which words keep company with a node, and the same list
//! over two slices side by side (`--compare`).
//!
//! Definitions (spec, "Analytics definitions"):
//! - the node's occurrences are the KWIC hits ([`concord::find`]);
//! - the window is `W` tokens left and right of the node inside one passage;
//!   the node's own tokens are not in it;
//! - `f(n,c)` = the node occurrences with `c` at least once in the window, so
//!   a row's `count` equals the lines of `scout kwic <node> --near c
//!   --window W` over the same corpora and filters (invariant 3);
//! - `f(n)` = node occurrences; `f(c)` = occurrences of `c` in the filtered
//!   documents; `N` = their tokens; `W₂` = the window span, `2·W`;
//! - logDice = 14 + log2(2·f(n,c) / (f(n) + f(c)));
//!   MI = log2(f(n,c)·N / (f(n)·f(c)·W₂)).
//!
//! Stopwords: by default a built-in English stopword is never listed as a
//! collocate (the node itself may be one). Function words dominate every
//! window, and the stopword-rich phrases they form are what the n-gram view
//! is for; `keep_stopwords` lists them.

use crate::concord::{self, Hit, Hits, Near};
use crate::filter::DocFilter;
use crate::rowdist::{Place, RowDist, RowDistAcc, RowDistBy};
use crate::stopwords::is_stopword;
use crate::{Corpus, Slice};
use anyhow::{bail, Result};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Score {
    LogDice,
    Mi,
}

impl FromStr for Score {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        Ok(match s.trim().to_ascii_lowercase().as_str() {
            "logdice" => Score::LogDice,
            "mi" => Score::Mi,
            _ => bail!("bad --score {s:?}: use logdice or mi"),
        })
    }
}

impl std::fmt::Display for Score {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Score::LogDice => "logdice",
            Score::Mi => "mi",
        })
    }
}

#[derive(Debug, Clone)]
pub struct CollocRequest {
    pub node: String,
    pub window: usize,
    pub score: Score,
    /// Minimum `f(n,c)`.
    pub min_freq: u64,
    pub top: usize,
    pub filter: DocFilter,
    pub keep_stopwords: bool,
    pub dist: Option<RowDistBy>,
}

impl CollocRequest {
    pub fn new(node: impl Into<String>) -> Self {
        CollocRequest {
            node: node.into(),
            window: 5,
            score: Score::LogDice,
            min_freq: 5,
            top: 30,
            filter: DocFilter::default(),
            keep_stopwords: false,
            dist: None,
        }
    }
}

/// A collocate row's concordance link.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct KwicLink {
    pub near: Near,
    /// The `scout kwic` command that lists exactly `count` lines.
    pub command: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CollocateRow {
    pub collocate: String,
    /// f(n,c): node occurrences with the collocate in the window.
    pub count: u64,
    /// f(c) in the filtered documents.
    pub collocate_freq: u64,
    pub logdice: f64,
    pub mi: f64,
    pub kwic: KwicLink,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dist: Option<RowDist>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CollocResults {
    pub schema_version: u32,
    pub node: String,
    pub node_tokens: Vec<String>,
    pub corpora: Vec<String>,
    pub filters: DocFilter,
    pub window: usize,
    /// W₂ in the MI formula: `2 · window`.
    pub window_span: usize,
    pub score: Score,
    pub min_freq: u64,
    pub keep_stopwords: bool,
    /// f(n).
    pub node_freq: u64,
    /// N.
    pub total_tokens: u64,
    /// Collocates reaching `min_freq`, before `top`.
    pub candidates: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dist: Option<RowDistBy>,
    /// Order: the chosen score desc, then count desc, then collocate asc.
    pub rows: Vec<CollocateRow>,
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

/// logDice = 14 + log2(2·f(n,c) / (f(n) + f(c))), three decimals.
pub fn log_dice(f_nc: u64, f_n: u64, f_c: u64) -> f64 {
    round3(14.0 + (2.0 * f_nc as f64 / (f_n + f_c) as f64).log2())
}

/// MI = log2(f(n,c)·N / (f(n)·f(c)·W₂)), three decimals.
pub fn mutual_information(f_nc: u64, f_n: u64, f_c: u64, n: u64, span: usize) -> f64 {
    round3((f_nc as f64 * n as f64 / (f_n as f64 * f_c as f64 * span as f64)).log2())
}

fn shell_word(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_alphanumeric() || "-_.,:/".contains(c))
    {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// `scout kwic` flags for corpora and filters, as the CLI spells them.
pub fn scope_flags(corpora: &[String], f: &DocFilter) -> String {
    let mut out = format!(" --in {}", corpora.join(","));
    if !f.lang.is_empty() {
        out.push_str(&format!(" --lang {}", shell_word(&f.lang.join(","))));
    }
    if !f.genre.is_empty() {
        out.push_str(&format!(" --genre {}", shell_word(&f.genre.join(","))));
    }
    if let Some(a) = &f.after {
        out.push_str(&format!(" --after {}", shell_word(a)));
    }
    if let Some(b) = &f.before {
        out.push_str(&format!(" --before {}", shell_word(b)));
    }
    if let Some(y) = f.year {
        out.push_str(&format!(" --y {y}"));
    }
    out
}

fn link(node: &str, c: &str, window: usize, corpora: &[String], f: &DocFilter) -> KwicLink {
    KwicLink {
        near: Near {
            word: c.to_string(),
            window,
        },
        command: format!(
            "scout kwic {} --near {} --window {window}{}",
            shell_word(node),
            shell_word(c),
            scope_flags(corpora, f)
        ),
    }
}

/// Node occurrences plus, per occurrence, the distinct collocate ids in its
/// window (stopwords dropped unless kept).
struct Side {
    hits: Hits,
    windows: Vec<Vec<u32>>,
    /// Hit vocab id → f(n,c).
    pair: HashMap<u32, u64>,
}

fn side(hits: Hits, window: usize, keep_stopwords: bool) -> Side {
    let stop: Vec<bool> = hits.vocab.iter().map(|t| is_stopword(t)).collect();
    let mut windows = Vec::with_capacity(hits.hits.len());
    let mut pair: HashMap<u32, u64> = HashMap::new();
    for &h in &hits.hits {
        let (l, r) = hits.window(h, window);
        let mut ids: Vec<u32> = l
            .iter()
            .chain(r)
            .copied()
            .filter(|&id| keep_stopwords || !stop[id as usize])
            .collect();
        ids.sort_unstable();
        ids.dedup();
        for &id in &ids {
            *pair.entry(id).or_default() += 1;
        }
        windows.push(ids);
    }
    Side {
        hits,
        windows,
        pair,
    }
}

fn place(hits: &Hits, h: Hit) -> Place {
    let d = hits.doc(h);
    Place {
        corpus: hits.corpus_id(h).to_string(),
        rel: d.rel_path.clone(),
        title: d.title.clone(),
        year: d
            .year()
            .map(|y| y.to_string())
            .unwrap_or_else(|| concord::UNKNOWN.into()),
    }
}

fn check(req: &CollocRequest) -> Result<()> {
    if req.window == 0 {
        bail!("--window must be at least 1");
    }
    Ok(())
}

/// Collocate rows from already-found node occurrences (the profile reuses
/// its own hits).
pub(crate) fn from_hits(hits: Hits, req: &CollocRequest) -> Result<CollocResults> {
    check(req)?;
    let corpora = hits.corpora.clone();
    let s = side(hits, req.window, req.keep_stopwords);
    let f_n = s.hits.len() as u64;
    let n_tokens = s.hits.total_tokens() as u64;
    let mut cands: Vec<(u32, u64)> = s
        .pair
        .iter()
        .filter(|(_, &c)| c >= req.min_freq.max(1))
        .map(|(&id, &c)| (id, c))
        .collect();
    let words: Vec<&str> = cands
        .iter()
        .map(|(id, _)| s.hits.vocab[*id as usize].as_str())
        .collect();
    let fc = crate::freq::counts_of(&corpora, &req.filter, &words)?;
    let span = 2 * req.window;
    let corpus_ids = s.hits.corpus_ids();
    let mut rows: Vec<(u32, CollocateRow)> = cands
        .drain(..)
        .map(|(id, count)| {
            let w = &s.hits.vocab[id as usize];
            let f_c = fc[w.as_str()];
            (
                id,
                CollocateRow {
                    collocate: w.clone(),
                    count,
                    collocate_freq: f_c,
                    logdice: log_dice(count, f_n, f_c),
                    mi: mutual_information(count, f_n, f_c, n_tokens, span),
                    kwic: link(&req.node, w, req.window, &corpus_ids, &req.filter),
                    dist: None,
                },
            )
        })
        .collect();
    let key = |r: &CollocateRow| match req.score {
        Score::LogDice => r.logdice,
        Score::Mi => r.mi,
    };
    rows.sort_by(|a, b| {
        key(&b.1)
            .total_cmp(&key(&a.1))
            .then(b.1.count.cmp(&a.1.count))
            .then_with(|| a.1.collocate.cmp(&b.1.collocate))
    });
    let candidates = rows.len();
    rows.truncate(req.top);
    if let Some(by) = req.dist {
        let chosen: HashMap<u32, usize> = rows
            .iter()
            .enumerate()
            .map(|(i, (id, _))| (*id, i))
            .collect();
        let mut accs = vec![RowDistAcc::default(); rows.len()];
        for (k, &h) in s.hits.hits.iter().enumerate() {
            let mut at = None;
            for id in &s.windows[k] {
                if let Some(&i) = chosen.get(id) {
                    let p = at.get_or_insert_with(|| place(&s.hits, h));
                    accs[i].add(by, p, 1);
                }
            }
        }
        for ((_, r), acc) in rows.iter_mut().zip(accs) {
            r.dist = Some(acc.finish(by, &corpus_ids));
        }
    }
    Ok(CollocResults {
        schema_version: SCHEMA_VERSION,
        node: req.node.clone(),
        node_tokens: s.hits.node.clone(),
        corpora: corpus_ids,
        filters: req.filter.clone(),
        window: req.window,
        window_span: span,
        score: req.score,
        min_freq: req.min_freq,
        keep_stopwords: req.keep_stopwords,
        node_freq: f_n,
        total_tokens: n_tokens,
        candidates,
        dist: req.dist,
        rows: rows.into_iter().map(|(_, r)| r).collect(),
    })
}

pub fn collocates(corpora: &[Corpus], req: &CollocRequest) -> Result<CollocResults> {
    check(req)?;
    let hits = concord::find(corpora, &req.node, &req.filter, None)?;
    from_hits(hits, req)
}

// ---------- compare ----------

#[derive(Debug, Clone, Serialize)]
pub struct CompareSide {
    /// The slice expression (`--after 2020` for the main scope).
    pub spec: String,
    pub corpora: Vec<String>,
    pub filters: DocFilter,
    pub node_freq: u64,
    pub total_tokens: u64,
}

/// One collocate on one side. `count` can be below `min_freq` (or zero) on
/// the side where the row did not qualify; scores are `null` at zero.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SideStats {
    pub count: u64,
    pub collocate_freq: u64,
    pub logdice: Option<f64>,
    pub mi: Option<f64>,
    pub kwic: KwicLink,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CompareRow {
    pub collocate: String,
    pub a: SideStats,
    pub b: SideStats,
    /// (a.count / a.node_freq) / (b.count / b.node_freq): how much more often
    /// the node has this collocate in A than in B. `null` when b.count is 0.
    pub ratio: Option<f64>,
    /// score(A) − score(B) in the chosen score; `null` when a side is 0.
    pub score_diff: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompareResults {
    pub schema_version: u32,
    pub node: String,
    pub node_tokens: Vec<String>,
    pub window: usize,
    pub window_span: usize,
    pub score: Score,
    pub min_freq: u64,
    pub keep_stopwords: bool,
    pub a: CompareSide,
    pub b: CompareSide,
    /// Collocates reaching `min_freq` on at least one side, before `top`.
    pub candidates: usize,
    /// Order: the higher of the two scores desc, then a.count + b.count desc,
    /// then collocate asc (symmetric under swapping A and B).
    pub rows: Vec<CompareRow>,
}

/// The same node, window and score over two slices, side by side. `req.filter`
/// is ignored: each slice carries its own.
pub fn compare(a: &Slice, b: &Slice, req: &CollocRequest) -> Result<CompareResults> {
    check(req)?;
    let sa = side(
        concord::find(&a.corpora, &req.node, &a.filter, None)?,
        req.window,
        req.keep_stopwords,
    );
    let sb = side(
        concord::find(&b.corpora, &req.node, &b.filter, None)?,
        req.window,
        req.keep_stopwords,
    );
    let words_of = |s: &Side| -> BTreeMap<String, u64> {
        s.pair
            .iter()
            .map(|(&id, &c)| (s.hits.vocab[id as usize].clone(), c))
            .collect()
    };
    let (pa, pb) = (words_of(&sa), words_of(&sb));
    let min = req.min_freq.max(1);
    let mut union: Vec<&str> = pa
        .iter()
        .chain(pb.iter())
        .filter(|(_, &c)| c >= min)
        .map(|(w, _)| w.as_str())
        .collect();
    union.sort_unstable();
    union.dedup();
    let fa = crate::freq::counts_of(&a.corpora, &a.filter, &union)?;
    let fb = crate::freq::counts_of(&b.corpora, &b.filter, &union)?;
    let span = 2 * req.window;
    let stat =
        |s: &Side, p: &BTreeMap<String, u64>, f: &HashMap<String, u64>, sl: &Slice, w: &str| {
            let count = p.get(w).copied().unwrap_or(0);
            let f_c = f.get(w).copied().unwrap_or(0);
            let f_n = s.hits.len() as u64;
            let n = s.hits.total_tokens() as u64;
            let ok = count > 0;
            SideStats {
                count,
                collocate_freq: f_c,
                logdice: ok.then(|| log_dice(count, f_n, f_c)),
                mi: ok.then(|| mutual_information(count, f_n, f_c, n, span)),
                kwic: link(&req.node, w, req.window, &s.hits.corpus_ids(), &sl.filter),
            }
        };
    let pick = |s: &SideStats| match req.score {
        Score::LogDice => s.logdice,
        Score::Mi => s.mi,
    };
    let (na, nb) = (sa.hits.len() as f64, sb.hits.len() as f64);
    let mut rows: Vec<CompareRow> = union
        .iter()
        .map(|w| {
            let x = stat(&sa, &pa, &fa, a, w);
            let y = stat(&sb, &pb, &fb, b, w);
            let ratio = (y.count > 0 && na > 0.0)
                .then(|| round3((x.count as f64 / na) / (y.count as f64 / nb)));
            let score_diff = match (pick(&x), pick(&y)) {
                (Some(p), Some(q)) => Some(round3(p - q)),
                _ => None,
            };
            CompareRow {
                collocate: w.to_string(),
                a: x,
                b: y,
                ratio,
                score_diff,
            }
        })
        .collect();
    let best = |r: &CompareRow| {
        let (p, q) = (pick(&r.a), pick(&r.b));
        p.unwrap_or(f64::NEG_INFINITY)
            .max(q.unwrap_or(f64::NEG_INFINITY))
    };
    rows.sort_by(|x, y| {
        best(y)
            .total_cmp(&best(x))
            .then((y.a.count + y.b.count).cmp(&(x.a.count + x.b.count)))
            .then_with(|| x.collocate.cmp(&y.collocate))
    });
    let candidates = rows.len();
    rows.truncate(req.top);
    let side_out = |s: &Side, sl: &Slice| CompareSide {
        spec: sl.label.clone(),
        corpora: s.hits.corpus_ids(),
        filters: sl.filter.clone(),
        node_freq: s.hits.len() as u64,
        total_tokens: s.hits.total_tokens() as u64,
    };
    Ok(CompareResults {
        schema_version: SCHEMA_VERSION,
        node: req.node.clone(),
        node_tokens: sa.hits.node.clone(),
        window: req.window,
        window_span: span,
        score: req.score,
        min_freq: req.min_freq,
        keep_stopwords: req.keep_stopwords,
        a: side_out(&sa, a),
        b: side_out(&sb, b),
        candidates,
        rows,
    })
}

//! N-grams: a full scan of the indexed token streams, within passages.
//!
//! Counting is independent of the concordance path (it reads the stored
//! passage tokens, not the FTS match), so invariant 3 is a real check: for a
//! word and filters, the n=1 count here equals the KWIC total.
//!
//! Stopword rule (amended 2026-09-27): by default only grams made entirely of
//! stopwords are dropped, which keeps phrases such as "at the same time".
//! `strict_stopwords` drops grams with at least n−1 stopwords (for n=1, any
//! stopword).
//!
//! Memory: grams are counted one n at a time over interned token ids packed
//! into a `u128`; each n keeps only its top candidates before the next n is
//! counted. The global top K is always inside the union of the per-n top K
//! under the same order, so the result is exact.

use crate::filter::DocFilter;
use crate::rowdist::{Place, RowDist, RowDistAcc, RowDistBy};
use crate::stopwords::is_stopword;
use crate::Corpus;
use anyhow::{bail, Result};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};

pub const SCHEMA_VERSION: u32 = 1;
pub const MAX_N: usize = 5;
const ID_BITS: u32 = 24;

#[derive(Debug, Clone)]
pub struct NgramRequest {
    pub n_min: usize,
    pub n_max: usize,
    pub filter: DocFilter,
    /// Inclusive year bounds. When either is set, undated documents are left
    /// out and every returned gram carries its per-year counts.
    pub since: Option<i32>,
    pub until: Option<i32>,
    pub top: usize,
    pub strict_stopwords: bool,
    /// Keep only grams containing this token sequence (the profile's
    /// "n-grams containing the word", and the invariant-3 check at n=1).
    pub containing: Option<Vec<String>>,
    /// Per-row distribution (`--dist year|doc|corpus`).
    pub dist: Option<RowDistBy>,
}

impl Default for NgramRequest {
    fn default() -> Self {
        NgramRequest {
            n_min: 3,
            n_max: 5,
            filter: DocFilter::default(),
            since: None,
            until: None,
            top: 50,
            strict_stopwords: false,
            containing: None,
            dist: None,
        }
    }
}

/// Parse `3-5` or `4` into an inclusive range within 1..=5.
pub fn parse_n_range(s: &str) -> Result<(usize, usize)> {
    let (a, b) = match s.split_once('-') {
        Some((a, b)) => (a.trim().parse::<usize>(), b.trim().parse::<usize>()),
        None => (s.trim().parse::<usize>(), s.trim().parse::<usize>()),
    };
    match (a, b) {
        (Ok(a), Ok(b)) if 1 <= a && a <= b && b <= MAX_N => Ok((a, b)),
        _ => bail!("bad --n {s:?}: use a range like 3-5 within 1-{MAX_N}"),
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Ngram {
    pub gram: String,
    pub n: usize,
    pub count: u64,
    /// Distinct documents containing the gram.
    pub pieces: usize,
    pub per_million: f64,
    /// Per-year counts, only when `since` or `until` is set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by_year: Option<BTreeMap<String, u64>>,
    /// Per-bucket counts summing to `count`, only with `dist`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dist: Option<RowDist>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NgramResults {
    pub schema_version: u32,
    pub corpora: Vec<String>,
    pub filters: DocFilter,
    pub n_min: usize,
    pub n_max: usize,
    pub since: Option<i32>,
    pub until: Option<i32>,
    pub strict_stopwords: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub containing: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dist: Option<RowDistBy>,
    /// Tokens in the scanned documents.
    pub total_tokens: u64,
    pub documents: usize,
    /// Order: count desc, then gram asc.
    pub grams: Vec<Ngram>,
}

/// FxHash-style hasher for the packed `u128` keys.
#[derive(Default)]
struct Fx(u64);
impl Hasher for Fx {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 = (self.0.rotate_left(5) ^ *b as u64).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
        }
    }
    fn write_u128(&mut self, i: u128) {
        for w in [i as u64, (i >> 64) as u64] {
            self.0 = (self.0.rotate_left(5) ^ w).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
        }
    }
}
type FxMap<K, V> = HashMap<K, V, BuildHasherDefault<Fx>>;

struct Scan {
    vocab: Vec<String>,
    stop: Vec<bool>,
    /// Passages as token-id streams, with their document index.
    passages: Vec<(u32, Vec<u32>)>,
    /// Per document: its year label (for `by_year`).
    doc_year: Vec<String>,
    /// Per document: where it sits (for `dist`).
    doc_place: Vec<Place>,
    total_tokens: u64,
}

fn load(corpora: &[Corpus], req: &NgramRequest) -> Result<Scan> {
    req.filter.validate()?;
    let mut ids: HashMap<String, u32> = HashMap::new();
    let mut scan = Scan {
        vocab: vec![],
        stop: vec![],
        passages: vec![],
        doc_year: vec![],
        doc_place: vec![],
        total_tokens: 0,
    };
    let period = req.since.is_some() || req.until.is_some();
    for c in corpora {
        let conn = crate::index::open_existing(&c.config, c.index_path())?;
        let mut docs = crate::filter::filtered_docs(&conn, &req.filter)?;
        if period {
            docs.retain(|_, d| match d.year() {
                None => false,
                Some(y) => req.since.is_none_or(|s| y >= s) && req.until.is_none_or(|u| y <= u),
            });
        }
        let mut doc_idx: HashMap<String, u32> = HashMap::new();
        for d in docs.values() {
            doc_idx.insert(d.rel_path.clone(), scan.doc_year.len() as u32);
            let year = d
                .year()
                .map(|y| y.to_string())
                .unwrap_or_else(|| crate::concord::UNKNOWN.into());
            scan.doc_place.push(Place {
                corpus: c.id().to_string(),
                rel: d.rel_path.clone(),
                title: d.title.clone(),
                year: year.clone(),
            });
            scan.doc_year.push(year);
        }
        let mut st = conn.prepare(
            "SELECT p.rel_path, f.tokens FROM passages p
             JOIN passages_fts f ON f.rowid = p.id
             ORDER BY p.rel_path, p.line_start",
        )?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let rel: String = r.get(0)?;
            let Some(&di) = doc_idx.get(&rel) else {
                continue;
            };
            let toks: String = r.get(1)?;
            let mut stream = Vec::new();
            for t in toks.split(' ').filter(|t| !t.is_empty()) {
                let id = match ids.get(t) {
                    Some(&id) => id,
                    None => {
                        let id = scan.vocab.len() as u32;
                        if id >= (1 << ID_BITS) {
                            bail!("vocabulary exceeds {} types", 1u32 << ID_BITS);
                        }
                        ids.insert(t.to_string(), id);
                        scan.vocab.push(t.to_string());
                        scan.stop.push(is_stopword(t));
                        id
                    }
                };
                stream.push(id);
            }
            scan.total_tokens += stream.len() as u64;
            scan.passages.push((di, stream));
        }
    }
    Ok(scan)
}

fn pack(ids: &[u32]) -> u128 {
    let mut k: u128 = ids.len() as u128;
    for &id in ids {
        k = (k << ID_BITS) | id as u128;
    }
    k
}

fn keep(scan: &Scan, gram: &[u32], strict: bool, containing: Option<&[u32]>) -> bool {
    let stops = gram.iter().filter(|&&id| scan.stop[id as usize]).count();
    let n = gram.len();
    let dropped = if strict {
        stops >= n.saturating_sub(1).max(1)
    } else {
        stops == n
    };
    if dropped {
        return false;
    }
    match containing {
        None => true,
        Some(c) => c.len() <= n && gram.windows(c.len()).any(|w| w == c),
    }
}

fn gram_text(scan: &Scan, key: u128) -> (String, usize) {
    let n = unpack_len(key);
    let mask = (1u128 << ID_BITS) - 1;
    let mut ws = Vec::with_capacity(n);
    for i in (0..n).rev() {
        let id = ((key >> (ID_BITS as usize * i)) & mask) as usize;
        ws.push(scan.vocab[id].as_str());
    }
    (ws.join(" "), n)
}

/// The length prefix sits just above the last id; recover it for short grams.
fn unpack_len(key: u128) -> usize {
    for n in 1..=MAX_N {
        if (key >> (ID_BITS as usize * n)) == n as u128 {
            return n;
        }
    }
    unreachable!("malformed gram key")
}

pub fn ngrams(corpora: &[Corpus], req: &NgramRequest) -> Result<NgramResults> {
    if !(1 <= req.n_min && req.n_min <= req.n_max && req.n_max <= MAX_N) {
        bail!(
            "n range {}-{} must lie within 1-{MAX_N}",
            req.n_min,
            req.n_max
        );
    }
    let scan = load(corpora, req)?;
    let mut res = NgramResults {
        schema_version: SCHEMA_VERSION,
        corpora: corpora.iter().map(|c| c.id().to_string()).collect(),
        filters: req.filter.clone(),
        n_min: req.n_min,
        n_max: req.n_max,
        since: req.since,
        until: req.until,
        strict_stopwords: req.strict_stopwords,
        containing: req.containing.clone(),
        dist: req.dist,
        total_tokens: scan.total_tokens,
        documents: scan.doc_year.len(),
        grams: vec![],
    };
    if req.top == 0 {
        return Ok(res);
    }
    // `containing` as ids; an unknown token means no gram can match.
    let containing: Option<Vec<u32>> = match &req.containing {
        None => None,
        Some(ws) => {
            let lookup: HashMap<&str, u32> = scan
                .vocab
                .iter()
                .enumerate()
                .map(|(i, w)| (w.as_str(), i as u32))
                .collect();
            let ids: Option<Vec<u32>> =
                ws.iter().map(|w| lookup.get(w.as_str()).copied()).collect();
            match ids {
                Some(v) if !v.is_empty() => Some(v),
                _ => return Ok(res),
            }
        }
    };

    // Pass 1: count each n, keep its top candidates.
    let mut candidates: Vec<(u64, String, usize, u128)> = Vec::new();
    for n in req.n_min..=req.n_max {
        let mut counts: FxMap<u128, u64> = FxMap::default();
        for (_, s) in &scan.passages {
            if s.len() < n {
                continue;
            }
            for w in s.windows(n) {
                if keep(&scan, w, req.strict_stopwords, containing.as_deref()) {
                    *counts.entry(pack(w)).or_default() += 1;
                }
            }
        }
        if counts.is_empty() {
            continue;
        }
        let mut cs: Vec<u64> = counts.values().copied().collect();
        let k = req.top.min(cs.len());
        let threshold = *cs.select_nth_unstable_by(k - 1, |a, b| b.cmp(a)).1;
        let mut top: Vec<(u64, String, usize, u128)> = counts
            .into_iter()
            .filter(|(_, c)| *c >= threshold)
            .map(|(key, c)| {
                let (g, n) = gram_text(&scan, key);
                (c, g, n, key)
            })
            .collect();
        top.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        top.truncate(req.top);
        candidates.extend(top);
    }
    candidates.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    candidates.truncate(req.top);

    // Pass 2: pieces (and per-year counts) for the chosen grams only.
    let period = req.since.is_some() || req.until.is_some();
    let chosen: HashSet<u128> = candidates.iter().map(|c| c.3).collect();
    let ns: Vec<usize> = {
        let mut v: Vec<usize> = candidates.iter().map(|c| c.2).collect();
        v.sort();
        v.dedup();
        v
    };
    // key -> (last doc seen, pieces, by_year, dist)
    type Extra = (u32, usize, BTreeMap<String, u64>, RowDistAcc);
    let mut extra: FxMap<u128, Extra> = FxMap::default();
    for (di, s) in &scan.passages {
        for &n in &ns {
            if s.len() < n {
                continue;
            }
            for w in s.windows(n) {
                let key = pack(w);
                if !chosen.contains(&key) {
                    continue;
                }
                let e = extra.entry(key).or_insert_with(|| {
                    (u32::MAX, 0, BTreeMap::new(), RowDistAcc::default())
                });
                if e.0 != *di {
                    e.0 = *di;
                    e.1 += 1;
                }
                if period {
                    *e.2.entry(scan.doc_year[*di as usize].clone()).or_default() += 1;
                }
                if let Some(by) = req.dist {
                    e.3.add(by, &scan.doc_place[*di as usize], 1);
                }
            }
        }
    }
    res.grams = candidates
        .into_iter()
        .map(|(count, gram, n, key)| {
            let (pieces, by_year, acc) = extra
                .remove(&key)
                .map(|(_, p, y, a)| (p, y, a))
                .unwrap_or_default();
            Ngram {
                gram,
                n,
                count,
                pieces,
                per_million: if scan.total_tokens == 0 {
                    0.0
                } else {
                    ((count as f64) * 1e6 / scan.total_tokens as f64 * 1000.0).round() / 1000.0
                },
                by_year: period.then_some(by_year),
                dist: req.dist.map(|by| acc.finish(by, &res.corpora)),
            }
        })
        .collect();
    Ok(res)
}

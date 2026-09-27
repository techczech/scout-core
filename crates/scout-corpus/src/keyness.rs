//! Keyness: which words set one slice of the archive apart from another
//! (`scout keyness --a <slice> --b <slice>`).
//!
//! Definition ("Keyness (T4)" in the spec):
//! - two frequency lists of single tokens over the stored token streams (the
//!   same tokens every other view counts); `a`, `b` = a word's counts,
//!   `c`, `d` = the slices' token totals;
//! - log-likelihood G² (Rayson and Garside 2000): E₁ = c·(a+b)/(c+d),
//!   E₂ = d·(a+b)/(c+d), G² = 2·(a·ln(a/E₁) + b·ln(b/E₂)), a zero count
//!   contributing 0;
//! - effect size %DIFF (Gabrielatos and Marchi 2012):
//!   ((a/c − b/d) / (b/d)) · 100, `null` when b = 0;
//! - a word is key for A when a/c > b/d, a ≥ `min_freq` and G² ≥ 3.84
//!   (p < 0.05, 1 d.f.); key for B symmetrically;
//! - each list is ordered by G² desc, then word asc, and cut at `top`.
//!
//! Stopwords stay in: a shift in function words is a finding about style.

use crate::filter::DocFilter;
use crate::Slice;
use anyhow::Result;
use serde::Serialize;
use std::collections::BTreeSet;

pub const SCHEMA_VERSION: u32 = 1;

/// G² at p < 0.05 with one degree of freedom.
pub const G2_CRITICAL: f64 = 3.84;

#[derive(Debug, Clone)]
pub struct KeynessRequest {
    pub top: usize,
    pub min_freq: u64,
}

impl Default for KeynessRequest {
    fn default() -> Self {
        KeynessRequest {
            top: 40,
            min_freq: 5,
        }
    }
}

/// Log-likelihood G² for a word with counts `a`, `b` in slices of `c`, `d`
/// tokens (unrounded).
pub fn log_likelihood(a: u64, b: u64, c: u64, d: u64) -> f64 {
    let (a, b, c, d) = (a as f64, b as f64, c as f64, d as f64);
    if c + d == 0.0 {
        return 0.0;
    }
    let e1 = c * (a + b) / (c + d);
    let e2 = d * (a + b) / (c + d);
    let term = |o: f64, e: f64| {
        if o > 0.0 && e > 0.0 {
            o * (o / e).ln()
        } else {
            0.0
        }
    };
    2.0 * (term(a, e1) + term(b, e2))
}

/// %DIFF = ((a/c − b/d) / (b/d)) · 100; `None` when `b` is 0.
pub fn pct_diff(a: u64, b: u64, c: u64, d: u64) -> Option<f64> {
    if b == 0 || c == 0 || d == 0 {
        return None;
    }
    let (na, nb) = (a as f64 / c as f64, b as f64 / d as f64);
    Some((na - nb) / nb * 100.0)
}

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct KeyWord {
    pub word: String,
    /// Count in A.
    pub a: u64,
    /// Count in B.
    pub b: u64,
    pub a_per_million: f64,
    pub b_per_million: f64,
    pub g2: f64,
    /// Positive for A keys, negative for B keys; `null` when b = 0.
    pub pct_diff: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct KeynessSide {
    pub spec: String,
    pub corpora: Vec<String>,
    pub filters: DocFilter,
    pub documents: usize,
    pub tokens: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct KeynessResults {
    pub schema_version: u32,
    pub a: KeynessSide,
    pub b: KeynessSide,
    /// Documents in both slices. Keyness assumes disjoint slices; a non-zero
    /// overlap dampens every score.
    pub overlap_documents: usize,
    pub min_freq: u64,
    pub g2_critical: f64,
    pub top: usize,
    /// Words overused in A relative to B.
    pub a_keys: Vec<KeyWord>,
    /// Words overused in B relative to A.
    pub b_keys: Vec<KeyWord>,
}

pub fn keyness(a: &Slice, b: &Slice, req: &KeynessRequest) -> Result<KeynessResults> {
    let (fa, c, da) = crate::freq::word_list(&a.corpora, &a.filter)?;
    let (fb, d, db) = crate::freq::word_list(&b.corpora, &b.filter)?;
    let keys = |docs: &crate::freq::DocsByCorpus| -> BTreeSet<(String, String)> {
        docs.iter()
            .flat_map(|(cid, ds)| ds.keys().map(move |r| (cid.clone(), r.clone())))
            .collect()
    };
    let (ka, kb) = (keys(&da), keys(&db));
    let pm = |n: u64, t: u64| {
        if t == 0 {
            0.0
        } else {
            round3(n as f64 * 1e6 / t as f64)
        }
    };
    let mut words: BTreeSet<&str> = fa.keys().map(String::as_str).collect();
    words.extend(fb.keys().map(String::as_str));
    let (mut ak, mut bk) = (Vec::new(), Vec::new());
    for w in words {
        let x = fa.get(w).copied().unwrap_or(0);
        let y = fb.get(w).copied().unwrap_or(0);
        if c == 0 || d == 0 {
            continue;
        }
        let over_a = (x as f64) * (d as f64) > (y as f64) * (c as f64);
        let over_b = (y as f64) * (c as f64) > (x as f64) * (d as f64);
        if !(over_a && x >= req.min_freq || over_b && y >= req.min_freq) {
            continue;
        }
        let g2 = log_likelihood(x, y, c, d);
        if g2 < G2_CRITICAL {
            continue;
        }
        let row = KeyWord {
            word: w.to_string(),
            a: x,
            b: y,
            a_per_million: pm(x, c),
            b_per_million: pm(y, d),
            g2: round3(g2),
            pct_diff: pct_diff(x, y, c, d).map(round3),
        };
        if over_a {
            ak.push(row);
        } else {
            bk.push(row);
        }
    }
    for v in [&mut ak, &mut bk] {
        v.sort_by(|p, q| q.g2.total_cmp(&p.g2).then_with(|| p.word.cmp(&q.word)));
        v.truncate(req.top);
    }
    let side = |s: &Slice, docs: &crate::freq::DocsByCorpus, tokens: u64| KeynessSide {
        spec: s.label.clone(),
        corpora: s.corpora.iter().map(|c| c.id().to_string()).collect(),
        filters: s.filter.clone(),
        documents: docs.values().map(|d| d.len()).sum(),
        tokens,
    };
    Ok(KeynessResults {
        schema_version: SCHEMA_VERSION,
        a: side(a, &da, c),
        b: side(b, &db, d),
        overlap_documents: ka.intersection(&kb).count(),
        min_freq: req.min_freq,
        g2_critical: G2_CRITICAL,
        top: req.top,
        a_keys: ak,
        b_keys: bk,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn g2_matches_a_hand_computed_table() {
        // a=10 in c=1000, b=5 in d=2000: E1 = 5, E2 = 10,
        // G2 = 2(10 ln 2 + 5 ln 0.5) = 10 ln 2 = 6.931472
        let g = log_likelihood(10, 5, 1000, 2000);
        assert!((g - 10.0 * 2f64.ln()).abs() < 1e-9, "{g}");
        // %DIFF = (0.01 - 0.0025) / 0.0025 * 100 = 300
        assert!((pct_diff(10, 5, 1000, 2000).unwrap() - 300.0).abs() < 1e-9);
        assert_eq!(pct_diff(3, 0, 10, 10), None);
        // Equal relative frequencies: G2 = 0.
        assert!(log_likelihood(4, 8, 100, 200).abs() < 1e-12);
        // A zero cell: a=0, b=6, c=d=100: E1=E2=3; G2 = 2(6 ln 2) = 8.317766
        assert!((log_likelihood(0, 6, 100, 100) - 12.0 * 2f64.ln()).abs() < 1e-9);
        // Symmetric under swapping the slices.
        assert!(
            (log_likelihood(10, 5, 1000, 2000) - log_likelihood(5, 10, 2000, 1000)).abs() < 1e-12
        );
    }
}

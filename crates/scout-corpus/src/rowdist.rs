//! Per-row distributions for list views (n-gram rows, collocate rows):
//! `--dist year|doc|corpus`.
//!
//! Invariant: for every row, `sum(buckets[].count) + other.count` equals the
//! row's own count. `doc` keeps the top [`DOC_TOP`] documents and folds the
//! rest into `other`; `year` lists the years with a count (`unknown` last);
//! `corpus` lists every corpus in scope, zero counts included.

use anyhow::{bail, Result};
use serde::Serialize;
use std::collections::BTreeMap;
use std::str::FromStr;

/// Documents listed by `--dist doc`; the rest go to `other`.
pub const DOC_TOP: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RowDistBy {
    Year,
    Doc,
    Corpus,
}

impl FromStr for RowDistBy {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        Ok(match s.trim().to_ascii_lowercase().as_str() {
            "year" => RowDistBy::Year,
            "doc" => RowDistBy::Doc,
            "corpus" => RowDistBy::Corpus,
            _ => bail!("bad --dist {s:?}: use year, doc or corpus"),
        })
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RowBucket {
    /// A year (`unknown` when undated), `<corpus>:<rel_path>`, or a corpus id.
    pub key: String,
    /// The document title (`doc` only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub count: u64,
}

/// What `doc` leaves out of its top list.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RowOther {
    pub documents: usize,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RowDist {
    pub by: RowDistBy,
    pub buckets: Vec<RowBucket>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other: Option<RowOther>,
}

impl RowDist {
    /// `sum(buckets) + other`: always the row's count.
    pub fn total(&self) -> u64 {
        self.buckets.iter().map(|b| b.count).sum::<u64>()
            + self.other.as_ref().map_or(0, |o| o.count)
    }
}

/// Where one occurrence falls: its corpus, document path, title and year.
#[derive(Debug, Clone)]
pub(crate) struct Place {
    pub corpus: String,
    pub rel: String,
    pub title: String,
    /// The year, or `unknown`.
    pub year: String,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct RowDistAcc {
    counts: BTreeMap<String, u64>,
    titles: BTreeMap<String, String>,
}

impl RowDistAcc {
    pub fn add(&mut self, by: RowDistBy, at: &Place, n: u64) {
        let key = match by {
            RowDistBy::Year => at.year.clone(),
            RowDistBy::Doc => format!("{}:{}", at.corpus, at.rel),
            RowDistBy::Corpus => at.corpus.clone(),
        };
        if by == RowDistBy::Doc && !self.titles.contains_key(&key) {
            self.titles.insert(key.clone(), at.title.clone());
        }
        *self.counts.entry(key).or_default() += n;
    }

    pub fn finish(self, by: RowDistBy, corpora: &[String]) -> RowDist {
        let mut counts = self.counts;
        let mut buckets: Vec<RowBucket>;
        let mut other = None;
        match by {
            RowDistBy::Year => {
                buckets = counts
                    .into_iter()
                    .map(|(key, count)| RowBucket {
                        key,
                        title: None,
                        count,
                    })
                    .collect();
                buckets.sort_by(|a, b| {
                    (a.key == crate::concord::UNKNOWN)
                        .cmp(&(b.key == crate::concord::UNKNOWN))
                        .then_with(|| a.key.cmp(&b.key))
                });
            }
            RowDistBy::Doc => {
                let mut v: Vec<(String, u64)> = counts.into_iter().collect();
                v.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
                let rest = v.split_off(v.len().min(DOC_TOP));
                if !rest.is_empty() {
                    other = Some(RowOther {
                        documents: rest.len(),
                        count: rest.iter().map(|r| r.1).sum(),
                    });
                }
                buckets = v
                    .into_iter()
                    .map(|(key, count)| RowBucket {
                        title: self.titles.get(&key).cloned(),
                        key,
                        count,
                    })
                    .collect();
            }
            RowDistBy::Corpus => {
                for c in corpora {
                    counts.entry(c.clone()).or_default();
                }
                buckets = counts
                    .into_iter()
                    .map(|(key, count)| RowBucket {
                        key,
                        title: None,
                        count,
                    })
                    .collect();
                buckets.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.key.cmp(&b.key)));
            }
        }
        RowDist { by, buckets, other }
    }
}

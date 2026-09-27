//! Document filters shared by every analytics view (KWIC, dist, profile,
//! n-grams), so the views count over exactly the same documents
//! (invariant 3).
//!
//! Filters are applied in Rust over the `documents` table (a few thousand
//! rows), never in per-view SQL, so there is one definition of "matches".

use anyhow::{bail, Result};
use rusqlite::Connection;
use serde::Serialize;
use std::collections::BTreeMap;

/// Metadata filters. Empty fields do not filter.
///
/// - `lang`, `genre`: any-of, case-insensitive equality.
/// - `after`: inclusive lower bound; `before`: exclusive upper bound. Both
///   accept `YYYY`, `YYYY-MM` or `YYYY-MM-DD` and mean the first day of that
///   period, as in the `@scout/query` grammar (`after:2016` = from 1 Jan 2016;
///   `before:2016` = up to 31 Dec 2015).
/// - `year`: the document's year equals it.
///
/// A document without a parseable date fails every date filter.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DocFilter {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub lang: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub genre: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub year: Option<i32>,
}

/// One indexed document's metadata.
#[derive(Debug, Clone)]
pub struct DocInfo {
    pub rel_path: String,
    pub title: String,
    pub date: Option<String>,
    pub genre: Option<String>,
    pub lang: Option<String>,
    pub author: Option<String>,
    pub tokens: i64,
}

impl DocInfo {
    pub fn year(&self) -> Option<i32> {
        self.date.as_deref().and_then(year_of)
    }

    /// The date expanded to a full `YYYY-MM-DD` (first day of its period).
    pub fn date_floor(&self) -> Option<String> {
        self.date.as_deref().and_then(date_floor)
    }
}

/// The year of a frontmatter date (`2016`, `2016-06`, `2016-06-23…`).
pub fn year_of(date: &str) -> Option<i32> {
    let y = date.trim().get(..4)?;
    if y.chars().all(|c| c.is_ascii_digit()) {
        y.parse().ok()
    } else {
        None
    }
}

/// Expand a date to `YYYY-MM-DD`, filling a missing month or day with `01`.
pub fn date_floor(date: &str) -> Option<String> {
    let t = date.trim();
    let t = t.split(['T', ' ']).next().unwrap_or(t);
    let parts: Vec<&str> = t.split('-').collect();
    let y = parts.first()?;
    if y.len() != 4 || !y.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let num2 = |s: Option<&&str>| -> Option<String> {
        match s {
            None => Some("01".into()),
            Some(v) if (1..=2).contains(&v.len()) && v.chars().all(|c| c.is_ascii_digit()) => {
                Some(format!("{:0>2}", v))
            }
            _ => None,
        }
    };
    Some(format!(
        "{}-{}-{}",
        y,
        num2(parts.get(1))?,
        num2(parts.get(2))?
    ))
}

impl DocFilter {
    /// Reject malformed date bounds up front (a usage error, not "no hits").
    pub fn validate(&self) -> Result<()> {
        for (name, v) in [("after", &self.after), ("before", &self.before)] {
            if let Some(v) = v {
                if date_floor(v).is_none() {
                    bail!("--{name} {v:?}: expected YYYY, YYYY-MM or YYYY-MM-DD");
                }
            }
        }
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self == &DocFilter::default()
    }

    pub fn matches(&self, d: &DocInfo) -> bool {
        let any_of = |want: &[String], have: &Option<String>| {
            want.is_empty()
                || have
                    .as_deref()
                    .is_some_and(|h| want.iter().any(|w| w.eq_ignore_ascii_case(h)))
        };
        if !any_of(&self.lang, &d.lang) || !any_of(&self.genre, &d.genre) {
            return false;
        }
        if let Some(y) = self.year {
            if d.year() != Some(y) {
                return false;
            }
        }
        if self.after.is_some() || self.before.is_some() {
            let Some(df) = d.date_floor() else {
                return false;
            };
            if let Some(a) = self.after.as_deref().and_then(date_floor) {
                if df < a {
                    return false;
                }
            }
            if let Some(b) = self.before.as_deref().and_then(date_floor) {
                if df >= b {
                    return false;
                }
            }
        }
        true
    }
}

/// Every document in an index, keyed by `rel_path`.
pub fn load_docs(conn: &Connection) -> Result<BTreeMap<String, DocInfo>> {
    let mut st =
        conn.prepare("SELECT rel_path, title, date, genre, lang, author, tokens FROM documents")?;
    let rows = st.query_map([], |r| {
        Ok(DocInfo {
            rel_path: r.get(0)?,
            title: r.get(1)?,
            date: r.get(2)?,
            genre: r.get(3)?,
            lang: r.get(4)?,
            author: r.get(5)?,
            tokens: r.get(6)?,
        })
    })?;
    let mut out = BTreeMap::new();
    for d in rows {
        let d = d?;
        out.insert(d.rel_path.clone(), d);
    }
    Ok(out)
}

/// The documents of an index that pass `f`.
pub fn filtered_docs(conn: &Connection, f: &DocFilter) -> Result<BTreeMap<String, DocInfo>> {
    let mut all = load_docs(conn)?;
    all.retain(|_, d| f.matches(d));
    Ok(all)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floors_partial_dates() {
        assert_eq!(date_floor("2004").as_deref(), Some("2004-01-01"));
        assert_eq!(date_floor("2003-5").as_deref(), Some("2003-05-01"));
        assert_eq!(
            date_floor("2016-06-23T10:00").as_deref(),
            Some("2016-06-23")
        );
        assert_eq!(date_floor("undated"), None);
    }
}

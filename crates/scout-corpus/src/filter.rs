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
/// - `source`: any-of the document's source system (`readwise`, `x`,
///   `zotero`), case-insensitive.
/// - `kind`: any-of the document type (`tweet`, `article`, `book` …); a
///   trailing plural `s` is ignored on both sides, so `ty:articles` (the
///   `@scout/query` shortcut form) matches `article`.
/// - `author`, `title`: case-insensitive substring.
///
/// A document without a parseable date fails every date filter; one
/// without the field fails the field's filter.
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub source: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub kind: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
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
    pub public_url: Option<String>,
    pub tokens: i64,
    pub topics: Vec<String>,
    pub kind: Option<String>,
    pub source: Option<String>,
    pub date_source: Option<String>,
    pub handle: Option<String>,
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

    /// The filter written as a slice expression (`after:2020 lang:en`), the
    /// inverse of [`SliceSpec::parse`]; empty when unfiltered.
    pub fn spec(&self) -> String {
        let mut t = Vec::new();
        if let Some(a) = &self.after {
            t.push(format!("after:{a}"));
        }
        if let Some(b) = &self.before {
            t.push(format!("before:{b}"));
        }
        if let Some(y) = self.year {
            t.push(format!("y:{y}"));
        }
        if !self.lang.is_empty() {
            t.push(format!("lang:{}", self.lang.join(",")));
        }
        if !self.genre.is_empty() {
            t.push(format!("genre:{}", self.genre.join(",")));
        }
        if !self.source.is_empty() {
            t.push(format!("source:{}", self.source.join(",")));
        }
        if !self.kind.is_empty() {
            t.push(format!("ty:{}", self.kind.join(",")));
        }
        for (k, v) in [("au", &self.author), ("ti", &self.title)] {
            if let Some(v) = v {
                if v.contains(char::is_whitespace) {
                    t.push(format!("{k}:\"{v}\""));
                } else {
                    t.push(format!("{k}:{v}"));
                }
            }
        }
        t.join(" ")
    }

    pub fn matches(&self, d: &DocInfo) -> bool {
        let any_of = |want: &[String], have: &Option<String>| {
            want.is_empty()
                || have
                    .as_deref()
                    .is_some_and(|h| want.iter().any(|w| w.eq_ignore_ascii_case(h)))
        };
        if !any_of(&self.lang, &d.lang)
            || !any_of(&self.genre, &d.genre)
            || !any_of(&self.source, &d.source)
        {
            return false;
        }
        if !self.kind.is_empty() {
            let sing = |s: &str| s.trim().to_lowercase().trim_end_matches('s').to_string();
            let Some(k) = d.kind.as_deref() else {
                return false;
            };
            if !self.kind.iter().any(|w| sing(w) == sing(k)) {
                return false;
            }
        }
        let contains = |want: &Option<String>, have: Option<&str>| match want {
            None => true,
            Some(w) => have.is_some_and(|h| h.to_lowercase().contains(&w.to_lowercase())),
        };
        if !contains(&self.author, d.author.as_deref()) || !contains(&self.title, Some(&d.title)) {
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

/// A slice of the archive written as one expression, for `--compare`,
/// `--a` and `--b`: whitespace-separated `field:value` terms in the
/// `@scout/query` field syntax, plus bare corpus ids.
///
/// - `in:writing,tweets` or a bare `writing`: the corpora (empty = the
///   command's own corpus selection);
/// - `after:2020`, `before:2015` (as `--after` / `--before`);
/// - `y:2016`, or an inclusive range `y:2010-2015` (= `after:2010 before:2016`);
/// - `lang:en,cs`, `genre:essay,note`.
///
/// A slice carries only its own terms: it does not inherit the main filter.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SliceSpec {
    /// The expression as given.
    pub spec: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub corpora: Vec<String>,
    pub filter: DocFilter,
}

/// Split a comma list.
fn list(v: &str) -> Vec<String> {
    v.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

/// Apply the document-level fields of a parsed `@scout/query` query to a
/// filter: `after: before: y: lang: genre: source: zo: ty: au: ti:`.
/// `y:2016` sets the year; `y:2010-2015` (and every other `y:` / `date:`
/// form) becomes the `after` / `before` range of the grammar's `dateRange`.
/// Returns the corpus ids of `in:`.
pub fn apply_query_fields(p: &scout_query::ParsedQuery, f: &mut DocFilter) -> Result<Vec<String>> {
    let nonempty = |name: &str, v: &Option<String>| -> Result<Option<String>> {
        match v {
            Some(v) if v.trim().is_empty() => bail!("{name}: needs a value"),
            Some(v) => Ok(Some(v.trim().to_string())),
            None => Ok(None),
        }
    };
    if let Some(v) = nonempty("after", &p.after)? {
        f.after = Some(v);
    }
    if let Some(v) = nonempty("before", &p.before)? {
        f.before = Some(v);
    }
    if let Some(v) = nonempty("y", &p.date)? {
        let year = |s: &str| {
            (s.len() == 4 && s.chars().all(|c| c.is_ascii_digit()))
                .then(|| s.parse::<i32>().ok())
                .flatten()
        };
        if let Some(y) = year(&v) {
            f.year = Some(y);
        } else if p.after.is_some() || p.before.is_some() {
            bail!("a y: range cannot combine with after: or before:");
        } else if let Some((a, b)) = v
            .split_once('-')
            .and_then(|(a, b)| Some((year(a)?, year(b)?)))
        {
            if b < a {
                bail!("y:{v} is not a range like 2010-2015");
            }
            // Inclusive years: y:2010-2015 = after:2010 before:2016.
            f.after = Some(format!("{a:04}"));
            f.before = Some(format!("{:04}", b + 1));
        } else {
            let r = scout_query::date_range(&v)
                .ok_or_else(|| anyhow::anyhow!("y:{v} is not a date or range like 2010-2015"))?;
            f.after = r.start;
            f.before = r.end;
        }
    }
    if let Some(v) = nonempty("lang", &p.lang)? {
        f.lang.extend(list(&v));
    }
    if let Some(v) = nonempty("genre", &p.genre)? {
        f.genre.extend(list(&v));
    }
    if let Some(v) = nonempty("source", &p.source)? {
        f.source.extend(list(&v));
    } else if p.zotero {
        f.source.push("zotero".into());
    }
    if let Some(v) = nonempty("ty", &p.type_)? {
        f.kind.extend(list(&v));
    }
    f.author = nonempty("au", &p.author)?;
    f.title = nonempty("ti", &p.title)?;
    let corpora = match nonempty("in", &p.in_)? {
        Some(v) => list(&v),
        None => vec![],
    };
    f.validate()?;
    Ok(corpora)
}

impl SliceSpec {
    /// Parse with the `@scout/query` grammar; bare words are corpus ids.
    /// Free-text search terms (`-x`, `"phrase"`, `prefix*`, `/regex/`) and
    /// the passage-level fields `tag:` and `co:` are refused.
    pub fn parse(spec: &str) -> Result<SliceSpec> {
        let mut out = SliceSpec {
            spec: spec.trim().to_string(),
            ..SliceSpec::default()
        };
        let q = scout_query::parse_query(spec);
        let p = &q.parsed;
        let fail = |why: &str| -> Result<SliceSpec> {
            bail!("slice {spec:?}: {why} (use in: after: before: y: lang: genre: source: ty: au: ti: or a corpus id)")
        };
        if !p.regexes.is_empty() || !q.not.is_empty() {
            return fail("a slice takes no search terms");
        }
        if p.tag.is_some() || p.color.is_some() || p.has_image {
            return fail("tag:, co: and i: select highlights, not documents");
        }
        for g in &q.any_of {
            for t in g {
                if t.form != scout_query::TermForm::Word || t.text.contains(':') {
                    return fail(&format!("unknown term {:?}", t.text));
                }
                out.corpora.extend(list(&t.text));
            }
        }
        let ins = apply_query_fields(p, &mut out.filter)
            .map_err(|e| anyhow::anyhow!("slice {spec:?}: {e}"))?;
        out.corpora.extend(ins);
        if out.corpora.is_empty() && out.filter.is_empty() {
            bail!("empty slice {spec:?}: give a corpus id or a filter such as before:2015");
        }
        Ok(out)
    }
}

/// Every document in an index, keyed by `rel_path`.
pub fn load_docs(conn: &Connection) -> Result<BTreeMap<String, DocInfo>> {
    let mut st = conn.prepare(
        "SELECT rel_path, title, date, genre, lang, author, public_url, tokens, topics, kind, source, date_source, handle FROM documents",
    )?;
    let rows = st.query_map([], |r| {
        let topics: String = r.get(8)?;
        Ok(DocInfo {
            rel_path: r.get(0)?,
            title: r.get(1)?,
            date: r.get(2)?,
            genre: r.get(3)?,
            lang: r.get(4)?,
            author: r.get(5)?,
            public_url: r.get(6)?,
            tokens: r.get(7)?,
            topics: serde_json::from_str(&topics).unwrap_or_default(),
            kind: r.get(9)?,
            source: r.get(10)?,
            date_source: r.get(11)?,
            handle: r.get(12)?,
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
    fn slice_specs_parse() {
        let s = SliceSpec::parse("y:2010-2015").unwrap();
        assert_eq!(s.filter.after.as_deref(), Some("2010"));
        assert_eq!(s.filter.before.as_deref(), Some("2016"));
        assert!(s.corpora.is_empty());
        let s = SliceSpec::parse("highlights").unwrap();
        assert_eq!(s.corpora, vec!["highlights"]);
        assert!(s.filter.is_empty());
        let s = SliceSpec::parse("in:writing,tweets before:2015 genre:essay y:2014").unwrap();
        assert_eq!(s.corpora, vec!["writing", "tweets"]);
        assert_eq!(s.filter.before.as_deref(), Some("2015"));
        assert_eq!(s.filter.genre, vec!["essay"]);
        assert_eq!(s.filter.year, Some(2014));
        assert!(SliceSpec::parse("y:2015-2010").is_err());
        assert!(SliceSpec::parse("y:2010-2015 after:2011").is_err());
        assert!(SliceSpec::parse("colour:red").is_err());
        assert!(SliceSpec::parse("metaphor*").is_err());
        assert!(SliceSpec::parse("-writing").is_err());
        assert!(SliceSpec::parse("foo:bar").is_err());
        let s = SliceSpec::parse("highlights source:zotero ty:books au:\"George Lakoff\"").unwrap();
        assert_eq!(s.corpora, vec!["highlights"]);
        assert_eq!(s.filter.source, vec!["zotero"]);
        assert_eq!(s.filter.kind, vec!["books"]);
        assert_eq!(s.filter.author.as_deref(), Some("George Lakoff"));
        let s = SliceSpec::parse("y:2016-05").unwrap();
        assert_eq!(s.filter.after.as_deref(), Some("2016-05-01"));
        assert_eq!(s.filter.before.as_deref(), Some("2016-06-01"));
        assert!(SliceSpec::parse("after:soon").is_err());
        assert!(SliceSpec::parse("  ").is_err());
    }

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

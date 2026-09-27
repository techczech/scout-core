//! `verify_quote`: is this exact string in the original text of a passage?
//!
//! The only leniency is quote and apostrophe unification (curly and straight
//! forms compare equal), applied to both sides. No case folding, no
//! whitespace collapsing, no entity decoding: a one-character change is a
//! miss. Matches report the ORIGINAL matched text and its source line.

use crate::normalize::{self, Mapped};
use crate::Corpus;
use anyhow::{bail, Result};
use serde::Serialize;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct QuoteMatch {
    pub corpus: String,
    pub passage_id: String,
    pub rel_path: String,
    pub path: String,
    pub title: String,
    pub date: Option<String>,
    /// 1-based source line where the match starts.
    pub line: usize,
    /// The matched ORIGINAL text (may differ from the query only in quote
    /// and apostrophe forms).
    pub original: String,
    pub link: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VerifyResult {
    pub schema_version: u32,
    pub quote: String,
    pub corpora: Vec<String>,
    pub found: bool,
    /// Every match, ordered by corpus, path, line.
    pub matches: Vec<QuoteMatch>,
}

pub fn verify_quote(corpora: &[Corpus], quote: &str) -> Result<VerifyResult> {
    let q = quote.trim();
    if q.is_empty() {
        bail!("empty quote");
    }
    let needle = normalize::unify_quotes_str(q);
    let mut matches = Vec::new();
    for c in corpora {
        let conn = crate::index::open_existing(&c.config, c.index_path())?;
        let docs = crate::filter::load_docs(&conn)?;
        let root = c.config.root();
        let mut st = conn.prepare(
            "SELECT rel_path, line_start, original FROM passages ORDER BY rel_path, line_start",
        )?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let original: String = r.get(2)?;
            if !normalize::unify_quotes_str(&original).contains(&needle) {
                continue;
            }
            let rel: String = r.get(0)?;
            let line_start: i64 = r.get(1)?;
            let m = normalize::unify_quotes(&Mapped::identity(&original));
            let abs = root.join(&rel);
            let doc = docs.get(&rel);
            for (ns, _) in m.text.match_indices(&needle) {
                let (os, oe) = m.original_range(ns, ns + needle.len());
                let line = line_start as usize + original[..os].matches('\n').count();
                matches.push(QuoteMatch {
                    corpus: c.id().to_string(),
                    passage_id: format!("{}:{}:{}", c.id(), rel, line_start),
                    rel_path: rel.clone(),
                    path: abs.display().to_string(),
                    title: doc.map(|d| d.title.clone()).unwrap_or_default(),
                    date: doc.and_then(|d| d.date.clone()),
                    line,
                    original: original[os..oe].to_string(),
                    link: c
                        .config
                        .link
                        .as_deref()
                        .map(|t| crate::cite::render_link(t, &abs, line)),
                });
            }
        }
    }
    matches.sort_by(|a, b| {
        a.corpus
            .cmp(&b.corpus)
            .then_with(|| a.rel_path.cmp(&b.rel_path))
            .then(a.line.cmp(&b.line))
    });
    Ok(VerifyResult {
        schema_version: SCHEMA_VERSION,
        quote: quote.to_string(),
        corpora: corpora.iter().map(|c| c.id().to_string()).collect(),
        found: !matches.is_empty(),
        matches,
    })
}

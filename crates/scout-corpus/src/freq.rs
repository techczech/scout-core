//! Token frequencies over the filtered documents: one full scan of the stored
//! token streams (the same streams KWIC matches and n-grams count), shared by
//! collocates (f(c)) and keyness (the two frequency lists).

use crate::filter::{DocFilter, DocInfo};
use crate::Corpus;
use anyhow::Result;
use std::collections::{BTreeMap, HashMap};

/// Filtered documents per corpus id (the denominators).
pub type DocsByCorpus = BTreeMap<String, BTreeMap<String, DocInfo>>;

/// Call `visit(token)` for every token of every passage in the documents of
/// `corpora` that pass `filter`. Returns those documents.
pub(crate) fn scan_tokens(
    corpora: &[Corpus],
    filter: &DocFilter,
    mut visit: impl FnMut(&str),
) -> Result<DocsByCorpus> {
    filter.validate()?;
    let mut out = BTreeMap::new();
    for c in corpora {
        let conn = crate::index::open_existing(&c.config, c.index_path())?;
        let docs = crate::filter::filtered_docs(&conn, filter)?;
        let mut st = conn.prepare(
            "SELECT p.rel_path, passages_fts.tokens FROM passages p
             JOIN passages_fts ON passages_fts.rowid = p.id",
        )?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let rel = r.get_ref(0)?.as_str()?;
            if !docs.contains_key(rel) {
                continue;
            }
            for t in r.get_ref(1)?.as_str()?.split(' ') {
                if !t.is_empty() {
                    visit(t);
                }
            }
        }
        out.insert(c.id().to_string(), docs);
    }
    Ok(out)
}

/// Frequencies of the given `words` only (the collocate candidates).
pub(crate) fn counts_of(
    corpora: &[Corpus],
    filter: &DocFilter,
    words: &[&str],
) -> Result<HashMap<String, u64>> {
    let mut want: HashMap<&str, u64> = words.iter().map(|w| (*w, 0)).collect();
    scan_tokens(corpora, filter, |t| {
        if let Some(n) = want.get_mut(t) {
            *n += 1;
        }
    })?;
    Ok(want.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

/// The full frequency list and the token total.
pub(crate) fn word_list(
    corpora: &[Corpus],
    filter: &DocFilter,
) -> Result<(HashMap<String, u64>, u64, DocsByCorpus)> {
    let mut counts: HashMap<String, u64> = HashMap::new();
    let mut total = 0u64;
    let docs = scan_tokens(corpora, filter, |t| {
        total += 1;
        match counts.get_mut(t) {
            Some(n) => *n += 1,
            None => {
                counts.insert(t.to_string(), 1);
            }
        }
    })?;
    Ok((counts, total, docs))
}

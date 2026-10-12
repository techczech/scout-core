//! Mentioned works: a tag row's optional `mentioned_works`, the layer keys
//! (`<source_system>:<source_id>`) of the works of the archive the item
//! mentions. The tagger writes a list of keys, never a title.
//!
//! - Reading ([`read_mentioned`]): from the line a second time, apart from
//!   the read that decides whether the line is a row, as `entities_typed`
//!   is. Nothing here makes a line a bad line.
//! - Which document a key names ([`Lookup`]): the join the build makes for
//!   tag rows, asked of the same index
//!   ([`crate::topics::documents_by_source_key`], then the rows the build
//!   joined by path). No title is matched against anything.

use crate::normalize::fold;
use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Deserialize)]
struct Mentioned {
    mentioned_works: Option<serde_json::Value>,
}

/// The `mentioned_works` of a tag-row line: each item of the list that is
/// a non-blank text, trimmed, as written, in the row's order; a key the
/// list already has under [`fold`] is not taken twice. Each item is judged
/// alone: one of another shape (a number, null, an object, a list, a blank
/// text) is passed over and the texts beside it still count. The whole
/// value gives none when it is not a list, when this read cannot read the
/// line (a number out of range inside the value) or when the key is on
/// the line twice.
pub(crate) fn read_mentioned(line: &str) -> Vec<String> {
    let listed = serde_json::from_str::<Mentioned>(line)
        .ok()
        .and_then(|m| m.mentioned_works);
    let Some(serde_json::Value::Array(list)) = listed else {
        return vec![];
    };
    let mut seen = BTreeSet::new();
    list.iter()
        .filter_map(|v| v.as_str().map(str::trim))
        .filter(|k| !k.is_empty() && seen.insert(fold(k)))
        .map(String::from)
        .collect()
}

/// A document of the index that a layer key names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Named {
    pub rel_path: String,
    pub title: String,
}

/// Layer key → the document it names in one index.
pub(crate) struct Lookup<'c> {
    conn: &'c Connection,
    by_key: BTreeMap<String, String>,
}

impl<'c> Lookup<'c> {
    /// The keys of `conn`'s documents: each document's own source key
    /// (the first document in path order where several share one), then,
    /// for a key no document has, the document its tag row was joined to
    /// by path at the build.
    pub fn open(conn: &'c Connection) -> Result<Lookup<'c>> {
        let (source, _) = crate::topics::documents_by_source_key(conn)?;
        let mut by_key: BTreeMap<String, String> = source
            .into_iter()
            .filter_map(|(key, mut rels)| {
                rels.sort();
                rels.into_iter().next().map(|rel| (key, rel))
            })
            .collect();
        let has: i64 = conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'topic_docs'",
            [],
            |r| r.get(0),
        )?;
        if has > 0 {
            let mut st = conn.prepare("SELECT key, rel_path FROM topic_docs ORDER BY rel_path")?;
            let rows =
                st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            for row in rows {
                let (key, rel) = row?;
                by_key.entry(key).or_insert(rel);
            }
        }
        Ok(Lookup { conn, by_key })
    }

    /// The document `key` names (the key trimmed, otherwise exactly as
    /// the build compares keys), when the index has it.
    pub fn document(&self, key: &str) -> Result<Option<Named>> {
        let Some(rel) = self.by_key.get(key.trim()) else {
            return Ok(None);
        };
        let title: Option<String> = self
            .conn
            .query_row(
                "SELECT title FROM documents WHERE rel_path = ?1",
                [rel],
                |r| r.get(0),
            )
            .optional()?;
        Ok(title.map(|title| Named {
            rel_path: rel.clone(),
            title,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mentioned_works_are_the_texts_of_a_list_and_nothing_else() {
        let read =
            |value: &str| read_mentioned(&format!(r#"{{"key":"x:1","mentioned_works":{value}}}"#));
        let keys = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(read(r#"["rw:1","x:2"]"#), keys(&["rw:1", "x:2"]));
        // Trimmed, as written; a repeat under the fold is not taken twice.
        assert_eq!(read(r#"[" rw:1 ","RW:1","x:2"]"#), keys(&["rw:1", "x:2"]));
        // Each item alone: the texts beside an odd item still count.
        assert_eq!(
            read(r#"[1,null,{"key":"rw:9"},["rw:8"],"","  ","rw:1",true]"#),
            keys(&["rw:1"])
        );
        for none in [
            "[]",
            "null",
            r#""rw:1""#,
            "7",
            r#"{"key":"rw:1"}"#,
            "[1e400]",
            r#"["rw:1",1e400]"#,
        ] {
            assert_eq!(read(none), Vec::<String>::new(), "{none}");
        }
        // No key, the key twice, a line that is not JSON.
        assert!(read_mentioned(r#"{"key":"x:1"}"#).is_empty());
        assert!(
            read_mentioned(r#"{"mentioned_works":["a:1"],"mentioned_works":["b:2"]}"#).is_empty()
        );
        assert!(read_mentioned(r#"{"mentioned_works":["a:1"]"#).is_empty());
        let deep = format!("{}{}", "[".repeat(200), "]".repeat(200));
        assert!(read(&deep).is_empty());
    }
}

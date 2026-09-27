//! Source adapters: one source file in, zero or more documents out.
//!
//! - markdown-folder, `document_unit = "file"`: the file is the document;
//!   passages are its paragraphs ([`crate::markdown`]).
//! - markdown-folder, `document_unit = "tweet"`: every tweet section is a
//!   document ([`crate::tweets`]).
//! - highlight-scout-archive: a work file is the document; each highlight
//!   is one passage ([`crate::highlights`]).
//!
//! A document's key is the file's relative path, plus `#<fragment>` when
//! the file holds several documents. Passage line numbers are always lines
//! of the source file.

use crate::markdown::{self, DocMeta};
use crate::registry::{CorpusConfig, CorpusKind, DocumentUnit};

/// One passage as the index stores it. `original` is source text: exactly
/// the file's lines, except that a highlight's blockquote markers (`> `)
/// are removed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PassageRecord {
    pub line_start: usize,
    pub line_end: usize,
    pub original: String,
    /// Highlights: the highlight's tags.
    pub tags: Vec<String>,
    /// Highlights: the annotation colour.
    pub color: Option<String>,
    /// Highlights: the date it was highlighted (`YYYY-MM-DD`).
    pub saved_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocRecord {
    pub key: String,
    pub meta: DocMeta,
    pub passages: Vec<PassageRecord>,
    /// Whether the title goes into the title index (it scores search hits).
    /// Off for tweets, whose "title" is their own first words.
    pub index_title: bool,
}

/// The documents in one source file.
pub fn records(cfg: &CorpusConfig, rel_path: &str, text: &str) -> Vec<DocRecord> {
    match (cfg.kind, cfg.document_unit) {
        (CorpusKind::HighlightScoutArchive, _) => crate::highlights::work(rel_path, text)
            .into_iter()
            .collect(),
        (CorpusKind::MarkdownFolder, DocumentUnit::Tweet) => {
            let parsed = markdown::parse_file(text);
            if !markdown::is_document(cfg, &parsed) {
                return vec![];
            }
            crate::tweets::split(cfg, rel_path, &parsed)
        }
        (CorpusKind::MarkdownFolder, DocumentUnit::File) => {
            let parsed = markdown::parse_file(text);
            if !markdown::is_document(cfg, &parsed) {
                return vec![];
            }
            let meta = markdown::doc_meta(cfg, &parsed);
            let passages = markdown::split_passages(&parsed)
                .into_iter()
                .map(|p| PassageRecord {
                    line_start: p.line_start,
                    line_end: p.line_end,
                    original: p.original,
                    ..PassageRecord::default()
                })
                .collect();
            vec![DocRecord {
                key: rel_path.to_string(),
                meta,
                passages,
                index_title: true,
            }]
        }
    }
}

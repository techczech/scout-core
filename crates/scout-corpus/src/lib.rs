//! Scout corpus engine.
//!
//! One index per registered corpus over documents, passages and tokens.
//! Sources are read-only; indexes are disposable and live in the platform
//! data dir; quotes are always original text; output is deterministic.
//!
//! Entry point: [`Corpus`]. Multi-corpus forms of every view take
//! `&[Corpus]`: [`search_all`], [`concord::kwic`], [`concord::distribution`],
//! [`concord::profile`], [`ngrams::ngrams`], [`verify::verify_quote`]. The CLI and the apps call these same functions.

pub mod cite;
pub mod concord;
pub mod filter;
pub mod index;
pub mod markdown;
pub mod ngrams;
pub mod normalize;
pub mod registry;
pub mod search;
pub mod stopwords;
pub mod tokenize;
pub mod verify;

pub use concord::{DistBy, Distribution, KwicRequest, KwicResults, KwicSort, Profile};
pub use filter::DocFilter;
pub use ngrams::{NgramRequest, NgramResults};
pub use verify::VerifyResult;
pub use index::{BuildReport, IndexMissing, IndexStatus};
pub use registry::{CorpusConfig, CorpusKind, Registry, RegistryMissing};
pub use search::{SearchRequest, SearchResults};

use anyhow::Result;
use std::path::{Path, PathBuf};

/// A registered corpus bound to its index file.
#[derive(Debug, Clone)]
pub struct Corpus {
    pub config: CorpusConfig,
    index_path: PathBuf,
}

impl Corpus {
    /// Open corpus `id` from the user registry, with its index in the
    /// platform data dir.
    pub fn open(id: &str) -> Result<Corpus> {
        let reg = Registry::load()?;
        Ok(Corpus::from_config(
            reg.get(id)?.clone(),
            &index::index_dir(),
        ))
    }

    /// Bind a config to an index directory (tests, embedding apps).
    pub fn from_config(config: CorpusConfig, index_dir: &Path) -> Corpus {
        let index_path = index::index_path_in(index_dir, &config.id);
        Corpus { config, index_path }
    }

    pub fn id(&self) -> &str {
        &self.config.id
    }

    pub fn index_path(&self) -> &Path {
        &self.index_path
    }

    /// Incremental build (mtime + hash); `force` deletes the index first.
    pub fn build_index(&self, force: bool) -> Result<BuildReport> {
        index::build(&self.config, &self.index_path, force)
    }

    pub fn status(&self) -> Result<IndexStatus> {
        index::status(&self.config, &self.index_path)
    }

    pub fn search(&self, req: &SearchRequest) -> Result<SearchResults> {
        let conn = index::open_existing(&self.config, &self.index_path)?;
        search::search_corpus(&conn, &self.config, req)
    }

    /// Key-word-in-context lines for a word or phrase.
    pub fn kwic(&self, req: &KwicRequest) -> Result<KwicResults> {
        concord::kwic(std::slice::from_ref(self), req)
    }

    /// Hit counts by year, corpus, genre or lang.
    pub fn distribution(&self, term: &str, by: DistBy, filter: &DocFilter) -> Result<Distribution> {
        concord::distribution(std::slice::from_ref(self), term, by, filter)
    }

    /// Word profile: frequency, per million, pieces, first use, per-year.
    pub fn profile(&self, word: &str, filter: &DocFilter) -> Result<Profile> {
        concord::profile(std::slice::from_ref(self), word, filter)
    }

    pub fn ngrams(&self, req: &NgramRequest) -> Result<NgramResults> {
        ngrams::ngrams(std::slice::from_ref(self), req)
    }

    pub fn verify_quote(&self, text: &str) -> Result<VerifyResult> {
        verify::verify_quote(std::slice::from_ref(self), text)
    }
}

/// Search several corpora and merge the results deterministically.
pub fn search_all(corpora: &[Corpus], req: &SearchRequest) -> Result<SearchResults> {
    let mut parts = Vec::new();
    for c in corpora {
        parts.push(c.search(req)?);
    }
    Ok(search::merge(&req.query, parts, req.limit))
}

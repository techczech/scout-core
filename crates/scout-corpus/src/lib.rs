//! Scout corpus engine.
//!
//! One index per registered corpus over documents, passages and tokens.
//! Sources are read-only; indexes are disposable and live in the platform
//! data dir; quotes are always original text; output is deterministic.
//!
//! Entry point: [`Corpus`]. The CLI and the apps call these same functions.

pub mod cite;
pub mod index;
pub mod markdown;
pub mod normalize;
pub mod registry;
pub mod search;
pub mod stopwords;
pub mod tokenize;

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
}

/// Search several corpora and merge the results deterministically.
pub fn search_all(corpora: &[Corpus], req: &SearchRequest) -> Result<SearchResults> {
    let mut parts = Vec::new();
    for c in corpora {
        parts.push(c.search(req)?);
    }
    Ok(search::merge(&req.query, parts, req.limit))
}

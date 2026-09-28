//! Scout corpus engine.
//!
//! One index per registered corpus over documents, passages and tokens.
//! Sources are read-only; indexes are disposable and live in the platform
//! data dir; quotes are always original text; output is deterministic.
//!
//! Entry point: [`Corpus`]. Multi-corpus forms of every view take
//! `&[Corpus]`: [`search_all`], [`concord::kwic`], [`concord::distribution`],
//! [`concord::profile`], [`colloc::collocates`], [`ngrams::ngrams`],
//! [`verify::verify_quote`]; the two-slice views [`colloc::compare`] and
//! [`keyness::keyness`] take [`Slice`]s. The CLI and the apps call these same
//! functions.
//!
//! Apps and the CLI enter through [`api::Engine`]: one request → response
//! call per `scout` command, whose JSON is the command's `--json` output.

pub mod adapter;
pub mod api;
pub mod cite;
pub mod clean;
pub mod colloc;
pub mod concord;
pub mod filter;
pub mod freq;
pub mod highlights;
pub mod index;
pub mod keyness;
pub mod markdown;
pub mod ngrams;
pub mod normalize;
pub mod profile;
pub mod registry;
pub mod rowdist;
pub mod search;
pub mod stopwords;
pub mod tokenize;
pub mod tweets;
pub mod verify;

pub use api::{Engine, Outcome, Reply};
pub use clean::{CleanOptions, CleanReport};
pub use colloc::{CollocRequest, CollocResults, CompareResults, Score};
pub use concord::{
    DistBy, Distribution, KwicRequest, KwicResults, KwicSort, Near, Profile, ProfileOptions,
};
pub use filter::{DocFilter, SliceSpec};
pub use index::{BuildReport, IndexMissing, IndexStatus, NoIndexedCorpus};
pub use keyness::{KeynessRequest, KeynessResults};
pub use ngrams::{NgramRequest, NgramResults};
pub use registry::{CiteStyle, CorpusConfig, CorpusKind, DocumentUnit, Registry, RegistryMissing};
pub use rowdist::RowDistBy;
pub use search::{CitedPassage, PassageId, PassageNotFound, SearchRequest, SearchResults};
pub use verify::VerifyResult;

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

    pub fn profile_with(
        &self,
        word: &str,
        filter: &DocFilter,
        opts: &ProfileOptions,
    ) -> Result<Profile> {
        concord::profile_with(std::slice::from_ref(self), word, filter, opts)
    }

    /// Collocates of a node (logDice or MI) within a passage window.
    pub fn collocates(&self, req: &CollocRequest) -> Result<CollocResults> {
        colloc::collocates(std::slice::from_ref(self), req)
    }

    pub fn ngrams(&self, req: &NgramRequest) -> Result<NgramResults> {
        ngrams::ngrams(std::slice::from_ref(self), req)
    }

    /// `scout cite`: a passage by id, quoted whole, with both citations.
    pub fn cite(&self, id: &search::PassageId) -> Result<search::CitedPassage> {
        let conn = index::open_existing(&self.config, &self.index_path)?;
        search::cite_passage(&conn, &self.config, id)
    }

    pub fn verify_quote(&self, text: &str) -> Result<VerifyResult> {
        verify::verify_quote(std::slice::from_ref(self), text)
    }
}

/// Search several corpora and merge the results deterministically. A
/// query's `in:` narrows `corpora` to the ids it names.
pub fn search_all(corpora: &[Corpus], req: &SearchRequest) -> Result<SearchResults> {
    let named = search::query_corpora(&req.query)?;
    let mut parts = Vec::new();
    for c in corpora {
        if named.is_empty() || named.iter().any(|n| n == c.id()) {
            parts.push(c.search(req)?);
        }
    }
    if parts.is_empty() {
        let have: Vec<&str> = corpora.iter().map(|c| c.id()).collect();
        anyhow::bail!(
            "in:{} names none of the searched corpora ({})",
            named.join(","),
            have.join(", ")
        );
    }
    search::merge(&req.query, parts, req.limit)
}

/// A slice of the archive for the two-slice views: corpora plus a filter.
#[derive(Debug, Clone)]
pub struct Slice {
    /// How the slice was written (the `--compare` / `--a` / `--b` value).
    pub label: String,
    pub corpora: Vec<Corpus>,
    pub filter: DocFilter,
}

impl Slice {
    pub fn new(label: impl Into<String>, corpora: Vec<Corpus>, filter: DocFilter) -> Slice {
        Slice {
            label: label.into(),
            corpora,
            filter,
        }
    }
}

/// The default corpus selection when a command names none: every corpus of
/// the registry whose index opens. Returns them and the ids left out.
pub fn indexed_corpora(reg: &Registry, index_dir: &Path) -> (Vec<Corpus>, Vec<String>) {
    let mut ok = Vec::new();
    let mut missing = Vec::new();
    for cfg in &reg.corpora {
        let c = Corpus::from_config(cfg.clone(), index_dir);
        if index::open_existing(&c.config, c.index_path()).is_ok() {
            ok.push(c);
        } else {
            missing.push(cfg.id.clone());
        }
    }
    (ok, missing)
}

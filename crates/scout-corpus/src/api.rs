//! The app-facing engine API: one request → response call per `scout`
//! command. The CLI and the apps (Highlight Scout, ArchiveScout) call these
//! same functions (invariant 5); the CLI only parses arguments and formats.
//!
//! - Requests are plain data (`Serialize + Deserialize`, every field
//!   defaulted), so an app can pass them as JSON; the defaults are the CLI's.
//! - A response is a [`Reply`]: the body (whose [`to_json`] is exactly the
//!   command's `--json` output, see `docs/cli-json.md`) plus the notes the
//!   CLI prints on stderr.
//! - [`Engine`] holds only the registry and the index directory, both
//!   immutable. Every call opens the index files it needs and closes them, so
//!   an `Engine` is `Send + Sync + Clone` and safe on any thread; there is no
//!   global mutable state.

use crate::clean::{self, CleanOptions, CleanReport};
use crate::colloc::{self, CollocRequest, CollocResults, CompareResults};
use crate::concord::{self, Distribution, KwicRequest, KwicResults, Near, Profile, ProfileOptions};
pub use crate::event_list::{EventListing, EventsQuery, EventsResults, MonthCount};
use crate::filter::{DocFilter, SliceSpec};
use crate::index::{self, BuildReport, IndexStatus, NoIndexedCorpus};
use crate::keyness::{self, KeynessRequest, KeynessResults};
use crate::ngrams::{self, NgramRequest, NgramResults};
use crate::register::RegistryStore;
pub use crate::register::{AddCorpus, Removed};
use crate::registry::{self, CorpusConfig, Preset, Registry};
use crate::search::{self, CitedPassage, PassageId, SearchRequest, SearchResults};
use crate::semantic::{self, store, Embedder, Freshness, SearchMode};
use crate::similar::{self, SimilarRequest, SimilarResults};
use crate::topics::FacetCounter;
pub use crate::topics::{FacetItem, Facets, TopicField};
use crate::verify::{self, VerifyResult};
use crate::{search_all, Corpus, Slice};
use anyhow::{anyhow, bail, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Schema version of the JSON shapes this module adds (index, corpora).
pub const SCHEMA_VERSION: u32 = 1;

/// A command's response: the JSON body and the stderr notes.
#[derive(Debug, Clone)]
pub struct Reply<T> {
    pub body: T,
    /// One-line notes, e.g. `scout: note: using writing; not indexed: tweets`.
    pub notes: Vec<String>,
}

impl<T> Reply<T> {
    fn new(body: T) -> Reply<T> {
        Reply {
            body,
            notes: vec![],
        }
    }
}

/// The exact `--json` text of a response body (pretty-printed, no trailing
/// newline; the CLI adds one).
pub fn to_json<T: Serialize>(body: &T) -> Result<String> {
    Ok(serde_json::to_string_pretty(body)?)
}

/// Whether a response counts as "results" (CLI exit 0) or "none" (exit 1).
pub trait Outcome {
    fn has_results(&self) -> bool;
}

// ---------- requests ----------

/// Corpus selection and document filters shared by the analytics views.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Scope {
    /// Corpus ids; empty = every indexed corpus (with a note naming the rest).
    #[serde(rename = "in")]
    pub in_: Vec<String>,
    pub lang: Vec<String>,
    pub genre: Vec<String>,
    pub after: Option<String>,
    pub before: Option<String>,
    pub year: Option<i32>,
    pub source: Vec<String>,
}

impl Scope {
    pub fn filter(&self) -> DocFilter {
        DocFilter {
            lang: self.lang.clone(),
            genre: self.genre.clone(),
            after: self.after.clone(),
            before: self.before.clone(),
            year: self.year,
            source: self.source.clone(),
            ..DocFilter::default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchQuery {
    pub query: String,
    /// Corpus ids; empty = the query's `in:`, else every indexed corpus.
    #[serde(rename = "in")]
    pub in_: Vec<String>,
    pub limit: usize,
    /// Quote the whole paragraph instead of the hit sentence.
    pub passage: bool,
    /// `auto` (hybrid where the engine has an embedder and the corpus has
    /// current vectors, else full text), `fts`, `semantic` or `hybrid`.
    pub mode: SearchMode,
}

impl Default for SearchQuery {
    fn default() -> Self {
        SearchQuery {
            query: String::new(),
            in_: vec![],
            limit: 20,
            passage: false,
            mode: SearchMode::Auto,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KwicQuery {
    pub term: String,
    pub scope: Scope,
    pub width: usize,
    /// `L1`..`L9`, `R1`..`R9`, `date` or `source`.
    pub sort: String,
    pub limit: usize,
    /// Only lines with this word within `window` tokens of the node.
    pub near: Option<String>,
    pub window: usize,
}

impl Default for KwicQuery {
    fn default() -> Self {
        KwicQuery {
            term: String::new(),
            scope: Scope::default(),
            width: 8,
            sort: "R1".into(),
            limit: 200,
            near: None,
            window: 5,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CollocatesQuery {
    pub node: String,
    pub scope: Scope,
    pub window: usize,
    /// `logdice` or `mi`.
    pub score: String,
    pub min: u64,
    pub top: usize,
    pub keep_stopwords: bool,
    /// A second slice side by side; the reply is then [`Collocates::Compare`].
    pub compare: Option<String>,
    /// Per-row distribution: `year`, `doc` or `corpus`.
    pub dist: Option<String>,
}

impl Default for CollocatesQuery {
    fn default() -> Self {
        CollocatesQuery {
            node: String::new(),
            scope: Scope::default(),
            window: 5,
            score: "logdice".into(),
            min: 5,
            top: 30,
            keep_stopwords: false,
            compare: None,
            dist: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeynessQuery {
    pub a: String,
    pub b: String,
    /// Corpora for a slice that names none (default: every indexed corpus).
    #[serde(rename = "in")]
    pub in_: Vec<String>,
    pub top: usize,
    pub min: u64,
}

impl Default for KeynessQuery {
    fn default() -> Self {
        let d = KeynessRequest::default();
        KeynessQuery {
            a: String::new(),
            b: String::new(),
            in_: vec![],
            top: d.top,
            min: d.min_freq,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DistQuery {
    pub term: String,
    /// `year`, `corpus`, `genre` or `lang`.
    pub by: String,
    pub scope: Scope,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NgramsQuery {
    pub scope: Scope,
    /// A range such as `3-5`, or one n.
    pub n: String,
    pub since: Option<i32>,
    pub until: Option<i32>,
    pub top: usize,
    pub strict_stopwords: bool,
    pub dist: Option<String>,
    /// Keep only grams containing this word or phrase (normalised and
    /// tokenised as passages are), e.g. the phrases with a word.
    pub containing: Option<String>,
}

impl Default for NgramsQuery {
    fn default() -> Self {
        NgramsQuery {
            scope: Scope::default(),
            n: "3-5".into(),
            since: None,
            until: None,
            top: NgramRequest::default().top,
            strict_stopwords: false,
            dist: None,
            containing: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProfileQuery {
    pub word: String,
    pub scope: Scope,
    pub min_hits: usize,
}

impl Default for ProfileQuery {
    fn default() -> Self {
        ProfileQuery {
            word: String::new(),
            scope: Scope::default(),
            min_hits: ProfileOptions::default().min_hits,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CiteQuery {
    /// `<corpus>:<path>:<line>`.
    pub passage_id: String,
}

/// `similar`: passages like 1–10 seeds (tf-idf cosine to their centroid).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SimilarQuery {
    /// Seed passage ids, `<corpus>:<path>:<line>`.
    pub seeds: Vec<String>,
    /// Corpora to search; empty = every indexed corpus. Seeds may lie outside.
    #[serde(rename = "in")]
    pub in_: Vec<String>,
    pub top: usize,
    /// Leave the seeds themselves out of the results.
    pub exclude_seeds: bool,
}

impl Default for SimilarQuery {
    fn default() -> Self {
        SimilarQuery {
            seeds: vec![],
            in_: vec![],
            top: 20,
            exclude_seeds: false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VerifyQuoteQuery {
    pub text: String,
    #[serde(rename = "in")]
    pub in_: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct IndexBuildQuery {
    /// Corpus ids (default: every registered corpus).
    pub ids: Vec<String>,
    /// Delete the index and rebuild from scratch.
    pub force: bool,
    /// Also build passage vectors (needs an embedder). Vectors that exist
    /// are updated by every build that has an embedder.
    pub semantic: bool,
}

/// `scout facets <query>`: topic-layer counts over a query's result set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FacetsQuery {
    pub query: String,
    /// Corpus ids; empty = the query's `in:`, else every indexed corpus.
    #[serde(rename = "in")]
    pub in_: Vec<String>,
    /// Values kept per facet for themes, entities and events (0 = all).
    pub top: usize,
    /// Leave out of the event facet every event with fewer items than this
    /// in the whole of its own corpus (not in the result set, and never
    /// added up across corpora); 1 keeps every event.
    pub min_event_items: usize,
}

impl Default for FacetsQuery {
    fn default() -> Self {
        FacetsQuery {
            query: String::new(),
            in_: vec![],
            top: 20,
            min_event_items: 1,
        }
    }
}

/// `scout topics themes|entities|events|months`: every value with its
/// document count.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TopicsQuery {
    /// `themes`, `entities`, `events` or `months`.
    pub field: String,
    /// Only documents of this layer month (`YYYY-MM` or `YYYY`).
    pub month: Option<String>,
    #[serde(rename = "in")]
    pub in_: Vec<String>,
    /// Values kept (0 = all).
    pub top: usize,
}

impl Default for TopicsQuery {
    fn default() -> Self {
        TopicsQuery {
            field: "themes".into(),
            month: None,
            in_: vec![],
            top: 0,
        }
    }
}

/// `scout topics theme-months`: documents per theme and layer month.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeMonthsQuery {
    /// Corpus ids; empty = the query's `in:`, else every indexed corpus.
    #[serde(rename = "in")]
    pub in_: Vec<String>,
    /// The first month, `YYYY-MM`; without it, the earliest month of a
    /// counted document.
    pub from: Option<String>,
    /// The last month, `YYYY-MM`; without it, the latest month of a
    /// counted document.
    pub to: Option<String>,
    /// Narrows the documents, in the grammar of `search`; empty = every
    /// tagged document search can return.
    pub query: String,
}

/// Most months one `theme-months` reply covers (150 years).
pub const MAX_THEME_MONTHS: usize = 1800;

/// `topics theme-months --json`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ThemeMonthsResults {
    pub schema_version: u32,
    pub query: String,
    pub corpora: Vec<String>,
    /// The first and last month of `months`; null when there is none.
    pub from: Option<String>,
    pub to: Option<String>,
    /// Every month from `from` to `to`, ascending, empty ones included.
    pub months: Vec<String>,
    /// Every theme with a document in the range: by total, largest first,
    /// then by name.
    pub themes: Vec<String>,
    /// `counts[t][m]`: documents carrying `themes[t]` whose layer month is
    /// `months[m]`. A document with two themes counts in both rows.
    pub counts: Vec<Vec<usize>>,
    /// Per theme: its row added up (a document has one month).
    pub theme_totals: Vec<usize>,
    /// Per month: the tagged documents of that month, each once, with or
    /// without a theme.
    pub month_totals: Vec<usize>,
    /// The tagged documents of the range, each once.
    pub documents: usize,
}

/// `facets --json`.
#[derive(Debug, Clone, Serialize)]
pub struct FacetsResults {
    pub schema_version: u32,
    pub query: String,
    pub plan: search::Plan,
    pub corpora: Vec<String>,
    /// Documents in the result set (as `search`'s `total_documents`).
    pub total_documents: usize,
    /// Of those, documents with topic tags.
    pub tagged_documents: usize,
    pub top: usize,
    /// The query's `min_event_items`, when it left events out (above 1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_event_items: Option<usize>,
    pub facets: Facets,
}

/// `topics --json`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TopicsResults {
    pub schema_version: u32,
    pub field: TopicField,
    pub month: Option<String>,
    pub corpora: Vec<String>,
    /// Documents with topic tags (in `month` when given).
    pub tagged_documents: usize,
    /// Distinct values before `top`.
    pub distinct: usize,
    pub items: Vec<FacetItem>,
}

/// What a long index build reports as it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildEvent {
    /// Shown before the embedding model is downloaded (name, size, where).
    Download(String),
    /// Passages embedded so far of those a corpus needs.
    Embedding {
        corpus: String,
        done: usize,
        total: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CleanReportQuery {
    pub corpus: String,
    /// Samples per rule.
    pub samples: usize,
    /// Candidate boilerplate regexes to count alongside the registry's.
    pub rules: Vec<String>,
}

impl Default for CleanReportQuery {
    fn default() -> Self {
        CleanReportQuery {
            corpus: String::new(),
            samples: CleanOptions::default().samples,
            rules: vec![],
        }
    }
}

// ---------- responses added by this module ----------

/// `collocates`: one slice, or two side by side (`compare`).
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum Collocates {
    Single(CollocResults),
    Compare(CompareResults),
}

/// `index build --json`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct IndexBuildReport {
    pub schema_version: u32,
    pub reports: Vec<BuildReport>,
}

/// `index status --json`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct IndexStatusReport {
    pub schema_version: u32,
    pub corpora: Vec<IndexStatus>,
}

/// One registry entry with its index location.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CorpusEntry {
    pub id: String,
    pub name: String,
    pub kind: String,
    /// The path as written in the registry.
    pub path: String,
    pub root: String,
    pub index_path: String,
    pub index_built: bool,
    /// False when the source folder is gone; the index is kept and still
    /// searchable (`corpora list` shows `folder missing`).
    pub folder_exists: bool,
}

/// `corpora list --json`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CorporaList {
    pub schema_version: u32,
    pub corpora: Vec<CorpusEntry>,
}

/// `corpora show`: the entry as registry TOML plus its locations.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CorpusShow {
    pub toml: String,
    pub entry: CorpusEntry,
}

/// `corpora init-defaults`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct InitDefaults {
    pub path: String,
    /// False when a registry existed and `force` was not given.
    pub wrote: bool,
    pub corpora: Vec<CorpusEntry>,
}

impl Outcome for FacetsResults {
    fn has_results(&self) -> bool {
        self.total_documents > 0
    }
}

impl Outcome for ThemeMonthsResults {
    fn has_results(&self) -> bool {
        self.documents > 0
    }
}

impl Outcome for TopicsResults {
    fn has_results(&self) -> bool {
        !self.items.is_empty()
    }
}

impl Outcome for SearchResults {
    fn has_results(&self) -> bool {
        !self.results.is_empty()
    }
}
impl Outcome for KwicResults {
    fn has_results(&self) -> bool {
        self.total > 0
    }
}
impl Outcome for Collocates {
    fn has_results(&self) -> bool {
        match self {
            Collocates::Single(r) => !r.rows.is_empty(),
            Collocates::Compare(r) => !r.rows.is_empty(),
        }
    }
}
impl Outcome for KeynessResults {
    fn has_results(&self) -> bool {
        !self.a_keys.is_empty() || !self.b_keys.is_empty()
    }
}
impl Outcome for Distribution {
    fn has_results(&self) -> bool {
        self.total > 0
    }
}
impl Outcome for NgramResults {
    fn has_results(&self) -> bool {
        !self.grams.is_empty()
    }
}
impl Outcome for Profile {
    fn has_results(&self) -> bool {
        self.frequency > 0
    }
}
impl Outcome for SimilarResults {
    fn has_results(&self) -> bool {
        !self.results.is_empty()
    }
}
impl Outcome for VerifyResult {
    fn has_results(&self) -> bool {
        self.found
    }
}
impl Outcome for CitedPassage {
    fn has_results(&self) -> bool {
        true
    }
}
impl Outcome for IndexBuildReport {
    fn has_results(&self) -> bool {
        true
    }
}
impl Outcome for IndexStatusReport {
    fn has_results(&self) -> bool {
        true
    }
}
impl Outcome for CorporaList {
    fn has_results(&self) -> bool {
        true
    }
}
impl Outcome for CleanReport {
    fn has_results(&self) -> bool {
        true
    }
}

// ---------- the engine ----------

/// The registry and the index directory every call works against, plus an
/// optional embedder: semantic and hybrid search, and vector builds, need
/// one. An engine without one searches full text only (apps opt in).
#[derive(Clone)]
pub struct Engine {
    registry: Registry,
    index_dir: PathBuf,
    embedder: Option<Arc<dyn Embedder>>,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Engine")
            .field("registry", &self.registry)
            .field("index_dir", &self.index_dir)
            .field("embedder", &self.embedder.as_ref().map(|e| e.model_id()))
            .finish()
    }
}

/// How one corpus of a search is ranked.
enum Ranking {
    Fts,
    Semantic(semantic::Vectors),
    Hybrid(semantic::Vectors),
}

fn entry(cfg: &crate::CorpusConfig, dir: &Path) -> CorpusEntry {
    let p = index::index_path_in(dir, &cfg.id);
    CorpusEntry {
        id: cfg.id.clone(),
        name: cfg.name.clone(),
        kind: cfg.kind.as_str().into(),
        path: cfg.path.clone(),
        root: cfg.root().display().to_string(),
        index_path: p.display().to_string(),
        index_built: p.exists(),
        folder_exists: cfg.root().is_dir(),
    }
}

/// `corpora init-defaults`: write a starter registry to the user registry
/// path (`$SCOUT_CONFIG`, else `~/.config/scout/corpora.toml`): empty, with a
/// comment on `scout corpora add`, unless a preset is named.
pub fn init_defaults(force: bool, preset: Preset) -> Result<InitDefaults> {
    let path = registry::registry_path();
    let wrote = registry::init_defaults(&path, force, preset)?;
    let reg = Registry::load_from(&path)?;
    let dir = index::index_dir();
    Ok(InitDefaults {
        path: path.display().to_string(),
        wrote,
        corpora: reg.corpora.iter().map(|c| entry(c, &dir)).collect(),
    })
}

/// `corpora add`: register a folder in the user registry (created when
/// missing). Validation is the CLI's: the folder must exist, its path and
/// id must be new. Does not build the index. See [`RegistryStore::add`] for
/// an explicit registry path and index dir.
pub fn add_corpus(req: AddCorpus) -> Result<CorpusConfig> {
    RegistryStore::from_env().add(&req)
}

/// `corpora remove`: unregister a corpus from the user registry and delete
/// its index files unless `keep_index`. Never touches the source folder.
pub fn remove_corpus(id: &str, keep_index: bool) -> Result<Removed> {
    RegistryStore::from_env().remove(id, keep_index)
}

/// The one rule for whose event a facet value is. When any searched
/// corpus has a records folder, an event belongs to its corpus: the same
/// id in two corpora is two values, each with `corpus`. When none has, the
/// event facet is what it was before lasting events: one value per id over
/// all the searched corpora together, without `corpus`.
fn events_are_per_corpus<'a>(
    searched: impl IntoIterator<Item = &'a rusqlite::Connection>,
) -> Result<bool> {
    for conn in searched {
        if crate::topics::has_records_folder(conn)? {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Every month from `first` to `last` (both `YYYY-MM`), ascending; empty
/// when `last` is before `first`. Refused over [`MAX_THEME_MONTHS`].
fn month_range(first: &str, last: &str) -> Result<Vec<String>> {
    let parts = |m: &str| -> Result<(i32, u32)> {
        let (y, mo) = m
            .split_once('-')
            .ok_or_else(|| anyhow!("{m:?} is not a month YYYY-MM"))?;
        Ok((y.parse()?, mo.parse()?))
    };
    let ((y0, m0), (y1, m1)) = (parts(first)?, parts(last)?);
    let count = (i64::from(y1) - i64::from(y0)) * 12 + i64::from(m1) - i64::from(m0) + 1;
    if count > MAX_THEME_MONTHS as i64 {
        bail!("{first} to {last} is {count} months; at most {MAX_THEME_MONTHS} are listed");
    }
    let mut out = Vec::new();
    let (mut y, mut m) = (y0, m0);
    for _ in 0..count.max(0) {
        out.push(format!("{y:04}-{m:02}"));
        (y, m) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    }
    Ok(out)
}

impl Engine {
    /// The user registry and the platform index directory (honouring
    /// `SCOUT_CONFIG` and `SCOUT_DATA_DIR`), as the CLI uses them.
    pub fn from_env() -> Result<Engine> {
        Ok(Engine::new(Registry::load()?, index::index_dir()))
    }

    pub fn new(registry: Registry, index_dir: impl Into<PathBuf>) -> Engine {
        Engine {
            registry,
            index_dir: index_dir.into(),
            embedder: None,
        }
    }

    /// The same engine with an embedder: semantic and hybrid search, and
    /// passage vectors in index builds.
    pub fn with_embedder(mut self, embedder: Arc<dyn Embedder>) -> Engine {
        self.embedder = Some(embedder);
        self
    }

    pub fn embedder(&self) -> Option<&Arc<dyn Embedder>> {
        self.embedder.as_ref()
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    pub fn index_dir(&self) -> &Path {
        &self.index_dir
    }

    fn corpus(&self, id: &str) -> Result<Corpus> {
        Ok(Corpus::from_config(
            self.registry.get(id)?.clone(),
            &self.index_dir,
        ))
    }

    /// The named corpora, or every registered one.
    fn select(&self, ids: &[String]) -> Result<Vec<Corpus>> {
        if ids.is_empty() {
            return Ok(self
                .registry
                .corpora
                .iter()
                .map(|c| Corpus::from_config(c.clone(), &self.index_dir))
                .collect());
        }
        ids.iter().map(|id| self.corpus(id)).collect()
    }

    /// The corpora a query runs over: the named ones, or else every indexed
    /// corpus, noting those left out.
    pub(crate) fn scope(&self, ids: &[String], notes: &mut Vec<String>) -> Result<Vec<Corpus>> {
        if !ids.is_empty() {
            return self.select(ids);
        }
        let (ok, missing) = crate::indexed_corpora(&self.registry, &self.index_dir);
        if ok.is_empty() {
            return Err(NoIndexedCorpus(missing).into());
        }
        if !missing.is_empty() {
            let used: Vec<&str> = ok.iter().map(|c| c.id()).collect();
            notes.push(format!(
                "scout: note: using {}; not indexed: {}",
                used.join(", "),
                missing.join(", ")
            ));
        }
        Ok(ok)
    }

    /// A slice expression: its own corpora, else `default`.
    fn slice(&self, spec: &str, default: &[Corpus]) -> Result<Slice> {
        let s = SliceSpec::parse(spec)?;
        let corpora = if s.corpora.is_empty() {
            default.to_vec()
        } else {
            self.select(&s.corpora)?
        };
        Ok(Slice::new(s.spec, corpora, s.filter))
    }

    pub fn search(&self, q: &SearchQuery) -> Result<Reply<SearchResults>> {
        if q.query.trim().is_empty() {
            bail!("empty query");
        }
        let mut notes = vec![];
        // `in:` in the query selects corpora when `in` does not.
        let ids = if q.in_.is_empty() {
            search::query_corpora(&q.query)?
        } else {
            q.in_.clone()
        };
        let corpora = self.scope(&ids, &mut notes)?;
        let mut req = SearchRequest::new(q.query.clone());
        req.limit = q.limit;
        req.whole_passage = q.passage;
        if q.mode == SearchMode::Fts || (q.mode == SearchMode::Auto && self.embedder.is_none()) {
            let body = search_all(&corpora, &req)?;
            return Ok(Reply { body, notes });
        }
        let body = self.search_vectors(&corpora, &req, q.mode, &mut notes)?;
        Ok(Reply { body, notes })
    }

    /// Search where vectors may rank: per corpus, semantic, hybrid or full
    /// text by `mode` and whether its vectors are current; then the usual
    /// merge.
    fn search_vectors(
        &self,
        corpora: &[Corpus],
        req: &SearchRequest,
        mode: SearchMode,
        notes: &mut Vec<String>,
    ) -> Result<SearchResults> {
        let explicit = mode != SearchMode::Auto;
        let Some(embedder) = self.embedder.as_ref() else {
            bail!("semantic search needs an embedding model; this engine has none");
        };
        let text = semantic::query_text(&req.query);
        if text.is_empty() {
            if explicit {
                bail!("semantic search needs words to embed, not only filters");
            }
            return search_all(corpora, req);
        }
        let named = search::query_corpora(&req.query)?;
        let model = embedder.model_id();
        let mut plan: Vec<(&Corpus, Ranking)> = Vec::new();
        let mut unready: Vec<String> = Vec::new();
        for c in corpora {
            if !(named.is_empty() || named.iter().any(|n| n == c.id())) {
                continue;
            }
            let conn = index::open_existing(&c.config, c.index_path())?;
            let vpath = c.vectors_path();
            let ranking = match store::freshness(&conn, &vpath, &model)? {
                Freshness::Current => {
                    let v = store::load(&vpath)?;
                    match mode {
                        SearchMode::Semantic => Ranking::Semantic(v),
                        _ => Ranking::Hybrid(v),
                    }
                }
                Freshness::Stale(why) => {
                    unready.push(format!("{} ({why})", c.id()));
                    Ranking::Fts
                }
                Freshness::Missing => {
                    if explicit {
                        unready.push(format!("{} (no vectors)", c.id()));
                    }
                    Ranking::Fts
                }
            };
            plan.push((c, ranking));
        }
        if plan.is_empty() {
            let have: Vec<&str> = corpora.iter().map(|c| c.id()).collect();
            bail!(
                "in:{} names none of the searched corpora ({})",
                named.join(","),
                have.join(", ")
            );
        }
        let any_vectors = plan.iter().any(|(_, r)| !matches!(r, Ranking::Fts));
        if !unready.is_empty() {
            notes.push(format!(
                "scout: note: full text only for {}; run `scout index build --semantic`",
                unready.join(", ")
            ));
        }
        if mode == SearchMode::Semantic && !any_vectors {
            bail!(
                "no searched corpus has current vectors; run `scout index build --semantic {}`",
                plan.iter()
                    .map(|(c, _)| c.id())
                    .collect::<Vec<_>>()
                    .join(" ")
            );
        }
        let qvec = if any_vectors {
            embedder.embed_query(&text)?
        } else {
            vec![]
        };
        let mut parts = Vec::new();
        let mut used = None;
        for (c, ranking) in &plan {
            let conn = index::open_existing(&c.config, c.index_path())?;
            let part = match ranking {
                Ranking::Fts if mode == SearchMode::Semantic => continue,
                Ranking::Fts => search::search_corpus(&conn, &c.config, req)?,
                Ranking::Semantic(v) => {
                    used = Some("semantic");
                    semantic::semantic_corpus(&conn, &c.config, req, v, &qvec)?
                }
                Ranking::Hybrid(v) => {
                    used = used.or(Some("hybrid"));
                    semantic::hybrid_corpus(&conn, &c.config, req, v, &qvec)?
                }
            };
            parts.push(part);
        }
        let mut body = search::merge(&req.query, parts, req.limit)?;
        body.mode = used.map(String::from);
        Ok(body)
    }

    /// Topic-layer facets over the documents `search` would return for the
    /// same query (the full-text and filter match, all of it, not `limit`).
    pub fn facets(&self, q: &FacetsQuery) -> Result<Reply<FacetsResults>> {
        if q.query.trim().is_empty() {
            bail!("empty query");
        }
        let mut notes = vec![];
        let named = search::query_corpora(&q.query)?;
        let ids = if q.in_.is_empty() {
            named.clone()
        } else {
            q.in_.clone()
        };
        let corpora = self.scope(&ids, &mut notes)?;
        let mut counter = FacetCounter::default();
        counter.event_floor(q.min_event_items);
        let mut used = Vec::new();
        let mut plan = search::Plan::parse(&q.query)?;
        let mut searched = Vec::new();
        for c in &corpora {
            if named.is_empty() || named.iter().any(|n| n == c.id()) {
                searched.push((c, index::open_existing(&c.config, c.index_path())?));
            }
        }
        let per_corpus = events_are_per_corpus(searched.iter().map(|(_, conn)| conn))?;
        for (c, conn) in &searched {
            if per_corpus {
                counter.corpus(c.id());
            }
            let (p, docs) = search::matching_docs(conn, &q.query)?;
            plan = p;
            for d in &docs {
                counter.add(d);
            }
            counter.add_event_records(&crate::events::load(conn)?);
            if q.min_event_items > 1 {
                // An event's size is counted as `scout events` counts it.
                for d in crate::filter::load_docs(conn)?.values() {
                    if d.tokens > 0 {
                        counter.add_event_size(d);
                    }
                }
            }
            used.push(c.id().to_string());
        }
        if used.is_empty() {
            let have: Vec<&str> = corpora.iter().map(|c| c.id()).collect();
            bail!(
                "in:{} names none of the searched corpora ({})",
                named.join(","),
                have.join(", ")
            );
        }
        Ok(Reply {
            body: FacetsResults {
                schema_version: crate::topics::SCHEMA_VERSION,
                query: q.query.clone(),
                plan,
                corpora: used,
                total_documents: counter.documents,
                tagged_documents: counter.tagged,
                top: q.top,
                min_event_items: (q.min_event_items > 1).then_some(q.min_event_items),
                facets: counter.facets(q.top),
            },
            notes,
        })
    }

    /// The theme × month matrix: documents per theme and layer month, in
    /// one call. A month's documents are the ones [`Engine::facets`]
    /// counts for the same query with `month:YYYY-MM` added, chosen by
    /// the same function ([`search::month_docs`]: the query's plan with
    /// that month as its filter), whatever the query is: none, a corpus
    /// alone (`in:`), fields, terms. A document counts once per theme it
    /// carries. So a cell is the count `facets` gives that theme for the
    /// same query with `month:YYYY-MM` added.
    ///
    /// A bound is a month `YYYY-MM` or is absent; an absent one is the
    /// documents' own first or last month. A blank bound is not absent.
    ///
    /// A query with a `month:` field is refused (as the engine's own
    /// parser reads the query, [`search::Plan::parse`]: the field, not
    /// the text `month:` inside a phrase): the months are the columns,
    /// chosen with `from` and `to`. So the equality with `facets` holds
    /// for every query this accepts.
    ///
    /// Several corpora: a theme is its name (under the fold of labels),
    /// and its counts are added up over the searched corpora, as the
    /// theme facet's are.
    pub fn theme_months(&self, q: &ThemeMonthsQuery) -> Result<Reply<ThemeMonthsResults>> {
        let bound = |what: &str, v: &Option<String>| -> Result<Option<String>> {
            match v.as_deref() {
                None => Ok(None),
                Some(m) if m.len() == 7 && crate::topics::valid_month(m) => Ok(Some(m.to_string())),
                Some(m) => bail!("--{what} {m:?} is not a month YYYY-MM"),
            }
        };
        let (from, to) = (bound("from", &q.from)?, bound("to", &q.to)?);
        if let (Some(f), Some(t)) = (&from, &to) {
            if t < f {
                bail!("--to {t} is before --from {f}");
            }
        }
        let query = q.query.trim();
        let mut notes = vec![];
        let plan = search::Plan::parse(query)?;
        if let Some(m) = &plan.filter.month {
            bail!(
                "month:{m} in the query: theme-months counts every month of its range, so a query does not pick one; use --from and --to"
            );
        }
        let named = plan.corpora.clone();
        let ids = if q.in_.is_empty() {
            named.clone()
        } else {
            q.in_.clone()
        };
        let corpora = self.scope(&ids, &mut notes)?;
        let mut used = Vec::new();
        // (month, the document's themes) of every counted document.
        let mut docs: Vec<(String, Vec<String>)> = Vec::new();
        for c in &corpora {
            if !(named.is_empty() || named.iter().any(|n| n == c.id())) {
                continue;
            }
            let conn = index::open_existing(&c.config, c.index_path())?;
            // The months to ask for: those some document of the corpus
            // has, within the bounds. (No other month has a document
            // under any query: `month:` is equality with it.)
            let months: std::collections::BTreeSet<String> = crate::filter::load_docs(&conn)?
                .into_values()
                .filter_map(|d| d.topic.and_then(|t| t.month))
                .filter(|m| m.len() == 7 && crate::topics::valid_month(m))
                .filter(|m| {
                    !(from.as_ref().is_some_and(|f| m < f) || to.as_ref().is_some_and(|t| m > t))
                })
                .collect();
            for month in months {
                for d in search::month_docs(&conn, &plan, &month)? {
                    // `month:` keeps tagged documents only.
                    let themes = d.topic.map(|t| t.themes).unwrap_or_default();
                    docs.push((month.clone(), themes));
                }
            }
            used.push(c.id().to_string());
        }
        if used.is_empty() {
            let have: Vec<&str> = corpora.iter().map(|c| c.id()).collect();
            bail!(
                "in:{} names none of the searched corpora ({})",
                named.join(","),
                have.join(", ")
            );
        }
        let first = from.or_else(|| docs.iter().map(|d| d.0.clone()).min());
        let last = to.or_else(|| docs.iter().map(|d| d.0.clone()).max());
        let months = match (&first, &last) {
            (Some(f), Some(l)) => month_range(f, l)?,
            _ => vec![],
        };
        let at: std::collections::BTreeMap<&str, usize> = months
            .iter()
            .enumerate()
            .map(|(i, m)| (m.as_str(), i))
            .collect();
        let mut month_totals = vec![0usize; months.len()];
        // Folded name → (spelling → documents, the row).
        type Row = (std::collections::BTreeMap<String, usize>, Vec<usize>);
        let mut rows: std::collections::BTreeMap<String, Row> = Default::default();
        let mut documents = 0;
        for (month, themes) in &docs {
            let Some(&m) = at.get(month.as_str()) else {
                continue;
            };
            documents += 1;
            month_totals[m] += 1;
            let mut seen = std::collections::BTreeSet::new();
            for t in themes {
                let f = crate::topics::fold(t);
                if f.is_empty() || !seen.insert(f.clone()) {
                    continue;
                }
                let row = rows
                    .entry(f)
                    .or_insert_with(|| (Default::default(), vec![0; months.len()]));
                *row.0.entry(t.trim().to_string()).or_default() += 1;
                row.1[m] += 1;
            }
        }
        // The spelling shown is the most frequent one (ties: the first in
        // order), as a facet shows it.
        let mut table: Vec<(String, usize, Vec<usize>)> = rows
            .into_values()
            .map(|(spellings, row)| {
                let max = spellings.values().max().copied().unwrap_or(0);
                let name = spellings
                    .into_iter()
                    .find(|(_, c)| *c == max)
                    .map(|(s, _)| s)
                    .unwrap_or_default();
                (name, row.iter().sum(), row)
            })
            .collect();
        table.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        Ok(Reply {
            body: ThemeMonthsResults {
                schema_version: crate::topics::SCHEMA_VERSION,
                query: query.to_string(),
                corpora: used,
                from: months.first().cloned(),
                to: months.last().cloned(),
                themes: table.iter().map(|r| r.0.clone()).collect(),
                theme_totals: table.iter().map(|r| r.1).collect(),
                counts: table.into_iter().map(|r| r.2).collect(),
                month_totals,
                documents,
                months,
            },
            notes,
        })
    }

    /// Every value of one topic field with its document count, over the
    /// documents with at least one indexed passage (those search can
    /// return), optionally in one layer month.
    pub fn topics(&self, q: &TopicsQuery) -> Result<Reply<TopicsResults>> {
        let field: TopicField = q.field.parse()?;
        let filter = DocFilter {
            month: q.month.clone(),
            ..DocFilter::default()
        };
        filter.validate()?;
        let mut notes = vec![];
        let corpora = self.scope(&q.in_, &mut notes)?;
        let mut counter = FacetCounter::default();
        let mut searched = Vec::new();
        for c in &corpora {
            searched.push((c, index::open_existing(&c.config, c.index_path())?));
        }
        let per_corpus = events_are_per_corpus(searched.iter().map(|(_, conn)| conn))?;
        for (c, conn) in &searched {
            if per_corpus {
                counter.corpus(c.id());
            }
            for d in crate::filter::load_docs(conn)?.values() {
                if d.topic.is_some() && d.tokens > 0 && filter.matches(d) {
                    counter.add(d);
                }
            }
            counter.add_event_records(&crate::events::load(conn)?);
        }
        let facet = counter.facet(field, q.top);
        Ok(Reply {
            body: TopicsResults {
                schema_version: crate::topics::SCHEMA_VERSION,
                field,
                month: q.month.clone(),
                corpora: corpora.iter().map(|c| c.id().to_string()).collect(),
                tagged_documents: counter.tagged,
                distinct: facet.distinct,
                items: facet.items,
            },
            notes,
        })
    }

    pub fn kwic(&self, q: &KwicQuery) -> Result<Reply<KwicResults>> {
        let mut notes = vec![];
        let corpora = self.scope(&q.scope.in_, &mut notes)?;
        let mut req = KwicRequest::new(q.term.clone());
        req.width = q.width;
        req.sort = q.sort.parse()?;
        req.limit = q.limit;
        req.filter = q.scope.filter();
        req.near = q.near.clone().map(|word| Near {
            word,
            window: q.window,
        });
        let body = concord::kwic(&corpora, &req)?;
        Ok(Reply { body, notes })
    }

    pub fn collocates(&self, q: &CollocatesQuery) -> Result<Reply<Collocates>> {
        let mut notes = vec![];
        let corpora = self.scope(&q.scope.in_, &mut notes)?;
        let mut req = CollocRequest::new(q.node.clone());
        req.window = q.window;
        req.score = q.score.parse()?;
        req.min_freq = q.min;
        req.top = q.top;
        req.keep_stopwords = q.keep_stopwords;
        req.filter = q.scope.filter();
        req.dist = q.dist.as_deref().map(str::parse).transpose()?;
        let body = match &q.compare {
            None => Collocates::Single(colloc::collocates(&corpora, &req)?),
            Some(spec) => {
                if req.dist.is_some() {
                    return Err(anyhow!("--dist does not combine with --compare"));
                }
                let f = q.scope.filter();
                let label = if f.is_empty() {
                    "all".to_string()
                } else {
                    f.spec()
                };
                let a = Slice::new(label, corpora.clone(), f);
                let b = self.slice(spec, &corpora)?;
                Collocates::Compare(colloc::compare(&a, &b, &req)?)
            }
        };
        Ok(Reply { body, notes })
    }

    pub fn keyness(&self, q: &KeynessQuery) -> Result<Reply<KeynessResults>> {
        let mut notes = vec![];
        // Default corpora only when a slice names none.
        let needs_default = SliceSpec::parse(&q.a)?.corpora.is_empty()
            || SliceSpec::parse(&q.b)?.corpora.is_empty();
        let default = if needs_default {
            self.scope(&q.in_, &mut notes)?
        } else {
            vec![]
        };
        let sa = self.slice(&q.a, &default)?;
        let sb = self.slice(&q.b, &default)?;
        let req = KeynessRequest {
            top: q.top,
            min_freq: q.min,
        };
        let body = keyness::keyness(&sa, &sb, &req)?;
        Ok(Reply { body, notes })
    }

    pub fn dist(&self, q: &DistQuery) -> Result<Reply<Distribution>> {
        let mut notes = vec![];
        let corpora = self.scope(&q.scope.in_, &mut notes)?;
        let body = concord::distribution(&corpora, &q.term, q.by.parse()?, &q.scope.filter())?;
        Ok(Reply { body, notes })
    }

    pub fn ngrams(&self, q: &NgramsQuery) -> Result<Reply<NgramResults>> {
        let mut notes = vec![];
        let corpora = self.scope(&q.scope.in_, &mut notes)?;
        let (n_min, n_max) = ngrams::parse_n_range(&q.n)?;
        let containing = match &q.containing {
            None => None,
            Some(c) => {
                let toks = concord::node_tokens(c);
                if toks.is_empty() {
                    bail!("empty --containing {c:?}: it has no word tokens");
                }
                Some(toks)
            }
        };
        let req = NgramRequest {
            n_min,
            n_max,
            filter: q.scope.filter(),
            since: q.since,
            until: q.until,
            top: q.top,
            strict_stopwords: q.strict_stopwords,
            containing,
            dist: q.dist.as_deref().map(str::parse).transpose()?,
        };
        let body = ngrams::ngrams(&corpora, &req)?;
        Ok(Reply { body, notes })
    }

    pub fn profile(&self, q: &ProfileQuery) -> Result<Reply<Profile>> {
        let mut notes = vec![];
        let corpora = self.scope(&q.scope.in_, &mut notes)?;
        let opts = ProfileOptions {
            min_hits: q.min_hits,
            ..Default::default()
        };
        let body = concord::profile_with(&corpora, &q.word, &q.scope.filter(), &opts)?;
        Ok(Reply { body, notes })
    }

    pub fn verify_quote(&self, q: &VerifyQuoteQuery) -> Result<Reply<VerifyResult>> {
        let mut notes = vec![];
        let corpora = self.scope(&q.in_, &mut notes)?;
        let body = verify::verify_quote(&corpora, &q.text)?;
        Ok(Reply { body, notes })
    }

    pub fn similar(&self, q: &SimilarQuery) -> Result<Reply<SimilarResults>> {
        let mut notes = vec![];
        let seeds = q
            .seeds
            .iter()
            .map(|s| {
                let id: PassageId = s.parse()?;
                Ok((self.corpus(&id.corpus)?, id))
            })
            .collect::<Result<Vec<_>>>()?;
        let corpora = self.scope(&q.in_, &mut notes)?;
        let req = SimilarRequest {
            top: q.top,
            exclude_seeds: q.exclude_seeds,
        };
        let body = similar::similar(&seeds, &corpora, &req)?;
        Ok(Reply { body, notes })
    }

    pub fn cite(&self, q: &CiteQuery) -> Result<Reply<CitedPassage>> {
        let id: PassageId = q.passage_id.parse()?;
        let body = self.corpus(&id.corpus)?.cite(&id)?;
        Ok(Reply::new(body))
    }

    pub fn index_build(&self, q: &IndexBuildQuery) -> Result<Reply<IndexBuildReport>> {
        self.index_build_with(q, &mut |_| {})
    }

    /// [`Engine::index_build`], reporting downloads and embedding progress
    /// as they happen.
    pub fn index_build_with(
        &self,
        q: &IndexBuildQuery,
        on: &mut dyn FnMut(BuildEvent),
    ) -> Result<Reply<IndexBuildReport>> {
        if q.semantic && self.embedder.is_none() {
            bail!("--semantic needs an embedding model; this engine has none");
        }
        let mut reports = Vec::new();
        let mut announced = false;
        let roots: Vec<PathBuf> = self.registry.corpora.iter().map(|c| c.root()).collect();
        for c in self.select(&q.ids)? {
            let mut r = c.build_index_within(q.force, &roots)?;
            if let Some(e) = &self.embedder {
                let vpath = c.vectors_path();
                if q.semantic || vpath.exists() {
                    index::check_target(&vpath, &roots)?;
                    if !announced {
                        if let Some(n) = e.download_note() {
                            on(BuildEvent::Download(n));
                        }
                        announced = true;
                    }
                    let conn = index::open_existing(&c.config, c.index_path())?;
                    let id = c.id().to_string();
                    let mut progress = |done, total| {
                        on(BuildEvent::Embedding {
                            corpus: id.clone(),
                            done,
                            total,
                        })
                    };
                    r.vectors = Some(store::build(&conn, &vpath, e.as_ref(), &mut progress)?);
                }
            }
            reports.push(r);
        }
        Ok(Reply::new(IndexBuildReport {
            schema_version: SCHEMA_VERSION,
            reports,
        }))
    }

    pub fn index_status(&self) -> Result<Reply<IndexStatusReport>> {
        let corpora = self
            .select(&[])?
            .iter()
            .map(|c| c.status())
            .collect::<Result<Vec<_>>>()?;
        Ok(Reply::new(IndexStatusReport {
            schema_version: SCHEMA_VERSION,
            corpora,
        }))
    }

    pub fn corpora_list(&self) -> Result<Reply<CorporaList>> {
        Ok(Reply::new(CorporaList {
            schema_version: SCHEMA_VERSION,
            corpora: self
                .registry
                .corpora
                .iter()
                .map(|c| entry(c, &self.index_dir))
                .collect(),
        }))
    }

    pub fn corpus_show(&self, id: &str) -> Result<Reply<CorpusShow>> {
        let c = self.registry.get(id)?;
        let one = Registry {
            corpora: vec![c.clone()],
        };
        Ok(Reply::new(CorpusShow {
            toml: one.to_toml()?,
            entry: entry(c, &self.index_dir),
        }))
    }

    /// Per normalisation rule: passages changed and before/after samples.
    /// Reads the sources, not the index.
    pub fn clean_report(&self, q: &CleanReportQuery) -> Result<Reply<CleanReport>> {
        let cfg = self.registry.get(&q.corpus)?;
        let opts = CleanOptions {
            samples: q.samples,
            preview_rules: q.rules.clone(),
        };
        Ok(Reply::new(clean::clean_report(cfg, &opts)?))
    }
}

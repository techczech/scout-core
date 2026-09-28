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
use crate::filter::{DocFilter, SliceSpec};
use crate::index::{self, BuildReport, IndexStatus, NoIndexedCorpus};
use crate::keyness::{self, KeynessRequest, KeynessResults};
use crate::ngrams::{self, NgramRequest, NgramResults};
use crate::registry::{self, Registry};
use crate::search::{self, CitedPassage, PassageId, SearchRequest, SearchResults};
use crate::semantic::{self, store, Embedder, Freshness, SearchMode};
use crate::similar::{self, SimilarRequest, SimilarResults};
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
    }
}

/// `corpora init-defaults`: write the default registry to the user registry
/// path (`$SCOUT_CONFIG`, else `~/.config/scout/corpora.toml`).
pub fn init_defaults(force: bool) -> Result<InitDefaults> {
    let path = registry::registry_path();
    let wrote = registry::init_defaults(&path, force)?;
    let reg = Registry::load_from(&path)?;
    let dir = index::index_dir();
    Ok(InitDefaults {
        path: path.display().to_string(),
        wrote,
        corpora: reg.corpora.iter().map(|c| entry(c, &dir)).collect(),
    })
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
    fn scope(&self, ids: &[String], notes: &mut Vec<String>) -> Result<Vec<Corpus>> {
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
        for c in self.select(&q.ids)? {
            let mut r = c.build_index(q.force)?;
            if let Some(e) = &self.embedder {
                let vpath = c.vectors_path();
                if q.semantic || vpath.exists() {
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

//! The per-corpus SQLite + FTS5 index: schema, incremental build and status.
//!
//! The index is derived and disposable (invariant 4): it lives in the
//! platform data dir, never inside a source folder, and a `--force` rebuild
//! deletes the file and builds from scratch.

use crate::adapter::{self, DocRecord};
use crate::markdown::{self, SourceFile};
use crate::normalize;
use crate::registry::CorpusConfig;
use crate::tokenize::{tokenize, IdentityLemmatizer};
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use sha1::{Digest, Sha1};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Bumped whenever normalisation, tokenisation, passage splitting or the
/// schema changes; a mismatch forces a full rebuild.
pub const ENGINE_VERSION: &str = "scout-corpus/6";

/// FTS5 column content is our own tokens joined by spaces; the FTS tokenizer
/// keeps the in-word characters UAX #29 allows so it re-splits only on spaces.
const FTS_TOKENIZE: &str = "unicode61 remove_diacritics 0 tokenchars '''.,;:_'";

/// Raised when a corpus has no index yet.
#[derive(Debug)]
pub struct IndexMissing {
    pub corpus: String,
}

impl std::fmt::Display for IndexMissing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "corpus {:?} has no index; run `scout index build {}`",
            self.corpus, self.corpus
        )
    }
}
impl std::error::Error for IndexMissing {}

/// Raised when a command names no corpus and none of the registered ones
/// has an index.
#[derive(Debug)]
pub struct NoIndexedCorpus(pub Vec<String>);

impl std::fmt::Display for NoIndexedCorpus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "no corpus has an index (registered: {}); run `scout index build`",
            self.0.join(", ")
        )
    }
}
impl std::error::Error for NoIndexedCorpus {}

/// The directory holding all indexes: `$SCOUT_DATA_DIR/indexes`, else
/// `<platform data dir>/scout/indexes`.
pub fn index_dir() -> PathBuf {
    data_base().join("indexes")
}

/// Where downloaded embedding models live: `$SCOUT_DATA_DIR/models`, else
/// `<platform data dir>/scout/models`.
pub fn models_dir() -> PathBuf {
    data_base().join("models")
}

fn data_base() -> PathBuf {
    std::env::var_os("SCOUT_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::data_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("scout")
        })
}

pub fn index_path_in(dir: &Path, corpus_id: &str) -> PathBuf {
    dir.join(format!("{corpus_id}.sqlite"))
}

/// The passage vector store beside a corpus's FTS index (a separate file,
/// so the FTS index and its version are untouched).
pub fn vectors_path_in(dir: &Path, corpus_id: &str) -> PathBuf {
    dir.join(format!("{corpus_id}.vectors.sqlite"))
}

/// Meta key naming the index's content generation: it changes whenever a
/// build may have changed a passage (ids or text).
const GENERATION_KEY: &str = "content_generation";

/// The index's content generation (None for an index built before it
/// existed; the next build sets one).
pub fn content_generation(conn: &Connection) -> Result<Option<String>> {
    meta_get(conn, GENERATION_KEY)
}

pub(crate) fn remove_index_files(path: &Path) -> Result<()> {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let p = PathBuf::from(format!("{}{}", path.display(), suffix));
        if p.exists() {
            std::fs::remove_file(&p).with_context(|| format!("remove {}", p.display()))?;
        }
    }
    Ok(())
}

/// Canonicalise the longest existing ancestor of `p`, re-appending the rest.
fn canonical_prefix(p: &Path) -> PathBuf {
    let mut tail = Vec::new();
    let mut cur = p.to_path_buf();
    loop {
        if let Ok(c) = cur.canonicalize() {
            let mut out = c;
            for t in tail.iter().rev() {
                out.push(t);
            }
            return out;
        }
        match (cur.file_name().map(|f| f.to_os_string()), cur.parent()) {
            (Some(f), Some(parent)) => {
                tail.push(f);
                cur = parent.to_path_buf();
            }
            _ => return p.to_path_buf(),
        }
    }
}

fn fingerprint(cfg: &CorpusConfig) -> String {
    let v = serde_json::json!({
        "engine": ENGINE_VERSION,
        "kind": cfg.kind,
        "path": cfg.root(),
        "include": cfg.include,
        "exclude": cfg.exclude,
        "require_frontmatter": cfg.require_frontmatter,
        "field_map": cfg.field_map,
        "default_author": cfg.default_author,
        "boilerplate": cfg.boilerplate,
        "document_unit": cfg.document_unit,
    });
    hex_sha1(v.to_string().as_bytes())
}

fn hex_sha1(bytes: &[u8]) -> String {
    let mut h = Sha1::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn init_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(&format!(
        "
        CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS files (
            rel_path TEXT PRIMARY KEY,
            mtime_ns INTEGER NOT NULL,
            size INTEGER NOT NULL,
            hash TEXT NOT NULL,
            is_doc INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS documents (
            rel_path TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            date TEXT,
            genre TEXT,
            topics TEXT NOT NULL DEFAULT '[]',
            lang TEXT,
            summary TEXT,
            author TEXT,
            public_url TEXT,
            kind TEXT,
            source TEXT,
            date_source TEXT,
            handle TEXT,
            passages INTEGER NOT NULL,
            tokens INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS passages (
            id INTEGER PRIMARY KEY,
            rel_path TEXT NOT NULL,
            ord INTEGER NOT NULL,
            line_start INTEGER NOT NULL,
            line_end INTEGER NOT NULL,
            original TEXT NOT NULL,
            normalized TEXT NOT NULL,
            tokens INTEGER NOT NULL,
            tags TEXT NOT NULL DEFAULT '[]',
            color TEXT,
            saved_at TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_passages_doc ON passages(rel_path, line_start);
        CREATE VIRTUAL TABLE IF NOT EXISTS passages_fts USING fts5(tokens, tokenize = \"{FTS_TOKENIZE}\");
        CREATE VIRTUAL TABLE IF NOT EXISTS titles_fts USING fts5(rel_path UNINDEXED, tokens, tokenize = \"{FTS_TOKENIZE}\");
        CREATE VIRTUAL TABLE IF NOT EXISTS fields_fts USING fts5(rel_path UNINDEXED, tokens, tokenize = \"{FTS_TOKENIZE}\");
        "
    ))?;
    Ok(())
}

fn meta_get(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
        .optional()?)
}

fn meta_set(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)",
        params![key, value],
    )?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BuildReport {
    pub corpus: String,
    pub index_path: String,
    /// True when every file was (re)indexed: first build, `--force`, or an
    /// engine / config change.
    pub full: bool,
    pub docs: i64,
    pub passages: i64,
    pub tokens: i64,
    pub files_seen: usize,
    pub indexed: usize,
    pub unchanged: usize,
    pub removed: usize,
    /// Files seen that fail `require_frontmatter` (all of them, not just
    /// those read this run).
    pub not_documents: usize,
    pub elapsed_ms: u128,
    /// The passage vector build, when the build had an embedder and the
    /// corpus has (or was asked for) vectors.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vectors: Option<crate::semantic::VectorReport>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct StaleFiles {
    pub added: usize,
    pub changed: usize,
    pub removed: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct IndexStatus {
    pub corpus: String,
    pub kind: String,
    pub index_path: String,
    pub exists: bool,
    pub docs: i64,
    pub passages: i64,
    pub tokens: i64,
    pub built_at: Option<String>,
    /// False when the engine version or the registry entry changed since the
    /// build; the next build is then full.
    pub config_current: bool,
    pub stale: Option<StaleFiles>,
}

/// SQL matching the document keys of one source file: the file itself, or
/// `<file>#<fragment>` when it holds several documents.
const OF_FILE: &str = "(rel_path = ?1 OR substr(rel_path, 1, length(?1) + 1) = ?1 || '#')";

/// Remove a source file and every document it produced.
fn delete_doc(conn: &Connection, rel_path: &str) -> Result<()> {
    conn.execute(
        &format!(
            "DELETE FROM passages_fts WHERE rowid IN (SELECT id FROM passages WHERE {OF_FILE})"
        ),
        [rel_path],
    )?;
    conn.execute(&format!("DELETE FROM passages WHERE {OF_FILE}"), [rel_path])?;
    conn.execute(
        &format!("DELETE FROM documents WHERE {OF_FILE}"),
        [rel_path],
    )?;
    conn.execute(
        &format!("DELETE FROM titles_fts WHERE {OF_FILE}"),
        [rel_path],
    )?;
    conn.execute(
        &format!("DELETE FROM fields_fts WHERE {OF_FILE}"),
        [rel_path],
    )?;
    conn.execute("DELETE FROM files WHERE rel_path = ?1", [rel_path])?;
    Ok(())
}

fn index_file(
    conn: &Connection,
    cfg: &CorpusConfig,
    rules: &[regex::Regex],
    f: &SourceFile,
    text: &str,
    hash: &str,
) -> Result<bool> {
    let docs = adapter::records(cfg, &f.rel_path, text);
    let is_doc = !docs.is_empty();
    conn.execute(
        "INSERT INTO files (rel_path, mtime_ns, size, hash, is_doc) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![f.rel_path, f.mtime_ns, f.size, hash, is_doc as i64],
    )?;
    for d in &docs {
        index_doc(conn, rules, f, d)?;
    }
    Ok(is_doc)
}

fn index_doc(
    conn: &Connection,
    rules: &[regex::Regex],
    f: &SourceFile,
    d: &DocRecord,
) -> Result<()> {
    let meta = &d.meta;
    let lang = meta.lang.clone();
    let mut ins_p = conn.prepare_cached(
        "INSERT INTO passages (rel_path, ord, line_start, line_end, original, normalized, tokens, tags, color, saved_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
    )?;
    let mut ins_f =
        conn.prepare_cached("INSERT INTO passages_fts (rowid, tokens) VALUES (?1, ?2)")?;
    let mut n_pass = 0i64;
    let mut n_tok = 0i64;
    for p in &d.passages {
        let m = normalize::normalize(&p.original, rules);
        let toks = tokenize(&m.text, &IdentityLemmatizer, lang.as_deref());
        if toks.is_empty() {
            continue;
        }
        let joined = toks
            .iter()
            .map(|t| t.lemma.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        ins_p.execute(params![
            d.key,
            n_pass,
            p.line_start as i64,
            p.line_end as i64,
            p.original,
            m.text,
            toks.len() as i64,
            serde_json::to_string(&p.tags)?,
            p.color,
            p.saved_at
        ])?;
        let id = conn.last_insert_rowid();
        ins_f.execute(params![id, joined])?;
        n_pass += 1;
        n_tok += toks.len() as i64;
    }
    let title = meta.title.clone().unwrap_or_else(|| {
        Path::new(&f.rel_path)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    });
    conn.execute(
        "INSERT INTO documents (rel_path, title, date, genre, topics, lang, summary, author, public_url, kind, source, date_source, handle, passages, tokens)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        params![
            d.key,
            title,
            meta.date,
            meta.genre,
            serde_json::to_string(&meta.topics)?,
            meta.lang,
            meta.summary,
            meta.author,
            meta.public_url,
            meta.kind,
            meta.source,
            meta.date_source,
            meta.handle,
            n_pass,
            n_tok
        ],
    )?;
    let field_tokens = |s: &str| {
        let norm = normalize::normalize(s, &[]);
        tokenize(&norm.text, &IdentityLemmatizer, lang.as_deref())
            .into_iter()
            .map(|t| t.lemma)
            .collect::<Vec<_>>()
    };
    // The document fields search may match a term in when the passage lacks
    // it: the title (when indexed), the topics and the summary.
    let mut fields: Vec<String> = Vec::new();
    if d.index_title {
        let joined = field_tokens(&title).join(" ");
        conn.execute(
            "INSERT INTO titles_fts (rel_path, tokens) VALUES (?1, ?2)",
            params![d.key, joined],
        )?;
        fields.push(joined);
    }
    for t in &meta.topics {
        fields.push(field_tokens(t).join(" "));
    }
    if let Some(s) = &meta.summary {
        fields.push(field_tokens(s).join(" "));
    }
    // One row per field, so a phrase never spans two fields.
    let mut ins_fields =
        conn.prepare_cached("INSERT INTO fields_fts (rel_path, tokens) VALUES (?1, ?2)")?;
    for f in fields.iter().filter(|f| !f.is_empty()) {
        ins_fields.execute(params![d.key, f])?;
    }
    Ok(())
}

fn totals(conn: &Connection) -> Result<(i64, i64, i64)> {
    Ok(conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(passages), 0), COALESCE(SUM(tokens), 0) FROM documents",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?)
}

/// Build or update the index at `path` for `cfg`.
pub fn build(cfg: &CorpusConfig, path: &Path, force: bool) -> Result<BuildReport> {
    let started = Instant::now();
    let root = cfg.root();
    if let (Ok(r), Some(dir)) = (root.canonicalize(), path.parent()) {
        let d = canonical_prefix(dir);
        if d.starts_with(&r) {
            bail!(
                "index path {} is inside the source folder {}; refusing",
                path.display(),
                r.display()
            );
        }
    }
    let rules = normalize::compile_rules(&cfg.boilerplate)?;
    if force {
        remove_index_files(path)?;
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let fp = fingerprint(cfg);
    let mut full = force || !path.exists();
    {
        let conn = scout_index::sqlite::open(path)?;
        init_schema(&conn)?;
        if meta_get(&conn, "fingerprint")?.as_deref() != Some(fp.as_str()) {
            full = true;
        }
    }
    if full && path.exists() {
        remove_index_files(path)?;
    }
    let mut conn = scout_index::sqlite::open(path)?;
    init_schema(&conn)?;

    let files = markdown::discover(cfg)?;
    let mut existing: BTreeMap<String, (i64, i64, String)> = BTreeMap::new();
    {
        let mut st = conn.prepare("SELECT rel_path, mtime_ns, size, hash FROM files")?;
        let rows = st.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get(2)?, r.get(3)?)))
        })?;
        for row in rows {
            let (k, v) = row?;
            existing.insert(k, v);
        }
    }

    let tx = conn.transaction()?;
    let (mut indexed, mut unchanged) = (0usize, 0usize);
    for f in &files {
        let prev = existing.remove(&f.rel_path);
        if let Some((mt, sz, _)) = &prev {
            if *mt == f.mtime_ns && *sz == f.size {
                unchanged += 1;
                continue;
            }
        }
        let text = markdown::read_source(&f.abs_path)?;
        let hash = hex_sha1(text.as_bytes());
        if let Some((_, _, h)) = &prev {
            if *h == hash {
                tx.execute(
                    "UPDATE files SET mtime_ns = ?2, size = ?3 WHERE rel_path = ?1",
                    params![f.rel_path, f.mtime_ns, f.size],
                )?;
                unchanged += 1;
                continue;
            }
            delete_doc(&tx, &f.rel_path)?;
        }
        if index_file(&tx, cfg, &rules, f, &text, &hash)? {
            indexed += 1;
        }
    }
    let removed = existing.len();
    for rel in existing.keys() {
        delete_doc(&tx, rel)?;
    }
    // A new content generation whenever passages may have changed, so a
    // vector store built from an older generation knows it is stale. A
    // meta row, not a schema change: ENGINE_VERSION stays.
    if full || indexed > 0 || removed > 0 || meta_get(&tx, GENERATION_KEY)?.is_none() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        meta_set(&tx, GENERATION_KEY, &format!("{now:x}-{}", files.len()))?;
    }
    meta_set(&tx, "fingerprint", &fp)?;
    meta_set(&tx, "engine", ENGINE_VERSION)?;
    meta_set(&tx, "corpus", &cfg.id)?;
    meta_set(
        &tx,
        "built_at",
        &chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
    )?;
    tx.commit()?;
    if full {
        conn.execute(
            "INSERT INTO passages_fts(passages_fts) VALUES('optimize')",
            [],
        )?;
    }
    let (docs, passages, tokens) = totals(&conn)?;
    let not_docs: i64 = conn.query_row("SELECT COUNT(*) FROM files WHERE is_doc = 0", [], |r| {
        r.get(0)
    })?;
    let not_docs = not_docs as usize;
    Ok(BuildReport {
        corpus: cfg.id.clone(),
        index_path: path.display().to_string(),
        full,
        docs,
        passages,
        tokens,
        files_seen: files.len(),
        indexed,
        unchanged,
        removed,
        not_documents: not_docs,
        elapsed_ms: started.elapsed().as_millis(),
        vectors: None,
    })
}

/// Open an existing index read-only-ish (no schema creation).
pub fn open_existing(cfg: &CorpusConfig, path: &Path) -> Result<Connection> {
    if !path.exists() {
        return Err(IndexMissing {
            corpus: cfg.id.clone(),
        }
        .into());
    }
    let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let has: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE name IN ('documents', 'passages', 'passages_fts', 'titles_fts', 'meta')",
        [],
        |r| r.get(0),
    )?;
    if has < 5 {
        return Err(IndexMissing {
            corpus: cfg.id.clone(),
        }
        .into());
    }
    Ok(conn)
}

pub fn status(cfg: &CorpusConfig, path: &Path) -> Result<IndexStatus> {
    let mut st = IndexStatus {
        corpus: cfg.id.clone(),
        kind: cfg.kind.as_str().into(),
        index_path: path.display().to_string(),
        exists: false,
        docs: 0,
        passages: 0,
        tokens: 0,
        built_at: None,
        config_current: false,
        stale: None,
    };
    let conn = match open_existing(cfg, path) {
        Ok(c) => c,
        Err(e) if e.downcast_ref::<IndexMissing>().is_some() => return Ok(st),
        Err(e) => return Err(e),
    };
    st.exists = true;
    let (d, p, t) = totals(&conn)?;
    st.docs = d;
    st.passages = p;
    st.tokens = t;
    st.built_at = meta_get(&conn, "built_at")?;
    st.config_current =
        meta_get(&conn, "fingerprint")?.as_deref() == Some(fingerprint(cfg).as_str());
    if cfg.root().is_dir() {
        let mut existing: BTreeMap<String, (i64, i64)> = BTreeMap::new();
        let mut q = conn.prepare("SELECT rel_path, mtime_ns, size FROM files")?;
        for row in q.query_map([], |r| Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get(2)?))))? {
            let (k, v) = row?;
            existing.insert(k, v);
        }
        let (mut added, mut changed) = (0, 0);
        for f in markdown::discover(cfg)? {
            match existing.remove(&f.rel_path) {
                None => added += 1,
                Some((mt, sz)) if mt != f.mtime_ns || sz != f.size => changed += 1,
                _ => {}
            }
        }
        st.stale = Some(StaleFiles {
            added,
            changed,
            removed: existing.len(),
        });
    }
    Ok(st)
}

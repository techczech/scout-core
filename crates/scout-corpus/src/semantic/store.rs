//! The passage vector store: `<corpus>.vectors.sqlite` beside the FTS
//! index, one unit vector per passage row.
//!
//! - Passages under [`MIN_TOKENS`] tokens get no vector.
//! - Rows are keyed by the FTS index's passage id and carry the SHA-1 of the
//!   text embedded (the passage's normalised text), so an incremental build
//!   embeds only new or changed text and reuses a vector whose passage moved
//!   to a new id.
//! - The store records the embedder's model id and the FTS index's content
//!   generation it was built from. A different model rebuilds everything; a
//!   different generation means the store is stale until the next build.
//! - Derived and disposable like the index; the FTS index is only read.

use super::embed::{l2_normalise, Embedder};
use crate::index;
use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

/// Passages embedded per call (and per committed chunk).
const CHUNK: usize = 1024;
/// Passages shorter than this many tokens get no vector: a heading such as
/// "Mind Mapping software" matches any query on its words and crowds out
/// the paragraphs that discuss them.
pub const MIN_TOKENS: i64 = 5;
/// What a store holds besides the model (passage selection, text); a store
/// of another scheme is re-selected on the next build (reusing vectors).
const SCHEME: &str = "1";

/// `index build --semantic`: what the vector build did.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct VectorReport {
    pub vectors_path: String,
    pub model: String,
    pub dim: usize,
    /// True when every vector was rebuilt (first build or a model change).
    pub full: bool,
    pub passages: usize,
    pub embedded: usize,
    pub reused: usize,
    pub removed: usize,
    pub bytes: u64,
    pub elapsed_ms: u128,
}

/// Whether a corpus's vectors can rank a query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Freshness {
    Missing,
    /// Built with another model, or from an older index generation.
    Stale(String),
    Current,
}

/// Progress of a vector build: passages embedded so far of those to embed.
pub type Progress<'a> = &'a mut dyn FnMut(usize, usize);

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

fn text_hash(text: &str) -> Vec<u8> {
    let mut h = Sha1::new();
    h.update(text.as_bytes());
    h.finalize().to_vec()
}

fn to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn from_blob(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

fn file_bytes(path: &Path) -> u64 {
    ["", "-wal"]
        .iter()
        .filter_map(|s| std::fs::metadata(format!("{}{s}", path.display())).ok())
        .map(|m| m.len())
        .sum()
}

/// Is the store at `path` usable for `model` against this FTS index?
pub fn freshness(fts: &Connection, path: &Path, model: &str) -> Result<Freshness> {
    if !path.exists() {
        return Ok(Freshness::Missing);
    }
    let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let has_meta: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE name IN ('meta', 'vectors')",
        [],
        |r| r.get(0),
    )?;
    if has_meta < 2 {
        return Ok(Freshness::Missing);
    }
    let built = meta_get(&conn, "model")?;
    if built.as_deref() != Some(model) {
        return Ok(Freshness::Stale(format!(
            "built with model {}, not {model}",
            built.unwrap_or_else(|| "?".into())
        )));
    }
    let generation = index::content_generation(fts)?;
    if generation.is_none()
        || meta_get(&conn, "generation")? != generation
        || meta_get(&conn, "scheme")?.as_deref() != Some(SCHEME)
    {
        return Ok(Freshness::Stale("the index changed since".into()));
    }
    Ok(Freshness::Current)
}

/// Every vector of a store: `ids[i]` owns `data[i*dim..(i+1)*dim]`, in id
/// order.
#[derive(Debug, Clone, Default)]
pub struct Vectors {
    pub dim: usize,
    pub ids: Vec<i64>,
    pub data: Vec<f32>,
}

impl Vectors {
    pub fn get(&self, i: usize) -> &[f32] {
        &self.data[i * self.dim..(i + 1) * self.dim]
    }

    /// `(cosine, passage id)` for every vector, best first, ties by id.
    /// Vectors are unit length, so the cosine is the dot product; each sum
    /// runs in a fixed order (deterministic).
    pub fn rank(&self, q: &[f32]) -> Vec<(f32, i64)> {
        let mut out: Vec<(f32, i64)> = self
            .ids
            .iter()
            .enumerate()
            .map(|(i, &id)| {
                let v = self.get(i);
                let s: f32 = v.iter().zip(q).map(|(a, b)| a * b).sum();
                (s, id)
            })
            .collect();
        out.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        out
    }
}

pub fn load(path: &Path) -> Result<Vectors> {
    let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let dim: usize = meta_get(&conn, "dim")?
        .and_then(|d| d.parse().ok())
        .unwrap_or(0);
    let mut v = Vectors {
        dim,
        ..Default::default()
    };
    let mut st = conn.prepare("SELECT id, vec FROM vectors ORDER BY id")?;
    let mut rows = st.query([])?;
    while let Some(r) = rows.next()? {
        let blob: Vec<u8> = r.get(1)?;
        if blob.len() != dim * 4 {
            bail!("vector store {} is corrupt; rebuild it", path.display());
        }
        v.ids.push(r.get(0)?);
        v.data.extend(from_blob(&blob));
    }
    Ok(v)
}

fn init(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         CREATE TABLE IF NOT EXISTS vectors (id INTEGER PRIMARY KEY, hash BLOB NOT NULL, vec BLOB NOT NULL);
         CREATE INDEX IF NOT EXISTS idx_vectors_hash ON vectors(hash);",
    )?;
    Ok(())
}

/// Build or update the store at `path` from the FTS index `fts`.
pub fn build(
    fts: &Connection,
    path: &Path,
    embedder: &dyn Embedder,
    progress: Progress<'_>,
) -> Result<VectorReport> {
    let started = Instant::now();
    let model = embedder.model_id();
    let generation = index::content_generation(fts)?;
    let mut conn = scout_index::sqlite::open(path)?;
    init(&conn)?;
    let mut full = false;
    if meta_get(&conn, "model")?.as_deref() != Some(model.as_str()) {
        conn.execute_batch("DELETE FROM vectors; DELETE FROM meta;")?;
        full = true;
    }
    let count = |c: &Connection| -> Result<usize> {
        Ok(c.query_row("SELECT COUNT(*) FROM vectors", [], |r| r.get::<_, i64>(0))? as usize)
    };
    let dim_of = |c: &Connection| -> Result<usize> {
        Ok(meta_get(c, "dim")?
            .and_then(|d| d.parse().ok())
            .unwrap_or(0))
    };
    // Nothing changed since the last build: no text is read.
    if !full
        && generation.is_some()
        && meta_get(&conn, "generation")? == generation
        && meta_get(&conn, "scheme")?.as_deref() == Some(SCHEME)
    {
        let passages = count(&conn)?;
        return Ok(VectorReport {
            vectors_path: path.display().to_string(),
            model,
            dim: dim_of(&conn)?,
            full,
            passages,
            embedded: 0,
            reused: passages,
            removed: 0,
            bytes: file_bytes(path),
            elapsed_ms: started.elapsed().as_millis(),
        });
    }

    // What the store holds: id → hash, and hash → an id holding it.
    let mut have: HashMap<i64, Vec<u8>> = HashMap::new();
    {
        let mut st = conn.prepare("SELECT id, hash FROM vectors")?;
        for row in st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)))? {
            let (id, h) = row?;
            have.insert(id, h);
        }
    }
    let mut by_hash: HashMap<Vec<u8>, i64> = HashMap::new();
    for (id, h) in &have {
        by_hash
            .entry(h.clone())
            .and_modify(|x| *x = (*x).min(*id))
            .or_insert(*id);
    }

    // What the index holds now.
    let mut keep: Vec<i64> = Vec::new();
    let mut moved: Vec<(i64, Vec<u8>, i64)> = Vec::new(); // (new id, hash, old id)
    let mut todo: Vec<(i64, Vec<u8>, String)> = Vec::new();
    {
        let mut st =
            fts.prepare("SELECT id, normalized FROM passages WHERE tokens >= ?1 ORDER BY id")?;
        let mut rows = st.query([MIN_TOKENS])?;
        while let Some(r) = rows.next()? {
            let id: i64 = r.get(0)?;
            let text: String = r.get(1)?;
            let h = text_hash(&text);
            if have.get(&id) == Some(&h) {
                keep.push(id);
            } else if let Some(&old) = by_hash.get(&h) {
                moved.push((id, h, old));
            } else {
                todo.push((id, h, text));
            }
        }
    }
    let live: std::collections::HashSet<i64> = keep.iter().copied().collect();
    let current: std::collections::HashSet<i64> = keep
        .iter()
        .copied()
        .chain(moved.iter().map(|m| m.0))
        .chain(todo.iter().map(|t| t.0))
        .collect();
    let removed = have.keys().filter(|id| !current.contains(id)).count();
    let reused = keep.len() + moved.len();

    // Moved vectors are read before any row is replaced.
    let mut moved_vecs: Vec<(i64, Vec<u8>, Vec<u8>)> = Vec::with_capacity(moved.len());
    {
        let mut st = conn.prepare_cached("SELECT vec FROM vectors WHERE id = ?1")?;
        for (id, h, old) in moved {
            let v: Vec<u8> = st.query_row([old], |r| r.get(0))?;
            moved_vecs.push((id, h, v));
        }
    }
    let stale: Vec<i64> = have
        .keys()
        .filter(|id| !live.contains(id))
        .copied()
        .collect();
    {
        let tx = conn.transaction()?;
        {
            let mut del = tx.prepare_cached("DELETE FROM vectors WHERE id = ?1")?;
            for id in &stale {
                del.execute([id])?;
            }
            let mut ins = tx.prepare_cached(
                "INSERT OR REPLACE INTO vectors (id, hash, vec) VALUES (?1, ?2, ?3)",
            )?;
            for (id, h, v) in &moved_vecs {
                ins.execute(params![id, h, v])?;
            }
        }
        // The stale generation until every vector is in.
        meta_set(&tx, "model", &model)?;
        meta_set(&tx, "generation", "building")?;
        tx.commit()?;
    }

    // Embed the rest, shortest first (less padding per batch), committing
    // each chunk so an interrupted build resumes where it stopped.
    todo.sort_by(|a, b| a.2.len().cmp(&b.2.len()).then(a.0.cmp(&b.0)));
    let total = todo.len();
    let mut dim = dim_of(&conn)?;
    let mut done = 0usize;
    progress(0, total);
    for chunk in todo.chunks(CHUNK) {
        let texts: Vec<&str> = chunk.iter().map(|(_, _, t)| t.as_str()).collect();
        let vecs = embedder.embed_passages(&texts)?;
        if vecs.len() != chunk.len() {
            bail!(
                "embedder returned {} vectors for {} passages",
                vecs.len(),
                chunk.len()
            );
        }
        let tx = conn.transaction()?;
        {
            let mut ins = tx.prepare_cached(
                "INSERT OR REPLACE INTO vectors (id, hash, vec) VALUES (?1, ?2, ?3)",
            )?;
            for ((id, h, _), mut v) in chunk.iter().zip(vecs) {
                if dim == 0 {
                    dim = v.len();
                    meta_set(&tx, "dim", &dim.to_string())?;
                }
                if v.len() != dim {
                    bail!("embedder returned a {}-d vector, expected {dim}", v.len());
                }
                l2_normalise(&mut v);
                ins.execute(params![id, h, to_blob(&v)])?;
            }
        }
        tx.commit()?;
        done += chunk.len();
        progress(done, total);
    }
    meta_set(&conn, "generation", generation.as_deref().unwrap_or(""))?;
    meta_set(&conn, "scheme", SCHEME)?;
    meta_set(
        &conn,
        "built_at",
        &chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
    )?;
    let passages = count(&conn)?;
    // Many rows gone (a model change, a removed folder): give the space back.
    if removed > 0 && removed * 10 > passages {
        conn.execute_batch("VACUUM;")?;
    }
    if full || total > 0 || removed > 0 {
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    }
    Ok(VectorReport {
        vectors_path: path.display().to_string(),
        model,
        dim,
        full,
        passages,
        embedded: total,
        reused,
        removed,
        bytes: file_bytes(path),
        elapsed_ms: started.elapsed().as_millis(),
    })
}

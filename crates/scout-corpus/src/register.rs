//! Corpus registration: `corpora add` and `corpora remove` as a
//! read-modify-write of the registry file, plus validation.
//!
//! - The only files written are the registry (atomically: a temp file in
//!   the same directory, created exclusively, then a rename), its lock file
//!   and, on remove, the corpus's own index files. A corpus's source folder
//!   is never written or deleted: an index file that lies inside any
//!   registered source folder is refused, and a folder that holds the index
//!   store (or lies inside it) cannot be registered. Every write (add,
//!   remove, init-defaults) first runs one precondition,
//!   [`check_registry_location`]: the registry is not a symlink and lies
//!   inside no source folder (for add, including the new one).
//! - Every read-modify-write holds an exclusive lock on `<registry>.lock`
//!   (beside the registry), so concurrent adds and removes, in threads or
//!   processes, never lose an update.
//! - Removal deletes the index files first and unregisters only when every
//!   one is gone, so a failed removal can be retried.
//! - An edit keeps the rest of the file byte-for-byte where it can (a new
//!   entry is appended; a removed entry's lines are cut). When the edited
//!   text would not parse back to exactly the intended registry, the file
//!   is regenerated with its leading comment block kept.
//! - Validation lives here, so the CLI and the apps refuse the same things.

use crate::index;
use crate::registry::{
    expand_home, registry_path, CorpusConfig, CorpusKind, DocumentUnit, FieldMap, Registry,
    RegistryMissing, REGISTRY_HEADER,
};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Schema version of the `corpora remove --json` body.
pub const SCHEMA_VERSION: u32 = 1;

/// The include globs of a new markdown-folder corpus.
pub const DEFAULT_INCLUDE: [&str; 2] = ["**/*.md", "**/*.txt"];

/// A request to register a folder. Every field but `path` is optional; the
/// defaults are the CLI's (`scout corpora add <path>`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AddCorpus {
    /// The folder; `~` is expanded. Stored absolute, with `~` when under
    /// the home folder.
    pub path: String,
    /// Default: a slug of the folder name, de-duplicated (`notes-2`).
    pub id: Option<String>,
    /// Default: the folder name.
    pub name: Option<String>,
    /// Default: markdown-folder. A Highlight Scout archive is registered
    /// only when asked for (`kind = highlight-scout-archive`); its folder
    /// must have `readings/works/`.
    pub kind: Option<CorpusKind>,
    /// The author of documents that name none.
    pub author: Option<String>,
    /// Default for markdown-folder: `**/*.md`, `**/*.txt`.
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

/// What `corpora remove` did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Removed {
    pub schema_version: u32,
    pub id: String,
    /// The source folder as written in the registry (left untouched).
    pub path: String,
    /// True when `keep_index` was asked for.
    pub index_kept: bool,
    /// The index files deleted (word index and meaning vectors, with their
    /// SQLite side files), absolute paths.
    pub deleted: Vec<String>,
}

type RenameFn = fn(&Path, &Path) -> std::io::Result<()>;

/// The registry file and index directory registration works against.
#[derive(Debug, Clone)]
pub struct RegistryStore {
    path: PathBuf,
    index_dir: PathBuf,
    rename: RenameFn,
}

impl RegistryStore {
    pub fn new(registry_path: impl Into<PathBuf>, index_dir: impl Into<PathBuf>) -> RegistryStore {
        RegistryStore {
            path: registry_path.into(),
            index_dir: index_dir.into(),
            rename: |a, b| std::fs::rename(a, b),
        }
    }

    /// The user registry and platform index dir, as the CLI uses them
    /// (`SCOUT_CONFIG`, `XDG_CONFIG_HOME`, `SCOUT_DATA_DIR` honoured).
    pub fn from_env() -> RegistryStore {
        RegistryStore::new(registry_path(), index::index_dir())
    }

    /// Test seam: replace the final rename of an atomic write (to simulate
    /// a write interrupted before the new file lands).
    #[doc(hidden)]
    pub fn with_rename(mut self, rename: RenameFn) -> RegistryStore {
        self.rename = rename;
        self
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The file's text and registry, or None when there is no file. A file
    /// that does not parse is an error (and is never rewritten).
    fn read(&self) -> Result<Option<(String, Registry)>> {
        if !self.path.exists() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(&self.path)
            .with_context(|| format!("read {}", self.path.display()))?;
        let reg = Registry::parse(&text)
            .with_context(|| format!("corpus registry {} left unchanged", self.path.display()))?;
        Ok(Some((text, reg)))
    }

    /// Register a folder; creates the registry when missing. Does not build
    /// the index.
    pub fn add(&self, req: &AddCorpus) -> Result<CorpusConfig> {
        // First, before anything can create a directory: the requested
        // folder must exist, so its canonical path is known.
        let src = resolve_source(req)?;
        // Before the lock: taking it creates the lock file, which must not
        // land inside a source folder, registered or new.
        refuse_index_store_overlap(&src.canonical, &self.index_dir)?;
        let mut sources = registered_sources(&self.path);
        sources.push(src.canonical.clone());
        check_registry_location(&self.path, &sources)?;
        let _lock = lock_registry(&self.path)?;
        let existing = self.read()?;
        let reg = existing
            .as_ref()
            .map(|(_, r)| r.clone())
            .unwrap_or_default();
        let cfg = new_config(req, &src, &reg)?;
        let mut want = reg.clone();
        want.corpora.push(cfg.clone());
        let text = match &existing {
            None => want.render(REGISTRY_HEADER)?,
            Some((old, _)) => {
                let block = Registry {
                    corpora: vec![cfg.clone()],
                }
                .to_toml()?;
                let mut t = old.clone();
                if !t.is_empty() && !t.ends_with('\n') {
                    t.push('\n');
                }
                if !t.is_empty() {
                    t.push('\n');
                }
                t.push_str(&block);
                spliced_or_rendered(t, &want, old)?
            }
        };
        write_atomic(&self.path, &text, self.rename)?;
        Ok(cfg)
    }

    /// Unregister a corpus and, unless `keep_index`, delete its index files
    /// first. The source folder is never touched. When an index file cannot
    /// be deleted (or must not be: it lies inside a registered source
    /// folder, or another corpus's id differs only by case and so shares
    /// it), the error names it and the corpus stays registered, so a retry
    /// (or `keep_index`) works.
    pub fn remove(&self, id: &str, keep_index: bool) -> Result<Removed> {
        check_registry_location(&self.path, &registered_sources(&self.path))?;
        let _lock = lock_registry(&self.path)?;
        let Some((old, reg)) = self.read()? else {
            return Err(RegistryMissing(self.path.clone()).into());
        };
        let cfg = reg.get(id)?.clone();
        let pos = reg.corpora.iter().position(|c| c.id == id).unwrap_or(0);
        let mut want = reg.clone();
        want.corpora.remove(pos);
        let text = spliced_or_rendered(cut_entry(&old, pos, reg.corpora.len()), &want, &old)?;
        let deleted = if keep_index {
            vec![]
        } else {
            delete_index_files(&self.index_dir, id, &reg)?
        };
        write_atomic(&self.path, &text, self.rename)?;
        Ok(Removed {
            schema_version: SCHEMA_VERSION,
            id: cfg.id,
            path: cfg.path,
            index_kept: keep_index,
            deleted,
        })
    }
}

/// A requested source folder: as given (absolute) and canonical.
struct Source {
    given: PathBuf,
    canonical: PathBuf,
}

/// The requested folder, which must exist and be a folder. The filesystem
/// resolves symlinks before `..`, so `link/../x` is the `x` beside the
/// link's target, not beside the link.
fn resolve_source(req: &AddCorpus) -> Result<Source> {
    if req.path.trim().is_empty() {
        bail!("no folder given");
    }
    let given = std::path::absolute(expand_home(req.path.trim()))
        .with_context(|| format!("resolve {}", req.path.trim()))?;
    let canonical = match given.canonicalize() {
        Ok(c) => c,
        Err(_) => bail!("no folder at {}", normalise(&given)?.display()),
    };
    if !canonical.is_dir() {
        bail!("{} is a file, not a folder", canonical.display());
    }
    Ok(Source { given, canonical })
}

/// Validate a request against the registry and build the new entry.
fn new_config(req: &AddCorpus, src: &Source, reg: &Registry) -> Result<CorpusConfig> {
    let (given, abs) = (&src.given, &src.canonical);
    for c in &reg.corpora {
        if source_root(c) == *abs {
            bail!("path {} already registered as `{}`", abs.display(), c.id);
        }
    }
    // Stored as the user spelled it (a symlink kept as a symlink, e.g. a
    // stable link to a cloud-storage mount), cleaned of `.`; only a path
    // with `..` is stored canonical, since its meaning depends on symlinks.
    // Validation and guards above always use the canonical path.
    let spelled = if given.components().any(|c| c == Component::ParentDir) {
        abs.clone()
    } else {
        normalise(given)?
    };
    let folder = spelled
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "corpus".into());
    let id = match &req.id {
        Some(id) => {
            let id = id.trim();
            if id.is_empty()
                || !id
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
            {
                bail!("corpus id {id:?} must be ASCII letters, digits, - or _");
            }
            if let Some(c) = reg.corpora.iter().find(|c| ids_clash(&c.id, id)) {
                bail!(
                    "corpus id `{id}` is already registered as `{}` (for {}); ids may not \
                     differ only by case; choose another --id",
                    c.id,
                    c.path
                );
            }
            id.to_string()
        }
        None => unique_id(&slug(&folder), reg),
    };
    // Never guessed from the folder's contents: an archive is asked for.
    let kind = req.kind.unwrap_or(CorpusKind::MarkdownFolder);
    if kind == CorpusKind::HighlightScoutArchive && !abs.join("readings").join("works").is_dir() {
        bail!(
            "{} has no readings/works/ folder; it is not a Highlight Scout archive",
            abs.display()
        );
    }
    let include = if !req.include.is_empty() {
        req.include.clone()
    } else {
        match kind {
            CorpusKind::MarkdownFolder => DEFAULT_INCLUDE.iter().map(|s| s.to_string()).collect(),
            // The adapter's own default: readings/works/*.md.
            CorpusKind::HighlightScoutArchive => vec![],
        }
    };
    crate::markdown::build_globs(&include)?;
    crate::markdown::build_globs(&req.exclude)?;
    let name = req
        .name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .unwrap_or(folder);
    Ok(CorpusConfig {
        id,
        name,
        kind,
        path: stored_path(&spelled),
        include,
        exclude: req.exclude.clone(),
        require_frontmatter: vec![],
        field_map: FieldMap::default(),
        default_author: req
            .author
            .as_deref()
            .map(str::trim)
            .filter(|a| !a.is_empty())
            .map(str::to_string),
        boilerplate: vec![],
        link: None,
        document_unit: DocumentUnit::File,
    })
}

/// Absolute and lexically clean (`.` and `..` resolved, no trailing `/`);
/// symlinks are kept as written. Only for paths that do not exist (error
/// messages, missing source folders): an existing path is canonicalised.
fn normalise(p: &Path) -> Result<PathBuf> {
    let abs = std::path::absolute(p).with_context(|| format!("resolve {}", p.display()))?;
    let mut out = PathBuf::new();
    for c in abs.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    Ok(out)
}

/// Refuse a (canonical) source folder that holds, or lies inside, the index
/// store: index files would be written inside a source.
fn refuse_index_store_overlap(abs: &Path, index_dir: &Path) -> Result<()> {
    let store = index::canonical_prefix(index_dir);
    if store.starts_with(abs) {
        bail!(
            "{} holds the index store {}; refusing to register it (scout would write inside \
             the source folder)",
            abs.display(),
            store.display()
        );
    }
    if abs.starts_with(&store) {
        bail!(
            "{} is inside the index store {}; refusing to register it",
            abs.display(),
            store.display()
        );
    }
    Ok(())
}

/// The precondition of every registry write (add, remove, init-defaults),
/// run before the lock is taken and before anything is deleted: the
/// registry file is not a symlink, and its location (canonical parent dir
/// plus file name) lies inside none of `sources` (canonical source
/// folders). Otherwise the registry, its temp files and its lock file would
/// be written inside a source folder.
pub(crate) fn check_registry_location(registry: &Path, sources: &[PathBuf]) -> Result<()> {
    refuse_symlink(registry)?;
    let loc = index::canonical_prefix(parent_dir(registry)).join(file_name(registry));
    for src in sources {
        if loc.starts_with(src) {
            bail!(
                "the corpus registry {} lies inside the source folder {}; refusing to write \
                 it there (move the registry, or point SCOUT_CONFIG elsewhere)",
                loc.display(),
                src.display()
            );
        }
    }
    Ok(())
}

/// The registered source folders, canonical where they exist, from a
/// lock-free read of the registry. A missing or unparseable registry gives
/// none (the locked read that follows reports a parse error).
pub(crate) fn registered_sources(registry: &Path) -> Vec<PathBuf> {
    std::fs::read_to_string(registry)
        .ok()
        .and_then(|t| Registry::parse(&t).ok())
        .map(|r| r.corpora.iter().map(source_root).collect())
        .unwrap_or_default()
}

/// A registered corpus's source folder, canonical when it exists, else
/// lexically clean.
fn source_root(c: &CorpusConfig) -> PathBuf {
    let root = c.root();
    root.canonicalize()
        .unwrap_or_else(|_| normalise(&root).unwrap_or(root))
}

/// Index files are named by id, and a case-insensitive filesystem (the
/// macOS default) treats `notes` and `NOTES` as one file: such ids clash.
fn ids_clash(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// The registry form of an absolute path: `~/…` when under the home
/// folder, as spelled or canonical.
fn stored_path(abs: &Path) -> String {
    if let Some(home) = dirs::home_dir() {
        let canon = home.canonicalize().unwrap_or_else(|_| home.clone());
        for h in [home, canon] {
            if abs == h {
                return "~".into();
            }
            if let Ok(rest) = abs.strip_prefix(&h) {
                return format!("~/{}", rest.to_string_lossy());
            }
        }
    }
    abs.to_string_lossy().into_owned()
}

/// A corpus id from a folder name: ASCII lowercase, accents dropped, runs
/// of anything else as one `-`.
fn slug(name: &str) -> String {
    use unicode_normalization::char::is_combining_mark;
    use unicode_normalization::UnicodeNormalization;
    let mut out = String::new();
    for ch in name.nfd().filter(|c| !is_combining_mark(*c)) {
        let c = ch.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "corpus".into()
    } else {
        out
    }
}

fn unique_id(base: &str, reg: &Registry) -> String {
    let taken = |id: &str| reg.corpora.iter().any(|c| ids_clash(&c.id, id));
    if !taken(base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|id| !taken(id))
        .unwrap_or_default()
}

/// The comment block at the top of a registry file (kept on regeneration).
fn header_of(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('#') {
            out.push_str(line);
            out.push('\n');
        } else if t.is_empty() {
            continue;
        } else {
            break;
        }
    }
    if out.is_empty() {
        REGISTRY_HEADER.to_string()
    } else {
        out
    }
}

/// `text` when it parses to exactly `want`, else `want` rendered under the
/// original file's header comments.
fn spliced_or_rendered(text: String, want: &Registry, old: &str) -> Result<String> {
    match Registry::parse(&text) {
        Ok(got) if &got == want => Ok(text),
        _ => want.render(&header_of(old)),
    }
}

/// The file text without the `pos`-th `[[corpus]]` table (its sub-tables
/// go with it). Returns the text unchanged when the tables cannot be told
/// apart line by line; the caller then regenerates.
fn cut_entry(text: &str, pos: usize, count: usize) -> String {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let starts: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.trim() == "[[corpus]]")
        .map(|(i, _)| i)
        .collect();
    if starts.len() != count || pos >= count {
        return text.to_string();
    }
    let from = starts[pos];
    let to = starts.get(pos + 1).copied().unwrap_or(lines.len());
    let mut out: String = lines[..from].concat();
    out.push_str(&lines[to..].concat());
    while out.ends_with("\n\n") {
        out.pop();
    }
    out
}

/// Hold an exclusive advisory lock on `<registry>.lock` (beside the
/// registry) until the returned file is dropped. Serialises every
/// read-modify-write of the registry across threads and processes. The lock
/// file is never written to (opened without truncation) and never deleted.
pub(crate) fn lock_registry(path: &Path) -> Result<std::fs::File> {
    let dir = parent_dir(path);
    std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    let lock = dir.join(format!(".{}.lock", file_name(path)));
    refuse_symlink(&lock)?;
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true).append(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // A symlink planted between the check and the open fails (ELOOP)
        // instead of being followed.
        opts.custom_flags(libc::O_NOFOLLOW);
    }
    let f = opts
        .open(&lock)
        .with_context(|| format!("open {} (it must not be a symlink)", lock.display()))?;
    f.lock()
        .with_context(|| format!("lock {}", lock.display()))?;
    Ok(f)
}

/// An error when `path` is a symlink (dangling or not).
fn refuse_symlink(path: &Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => {
            bail!(
                "{} is a symlink; refusing to write through it",
                path.display()
            )
        }
        _ => Ok(()),
    }
}

fn parent_dir(path: &Path) -> &Path {
    path.parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "corpora.toml".into())
}

/// An unpredictable temp-file name beside `path`: pid, time, a per-process
/// counter and a randomly seeded hash.
fn temp_name(path: &Path) -> String {
    use std::hash::{BuildHasher, Hasher};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(nanos);
    h.write_u64(n);
    format!(
        ".{}.{}.{n}.{:016x}.tmp",
        file_name(path),
        std::process::id(),
        h.finish()
    )
}

/// Write `text` to `path` atomically: a temp file in the same directory,
/// created exclusively (`O_EXCL`: an existing file or symlink of that name
/// is never opened, so nothing it points at is truncated), flushed, then
/// renamed over `path`. On any failure a temp file this call created is
/// removed and `path` is as it was. Callers refuse a symlinked registry
/// first ([`check_registry_location`]).
pub fn write_atomic(path: &Path, text: &str, rename: RenameFn) -> Result<()> {
    let dir = parent_dir(path);
    std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    let tmp = dir.join(temp_name(path));
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .with_context(|| format!("create {}", tmp.display()))?;
    let result = (|| -> Result<()> {
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        rename(&tmp, path).with_context(|| format!("replace {}", path.display()))?;
        Ok(())
    })();
    drop(f);
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Delete a corpus's word index and meaning vectors (with SQLite side
/// files) from `index_dir`; returns the files removed. Refuses, deleting
/// nothing, when the index dir lies inside any registered source folder
/// or another registered id differs from `id` only by case (the files are
/// shared). Otherwise every file is tried, and an error names the files
/// left behind.
fn delete_index_files(index_dir: &Path, id: &str, reg: &Registry) -> Result<Vec<String>> {
    let bases = [
        index::index_path_in(index_dir, id),
        index::vectors_path_in(index_dir, id),
    ];
    let mut files = Vec::new();
    for b in &bases {
        files.extend(index::sqlite_files(b).with_context(|| {
            format!("corpus `{id}` left registered (run the remove again to retry)")
        })?);
    }
    if files.is_empty() {
        return Ok(vec![]);
    }
    if let Some(other) = reg
        .corpora
        .iter()
        .find(|c| c.id != id && ids_clash(&c.id, id))
    {
        bail!(
            "corpus `{id}` left registered: its index files are shared with `{}` (ids differ \
             only by case); remove it with --keep-index",
            other.id
        );
    }
    let store = index::canonical_prefix(index_dir);
    for c in &reg.corpora {
        let root = source_root(c);
        if store.starts_with(&root) {
            bail!(
                "corpus `{id}` left registered: its index files in {} are inside the source \
                 folder {} of `{}`; refusing to delete them (use --keep-index to unregister only)",
                store.display(),
                root.display(),
                c.id
            );
        }
    }
    let mut deleted = Vec::new();
    let mut errors = Vec::new();
    for b in &bases {
        match index::remove_index_files(b) {
            Ok(gone) => deleted.extend(gone.into_iter().map(|p| p.display().to_string())),
            Err(e) => errors.push(e.to_string()),
        }
    }
    if !errors.is_empty() {
        bail!(
            "corpus `{id}` left registered (run the remove again to retry): {}",
            errors.join("; ")
        );
    }
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;

    #[test]
    fn slugs_fold_accents_and_punctuation() {
        assert_eq!(slug("Poznámky k četbě"), "poznamky-k-cetbe");
        assert_eq!(slug("My Notes!"), "my-notes");
        assert_eq!(slug("…"), "corpus");
    }

    #[test]
    fn header_keeps_leading_comments_only() {
        assert_eq!(header_of("# a\n# b\n\n[[corpus]]\n# c\n"), "# a\n# b\n");
        assert_eq!(header_of("[[corpus]]\n"), REGISTRY_HEADER);
    }

    #[test]
    fn errors_name_the_registry() {
        let e = anyhow!(RegistryMissing(PathBuf::from("/x/corpora.toml")));
        assert_eq!(
            e.to_string(),
            "no corpus registry at /x/corpora.toml; add a folder with `scout corpora add <folder>`"
        );
    }
}

//! Corpus registration: `corpora add` and `corpora remove` as a
//! read-modify-write of the registry file, plus kind detection.
//!
//! - The only files written are the registry (atomically: a temp file in
//!   the same directory, then a rename) and, on remove, the corpus's own
//!   index files. A corpus's source folder is never written or deleted.
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
    /// Default: detected (a Highlight Scout archive has `readings/works/`),
    /// else markdown-folder.
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
        let existing = self.read()?;
        let reg = existing
            .as_ref()
            .map(|(_, r)| r.clone())
            .unwrap_or_default();
        let cfg = new_config(req, &reg)?;
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

    /// Unregister a corpus and, unless `keep_index`, delete its index files.
    /// The source folder is never touched.
    pub fn remove(&self, id: &str, keep_index: bool) -> Result<Removed> {
        let Some((old, reg)) = self.read()? else {
            return Err(RegistryMissing(self.path.clone()).into());
        };
        let cfg = reg.get(id)?.clone();
        let pos = reg.corpora.iter().position(|c| c.id == id).unwrap_or(0);
        let mut want = reg.clone();
        want.corpora.remove(pos);
        let text = spliced_or_rendered(cut_entry(&old, pos, reg.corpora.len()), &want, &old)?;
        write_atomic(&self.path, &text, self.rename)?;
        let deleted = if keep_index {
            vec![]
        } else {
            delete_index_files(&self.index_dir, id)?
        };
        Ok(Removed {
            schema_version: SCHEMA_VERSION,
            id: cfg.id,
            path: cfg.path,
            index_kept: keep_index,
            deleted,
        })
    }
}

/// A folder's corpus kind: a Highlight Scout archive has `readings/works/`.
pub fn detect_kind(dir: &Path) -> CorpusKind {
    if dir.join("readings").join("works").is_dir() {
        CorpusKind::HighlightScoutArchive
    } else {
        CorpusKind::MarkdownFolder
    }
}

/// Validate a request against the registry and build the new entry.
fn new_config(req: &AddCorpus, reg: &Registry) -> Result<CorpusConfig> {
    if req.path.trim().is_empty() {
        bail!("no folder given");
    }
    let abs = normalise(&expand_home(req.path.trim()))?;
    if !abs.exists() {
        bail!("no folder at {}", abs.display());
    }
    if !abs.is_dir() {
        bail!("{} is a file, not a folder", abs.display());
    }
    let canon = abs.canonicalize().unwrap_or_else(|_| abs.clone());
    for c in &reg.corpora {
        let root = c.root();
        let theirs = root
            .canonicalize()
            .unwrap_or_else(|_| normalise(&root).unwrap_or(root));
        if theirs == canon {
            bail!("path {} already registered as `{}`", abs.display(), c.id);
        }
    }
    let folder = abs
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
            if let Some(c) = reg.corpora.iter().find(|c| c.id == id) {
                bail!(
                    "corpus id `{id}` is already registered (for {}); choose another --id",
                    c.path
                );
            }
            id.to_string()
        }
        None => unique_id(&slug(&folder), reg),
    };
    let kind = req.kind.unwrap_or_else(|| detect_kind(&abs));
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
        path: stored_path(&abs),
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
/// symlinks are kept as written.
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

/// The registry form of an absolute path: `~/…` under the home folder.
fn stored_path(abs: &Path) -> String {
    if let Some(home) = dirs::home_dir() {
        if abs == home {
            return "~".into();
        }
        if let Ok(rest) = abs.strip_prefix(&home) {
            return format!("~/{}", rest.to_string_lossy());
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
    let taken = |id: &str| reg.corpora.iter().any(|c| c.id == id);
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

/// Write `text` to `path` atomically: a temp file in the same directory,
/// flushed, then renamed over `path`. On any failure the temp file is
/// removed and `path` is as it was.
pub fn write_atomic(path: &Path, text: &str, rename: RenameFn) -> Result<()> {
    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "corpora.toml".into());
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let result = (|| -> Result<()> {
        let mut f =
            std::fs::File::create(&tmp).with_context(|| format!("create {}", tmp.display()))?;
        f.write_all(text.as_bytes())?;
        f.sync_all()?;
        drop(f);
        rename(&tmp, path).with_context(|| format!("replace {}", path.display()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Delete a corpus's word index and meaning vectors (with SQLite side
/// files) from `index_dir`; returns the files removed.
fn delete_index_files(index_dir: &Path, id: &str) -> Result<Vec<String>> {
    let mut deleted = Vec::new();
    for base in [
        index::index_path_in(index_dir, id),
        index::vectors_path_in(index_dir, id),
    ] {
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let p = PathBuf::from(format!("{}{suffix}", base.display()));
            if p.is_file() {
                std::fs::remove_file(&p).with_context(|| format!("remove {}", p.display()))?;
                deleted.push(p.display().to_string());
            }
        }
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

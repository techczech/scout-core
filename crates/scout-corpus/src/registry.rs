//! Corpus registry: `~/.config/scout/corpora.toml`, one `[[corpus]]` per corpus.
//!
//! Paths may start with `~/`; they are expanded on use. `SCOUT_CONFIG`
//! overrides the registry file path, and `XDG_CONFIG_HOME` is honoured.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CorpusKind {
    MarkdownFolder,
    HighlightScoutArchive,
}

impl CorpusKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            CorpusKind::MarkdownFolder => "markdown-folder",
            CorpusKind::HighlightScoutArchive => "highlight-scout-archive",
        }
    }
}

/// What one document is inside a markdown-folder corpus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DocumentUnit {
    /// One file = one document.
    #[default]
    File,
    /// One tweet = one document: files hold `## <timestamp> — tweet <id>`
    /// sections with the text in a `~~~` fence (the Twitter-archive import
    /// of the writing repo). Each tweet keeps its own date and public URL.
    Tweet,
}

impl DocumentUnit {
    fn is_file(&self) -> bool {
        *self == DocumentUnit::File
    }
}

/// How a corpus's citations read (derived from kind and document unit).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CiteStyle {
    /// `— Author, *Title*, 23 June 2016 · [archive](…) · [public](…)`
    #[default]
    Writing,
    /// `— Author (@handle), tweet, 26 July 2025 · [public](…)`
    Tweet,
    /// `— Author, *Title*, 2019 · [highlight](…)`
    Highlight,
}

fn d_title() -> String {
    "title".into()
}
fn d_date() -> String {
    "date".into()
}
fn d_genre() -> String {
    "genre".into()
}
fn d_topics() -> String {
    "topics".into()
}
fn d_lang() -> String {
    "lang".into()
}
fn d_summary() -> String {
    "summary".into()
}
fn d_author() -> String {
    "author".into()
}
fn d_public_url() -> Vec<String> {
    vec!["published_url".into(), "canonical_url".into()]
}

/// A frontmatter key list that also accepts a single string in TOML.
fn one_or_many<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }
    Ok(match OneOrMany::deserialize(d)? {
        OneOrMany::One(s) => vec![s],
        OneOrMany::Many(v) => v,
    })
}

/// Maps document metadata to frontmatter keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldMap {
    #[serde(default = "d_title")]
    pub title: String,
    #[serde(default = "d_date")]
    pub date: String,
    #[serde(default = "d_genre")]
    pub genre: String,
    #[serde(default = "d_topics")]
    pub topics: String,
    #[serde(default = "d_lang")]
    pub lang: String,
    #[serde(default = "d_summary")]
    pub summary: String,
    #[serde(default = "d_author")]
    pub author: String,
    /// Keys holding the document's public URL, tried in order; the first
    /// `http(s)://` value wins. A single string is accepted.
    #[serde(default = "d_public_url", deserialize_with = "one_or_many")]
    pub public_url: Vec<String>,
}

impl Default for FieldMap {
    fn default() -> Self {
        FieldMap {
            title: d_title(),
            date: d_date(),
            genre: d_genre(),
            topics: d_topics(),
            lang: d_lang(),
            summary: d_summary(),
            author: d_author(),
            public_url: d_public_url(),
        }
    }
}

/// The writing repo holds its public URL in `published_url` (most imports)
/// or `source_url` (the Promising Paragraphs Substack imports).
fn writing_public_url() -> Vec<String> {
    vec![
        "published_url".into(),
        "source_url".into(),
        "canonical_url".into(),
    ]
}

pub const WRITEFLEX_LINK: &str = "writeflex://open?path={path}&line={line}";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorpusConfig {
    pub id: String,
    pub name: String,
    pub kind: CorpusKind,
    pub path: String,
    #[serde(default)]
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
    #[serde(default)]
    pub require_frontmatter: Vec<String>,
    #[serde(default)]
    pub field_map: FieldMap,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_author: Option<String>,
    #[serde(default)]
    pub boilerplate: Vec<String>,
    /// Link template; `{path}` is the URL-encoded absolute file path and
    /// `{line}` the 1-based source line. A highlight-scout-archive corpus
    /// without one links to the work file (`file://…`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    /// markdown-folder only: `file` (default) or `tweet`.
    #[serde(default, skip_serializing_if = "DocumentUnit::is_file")]
    pub document_unit: DocumentUnit,
}

impl CorpusConfig {
    /// The corpus root with `~/` expanded.
    pub fn root(&self) -> PathBuf {
        expand_home(&self.path)
    }

    /// The source file of a document key: keys are the file's relative path,
    /// plus `#<fragment>` when one file holds several documents (tweets).
    pub fn source_rel<'a>(&self, doc_key: &'a str) -> &'a str {
        doc_key.split_once('#').map(|(f, _)| f).unwrap_or(doc_key)
    }

    /// The absolute source file of a document key.
    pub fn source_path(&self, doc_key: &str) -> PathBuf {
        self.root().join(self.source_rel(doc_key))
    }

    /// The archive link for a line of a document (the `link` template, or
    /// the work file's `file://` URL for a highlights archive).
    pub fn link_for(&self, doc_key: &str, line: usize) -> Option<String> {
        let path = self.source_path(doc_key);
        match (&self.link, self.kind) {
            (Some(t), _) => Some(crate::cite::render_link(t, &path, line)),
            (None, CorpusKind::HighlightScoutArchive) => Some(crate::cite::file_url(&path)),
            (None, _) => None,
        }
    }

    pub fn cite_style(&self) -> CiteStyle {
        match (self.kind, self.document_unit) {
            (CorpusKind::HighlightScoutArchive, _) => CiteStyle::Highlight,
            (_, DocumentUnit::Tweet) => CiteStyle::Tweet,
            _ => CiteStyle::Writing,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registry {
    #[serde(default, rename = "corpus")]
    pub corpora: Vec<CorpusConfig>,
}

/// Raised when the registry file does not exist.
#[derive(Debug)]
pub struct RegistryMissing(pub PathBuf);

impl std::fmt::Display for RegistryMissing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "no corpus registry at {}; add a folder with `scout corpora add <folder>`",
            self.0.display()
        )
    }
}
impl std::error::Error for RegistryMissing {}

pub fn expand_home(p: &str) -> PathBuf {
    if p == "~" {
        return dirs::home_dir().unwrap_or_default();
    }
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(h) = dirs::home_dir() {
            return h.join(rest);
        }
    }
    PathBuf::from(p)
}

/// The registry file path.
pub fn registry_path() -> PathBuf {
    if let Some(p) = std::env::var_os("SCOUT_CONFIG") {
        return PathBuf::from(p);
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".config"));
    base.join("scout").join("corpora.toml")
}

impl Registry {
    pub fn parse(text: &str) -> Result<Registry> {
        let reg: Registry = toml::from_str(text).context("parse corpus registry")?;
        let mut seen = std::collections::BTreeSet::new();
        for c in &reg.corpora {
            if !seen.insert(c.id.clone()) {
                return Err(anyhow!("duplicate corpus id {:?} in registry", c.id));
            }
            if c.id.is_empty()
                || !c
                    .id
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
            {
                return Err(anyhow!(
                    "corpus id {:?} must be ASCII letters, digits, - or _",
                    c.id
                ));
            }
        }
        Ok(reg)
    }

    pub fn load_from(path: &Path) -> Result<Registry> {
        if !path.exists() {
            return Err(RegistryMissing(path.to_path_buf()).into());
        }
        let text =
            std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
        Registry::parse(&text)
    }

    pub fn load() -> Result<Registry> {
        Registry::load_from(&registry_path())
    }

    pub fn get(&self, id: &str) -> Result<&CorpusConfig> {
        self.corpora.iter().find(|c| c.id == id).ok_or_else(|| {
            let ids: Vec<&str> = self.corpora.iter().map(|c| c.id.as_str()).collect();
            anyhow!("unknown corpus {id:?}; registered: {}", ids.join(", "))
        })
    }

    pub fn to_toml(&self) -> Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }

    /// Render the registry file: `header` (comment lines), then the
    /// entries. An empty registry is the header alone, so a later append of
    /// a `[[corpus]]` table stays valid TOML.
    pub fn render(&self, header: &str) -> Result<String> {
        let mut out = String::from(header);
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        if !self.corpora.is_empty() {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&self.to_toml()?);
        }
        Ok(out)
    }
}

/// Read only `archive_path` from `~/.config/highlight-scout/config.toml`.
/// Nothing else from that file is kept or printed.
pub fn highlight_scout_archive_path() -> Option<String> {
    let p = dirs::home_dir()?.join(".config/highlight-scout/config.toml");
    let text = std::fs::read_to_string(p).ok()?;
    let v: toml::Value = toml::from_str(&text).ok()?;
    v.get("archive_path")?.as_str().map(|s| s.to_string())
}

/// The header of a registry written by `init-defaults` or by the first
/// `corpora add`.
pub const REGISTRY_HEADER: &str = "# Scout corpus registry. One [[corpus]] per corpus.\n\
# Add a folder with `scout corpora add <folder>`, then build its index with\n\
# `scout index build <id>`. `scout corpora remove <id>` takes one out again.\n";

/// What `init-defaults` writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Preset {
    /// An empty registry whose header explains `scout corpora add`.
    #[default]
    Empty,
    /// The maintainer's own corpora: the `--preset dominik` block below.
    Maintainer,
}

impl Preset {
    fn registry(self) -> Registry {
        match self {
            Preset::Empty => Registry::default(),
            Preset::Maintainer => maintainer_preset(),
        }
    }

    fn header(self) -> &'static str {
        match self {
            Preset::Empty => REGISTRY_HEADER,
            Preset::Maintainer => "# Scout corpus registry. One [[corpus]] per corpus.\n# Written by `scout corpora init-defaults`.\n",
        }
    }
}

// ---- `--preset dominik` -------------------------------------------------
// The maintainer's own three corpora, written only by
// `scout corpora init-defaults --preset dominik` (a hidden flag, deliberately
// absent from `--help` and from every error message). `highlights` is
// included only when the Highlight Scout config names an archive path.
fn maintainer_preset() -> Registry {
    let writing = "~/gitrepos/02_writing-creation/writing";
    let mut corpora = vec![
        CorpusConfig {
            id: "writing".into(),
            name: "Writing".into(),
            kind: CorpusKind::MarkdownFolder,
            path: writing.into(),
            include: vec!["**/*.md".into()],
            exclude: vec![
                "cache/**".into(),
                "_sources/**".into(),
                "indexes/**".into(),
                "docs/**".into(),
                "tweets/**".into(),
            ],
            require_frontmatter: vec!["genre".into()],
            field_map: FieldMap {
                author: "authors".into(),
                public_url: writing_public_url(),
                ..FieldMap::default()
            },
            default_author: Some("Dominik Lukeš".into()),
            boilerplate: vec![],
            link: Some(WRITEFLEX_LINK.into()),
            document_unit: DocumentUnit::File,
        },
        CorpusConfig {
            id: "tweets".into(),
            name: "Tweets".into(),
            kind: CorpusKind::MarkdownFolder,
            path: format!("{writing}/tweets"),
            include: vec!["**/*.md".into()],
            exclude: vec![],
            require_frontmatter: vec![],
            field_map: FieldMap {
                author: "authors".into(),
                public_url: writing_public_url(),
                ..FieldMap::default()
            },
            default_author: Some("Dominik Lukeš".into()),
            boilerplate: vec![],
            link: Some(WRITEFLEX_LINK.into()),
            document_unit: DocumentUnit::Tweet,
        },
    ];
    if let Some(archive) = highlight_scout_archive_path() {
        corpora.push(CorpusConfig {
            id: "highlights".into(),
            name: "Highlights".into(),
            kind: CorpusKind::HighlightScoutArchive,
            path: archive,
            include: vec![],
            exclude: vec![],
            require_frontmatter: vec![],
            field_map: FieldMap::default(),
            default_author: None,
            boilerplate: vec![],
            link: None,
            document_unit: DocumentUnit::File,
        });
    }
    Registry { corpora }
}
// ---- end `--preset dominik` ---------------------------------------------

/// Write a starter registry to `path`: empty by default, or a preset.
/// Refuses to overwrite unless `force`; returns false (and writes nothing)
/// when the file exists and `force` is off. The write is atomic and holds
/// the registry lock (see [`crate::register`]).
pub fn init_defaults(path: &Path, force: bool, preset: Preset) -> Result<bool> {
    crate::register::check_registry_location(path, &crate::register::registered_sources(path))?;
    let _lock = crate::register::lock_registry(path)?;
    if path.exists() && !force {
        return Ok(false);
    }
    let body = preset.registry().render(preset.header())?;
    crate::register::write_atomic(path, &body, |a, b| std::fs::rename(a, b))?;
    Ok(true)
}

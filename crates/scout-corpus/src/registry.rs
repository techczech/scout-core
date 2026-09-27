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
        }
    }
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
    /// `{line}` the 1-based source line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
}

impl CorpusConfig {
    /// The corpus root with `~/` expanded.
    pub fn root(&self) -> PathBuf {
        expand_home(&self.path)
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
            "no corpus registry at {}; run `scout corpora init-defaults`",
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

    /// The three default corpora. `highlights` is included only when the
    /// Highlight Scout config names an archive path.
    pub fn defaults() -> Registry {
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
                    ..FieldMap::default()
                },
                default_author: Some("Dominik Lukeš".into()),
                boilerplate: vec![],
                link: Some(WRITEFLEX_LINK.into()),
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
                    ..FieldMap::default()
                },
                default_author: Some("Dominik Lukeš".into()),
                boilerplate: vec![],
                link: Some(WRITEFLEX_LINK.into()),
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
            });
        }
        Registry { corpora }
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

/// Write the default registry to `path`. Refuses to overwrite unless `force`.
/// Returns false (and writes nothing) when the file exists and `force` is off.
pub fn init_defaults(path: &Path, force: bool) -> Result<bool> {
    if path.exists() && !force {
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let body = format!(
        "# Scout corpus registry. One [[corpus]] per corpus.\n# Written by `scout corpora init-defaults`.\n\n{}",
        Registry::defaults().to_toml()?
    );
    std::fs::write(path, body)?;
    Ok(true)
}

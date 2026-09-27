//! Markdown-folder adapter: file discovery, frontmatter, document metadata and
//! passage splitting. Read-only over the source folder (invariant 1).

use crate::registry::CorpusConfig;
use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One candidate source file.
#[derive(Debug, Clone)]
pub struct SourceFile {
    /// Path relative to the corpus root, `/`-separated.
    pub rel_path: String,
    pub abs_path: PathBuf,
    pub mtime_ns: i64,
    pub size: i64,
}

fn build_globs(patterns: &[String]) -> Result<GlobSet> {
    let mut b = GlobSetBuilder::new();
    for p in patterns {
        b.add(Glob::new(p).with_context(|| format!("bad glob {p:?}"))?);
    }
    Ok(b.build()?)
}

/// List candidate files under the corpus root, sorted by relative path.
/// Hidden path components (`.git`, `.worktrees`, ...) are always skipped.
pub fn discover(cfg: &CorpusConfig) -> Result<Vec<SourceFile>> {
    let root = cfg.root();
    if !root.is_dir() {
        anyhow::bail!(
            "corpus {:?}: path {} is not a directory",
            cfg.id,
            root.display()
        );
    }
    let include = if cfg.include.is_empty() {
        build_globs(&["**/*.md".to_string()])?
    } else {
        build_globs(&cfg.include)?
    };
    let exclude = build_globs(&cfg.exclude)?;
    let mut out = Vec::new();
    let walker = walkdir::WalkDir::new(&root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || !e.file_name().to_string_lossy().starts_with('.'));
    for entry in walker {
        let entry = entry?;
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry.path().strip_prefix(&root).unwrap_or(entry.path());
        let rel_path = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        if !include.is_match(&rel_path) || exclude.is_match(&rel_path) {
            continue;
        }
        let md = entry.metadata()?;
        let mtime_ns = md
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0);
        out.push(SourceFile {
            rel_path,
            abs_path: entry.path().to_path_buf(),
            mtime_ns,
            size: md.len() as i64,
        });
    }
    out.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    Ok(out)
}

/// A frontmatter value: a scalar or a list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FmValue {
    Scalar(String),
    List(Vec<String>),
}

impl FmValue {
    pub fn is_empty(&self) -> bool {
        match self {
            FmValue::Scalar(s) => s.trim().is_empty(),
            FmValue::List(l) => l.is_empty(),
        }
    }
    pub fn as_scalar(&self) -> Option<String> {
        match self {
            FmValue::Scalar(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
            FmValue::List(l) if !l.is_empty() => Some(l.join(", ")),
            _ => None,
        }
    }
    /// A list; scalars are split on commas.
    pub fn as_list(&self) -> Vec<String> {
        match self {
            FmValue::Scalar(s) => s
                .split(',')
                .map(|x| x.trim().to_string())
                .filter(|x| !x.is_empty())
                .collect(),
            FmValue::List(l) => l.clone(),
        }
    }
}

fn unquote(s: &str) -> String {
    let t = s.trim();
    if t.len() >= 2
        && ((t.starts_with('"') && t.ends_with('"')) || (t.starts_with('\'') && t.ends_with('\'')))
    {
        let inner = &t[1..t.len() - 1];
        if t.starts_with('"') {
            return inner.replace("\\\"", "\"").replace("\\\\", "\\");
        }
        return inner.replace("''", "'");
    }
    t.to_string()
}

fn parse_inline_list(s: &str) -> Option<Vec<String>> {
    let t = s.trim();
    let inner = t.strip_prefix('[')?.strip_suffix(']')?;
    let mut items = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    for c in inner.chars() {
        match quote {
            Some(q) if c == q => {
                quote = None;
                cur.push(c);
            }
            Some(_) => cur.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                cur.push(c);
            }
            None if c == ',' => {
                items.push(unquote(&cur));
                cur.clear();
            }
            None => cur.push(c),
        }
    }
    if !cur.trim().is_empty() {
        items.push(unquote(&cur));
    }
    Some(items.into_iter().filter(|x| !x.is_empty()).collect())
}

/// Parsed file: frontmatter keys and the body with its first line number.
#[derive(Debug, Clone)]
pub struct ParsedFile {
    pub frontmatter: Option<BTreeMap<String, FmValue>>,
    /// Body lines (without newline), paired with 1-based source line numbers.
    pub body_start_line: usize,
    pub body: Vec<String>,
}

/// Split frontmatter from body. Frontmatter is a flat `key: value` block
/// between `---` lines at the top of the file; block lists (`- item`) and
/// inline lists (`[a, b]`) are understood. Parsing is deliberately lenient:
/// unquoted colons in values are kept as text.
pub fn parse_file(text: &str) -> ParsedFile {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let lines: Vec<&str> = text
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect();
    let mut fm = None;
    let mut body_from = 0;
    if lines
        .first()
        .map(|l| l.trim_end() == "---")
        .unwrap_or(false)
    {
        if let Some(close) = lines.iter().skip(1).position(|l| {
            let t = l.trim_end();
            t == "---" || t == "..."
        }) {
            let close = close + 1;
            let mut map: BTreeMap<String, FmValue> = BTreeMap::new();
            let mut last_key: Option<String> = None;
            for l in &lines[1..close] {
                if l.trim().is_empty() || l.trim_start().starts_with('#') {
                    continue;
                }
                let indented = l.starts_with(' ') || l.starts_with('\t');
                let t = l.trim();
                let item = if t == "-" {
                    Some("")
                } else {
                    t.strip_prefix("- ")
                };
                if let (Some(k), Some(item)) = (last_key.as_ref(), item) {
                    let v = unquote(item);
                    let e = map
                        .entry(k.clone())
                        .or_insert_with(|| FmValue::List(vec![]));
                    if let FmValue::Scalar(sc) = e {
                        if sc.trim().is_empty() {
                            *e = FmValue::List(vec![]);
                        }
                    }
                    if let FmValue::List(list) = e {
                        if !v.is_empty() {
                            list.push(v);
                        }
                    }
                    continue;
                }
                if indented {
                    continue; // nested mapping: ignored
                }
                if let Some(idx) = t.find(':') {
                    let key = t[..idx].trim().to_string();
                    if key.is_empty() || key.contains(' ') {
                        continue;
                    }
                    let raw = t[idx + 1..].trim();
                    let val = match parse_inline_list(raw) {
                        Some(list) => FmValue::List(list),
                        None => FmValue::Scalar(unquote(raw)),
                    };
                    map.insert(key.clone(), val);
                    last_key = Some(key);
                }
            }
            fm = Some(map);
            body_from = close + 1;
        }
    }
    let body = lines[body_from.min(lines.len())..]
        .iter()
        .map(|s| s.to_string())
        .collect();
    ParsedFile {
        frontmatter: fm,
        body_start_line: body_from + 1,
        body,
    }
}

/// Document metadata resolved through the field map.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DocMeta {
    pub title: Option<String>,
    pub date: Option<String>,
    pub genre: Option<String>,
    pub topics: Vec<String>,
    pub lang: Option<String>,
    pub summary: Option<String>,
    pub author: Option<String>,
    /// The first `http(s)://` value among `field_map.public_url` keys.
    pub public_url: Option<String>,
}

/// Does the file count as a document under `require_frontmatter`?
pub fn is_document(cfg: &CorpusConfig, parsed: &ParsedFile) -> bool {
    if cfg.require_frontmatter.is_empty() {
        return true;
    }
    match &parsed.frontmatter {
        None => false,
        Some(fm) => cfg
            .require_frontmatter
            .iter()
            .all(|k| fm.get(k).map(|v| !v.is_empty()).unwrap_or(false)),
    }
}

pub fn doc_meta(cfg: &CorpusConfig, parsed: &ParsedFile) -> DocMeta {
    let empty = BTreeMap::new();
    let fm = parsed.frontmatter.as_ref().unwrap_or(&empty);
    let get = |k: &str| fm.get(k).and_then(|v| v.as_scalar());
    let fmap = &cfg.field_map;
    let title = get(&fmap.title).or_else(|| {
        parsed.body.iter().find_map(|l| {
            let t = l.trim_start();
            let h = t.trim_start_matches('#');
            if t.starts_with('#') && h.starts_with(' ') && t.len() - h.len() <= 6 {
                Some(h.trim().to_string()).filter(|s| !s.is_empty())
            } else {
                None
            }
        })
    });
    DocMeta {
        title,
        date: get(&fmap.date),
        genre: get(&fmap.genre),
        topics: fm
            .get(&fmap.topics)
            .map(|v| v.as_list())
            .unwrap_or_default(),
        lang: get(&fmap.lang),
        summary: get(&fmap.summary),
        author: get(&fmap.author).or_else(|| cfg.default_author.clone()),
        public_url: fmap.public_url.iter().find_map(|k| {
            get(k)
                .map(|v| v.trim().to_string())
                .filter(|v| v.starts_with("https://") || v.starts_with("http://"))
        }),
    }
}

/// A raw passage: original lines and their 1-based line range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawPassage {
    pub line_start: usize,
    pub line_end: usize,
    /// The original lines joined with `\n`, exactly as in the file.
    pub original: String,
}

fn is_heading(l: &str) -> bool {
    let t = l.trim_start();
    let hashes = t.len() - t.trim_start_matches('#').len();
    (1..=6).contains(&hashes) && (t.len() == hashes || t[hashes..].starts_with(' '))
}

fn is_rule_or_underline(l: &str) -> bool {
    let t: String = l.chars().filter(|c| !c.is_whitespace()).collect();
    t.len() >= 2
        && (t.chars().all(|c| c == '=')
            || t.chars().all(|c| c == '-')
            || (t.len() >= 3 && t.chars().all(|c| c == '*' || c == '_')))
}

fn is_list_item(l: &str) -> bool {
    let t = l.trim_start();
    if let Some(r) = t
        .strip_prefix("- ")
        .or(t.strip_prefix("* "))
        .or(t.strip_prefix("+ "))
    {
        let _ = r;
        return true;
    }
    let digits = t.len() - t.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits > 0 && digits <= 9 {
        let rest = &t[digits..];
        return rest.starts_with(". ") || rest.starts_with(") ");
    }
    false
}

/// Split a body into passages: blank-line-separated blocks, where a heading
/// or a list item is its own block.
pub fn split_passages(parsed: &ParsedFile) -> Vec<RawPassage> {
    let mut out = Vec::new();
    let mut cur: Vec<(usize, &str)> = Vec::new();
    let flush = |cur: &mut Vec<(usize, &str)>, out: &mut Vec<RawPassage>| {
        if cur.is_empty() {
            return;
        }
        let line_start = cur[0].0;
        let line_end = cur[cur.len() - 1].0;
        let original = cur.iter().map(|(_, l)| *l).collect::<Vec<_>>().join("\n");
        out.push(RawPassage {
            line_start,
            line_end,
            original,
        });
        cur.clear();
    };
    for (i, line) in parsed.body.iter().enumerate() {
        let n = parsed.body_start_line + i;
        if line.trim().is_empty() {
            flush(&mut cur, &mut out);
            continue;
        }
        if is_rule_or_underline(line) {
            // Setext underline or thematic break: ends the block, carries no text.
            flush(&mut cur, &mut out);
            continue;
        }
        if is_heading(line) {
            flush(&mut cur, &mut out);
            cur.push((n, line));
            flush(&mut cur, &mut out);
            continue;
        }
        if is_list_item(line) {
            flush(&mut cur, &mut out);
        }
        cur.push((n, line));
    }
    flush(&mut cur, &mut out);
    out
}

/// Read a source file (read-only).
pub fn read_source(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

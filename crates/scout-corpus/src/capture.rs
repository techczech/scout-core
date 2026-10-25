//! Research capture: source writes are explicit here; indexing remains derived.
use crate::api::{Engine, IndexBuildQuery, Reply};
use crate::event_seed::render::quoted;
use crate::registry::{expand_home, CorpusKind, Registry};
use anyhow::{bail, Context, Result};
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use yaml_rust2::{Yaml, YamlEmitter, YamlLoader};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptureQuery {
    pub project: String,
    /// Optional destination relative to research/.
    pub into: Option<String>,
    pub kind: String,
    pub title: String,
    pub prompt: Option<String>,
    pub sources: Vec<String>,
    pub artefact: Option<String>,
    pub slug: Option<String>,
    /// Empty means `verb`.
    pub by: String,
    /// Markdown path, or `-` for stdin. None uses `body` (app callers).
    pub input: Option<String>,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CaptureReceipt {
    pub file: String,
    pub kind: String,
    pub title: String,
    pub created: String,
    pub by: String,
    pub sources: Vec<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CaptureResult {
    pub schema_version: u32,
    pub file: String,
    pub manifest: bool,
    pub corpus: Option<String>,
    pub indexed: bool,
    /// Passages belonging to the new stub after the build.
    pub passages: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CaptureUsage {
    pub file: String,
    pub line: usize,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CaptureList {
    pub schema_version: u32,
    pub project: String,
    pub entries: Vec<CaptureReceipt>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CaptureUsages {
    pub schema_version: u32,
    pub file: String,
    pub hits: Vec<CaptureUsage>,
    pub skipped: usize,
}

fn project_dir(project: &str) -> Result<PathBuf> {
    let path = fs::canonicalize(expand_home(project))
        .with_context(|| format!("project {project:?} must exist and be a directory"))?;
    if !path.is_dir() {
        bail!("project {project:?} must be a directory");
    }
    Ok(path)
}

fn safe_research(project: &Path, create: bool) -> Result<PathBuf> {
    let path = project.join("research");
    if let Ok(m) = fs::symlink_metadata(&path) {
        if m.file_type().is_symlink() || !m.is_dir() {
            bail!("{} must be a real directory", path.display());
        }
    } else if create {
        match fs::create_dir(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
        return safe_research(project, false);
    }
    Ok(path)
}

// Validate before creating any directory. All components remain below research/;
// refusing symlinks also refuses dangling links and aliases within the tree.
fn research_relative(raw: &str) -> Result<&Path> {
    let path = Path::new(raw);
    if raw.is_empty()
        || path
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        bail!("research path must be relative, without . or .. components");
    }
    Ok(path)
}

fn research_path(root: &Path, relative: &Path, create_dirs: bool) -> Result<PathBuf> {
    let mut path = root.to_path_buf();
    for component in relative.components() {
        path.push(component);
        match fs::symlink_metadata(&path) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    bail!(
                        "research path must not contain symlinks: {}",
                        path.display()
                    );
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if create_dirs {
                    fs::create_dir(&path)?;
                }
            }
            Err(e) => return Err(e.into()),
        }
        if create_dirs && !path.is_dir() {
            bail!("capture destination must be a directory");
        }
    }
    Ok(path)
}

fn path_text(path: &Path) -> Result<String> {
    Ok(path
        .to_str()
        .context("capture paths must be valid UTF-8")?
        .to_owned())
}

fn relative_or_absolute(path: &Path, project: &Path) -> Result<String> {
    path_text(path.strip_prefix(project).unwrap_or(path))
}

// Provenance is recorded, never resolved against the filesystem.
fn source_path(raw: &str, project: &Path) -> Result<PathBuf> {
    if raw.trim().is_empty() {
        bail!("file source must not be empty");
    }
    let path = expand_home(raw);
    let path = if path.is_absolute() {
        path
    } else {
        project.join(path)
    };
    let mut normal = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                normal.pop();
            }
            std::path::Component::CurDir => {}
            other => normal.push(other.as_os_str()),
        }
    }
    Ok(normal)
}

fn artefact_path(raw: &str, project: &Path) -> Result<PathBuf> {
    if raw.trim().is_empty() {
        bail!("artefact must be a file");
    }
    let path = expand_home(raw);
    let path = if path.is_absolute() || path.exists() {
        path
    } else {
        project.join(path)
    };
    let path = fs::canonicalize(path).context("artefact must resolve to an existing file")?;
    if !path.is_file() {
        bail!("artefact must be a file");
    }
    Ok(path)
}

fn slug(title: &str) -> String {
    use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};
    let mut out = String::new();
    for c in title.nfd().filter(|c| !is_combining_mark(*c)) {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let mut s = out.trim_end_matches('-');
    if s.len() > 60 {
        let end = if s.as_bytes()[60] == b'-' {
            60
        } else {
            s[..60].rfind('-').unwrap_or(60)
        };
        s = &s[..end];
    }
    if s.is_empty() {
        "capture".into()
    } else {
        s.into()
    }
}

fn frontmatter(text: &str) -> Result<(Yaml, &str)> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.split_inclusive('\n');
    if lines.next().map(str::trim_end) != Some("---") {
        return Ok((Yaml::Hash(Default::default()), text));
    }
    let start = text
        .find('\n')
        .context("capture frontmatter has no closing ---")?
        + 1;
    let mut end = start;
    for line in lines {
        if line.trim_end() == "---" {
            let docs = YamlLoader::load_from_str(&text[start..end])?;
            if docs.len() != 1 || !matches!(docs[0], Yaml::Hash(_)) {
                bail!("capture frontmatter must be one YAML mapping");
            }
            return Ok((docs.into_iter().next().unwrap(), &text[end + line.len()..]));
        }
        end += line.len();
    }
    bail!("capture frontmatter has no closing ---")
}

fn set(front: &mut Yaml, key: &str, value: Yaml) {
    if let Yaml::Hash(map) = front {
        map.insert(Yaml::String(key.into()), value);
    }
}

fn scalar(value: &str) -> Yaml {
    Yaml::String(value.into())
}

fn render_frontmatter(front: &Yaml) -> Result<String> {
    let mut out = String::from("---\n");
    for (key, value) in front.as_hash().unwrap() {
        if let (Some(key), Some(value)) = (key.as_str(), value.as_str()) {
            if !key.is_empty()
                && key
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            {
                // Share the topic record writer's scalar escaping on disk.
                out.push_str(&format!("{key}: {}\n", quoted(value)));
                continue;
            }
        }
        // Preserve unknown structured values through the existing YAML library.
        let mut field = yaml_rust2::yaml::Hash::new();
        field.insert(key.clone(), value.clone());
        let mut yaml = String::new();
        YamlEmitter::new(&mut yaml).dump(&Yaml::Hash(field))?;
        out.push_str(yaml.strip_prefix("---\n").unwrap_or(&yaml));
        out.push('\n');
    }
    out.push_str("---\n");
    Ok(out)
}

pub(crate) fn capture(engine: &Engine, q: &CaptureQuery) -> Result<Reply<CaptureResult>> {
    if ![
        "evidence",
        "table",
        "visualisation",
        "snippet",
        "search",
        "widget",
    ]
    .contains(&q.kind.as_str())
    {
        bail!("kind must be evidence, table, visualisation, snippet, search or widget");
    }
    if q.title.trim().is_empty() {
        bail!("title must not be empty");
    }
    let by = if q.by.is_empty() { "verb" } else { &q.by };
    if !["verb", "agent", "app:talkweaver", "app:writeflex"].contains(&by) {
        bail!("by must be verb, agent, app:talkweaver or app:writeflex");
    }
    if let Some(s) = &q.slug {
        if s.len() > 60 {
            bail!("slug must be at most 60 characters");
        }
        if s.is_empty() || s != &slug(s) {
            bail!("slug must be lower-case ASCII words separated by hyphens");
        }
    }
    let into = q.into.as_deref().map(research_relative).transpose()?;
    let project = project_dir(&q.project)?;
    let provenance_project = source_path(&q.project, &std::env::current_dir()?)?;
    let content = match q.input.as_deref() {
        Some("-") => {
            let mut s = String::new();
            std::io::stdin().read_to_string(&mut s)?;
            s
        }
        Some(p) => {
            fs::read_to_string(expand_home(p)).with_context(|| format!("read capture input {p}"))?
        }
        None => q.body.clone(),
    };
    let (mut front, body) = frontmatter(&content)?;
    let mut sources = match &front["sources"] {
        Yaml::BadValue | Yaml::Null => vec![],
        value => yaml_to_sources(value, &provenance_project)?,
    };
    for source in &q.sources {
        let Some((kind, value)) = parse_source(source, &provenance_project)? else {
            bail!("source must be scout:<id>, a URL, or file:<path>");
        };
        sources.push(BTreeMap::from([(kind.into(), value)]));
    }
    let artefact = if let Some(raw) = q.artefact.as_deref() {
        Some(artefact_path(raw, &project)?)
    } else if let Some(raw) = front["artefact"].as_str() {
        let path = artefact_path(&path_text(&project.join(raw))?, &project)?;
        if !path.starts_with(&project) {
            bail!("frontmatter artefact must resolve inside the project");
        }
        Some(path)
    } else if !matches!(front["artefact"], Yaml::BadValue | Yaml::Null) {
        bail!("frontmatter artefact must be a file path");
    } else {
        None
    };
    // Re-emit user fields first, then the capture contract in its defined order.
    let old_prompt = front["prompt"].clone();
    if let Yaml::Hash(map) = &mut front {
        for key in [
            "title", "kind", "created", "project", "prompt", "sources", "artefact",
        ] {
            map.remove(&scalar(key));
        }
    }
    let containing = engine.registry().containing(&project);
    let project_value = match &containing {
        Some((_, root)) => {
            let relative = project.strip_prefix(root)?;
            if relative.as_os_str().is_empty() {
                ".".into()
            } else {
                path_text(relative)?
            }
        }
        None => path_text(&project)?,
    };
    let created = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
    for (key, value) in [
        ("title", q.title.as_str()),
        ("kind", &q.kind),
        ("created", &created),
        ("project", &project_value),
    ] {
        set(&mut front, key, scalar(value));
    }
    if let Some(prompt) = &q.prompt {
        set(&mut front, "prompt", scalar(prompt));
    } else if !matches!(old_prompt, Yaml::BadValue | Yaml::Null) {
        set(&mut front, "prompt", old_prompt);
    }
    if !sources.is_empty() {
        let mut values = vec![];
        for source in &sources {
            let mut map = yaml_rust2::yaml::Hash::new();
            for (k, v) in source {
                map.insert(scalar(k), scalar(v));
            }
            values.push(Yaml::Hash(map));
        }
        set(&mut front, "sources", Yaml::Array(values));
    }
    let research = safe_research(&project, true)?;
    let manifest = research.join("_manifest.jsonl");
    let _lock = crate::register::lock_registry(&research.join("manifest"))?;
    let destination = match into {
        Some(path) => research_path(&research, path, true)?,
        None => research.clone(),
    };
    let base = format!(
        "{}-{}",
        &created[..10],
        q.slug.clone().unwrap_or_else(|| slug(&q.title))
    );
    let mut n = 1;
    let (file, artefact_target) = loop {
        let name = if n == 1 {
            base.clone()
        } else {
            format!("{base}-{n}")
        };
        let file = destination.join(format!("{name}.md"));
        let copied = artefact
            .as_ref()
            .filter(|p| !p.starts_with(&project))
            .map(|p| {
                let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("bin");
                let suffix = if ext.eq_ignore_ascii_case("md")
                    || p.file_stem()
                        .is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case(&name))
                {
                    "-file"
                } else {
                    ""
                };
                destination.join(format!("{name}{suffix}.{ext}"))
            });
        let copy_free = match &copied {
            Some(path) => {
                !case_exists(path)?
                    && !path
                        .to_string_lossy()
                        .eq_ignore_ascii_case(&file.to_string_lossy())
            }
            None => true,
        };
        if !case_exists(&file)? && copy_free {
            break (file, copied);
        }
        n += 1;
    };
    if let Some(path) = &artefact {
        let value = match &artefact_target {
            Some(target) => relative_or_absolute(target, &project)?,
            None => relative_or_absolute(path, &project)?,
        };
        set(&mut front, "artefact", scalar(&value));
    }
    let bom = if content.starts_with('\u{feff}') {
        "\u{feff}"
    } else {
        ""
    };
    let markdown = format!("{bom}{}{body}", render_frontmatter(&front)?);
    let mut notes = vec![];
    if let (Some(source), Some(target)) = (&artefact, &artefact_target) {
        let mut input = fs::File::open(source)?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(target)?;
        if let Err(e) = std::io::copy(&mut input, &mut output).and_then(|_| output.sync_all()) {
            drop(output);
            let _ = fs::remove_file(target);
            return Err(e.into());
        }
    }
    let name = file.file_name().unwrap().to_str().unwrap();
    match crate::event_seed::create_exclusive(&destination, name, |f| {
        f.write_all(markdown.as_bytes())?;
        f.sync_all()
    }) {
        Ok(true) => {}
        result => {
            if let Some(target) = &artefact_target {
                let _ = fs::remove_file(target);
            }
            result?;
            bail!("capture file already exists; retry capture");
        }
    }
    let receipt = CaptureReceipt {
        file: path_text(file.strip_prefix(&research)?)?,
        kind: q.kind.clone(),
        title: q.title.clone(),
        created,
        by: by.into(),
        sources,
    };
    let line = serde_json::to_string(&receipt)? + "\n";
    let manifest_ok = match crate::register::write_atomic(&manifest, &line, append_staged) {
        Ok(()) => true,
        Err(e) => {
            notes.push(format!("scout: capture saved; manifest not appended: {e}"));
            false
        }
    };
    drop(_lock);
    let mut result = CaptureResult {
        schema_version: 1,
        file: path_text(&file)?,
        manifest: manifest_ok,
        corpus: containing.map(|(c, _)| c.id.clone()),
        indexed: false,
        passages: 0,
    };
    if let Some(id) = &result.corpus {
        match engine.index_build(&IndexBuildQuery {
            ids: vec![id.clone()],
            ..Default::default()
        }) {
            Ok(build) => {
                let checked = (|| -> Result<Option<usize>> {
                    use rusqlite::OptionalExtension;
                    let cfg = engine.registry().get(id)?;
                    let conn = crate::index::open_existing(
                        cfg,
                        &crate::index::index_path_in(engine.index_dir(), id),
                    )?;
                    let relative = path_text(file.strip_prefix(fs::canonicalize(cfg.root())?)?)?;
                    Ok(conn
                        .query_row(
                            "SELECT passages FROM documents WHERE rel_path = ?1",
                            [relative],
                            |row| row.get(0),
                        )
                        .optional()?)
                })();
                match checked {
                    Ok(Some(passages)) => {
                        result.indexed = true;
                        result.passages = passages;
                    }
                    Ok(None) => {}
                    Err(e) => notes.push(format!(
                        "scout: capture saved; index verification failed: {e}"
                    )),
                }
                notes.extend(build.notes);
            }
            Err(e) => notes.push(format!("scout: capture saved; index build failed: {e}")),
        }
    } else {
        notes.push("scout: capture saved; no registered corpus contains the project".into());
    }
    Ok(Reply {
        body: result,
        notes,
    })
}

/// The one source-prefix parser shared by `--source` flags and frontmatter
/// string rows. Returns `None` when no known prefix matches.
fn parse_source(raw: &str, project: &Path) -> Result<Option<(&'static str, String)>> {
    Ok(if let Some(id) = raw.strip_prefix("scout:") {
        Some(("scout", id.to_owned()))
    } else if let Some(file) = raw.strip_prefix("file:") {
        Some((
            "file",
            relative_or_absolute(&source_path(file, project)?, project)?,
        ))
    } else if let Some(url) = raw.strip_prefix("url:") {
        Some(("url", url.to_owned()))
    } else if raw.starts_with("https://") || raw.starts_with("http://") {
        Some(("url", raw.to_owned()))
    } else {
        None
    })
}

fn yaml_to_sources(value: &Yaml, project: &Path) -> Result<Vec<BTreeMap<String, String>>> {
    let rows = value
        .as_vec()
        .context("frontmatter sources must be a list")?;
    rows.iter()
        .map(|row| {
            if let Some(raw) = row.as_str() {
                // A bare string with no recognised prefix is a file path.
                let (kind, value) = match parse_source(raw, project)? {
                    Some(pair) => pair,
                    None => (
                        "file",
                        relative_or_absolute(&source_path(raw, project)?, project)?,
                    ),
                };
                return Ok(BTreeMap::from([(kind.into(), value)]));
            }
            row.as_hash()
                .context("each source must be a mapping")?
                .iter()
                .map(|(k, v)| {
                    let key = k.as_str().context("source key must be text")?;
                    let value = v.as_str().context("source value must be text")?;
                    let value = if key == "file" {
                        relative_or_absolute(&source_path(value, project)?, project)?
                    } else {
                        value.into()
                    };
                    Ok((key.into(), value))
                })
                .collect()
        })
        .collect()
}

/// Called under the manifest lock after the complete receipt has been synced
/// to a temporary file. Append in one write; roll back our partial write.
fn append_staged(from: &Path, to: &Path) -> std::io::Result<()> {
    let bytes = fs::read(from)?;
    let mut opts = OpenOptions::new();
    opts.read(true).append(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = opts.open(to)?;
    let meta = file.metadata()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.nlink() != 1 {
            return Err(std::io::Error::other("manifest must not have hard links"));
        }
    }
    if !meta.is_file() {
        return Err(std::io::Error::other("manifest must be a file"));
    }
    if meta.len() > 0 {
        use std::io::{Seek, SeekFrom};
        file.seek(SeekFrom::End(-1))?;
        let mut last = [0];
        file.read_exact(&mut last)?;
        if last != [b'\n'] {
            return Err(std::io::Error::other("manifest must end with a newline"));
        }
    }
    match file.write(&bytes) {
        Ok(n) if n == bytes.len() => {}
        other => {
            file.set_len(meta.len())?;
            return Err(other
                .err()
                .unwrap_or_else(|| std::io::Error::other("partial manifest append")));
        }
    }
    if let Err(e) = file.sync_all() {
        file.set_len(meta.len())?;
        return Err(e);
    }
    fs::remove_file(from)
}

pub(crate) fn list(project: &str) -> Result<CaptureList> {
    Ok(CaptureList {
        schema_version: 1,
        project: project.into(),
        entries: receipts(project)?,
    })
}

fn receipts(project: &str) -> Result<Vec<CaptureReceipt>> {
    let research = safe_research(&project_dir(project)?, false)?;
    let path = research.join("_manifest.jsonl");
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e.into()),
    };
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| Ok(serde_json::from_str(l)?))
        .collect()
}

/// Record an already completed move. Reads never repair the manifest implicitly.
pub(crate) fn moved(project: &str, old: &str, new: &str) -> Result<CaptureList> {
    let old = research_relative(old)?;
    let new = research_relative(new)?;
    let research = safe_research(&project_dir(project)?, false)?;
    if !research.is_dir() {
        bail!("research directory must exist");
    }
    let _lock = crate::register::lock_registry(&research.join("manifest"))?;
    let old_path = research_path(&research, old, false)?;
    let new_path = research_path(&research, new, false)?;
    if fs::symlink_metadata(&old_path).is_ok() {
        bail!("old capture still exists; move the file before updating its receipt");
    }
    if !new_path.is_file() {
        bail!("new capture must exist and be a file");
    }
    let manifest = research.join("_manifest.jsonl");
    let meta = fs::symlink_metadata(&manifest)?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        bail!("manifest must be a regular file");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.nlink() != 1 {
            bail!("manifest must not have hard links");
        }
    }
    let old = path_text(old)?;
    let new = path_text(new)?;
    let text = fs::read_to_string(&manifest)?;
    let mut found = false;
    let mut updated = String::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            updated.push_str(line);
        } else {
            let mut row: CaptureReceipt = serde_json::from_str(line)?;
            if row.file == new {
                bail!("new path already has a receipt");
            }
            if row.file == old {
                if found {
                    bail!("old path has multiple receipts");
                }
                row.file = new.clone();
                updated.push_str(&serde_json::to_string(&row)?);
                found = true;
            } else {
                updated.push_str(line);
            }
        }
        updated.push('\n');
    }
    if !found {
        bail!("old path has no receipt");
    }
    crate::register::write_atomic(&manifest, &updated, |from, to| fs::rename(from, to))?;
    list(project)
}

fn research_suffix(path: &Path) -> Option<PathBuf> {
    // Keep all subfolders below the research root in the identity.
    let mut suffix = None;
    let mut tail = PathBuf::new();
    for part in path.components() {
        if suffix.is_none() && part.as_os_str() == "research" {
            tail.clear();
            suffix = Some(PathBuf::new());
        } else if suffix.is_some() {
            tail.push(part);
            suffix = Some(tail.clone());
        }
    }
    suffix
}

fn mentions_research(line: &str, referencing: &Path, requested: &Path) -> bool {
    let absolute = requested.is_absolute();
    let identity = if absolute {
        research_suffix(requested)
    } else {
        Some(
            requested
                .strip_prefix("research")
                .unwrap_or(requested)
                .to_path_buf(),
        )
    };
    // Enclosed Markdown destinations may contain spaces; ordinary prose paths
    // are also considered as individual tokens.
    line.split(|c: char| "\"'`()[]<>{},;=|".contains(c))
        .map(str::trim)
        .chain(line.split(|c: char| c.is_whitespace() || "\"'`()[]<>{},;=|".contains(c)))
        .filter(|s| !s.is_empty())
        .any(|token| {
            let token = token.split(['#', '?']).next().unwrap();
            let decoded = percent_encoding::percent_decode_str(token).decode_utf8_lossy();
            let candidate = Path::new(decoded.as_ref());
            let resolved = source_path(decoded.as_ref(), referencing.parent().unwrap()).ok();
            if absolute && resolved.as_deref() == Some(requested) {
                return true;
            }
            let Some(identity) = identity.as_deref() else {
                return false;
            };
            // Research-relative shorthand is exact, never a basename substring.
            if !candidate.is_absolute()
                && (candidate == identity
                    || candidate.strip_prefix("research").ok() == Some(identity))
            {
                return true;
            }
            !absolute && resolved.as_deref().and_then(research_suffix).as_deref() == Some(identity)
        })
}

pub(crate) fn where_used(registry: &Registry, file: &str) -> Result<CaptureUsages> {
    let requested = file.to_owned();
    let file = Path::new(file);
    if file.file_name().is_none() {
        bail!("where-used needs a file path");
    }
    let normal = if file.exists() {
        fs::canonicalize(file)?
    } else if file.is_absolute() {
        source_path(&requested, Path::new("/"))?
    } else {
        research_relative(&requested)?.to_path_buf()
    };
    let mut hits = BTreeMap::new();
    let mut skipped = std::collections::BTreeSet::new();
    for corpus in registry
        .corpora
        .iter()
        .filter(|c| c.kind == CorpusKind::MarkdownFolder)
    {
        let root = corpus.root();
        let include = crate::markdown::build_globs(&if corpus.include.is_empty() {
            vec!["**/*.md".into()]
        } else {
            corpus.include.clone()
        })?;
        let exclude = crate::markdown::build_globs(&corpus.exclude)?;
        // Scan source Markdown, including outlines and pieces without required
        // corpus metadata. Research stubs and their receipts are not usages.
        for entry in walkdir::WalkDir::new(corpus.root())
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                e.depth() == 0
                    || (!e.file_name().to_string_lossy().starts_with('.')
                        && e.file_name() != "research")
            })
        {
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) => {
                    skipped.insert(e.path().unwrap_or(&root).to_path_buf());
                    continue;
                }
            };
            if !entry.file_type().is_file() || entry.path().extension().is_none_or(|e| e != "md") {
                continue;
            }
            let relative = entry.path().strip_prefix(&root).unwrap_or(entry.path());
            if !include.is_match(relative) || exclude.is_match(relative) {
                continue;
            }
            let read = (|| -> Result<(String, String)> {
                Ok((
                    path_text(&fs::canonicalize(entry.path())?)?,
                    fs::read_to_string(entry.path())?,
                ))
            })();
            let (path, text) = match read {
                Ok(pair) => pair,
                Err(_) => {
                    skipped.insert(entry.path().to_path_buf());
                    continue;
                }
            };
            for (i, line) in text.lines().enumerate() {
                if mentions_research(line, Path::new(&path), &normal) {
                    hits.insert(
                        (path.clone(), i + 1),
                        CaptureUsage {
                            file: path.clone(),
                            line: i + 1,
                            text: line.into(),
                        },
                    );
                }
            }
        }
    }
    Ok(CaptureUsages {
        schema_version: 1,
        file: requested,
        hits: hits.into_values().collect(),
        skipped: skipped.len(),
    })
}

fn case_exists(path: &Path) -> Result<bool> {
    let name = path.file_name().unwrap().to_string_lossy();
    for entry in fs::read_dir(path.parent().unwrap())? {
        if entry?
            .file_name()
            .to_string_lossy()
            .eq_ignore_ascii_case(&name)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

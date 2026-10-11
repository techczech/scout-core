//! The topic layer: model-made themes, entities and events kept BESIDE a
//! corpus, read-only, and copied into the derived index at build time.
//!
//! - Layer: `<corpus root>/analysis/topics/current.jsonl`, one JSON object
//!   per item: `key` (`<source_system>:<source_id>`), `path` (relative to
//!   the corpus root), `date`, `month`, `ai_related`, `themes[]`,
//!   `entities[]`, `event` (`YYYY-MM/Exx` or null), `event_name`,
//!   `confidence`, `run_id`, `prompt_version` (spec
//!   `docs/specs/2026-10-10-topic-tagging-layer.md`). A missing file means
//!   no layer, not an error. Lines that do not parse are skipped and counted
//!   (the file may be mid-write); a repeated key keeps its last line.
//! - Corrections: `<root>/analysis/topics/corrections.jsonl`, append-only,
//!   optional. Each line names a `key` and any of `themes`, `entities`,
//!   `event`, `event_name`, `ai_related`; each field present overrides the
//!   layer's value for that key (null clears `event`). Later lines win.
//!   Other keys on the line (`by`, `date`, `note`) are provenance and are
//!   not read. A correction for a key absent from the layer is counted, not
//!   applied.
//! - Join: a layer item belongs to every indexed document whose frontmatter
//!   `source_system` + `source_id` give its key; failing that, to the
//!   document whose key (relative path) equals its `path`.
//! - Month: the item's `date` cut to `YYYY-MM` (the month of its latest
//!   highlight), else the layer's `month` when that is `YYYY-MM`. It is
//!   the month `month:` filters on and the month facet counts; `y:` /
//!   `after:` / `before:` keep filtering on the document's own date.
//!
//! The archive files are never written. The index copy is rebuilt whenever
//! the layer files' bytes or the corpus's documents change.

use crate::filter::DocInfo;
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

/// Schema version of `scout facets --json` and `scout topics --json`.
pub const SCHEMA_VERSION: u32 = 1;

/// The layer file, relative to the corpus root.
pub const LAYER_FILE: &str = "analysis/topics/current.jsonl";
/// The corrections file, relative to the corpus root.
pub const CORRECTIONS_FILE: &str = "analysis/topics/corrections.jsonl";

/// The fields a correction may override.
pub const CORRECTABLE: &[&str] = &["themes", "entities", "event", "event_name", "ai_related"];

/// One document's topic tags, corrections applied.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopicInfo {
    /// The layer key, `<source_system>:<source_id>`.
    pub key: String,
    pub themes: Vec<String>,
    pub entities: Vec<String>,
    /// `YYYY-MM/Exx`, or null.
    pub event: Option<String>,
    pub event_name: Option<String>,
    pub ai_related: Option<bool>,
    /// `YYYY-MM` (see the module notes).
    pub month: Option<String>,
    pub confidence: Option<String>,
    pub run_id: Option<String>,
    pub prompt_version: Option<String>,
    /// Fields a correction overrode, in [`CORRECTABLE`] order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub corrected: Vec<String>,
}

fn fold(s: &str) -> String {
    s.trim().to_lowercase()
}

impl TopicInfo {
    pub fn has_theme(&self, name: &str) -> bool {
        let f = fold(name);
        self.themes.iter().any(|t| fold(t) == f)
    }

    pub fn has_entity(&self, name: &str) -> bool {
        let f = fold(name);
        self.entities.iter().any(|t| fold(t) == f)
    }

    pub fn has_event(&self, id: &str) -> bool {
        self.event.as_deref().is_some_and(|e| fold(e) == fold(id))
    }

    /// `month` is `YYYY-MM` (that month) or `YYYY` (any month of the year).
    pub fn in_month(&self, month: &str) -> bool {
        let m = month.trim();
        self.month
            .as_deref()
            .is_some_and(|have| have == m || (m.len() == 4 && have.starts_with(m)))
    }
}

/// Is `v` a `month:` value: `YYYY` or `YYYY-MM`?
pub fn valid_month(v: &str) -> bool {
    let b = v.trim().as_bytes();
    let digits = |s: &[u8]| s.iter().all(u8::is_ascii_digit);
    match b.len() {
        4 => digits(b),
        7 => {
            digits(&b[..4])
                && b[4] == b'-'
                && digits(&b[5..])
                && (1..=12).contains(&(((b[5] - b'0') * 10) + (b[6] - b'0')))
        }
        _ => false,
    }
}

/// `ai:` values: `true|yes|1` and `false|no|0`, any case.
pub fn parse_ai(v: &str) -> Result<bool> {
    match v.trim().to_lowercase().as_str() {
        "true" | "yes" | "1" => Ok(true),
        "false" | "no" | "0" => Ok(false),
        _ => bail!("ai:{v} must be true or false"),
    }
}

fn month_of(date: Option<&str>, layer_month: Option<&str>) -> Option<String> {
    let cut = |s: &str| {
        let m = s.trim().get(..7)?;
        (m.len() == 7 && valid_month(m)).then(|| m.to_string())
    };
    date.and_then(cut)
        .or_else(|| layer_month.filter(|m| m.trim().len() == 7).and_then(cut))
}

#[derive(Deserialize)]
struct LayerLine {
    key: String,
    path: Option<String>,
    date: Option<String>,
    month: Option<String>,
    ai_related: Option<bool>,
    themes: Option<Vec<String>>,
    entities: Option<Vec<String>>,
    event: Option<String>,
    event_name: Option<String>,
    confidence: Option<String>,
    run_id: Option<String>,
    prompt_version: Option<String>,
}

/// One layer item: its locator path and its tags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerItem {
    pub path: Option<String>,
    pub info: TopicInfo,
}

/// The layer as read from the corpus folder, corrections applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layer {
    pub layer_path: PathBuf,
    /// Items by key.
    pub items: BTreeMap<String, LayerItem>,
    /// Layer lines that did not parse or had no key.
    pub bad_lines: usize,
    /// Correction lines applied.
    pub corrections: usize,
    /// Correction lines whose key the layer does not hold.
    pub corrections_unmatched: usize,
    /// Correction lines that did not parse or carried a wrongly typed field.
    pub corrections_bad_lines: usize,
}

/// The raw bytes of the layer and corrections files and their combined hash.
struct LayerBytes {
    layer: Vec<u8>,
    corrections: Option<Vec<u8>>,
    hash: String,
}

/// `None` when the layer file is missing.
fn read_bytes(root: &Path) -> Result<Option<LayerBytes>> {
    let read = |p: PathBuf| -> Result<Option<Vec<u8>>> {
        match std::fs::read(&p) {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).with_context(|| format!("read {}", p.display())),
        }
    };
    let Some(layer) = read(root.join(LAYER_FILE))? else {
        return Ok(None);
    };
    let corr = read(root.join(CORRECTIONS_FILE))?;
    let mut h = Sha1::new();
    h.update(&layer);
    h.update([0u8]);
    if let Some(c) = &corr {
        h.update(c);
    }
    let hash = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    Ok(Some(LayerBytes {
        layer,
        corrections: corr,
        hash,
    }))
}

/// Read the topic layer of the corpus at `root`; `None` when it has none.
pub fn read_layer(root: &Path) -> Result<Option<Layer>> {
    Ok(read_bytes(root)?.map(|b| parse_layer(root, &b.layer, b.corrections.as_deref())))
}

fn parse_layer(root: &Path, layer: &[u8], corrections: Option<&[u8]>) -> Layer {
    let mut out = Layer {
        layer_path: root.join(LAYER_FILE),
        items: BTreeMap::new(),
        bad_lines: 0,
        corrections: 0,
        corrections_unmatched: 0,
        corrections_bad_lines: 0,
    };
    for line in String::from_utf8_lossy(layer).lines() {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(l) = serde_json::from_str::<LayerLine>(line) else {
            out.bad_lines += 1;
            continue;
        };
        let key = l.key.trim().to_string();
        if key.is_empty() {
            out.bad_lines += 1;
            continue;
        }
        let info = TopicInfo {
            key: key.clone(),
            themes: l.themes.unwrap_or_default(),
            entities: l.entities.unwrap_or_default(),
            event: l.event.filter(|e| !e.trim().is_empty()),
            event_name: l.event_name,
            ai_related: l.ai_related,
            month: month_of(l.date.as_deref(), l.month.as_deref()),
            confidence: l.confidence,
            run_id: l.run_id,
            prompt_version: l.prompt_version,
            corrected: vec![],
        };
        out.items.insert(
            key,
            LayerItem {
                path: l.path.filter(|p| !p.trim().is_empty()),
                info,
            },
        );
    }
    if let Some(c) = corrections {
        for line in String::from_utf8_lossy(c).lines() {
            if line.trim().is_empty() {
                continue;
            }
            match apply_correction(&mut out.items, line) {
                Correction::Applied => out.corrections += 1,
                Correction::UnknownKey => out.corrections_unmatched += 1,
                Correction::Bad => out.corrections_bad_lines += 1,
            }
        }
    }
    fill_event_names(&mut out.items);
    out
}

enum Correction {
    Applied,
    UnknownKey,
    Bad,
}

fn apply_correction(items: &mut BTreeMap<String, LayerItem>, line: &str) -> Correction {
    let Ok(serde_json::Value::Object(m)) = serde_json::from_str::<serde_json::Value>(line) else {
        return Correction::Bad;
    };
    let Some(key) = m.get("key").and_then(|k| k.as_str()).map(str::trim) else {
        return Correction::Bad;
    };
    // Validate every field before touching the item.
    let strings = |v: &serde_json::Value| -> Option<Vec<String>> {
        v.as_array()?
            .iter()
            .map(|x| x.as_str().map(String::from))
            .collect()
    };
    let opt_str = |v: &serde_json::Value| -> Option<Option<String>> {
        match v {
            serde_json::Value::Null => Some(None),
            serde_json::Value::String(s) if s.trim().is_empty() => Some(None),
            serde_json::Value::String(s) => Some(Some(s.clone())),
            _ => None,
        }
    };
    let mut themes = None;
    let mut entities = None;
    let mut event = None;
    let mut event_name = None;
    let mut ai = None;
    for (field, v) in &m {
        let ok = match field.as_str() {
            "themes" => strings(v).map(|s| themes = Some(s)).is_some(),
            "entities" => strings(v).map(|s| entities = Some(s)).is_some(),
            "event" => opt_str(v).map(|s| event = Some(s)).is_some(),
            "event_name" => opt_str(v).map(|s| event_name = Some(s)).is_some(),
            "ai_related" => match v {
                serde_json::Value::Bool(b) => {
                    ai = Some(Some(*b));
                    true
                }
                serde_json::Value::Null => {
                    ai = Some(None);
                    true
                }
                _ => false,
            },
            _ => true,
        };
        if !ok {
            return Correction::Bad;
        }
    }
    let Some(item) = items.get_mut(key) else {
        return Correction::UnknownKey;
    };
    let t = &mut item.info;
    let mark = |f: &str, t: &mut TopicInfo| {
        if !t.corrected.iter().any(|c| c == f) {
            t.corrected.push(f.to_string());
        }
    };
    if let Some(v) = themes {
        t.themes = v;
        mark("themes", t);
    }
    if let Some(v) = entities {
        t.entities = v;
        mark("entities", t);
    }
    if let Some(v) = event {
        // A new event id without a name of its own takes the layer's name
        // for that id (filled below), not the old event's name.
        if v != t.event && event_name.is_none() {
            t.event_name = None;
        }
        t.event = v;
        mark("event", t);
    }
    if let Some(v) = event_name {
        t.event_name = v;
        mark("event_name", t);
    }
    if let Some(v) = ai {
        t.ai_related = v;
        mark("ai_related", t);
    }
    t.corrected
        .sort_by_key(|f| CORRECTABLE.iter().position(|c| c == f));
    Correction::Applied
}

/// Items with an event but no name take the most frequent name the layer
/// gives that event id (ties: the alphabetically first).
fn fill_event_names(items: &mut BTreeMap<String, LayerItem>) {
    let mut names: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    for it in items.values() {
        if let (Some(e), Some(n)) = (&it.info.event, &it.info.event_name) {
            *names
                .entry(fold(e))
                .or_default()
                .entry(n.clone())
                .or_default() += 1;
        }
    }
    let best: BTreeMap<String, String> = names
        .into_iter()
        .filter_map(|(e, ns)| {
            let max = *ns.values().max()?;
            ns.into_iter().find(|(_, c)| *c == max).map(|(n, _)| (e, n))
        })
        .collect();
    for it in items.values_mut() {
        if it.info.event_name.is_none() {
            if let Some(e) = &it.info.event {
                it.info.event_name = best.get(&fold(e)).cloned();
            }
        }
    }
}

/// What a build did with the topic layer (`index build` reports it).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TopicReport {
    pub layer_path: String,
    /// Distinct keys in the layer.
    pub items: usize,
    /// Items joined to at least one document.
    pub matched: usize,
    pub matched_by_key: usize,
    pub matched_by_path: usize,
    pub unmatched: usize,
    /// Up to 5 unmatched keys, sorted.
    pub unmatched_sample: Vec<String>,
    /// Documents carrying tags.
    pub tagged_documents: usize,
    pub bad_lines: usize,
    pub corrections: usize,
    pub corrections_unmatched: usize,
    pub corrections_bad_lines: usize,
    /// False when the layer and the documents were unchanged since the last
    /// build, so the stored copy was kept.
    pub reloaded: bool,
}

pub(crate) const TABLE_SQL: &str = "
    CREATE TABLE IF NOT EXISTS topic_docs (
        rel_path TEXT PRIMARY KEY,
        key TEXT NOT NULL,
        matched_by TEXT NOT NULL,
        info TEXT NOT NULL
    );";

const HASH_KEY: &str = "topic_layer_hash";
const REPORT_KEY: &str = "topic_report";

/// Copy a layer into the index's `topic_docs`, joined to its documents.
/// `None` empties the table.
pub fn apply(conn: &Connection, layer: Option<&Layer>) -> Result<Option<TopicReport>> {
    conn.execute("DELETE FROM topic_docs", [])?;
    let Some(layer) = layer else {
        return Ok(None);
    };
    let mut by_key: HashMap<String, Vec<String>> = HashMap::new();
    let mut paths: BTreeSet<String> = BTreeSet::new();
    {
        let mut st = conn.prepare("SELECT rel_path, source, source_id FROM documents")?;
        let rows = st.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })?;
        for row in rows {
            let (rel, source, id) = row?;
            if let (Some(s), Some(i)) = (source, id) {
                by_key
                    .entry(format!("{}:{}", s.trim(), i.trim()))
                    .or_default()
                    .push(rel.clone());
            }
            paths.insert(rel);
        }
    }
    // Key matches first, so a path match never displaces one.
    let mut rows: BTreeMap<String, (&str, &'static str, &TopicInfo)> = BTreeMap::new();
    let (mut by_k, mut by_p) = (0usize, 0usize);
    let mut unmatched_items: Vec<&LayerItem> = Vec::new();
    for (key, it) in &layer.items {
        match by_key.get(key) {
            Some(rels) => {
                by_k += 1;
                for r in rels {
                    rows.insert(r.clone(), (key, "key", &it.info));
                }
            }
            None => unmatched_items.push(it),
        }
    }
    let mut unmatched = Vec::new();
    for it in unmatched_items {
        match it.path.as_deref().map(str::trim) {
            Some(p) if paths.contains(p) && !rows.contains_key(p) => {
                by_p += 1;
                rows.insert(p.to_string(), (&it.info.key, "path", &it.info));
            }
            _ => unmatched.push(it.info.key.clone()),
        }
    }
    let mut ins = conn.prepare_cached(
        "INSERT INTO topic_docs (rel_path, key, matched_by, info) VALUES (?1, ?2, ?3, ?4)",
    )?;
    for (rel, (key, how, info)) in &rows {
        ins.execute(params![rel, key, how, serde_json::to_string(info)?])?;
    }
    unmatched.sort();
    Ok(Some(TopicReport {
        layer_path: layer.layer_path.display().to_string(),
        items: layer.items.len(),
        matched: by_k + by_p,
        matched_by_key: by_k,
        matched_by_path: by_p,
        unmatched: unmatched.len(),
        unmatched_sample: unmatched.into_iter().take(5).collect(),
        tagged_documents: rows.len(),
        bad_lines: layer.bad_lines,
        corrections: layer.corrections,
        corrections_unmatched: layer.corrections_unmatched,
        corrections_bad_lines: layer.corrections_bad_lines,
        reloaded: true,
    }))
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

/// Bring the index's topic copy up to date with the layer at `root`
/// (build time, inside the build transaction). Re-joins when the layer
/// files' bytes or the documents (`docs_changed`) changed; otherwise keeps
/// the stored copy and returns its report with `reloaded: false`.
pub(crate) fn refresh(
    conn: &Connection,
    root: &Path,
    docs_changed: bool,
) -> Result<Option<TopicReport>> {
    let bytes = read_bytes(root)?;
    let hash = bytes
        .as_ref()
        .map(|b| b.hash.clone())
        .unwrap_or_else(|| "none".into());
    if !docs_changed && meta_get(conn, HASH_KEY)?.as_deref() == Some(hash.as_str()) {
        let stored: Option<TopicReport> = meta_get(conn, REPORT_KEY)?
            .and_then(|s| serde_json::from_str::<Option<TopicReport>>(&s).ok())
            .flatten();
        return Ok(stored.map(|mut r| {
            r.reloaded = false;
            r
        }));
    }
    let layer = bytes.map(|b| parse_layer(root, &b.layer, b.corrections.as_deref()));
    let report = apply(conn, layer.as_ref())?;
    meta_set(conn, HASH_KEY, &hash)?;
    meta_set(conn, REPORT_KEY, &serde_json::to_string(&report)?)?;
    Ok(report)
}

/// Every document's tags, by document key; empty for an index built
/// before the topic layer existed.
pub fn load(conn: &Connection) -> Result<BTreeMap<String, TopicInfo>> {
    let has: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE name = 'topic_docs'",
        [],
        |r| r.get(0),
    )?;
    let mut out = BTreeMap::new();
    if has == 0 {
        return Ok(out);
    }
    let mut st = conn.prepare("SELECT rel_path, info FROM topic_docs")?;
    for row in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (rel, info) = row?;
        if let Ok(t) = serde_json::from_str::<TopicInfo>(&info) {
            out.insert(rel, t);
        }
    }
    Ok(out)
}

// ---------- facets ----------

/// One facet value and the documents carrying it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FacetItem {
    pub value: String,
    pub count: usize,
    /// Events: the event's name from the layer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Facet {
    /// Distinct values before `top`.
    pub distinct: usize,
    pub items: Vec<FacetItem>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct AiCounts {
    #[serde(rename = "true")]
    pub yes: usize,
    #[serde(rename = "false")]
    pub no: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Facets {
    pub theme: Facet,
    pub entity: Facet,
    pub event: Facet,
    pub month: Facet,
    pub ai: AiCounts,
}

/// Which facet a listing asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TopicField {
    Theme,
    Entity,
    Event,
    Month,
}

impl std::str::FromStr for TopicField {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        Ok(match s.trim().to_lowercase().as_str() {
            "theme" | "themes" => TopicField::Theme,
            "entity" | "entities" => TopicField::Entity,
            "event" | "events" => TopicField::Event,
            "month" | "months" => TopicField::Month,
            other => bail!("unknown topic field {other:?}: use themes, entities, events or months"),
        })
    }
}

/// Spellings per case-folded value, and the documents per value.
#[derive(Debug, Default)]
struct ValueCounts {
    /// folded value -> (spelling -> documents, documents)
    values: BTreeMap<String, (BTreeMap<String, usize>, usize)>,
}

impl ValueCounts {
    fn add_doc<'a>(&mut self, vals: impl IntoIterator<Item = &'a String>) {
        let mut seen = BTreeSet::new();
        for v in vals {
            let f = fold(v);
            if f.is_empty() || !seen.insert(f.clone()) {
                continue;
            }
            let e = self.values.entry(f).or_default();
            *e.0.entry(v.trim().to_string()).or_default() += 1;
            e.1 += 1;
        }
    }

    /// Items by count desc, then value; the displayed spelling is the most
    /// frequent one (ties: alphabetically first). `top` 0 keeps all.
    fn facet(&self, top: usize, chronological: bool) -> Facet {
        let mut items: Vec<FacetItem> = self
            .values
            .values()
            .map(|(spellings, count)| {
                let max = spellings.values().max().copied().unwrap_or(0);
                let value = spellings
                    .iter()
                    .find(|(_, c)| **c == max)
                    .map(|(s, _)| s.clone())
                    .unwrap_or_default();
                FacetItem {
                    value,
                    count: *count,
                    name: None,
                }
            })
            .collect();
        if chronological {
            items.sort_by(|a, b| a.value.cmp(&b.value));
        } else {
            items.sort_by(|a, b| b.count.cmp(&a.count).then(a.value.cmp(&b.value)));
        }
        let distinct = items.len();
        if top > 0 {
            items.truncate(top);
        }
        Facet { distinct, items }
    }
}

/// Counts documents per theme, entity, event and month. Feed it the
/// documents of a result set (several corpora may feed one counter); each
/// document counts once per value. Values match case-insensitively, as the
/// query fields do, so a facet's count is the document total of the
/// matching `theme:` / `entity:` / `event:` / `month:` query.
#[derive(Debug, Default)]
pub struct FacetCounter {
    pub documents: usize,
    pub tagged: usize,
    theme: ValueCounts,
    entity: ValueCounts,
    event: ValueCounts,
    month: ValueCounts,
    event_names: BTreeMap<String, BTreeMap<String, usize>>,
    ai: AiCounts,
}

impl FacetCounter {
    pub fn add(&mut self, d: &DocInfo) {
        self.documents += 1;
        let Some(t) = &d.topic else {
            return;
        };
        self.tagged += 1;
        self.theme.add_doc(&t.themes);
        self.entity.add_doc(&t.entities);
        self.event.add_doc(t.event.iter());
        self.month.add_doc(t.month.iter());
        if let (Some(e), Some(n)) = (&t.event, &t.event_name) {
            *self
                .event_names
                .entry(fold(e))
                .or_default()
                .entry(n.clone())
                .or_default() += 1;
        }
        match t.ai_related {
            Some(true) => self.ai.yes += 1,
            Some(false) => self.ai.no += 1,
            None => {}
        }
    }

    fn event_facet(&self, top: usize) -> Facet {
        let mut f = self.event.facet(top, false);
        for it in f.items.iter_mut() {
            it.name = self.event_names.get(&fold(&it.value)).and_then(|ns| {
                let max = ns.values().max()?;
                ns.iter().find(|(_, c)| *c == max).map(|(n, _)| n.clone())
            });
        }
        f
    }

    /// All four facets; `top` cuts themes, entities and events (0 = all);
    /// months are all listed, chronologically.
    pub fn facets(&self, top: usize) -> Facets {
        Facets {
            theme: self.theme.facet(top, false),
            entity: self.entity.facet(top, false),
            event: self.event_facet(top),
            month: self.month.facet(0, true),
            ai: self.ai.clone(),
        }
    }

    /// One facet (`scout topics`); months chronological, others by count.
    pub fn facet(&self, field: TopicField, top: usize) -> Facet {
        match field {
            TopicField::Theme => self.theme.facet(top, false),
            TopicField::Entity => self.entity.facet(top, false),
            TopicField::Event => self.event_facet(top),
            TopicField::Month => self.month.facet(top, true),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn months_and_ai_values() {
        assert!(valid_month("2026-09"));
        assert!(valid_month("2026"));
        assert!(!valid_month("2026-13"));
        assert!(!valid_month("2026-9"));
        assert!(!valid_month("2026-Q2"));
        assert_eq!(
            month_of(Some("2026-09-30"), Some("2026-09")).as_deref(),
            Some("2026-09")
        );
        assert_eq!(month_of(None, Some("2022-Q2")), None);
        assert_eq!(month_of(None, Some("2022-05")).as_deref(), Some("2022-05"));
        assert!(parse_ai("Yes").unwrap());
        assert!(!parse_ai("false").unwrap());
        assert!(parse_ai("maybe").is_err());
    }
}

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
//!   applied. In a corpus with a records folder, `event_add` /
//!   `event_remove` (lists of event ids) put the item in further events or
//!   take it out of one; see "Events" below.
//! - Events: everything in this note holds only for a corpus that has a
//!   records folder (`analysis/events/`, [`crate::events`]). There an item
//!   can be in several events: the tag row's `event`, the ids of its
//!   optional `events` list, plus `event_add` and minus `event_remove` from
//!   the corrections, in line order (the later line wins; within a line
//!   removals follow additions). Every id is resolved through the records:
//!   a monthly id becomes the lasting event that holds it, and
//!   `TopicInfo::events` carries the result. Without the folder none of it
//!   is read: a tag row's `events` and a correction's `event_add` /
//!   `event_remove` are unknown keys (ignored whatever their type, as
//!   before lasting events), `TopicInfo::events` is absent and the item's
//!   one event is `event`.
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

use crate::events::{EventSet, EventsReport};
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub use crate::topic_facets::{AiCounts, Facet, FacetCounter, FacetItem, Facets, TopicField};
use std::path::{Path, PathBuf};

/// Schema version of `scout facets --json` and `scout topics --json`.
pub const SCHEMA_VERSION: u32 = 1;

/// The layer file, relative to the corpus root.
pub const LAYER_FILE: &str = "analysis/topics/current.jsonl";
/// The corrections file, relative to the corpus root.
pub const CORRECTIONS_FILE: &str = "analysis/topics/corrections.jsonl";

/// Longest layer or corrections line read, in bytes; a longer line is a
/// bad line.
pub const MAX_LINE_BYTES: usize = 1 << 20;

/// A query term selecting one facet value: `field:value`, the value quoted
/// and escaped as the grammar needs (`entity:"GPT \"Astra\""`). Every
/// value `scout facets` returns gives the facet's count this way.
pub fn field_query(field: &str, value: &str) -> String {
    format!("{field}:{}", scout_query::quote_value(value))
}

/// The fields a correction may override.
pub const CORRECTABLE: &[&str] = &[
    "themes",
    "entities",
    "event",
    "event_name",
    "ai_related",
    "events",
];

/// One document's topic tags, corrections applied.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopicInfo {
    /// The layer key, `<source_system>:<source_id>`.
    pub key: String,
    pub themes: Vec<String>,
    pub entities: Vec<String>,
    /// The tag row's event, `YYYY-MM/Exx`, or null (also null when a
    /// correction took the item out of that event).
    pub event: Option<String>,
    pub event_name: Option<String>,
    /// Every event the item is in, as lasting ids where a record holds the
    /// event (see the module notes). Present exactly when the corpus has a
    /// records folder; absent, the item's one event is `event`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub events: Option<Vec<String>>,
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

/// The one matching rule for labels, applied alike to stored labels, query
/// values and facet grouping: Unicode NFC, curly quotes and apostrophes
/// unified to `"` and `'` (as `verify-quote` does), whitespace runs to one
/// space, trimmed, lowercased. So `O’Reilly` = `o'reilly` and
/// `GPT “Astra”` = `gpt "astra"`.
pub fn fold(s: &str) -> String {
    crate::normalize::fold(s)
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

    /// The ids of every event the item is in: `events`, else `event`.
    pub fn event_ids(&self) -> &[String] {
        match &self.events {
            Some(l) => l,
            None => self.event.as_slice(),
        }
    }

    /// Is the item in the event with this id? `id` is compared as given:
    /// resolve a monthly or merged id first ([`crate::events::resolve_in`]).
    pub fn has_event(&self, id: &str) -> bool {
        let f = fold(id);
        self.event_ids().iter().any(|e| fold(e) == f)
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

/// A tag row's further events, read from the line a second time and only
/// in a corpus with a records folder, so that without one the key is as
/// unknown to the reader as it was before lasting events. Anything but a
/// list is left unread.
#[derive(Deserialize)]
struct ListedEvents {
    events: Option<serde_json::Value>,
}

/// What a tag row and the corrections say about an item's events beyond
/// its `event`.
#[derive(Debug, Default)]
struct Membership {
    /// The tag row's `events` list.
    listed: Option<Vec<String>>,
    /// `event_add` (true) and `event_remove` (false), in line order.
    ops: Vec<(String, bool)>,
}

/// One layer item: its locator path and its tags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerItem {
    pub path: Option<String>,
    /// The row's `date` as written (`month` in `info` is derived from it).
    pub date: Option<String>,
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
    /// The corpus's event records, when it has a records folder.
    pub events: Option<EventSet>,
}

/// A line of a layer file: valid UTF-8 within [`MAX_LINE_BYTES`], or bad.
enum Line<'a> {
    Text(&'a str),
    Bad,
}

/// Stream `path` line by line (`\n`, an optional `\r` dropped), feeding
/// every byte to `hash` and every non-blank line to `each`. A line over
/// `cap` bytes is not kept in memory: it is drained and reported `Bad`, as
/// is a line that is not valid UTF-8. `Ok(false)` when the file is missing.
fn stream_lines(
    path: &Path,
    cap: usize,
    hash: &mut Sha1,
    mut each: impl FnMut(Line<'_>),
) -> Result<bool> {
    use std::io::BufRead;
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e).with_context(|| format!("read {}", path.display())),
    };
    let mut r = std::io::BufReader::with_capacity(64 * 1024, file);
    let mut buf: Vec<u8> = Vec::new();
    let mut over = false;
    let mut emit = |buf: &mut Vec<u8>, over: &mut bool| {
        if *over {
            each(Line::Bad);
        } else {
            let mut b: &[u8] = buf;
            if b.last() == Some(&b'\r') {
                b = &b[..b.len() - 1];
            }
            match std::str::from_utf8(b) {
                Ok(t) if t.trim().is_empty() => {}
                Ok(t) => each(Line::Text(t)),
                Err(_) => each(Line::Bad),
            }
        }
        buf.clear();
        *over = false;
    };
    loop {
        let chunk = r
            .fill_buf()
            .with_context(|| format!("read {}", path.display()))?;
        if chunk.is_empty() {
            if !buf.is_empty() || over {
                emit(&mut buf, &mut over);
            }
            return Ok(true);
        }
        let (part, newline) = match chunk.iter().position(|&b| b == b'\n') {
            Some(i) => (&chunk[..i], Some(i)),
            None => (chunk, None),
        };
        hash.update(&chunk[..newline.map_or(chunk.len(), |i| i + 1)]);
        if !over {
            if buf.len() + part.len() > cap {
                over = true;
                buf.clear();
            } else {
                buf.extend_from_slice(part);
            }
        }
        let used = newline.map_or(chunk.len(), |i| i + 1);
        r.consume(used);
        if newline.is_some() {
            emit(&mut buf, &mut over);
        }
    }
}

fn hex(h: Sha1) -> String {
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// The hash stored for a corpus with neither a layer file nor a records
/// folder.
const NO_HASH: &str = "none";

/// The combined hash of the event records, the layer file and the
/// corrections file, streamed in that order (the order [`read_beside`]
/// reads them in). The records are hashed whether or not there is a layer
/// file (the corrections are read only with one); [`NO_HASH`] when the
/// corpus has neither. Without a records folder the hash is that of the
/// layer and corrections bytes alone, as before lasting events.
fn layer_hash(root: &Path) -> Result<String> {
    let mut h = Sha1::new();
    let records = crate::events::hash_into(root, &mut h)?;
    let layer = stream_lines(&root.join(LAYER_FILE), 0, &mut h, |_| {})?;
    if layer {
        h.update([0u8]);
        stream_lines(&root.join(CORRECTIONS_FILE), 0, &mut h, |_| {})?;
    }
    Ok(if layer || records {
        hex(h)
    } else {
        NO_HASH.into()
    })
}

/// What a build reads beside the sources.
struct Beside {
    /// The topic layer, with the event records in `Layer::events`.
    layer: Option<Layer>,
    /// The event records of a corpus that has no layer file.
    records_alone: Option<EventSet>,
    /// The hash of exactly the bytes read ([`layer_hash`]).
    hash: String,
}

/// Read the topic layer of the corpus at `root`; `None` when it has none.
pub fn read_layer(root: &Path) -> Result<Option<Layer>> {
    Ok(read_beside(root)?.layer)
}

/// The layer and the event records, each read whether or not the other is
/// there, plus the hash of exactly the bytes read.
///
/// The records are read first: whether the corpus has a records folder
/// decides how the layer and the corrections are read (`lasting` below).
/// Without one, nothing of lasting events is applied.
fn read_beside(root: &Path) -> Result<Beside> {
    let mut h = Sha1::new();
    let records = crate::events::read_hashed(root, &mut h)?;
    let lasting = records.is_some();
    let Some(rows) = read_tag_rows_hashed(root, lasting, &mut h)? else {
        // No tags: the records are still read, hashed and (by `refresh`)
        // stored, so `scout events --min-items 0` lists them.
        let hash = match records {
            Some(_) => hex(h),
            None => NO_HASH.into(),
        };
        return Ok(Beside {
            layer: None,
            records_alone: records,
            hash,
        });
    };
    let mut members = rows.members;
    let mut out = Layer {
        layer_path: root.join(LAYER_FILE),
        items: rows.items,
        bad_lines: rows.bad_lines,
        corrections: 0,
        corrections_unmatched: 0,
        corrections_bad_lines: 0,
        events: None,
    };
    h.update([0u8]);
    let (mut applied, mut unknown, mut bad) = (0, 0, 0);
    stream_lines(
        &root.join(CORRECTIONS_FILE),
        MAX_LINE_BYTES,
        &mut h,
        |l| match l {
            Line::Text(t) => match apply_correction(&mut out.items, &mut members, lasting, t) {
                Correction::Applied => applied += 1,
                Correction::UnknownKey => unknown += 1,
                Correction::Bad => bad += 1,
            },
            Line::Bad => bad += 1,
        },
    )?;
    out.corrections = applied;
    out.corrections_unmatched = unknown;
    out.corrections_bad_lines = bad;
    fill_event_names(&mut out.items);
    out.events = records;
    if let Some(set) = &out.events {
        resolve_events(&mut out.items, &members, set);
    }
    Ok(Beside {
        layer: Some(out),
        records_alone: None,
        hash: hex(h),
    })
}

/// The tag rows of the layer file, before any correction: what
/// [`read_tag_rows_hashed`] gives.
pub(crate) struct TagRows {
    /// The last valid row of each key, by key.
    pub items: BTreeMap<String, LayerItem>,
    /// Each row's `events` list (kept only when read as `lasting`).
    members: BTreeMap<String, Membership>,
    /// Lines that are not a valid row.
    pub bad_lines: usize,
}

/// The tag rows of the corpus at `root` as an index build has them before
/// corrections; `None` when there is no layer file. For a reader that is
/// not the build (`scout events seed`): the same function the build reads
/// the file with, so both keep the same rows.
pub(crate) fn read_tag_rows(root: &Path) -> Result<Option<TagRows>> {
    read_tag_rows_hashed(root, false, &mut Sha1::new())
}

/// The one reader of the layer file ([`read_beside`], and so every index
/// build, reads it here), feeding `hash` every byte. A line is a row when
/// it is valid UTF-8 within [`MAX_LINE_BYTES`], parses as a JSON object
/// with every known field of its type, and has a non-blank `key`; any
/// other non-blank line is counted in `bad_lines` and changes nothing. A
/// later row replaces an earlier row of the same key, so the last VALID
/// row of a key is the one kept. `lasting`: also keep each row's `events`
/// list (a corpus with a records folder).
fn read_tag_rows_hashed(root: &Path, lasting: bool, hash: &mut Sha1) -> Result<Option<TagRows>> {
    let mut rows = TagRows {
        items: BTreeMap::new(),
        members: BTreeMap::new(),
        bad_lines: 0,
    };
    let found = stream_lines(&root.join(LAYER_FILE), MAX_LINE_BYTES, hash, |l| match l {
        Line::Text(t) => {
            if !add_layer_line(&mut rows.items, &mut rows.members, lasting, t) {
                rows.bad_lines += 1;
            }
        }
        Line::Bad => rows.bad_lines += 1,
    })?;
    Ok(found.then_some(rows))
}

/// Parse one layer line into `items` (a repeated key replaces the earlier
/// line); false when it does not parse or has no key. The row's `events`
/// list is kept only when `lasting` (the corpus has a records folder).
fn add_layer_line(
    items: &mut BTreeMap<String, LayerItem>,
    members: &mut BTreeMap<String, Membership>,
    lasting: bool,
    line: &str,
) -> bool {
    let Ok(l) = serde_json::from_str::<LayerLine>(line) else {
        return false;
    };
    let key = l.key.trim().to_string();
    if key.is_empty() {
        return false;
    }
    // A repeated key replaces the earlier line, its events included.
    members.remove(&key);
    let listed = lasting
        .then(|| serde_json::from_str::<ListedEvents>(line).ok())
        .flatten()
        .and_then(|l| l.events);
    if let Some(serde_json::Value::Array(list)) = listed {
        members.entry(key.clone()).or_default().listed = Some(
            list.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect(),
        );
    }
    let month = month_of(l.date.as_deref(), l.month.as_deref());
    let info = TopicInfo {
        key: key.clone(),
        themes: l.themes.unwrap_or_default(),
        entities: l.entities.unwrap_or_default(),
        event: l.event.filter(|e| !e.trim().is_empty()),
        event_name: l.event_name,
        events: None,
        ai_related: l.ai_related,
        month,
        confidence: l.confidence,
        run_id: l.run_id,
        prompt_version: l.prompt_version,
        corrected: vec![],
    };
    items.insert(
        key,
        LayerItem {
            path: l.path.filter(|p| !p.trim().is_empty()),
            date: l.date,
            info,
        },
    );
    true
}

enum Correction {
    Applied,
    UnknownKey,
    Bad,
}

/// Apply one corrections line. `event_add` / `event_remove` are fields
/// only when `lasting` (the corpus has a records folder); otherwise they
/// are unknown keys like `by` or `note`: not read, not type-checked.
fn apply_correction(
    items: &mut BTreeMap<String, LayerItem>,
    members: &mut BTreeMap<String, Membership>,
    lasting: bool,
    line: &str,
) -> Correction {
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
    let (mut add, mut remove) = (None, None);
    for (field, v) in &m {
        let ok = match field.as_str() {
            "themes" => strings(v).map(|s| themes = Some(s)).is_some(),
            "entities" => strings(v).map(|s| entities = Some(s)).is_some(),
            "event" => opt_str(v).map(|s| event = Some(s)).is_some(),
            "event_name" => opt_str(v).map(|s| event_name = Some(s)).is_some(),
            "event_add" if lasting => strings(v).map(|s| add = Some(s)).is_some(),
            "event_remove" if lasting => strings(v).map(|s| remove = Some(s)).is_some(),
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
    if add.is_some() || remove.is_some() {
        let ops = &mut members.entry(key.to_string()).or_default().ops;
        ops.extend(add.unwrap_or_default().into_iter().map(|e| (e, true)));
        ops.extend(remove.unwrap_or_default().into_iter().map(|e| (e, false)));
        mark("events", t);
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

/// Work out every item's events (see the module notes). Called only for a
/// corpus with a records folder (`set`); every item then gets `events`.
fn resolve_events(
    items: &mut BTreeMap<String, LayerItem>,
    members: &BTreeMap<String, Membership>,
    set: &EventSet,
) {
    let resolve = |id: &str| -> String { set.resolve(id).unwrap_or(id.trim()).to_string() };
    for (key, it) in items.iter_mut() {
        let m = members.get(key);
        let t = &mut it.info;
        let first = t.event.as_deref().map(resolve);
        let listed = m.and_then(|m| m.listed.as_deref()).unwrap_or_default();
        let mut ids = crate::events::dedup(
            first
                .iter()
                .cloned()
                .chain(listed.iter().map(|e| resolve(e))),
        );
        for (id, add) in m.map(|m| m.ops.as_slice()).unwrap_or_default() {
            let id = resolve(id);
            let f = fold(&id);
            if f.is_empty() {
                continue;
            }
            let at = ids.iter().position(|e| fold(e) == f);
            match (add, at) {
                (true, None) => ids.push(id),
                (false, Some(i)) => {
                    ids.remove(i);
                }
                _ => {}
            }
        }
        // A removed event is no event of any item, whoever put it there.
        ids.retain(|e| !set.is_removed(e));
        // Taken out of its tag row's event: `event` no longer names it.
        // (A removed event is not a correction of the item.)
        if let Some(first) = first.as_deref() {
            let f = fold(first);
            if !ids.iter().any(|e| fold(e) == f) {
                t.event = None;
                t.event_name = None;
                if !set.is_removed(first) && !t.corrected.iter().any(|c| c == "event") {
                    t.corrected.push("event".into());
                    t.corrected
                        .sort_by_key(|f| CORRECTABLE.iter().position(|c| c == f));
                }
            }
        }
        t.events = Some(ids);
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
    /// The event records, when the corpus has a records folder
    /// (`analysis/events/`); its warnings name every file left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub events: Option<EventsReport>,
}

pub(crate) const TABLE_SQL: &str = "
    CREATE TABLE IF NOT EXISTS topic_docs (
        rel_path TEXT PRIMARY KEY,
        key TEXT NOT NULL,
        matched_by TEXT NOT NULL,
        info TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS event_records (
        id TEXT PRIMARY KEY,
        record TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS event_aliases (
        alias TEXT PRIMARY KEY,
        id TEXT NOT NULL
    );";

const HASH_KEY: &str = "topic_layer_hash";
const REPORT_KEY: &str = "topic_report";
/// The records report of a corpus that has no layer file.
const EVENTS_REPORT_KEY: &str = "events_report";

/// Copy a layer into the index's `topic_docs`, joined to its documents,
/// and its event records into the index's copy of them. `None` empties
/// both; a build then stores the records of a corpus without a layer file
/// itself ([`refresh`]).
pub fn apply(conn: &Connection, layer: Option<&Layer>) -> Result<Option<TopicReport>> {
    conn.execute("DELETE FROM topic_docs", [])?;
    crate::events::store(conn, layer.and_then(|l| l.events.as_ref()))?;
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
        events: layer.events.as_ref().map(EventSet::report),
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

/// Bring the index's topic copy and its copy of the event records up to
/// date with the files at `root` (build time, inside the build
/// transaction). Re-joins when the layer files' or the records' bytes or
/// the documents (`docs_changed`) changed; otherwise keeps the stored copy
/// and returns its report with `reloaded: false`.
///
/// The records do not depend on the layer: a corpus with a records folder
/// and no layer file has its records stored all the same, and their report
/// is the second value (it is `None` whenever there is a layer, whose
/// report then carries it as `TopicReport::events`).
pub(crate) fn refresh(
    conn: &Connection,
    root: &Path,
    docs_changed: bool,
) -> Result<(Option<TopicReport>, Option<EventsReport>)> {
    let current = layer_hash(root)?;
    if !docs_changed && meta_get(conn, HASH_KEY)?.as_deref() == Some(current.as_str()) {
        let stored: Option<TopicReport> = meta_get(conn, REPORT_KEY)?
            .and_then(|s| serde_json::from_str::<Option<TopicReport>>(&s).ok())
            .flatten();
        let records: Option<EventsReport> = meta_get(conn, EVENTS_REPORT_KEY)?
            .and_then(|s| serde_json::from_str::<Option<EventsReport>>(&s).ok())
            .flatten();
        let stored = stored.map(|mut r| {
            r.reloaded = false;
            r
        });
        return Ok((stored, records));
    }
    // Store the hash of the bytes actually parsed, so a file changed
    // between the check and the read is re-read next build.
    let beside = read_beside(root)?;
    let report = apply(conn, beside.layer.as_ref())?;
    if beside.layer.is_none() {
        crate::events::store(conn, beside.records_alone.as_ref())?;
    }
    let records = beside.records_alone.as_ref().map(EventSet::report);
    meta_set(conn, HASH_KEY, &beside.hash)?;
    meta_set(conn, REPORT_KEY, &serde_json::to_string(&report)?)?;
    meta_set(conn, EVENTS_REPORT_KEY, &serde_json::to_string(&records)?)?;
    Ok((report, records))
}

/// Whether the corpus had a records folder (`analysis/events/`, empty or
/// not) when this index was last built. False for an index built before
/// lasting events.
pub fn has_records_folder(conn: &Connection) -> Result<bool> {
    let with_layer = meta_get(conn, REPORT_KEY)?
        .and_then(|s| serde_json::from_str::<Option<TopicReport>>(&s).ok())
        .flatten()
        .is_some_and(|r| r.events.is_some());
    let alone = meta_get(conn, EVENTS_REPORT_KEY)?
        .and_then(|s| serde_json::from_str::<Option<EventsReport>>(&s).ok())
        .flatten()
        .is_some();
    Ok(with_layer || alone)
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

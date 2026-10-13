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
//! - Numbered corrections: a line whose `id` is a canonical numbered id (`c` and the number in
//!   at least six digits and no more than it needs)
//!   (the lines `scout correct` writes, [`crate::corrections`]) is read as
//!   above, and three things more are read on it and on no other line:
//!   `theme_add` / `theme_remove` and `entity_add` / `entity_remove`
//!   (string lists) patch the item's list as it stands at that line (after
//!   the line's own `themes` / `entities`, additions first: a name the
//!   list has under [`fold`] is not added again; a removal takes out every
//!   name equal under the fold); and `undoes` (a numbered id) makes the
//!   line an undo line. An undo line takes back the earlier numbered
//!   correction of that id and the same `key`: that line is not applied
//!   at all, as if it were not in the file. A numbered line is read only
//!   when it is valid, and what is valid is decided in one place for this
//!   reader and for the writer ([`crate::correction_lines`]: the keys
//!   `scout correct` writes and no other, one change, an id greater than
//!   every earlier numbered id, an `undoes` that names an earlier valid
//!   correction of the same key not yet undone). Any other numbered line
//!   is a bad line: counted, not applied, and not undone by a later line.
//!   A file without a numbered line reads exactly as it did before
//!   numbered lines existed.
//! - Limits: a corrections file over 64 MiB
//!   ([`crate::correction_lines::MAX_FILE_BYTES`], the limit the writer
//!   keeps the file under) is not read at all: no correction is applied
//!   and the layer says so (`Layer::corrections_skipped`). The file is
//!   read line by line, twice; no line is kept once it is read.
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
//! - Entity types: a tag row's optional `entities_typed`
//!   (`[{"name", "type"}]`) types the names of its `entities`; a numbered
//!   `entity_add` correction's `entity_type` types the name it adds.
//!   `TopicInfo::entities` stays the list of names and
//!   `TopicInfo::entity_types` maps a name under [`fold`] to its type.
//! - Month: the item's `date` cut to `YYYY-MM` (the month of its latest
//!   highlight), else the layer's `month` when that is `YYYY-MM`. It is
//!   the month `month:` filters on and the month facet counts; `y:` /
//!   `after:` / `before:` keep filtering on the document's own date.
//!
//! Nothing here writes the archive files (`corrections.jsonl` has one
//! writer, [`crate::corrections`]). The index copy is rebuilt whenever the
//! layer files' bytes or the corpus's documents change.

use crate::correction_lines::{self, numbered, Counts, Kept, LineRead, Opened, Skipped};
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
    /// The type of each entity that has one, by the entity's name under
    /// [`fold`] (`"terence tao": "person"`): from the tag row's
    /// `entities_typed`, and from the `entity_type` of a numbered
    /// `entity_add` correction. `entities` stays a list of names; a name
    /// without a type has no entry. Absent when no entity has a type (an
    /// index built before engine 13).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub entity_types: BTreeMap<String, String>,
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
    /// The tag row's `mentioned_works`: the layer keys of the works of the
    /// archive the item mentions, as written ([`crate::works`]). Absent
    /// when the row names none (an index built before engine 14).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mentioned_works: Vec<String>,
    /// The tag row's `date` as a calendar day (`YYYY-MM-DD`), when it is
    /// one and it is the date that gave `month`. Kept in the index and
    /// shown in no reply as a key of its own: `scout events` gives each
    /// month's first and last day from it.
    #[serde(default, skip_serializing)]
    pub day: Option<String>,
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

    /// The type of the entity `name` on this item, when it has one.
    pub fn entity_type(&self, name: &str) -> Option<&str> {
        self.entity_types.get(&fold(name)).map(String::as_str)
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

    /// The month the item is counted in: the one assignment of an item
    /// to a month. `month:` filters on it ([`TopicInfo::in_month`]), the
    /// month facet counts it and `theme-months` puts the item in its
    /// column.
    pub fn layer_month(&self) -> Option<&str> {
        self.month.as_deref()
    }

    /// `month` is `YYYY-MM` (that month) or `YYYY` (any month of the year).
    pub fn in_month(&self, month: &str) -> bool {
        let m = month.trim();
        self.layer_month()
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

/// Is `v` exactly a month `YYYY-MM`: seven bytes, a real month, nothing
/// around it? (A bare year and a padded value are not.)
pub fn exact_month(v: &str) -> bool {
    v.len() == 7 && v.trim().len() == 7 && valid_month(v)
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

/// The day of a tag row's `date`: its first ten bytes when they are a
/// calendar date of `month` (the month [`month_of`] took from it).
fn day_of(date: Option<&str>, month: Option<&str>) -> Option<String> {
    let day = date?.trim().get(..10)?;
    (crate::events::is_date(day) && month == day.get(..7)).then(|| day.to_string())
}

/// A document's tags as the index stores them: [`TopicInfo`] as every
/// reply writes it, plus the `day` no reply shows.
#[derive(Serialize)]
struct StoredInfo<'a> {
    #[serde(flatten)]
    info: &'a TopicInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    day: &'a Option<String>,
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

/// A tag row's `entities_typed`, read from the line a second time and
/// apart from [`LayerLine`], which does not name the key: whether a line
/// is a row is decided without it, exactly as before types were read
/// (the key is skipped unread there, like any unknown key). This read may
/// fail (a number out of range or nesting past serde_json's depth inside
/// the value, the key written twice); the row then has no types and
/// stands.
#[derive(Deserialize)]
struct TypedEntities {
    entities_typed: Option<serde_json::Value>,
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
#[derive(Debug, Default, Clone)]
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
    /// Numbered correction lines an undo line took back. Neither they nor
    /// the undo lines count in `corrections`.
    pub corrections_undone: usize,
    /// Set when the corrections file was not read at all (it is over the
    /// size limit): then no correction is applied and every count above
    /// is 0.
    pub corrections_skipped: Option<Skipped>,
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
/// Only a regular file is read ([`correction_lines::open_regular`]):
/// anything else at `path` is an error. There is no bound on the size of
/// the file, only on a line.
fn stream_lines(
    path: &Path,
    cap: usize,
    hash: &mut Sha1,
    each: impl FnMut(Line<'_>),
) -> Result<bool> {
    let file = match correction_lines::open_regular(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e).with_context(|| format!("read {}", path.display())),
    };
    let r = std::io::BufReader::with_capacity(64 * 1024, file);
    split_lines(r, cap, hash, each).with_context(|| format!("read {}", path.display()))?;
    Ok(true)
}

/// [`stream_lines`] over any source of bytes: the one place a layer or
/// corrections file is cut into lines.
fn split_lines(
    mut r: impl std::io::BufRead,
    cap: usize,
    hash: &mut Sha1,
    mut each: impl FnMut(Line<'_>),
) -> std::io::Result<()> {
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
        let chunk = r.fill_buf()?;
        if chunk.is_empty() {
            if !buf.is_empty() || over {
                emit(&mut buf, &mut over);
            }
            return Ok(());
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
        let path = root.join(CORRECTIONS_FILE);
        match correction_lines::open(&path, correction_lines::MAX_FILE_BYTES)? {
            Opened::Missing => {}
            Opened::TooLarge(skipped) => correction_lines::hash_skipped(&mut h, &skipped),
            Opened::File(mut file, len) => split_lines(
                correction_lines::rewound(&mut file, len)?,
                0,
                &mut h,
                |_| {},
            )
            .with_context(|| format!("read {}", path.display()))?,
        }
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
    let Some(mut rows) = read_tag_rows_hashed(root, lasting, &mut h)? else {
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
    h.update([0u8]);
    let read = read_corrections_file(
        &root.join(CORRECTIONS_FILE),
        correction_lines::MAX_FILE_BYTES,
        &mut rows.items,
        &mut rows.members,
        lasting,
        &mut h,
    )?;
    finish(&mut rows.items, &rows.members, records.as_ref());
    Ok(Beside {
        layer: Some(Layer {
            layer_path: root.join(LAYER_FILE),
            items: rows.items,
            bad_lines: rows.bad_lines,
            corrections: read.counts.applied,
            corrections_unmatched: read.counts.unknown_key,
            corrections_bad_lines: read.counts.bad,
            corrections_undone: read.counts.undone,
            corrections_skipped: read.skipped,
            events: records,
        }),
        records_alone: None,
        hash: hex(h),
    })
}

/// What reading a corrections file gave.
struct CorrectionsRead {
    /// What its lines were read as.
    counts: Counts,
    /// What was kept in memory while they went past.
    #[cfg_attr(not(test), allow(dead_code))]
    kept: Kept,
    /// Set when the file was not read (over `limit`).
    skipped: Option<Skipped>,
}

/// Apply the corrections file at `path` to `items`, feeding `hash` every
/// byte read. A file over `limit` bytes is not read: nothing is applied,
/// `skipped` says why, and `hash` takes its size instead of its bytes.
/// The file is streamed twice ([`correction_lines::read`]), the same
/// bytes both times or an error; no line is kept in memory.
fn read_corrections_file(
    path: &Path,
    limit: u64,
    items: &mut BTreeMap<String, LayerItem>,
    members: &mut BTreeMap<String, Membership>,
    lasting: bool,
    hash: &mut Sha1,
) -> Result<CorrectionsRead> {
    let mut out = CorrectionsRead {
        counts: Counts::default(),
        kept: Kept::default(),
        skipped: None,
    };
    let (mut file, len) = match correction_lines::open(path, limit)? {
        Opened::Missing => return Ok(out),
        Opened::TooLarge(skipped) => {
            correction_lines::hash_skipped(hash, &skipped);
            out.skipped = Some(skipped);
            return Ok(out);
        }
        Opened::File(file, len) => (file, len),
    };
    let mut passes: Vec<Sha1> = Vec::new();
    out.kept = correction_lines::read(
        |each| -> std::io::Result<()> {
            let mut h = hash.clone();
            let r = correction_lines::rewound(&mut file, len)?;
            split_lines(r, MAX_LINE_BYTES, &mut h, |l| match l {
                Line::Text(t) => each(Some(t)),
                Line::Bad => each(None),
            })?;
            passes.push(h);
            Ok(())
        },
        |m| apply_correction(items, members, lasting, m),
        |r| out.counts.add(r),
    )
    .with_context(|| format!("read {}", path.display()))?;
    correction_lines::same_bytes(path, &passes[0], &passes[1])?;
    *hash = passes.swap_remove(1);
    Ok(out)
}

/// What every reader of the layer does once the corrections are applied:
/// the event names filled and, with records, every item's events
/// resolved.
fn finish(
    items: &mut BTreeMap<String, LayerItem>,
    members: &BTreeMap<String, Membership>,
    records: Option<&EventSet>,
) {
    fill_event_names(items);
    if let Some(set) = records {
        resolve_events(items, members, set);
    }
}

/// The tag rows and the event records of one corpus, read once, for the
/// writer of the corrections file ([`crate::corrections`]): it asks what
/// the reader makes of a set of correction lines, with the reader's own
/// code, before it writes one.
pub(crate) struct TagSource {
    rows: TagRows,
    pub records: Option<EventSet>,
}

impl TagSource {
    /// Read the records folder and the layer file of the corpus at
    /// `root`, as [`read_layer`] reads them; `None` without a layer file.
    pub(crate) fn read(root: &Path) -> Result<Option<TagSource>> {
        let mut h = Sha1::new();
        let records = crate::events::read_hashed(root, &mut h)?;
        let rows = read_tag_rows_hashed(root, records.is_some(), &mut h)?;
        Ok(rows.map(|rows| TagSource { rows, records }))
    }

    /// Every item's tags with `text` as the corrections file, and what
    /// each of its non-blank lines was read as. `text` is cut into lines
    /// by the function that cuts the file ([`split_lines`]).
    pub(crate) fn with_corrections(
        &self,
        text: &[u8],
    ) -> (BTreeMap<String, LayerItem>, Vec<LineRead>) {
        let mut items = self.rows.items.clone();
        let mut members = self.rows.members.clone();
        let lasting = self.records.is_some();
        let mut read = Vec::new();
        // Reading from memory does not fail.
        let _ = correction_lines::read(
            |each| {
                split_lines(text, MAX_LINE_BYTES, &mut Sha1::new(), |l| match l {
                    Line::Text(t) => each(Some(t)),
                    Line::Bad => each(None),
                })
            },
            |m| apply_correction(&mut items, &mut members, lasting, m),
            |r| read.push(r),
        );
        finish(&mut items, &members, self.records.as_ref());
        (items, read)
    }
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
    let entities = l.entities.unwrap_or_default();
    // No part of this read can make the line a bad line: the row is
    // already accepted.
    let typed = (!entities.is_empty())
        .then(|| serde_json::from_str::<TypedEntities>(line).ok())
        .flatten()
        .and_then(|t| t.entities_typed);
    let entity_types = typed_entities(typed.as_ref(), &entities);
    let day = day_of(l.date.as_deref(), month.as_deref());
    let info = TopicInfo {
        key: key.clone(),
        themes: l.themes.unwrap_or_default(),
        entities,
        entity_types,
        event: l.event.filter(|e| !e.trim().is_empty()),
        event_name: l.event_name,
        events: None,
        ai_related: l.ai_related,
        month,
        confidence: l.confidence,
        run_id: l.run_id,
        prompt_version: l.prompt_version,
        // Like the types: read apart, after the row is accepted.
        mentioned_works: crate::works::read_mentioned(line),
        day,
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

/// The types a tag row's `entities_typed` gives the names of its
/// `entities`: each `{"name", "type"}` object with a non-blank text for
/// both, whose name is one of `entities` under [`fold`]; the type trimmed,
/// as written. A later object of the same name wins. Each item is judged
/// alone: an item of another shape (not an object, a `name` or `type`
/// that is not text or is blank, a name the row does not list) gives no
/// type and the well-formed items beside it still count. A value that is
/// not a list gives none.
fn typed_entities(
    typed: Option<&serde_json::Value>,
    entities: &[String],
) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Some(list) = typed.and_then(|v| v.as_array()) else {
        return out;
    };
    let names: BTreeSet<String> = entities.iter().map(|e| fold(e)).collect();
    for item in list {
        let text = |k: &str| item.get(k).and_then(|v| v.as_str()).map(str::trim);
        if let (Some(name), Some(kind)) = (text("name"), text("type")) {
            let f = fold(name);
            if !f.is_empty() && !kind.is_empty() && names.contains(&f) {
                out.insert(f, kind.to_string());
            }
        }
    }
    out
}

/// Add to and take from a list of names under [`fold`]: additions first
/// (a name the list has is not added again), then removals (every name
/// equal under the fold).
fn patch_names(list: &mut Vec<String>, add: &[String], remove: &[String]) {
    for a in add {
        let f = fold(a);
        if !f.is_empty() && !list.iter().any(|have| fold(have) == f) {
            list.push(a.trim().to_string());
        }
    }
    for r in remove {
        let f = fold(r);
        list.retain(|have| fold(have) != f);
    }
}

/// Apply one corrections line, already parsed: a hand-written line, or a
/// numbered correction [`correction_lines`] found valid and not undone
/// (its only caller is [`correction_lines::read`]). `event_add` /
/// `event_remove` are fields only when `lasting` (the corpus has a records
/// folder), and `theme_add` / `theme_remove` / `entity_add` /
/// `entity_remove` only on a numbered line; otherwise they are unknown
/// keys like `by` or `note`: not read, not type-checked.
fn apply_correction(
    items: &mut BTreeMap<String, LayerItem>,
    members: &mut BTreeMap<String, Membership>,
    lasting: bool,
    m: &serde_json::Map<String, serde_json::Value>,
) -> LineRead {
    let patch = numbered(m).is_some();
    let Some(key) = m.get("key").and_then(|k| k.as_str()).map(str::trim) else {
        return LineRead::Bad;
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
    let (mut theme_add, mut theme_remove) = (None, None);
    let (mut entity_add, mut entity_remove) = (None, None);
    for (field, v) in m {
        let ok = match field.as_str() {
            "themes" => strings(v).map(|s| themes = Some(s)).is_some(),
            "entities" => strings(v).map(|s| entities = Some(s)).is_some(),
            "event" => opt_str(v).map(|s| event = Some(s)).is_some(),
            "event_name" => opt_str(v).map(|s| event_name = Some(s)).is_some(),
            "event_add" if lasting => strings(v).map(|s| add = Some(s)).is_some(),
            "event_remove" if lasting => strings(v).map(|s| remove = Some(s)).is_some(),
            "theme_add" if patch => strings(v).map(|s| theme_add = Some(s)).is_some(),
            "theme_remove" if patch => strings(v).map(|s| theme_remove = Some(s)).is_some(),
            "entity_add" if patch => strings(v).map(|s| entity_add = Some(s)).is_some(),
            "entity_remove" if patch => strings(v).map(|s| entity_remove = Some(s)).is_some(),
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
            return LineRead::Bad;
        }
    }
    let Some(item) = items.get_mut(key) else {
        return LineRead::UnknownKey;
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
    if theme_add.is_some() || theme_remove.is_some() {
        patch_names(
            &mut t.themes,
            &theme_add.unwrap_or_default(),
            &theme_remove.unwrap_or_default(),
        );
        mark("themes", t);
    }
    if let Some(v) = entities {
        // A whole list: the names it keeps keep their types.
        let kept: BTreeSet<String> = v.iter().map(|n| fold(n)).collect();
        t.entity_types.retain(|name, _| kept.contains(name));
        t.entities = v;
        mark("entities", t);
    }
    if entity_add.is_some() || entity_remove.is_some() {
        let (add, remove) = (
            entity_add.unwrap_or_default(),
            entity_remove.unwrap_or_default(),
        );
        patch_names(&mut t.entities, &add, &remove);
        // The line's `entity_type` (a numbered line's, checked by
        // `correction_lines`) is the type of the name it adds, also when
        // the row has the name by now. A name taken off has no type.
        if let Some(kind) = m.get("entity_type").and_then(|v| v.as_str()) {
            for name in add.iter().map(|a| fold(a)).filter(|f| !f.is_empty()) {
                t.entity_types.insert(name, kind.to_string());
            }
        }
        for name in &remove {
            t.entity_types.remove(&fold(name));
        }
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
    LineRead::Applied
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

/// The events one item is in, under the records of `set`: the one
/// statement of it (see "Events" in the module notes). `event` is the
/// item's `event` with the corrections applied, `m` what its tag row
/// lists and the corrections add and remove. Returns the event `event`
/// resolves to, and every event of the item, in order.
///
/// Every id is resolved through `set` as it is met, the ids of the
/// corrections too: an `event_add` / `event_remove` names the event its
/// id resolves to in `set`, whatever it resolved to when the line was
/// written. A removed event is no event of any item.
fn item_events(
    event: Option<&str>,
    m: Option<&Membership>,
    set: &EventSet,
) -> (Option<String>, Vec<String>) {
    let resolve = |id: &str| -> String { set.resolve(id).unwrap_or(id.trim()).to_string() };
    let first = event.map(resolve);
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
    (first, ids)
}

/// Work out every item's events ([`item_events`]). Called only for a
/// corpus with a records folder (`set`); every item then gets `events`.
fn resolve_events(
    items: &mut BTreeMap<String, LayerItem>,
    members: &BTreeMap<String, Membership>,
    set: &EventSet,
) {
    for (key, it) in items.iter_mut() {
        let t = &mut it.info;
        let (first, ids) = item_events(t.event.as_deref(), members.get(key), set);
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

/// Which events every tag row of a corpus is in, for a reader that is not
/// the build and asks it of records that are not the folder's as it is
/// (`scout events unmerge-preview`: the records as an undo would leave
/// them). The tag rows and the corrections are read by the build's own
/// functions ([`read_tag_rows_hashed`], [`read_corrections_file`]), as
/// the build reads them in a corpus with a records folder, and each
/// item's events are worked out by [`item_events`], the function the
/// build stores them from. So what this says of a set of records is what
/// an index build over those records stores.
pub(crate) struct EventMembers {
    /// The last valid row of each key, corrections applied, events not
    /// yet resolved.
    items: BTreeMap<String, LayerItem>,
    members: BTreeMap<String, Membership>,
}

impl EventMembers {
    /// Read `analysis/topics/current.jsonl` and `corrections.jsonl` of
    /// the corpus at `root`: the layer file streamed once, the
    /// corrections file by its reader (line by line, twice). No other
    /// file is opened. A corpus without a layer file has no items.
    pub(crate) fn read(root: &Path) -> Result<EventMembers> {
        let mut h = Sha1::new();
        let Some(mut rows) = read_tag_rows_hashed(root, true, &mut h)? else {
            return Ok(EventMembers {
                items: BTreeMap::new(),
                members: BTreeMap::new(),
            });
        };
        read_corrections_file(
            &root.join(CORRECTIONS_FILE),
            correction_lines::MAX_FILE_BYTES,
            &mut rows.items,
            &mut rows.members,
            true,
            &mut h,
        )?;
        Ok(EventMembers {
            items: rows.items,
            members: rows.members,
        })
    }

    /// The events of each item under the records of `set`, items in key
    /// order (the same order for every `set`).
    pub(crate) fn under<'a>(&'a self, set: &'a EventSet) -> impl Iterator<Item = Vec<String>> + 'a {
        self.items.iter().map(move |(key, it)| {
            item_events(it.info.event.as_deref(), self.members.get(key), set).1
        })
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
    /// Numbered correction lines an undo line took back (present when not
    /// 0).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub corrections_undone: usize,
    /// Present when the corrections file was NOT READ (it is over the
    /// size limit): no correction was applied by this build. Says why,
    /// with the file's size and the limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corrections_skipped: Option<Skipped>,
    /// How many things about the layer files need his attention (present
    /// when not 0): 1 for a corrections file that was not read.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub warnings: usize,
    /// False when the layer and the documents were unchanged since the last
    /// build, so the stored copy was kept.
    pub reloaded: bool,
    /// The event records, when the corpus has a records folder
    /// (`analysis/events/`); its warnings name every file left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub events: Option<EventsReport>,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
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

/// Layer key → the documents that have it, and every document key.
pub(crate) type KeyedDocuments = (HashMap<String, Vec<String>>, BTreeSet<String>);

/// The documents of an index by layer key (`<source>:<source_id>`, each
/// trimmed), and every document key. The one statement of which document
/// a layer key names: the join of tag rows ([`apply`]) and the look-up of
/// a mentioned work ([`crate::works`]) both read it here.
pub(crate) fn documents_by_source_key(conn: &Connection) -> Result<KeyedDocuments> {
    let mut by_key: HashMap<String, Vec<String>> = HashMap::new();
    let mut paths: BTreeSet<String> = BTreeSet::new();
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
    Ok((by_key, paths))
}

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
    let (by_key, paths) = documents_by_source_key(conn)?;
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
        let stored = StoredInfo {
            info,
            day: &info.day,
        };
        ins.execute(params![rel, key, how, serde_json::to_string(&stored)?])?;
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
        corrections_undone: layer.corrections_undone,
        corrections_skipped: layer.corrections_skipped.clone(),
        warnings: usize::from(layer.corrections_skipped.is_some()),
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

    /// A tag row as it was read before `entities_typed` was: the fields
    /// of [`LayerLine`] on `main`, to which `entities_typed` is an
    /// unknown key.
    #[derive(Deserialize)]
    #[allow(dead_code)]
    struct RowBeforeTypes {
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

    fn one_row(line: &str) -> Option<TopicInfo> {
        let (mut items, mut members) = (BTreeMap::new(), BTreeMap::new());
        let ok = add_layer_line(&mut items, &mut members, false, line);
        assert_eq!(ok, items.len() == 1, "{line}");
        items.into_values().next().map(|i| i.info)
    }

    /// Whatever `entities_typed` holds, a line is a row exactly when it
    /// was one before the key was read, with the same themes, entities
    /// and event; the value only ever decides the types.
    #[test]
    fn an_odd_entities_typed_never_decides_whether_a_line_is_a_row() {
        let deep = |open: &str, close: &str| format!("{}{}", open.repeat(150), close.repeat(150));
        let tao = r#"{"name":"Tao","type":"person"}"#;
        // (the value as written, the types it gives the good row)
        let person = vec![("tao", "person")];
        let values: Vec<(String, Vec<(&str, &str)>)> = vec![
            // Not readable as a value of the field: no types.
            ("1e400".into(), vec![]),
            ("-1e400".into(), vec![]),
            (deep("[", "]"), vec![]),
            (
                deep(r#"{"a":"#, "}").replace(r#"{"a":}"#, r#"{"a":1}"#),
                vec![],
            ),
            (format!("[{tao},1e400]"), vec![]),
            (format!("[{tao},{}]", deep("[", "]")), vec![]),
            (
                format!(
                    r#"[{{"name":"Tao","type":"person","x":{}}}]"#,
                    deep("[", "]")
                ),
                vec![],
            ),
            // (Half a surrogate pair: skipped unread it passes, read it
            // does not.)
            (r#""\ud800""#.into(), vec![]),
            // Written twice: no types.
            (format!(r#"1,"entities_typed":[{tao}]"#), vec![]),
            (format!(r#"[{tao}],"entities_typed":[{tao}]"#), vec![]),
            // Not a list: no types.
            ("null".into(), vec![]),
            ("true".into(), vec![]),
            ("7".into(), vec![]),
            ("-0.5e-3".into(), vec![]),
            (r#""person""#.into(), vec![]),
            ("{}".into(), vec![]),
            (tao.into(), vec![]),
            // A list: each item alone.
            ("[]".into(), vec![]),
            (r#"[1,"x",null,[],{}]"#.into(), vec![]),
            (r#"[{"name":"Tao"}]"#.into(), vec![]),
            (r#"[{"type":"person"}]"#.into(), vec![]),
            (r#"[{"name":"Tao","type":7}]"#.into(), vec![]),
            (r#"[{"name":7,"type":"person"}]"#.into(), vec![]),
            (r#"[{"name":"Tao","type":" "}]"#.into(), vec![]),
            (r#"[{"name":"Nobody","type":"person"}]"#.into(), vec![]),
            (format!("[{tao}]"), person.clone()),
            (
                format!(r#"[7,{tao},{{"name":"Astra","type":null}},"x"]"#),
                person.clone(),
            ),
            (
                format!(r#"[{tao},{{"name":"ASTRA","type":" model "}},{{"name":"Astra"}}]"#),
                vec![("astra", "model"), ("tao", "person")],
            ),
            (
                r#"[{"name":"Tao","type":"org"},{"name":"tao","type":"person"}]"#.into(),
                person,
            ),
            // Not JSON: the line was never a row.
            ("[".into(), vec![]),
            ("01".into(), vec![]),
            ("1e".into(), vec![]),
            ("".into(), vec![]),
        ];
        // (the rest of the line, is it a row without the key)
        let rests = [
            (
                r#""key":"x:1","path":"a.md","themes":["Maths"],"entities":["Tao","Astra"],"event":"2026-02/E01""#,
                true,
            ),
            (
                r#""key":"x:1","themes":["Maths"],"entities":["Tao","Astra"],"event":null,"other":1e400"#,
                true,
            ),
            (r#""key":"x:1","themes":["Maths"]"#, true),
            (r#""key":"x:1","themes":"Maths","entities":["Tao"]"#, false),
            (r#""key":" ","themes":["Maths"],"entities":["Tao"]"#, false),
            (r#""themes":["Maths"],"entities":["Tao"]"#, false),
        ];
        let mut rows = 0;
        for (rest, is_row) in rests {
            let bare = one_row(&format!("{{{rest}}}"));
            assert_eq!(bare.is_some(), is_row, "{rest}");
            for (value, types) in &values {
                for line in [
                    format!(r#"{{{rest},"entities_typed":{value}}}"#),
                    format!(r#"{{"entities_typed":{value},{rest}}}"#),
                ] {
                    let got = one_row(&line);
                    // As the reader before types: the same lines are rows.
                    let before = serde_json::from_str::<RowBeforeTypes>(&line)
                        .is_ok_and(|r| !r.key.trim().is_empty());
                    assert_eq!(got.is_some(), before, "{line}");
                    let Some(got) = got else { continue };
                    rows += 1;
                    // And the row is the row without the key, types apart.
                    let bare = bare.clone().expect(&line);
                    assert_eq!(
                        (
                            &got.themes,
                            &got.entities,
                            &got.event,
                            &got.month,
                            got.ai_related
                        ),
                        (
                            &bare.themes,
                            &bare.entities,
                            &bare.event,
                            &bare.month,
                            bare.ai_related
                        ),
                        "{line}"
                    );
                    let want: BTreeMap<String, String> = if bare.entities.len() == 2 {
                        types
                            .iter()
                            .map(|(n, t)| (n.to_string(), t.to_string()))
                            .collect()
                    } else {
                        BTreeMap::new()
                    };
                    assert_eq!(got.entity_types, want, "{line}");
                    assert!(bare.entity_types.is_empty());
                }
            }
        }
        // Three rests are rows, with each of the 29 values the reader
        // before types passed over, in both places.
        assert_eq!(rows, 3 * 29 * 2);
    }

    /// Whatever `mentioned_works` holds, a line is a row exactly when it
    /// was one before the key was read, with the same tags; the value
    /// only ever decides the works.
    #[test]
    fn an_odd_mentioned_works_never_decides_whether_a_line_is_a_row() {
        let deep = |open: &str, close: &str| format!("{}{}", open.repeat(150), close.repeat(150));
        // (the value as written, the works it gives a row)
        let values: Vec<(String, Vec<&str>)> = vec![
            (r#"["rw:1","x:2"]"#.into(), vec!["rw:1", "x:2"]),
            (r#"[" rw:1 ","RW:1"]"#.into(), vec!["rw:1"]),
            ("[]".into(), vec![]),
            // Items of another shape are passed over, one by one.
            (
                r#"[1,null,true,"rw:1",{"key":"x:2"},["x:3"],""]"#.into(),
                vec!["rw:1"],
            ),
            (r#"[{"key":"rw:1","title":"T"}]"#.into(), vec![]),
            // Not a list: none.
            ("null".into(), vec![]),
            ("7".into(), vec![]),
            (r#""rw:1""#.into(), vec![]),
            (r#"{"rw:1":1}"#.into(), vec![]),
            // Not readable as a value of the field: none.
            ("1e400".into(), vec![]),
            (r#"["rw:1",1e400]"#.into(), vec![]),
            (deep("[", "]"), vec![]),
            (format!(r#"["rw:1",{}]"#, deep("[", "]")), vec![]),
            // The key a second time: none.
            (r#"["rw:1"],"mentioned_works":["x:2"]"#.into(), vec![]),
            (r#"1,"mentioned_works":["x:2"]"#.into(), vec![]),
            // Not JSON: the line was never a row.
            ("[".into(), vec![]),
            ("01".into(), vec![]),
            ("".into(), vec![]),
        ];
        let rests = [
            (
                r#""key":"x:1","path":"a.md","date":"2026-02-03","themes":["Maths"],"entities":["Tao"],"entities_typed":[{"name":"Tao","type":"person"}],"event":"2026-02/E01""#,
                true,
            ),
            (r#""key":"x:1","themes":["Maths"],"other":1e400"#, true),
            (r#""key":"x:1","themes":"Maths""#, false),
            (r#""key":" ","themes":["Maths"]"#, false),
            (r#""themes":["Maths"]"#, false),
        ];
        let mut rows = 0;
        for (rest, is_row) in rests {
            let bare = one_row(&format!("{{{rest}}}"));
            assert_eq!(bare.is_some(), is_row, "{rest}");
            for (value, works) in &values {
                for line in [
                    format!(r#"{{{rest},"mentioned_works":{value}}}"#),
                    format!(r#"{{"mentioned_works":{value},{rest}}}"#),
                ] {
                    let got = one_row(&line);
                    // As the reader before the field: the same lines are rows.
                    let before = serde_json::from_str::<RowBeforeTypes>(&line)
                        .is_ok_and(|r| !r.key.trim().is_empty());
                    assert_eq!(got.is_some(), before, "{line}");
                    let Some(got) = got else { continue };
                    rows += 1;
                    // And the row is the row without the key, works apart.
                    let bare = bare.clone().expect(&line);
                    assert!(bare.mentioned_works.is_empty());
                    assert_eq!(got.mentioned_works, *works, "{line}");
                    assert_eq!(
                        got,
                        TopicInfo {
                            mentioned_works: got.mentioned_works.clone(),
                            ..bare
                        },
                        "{line}"
                    );
                }
            }
        }
        // Two rests are rows, with each of the 15 values the reader
        // before the field passed over, in both places.
        assert_eq!(rows, 2 * 15 * 2);
    }

    #[test]
    fn a_row_has_a_day_when_its_date_is_one_and_gave_the_month() {
        let day = |rest: &str| one_row(&format!(r#"{{"key":"x:1",{rest}}}"#)).unwrap().day;
        assert_eq!(day(r#""date":"2026-09-06""#).as_deref(), Some("2026-09-06"));
        assert_eq!(
            day(r#""date":"2026-09-06T10:00:00Z""#).as_deref(),
            Some("2026-09-06")
        );
        // A date that is no day gives the month and no day; the layer's
        // own month gives none.
        for none in [
            r#""date":"2026-09""#,
            r#""date":"2026-09-31""#,
            r#""month":"2026-09""#,
            r#""date":"soon","month":"2026-09""#,
            r#""themes":[]"#,
        ] {
            assert_eq!(day(none), None, "{none}");
        }
        // The day is kept in the index's copy and written in no reply.
        let row = one_row(r#"{"key":"x:1","date":"2026-09-06"}"#).unwrap();
        let shown = serde_json::to_string(&row).unwrap();
        assert!(
            !shown.contains("day") && !shown.contains("mentioned_works"),
            "{shown}"
        );
        let stored = serde_json::to_string(&StoredInfo {
            info: &row,
            day: &row.day,
        })
        .unwrap();
        assert_eq!(serde_json::from_str::<TopicInfo>(&stored).unwrap(), row);
        assert_eq!(serde_json::from_str::<TopicInfo>(&shown).unwrap().day, None);
    }

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
        assert!(exact_month("2026-09"));
        for bad in [
            "2026",
            "2026   ",
            "   2026",
            " 2026  ",
            "2026-9",
            "2026-13",
            "2026-00",
            " 2026-09",
            "2026-09 ",
            "2026\u{a0}\u{a0}",
            "",
        ] {
            assert!(!exact_month(bad), "{bad:?}");
        }
        assert!(parse_ai("Yes").unwrap());
        assert!(!parse_ai("false").unwrap());
        assert!(parse_ai("maybe").is_err());
    }

    fn item(key: &str) -> (String, LayerItem) {
        let info = TopicInfo {
            key: key.to_string(),
            ..TopicInfo::default()
        };
        let it = LayerItem {
            path: None,
            date: None,
            info,
        };
        (key.to_string(), it)
    }

    fn read_file(path: &Path, limit: u64) -> (BTreeMap<String, LayerItem>, CorrectionsRead) {
        let mut items = BTreeMap::from([item("x:1")]);
        let mut members = BTreeMap::new();
        let read = read_corrections_file(
            path,
            limit,
            &mut items,
            &mut members,
            false,
            &mut Sha1::new(),
        )
        .unwrap();
        (items, read)
    }

    /// A million lines that are not corrections, with four numbered lines
    /// among them: every line is counted, and what the reader kept in
    /// memory is the four numbered lines' worth, not the million.
    #[test]
    fn a_million_lines_that_are_not_corrections_are_counted_and_not_kept() {
        use std::io::Write;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("corrections.jsonl");
        let numbered = |n: u32, change: &str| {
            format!(
                "{{\"id\":\"c{n:06}\",\"at\":\"t\",\"date\":\"d\",\"by\":\"dominik\",\"key\":\"x:1\",{change}}}\n"
            )
        };
        {
            let mut f = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
            let junk = "{}\n".repeat(1000);
            for block in 0..1000 {
                f.write_all(junk.as_bytes()).unwrap();
                match block {
                    10 => f.write_all(numbered(1, "\"theme_add\":[\"A\"]").as_bytes()),
                    20 => f.write_all(numbered(2, "\"theme_add\":[\"B\"]").as_bytes()),
                    30 => f.write_all(
                        numbered(3, "\"entity_add\":[\"E\"],\"entity_type\":\"person\"").as_bytes(),
                    ),
                    40 => f.write_all(numbered(4, "\"undoes\":\"c000001\"").as_bytes()),
                    _ => Ok(()),
                }
                .unwrap();
            }
            f.flush().unwrap();
        }
        let (items, read) = read_file(&path, correction_lines::MAX_FILE_BYTES);
        assert_eq!(
            read.counts,
            Counts {
                applied: 2,
                unknown_key: 0,
                bad: 1_000_000,
                undo: 1,
                undone: 1,
            }
        );
        assert!(read.skipped.is_none());
        // The whole of what was kept while 1,000,004 lines went past.
        assert_eq!(
            read.kept,
            Kept {
                corrections: 3,
                undone: 1
            }
        );
        assert_eq!(items["x:1"].info.themes, ["B"]);
        assert_eq!(items["x:1"].info.entities, ["E"]);

        // Only lines that are not corrections: nothing is kept at all.
        std::fs::write(&path, "{}\n".repeat(1_000_000)).unwrap();
        let (items, read) = read_file(&path, correction_lines::MAX_FILE_BYTES);
        assert_eq!(read.counts.bad, 1_000_000);
        assert_eq!(read.counts.applied + read.counts.unknown_key, 0);
        assert_eq!(read.kept, Kept::default());
        assert!(items["x:1"].info.corrected.is_empty());
    }

    /// The reader's limit is the writer's, one constant; a file one byte
    /// over it is not read, and one at it is.
    #[test]
    fn a_corrections_file_over_the_limit_is_not_read_and_one_at_it_is() {
        assert_eq!(
            correction_lines::MAX_FILE_BYTES,
            crate::corrections::MAX_FILE_BYTES
        );
        assert_eq!(correction_lines::MAX_FILE_BYTES, 64 << 20);
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("corrections.jsonl");
        let line = "{\"key\":\"x:1\",\"themes\":[\"Fixed\"]}\n";
        std::fs::write(&path, line).unwrap();
        let len = line.len() as u64;
        let (items, read) = read_file(&path, len);
        assert_eq!((read.counts.applied, &read.skipped), (1, &None));
        assert_eq!(items["x:1"].info.themes, ["Fixed"]);
        let (items, read) = read_file(&path, len - 1);
        assert_eq!(read.counts, Counts::default());
        assert!(items["x:1"].info.themes.is_empty());
        let skipped = read.skipped.unwrap();
        assert_eq!((skipped.bytes, skipped.limit), (len, len - 1));
        assert!(
            skipped.reason.contains(&format!("{len} bytes"))
                && skipped.reason.contains("NO correction was applied"),
            "{}",
            skipped.reason
        );
    }
}

//! `scout correct …`: the one writer of
//! `<archive>/analysis/topics/corrections.jsonl`, the file his fixes to an
//! item's tags are kept in, apart from the model's rows
//! (`current.jsonl`), which are never written.
//!
//! What the reader does with the file ([`crate::topics`], its only
//! reader; this module parses no tag row and applies no correction
//! itself):
//!
//! 1. A line is a JSON object with a `key` and any of `themes`,
//!    `entities`, `event`, `event_name`, `ai_related`, and, in a corpus
//!    with a records folder, `event_add` / `event_remove`; other keys are
//!    not read.
//! 2. `themes` and `entities` REPLACE the model's list whole; `event_add`
//!    / `event_remove` patch the item's events.
//! 3. Lines apply in file order to the item of their key, so for a
//!    replaced field the last line wins.
//! 4. A line that is not such an object is skipped and counted; a line
//!    for a key without a tag row is counted and not applied.
//! 5. `corrected` on an item names every field some applied line carried
//!    (`events` for `event_add` / `event_remove`).
//!
//! The design chosen: patch lines, and an undo that takes a line back.
//! Every line this module writes carries an `id` (`c000001`, `c000002`
//! …). On a line with such an id, and on no other, the reader also reads
//! `theme_add`, `theme_remove`, `entity_add`, `entity_remove` (patches,
//! like `event_add` / `event_remove`) and `undoes`. A file without a
//! numbered line reads as it always did. The other design, a line
//! carrying the whole resulting list, was not taken: such a line is
//! computed from the tags as they are on the day (a later re-tag of the
//! item would be overridden whole by it), and its undo would have to
//! write a list kept from before.
//!
//! - An operation appends ONE line: the id, `at`, `date`, `by:
//!   "dominik"`, the `key`, and one patch key holding a list of one name
//!   (`entity_add` also `entity_type`, which the reader does not read:
//!   an item's entities are plain names).
//! - `undo` appends ONE line, `{id, at, date, by, key, undoes}`. The
//!   reader then does not apply the line named, as if it were not in the
//!   file. No value is computed or kept: the undo is exact whatever the
//!   model's row is by then, and once every correction of an item is
//!   undone its tags are the model's and its `corrected` is empty.
//! - Undo is last change first, per item and per value: a correction is
//!   not undone while a later line in effect for the same item touches
//!   the same theme, entity or event (a later line with a whole `themes`
//!   / `entities` / `event` touches every value of its field). The
//!   refusal names the later line. Corrections of other values, fields
//!   and items do not stand in the way. An undo line is not undone: make
//!   the correction again.
//! - Undo, like every operation, is refused for a key that has no tag
//!   row (the item left `current.jsonl` after the correction was made),
//!   before any write. The correction's line stays in the file, where the
//!   reader does with it what it does with any line for a key without a
//!   tag row: counts it (`corrections_unmatched`) and applies it to
//!   nothing.
//! - Whether an undo would be accepted (`list`: `undoable`, `why_not`) is
//!   answered by running the undo itself up to its append
//!   ([`State::undo_plan`]), the readiness of the file included
//!   (`Jsonl::ready`, the checks of the append with no write). `undo`
//!   calls the same function and then appends. There is no second copy
//!   of any of its checks.
//!
//! Before any write, each operation:
//!
//! - reads the corrections file whole (`event_edit::log::Jsonl`, the
//!   file handling of the merge log: no-follow, a plain file, UTF-8,
//!   every line a JSON object ended by a newline, within the limits
//!   below) and refuses to go on when the reader does not read one of its
//!   lines. For a numbered line that is one judgement, made in one place
//!   for the reader and for this module
//!   ([`crate::correction_lines::Sequence::judge`]: the keys written
//!   here and no other, one name per line, an id greater than every
//!   earlier numbered id, an `undoes` that names an earlier correction of
//!   the same key not yet undone). The reader skips and counts a line
//!   that fails it; this module does not append after one, and names the
//!   line and the same reason;
//! - reads the tag rows and the event records with the reader
//!   ([`topics::TagSource`]) and asks it what it makes of the file as it
//!   is (the item's tags in effect), and of the file with the new line.
//!   The second answer must be the first with exactly the change meant,
//!   on that item and no other; for an undo it must equal what the reader
//!   makes of the file without the line taken back.
//!
//! A refusal leaves the topics folder byte for byte as it was.
//!
//! Appending (`Jsonl::append`, the merge log's): the file is opened for
//! append without following a link; on the open descriptor it must be a
//! regular file with one hard link, the file that was read, at the length
//! read; the line with its newline is at most 1 MiB (the reader's line
//! limit, [`topics::MAX_LINE_BYTES`]) and the file with it at most 64 MiB
//! ([`MAX_FILE_BYTES`], the limit over which the reader does not read the
//! file: one constant for both);
//! the line goes out in one `write` call and the file is synced. A file
//! that is not there is created create-new with the default mode of a new
//! file (the umask applies). Nothing is rewritten or removed.
//!
//! Paths: `--archive` as the seed takes it (a `..` component is refused);
//! `<archive>/analysis/topics` must exist and be a real folder, not a
//! symlink (symlinks above it are accepted).
//!
//! What is read besides: `<archive>/analysis/context/themes.json`, the
//! tagger's theme list (`themes[].name`), by `add_theme` only. This is
//! its one reader in this workspace; the index never reads it.
//!
//! The index is not touched. After a correction search shows the old
//! tags until `scout index build` runs; every result says so
//! (`reindex_needed`).
//!
//! Threat model: the seed's and the event edits' (a single-user desktop
//! tool, no hostile process beside it). Not defended: `current.jsonl` or
//! a record changing between this module's read of it and the append; a
//! link added to the corrections file in the instant between the check on
//! its descriptor and the write.

use crate::correction_lines::{numbered, LineRead, Numbered, Sequence};
use crate::event_edit::log::{Caps, Jsonl};
use crate::event_edit::store::read_plain;
use crate::event_seed::{refuse_parent_dir, resolve_out_dir};
use crate::normalize::fold;
use crate::topics::{self, LayerItem, TagSource, TopicInfo};
use anyhow::{bail, Result};
use serde::Serialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The topics folder, relative to the archive.
pub const TOPICS_DIR: &str = "analysis/topics";
/// The corrections file's name in the topics folder.
pub const CORRECTIONS_NAME: &str = "corrections.jsonl";
/// The tagger's theme list, relative to the archive.
pub const THEMES_FILE: &str = "analysis/context/themes.json";
pub use crate::correction_lines::{Op, ENTITY_TYPES, MAX_FILE_BYTES};
/// Largest theme list read.
const MAX_THEMES_BYTES: u64 = 16 << 20;
/// Largest number of a correction id (`c999999999`).
pub const MAX_ID: u64 = 999_999_999;
/// Longest entity name, in characters.
pub const MAX_NAME_CHARS: usize = 200;
const CAPS: Caps = Caps {
    file: MAX_FILE_BYTES,
    line: topics::MAX_LINE_BYTES as u64,
};

impl Op {
    fn field(self) -> Field {
        match self {
            Op::ThemeAdd | Op::ThemeRemove => Field::Themes,
            Op::EntityAdd | Op::EntityRemove => Field::Entities,
            Op::EventAdd | Op::EventRemove => Field::Events,
        }
    }

    fn adds(self) -> bool {
        matches!(self, Op::ThemeAdd | Op::EntityAdd | Op::EventAdd)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Themes,
    Entities,
    Events,
}

impl Field {
    /// The name `corrected` gives the field.
    fn corrected(self) -> &'static str {
        match self {
            Field::Themes => "themes",
            Field::Entities => "entities",
            Field::Events => "events",
        }
    }
}

/// What a line touches: one value of a field (folded; an event resolved),
/// or, with `None`, the whole field.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Slot {
    field: Field,
    value: Option<String>,
}

impl Slot {
    fn meets(&self, other: &Slot) -> bool {
        self.field == other.field
            && (self.value.is_none() || other.value.is_none() || self.value == other.value)
    }
}

/// The line an operation appends. Keys in this order; absent ones are not
/// written.
#[derive(Debug, Clone, Serialize)]
struct NewLine<'a> {
    id: &'a str,
    at: &'a str,
    date: &'a str,
    by: &'a str,
    key: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    theme_add: Option<[&'a str; 1]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    theme_remove: Option<[&'a str; 1]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    entity_add: Option<[&'a str; 1]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    entity_type: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    entity_remove: Option<[&'a str; 1]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    event_add: Option<[&'a str; 1]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    event_remove: Option<[&'a str; 1]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    undoes: Option<&'a str>,
}

/// What a line of the file is.
#[derive(Debug, Clone, PartialEq, Eq)]
enum What {
    /// A numbered correction.
    Change {
        op: Op,
        value: String,
        entity_type: Option<String>,
    },
    /// A numbered undo line.
    Undo { undoes: String },
    /// A line without a number: written by hand, or by an earlier tool.
    /// The fields of it the reader reads.
    Hand { fields: Vec<String> },
}

/// One line of the corrections file.
#[derive(Debug, Clone)]
struct Entry {
    /// Its line in the file, from 1.
    line: usize,
    id: Option<String>,
    key: String,
    at: Option<String>,
    date: Option<String>,
    by: Option<String>,
    what: What,
    read: LineRead,
    /// The whole line, for the slots of a hand-written one.
    raw: serde_json::Map<String, Value>,
}

impl Entry {
    /// How a refusal names the line.
    fn label(&self) -> String {
        match &self.id {
            Some(id) => id.clone(),
            None => format!("line {}", self.line),
        }
    }
}

/// What an operation wrote (one JSON line).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Corrected {
    pub tool: String,
    /// When the line was written, UTC.
    pub at: String,
    /// The id of the line written.
    pub id: String,
    /// The item.
    pub key: String,
    /// `theme_add`, `theme_remove`, `entity_add`, `entity_remove`,
    /// `event_add`, `event_remove` or `undo`.
    pub op: String,
    /// The name or event id written (for an undo: of the line taken back).
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_type: Option<String>,
    /// The correction an undo took back.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undoes: Option<String>,
    /// The corrections file.
    pub file: String,
    /// Always true: the index still has the old tags until `scout index
    /// build` runs. Nothing here builds it.
    pub reindex_needed: bool,
    /// The item's tags as the reader has them now.
    pub tags: TopicInfo,
}

/// One line of an item's corrections, in `scout correct list`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ListedCorrection {
    /// Null for a line without a number.
    pub id: Option<String>,
    /// The line in the file, from 1.
    pub line: usize,
    pub at: Option<String>,
    pub date: Option<String>,
    pub by: Option<String>,
    /// The op's name, `undo`, or `hand-written`.
    pub op: String,
    /// The name or event id of a numbered correction.
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_type: Option<String>,
    /// Of a hand-written line: the fields the reader reads on it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<String>,
    /// Of an undo line: the correction it took back.
    pub undoes: Option<String>,
    /// Of a correction taken back: the undo line.
    pub undone_by: Option<String>,
    /// `scout correct undo <id>` would be accepted now: every check of
    /// the undo up to its append passes.
    pub undoable: bool,
    /// Why not, when it would not: the refusal `undo` would give.
    pub why_not: Option<String>,
}

/// `scout correct list` (one JSON line).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Listing {
    pub tool: String,
    pub key: String,
    /// The corrections file (it need not exist).
    pub file: String,
    /// The item's tags with no correction applied.
    pub model: TopicInfo,
    /// The item's tags as the reader has them now.
    pub effective: TopicInfo,
    /// The lines of the file for this item, newest first.
    pub corrections: Vec<ListedCorrection>,
}

/// The topics folder of one archive, for correcting.
pub struct CorrectionEditor {
    archive: PathBuf,
    dir: PathBuf,
    now: Option<String>,
    /// The limits of the corrections file ([`CAPS`]; smaller in a test).
    caps: Caps,
}

type Items = BTreeMap<String, LayerItem>;

/// Everything one operation read, before it writes.
struct State {
    file: Jsonl,
    source: TagSource,
    entries: Vec<Entry>,
    /// What the reader made of each line.
    read: Vec<LineRead>,
    /// Every item's tags as the reader has them with the file as it is.
    items: Items,
    /// Correction id → the id of the undo line that took it back.
    undone: BTreeMap<String, String>,
    /// The largest id in the file.
    last: u64,
}

fn text(what: &str, s: &str) -> Result<String> {
    let s = crate::event_edit::plain_text(what, s)?;
    if s.is_empty() {
        bail!("{what} is empty (nothing was changed)");
    }
    Ok(s)
}

impl State {
    fn bad_line(&self, line: usize, why: &str) -> anyhow::Error {
        anyhow::anyhow!(
            "{}: line {line}: {why}; fix or remove the line by hand; nothing was changed",
            self.file.path.display()
        )
    }

    /// An event id as the reader compares it: resolved through the
    /// records when there are any, folded.
    fn event_key(&self, id: &str) -> String {
        match &self.source.records {
            Some(set) => fold(set.resolve(id).unwrap_or(id.trim())),
            None => fold(id),
        }
    }

    /// What a line in effect touches.
    fn slots(&self, e: &Entry) -> Vec<Slot> {
        let lasting = self.source.records.is_some();
        match &e.what {
            What::Undo { .. } => vec![],
            What::Change { op, value, .. } => vec![Slot {
                field: op.field(),
                value: Some(match op.field() {
                    Field::Events => self.event_key(value),
                    _ => fold(value),
                }),
            }],
            What::Hand { .. } => {
                let mut out = Vec::new();
                for (k, v) in &e.raw {
                    let whole = |field| Slot { field, value: None };
                    match k.as_str() {
                        "themes" => out.push(whole(Field::Themes)),
                        "entities" => out.push(whole(Field::Entities)),
                        "event" => out.push(whole(Field::Events)),
                        "event_add" | "event_remove" if lasting => {
                            for id in v.as_array().into_iter().flatten() {
                                out.push(Slot {
                                    field: Field::Events,
                                    value: Some(self.event_key(id.as_str().unwrap_or_default())),
                                });
                            }
                        }
                        _ => {}
                    }
                }
                out
            }
        }
    }

    /// Is the line in effect: applied by the reader, not an undo line and
    /// not taken back?
    fn in_effect(&self, e: &Entry) -> bool {
        e.read == LineRead::Applied
    }

    /// Why the line at `at` is not one an undo takes back, if it is not:
    /// the first of the checks of [`State::undo_plan`], its only caller.
    /// Not the whole answer to "would `undo` be accepted?".
    fn why_not_undoable(&self, at: usize) -> Option<String> {
        let e = &self.entries[at];
        let id = e.id.as_deref().unwrap_or_default();
        match &e.what {
            What::Hand { .. } => {
                return Some("a line without an id; `undo` takes a numbered correction".into())
            }
            What::Undo { undoes } => {
                return Some(format!(
                    "{id} is the undo of {undoes}; an undo is not undone, make the correction again"
                ))
            }
            What::Change { .. } => {}
        }
        if let Some(by) = self.undone.get(id) {
            return Some(format!("{id}: already undone, by {by}"));
        }
        let mine = self.slots(e);
        let later = self.entries[at + 1..].iter().find(|l| {
            l.key == e.key
                && self.in_effect(l)
                && self
                    .slots(l)
                    .iter()
                    .any(|s| mine.iter().any(|m| m.meets(s)))
        })?;
        let what = match &later.what {
            What::Change { op, value, .. } => format!("{} {value:?}", op.name()),
            _ => "a hand-written line".into(),
        };
        Some(format!(
            "{id}: {} ({what}) later changed the same {} of {}; undo is last change first, so undo that one before this",
            later.label(),
            match mine[0].field {
                Field::Themes => "theme",
                Field::Entities => "entity",
                Field::Events => "event",
            },
            e.key
        ))
    }

    fn next_id(&self) -> Result<String> {
        match self.last.checked_add(1).filter(|n| *n <= MAX_ID) {
            Some(n) => Ok(format!("c{n:06}")),
            None => bail!(
                "{}: the last correction has the largest id there is (c{MAX_ID}); move the file away by hand to start a new one; nothing was changed",
                self.file.path.display()
            ),
        }
    }

    /// Every check of the undo of the line at `at_line`, in order, with
    /// no write: the line is a correction, not undone, with no later
    /// change of the same value in effect; its item still has a tag row;
    /// an id is left; the reader would read the file with the undo line
    /// as the file without the line taken back; and the file can take the
    /// line (`Jsonl::ready`: the checks of the append itself). `undo`
    /// appends [`UndoPlan::line`] after this; `list` asks this for every
    /// line it shows. An error is the refusal `undo` gives.
    fn undo_plan(&self, at_line: usize, now: &str) -> Result<UndoPlan> {
        if let Some(why) = self.why_not_undoable(at_line) {
            bail!("{why} (nothing was changed)");
        }
        let entry = &self.entries[at_line];
        let id = entry.id.as_deref().unwrap_or_default();
        let What::Change {
            op,
            value,
            entity_type,
        } = &entry.what
        else {
            bail!("{id}: not a correction (nothing was changed)");
        };
        self.item(&entry.key)?;
        let mut plan = UndoPlan {
            new_id: self.next_id()?,
            at: now.to_string(),
            key: entry.key.clone(),
            undoes: id.to_string(),
            op: *op,
            value: value.clone(),
            entity_type: entity_type.clone(),
            tags: TopicInfo::default(),
        };
        // The file with the undo line must read as the file without the
        // line taken back.
        let mut with = self.file.bytes.clone();
        with.extend_from_slice(self.file.text_of(&plan.line())?.as_bytes());
        let without: Vec<u8> = self
            .file
            .bytes
            .split_inclusive(|b| *b == b'\n')
            .enumerate()
            .filter(|(i, _)| *i != at_line)
            .flat_map(|(_, l)| l.iter().copied())
            .collect();
        let (after, read) = self.source.with_corrections(&with);
        let (meant, _) = self.source.with_corrections(&without);
        let as_meant = read.len() == self.read.len() + 1
            && read.last() == Some(&LineRead::Undo)
            && read[at_line] == LineRead::Undone
            && (0..self.read.len()).all(|i| i == at_line || read[i] == self.read[i])
            && after == meant;
        let Some(tags) = after.get(&entry.key).filter(|_| as_meant) else {
            bail!(
                "{}: the reader would not read the undo of {id} as the file without that line; nothing was changed",
                self.file.path.display()
            );
        };
        plan.tags = tags.info.clone();
        self.file.ready(&plan.line())?;
        Ok(plan)
    }

    /// The item of `key` as the reader has it, or the refusal.
    fn item(&self, key: &str) -> Result<&TopicInfo> {
        match self.items.get(key) {
            Some(it) => Ok(&it.info),
            None => bail!(
                "{key}: no tag row in {}; only an item the layer holds is corrected (nothing was changed)",
                topics::LAYER_FILE
            ),
        }
    }
}

/// An undo every check allows ([`State::undo_plan`]): the line `undo`
/// appends, and what it reports.
struct UndoPlan {
    new_id: String,
    at: String,
    key: String,
    /// The correction taken back.
    undoes: String,
    op: Op,
    value: String,
    entity_type: Option<String>,
    /// The item's tags as the reader has them with the undo line.
    tags: TopicInfo,
}

impl UndoPlan {
    /// The line checked, and the line appended.
    fn line(&self) -> NewLine<'_> {
        NewLine {
            id: &self.new_id,
            at: &self.at,
            date: &self.at[..10.min(self.at.len())],
            by: "dominik",
            key: &self.key,
            theme_add: None,
            theme_remove: None,
            entity_add: None,
            entity_type: None,
            entity_remove: None,
            event_add: None,
            event_remove: None,
            undoes: Some(&self.undoes),
        }
    }
}

impl CorrectionEditor {
    /// The topics folder of `archive`, `<archive>/analysis/topics`, under
    /// the seed's path rules: no `..` component, and the folder itself a
    /// real folder, not a symlink (symlinks above it are accepted). It
    /// must exist. Nothing is written here.
    pub fn open(archive: &Path) -> Result<CorrectionEditor> {
        refuse_parent_dir(archive, "the archive")
            .map_err(|e| anyhow::anyhow!("{}", nothing_changed(&e.to_string())))?;
        if !archive.is_dir() {
            bail!("{}: the archive is not a folder", archive.display());
        }
        let dir = resolve_out_dir(&archive.join(TOPICS_DIR)).map_err(|e| {
            anyhow::anyhow!(
                "{}",
                e.to_string().replace("the out folder", "the topics folder")
            )
        })?;
        if !std::fs::symlink_metadata(&dir).is_ok_and(|m| m.is_dir()) {
            bail!("{}: no topics folder", dir.display());
        }
        Ok(CorrectionEditor {
            archive: archive.to_path_buf(),
            dir,
            now: None,
            caps: CAPS,
        })
    }

    /// The corrections file (it need not exist).
    pub fn file(&self) -> PathBuf {
        self.dir.join(CORRECTIONS_NAME)
    }

    /// Tests: the time every operation takes as now
    /// (`2026-10-12T08:00:00.000Z`).
    #[doc(hidden)]
    pub fn with_now(mut self, now: &str) -> Self {
        self.now = Some(now.to_string());
        self
    }

    fn now(&self) -> String {
        self.now.clone().unwrap_or_else(|| {
            chrono::Utc::now()
                .format("%Y-%m-%dT%H:%M:%S%.3fZ")
                .to_string()
        })
    }

    /// Read the corrections file, the tag rows and the records, and check
    /// the file as the writer needs it (see the module notes).
    fn read(&self) -> Result<State> {
        let file = Jsonl::read_keeping(&self.dir, CORRECTIONS_NAME, self.caps)?;
        let Some(source) = TagSource::read(&self.archive)? else {
            bail!(
                "{}: no tag rows; there is nothing to correct (nothing was changed)",
                self.archive.join(topics::LAYER_FILE).display()
            );
        };
        let (items, read) = source.with_corrections(&file.bytes);
        let mut state = State {
            file,
            source,
            entries: Vec::new(),
            read,
            items,
            undone: BTreeMap::new(),
            last: 0,
        };
        if state.read.len() != state.file.values.len() {
            bail!(
                "{}: the reader reads {} lines where the file has {}; nothing was changed",
                state.file.path.display(),
                state.read.len(),
                state.file.values.len()
            );
        }
        // The one judge of numbered lines, the reader's too.
        let mut seq = Sequence::default();
        for i in 0..state.file.values.len() {
            let line = i + 1;
            let Some(m) = state.file.values[i].as_object().cloned() else {
                return Err(state.bad_line(line, "not a JSON object"));
            };
            let id = numbered(&m).map(String::from);
            let key = m
                .get("key")
                .and_then(|v| v.as_str())
                .map(|k| k.trim().to_string());
            let verdict = state.read[i];
            let what = match &id {
                None => {
                    if verdict == LineRead::Bad {
                        return Err(state.bad_line(
                            line,
                            "the reader does not read it (no text `key`, or a field of the wrong type)",
                        ));
                    }
                    What::Hand {
                        fields: m
                            .keys()
                            .filter(|k| {
                                topics::CORRECTABLE.contains(&k.as_str())
                                    || (state.source.records.is_some()
                                        && matches!(k.as_str(), "event_add" | "event_remove"))
                            })
                            .cloned()
                            .collect(),
                    }
                }
                Some(_) => {
                    let what = seq.judge(&m).map_err(|why| state.bad_line(line, &why))?;
                    if verdict == LineRead::Bad {
                        return Err(state.bad_line(line, "the reader does not read it"));
                    }
                    match what {
                        Numbered::Change {
                            op,
                            value,
                            entity_type,
                        } => What::Change {
                            op,
                            value,
                            entity_type,
                        },
                        Numbered::Undo { undoes } => What::Undo { undoes },
                    }
                }
            };
            let text = |k: &str| m.get(k).and_then(|v| v.as_str()).map(String::from);
            state.entries.push(Entry {
                line,
                id,
                key: key.unwrap_or_default(),
                at: text("at"),
                date: text("date"),
                by: text("by"),
                what,
                read: verdict,
                raw: m,
            });
        }
        state.last = seq.last();
        state.undone = seq.undone_ids();
        Ok(state)
    }

    /// The tagger's theme names, in the list's spelling.
    fn theme_list(&self) -> Result<Vec<String>> {
        let path = self.archive.join(THEMES_FILE);
        let at = path.display();
        let Some(plain) = read_plain(&path, MAX_THEMES_BYTES)? else {
            bail!("{at}: no theme list; a theme is added only from it (nothing was changed)");
        };
        let v: Value = serde_json::from_slice(&plain.bytes)
            .map_err(|e| anyhow::anyhow!("{at}: not JSON ({e}) (nothing was changed)"))?;
        let Some(list) = v.get("themes").and_then(|t| t.as_array()) else {
            bail!("{at}: no `themes` list (nothing was changed)");
        };
        let mut names = Vec::new();
        for (i, t) in list.iter().enumerate() {
            match t.get("name").and_then(|n| n.as_str()).map(str::trim) {
                Some(n) if !n.is_empty() => names.push(n.to_string()),
                _ => bail!(
                    "{at}: theme {} has no text `name` (nothing was changed)",
                    i + 1
                ),
            }
        }
        Ok(names)
    }

    /// Add `theme` to the item `key`. The theme must be a name of the
    /// archive's theme list (`analysis/context/themes.json`), compared
    /// under the reader's fold; it is written in the list's spelling.
    /// Refused: no tag row for the key; a theme not on the list; one the
    /// item already has.
    pub fn add_theme(&self, key: &str, theme: &str) -> Result<Corrected> {
        let theme = text("the theme", theme)?;
        let f = fold(&theme);
        let Some(listed) = self.theme_list()?.into_iter().find(|n| fold(n) == f) else {
            bail!(
                "{theme:?}: not a theme of {THEMES_FILE}; give a name of the list (nothing was changed)"
            );
        };
        self.change(key, Op::ThemeAdd, &listed, None)
    }

    /// Take `theme` off the item `key`. It must be a theme the item has
    /// (under the reader's fold), on the list or not; it is written in the
    /// item's spelling.
    pub fn remove_theme(&self, key: &str, theme: &str) -> Result<Corrected> {
        self.change(key, Op::ThemeRemove, &text("the theme", theme)?, None)
    }

    /// Add the entity `name` to the item `key`. The name is free text:
    /// trimmed, no control character, at most [`MAX_NAME_CHARS`]
    /// characters. `kind` is one of [`ENTITY_TYPES`]; it is kept on the
    /// line and the reader does not read it. Refused: an entity the item
    /// has.
    pub fn add_entity(&self, key: &str, name: &str, kind: &str) -> Result<Corrected> {
        let name = text("the entity", name)?;
        if name.chars().count() > MAX_NAME_CHARS {
            bail!("the entity is longer than {MAX_NAME_CHARS} characters (nothing was changed)");
        }
        let kind = kind.trim();
        if !ENTITY_TYPES.contains(&kind) {
            bail!(
                "{kind:?}: not an entity type; one of {} (nothing was changed)",
                ENTITY_TYPES.join(", ")
            );
        }
        self.change(key, Op::EntityAdd, &name, Some(kind))
    }

    /// Take the entity `name` off the item `key`. It must be one the item
    /// has; it is written in the item's spelling.
    pub fn remove_entity(&self, key: &str, name: &str) -> Result<Corrected> {
        self.change(key, Op::EntityRemove, &text("the entity", name)?, None)
    }

    /// Put the item `key` in the event `event_id` names: a lasting id, a
    /// merged-away id or a monthly id. It must resolve to a record that is
    /// not removed; that record's id is what is written. Refused: no
    /// records folder (the reader reads no `event_add` without one); an
    /// id no record holds; a removed event; an event the item is in.
    pub fn add_to_event(&self, key: &str, event_id: &str) -> Result<Corrected> {
        self.change(key, Op::EventAdd, &text("the event id", event_id)?, None)
    }

    /// Take the item `key` out of the event `event_id` names, whoever put
    /// it there (its tag row, or a correction). Refusals as
    /// [`CorrectionEditor::add_to_event`], and an event the item is not
    /// in.
    pub fn remove_from_event(&self, key: &str, event_id: &str) -> Result<Corrected> {
        self.change(key, Op::EventRemove, &text("the event id", event_id)?, None)
    }

    /// The value an operation writes, checked against the item as it is.
    fn value_for(&self, state: &State, t: &TopicInfo, op: Op, given: &str) -> Result<String> {
        let f = fold(given);
        let key = &t.key;
        let (list, what) = match op.field() {
            Field::Themes => (&t.themes, "theme"),
            Field::Entities => (&t.entities, "entity"),
            Field::Events => {
                let Some(set) = &state.source.records else {
                    bail!(
                        "{}: no records folder; without one the reader reads no event_add or event_remove, so the item's events are not corrected (run `scout events seed` first) (nothing was changed)",
                        self.archive.join(crate::events::EVENTS_DIR).display()
                    );
                };
                let record = set.resolve(given).and_then(|id| set.records.get(&fold(id)));
                let Some(record) = record else {
                    bail!("{given}: no event record holds this id (nothing was changed)");
                };
                if record.removed.is_some() {
                    bail!(
                        "{}: a removed event; restore it first (nothing was changed)",
                        record.id
                    );
                }
                let has = t.has_event(&record.id);
                return match (op.adds(), has) {
                    (true, true) => bail!(
                        "{key}: already in the event {} (nothing was changed)",
                        record.id
                    ),
                    (false, false) => bail!(
                        "{key}: not in the event {} (nothing was changed)",
                        record.id
                    ),
                    _ => Ok(record.id.clone()),
                };
            }
        };
        let have = list.iter().find(|x| fold(x) == f);
        match (op.adds(), have) {
            (true, Some(have)) => {
                bail!("{key}: already has the {what} {have:?} (nothing was changed)")
            }
            (true, None) => Ok(given.to_string()),
            (false, Some(have)) => Ok(have.trim().to_string()),
            (false, None) => bail!("{key}: has no {what} {given:?} (nothing was changed)"),
        }
    }

    fn change(&self, key: &str, op: Op, given: &str, kind: Option<&str>) -> Result<Corrected> {
        let key = text("the key", key)?;
        let mut state = self.read()?;
        let before = state.item(&key)?.clone();
        let value = self.value_for(&state, &before, op, given)?;
        let id = state.next_id()?;
        let at = self.now();
        let one = |o: Op| (o == op).then_some([value.as_str()]);
        let line = NewLine {
            id: &id,
            at: &at,
            date: &at[..10.min(at.len())],
            by: "dominik",
            key: &key,
            theme_add: one(Op::ThemeAdd),
            theme_remove: one(Op::ThemeRemove),
            entity_add: one(Op::EntityAdd),
            entity_type: kind,
            entity_remove: one(Op::EntityRemove),
            event_add: one(Op::EventAdd),
            event_remove: one(Op::EventRemove),
            undoes: None,
        };
        // What the reader makes of the file with the line: the item with
        // exactly this change, every other item as it is.
        let mut with = state.file.bytes.clone();
        with.extend_from_slice(state.file.text_of(&line)?.as_bytes());
        let (after, read) = state.source.with_corrections(&with);
        let faults = |why: String| -> anyhow::Error {
            anyhow::anyhow!(
                "{}: the reader would not read the correction as meant ({why}); nothing was changed",
                state.file.path.display()
            )
        };
        if read.len() != state.read.len() + 1
            || read[..state.read.len()] != state.read[..]
            || read.last() != Some(&LineRead::Applied)
        {
            return Err(faults(
                "the line, or a line before it, would be read otherwise".into(),
            ));
        }
        let f = fold(&value);
        let mut want = before.clone();
        let list = match op.field() {
            Field::Themes => &mut want.themes,
            Field::Entities => &mut want.entities,
            Field::Events => want.events.get_or_insert_with(Vec::new),
        };
        if op.adds() {
            list.push(value.clone());
        } else {
            list.retain(|x| fold(x) != f);
        }
        for (k, it) in &state.items {
            let Some(now) = after.get(k) else {
                return Err(faults(format!("{k} would have no tags")));
            };
            if *k != key {
                if now != it {
                    return Err(faults(format!("the tags of {k} would change")));
                }
                continue;
            }
            // `corrected` gains the field. A change of the item's events
            // also decides `event` and `event_name` (the tag row's own
            // event, null while the item is out of it): the reader's to
            // work out, and `events` is the list compared.
            let strip = |t: &TopicInfo| {
                let mut t = t.clone();
                t.corrected.clear();
                if op.field() == Field::Events {
                    t.event = None;
                    t.event_name = None;
                }
                t
            };
            if strip(&now.info) != strip(&want) || now.path != it.path || now.date != it.date {
                return Err(faults(format!("{k} would not have exactly this change")));
            }
            if !now
                .info
                .corrected
                .iter()
                .any(|c| c == op.field().corrected())
            {
                return Err(faults(format!("{k} would not be marked corrected")));
            }
        }
        if after.len() != state.items.len() {
            return Err(faults("an item would appear".into()));
        }
        state.file.append(&line)?;
        Ok(Corrected {
            tool: format!("scout correct {}", op.name().replace('_', "-")),
            at,
            id,
            op: op.name().into(),
            value,
            entity_type: kind.map(String::from),
            undoes: None,
            file: state.file.path.display().to_string(),
            reindex_needed: true,
            tags: after[&key].info.clone(),
            key,
        })
    }

    /// Undo the correction `id`: append one line naming it, after which
    /// the reader does not apply that correction. Nothing else is written
    /// and no value is restored from anywhere.
    ///
    /// Refused, with nothing changed: no such correction; an undo line;
    /// one already undone; one after which a later line in effect touched
    /// the same theme, entity or event of the same item (the refusal
    /// names it: undo that one first); a correction whose item has no tag
    /// row any more (its line stays, and the reader goes on counting it
    /// as a correction for an unknown key, applied to nothing); no id
    /// left; a file that cannot take the line. Every one of these is
    /// [`State::undo_plan`]'s, which `list` runs for its `undoable`.
    pub fn undo(&self, id: &str) -> Result<Corrected> {
        let id = id.trim();
        let mut state = self.read()?;
        let Some(at_line) = state
            .entries
            .iter()
            .position(|e| e.id.as_deref() == Some(id))
        else {
            bail!(
                "{id}: no such correction in {} (nothing was changed)",
                state.file.path.display()
            );
        };
        let plan = state.undo_plan(at_line, &self.now())?;
        state.file.append(&plan.line())?;
        Ok(Corrected {
            tool: "scout correct undo".into(),
            at: plan.at,
            id: plan.new_id,
            op: "undo".into(),
            value: plan.value,
            entity_type: plan.entity_type.filter(|_| plan.op == Op::EntityAdd),
            undoes: Some(plan.undoes),
            file: state.file.path.display().to_string(),
            reindex_needed: true,
            tags: plan.tags,
            key: plan.key,
        })
    }

    /// The item `key`: its tags with no correction applied, its tags as
    /// the reader has them, and the lines of the corrections file for it,
    /// newest first, each with whether `undo` would take it now: the
    /// answer of [`State::undo_plan`], the function `undo` runs before
    /// its append, so `why_not` is word for word the refusal `undo`
    /// would give. Reads only. The file is checked as for a write, so a
    /// line the writer would refuse to append after is an error here too.
    pub fn list(&self, key: &str) -> Result<Listing> {
        let key = text("the key", key)?;
        let state = self.read()?;
        let effective = state.item(&key)?.clone();
        let (model, _) = state.source.with_corrections(b"");
        let now = self.now();
        let corrections = state
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.key == key)
            .rev()
            .map(|(i, e)| {
                let why_not = state.undo_plan(i, &now).err().map(|e| format!("{e:#}"));
                let (op, value, entity_type, fields, undoes) = match &e.what {
                    What::Change {
                        op,
                        value,
                        entity_type,
                    } => (
                        op.name().to_string(),
                        Some(value.clone()),
                        entity_type.clone(),
                        vec![],
                        None,
                    ),
                    What::Undo { undoes } => {
                        ("undo".into(), None, None, vec![], Some(undoes.clone()))
                    }
                    What::Hand { fields } => {
                        ("hand-written".into(), None, None, fields.clone(), None)
                    }
                };
                ListedCorrection {
                    id: e.id.clone(),
                    line: e.line,
                    at: e.at.clone(),
                    date: e.date.clone(),
                    by: e.by.clone(),
                    op,
                    value,
                    entity_type,
                    fields,
                    undoes,
                    undone_by: e.id.as_ref().and_then(|id| state.undone.get(id).cloned()),
                    undoable: why_not.is_none(),
                    why_not,
                }
            })
            .collect();
        Ok(Listing {
            tool: "scout correct list".into(),
            file: state.file.path.display().to_string(),
            model: model
                .get(&key)
                .map(|it| it.info.clone())
                .unwrap_or_default(),
            effective,
            corrections,
            key,
        })
    }
}

/// The seed's refusals end "nothing was written"; here nothing is changed.
fn nothing_changed(e: &str) -> String {
    e.replace("nothing was written", "nothing was changed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_append_never_leaves_a_file_over_its_limit() {
        let tmp = tempfile::tempdir().unwrap();
        let topics_dir = tmp.path().join(TOPICS_DIR);
        std::fs::create_dir_all(&topics_dir).unwrap();
        std::fs::write(
            tmp.path().join(topics::LAYER_FILE),
            "{\"key\":\"x:1\",\"themes\":[],\"entities\":[]}\n",
        )
        .unwrap();
        let mut editor = CorrectionEditor::open(tmp.path())
            .unwrap()
            .with_now("2026-10-12T08:00:00.000Z");
        editor.caps = Caps {
            file: 450,
            line: 200,
        };
        let file = editor.file();
        let first = editor.add_entity("x:1", "E1", "person").unwrap();
        assert_eq!(first.id, "c000001");
        let each = std::fs::metadata(&file).unwrap().len();
        assert!(each * 3 < 450 && each * 4 > 450, "{each}");
        editor.add_entity("x:1", "E2", "person").unwrap();
        editor.add_entity("x:1", "E3", "person").unwrap();
        let full = std::fs::read(&file).unwrap();
        // A fourth line would pass the file limit.
        let e = editor.add_entity("x:1", "E4", "person").unwrap_err();
        assert!(
            e.to_string().contains("over the limit of 450 bytes")
                && e.to_string().contains("nothing was changed"),
            "{e}"
        );
        assert_eq!(std::fs::read(&file).unwrap(), full);
        // A line over the line limit: refused, nothing written.
        let long = "n".repeat(MAX_NAME_CHARS);
        let e = editor.add_entity("x:1", &long, "person").unwrap_err();
        assert!(
            e.to_string().contains("over the limit of 200 for one line"),
            "{e}"
        );
        assert_eq!(std::fs::read(&file).unwrap(), full);
        // The file is still read, by the writer and by the reader.
        assert_eq!(
            editor.list("x:1").unwrap().effective.entities,
            ["E1", "E2", "E3"]
        );
        // A file over the limit by other means is not read, so not written.
        std::fs::write(&file, [full.clone(), full.clone()].concat()).unwrap();
        let e = editor.undo("c000001").unwrap_err();
        assert!(e.to_string().contains("larger than 450 bytes"), "{e}");
    }

    #[test]
    fn a_whole_field_meets_every_value_of_it_and_no_other_field() {
        let s = |field, v: Option<&str>| Slot {
            field,
            value: v.map(String::from),
        };
        assert!(s(Field::Themes, Some("a")).meets(&s(Field::Themes, Some("a"))));
        assert!(!s(Field::Themes, Some("a")).meets(&s(Field::Themes, Some("b"))));
        assert!(s(Field::Themes, Some("a")).meets(&s(Field::Themes, None)));
        assert!(s(Field::Themes, None).meets(&s(Field::Themes, Some("b"))));
        assert!(!s(Field::Themes, None).meets(&s(Field::Entities, None)));
        assert!(!s(Field::Events, Some("a")).meets(&s(Field::Themes, Some("a"))));
    }
}

//! Lasting events: records kept BESIDE a corpus, read-only, one Markdown
//! file per event in `<corpus root>/analysis/events/<id>.md` (spec
//! `archive-scout/docs/specs/2026-10-11-lasting-events-spec.md`).
//!
//! - A record's YAML frontmatter is the contract: `id` (equal to the file
//!   stem), `name`, `aliases[]`, `start`, `end`, `members[]` (the monthly
//!   event ids folded into it, `2026-09/E21`), `themes[]`, `origin`,
//!   `edited`, `merged_into`. The body is the description.
//! - Resolution: a monthly id resolves to the record that lists it in
//!   `members`; a record with `merged_into` resolves to the record it was
//!   merged into, followed to the end of the chain. A `merged_into` that
//!   names no record, or that closes a cycle, is reported and that record
//!   is treated as not merged.
//! - A malformed record (no frontmatter, bad YAML, no `id`, an `id` that
//!   is not the file stem, a repeated id, a wrongly typed field, YAML
//!   nested deeper than [`MAX_YAML_DEPTH`]) is skipped with a warning
//!   naming its file; the rest is read.
//! - Nothing here recurses over record content. The frontmatter is read
//!   by pulling parser events one at a time ([`read_flat`]) and the pull
//!   stops at the first value that nests too deep; merge chains are
//!   walked in loops bounded by the number of records.
//! - No folder: `read` gives `None` and the topic layer behaves exactly as
//!   it did before lasting events: monthly events only, one per item; a
//!   tag row's `events` list and the corrections' `event_add` /
//!   `event_remove` are not read ([`crate::topics`]).
//!
//! The archive files are never written. [`store`] copies the records and
//! the id table into the derived index at build time; queries read only
//! that copy.

use crate::normalize::fold;
use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use yaml_rust2::parser::{Event, Parser};
use yaml_rust2::scanner::TScalarStyle;

/// The records folder, relative to the corpus root.
pub const EVENTS_DIR: &str = "analysis/events";
/// Largest record file read, in bytes; a larger one is skipped.
pub const MAX_RECORD_BYTES: u64 = 1 << 20;
/// Deepest YAML nesting read in a record's frontmatter: the mapping (1),
/// a list value (2) and one level inside an ignored value (3). A record
/// that nests deeper is skipped.
pub const MAX_YAML_DEPTH: usize = 3;
/// Warnings kept in a build report (the total is always reported).
pub const MAX_WARNINGS: usize = 50;

/// One lasting event, as its record file gives it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventRecord {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub start: Option<String>,
    pub end: Option<String>,
    /// Monthly event ids folded into this event.
    #[serde(default)]
    pub members: Vec<String>,
    #[serde(default)]
    pub themes: Vec<String>,
    pub origin: Option<String>,
    pub edited: Option<String>,
    /// The record this one was merged into. In an [`EventSet`] this is the
    /// value in effect: `None` when the file's value names no record or
    /// closes a cycle.
    pub merged_into: Option<String>,
    /// The record body.
    #[serde(default)]
    pub description: String,
}

/// The records of one corpus, with every id resolved.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EventSet {
    pub dir: PathBuf,
    /// Records by folded id.
    pub records: BTreeMap<String, EventRecord>,
    /// Folded record id or monthly member id → the lasting id it resolves
    /// to (as written in that record).
    pub aliases: BTreeMap<String, String>,
    /// Record files skipped as malformed.
    pub skipped: usize,
    /// One line per skipped file, unusable `merged_into` or member conflict.
    pub warnings: Vec<String>,
}

/// What a build did with the records (`index build` reports it).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventsReport {
    pub dir: String,
    /// Records read.
    pub records: usize,
    /// Of those, records merged into another.
    pub merged: usize,
    /// Monthly event ids that resolve to a record.
    pub members: usize,
    /// Record files skipped as malformed.
    pub skipped: usize,
    pub warnings_total: usize,
    /// The first [`MAX_WARNINGS`] warnings.
    pub warnings: Vec<String>,
}

impl EventSet {
    /// Resolve a set from parsed records (later records with an id already
    /// seen are skipped). `dir` only labels the report.
    pub fn from_records(dir: PathBuf, records: Vec<EventRecord>) -> EventSet {
        let mut set = EventSet {
            dir,
            ..EventSet::default()
        };
        for r in records {
            let key = fold(&r.id);
            if key.is_empty() || set.records.contains_key(&key) {
                set.skipped += 1;
                set.warnings.push(format!(
                    "{}: repeated or empty id {:?}",
                    file_of(&r.id),
                    r.id
                ));
                continue;
            }
            set.records.insert(key, r);
        }
        set.resolve_merges();
        set.resolve_members();
        set
    }

    /// Clear every `merged_into` that names no record or closes a cycle.
    fn resolve_merges(&mut self) {
        let target: BTreeMap<String, Option<String>> = self
            .records
            .iter()
            .map(|(k, r)| (k.clone(), r.merged_into.as_deref().map(fold)))
            .collect();
        let mut broken: Vec<(String, &'static str)> = Vec::new();
        for (k, t) in &target {
            let Some(t) = t else { continue };
            if !target.contains_key(t) {
                broken.push((k.clone(), "names no record"));
                continue;
            }
            // In a cycle exactly when the chain from here comes back here.
            // A chain is at most as long as the set, so the walk is bounded.
            let mut cur = t;
            for _ in 0..=target.len() {
                if cur == k {
                    broken.push((k.clone(), "closes a cycle"));
                    break;
                }
                match target.get(cur).and_then(Option::as_ref) {
                    Some(next) => cur = next,
                    None => break,
                }
            }
        }
        for (k, why) in broken {
            if let Some(r) = self.records.get_mut(&k) {
                self.warnings.push(format!(
                    "{}: merged_into {:?} {why}; treated as not merged",
                    file_of(&r.id),
                    r.merged_into.as_deref().unwrap_or_default()
                ));
                r.merged_into = None;
            }
        }
    }

    /// The record at the end of `key`'s merge chain (cycles are gone).
    fn last_of(&self, key: &str) -> Option<&EventRecord> {
        let mut cur = self.records.get(key)?;
        for _ in 0..self.records.len() {
            match cur
                .merged_into
                .as_deref()
                .and_then(|t| self.records.get(&fold(t)))
            {
                Some(next) => cur = next,
                None => break,
            }
        }
        Some(cur)
    }

    fn resolve_members(&mut self) {
        let mut aliases: BTreeMap<String, String> = BTreeMap::new();
        let mut warnings = Vec::new();
        for k in self.records.keys() {
            if let Some(last) = self.last_of(k) {
                aliases.insert(k.clone(), last.id.clone());
            }
        }
        // Records in id order, so the first to claim a member keeps it.
        for (k, r) in &self.records {
            let Some(last) = aliases.get(k).cloned() else {
                continue;
            };
            for m in &r.members {
                let mk = fold(m);
                if mk.is_empty() {
                    continue;
                }
                if self.records.contains_key(&mk) {
                    warnings.push(format!(
                        "{}: member {m:?} is a record id, not a monthly event; ignored",
                        file_of(&r.id)
                    ));
                    continue;
                }
                match aliases.get(&mk) {
                    None => {
                        aliases.insert(mk, last.clone());
                    }
                    Some(have) if *have == last => {}
                    Some(have) => warnings.push(format!(
                        "{}: member {m:?} already belongs to {have:?}; kept there",
                        file_of(&r.id)
                    )),
                }
            }
        }
        self.aliases = aliases;
        self.warnings.extend(warnings);
    }

    /// The lasting id `id` (a record id, merged or not, or a monthly member
    /// id) resolves to; `None` when no record holds it.
    pub fn resolve(&self, id: &str) -> Option<&str> {
        self.aliases.get(&fold(id)).map(String::as_str)
    }

    pub fn report(&self) -> EventsReport {
        EventsReport {
            dir: self.dir.display().to_string(),
            records: self.records.len(),
            merged: self
                .records
                .values()
                .filter(|r| r.merged_into.is_some())
                .count(),
            members: self.aliases.len().saturating_sub(self.records.len()),
            skipped: self.skipped,
            warnings_total: self.warnings.len(),
            warnings: self.warnings.iter().take(MAX_WARNINGS).cloned().collect(),
        }
    }
}

fn file_of(id: &str) -> String {
    format!("{EVENTS_DIR}/{id}.md")
}

// ---------- reading the folder ----------

enum Entry {
    /// A regular `.md` file within the size cap.
    Bytes(Vec<u8>),
    /// A `.md` name that is not a readable regular file of that size.
    Refused(&'static str),
}

/// Visit every `*.md` entry of the records folder in name order, feeding
/// names and bytes to `hash`, then one zero byte (no file name is empty,
/// so what a caller hashes next cannot read as a further record).
/// `Ok(false)` when the folder is missing; nothing is hashed then.
fn scan(root: &Path, hash: &mut Sha1, mut each: impl FnMut(&str, Entry)) -> Result<bool> {
    let dir = root.join(EVENTS_DIR);
    let rd = match std::fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) =>
        {
            return Ok(false)
        }
        Err(e) => return Err(e).with_context(|| format!("read {}", dir.display())),
    };
    let mut names: Vec<(std::ffi::OsString, PathBuf)> = Vec::new();
    for e in rd {
        let e = e.with_context(|| format!("read {}", dir.display()))?;
        names.push((e.file_name(), e.path()));
    }
    names.sort();
    hash.update(b"\0events\0");
    for (name, path) in names {
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.starts_with('.') || !name.ends_with(".md") {
            continue;
        }
        hash.update(name.as_bytes());
        hash.update([0u8]);
        // Not followed: a record is a plain file of this folder.
        let entry = match std::fs::symlink_metadata(&path) {
            Ok(m) if !m.is_file() => Entry::Refused("not a plain file"),
            Ok(m) if m.len() > MAX_RECORD_BYTES => Entry::Refused("larger than 1 MiB"),
            Ok(_) => match read_capped(&path) {
                Ok(Some(b)) => Entry::Bytes(b),
                Ok(None) => Entry::Refused("larger than 1 MiB"),
                Err(_) => Entry::Refused("cannot be read"),
            },
            Err(_) => Entry::Refused("cannot be read"),
        };
        match &entry {
            Entry::Bytes(b) => {
                hash.update((b.len() as u64).to_le_bytes());
                hash.update(b);
            }
            Entry::Refused(why) => hash.update(why.as_bytes()),
        }
        each(name, entry);
    }
    hash.update([0u8]);
    Ok(true)
}

/// The file's bytes, or `None` when it grew past the cap since it was
/// measured. The open does not follow a link put there after the check
/// above, and what was opened must itself be a plain file.
fn read_capped(path: &Path) -> std::io::Result<Option<Vec<u8>>> {
    use std::io::Read;
    let mut open = std::fs::OpenOptions::new();
    open.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        open.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = open.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::other("not a plain file"));
    }
    let mut buf = Vec::new();
    file.take(MAX_RECORD_BYTES + 1).read_to_end(&mut buf)?;
    Ok((buf.len() as u64 <= MAX_RECORD_BYTES).then_some(buf))
}

/// Feed the records folder to `hash` (nothing when it is missing), so a
/// build can tell whether the records changed. False when it is missing.
pub(crate) fn hash_into(root: &Path, hash: &mut Sha1) -> Result<bool> {
    scan(root, hash, |_, _| {})
}

/// Read the lasting events of the corpus at `root`; `None` when it has no
/// records folder.
pub fn read(root: &Path) -> Result<Option<EventSet>> {
    read_hashed(root, &mut Sha1::new())
}

/// [`read`], feeding `hash` exactly the bytes it read.
pub(crate) fn read_hashed(root: &Path, hash: &mut Sha1) -> Result<Option<EventSet>> {
    let mut records = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let found = scan(root, hash, |name, entry| {
        let parsed = match entry {
            Entry::Refused(why) => Err(why.to_string()),
            Entry::Bytes(b) => match std::str::from_utf8(&b) {
                Ok(text) => parse_record(text, name.trim_end_matches(".md")),
                Err(_) => Err("not valid UTF-8".to_string()),
            },
        };
        match parsed {
            Ok(r) => records.push(r),
            Err(why) => skipped.push(format!("{EVENTS_DIR}/{name}: {why}; skipped")),
        }
    })?;
    if !found {
        return Ok(None);
    }
    let mut set = EventSet::from_records(root.join(EVENTS_DIR), records);
    set.skipped += skipped.len();
    skipped.append(&mut set.warnings);
    set.warnings = skipped;
    Ok(Some(set))
}

// ---------- one record ----------

/// A frontmatter value, as far as a record needs it.
#[derive(Debug, PartialEq, Eq)]
enum Val {
    Null,
    Str(String),
    List(Vec<String>),
    /// A mapping, or a list holding something other than scalars.
    Other,
}

/// Collects the top-level mapping of one YAML document: scalar and
/// scalar-list values by key. Aliases are refused (they can expand without
/// bound) and nothing below the second level is kept.
#[derive(Default)]
struct Flat {
    depth: usize,
    /// A value opened below [`MAX_YAML_DEPTH`]; reading stops there.
    too_deep: bool,
    top_is_map: bool,
    key: Option<String>,
    list: Option<Vec<String>>,
    other: bool,
    map: BTreeMap<String, Val>,
    err: Option<String>,
}

impl Flat {
    fn open(&mut self, sequence: bool) {
        if self.depth >= MAX_YAML_DEPTH {
            self.too_deep = true;
            return;
        }
        match self.depth {
            0 => self.top_is_map = !sequence,
            1 if self.key.is_none() => self.fail("a key is not a plain name"),
            1 if sequence => self.list = Some(Vec::new()),
            _ => self.other = true,
        }
        self.depth += 1;
    }

    fn close(&mut self) {
        self.depth = self.depth.saturating_sub(1);
        if self.depth == 1 {
            let v = match (self.other, self.list.take()) {
                (false, Some(l)) => Val::List(l),
                _ => Val::Other,
            };
            self.other = false;
            self.put(v);
        }
    }

    fn put(&mut self, v: Val) {
        if let Some(k) = self.key.take() {
            if self.map.insert(k.clone(), v).is_some() {
                self.fail(&format!("key {k:?} appears twice"));
            }
        }
    }

    fn fail(&mut self, why: &str) {
        if self.err.is_none() {
            self.err = Some(why.to_string());
        }
    }
}

fn is_null(v: &str, style: TScalarStyle) -> bool {
    style == TScalarStyle::Plain && matches!(v, "" | "~" | "null" | "Null" | "NULL")
}

impl Flat {
    fn on_event(&mut self, ev: Event) {
        match ev {
            Event::Alias(_) => self.fail("YAML aliases are not read"),
            Event::MappingStart(..) => self.open(false),
            Event::SequenceStart(..) => self.open(true),
            Event::MappingEnd | Event::SequenceEnd => self.close(),
            Event::Scalar(v, style, ..) => match self.depth {
                1 if self.key.is_none() => self.key = Some(v),
                1 if is_null(&v, style) => self.put(Val::Null),
                1 => self.put(Val::Str(v)),
                2 if !is_null(&v, style) => {
                    if let Some(l) = self.list.as_mut() {
                        l.push(v);
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
}

/// Read the first YAML document of `front` into a [`Flat`].
///
/// The parser is driven one event at a time (`Parser::next_token`, a state
/// machine over explicit stacks) and never through `Parser::load`, which
/// calls itself once per nesting level and overflows the stack on a deeply
/// nested value. The pull stops at the first value below
/// [`MAX_YAML_DEPTH`], so the rest of such a record is not parsed. (The
/// scanner may look ahead of the parser; it has no recursion and refuses
/// more than 255 open flow brackets itself, "recursion limit exceeded".)
fn read_flat(front: &str) -> std::result::Result<Flat, String> {
    let mut flat = Flat::default();
    let mut parser = Parser::new_from_str(front);
    loop {
        let (ev, _) = parser.next_token().map_err(|e| format!("bad YAML ({e})"))?;
        if matches!(ev, Event::DocumentEnd | Event::StreamEnd) {
            return Ok(flat);
        }
        flat.on_event(ev);
        if flat.too_deep {
            return Err(format!(
                "bad YAML (nested deeper than {MAX_YAML_DEPTH} levels)"
            ));
        }
    }
}

/// Split `text` into its frontmatter block and body.
fn split_frontmatter(text: &str) -> Option<(&str, &str)> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let rest = text
        .strip_prefix("---\r\n")
        .or_else(|| text.strip_prefix("---\n"))?;
    let mut at = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            return Some((&rest[..at], &rest[at + line.len()..]));
        }
        at += line.len();
    }
    None
}

/// Parse one record file; `stem` is its file name without `.md`.
pub fn parse_record(text: &str, stem: &str) -> std::result::Result<EventRecord, String> {
    let (front, body) = split_frontmatter(text).ok_or("no frontmatter")?;
    let flat = read_flat(front)?;
    if let Some(e) = flat.err {
        return Err(format!("bad YAML ({e})"));
    }
    if !flat.top_is_map {
        return Err("frontmatter is not a mapping".into());
    }
    let mut map = flat.map;
    let mut scalar = |k: &str| match map.remove(k) {
        None | Some(Val::Null) => Ok(None),
        Some(Val::Str(s)) if s.trim().is_empty() => Ok(None),
        Some(Val::Str(s)) => Ok(Some(s.trim().to_string())),
        Some(_) => Err(format!("{k} is not a single value")),
    };
    let id = scalar("id")?.ok_or("no id")?;
    if id != stem {
        return Err(format!("id {id:?} does not match the file name"));
    }
    let name = scalar("name")?.unwrap_or_else(|| id.clone());
    let (start, end) = (scalar("start")?, scalar("end")?);
    let (origin, edited) = (scalar("origin")?, scalar("edited")?);
    let merged_into = scalar("merged_into")?;
    let mut list = |k: &str| match map.remove(k) {
        None | Some(Val::Null) => Ok(Vec::new()),
        Some(Val::List(l)) => Ok(l
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()),
        Some(_) => Err(format!("{k} is not a list")),
    };
    Ok(EventRecord {
        aliases: list("aliases")?,
        members: list("members")?,
        themes: list("themes")?,
        id,
        name,
        start,
        end,
        origin,
        edited,
        merged_into,
        description: body.trim().to_string(),
    })
}

// ---------- the index copy ----------

fn has_tables(conn: &Connection) -> Result<bool> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE name IN ('event_records', 'event_aliases')",
        [],
        |r| r.get(0),
    )?;
    Ok(n == 2)
}

/// Replace the index's copy of the records; `None` empties it.
pub(crate) fn store(conn: &Connection, set: Option<&EventSet>) -> Result<()> {
    conn.execute("DELETE FROM event_records", [])?;
    conn.execute("DELETE FROM event_aliases", [])?;
    let Some(set) = set else {
        return Ok(());
    };
    let mut rec = conn.prepare_cached("INSERT INTO event_records (id, record) VALUES (?1, ?2)")?;
    for (k, r) in &set.records {
        rec.execute(params![k, serde_json::to_string(r)?])?;
    }
    let mut al = conn.prepare_cached("INSERT INTO event_aliases (alias, id) VALUES (?1, ?2)")?;
    for (alias, id) in &set.aliases {
        al.execute(params![alias, id])?;
    }
    Ok(())
}

/// The index's records by folded id; empty for a corpus without records
/// or an index built before lasting events.
pub fn load(conn: &Connection) -> Result<BTreeMap<String, EventRecord>> {
    let mut out = BTreeMap::new();
    if !has_tables(conn)? {
        return Ok(out);
    }
    let mut st = conn.prepare("SELECT id, record FROM event_records")?;
    for row in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (k, json) = row?;
        if let Ok(r) = serde_json::from_str::<EventRecord>(&json) {
            out.insert(k, r);
        }
    }
    Ok(out)
}

/// The lasting id an `event:` value resolves to in this index: `id` itself
/// when no record holds it (a monthly event without a record, or a corpus
/// without records).
pub fn resolve_in(conn: &Connection, id: &str) -> Result<String> {
    if !has_tables(conn)? {
        return Ok(id.to_string());
    }
    let hit: Option<String> = conn
        .query_row(
            "SELECT id FROM event_aliases WHERE alias = ?1",
            [fold(id)],
            |r| r.get(0),
        )
        .optional()?;
    Ok(hit.unwrap_or_else(|| id.to_string()))
}

/// Folded ids that name the same event are kept once, in order.
pub(crate) fn dedup(ids: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    ids.into_iter()
        .map(|i| i.trim().to_string())
        .filter(|i| !i.is_empty() && seen.insert(fold(i)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(id: &str, members: &[&str], merged_into: Option<&str>) -> EventRecord {
        EventRecord {
            id: id.into(),
            name: id.to_uppercase(),
            members: members.iter().map(|m| m.to_string()).collect(),
            merged_into: merged_into.map(String::from),
            ..EventRecord::default()
        }
    }

    #[test]
    fn frontmatter_values_are_read_as_yaml() {
        let text = "---\nid: 2026-09-a\nname: \"He said: \\\"it's\\\" \\u00e9\"\naliases:\n  - 'O''Reilly: x'\n  - plain # comment\nstart: 2026-09-01\nend:\nmembers: [2026-09/E01, \"2026-10/E04\"]\nthemes: []\norigin: model\nedited: ~\nmerged_into: null\nextra: {nested: [1, 2]}\n---\n\nBody: line.\n---\nmore\n";
        let r = parse_record(text, "2026-09-a").unwrap();
        assert_eq!(r.name, "He said: \"it's\" é");
        assert_eq!(r.aliases, vec!["O'Reilly: x", "plain"]);
        assert_eq!(r.start.as_deref(), Some("2026-09-01"));
        assert_eq!((r.end, r.edited, r.merged_into), (None, None, None));
        assert_eq!(r.members, vec!["2026-09/E01", "2026-10/E04"]);
        assert_eq!(r.description, "Body: line.\n---\nmore");
        // A quoted "null" is a name, not an empty field.
        let r = parse_record("---\nid: x\nname: 'null'\n---\n", "x").unwrap();
        assert_eq!(r.name, "null");
        // No name: the id stands in.
        assert_eq!(parse_record("---\nid: x\n---\n", "x").unwrap().name, "x");
    }

    #[test]
    fn malformed_records_are_refused_with_a_reason() {
        for (text, why) in [
            ("no frontmatter\n", "no frontmatter"),
            ("---\nid: x\n", "no frontmatter"),
            ("---\nid: [x\n---\n", "bad YAML"),
            ("---\n- id\n---\n", "not a mapping"),
            ("---\nname: X\n---\n", "no id"),
            ("---\nid: y\n---\n", "does not match the file name"),
            ("---\nid: X\n---\n", "does not match the file name"),
            (
                "---\nid: x\nmembers: 2026-09/E01\n---\n",
                "members is not a list",
            ),
            ("---\nid: x\nmembers: [[a]]\n---\n", "members is not a list"),
            ("---\nid: x\nname: [a]\n---\n", "name is not a single value"),
            ("---\nid: x\nid: x\n---\n", "appears twice"),
            (
                "---\na: &a [x]\nid: x\nmembers: *a\n---\n",
                "aliases are not read",
            ),
            ("---\n[a]: b\nid: x\n---\n", "not a plain name"),
        ] {
            let e = parse_record(text, "x").unwrap_err();
            assert!(e.contains(why), "{text:?}: {e}");
        }
    }

    /// Records nested 100,000 deep, in every shape that fits the size cap.
    fn deep_records() -> Vec<(&'static str, String)> {
        let n = 100_000;
        vec![
            (
                "block lists",
                format!("---\nid: x\nmembers:\n  {}a\n---\n", "- ".repeat(n)),
            ),
            (
                "flow lists",
                format!("---\nid: x\nmembers: {}\n---\n", "[".repeat(n)),
            ),
            (
                "closed flow lists",
                format!(
                    "---\nid: x\nextra: {}{}\n---\n",
                    "[".repeat(n),
                    "]".repeat(n)
                ),
            ),
            (
                "flow mappings",
                format!("---\nid: x\nextra: {}\n---\n", "{a: ".repeat(n)),
            ),
            (
                "lists of mappings",
                format!("---\nid: x\nextra:\n  {}a\n---\n", "- ? ".repeat(n)),
            ),
            ("at the top", format!("---\n{}a\n---\n", "- ".repeat(n))),
        ]
    }

    #[test]
    fn a_record_nested_100_000_deep_is_refused_without_deep_recursion() {
        // A 64 KiB stack: one frame per nesting level would not fit.
        let run = std::thread::Builder::new()
            .stack_size(64 * 1024)
            .spawn(|| {
                for (shape, text) in deep_records() {
                    assert!(text.len() as u64 <= MAX_RECORD_BYTES, "{shape}");
                    let e = parse_record(&text, "x").unwrap_err();
                    // Bare `[[[[…` is stopped by the scanner's own cap of
                    // 255 open brackets, reached while it looks ahead.
                    let why = if shape.contains("flow lists") {
                        "recursion limit exceeded"
                    } else {
                        "nested deeper than 3 levels"
                    };
                    assert!(e.contains(why), "{shape}: {e}");
                }
            })
            .unwrap();
        run.join().unwrap();
    }

    #[test]
    fn nesting_is_read_to_three_levels_and_no_deeper() {
        // Level 3 inside a key that is not a record field: ignored.
        let ok = "---\nid: x\nextra: {nested: [1, 2]}\nmore:\n  - [a, b]\n---\n";
        assert!(parse_record(ok, "x").is_ok());
        // Level 3 inside a record field: the field is wrongly typed.
        let e = parse_record("---\nid: x\nmembers: [[a]]\n---\n", "x").unwrap_err();
        assert!(e.contains("members is not a list"), "{e}");
        // Level 4 anywhere: the record is refused.
        for deep in [
            "---\nid: x\nextra: {a: {b: {c: 1}}}\n---\n",
            "---\nid: x\nextra: [[[a]]]\n---\n",
            "---\nid: x\nextra:\n  - - - a\n---\n",
        ] {
            let e = parse_record(deep, "x").unwrap_err();
            assert!(e.contains("nested deeper than 3 levels"), "{deep:?}: {e}");
        }
    }

    #[test]
    fn members_and_merge_chains_resolve_to_the_last_record() {
        let set = EventSet::from_records(
            PathBuf::new(),
            vec![
                rec("a", &["2026-09/E01"], Some("b")),
                rec("b", &["2026-10/E01"], Some("c")),
                rec("c", &["2026-11/E01", "2026-09/e01"], None),
                rec("d", &["2026-12/E01"], None),
            ],
        );
        assert!(set.warnings.is_empty(), "{:?}", set.warnings);
        for id in ["a", "B", "c", "2026-09/E01", "2026-10/e01", " 2026-11/E01 "] {
            assert_eq!(set.resolve(id), Some("c"), "{id}");
        }
        assert_eq!(set.resolve("2026-12/E01"), Some("d"));
        assert_eq!(set.resolve("2027-01/E01"), None);
        let r = set.report();
        assert_eq!((r.records, r.merged, r.members), (4, 2, 4));
    }

    #[test]
    fn cycles_and_dangling_merges_are_reported_and_not_followed() {
        let set = EventSet::from_records(
            PathBuf::new(),
            vec![
                rec("a", &["m/1"], Some("b")),
                rec("b", &["m/2"], Some("a")),
                rec("c", &["m/3"], Some("a")),
                rec("d", &["m/4"], Some("gone")),
                rec("e", &["m/5"], Some("e")),
            ],
        );
        // a and b close a cycle: each stands alone. c joins the cycle from
        // outside: it is merged into a, which is no longer merged.
        assert_eq!(set.resolve("m/1"), Some("a"));
        assert_eq!(set.resolve("m/2"), Some("b"));
        assert_eq!(set.resolve("m/3"), Some("a"));
        assert_eq!(set.resolve("d"), Some("d"));
        assert_eq!(set.resolve("m/5"), Some("e"));
        assert_eq!(set.records["c"].merged_into.as_deref(), Some("a"));
        assert!(set.records["a"].merged_into.is_none());
        let w = set.warnings.join("\n");
        for needle in [
            "analysis/events/a.md: merged_into \"b\" closes a cycle",
            "analysis/events/b.md: merged_into \"a\" closes a cycle",
            "analysis/events/d.md: merged_into \"gone\" names no record",
            "analysis/events/e.md: merged_into \"e\" closes a cycle",
        ] {
            assert!(w.contains(needle), "{needle}\n{w}");
        }
        assert_eq!(set.warnings.len(), 4);
    }

    #[test]
    fn a_long_chain_is_followed_to_its_end() {
        let n = 500;
        let recs = (0..n)
            .map(|i| {
                let next = (i + 1 < n).then(|| format!("r{:04}", i + 1));
                rec(&format!("r{i:04}"), &[&format!("m/{i}")], next.as_deref())
            })
            .collect();
        let set = EventSet::from_records(PathBuf::new(), recs);
        assert!(set.warnings.is_empty());
        assert_eq!(set.resolve("r0000"), Some("r0499"));
        assert_eq!(set.resolve("m/0"), Some("r0499"));
    }

    #[test]
    fn a_member_belongs_to_one_event_and_repeated_ids_are_skipped() {
        let set = EventSet::from_records(
            PathBuf::new(),
            vec![
                rec("a", &["m/1", "b"], None),
                rec("b", &["m/1", "m/2"], None),
                rec("B", &["m/9"], None),
            ],
        );
        assert_eq!(set.resolve("m/1"), Some("a"));
        assert_eq!(set.resolve("m/2"), Some("b"));
        assert_eq!(set.resolve("m/9"), None);
        assert_eq!(set.skipped, 1);
        let w = set.warnings.join("\n");
        assert!(w.contains("analysis/events/B.md: repeated or empty id"));
        assert!(w.contains("analysis/events/b.md: member \"m/1\" already belongs to \"a\""));
        assert!(w.contains("analysis/events/a.md: member \"b\" is a record id"));
    }
}

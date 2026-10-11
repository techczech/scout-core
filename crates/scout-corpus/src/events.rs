//! Lasting events: records kept BESIDE a corpus, read-only, one Markdown
//! file per event in `<corpus root>/analysis/events/<id>.md` (spec
//! `archive-scout/docs/specs/2026-10-11-lasting-events-spec.md`).
//!
//! - A record's YAML frontmatter is the contract: `id` (equal to the file
//!   stem), `name`, `aliases[]`, `start`, `end`, `members[]` (the monthly
//!   event ids folded into it, `2026-09/E21`), `themes[]`, `origin`,
//!   `edited`, `merged_into`, and the optional `removed` (the date he
//!   removed the event). The body is the description.
//! - A removed event keeps its record and its hold on its ids (`resolve`
//!   still gives it, so the seed makes no second record for its members);
//!   the topic layer takes it out of every item's events
//!   ([`EventSet::is_removed`]), and it is not listed. `removed` counts on
//!   the record an id resolves to; on a record merged into another it has
//!   no effect.
//! - Resolution: a monthly id resolves to the record that lists it in
//!   `members`; a record with `merged_into` resolves to the record it was
//!   merged into, followed to the end of the chain. A `merged_into` that
//!   names no record, or that closes a cycle, is reported and that record
//!   is treated as not merged.
//! - A merge is that one field. The record merged into holds no copy of
//!   the other's members, aliases or dates. What the combined event shows
//!   beyond its own record is derived here, from the records, each time:
//!   its ids by the resolution above, its span and aliases and the list
//!   of records merged into it by [`combined`] (which `scout events`
//!   lists). Themes, description, name, origin, `edited` and `removed`
//!   are the surviving record's own.
//! - A malformed record (a control character anywhere in the file, no
//!   frontmatter, bad YAML, no `id`, an `id` that is not the file stem, a
//!   repeated id, a wrongly typed field, YAML nested deeper than
//!   [`MAX_YAML_DEPTH`]) is skipped with a warning naming its file; the
//!   rest is read.
//! - Nothing here recurses over record content. One file's bytes are
//!   parsed in `events/record.rs` (the frontmatter is pulled one parser
//!   event at a time and the pull stops at the first value that nests too
//!   deep); merge chains are walked in loops bounded by the number of
//!   records.
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

mod combined;
mod record;
pub(crate) use combined::is_date;
pub use combined::{combined, cover, merged_in, Combined, MergedFrom};
use record::TOO_LARGE;
pub use record::{
    is_record_control, parse_bytes, parse_record, record_stem, MAX_RECORD_BYTES, MAX_YAML_DEPTH,
};

/// The records folder, relative to the corpus root.
pub const EVENTS_DIR: &str = "analysis/events";
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
    /// The date the event was removed (`scout events remove`); absent on
    /// every other record, in the file and in the index copy alike.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub removed: Option<String>,
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
    /// Each skipped file with the reason; `skipped` of them.
    pub refused: Vec<Refused>,
    /// One line per skipped file, unusable `merged_into` or member conflict.
    pub warnings: Vec<String>,
    /// Records whose `merged_into` was cleared here, with the value their
    /// file still holds.
    pub unmerged: Vec<Unmerged>,
}

/// A record whose `merged_into` is not in effect: it names no record or
/// closes a cycle. The file keeps the value; the set's record has `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unmerged {
    /// The record's id.
    pub id: String,
    /// The `merged_into` of its file.
    pub merged_into: String,
    /// `names no record` or `closes a cycle`.
    pub why: &'static str,
}

/// A record file the reader skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    /// The file's name in the records folder.
    pub file: String,
    pub why: String,
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
                let why = format!("repeated or empty id {:?}", r.id);
                set.skipped += 1;
                set.warnings.push(format!("{}: {why}", file_of(&r.id)));
                set.refused.push(Refused {
                    file: format!("{}.md", r.id),
                    why,
                });
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
                let was = r.merged_into.take().unwrap_or_default();
                self.warnings.push(format!(
                    "{}: merged_into {was:?} {why}; treated as not merged",
                    file_of(&r.id),
                ));
                self.unmerged.push(Unmerged {
                    id: r.id.clone(),
                    merged_into: was,
                    why,
                });
            }
        }
    }

    /// The records as their files have them: each `merged_into` this set
    /// cleared ([`EventSet::unmerged`]) put back. What a later read of the
    /// folder starts from, whatever records it holds by then.
    pub fn as_on_disk(&self) -> Vec<EventRecord> {
        let mut records = self.records.clone();
        for u in &self.unmerged {
            if let Some(r) = records.get_mut(&fold(&u.id)) {
                r.merged_into = Some(u.merged_into.clone());
            }
        }
        records.into_values().collect()
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

    /// Is the event `id` resolves to (or names) a removed one? False for
    /// an id no record holds.
    pub fn is_removed(&self, id: &str) -> bool {
        self.resolve(id)
            .and_then(|last| self.records.get(&fold(last)))
            .is_some_and(|r| r.removed.is_some())
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
fn scan(dir: &Path, hash: &mut Sha1, mut each: impl FnMut(&str, Entry)) -> Result<bool> {
    let rd = match std::fs::read_dir(dir) {
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
        if record_stem(name).is_none() {
            continue;
        }
        hash.update(name.as_bytes());
        hash.update([0u8]);
        // Not followed: a record is a plain file of this folder.
        let entry = match std::fs::symlink_metadata(&path) {
            Ok(m) if !m.is_file() => Entry::Refused("not a plain file"),
            Ok(m) if m.len() > MAX_RECORD_BYTES => Entry::Refused(TOO_LARGE),
            Ok(_) => match read_capped(&path) {
                Ok(Some(b)) => Entry::Bytes(b),
                Ok(None) => Entry::Refused(TOO_LARGE),
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
pub(crate) fn read_capped(path: &Path) -> std::io::Result<Option<Vec<u8>>> {
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
    scan(&root.join(EVENTS_DIR), hash, |_, _| {})
}

/// Read the lasting events of the corpus at `root`; `None` when it has no
/// records folder.
pub fn read(root: &Path) -> Result<Option<EventSet>> {
    read_hashed(root, &mut Sha1::new())
}

/// [`read`] over a records folder given directly (`scout events seed
/// --out`); `None` when there is no such folder. Warnings still label
/// files as `analysis/events/<name>`.
pub fn read_dir(dir: &Path) -> Result<Option<EventSet>> {
    read_dir_hashed(dir, &mut Sha1::new())
}

/// [`read`], feeding `hash` exactly the bytes it read.
pub(crate) fn read_hashed(root: &Path, hash: &mut Sha1) -> Result<Option<EventSet>> {
    read_dir_hashed(&root.join(EVENTS_DIR), hash)
}

/// A set and, by file name, the bytes each of its records was read from.
pub(crate) type SetWithBytes = (EventSet, BTreeMap<String, Vec<u8>>);

/// [`read_dir`], with the bytes each record was read from, by file name
/// (only the files read as records). `scout events` edits compare a file
/// with these bytes before replacing it.
pub(crate) fn read_dir_with_bytes(dir: &Path) -> Result<Option<SetWithBytes>> {
    let mut bytes = BTreeMap::new();
    let set = read_dir_kept(dir, &mut Sha1::new(), Some(&mut bytes))?;
    Ok(set.map(|s| (s, bytes)))
}

fn read_dir_hashed(dir: &Path, hash: &mut Sha1) -> Result<Option<EventSet>> {
    read_dir_kept(dir, hash, None)
}

fn read_dir_kept(
    dir: &Path,
    hash: &mut Sha1,
    mut keep: Option<&mut BTreeMap<String, Vec<u8>>>,
) -> Result<Option<EventSet>> {
    let mut records = Vec::new();
    let mut refused: Vec<Refused> = Vec::new();
    let found = scan(dir, hash, |name, entry| {
        let parsed = match entry {
            Entry::Refused(why) => Err(why.to_string()),
            Entry::Bytes(b) => {
                let parsed = parse_bytes(&b, record_stem(name).unwrap_or(name));
                if let (Ok(_), Some(keep)) = (&parsed, keep.as_deref_mut()) {
                    keep.insert(name.to_string(), b);
                }
                parsed
            }
        };
        match parsed {
            Ok(r) => records.push(r),
            Err(why) => refused.push(Refused {
                file: name.to_string(),
                why,
            }),
        }
    })?;
    if !found {
        return Ok(None);
    }
    let mut set = EventSet::from_records(dir.to_path_buf(), records);
    set.skipped += refused.len();
    let mut warnings: Vec<String> = refused
        .iter()
        .map(|r| format!("{EVENTS_DIR}/{}: {}; skipped", r.file, r.why))
        .collect();
    warnings.append(&mut set.warnings);
    set.warnings = warnings;
    refused.append(&mut set.refused);
    set.refused = refused;
    Ok(Some(set))
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

//! Reading the monthly briefings of an archive: the list of them
//! ([`briefings`]) and one of them with each of its events linked to the
//! lasting event record that holds it now ([`briefing`]).
//!
//! Read only. Nothing is written, no lock is taken, and neither the
//! registry nor an index is read: the two calls work on an archive folder
//! as `scout events seed` does.
//!
//! What is read:
//!
//! - `<archive>/analysis/context/briefings/<period>-briefing.json`, period
//!   `YYYY-MM` or `YYYY-Qn`: plain files directly in that folder. A name
//!   holding `.superseded-` is ignored, as the seed ignores it. A symlink,
//!   a folder or a special file under a briefing's name is refused, and so
//!   is a file over [`MAX_BRIEFING_BYTES`]. Each of `analysis`, `context`
//!   and `briefings` must itself be a folder, not a link to one, so that
//!   nothing outside the archive is read; the archive path itself may be
//!   a link.
//! - for [`briefing`] of a briefing that has events, the records folder
//!   `<archive>/analysis/events`, by the records reader ([`events::read`]),
//!   whose resolution (`members`, `merged_into`) says which record holds a
//!   monthly id now.
//!
//! Shared with the seed, not copied: the folder and the file-name rule
//! (`sources::BRIEFINGS_DIR`, `sources::file_period`), the period rule
//! (`naming::is_period`), the reading of one event (`sources::parse_event`:
//! the id in the index's spelling, name, aliases, description) and the
//! dropping of control characters (`render::body_text`).
//!
//! Text is returned as stored, with no Markdown rendering. Every text has
//! passed [`body_text`]; a value that is one line by nature (a name, an
//! alias, a date, a title) also has its whitespace runs as one space. The
//! id and the name of a linked record are cleaned the same way.

use crate::event_seed::naming::{is_period, period_month};
use crate::event_seed::refuse_parent_dir;
use crate::event_seed::render::body_text;
use crate::event_seed::sources::{file_period, one_line, parse_event, BRIEFINGS_DIR};
use crate::events::{self, EventSet, EVENTS_DIR};
use crate::normalize::fold;
use anyhow::{anyhow, bail, Context, Result};
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};

/// A briefing file larger than this is refused, unread (4 MiB).
pub const MAX_BRIEFING_BYTES: u64 = 4 << 20;
/// What a briefing file's name ends with, after its period.
const SUFFIX: &str = "-briefing.json";
/// Record files named in one refusal (the total is always said).
const MAX_FAULTS: usize = 20;

/// A JSON object of a briefing file.
type Object = Map<String, Value>;

/// What a period is: `2026-09` is a month, `2022-Q2` a quarter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PeriodKind {
    Month,
    Quarter,
}

/// One row of [`briefings`]: a briefing, or a file under a briefing's name
/// that cannot be read as one (`{"error": …}` and no other value).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum BriefingListing {
    Briefing(BriefingSummary),
    Unreadable {
        /// One line, naming the file.
        error: String,
    },
}

/// A briefing in the list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BriefingSummary {
    pub period: String,
    pub kind: PeriodKind,
    /// `meta.model`; empty when the file has none.
    pub model: String,
    /// `meta.generated`; empty when the file has none.
    pub generated: String,
    /// `meta.items`: the archive items the briefing was written from.
    pub items: Option<u64>,
    /// How many events it has.
    pub events: usize,
}

/// One briefing, each event linked to its lasting event record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Briefing {
    pub period: String,
    pub kind: PeriodKind,
    pub model: String,
    pub generated: String,
    pub items: Option<u64>,
    pub overview: String,
    /// In the file's order.
    pub events: Vec<BriefingEvent>,
    pub names: Vec<BriefingName>,
    pub debates: Vec<BriefingDebate>,
    pub engagement: String,
}

/// One event of a briefing, as the briefing wrote it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BriefingEvent {
    /// `<period>/E<nn>`, in the spelling the index uses.
    pub monthly_id: String,
    pub name: String,
    pub date: String,
    pub aliases: Vec<String>,
    pub description: String,
    /// The lasting event record that holds this monthly id now; `None`
    /// when no record holds it.
    pub record: Option<EventLink>,
}

/// The lasting event record a monthly event leads to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EventLink {
    pub id: String,
    pub name: String,
    /// The record carries `removed`.
    pub removed: bool,
}

/// One entry of a briefing's names and aliases.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BriefingName {
    pub name: String,
    pub aliases: Vec<String>,
    pub note: String,
}

/// One debate of a briefing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BriefingDebate {
    pub title: String,
    pub summary: String,
    pub when: String,
}

/// The error of [`briefing`] for a valid period the archive has no
/// briefing file for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoBriefing(pub String);

impl std::fmt::Display for NoBriefing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "No briefing for {}.", self.0)
    }
}

impl std::error::Error for NoBriefing {}

/// The briefings of the archive, newest period first (a quarter sorts by
/// its first month). One row per entry of the briefings folder named
/// `<period>-briefing.json`; an entry that cannot be read as a briefing
/// gives an error row and the others still list. Empty when the archive
/// has no briefings folder. The records folder is not read.
pub fn briefings(archive: &Path) -> Result<Vec<BriefingListing>> {
    let Some(dir) = briefings_dir(archive)? else {
        return Ok(Vec::new());
    };
    let rd = match std::fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("read {}", dir.display())),
    };
    let mut periods = Vec::new();
    for e in rd {
        let e = e.with_context(|| format!("read {}", dir.display()))?;
        if let Some(period) = e.file_name().to_str().and_then(|n| file_period(n, SUFFIX)) {
            periods.push(period.to_string());
        }
    }
    // Newest first; a quarter and its first month in a fixed order.
    periods.sort_by(|a, b| (period_month(b), b).cmp(&(period_month(a), a)));
    Ok(periods
        .iter()
        .map(|period| match load(&dir, period) {
            Ok(Some(b)) => BriefingListing::Briefing(BriefingSummary {
                period: b.period,
                kind: b.kind,
                model: b.model,
                generated: b.generated,
                items: b.items,
                events: b.events.len(),
            }),
            Ok(None) => BriefingListing::Unreadable {
                error: format!("{}: no longer there", file_of(&dir, period).display()),
            },
            Err(e) => BriefingListing::Unreadable {
                error: line(&format!("{e:#}")),
            },
        })
        .collect())
}

/// The briefing of `period` (`YYYY-MM` or `YYYY-Qn`), each event with the
/// lasting event record that holds it now.
///
/// Errors: a `period` of another shape, before anything is looked at;
/// [`NoBriefing`] when the archive has no briefing file of that period; a
/// file that is not a plain file, is over [`MAX_BRIEFING_BYTES`] or is not
/// a briefing, with the file and the field named; and, for a briefing with
/// events, a missing records folder or a record file the records reader
/// skips (a link is never left out silently).
pub fn briefing(archive: &Path, period: &str) -> Result<Briefing> {
    // The period is only ever joined into a path after this.
    if !is_period(period) {
        bail!("\"{period}\" is not a month (YYYY-MM) or a quarter (YYYY-Qn).");
    }
    let none = || NoBriefing(period.to_string());
    let dir = briefings_dir(archive)?.ok_or_else(none)?;
    let mut b = load(&dir, period)?.ok_or_else(none)?;
    if b.events.is_empty() {
        return Ok(b);
    }
    let set = records(archive)?;
    for ev in &mut b.events {
        ev.record = set
            .resolve(&ev.monthly_id)
            .and_then(|last| set.records.get(&fold(last)))
            .map(|r| EventLink {
                // The reader decodes YAML escapes after its look for
                // control characters, so a name can hold one; an id is
                // the file's name, which can too.
                id: line(&r.id),
                name: line(&r.name),
                removed: r.removed.is_some(),
            });
    }
    Ok(b)
}

/// The briefings folder of `archive`, after the seed's checks of the
/// archive path (no `..` component; a folder); `None` when the archive
/// has none.
///
/// The archive path may be reached through a link: the caller chose it.
/// Each folder below it on the way (`analysis`, `context`, `briefings`)
/// must itself be a folder: a link there, even to a folder, would have
/// files outside the archive read as its briefings, and is refused with
/// the path named, as is a file. The first one that is not there means
/// no briefings.
fn briefings_dir(archive: &Path) -> Result<Option<PathBuf>> {
    refuse_parent_dir(archive, "the archive")?;
    if !archive.is_dir() {
        bail!("{}: the archive is not a folder", archive.display());
    }
    let mut dir = archive.to_path_buf();
    for part in Path::new(BRIEFINGS_DIR).components() {
        dir.push(part);
        match std::fs::symlink_metadata(&dir) {
            Ok(m) if m.is_dir() => {}
            Ok(_) => bail!(
                "{}: not a plain folder; the briefings were not read",
                dir.display()
            ),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).with_context(|| format!("read {}", dir.display())),
        }
    }
    Ok(Some(dir))
}

/// Where the briefing of a valid `period` is.
fn file_of(dir: &Path, period: &str) -> PathBuf {
    dir.join(format!("{period}{SUFFIX}"))
}

/// The records of the archive, every one of them read. A missing folder
/// and a file the reader skips are errors here: either could hold the
/// monthly id asked about. The folder is found and read by the records
/// reader ([`events::read`]), under its own rules; the path joined here
/// only names it in an error.
fn records(archive: &Path) -> Result<EventSet> {
    let dir = archive.join(EVENTS_DIR);
    let Some(set) = events::read(archive)? else {
        bail!(
            "{}: no records folder (run `scout events seed` first)",
            dir.display()
        );
    };
    if set.refused.is_empty() {
        return Ok(set);
    }
    let mut lines: Vec<String> = set
        .refused
        .iter()
        .take(MAX_FAULTS)
        .map(|r| {
            format!(
                "{}: cannot read the record ({}); fix or remove it first",
                dir.join(&r.file).display(),
                r.why
            )
        })
        .collect();
    if set.refused.len() > MAX_FAULTS {
        lines.push(format!("and {} more", set.refused.len() - MAX_FAULTS));
    }
    Err(crate::ErrorLines(lines).into())
}

/// The briefing of a valid `period` in `dir`, its events not yet linked;
/// `None` when the folder has no entry of its name. Every error names the
/// file.
fn load(dir: &Path, period: &str) -> Result<Option<Briefing>> {
    let path = file_of(dir, period);
    let Some(bytes) = read_plain(&path)? else {
        return Ok(None);
    };
    parse(&bytes, period)
        .map(Some)
        .map_err(|why| anyhow!("{}: {why}", path.display()))
}

/// What [`open_plain`] says of a path that is not a plain file.
const NOT_PLAIN: &str = "not a plain file";

/// Open `path` for reading only when the entry itself is a regular file.
///
/// A link is not followed, whatever it leads to: the open carries
/// `O_NOFOLLOW`, so a link at the path fails at the open itself, also one
/// put there after the caller looked at the path. The opened file must
/// then itself be a regular file. Either refusal is an `InvalidInput`
/// error that says [`NOT_PLAIN`]. No other flag is added (no
/// `O_NONBLOCK`): a regular file is opened exactly as it always was.
///
/// The one gap: another process that puts a FIFO at the path between the
/// caller's look at it and this open can still make the open wait. The
/// engine does not defend against a concurrent hostile writer in the
/// user's own archive.
#[cfg(unix)]
fn open_plain(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    let not_plain = || std::io::Error::new(std::io::ErrorKind::InvalidInput, NOT_PLAIN);
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|e| match e.raw_os_error() {
            // What an open with O_NOFOLLOW gives for a link (EMLINK on
            // some BSDs).
            Some(libc::ELOOP | libc::EMLINK) => not_plain(),
            _ => e,
        })?;
    if !file.metadata()?.is_file() {
        return Err(not_plain());
    }
    Ok(file)
}

/// Without `O_NOFOLLOW`: the shared open of a regular file, which follows
/// a link put at the path after the caller looked at it.
#[cfg(not(unix))]
fn open_plain(path: &Path) -> std::io::Result<std::fs::File> {
    crate::correction_lines::open_regular(path)
}

/// The bytes of the plain file at `path`; `None` when nothing is there.
/// The entry itself must be a plain file (a link is not followed, as in
/// the seed): it is looked at first, so that a folder or a special file
/// already there is refused without being opened, then opened by
/// [`open_plain`], and its size is taken from the open file.
fn read_plain(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_file() => {}
        Ok(_) => bail!("{}: {NOT_PLAIN}", path.display()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("read {}", path.display())),
    }
    let file = match open_plain(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::InvalidInput => {
            bail!("{}: {NOT_PLAIN}", path.display())
        }
        Err(e) => return Err(e).with_context(|| format!("read {}", path.display())),
    };
    let too_large = |bytes: String| {
        anyhow!(
            "{}: {bytes} bytes, larger than the limit of {MAX_BRIEFING_BYTES} bytes (4 MiB); it was not read",
            path.display()
        )
    };
    let len = file
        .metadata()
        .with_context(|| format!("read {}", path.display()))?
        .len();
    if len > MAX_BRIEFING_BYTES {
        return Err(too_large(len.to_string()));
    }
    let mut bytes = Vec::new();
    file.take(MAX_BRIEFING_BYTES + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("read {}", path.display()))?;
    // Grown since it was measured.
    if bytes.len() as u64 > MAX_BRIEFING_BYTES {
        return Err(too_large(format!("over {MAX_BRIEFING_BYTES}")));
    }
    Ok(Some(bytes))
}

/// Prose as stored, without the characters the seed drops from a
/// description; the ends trimmed.
fn prose(s: &str) -> String {
    body_text(s)
}

/// [`prose`] on one line: whitespace runs as one space.
fn line(s: &str) -> String {
    one_line(&body_text(s))
}

/// The text at `key` of `obj`: empty when absent or null, an error naming
/// `what` when it is anything else.
fn text<'a>(obj: &'a Object, key: &str, what: &str) -> Result<&'a str, String> {
    match obj.get(key) {
        None | Some(Value::Null) => Ok(""),
        Some(Value::String(s)) => Ok(s),
        Some(_) => Err(format!("{what} is not text")),
    }
}

/// The list at `key` of `doc`, each entry an object: empty when absent or
/// null. `one` names an entry in an error (`name`, `debate`).
fn objects<'a>(doc: &'a Object, key: &str, one: &str) -> Result<Vec<(usize, &'a Object)>, String> {
    let list = match doc.get(key) {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(l)) => l,
        Some(_) => return Err(format!("\"{key}\" is not a list")),
    };
    list.iter()
        .enumerate()
        .map(|(i, v)| {
            v.as_object()
                .map(|o| (i + 1, o))
                .ok_or_else(|| format!("{one} {} of the list is not an object", i + 1))
        })
        .collect()
}

/// The briefing in a file's bytes, its events not linked, or what is wrong
/// with the file.
fn parse(bytes: &[u8], period: &str) -> Result<Briefing, String> {
    let doc: Value = serde_json::from_slice(bytes).map_err(|e| format!("not JSON ({e})"))?;
    let Some(doc) = doc.as_object() else {
        return Err("not a JSON object".into());
    };
    let Some(meta) = doc.get("meta").and_then(Value::as_object) else {
        return Err("no \"meta\" object".into());
    };
    let month = meta.get("month").unwrap_or(&Value::Null);
    if month.as_str() != Some(period) {
        return Err(format!(
            "\"meta.month\" is {month}, the file name says {period}"
        ));
    }
    let items = match meta.get("items") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            v.as_u64()
                .ok_or("\"meta.items\" is not a whole number of items")?,
        ),
    };
    let Some(list) = doc.get("events").and_then(Value::as_array) else {
        return Err("no \"events\" list".into());
    };
    let mut seen = BTreeSet::new();
    let mut events = Vec::with_capacity(list.len());
    for (i, ev) in list.iter().enumerate() {
        let (id, read) = parse_event(ev, i, period)?;
        if !seen.insert(id.clone()) {
            return Err(format!("event {id} appears twice"));
        }
        // `parse_event` has seen that this entry is an object.
        let date = match ev.get("date") {
            None | Some(Value::Null) => "",
            Some(Value::String(s)) => s,
            Some(_) => return Err(format!("event {id}: \"date\" is not text")),
        };
        events.push(BriefingEvent {
            name: line(&read.name),
            date: line(date),
            aliases: lines(read.aliases.iter().map(String::as_str)),
            description: read.description,
            monthly_id: id,
            record: None,
        });
    }
    let mut names = Vec::new();
    for (n, o) in objects(doc, "names", "name")? {
        let aliases = match o.get("aliases") {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(l)) => lines(
                l.iter()
                    .map(|a| a.as_str())
                    .collect::<Option<Vec<&str>>>()
                    .ok_or_else(|| format!("name {n}: \"aliases\" is not a list of text"))?,
            ),
            Some(_) => return Err(format!("name {n}: \"aliases\" is not a list of text")),
        };
        names.push(BriefingName {
            name: line(text(o, "name", &format!("name {n}: \"name\""))?),
            aliases,
            note: prose(text(o, "note", &format!("name {n}: \"note\""))?),
        });
    }
    let mut debates = Vec::new();
    for (n, o) in objects(doc, "debates", "debate")? {
        debates.push(BriefingDebate {
            title: line(text(o, "title", &format!("debate {n}: \"title\""))?),
            summary: prose(text(o, "summary", &format!("debate {n}: \"summary\""))?),
            when: line(text(o, "when", &format!("debate {n}: \"when\""))?),
        });
    }
    Ok(Briefing {
        period: period.to_string(),
        kind: if period.contains('Q') {
            PeriodKind::Quarter
        } else {
            PeriodKind::Month
        },
        model: line(text(meta, "model", "\"meta.model\"")?),
        generated: line(text(meta, "generated", "\"meta.generated\"")?),
        items,
        overview: prose(text(doc, "overview", "\"overview\"")?),
        events,
        names,
        debates,
        engagement: prose(text(doc, "engagement", "\"engagement\"")?),
    })
}

/// Each of `texts` as a [`line`], the empty ones left out.
fn lines<'a>(texts: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    texts
        .into_iter()
        .map(line)
        .filter(|a| !a.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_briefing_is_read_as_stored_without_control_characters() {
        let bytes = serde_json::json!({
            "meta": {"month": "2022-Q2", "model": "m\u{1b}1", "items": 7},
            "overview": "  One\u{7}.\n\nTwo.  ",
            "events": [{"id": "e02", "name": " A \u{1b}[31m b ", "date": "2022-04-03",
                        "aliases": ["x\ny", ""], "description": "D\u{0}.", "items": ["m0001"]}],
            "names": [{"name": "N", "aliases": ["a  b"], "note": "what\u{9c} it is"}],
            "debates": [{"title": "T", "summary": "S", "when": "mid-April"}],
            "problems": ["ignored"],
        })
        .to_string();
        let b = parse(bytes.as_bytes(), "2022-Q2").unwrap();
        assert_eq!(b.kind, PeriodKind::Quarter);
        assert_eq!((b.model.as_str(), b.items), ("m1", Some(7)));
        assert_eq!(b.overview, "One.\n\nTwo.");
        assert_eq!(b.engagement, "");
        let e = &b.events[0];
        assert_eq!(e.monthly_id, "2022-Q2/E02");
        assert_eq!(e.name, "A [31m b");
        assert_eq!(e.aliases, ["x y"]);
        assert_eq!(e.description, "D.");
        assert_eq!(e.record, None);
        assert_eq!(b.names[0].aliases, ["a b"]);
        assert_eq!(b.names[0].note, "what it is");
        assert_eq!(b.debates[0].when, "mid-April");
        // The briefing's own item ids are not part of what is returned.
        assert!(!serde_json::to_string(&b).unwrap().contains("m0001"));
    }

    /// The open itself, with no look at the path before it (the look
    /// `read_plain` takes first would refuse these on its own).
    #[cfg(unix)]
    #[test]
    fn the_open_does_not_follow_a_link_and_opens_a_file_as_ever() {
        use std::os::fd::AsRawFd;
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("real.json");
        std::fs::write(&real, "{}").unwrap();
        let link = tmp.path().join("link.json");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let e = open_plain(&link).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::InvalidInput);
        assert_eq!(e.to_string(), NOT_PLAIN);
        // A folder opens for reading; the opened handle is what is refused.
        let e = open_plain(tmp.path()).unwrap_err();
        assert_eq!(e.to_string(), NOT_PLAIN);
        let e = open_plain(&tmp.path().join("none")).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::NotFound);

        let mut text = String::new();
        let mut file = open_plain(&real).unwrap();
        file.read_to_string(&mut text).unwrap();
        assert_eq!(text, "{}");
        // A blocking open, as a plain `File::open`.
        let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
        assert_eq!(flags & libc::O_NONBLOCK, 0);
    }

    #[test]
    fn what_is_not_a_briefing_is_named_by_field() {
        for (doc, why) in [
            ("[]", "not a JSON object"),
            ("{", "not JSON"),
            (r#"{"events": []}"#, "no \"meta\" object"),
            (
                r#"{"meta": {}, "events": []}"#,
                "\"meta.month\" is null, the file name says 2026-09",
            ),
            (
                r#"{"meta": {"month": "2026-09", "items": "9"}, "events": []}"#,
                "\"meta.items\" is not a whole number",
            ),
            (
                r#"{"meta": {"month": "2026-09"}, "events": [], "names": {}}"#,
                "\"names\" is not a list",
            ),
            (
                r#"{"meta": {"month": "2026-09"}, "events": [], "names": [{"aliases": [1]}]}"#,
                "name 1: \"aliases\" is not a list of text",
            ),
            (
                r#"{"meta": {"month": "2026-09"}, "events": [], "debates": [1]}"#,
                "debate 1 of the list is not an object",
            ),
            (
                r#"{"meta": {"month": "2026-09"}, "events": [], "debates": [{"when": 2}]}"#,
                "debate 1: \"when\" is not text",
            ),
        ] {
            let got = parse(doc.as_bytes(), "2026-09").unwrap_err();
            assert!(got.contains(why), "{doc}: {got}");
        }
    }
}

//! The two append-only files of the records folder, `merge-log.jsonl` and
//! `merge-decisions.jsonl`: their only reader and their only writer. The
//! file handling ([`Jsonl`]: the read, the checks and the append) is also
//! what the writer of `analysis/topics/corrections.jsonl` stands on
//! ([`crate::corrections`]), with limits of its own.
//!
//! Reading. A file is read whole before anything is written. It must be a
//! plain file (not a symlink: the open does not follow one) of at most
//! 256 MiB ([`MAX_LOG_BYTES`]), UTF-8, every line at most 4 MiB with its
//! newline ([`MAX_LINE_BYTES`]) and a complete JSON object of the file's
//! kind, the last line ended by a newline; anything else stops the command
//! with the file and the line named. A hard link is read like any plain
//! file: reading writes nowhere.
//!
//! Appending. A log is a write target, so more is asked of it. The file
//! is opened without following a link, and the checks are made on the
//! open descriptor, before the write: a regular file, with exactly one
//! link (an append to a file with a second name would also change that
//! other name's contents, which may lie outside the file's folder), the
//! file that was read, at the length it was read at. The line with its
//! newline must be at most 4 MiB, and the file with the line at most
//! 256 MiB, so no append leaves a log this module then refuses to read.
//! The object and its newline go out in one `write` call, then the file
//! is synced. A log that is not there is created (create-new, so the name
//! cannot be a link of any kind) with the default mode of a new file, so
//! its permission bits are what the process's umask leaves of 0666; no
//! mode is set on it afterwards. Nothing is ever rewritten or removed.
//!
//! Lines. A merge line must carry every key of [`MergeLine`], `reason`
//! and `a_merged_into_before` included (their value may be null); an undo
//! line every key of [`UndoLine`]; a decision line every key of
//! [`DecisionLine`]. A line without one of its keys, or with a key of no
//! kind, is malformed.
//!
//! One function, two callers. Every check above is made by
//! [`Jsonl::check`], and by nothing else: the append calls it and then
//! writes to the descriptor it opened; "would an append succeed?" is the
//! same call without the write ([`Jsonl::ready`],
//! [`MergeLog::ready_to_append`], [`MergeLog::ready_for_undo`],
//! [`Decisions::ready`]). The two differ in one place only: where no file
//! is there, the append creates it and the question only sees that the
//! name is still free. `unmerge` and `decide` append their line last,
//! after records have changed, so they ask before their first write;
//! `merge` asks before its first write too, and `apply-proposals` asks for
//! every merge it plans, with and without `--apply`.
//!
//! A check gives two answers, kept apart. The line: its length, and the
//! file's length with it (and, for the merge log, whether an id is left).
//! The file, whatever the line: its type, its link count, whether it is
//! the file that was read at the length read, whether it opens for
//! append.
//!
//! Ids of the merge log are canonical numbered ids
//! ([`crate::numbered_id`]): `m` and the number written with at least six
//! digits and no more than it needs, so at most `m999999999`
//! ([`MAX_ID`]), and one spelling per number (`m0000001` is not an id). A
//! line with any other id is malformed. When
//! the last line holds the largest id, no further line can be written and
//! `merge` and `unmerge` refuse.

use super::store::{ident_of, links_of, no_follow, read_plain};
use anyhow::{bail, Context, Result};
use serde::{de::DeserializeOwned, Deserialize, Deserializer, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const MERGE_LOG: &str = "merge-log.jsonl";
pub const DECISIONS_LOG: &str = "merge-decisions.jsonl";
/// Largest log file read, and the largest an append may leave.
pub(super) const MAX_LOG_BYTES: u64 = 256 << 20;
/// Largest line, with its newline, read or written.
pub(super) const MAX_LINE_BYTES: u64 = 4 << 20;
/// Largest number of a merge-log id.
pub(super) const MAX_ID: u64 = crate::numbered_id::MAX;

/// The size limits of one append-only file.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Caps {
    /// Largest file read, and the largest an append may leave.
    pub file: u64,
    /// Largest line, with its newline, read or written.
    pub line: u64,
}

/// A key that must be on the line; its value may be null. (An `Option`
/// field read by serde's own rule is `None` when the key is missing.)
fn present<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(d)
}

const CAPS: Caps = Caps {
    file: MAX_LOG_BYTES,
    line: MAX_LINE_BYTES,
};

/// Who made a merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum By {
    Dominik,
    Model,
}

/// One merge: `a` merged into `b`. The merge wrote `merged_into: b` on
/// record `a` and nothing else; the line holds no copy of either record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MergeLine {
    /// `m000001`, `m000002` …: one sequence for merge and undo lines.
    pub id: String,
    /// When the line was written, UTC.
    pub at: String,
    pub by: By,
    /// Null when no reason was given; the key is always there.
    #[serde(deserialize_with = "present")]
    pub reason: Option<String>,
    /// The record merged away.
    pub a: String,
    /// The surviving record.
    pub b: String,
    /// `a`'s `merged_into` before the merge. Always null: a record that is
    /// already merged into another is not merged. A line with any other
    /// value, or without the key, is malformed.
    #[serde(deserialize_with = "present")]
    pub a_merged_into_before: Option<String>,
}

/// The undo of one merge line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UndoLine {
    pub id: String,
    pub at: String,
    pub undoes: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogLine {
    Merge(Box<MergeLine>),
    Undo(UndoLine),
}

/// One decision on a proposal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DecisionLine {
    pub at: String,
    /// The proposals file's name inside `merge-proposals/`.
    pub file: String,
    pub a: String,
    pub b: String,
    pub decision: Decision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    Approve,
    Dismiss,
}

/// An append-only file as one read gave it.
pub(crate) struct Jsonl {
    pub path: PathBuf,
    /// Device and inode, and the length read; `None` when no file is there.
    read: Option<((u64, u64), u64)>,
    /// Each line's JSON object.
    pub values: Vec<serde_json::Value>,
    /// The bytes read, when the reader asked to keep them
    /// ([`Jsonl::read_keeping`]); empty when no file is there.
    pub bytes: Vec<u8>,
    caps: Caps,
}

impl Jsonl {
    fn read(dir: &Path, name: &str) -> Result<Jsonl> {
        Jsonl::read_with(dir, name, CAPS)
    }

    fn read_with(dir: &Path, name: &str, caps: Caps) -> Result<Jsonl> {
        let mut file = Jsonl::read_keeping(dir, name, caps)?;
        file.bytes = Vec::new();
        Ok(file)
    }

    /// Read `<dir>/<name>` whole and check it (see the notes of this
    /// file), keeping the bytes read.
    pub(crate) fn read_keeping(dir: &Path, name: &str, caps: Caps) -> Result<Jsonl> {
        let path = dir.join(name);
        let Some(plain) = read_plain(&path, caps.file)? else {
            return Ok(Jsonl {
                path,
                read: None,
                values: vec![],
                bytes: vec![],
                caps,
            });
        };
        let at = path.display();
        let fix = "fix or remove the line by hand; nothing was changed";
        let Ok(text) = std::str::from_utf8(&plain.bytes) else {
            bail!("{at}: not valid UTF-8; {fix}");
        };
        if !text.is_empty() && !text.ends_with('\n') {
            bail!(
                "{at}: line {}: the last line is not ended by a newline (an append that was cut short?); {fix}",
                text.lines().count()
            );
        }
        let mut values = Vec::new();
        for (i, line) in text.split_terminator('\n').enumerate() {
            // With its newline.
            if line.len() as u64 >= caps.line {
                bail!(
                    "{at}: line {}: longer than {} bytes; {fix}",
                    i + 1,
                    caps.line
                );
            }
            match serde_json::from_str::<serde_json::Value>(line) {
                Ok(v) if v.is_object() => values.push(v),
                Ok(_) => bail!("{at}: line {}: not a JSON object; {fix}", i + 1),
                Err(e) => bail!("{at}: line {}: not JSON ({e}); {fix}", i + 1),
            }
        }
        Ok(Jsonl {
            path,
            read: Some((plain.ident, plain.bytes.len() as u64)),
            values,
            bytes: plain.bytes,
            caps,
        })
    }

    fn typed<T: DeserializeOwned>(&self, i: usize, what: &str) -> Result<T> {
        serde_json::from_value(self.values[i].clone()).map_err(|e| {
            anyhow::anyhow!(
                "{}: line {}: not {what} ({e}); fix or remove the line by hand; nothing was changed",
                self.path.display(),
                i + 1
            )
        })
    }

    /// `line` as the bytes an append writes: one line of JSON and its
    /// newline. Refused when the line is over the line limit, or when the
    /// file with it would be over the file limit (it could not be read
    /// again).
    pub(crate) fn text_of<T: Serialize>(&self, line: &T) -> Result<String> {
        self.text_after(line, 0)
    }

    /// [`Jsonl::text_of`] for a file that has grown by `planned` bytes
    /// since it was read (lines a run means to append before this one).
    fn text_after<T: Serialize>(&self, line: &T, planned: u64) -> Result<String> {
        let at = self.path.display();
        let mut text = serde_json::to_string(line)?;
        if text.contains('\n') {
            bail!("{at}: a log line would hold a line break; nothing was changed");
        }
        text.push('\n');
        let add = text.len() as u64;
        if add > self.caps.line {
            bail!(
                "{at}: the line to append is {add} bytes, over the limit of {} for one line (the records are too large to log); nothing was changed",
                self.caps.line
            );
        }
        let have = self.read.map_or(0, |(_, len)| len).saturating_add(planned);
        if have.checked_add(add).is_none_or(|n| n > self.caps.file) {
            bail!(
                "{at}: is {have} bytes, and a line of {add} more would take it over the limit of {} bytes, past which it is not read; move the file away by hand to start a new one; nothing was changed",
                self.caps.file
            );
        }
        Ok(text)
    }

    /// Is the open file one an append may write to? A regular file with
    /// one link, and, when a file was read, that file at the length read.
    /// Gives its length. Called by [`Jsonl::check`] only.
    fn appendable(&self, meta: &std::fs::Metadata) -> Result<u64> {
        let at = self.path.display();
        if !meta.is_file() {
            bail!("{at}: not a plain file; nothing was changed");
        }
        let links = links_of(meta);
        if links != 1 {
            bail!(
                "{at}: has {links} hard links; an append would also change the file under its other name, which may be outside its folder, so it is not written; nothing was changed"
            );
        }
        // The file just opened, not the path: it must be the file that was
        // read, and no longer than it was then.
        if let Some((ident, len)) = self.read {
            if ident_of(meta) != ident || meta.len() != len {
                bail!("{at}: changed since it was read (another tool wrote to it meanwhile); nothing was changed");
            }
        }
        Ok(meta.len())
    }

    /// Every check of an append of `line`, to a file that has grown by
    /// `planned` bytes since it was read (lines a run means to append
    /// first; 0 for the next line). This is the one place the checks are
    /// made: [`Jsonl::append`] calls it with `write` and writes to the
    /// descriptor it gives, and every "would the append succeed?" answer
    /// is the same call without `write`.
    ///
    /// The outer result is the line's: over the line limit, or the file
    /// with it over the file limit. The inner one is the file's, whatever
    /// the line: it is opened for append without following a link (so a
    /// file that may not be written fails here, with or without `write`),
    /// and on the descriptor it must be a regular file with one link, the
    /// file that was read, at the length read.
    ///
    /// Where no file was read: with `write` the file is created
    /// (create-new); without, the name must still be free and nothing is
    /// created.
    fn check<T: Serialize>(
        &self,
        line: &T,
        planned: u64,
        write: bool,
    ) -> Result<(String, Result<Option<Target>>)> {
        let text = self.text_after(line, planned)?;
        Ok((text, self.target(write)))
    }

    /// The file's half of [`Jsonl::check`].
    fn target(&self, write: bool) -> Result<Option<Target>> {
        let at = self.path.display().to_string();
        let new = self.read.is_none();
        if new && !write {
            return match std::fs::symlink_metadata(&self.path) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e).with_context(|| format!("look at {at}")),
                Ok(_) => bail!("{at}: appeared since it was looked for (another tool made it meanwhile); nothing was changed"),
            };
        }
        let mut open = std::fs::OpenOptions::new();
        open.append(true);
        if new {
            open.create_new(true);
        }
        let file = no_follow(&mut open).open(&self.path).with_context(|| {
            format!(
                "open {at} to append (it must be a plain file, not a link, that may be written; nothing was changed)"
            )
        })?;
        let meta = file.metadata().with_context(|| format!("look at {at}"))?;
        let len = self.appendable(&meta)?;
        Ok(Some(Target {
            file,
            ident: ident_of(&meta),
            len,
        }))
    }

    /// Would an append of `line` succeed? [`Jsonl::check`] with no write,
    /// the two answers as one: for an operation that appends its line
    /// after it has changed records, and for a listing that says what an
    /// operation would do.
    pub(crate) fn ready<T: Serialize>(&self, line: &T) -> Result<()> {
        self.check(line, 0, false)?.1.map(|_| ())
    }

    /// Append `line` and a newline in one `write` call, then sync. Every
    /// check is [`Jsonl::check`]'s, made before the write, on the open
    /// descriptor (see the notes of this file). A file that was not there
    /// is created there, create-new, with the default mode of a new file
    /// (the umask applies).
    pub(crate) fn append<T: Serialize>(&mut self, line: &T) -> Result<()> {
        let at = self.path.display().to_string();
        let (text, target) = self.check(line, 0, true)?;
        let Some(Target {
            mut file,
            ident,
            len: before,
        }) = target?
        else {
            bail!("{at}: not opened to append; nothing was changed");
        };
        let wrote = file
            .write(text.as_bytes())
            .with_context(|| format!("append to {at}"))?;
        if wrote != text.len() {
            // Our own partial bytes: take them back, so no half line stays.
            let _ = file.set_len(before);
            bail!("{at}: the line could not be appended whole; nothing was changed");
        }
        file.sync_all().with_context(|| format!("sync {at}"))?;
        self.read = Some((ident, before + wrote as u64));
        Ok(())
    }
}

/// The file an append writes to: open, and checked on its descriptor.
struct Target {
    file: std::fs::File,
    ident: (u64, u64),
    len: u64,
}

/// What a run means to append before the line asked about.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Planned {
    pub lines: u64,
    pub bytes: u64,
}

/// The answer of [`MergeLog::ready_to_append`] for a line the log's
/// limits allow.
pub(super) struct Ready {
    /// The line, with the id it would take.
    pub line: MergeLine,
    /// Its length with its newline.
    pub bytes: u64,
    /// Whether the file can be appended to at all: the same answer for
    /// every line.
    pub file: Result<()>,
}

impl Ready {
    /// The line, when the file can take it.
    pub fn line(self) -> Result<MergeLine> {
        self.file.map(|()| self.line)
    }
}

/// The number of an id: the canonical spelling only, as for a numbered
/// correction ([`crate::numbered_id`]).
fn number(id: &str) -> Option<u64> {
    crate::numbered_id::number('m', id)
}

/// `merge-log.jsonl`, read and checked.
pub(super) struct MergeLog {
    file: Jsonl,
    pub lines: Vec<LogLine>,
    /// Merge line id → the id of the line that undoes it.
    pub undone: BTreeMap<String, String>,
    /// Merge line id → its place in `lines`, kept as lines are read and
    /// appended: an undo line finds its merge line here, so the read is
    /// linear in the log.
    merge_at: HashMap<String, usize>,
    last: u64,
}

#[cfg(test)]
thread_local! {
    /// Lines [`MergeLog::merge`] has looked at on this thread.
    static LINES_LOOKED_AT: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

impl MergeLog {
    /// Read the log. Besides the line format: ids are canonical
    /// numbered ids, each greater than the one before, a merge line's
    /// `a_merged_into_before` is null, and an undo line names an earlier
    /// merge line that no earlier line has undone.
    pub fn read(dir: &Path) -> Result<MergeLog> {
        let file = Jsonl::read(dir, MERGE_LOG)?;
        let mut log = MergeLog {
            lines: Vec::new(),
            undone: BTreeMap::new(),
            merge_at: HashMap::new(),
            last: 0,
            file,
        };
        for i in 0..log.file.values.len() {
            let bad = |why: String| -> anyhow::Error {
                anyhow::anyhow!(
                    "{}: line {}: {why}; fix or remove the line by hand; nothing was changed",
                    log.file.path.display(),
                    i + 1
                )
            };
            let line = if log.file.values[i].get("undoes").is_some() {
                LogLine::Undo(log.file.typed(i, "an undo line")?)
            } else {
                LogLine::Merge(Box::new(log.file.typed(i, "a merge line")?))
            };
            let id = match &line {
                LogLine::Merge(m) => &m.id,
                LogLine::Undo(u) => &u.id,
            };
            let Some(n) = number(id).filter(|n| *n > log.last) else {
                return Err(bad(format!(
                    "id {id:?} is not `m` and its number in six digits or more with no zero too many, or is not greater than the line before"
                )));
            };
            if let LogLine::Merge(m) = &line {
                if let Some(was) = &m.a_merged_into_before {
                    return Err(bad(format!(
                        "a_merged_into_before is {was:?}, where a merge line has null"
                    )));
                }
            }
            if let LogLine::Undo(u) = &line {
                if log.merge(&u.undoes).is_none() {
                    return Err(bad(format!(
                        "undoes {:?}, which no earlier line is",
                        u.undoes
                    )));
                }
                if log.undone.contains_key(&u.undoes) {
                    return Err(bad(format!("undoes {:?} a second time", u.undoes)));
                }
                log.undone.insert(u.undoes.clone(), u.id.clone());
            }
            log.last = n;
            log.push(line);
        }
        Ok(log)
    }

    pub fn path(&self) -> &Path {
        &self.file.path
    }

    /// The id the next line takes. Refused when the last line holds the
    /// largest id.
    pub fn next_id(&self) -> Result<String> {
        match self.last.checked_add(1).filter(|n| *n <= MAX_ID) {
            Some(n) => Ok(crate::numbered_id::write('m', n)),
            None => bail!(
                "{}: the last line has the largest id there is (m{MAX_ID}); move the file away by hand to start a new one; nothing was changed",
                self.file.path.display()
            ),
        }
    }

    /// The id of the line after `planned` further lines. Refused past the
    /// largest id.
    fn id_after(&self, planned: u64) -> Result<String> {
        match self
            .last
            .checked_add(planned)
            .and_then(|n| n.checked_add(1))
            .filter(|n| *n <= MAX_ID)
        {
            Some(n) => Ok(crate::numbered_id::write('m', n)),
            None => self.next_id().and_then(|_| {
                bail!(
                    "{}: the merges before this one use the last id there is (m{MAX_ID}); nothing was changed",
                    self.file.path.display()
                )
            }),
        }
    }

    /// Would the append of a merge line succeed, after the `planned`
    /// lines of a run? `make` builds the line from the id it would take.
    /// This is what [`MergeLog::append_merge`] goes through
    /// ([`Jsonl::check`]) and the id, with no write: `merge` calls it
    /// before its first write, and `apply-proposals` for every merge it
    /// plans, with and without `--apply`.
    ///
    /// An error is the line's: no id left, a line over the line limit, a
    /// log it would take over the file limit. [`Ready::file`] is the
    /// file's, whatever the line.
    pub fn ready_to_append(
        &self,
        planned: Planned,
        make: impl FnOnce(String) -> MergeLine,
    ) -> Result<Ready> {
        let line = make(self.id_after(planned.lines)?);
        let (text, target) = self.file.check(&line, planned.bytes, false)?;
        Ok(Ready {
            line,
            bytes: text.len() as u64,
            file: target.map(|_| ()),
        })
    }

    /// The checks of an append of `line`, with no write.
    pub fn ready_for_undo(&self, line: &UndoLine) -> Result<()> {
        self.file.ready(line)
    }

    /// Add a line, with its place when it is a merge line.
    fn push(&mut self, line: LogLine) {
        if let LogLine::Merge(m) = &line {
            self.merge_at.insert(m.id.clone(), self.lines.len());
        }
        self.lines.push(line);
    }

    /// The merge line `id`, found by its place ([`MergeLog::merge_at`]):
    /// one line looked at, however long the log.
    pub fn merge(&self, id: &str) -> Option<&MergeLine> {
        let at = *self.merge_at.get(id)?;
        #[cfg(test)]
        LINES_LOOKED_AT.with(|n| n.set(n.get() + 1));
        match self.lines.get(at)? {
            LogLine::Merge(m) => Some(&**m),
            LogLine::Undo(_) => None,
        }
    }

    pub fn merges(&self) -> impl Iterator<Item = &MergeLine> {
        self.lines.iter().filter_map(|l| match l {
            LogLine::Merge(m) => Some(&**m),
            LogLine::Undo(_) => None,
        })
    }

    /// The merge lines no line undoes.
    pub fn live(&self) -> impl Iterator<Item = &MergeLine> {
        self.merges().filter(|m| !self.undone.contains_key(&m.id))
    }

    pub fn append_merge(&mut self, line: MergeLine) -> Result<()> {
        self.file.append(&line)?;
        self.last = number(&line.id).unwrap_or(self.last);
        self.push(LogLine::Merge(Box::new(line)));
        Ok(())
    }

    pub fn append_undo(&mut self, line: UndoLine) -> Result<()> {
        self.file.append(&line)?;
        self.last = number(&line.id).unwrap_or(self.last);
        self.undone.insert(line.undoes.clone(), line.id.clone());
        self.push(LogLine::Undo(line));
        Ok(())
    }
}

/// `merge-decisions.jsonl`, read and checked.
pub(super) struct Decisions {
    file: Jsonl,
    pub lines: Vec<DecisionLine>,
}

impl Decisions {
    pub fn read(dir: &Path) -> Result<Decisions> {
        let file = Jsonl::read(dir, DECISIONS_LOG)?;
        let lines = (0..file.values.len())
            .map(|i| file.typed(i, "a decision line"))
            .collect::<Result<_>>()?;
        Ok(Decisions { file, lines })
    }

    /// The checks of an append of `line`, with no write.
    pub fn ready(&self, line: &DecisionLine) -> Result<()> {
        self.file.ready(line)
    }

    pub fn append(&mut self, line: DecisionLine) -> Result<()> {
        self.file.append(&line)?;
        self.lines.push(line);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SMALL: Caps = Caps {
        file: 300,
        line: 120,
    };

    fn decision(file: &str) -> DecisionLine {
        DecisionLine {
            at: "t".into(),
            file: file.into(),
            a: "a".into(),
            b: "b".into(),
            decision: Decision::Dismiss,
        }
    }

    #[test]
    fn an_append_never_leaves_a_log_too_large_to_read() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(DECISIONS_LOG);
        let mut log = Jsonl::read_with(tmp.path(), DECISIONS_LOG, SMALL).unwrap();
        let line = decision("x");
        let each = serde_json::to_string(&line).unwrap().len() as u64 + 1;
        let fit = SMALL.file / each;
        assert!(fit >= 2 && fit * each < SMALL.file);
        for _ in 0..fit {
            log.ready(&line).unwrap();
            log.append(&line).unwrap();
        }
        let full = std::fs::read(&path).unwrap();
        assert_eq!(full.len() as u64, fit * each);
        // One more would pass the limit: refused by the check and by the
        // append, nothing written, and the file is still read.
        for e in [
            log.ready(&line).unwrap_err(),
            log.append(&line).unwrap_err(),
        ] {
            let e = e.to_string();
            assert!(
                e.contains("merge-decisions.jsonl: is ")
                    && e.contains("over the limit of 300 bytes")
                    && e.contains("nothing was changed"),
                "{e}"
            );
        }
        assert_eq!(std::fs::read(&path).unwrap(), full);
        let again = Jsonl::read_with(tmp.path(), DECISIONS_LOG, SMALL).unwrap();
        assert_eq!(again.values.len() as u64, fit);
    }

    #[test]
    fn a_log_of_exactly_the_file_limit_is_written_and_read() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(DECISIONS_LOG);
        let base = serde_json::to_string(&decision("")).unwrap().len() as u64 + 1;
        let caps = Caps {
            file: base + 100,
            line: 120,
        };
        let mut log = Jsonl::read_with(tmp.path(), DECISIONS_LOG, caps).unwrap();
        log.append(&decision("")).unwrap();
        let one_over = decision(&"x".repeat((101 - base) as usize));
        assert!(log.append(&one_over).is_err());
        log.append(&decision(&"x".repeat((100 - base) as usize)))
            .unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), caps.file);
        let log = Jsonl::read_with(tmp.path(), DECISIONS_LOG, caps).unwrap();
        assert_eq!(log.values.len(), 2);
        assert!(log.ready(&decision("")).is_err());
        // One byte more, by other means: not read.
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.extend_from_slice(b"\n");
        std::fs::write(&path, bytes).unwrap();
        let e = Jsonl::read_with(tmp.path(), DECISIONS_LOG, caps)
            .err()
            .unwrap()
            .to_string();
        assert!(
            e.contains(&format!("larger than {} bytes", caps.file)),
            "{e}"
        );
    }

    #[test]
    fn a_line_over_the_line_limit_is_neither_written_nor_read() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(DECISIONS_LOG);
        let mut log = Jsonl::read_with(tmp.path(), DECISIONS_LOG, SMALL).unwrap();
        let base = serde_json::to_string(&decision("")).unwrap().len();
        // Exactly the limit with its newline: written and read.
        let fits = decision(&"x".repeat(SMALL.line as usize - base - 1));
        log.append(&fits).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), SMALL.line);
        let mut log = Jsonl::read_with(tmp.path(), DECISIONS_LOG, SMALL).unwrap();
        let before = std::fs::read(&path).unwrap();
        let long = decision(&"x".repeat(SMALL.line as usize - base));
        for e in [
            log.ready(&long).unwrap_err(),
            log.append(&long).unwrap_err(),
        ] {
            let e = e.to_string();
            assert!(e.contains("over the limit of 120 for one line"), "{e}");
        }
        assert_eq!(std::fs::read(&path).unwrap(), before);
        // Such a line put there by other means stops the read.
        let text = format!("{}\n", serde_json::to_string(&long).unwrap());
        std::fs::write(&path, text).unwrap();
        let e = Jsonl::read_with(tmp.path(), DECISIONS_LOG, SMALL)
            .err()
            .unwrap()
            .to_string();
        assert!(e.contains("line 1: longer than 120 bytes"), "{e}");
    }

    /// What the question says (`ready`) is what the append does, in the
    /// same words, for every state of the file; a refused append writes
    /// nothing.
    #[cfg(unix)]
    #[test]
    fn the_question_and_the_append_give_one_answer() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(DECISIONS_LOG);
        let other = tmp.path().join("other-name");
        let line = decision("x");
        let read = || Jsonl::read_with(tmp.path(), DECISIONS_LOG, SMALL).unwrap();
        let same = |log: &mut Jsonl, why: &str| {
            let before = std::fs::read(&path).ok();
            let asked = format!("{:#}", log.ready(&line).unwrap_err());
            let done = format!("{:#}", log.append(&line).unwrap_err());
            assert_eq!(asked, done);
            assert!(asked.contains(why), "want {why:?} in {asked}");
            assert_eq!(std::fs::read(&path).ok(), before, "{why}");
        };
        // No file: both say yes, and the append makes it.
        let mut log = read();
        log.ready(&line).unwrap();
        assert!(!path.exists(), "the question made the file");
        log.append(&line).unwrap();
        log.ready(&line).unwrap();
        // A second name.
        std::fs::hard_link(&path, &other).unwrap();
        same(&mut log, "has 2 hard links");
        same(&mut read(), "has 2 hard links");
        std::fs::remove_file(&other).unwrap();
        // Written by another since it was read.
        let mut stale = read();
        read().append(&line).unwrap();
        same(&mut stale, "changed since it was read");
        // Replaced by another file of the same length.
        let mut stale = read();
        let bytes = std::fs::read(&path).unwrap();
        std::fs::rename(&path, &other).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        same(&mut stale, "changed since it was read");
        std::fs::remove_file(&other).unwrap();
        // Not open to writing.
        let mut log = read();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).unwrap();
        if std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .is_err()
        {
            same(&mut log, "to append");
        }
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        // A symlink, or a folder, where the file was.
        let mut log = read();
        std::fs::rename(&path, &other).unwrap();
        std::os::unix::fs::symlink(&other, &path).unwrap();
        same(&mut log, "to append");
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        same(&mut log, "to append");
        std::fs::remove_dir(&path).unwrap();
        // Read as missing, there by the time of the append.
        let mut absent = read();
        std::fs::write(&path, "").unwrap();
        let asked = absent.ready(&line).unwrap_err().to_string();
        assert!(
            asked.contains("appeared since it was looked for"),
            "{asked}"
        );
        assert!(absent.append(&line).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"");
        // The line's own limits.
        let mut log = read();
        let long = decision(&"x".repeat(SMALL.line as usize));
        let asked = log.ready(&long).unwrap_err().to_string();
        assert_eq!(asked, log.append(&long).unwrap_err().to_string());
        // And yes means yes.
        log.ready(&line).unwrap();
        log.append(&line).unwrap();
    }

    #[test]
    fn an_id_is_m_and_its_number_in_the_one_canonical_spelling() {
        assert_eq!(number("m000001"), Some(1));
        assert_eq!(number("m999999999"), Some(MAX_ID));
        assert_eq!(number("m1000000"), Some(1_000_000));
        for bad in [
            "m1",
            "m00001",
            "m000000",
            "m0000001",
            "m000000001",
            "m0999999",
            "m1000000000",
            "m0000000001",
            "m18446744073709551615",
            "m18446744073709551616",
            "m-00001",
            "m+00001",
            "m00000a",
            "000001",
            "M000001",
            "m٠٠٠٠٠١",
        ] {
            assert_eq!(number(bad), None, "{bad}");
        }
    }

    /// 30,000 merges, each undone by the next line, then one merge no
    /// line undoes.
    fn long_log(dir: &Path, pairs: u64) {
        let id = |n: u64| crate::numbered_id::write('m', n);
        let mut text = String::new();
        let merge = |n: u64| {
            let line = MergeLine {
                id: id(n),
                at: "2026-10-12T00:00:00Z".into(),
                by: By::Dominik,
                reason: None,
                a: format!("a{n}"),
                b: "b".into(),
                a_merged_into_before: None,
            };
            serde_json::to_string(&line).unwrap() + "\n"
        };
        for i in 0..pairs {
            text += &merge(2 * i + 1);
            let undo = UndoLine {
                id: id(2 * i + 2),
                at: "2026-10-12T00:00:01Z".into(),
                undoes: id(2 * i + 1),
            };
            text += &(serde_json::to_string(&undo).unwrap() + "\n");
        }
        text += &merge(2 * pairs + 1);
        std::fs::write(dir.join(MERGE_LOG), text).unwrap();
    }

    #[test]
    fn a_long_log_is_read_with_one_line_looked_at_per_undo() {
        const PAIRS: u64 = 30_000;
        let tmp = tempfile::tempdir().unwrap();
        long_log(tmp.path(), PAIRS);
        LINES_LOOKED_AT.with(|n| n.set(0));
        let started = std::time::Instant::now();
        let log = MergeLog::read(tmp.path()).unwrap();
        let took = started.elapsed();
        let looked_at = LINES_LOOKED_AT.with(|n| n.get());
        // Each undo line finds its merge line by its place: one line
        // looked at per undo, where a scan from the top looks at
        // 1 + 2 + … + 30,000 of them.
        assert_eq!(looked_at, PAIRS, "lines looked at while reading");
        assert!(took.as_secs_f64() < 3.0, "the read took {took:?}");
        assert_eq!(log.lines.len() as u64, 2 * PAIRS + 1);
        assert_eq!(log.undone.len() as u64, PAIRS);
        assert_eq!(log.merge_at.len() as u64, PAIRS + 1);
        let live: Vec<&str> = log.live().map(|m| m.id.as_str()).collect();
        assert_eq!(live, ["m060001"]);
        // The place kept for an id is the place of that line.
        for id in ["m000001", "m030001", "m059999", "m060001"] {
            assert_eq!(log.merge(id).map(|m| m.id.as_str()), Some(id));
        }
        // An undo line's id, and an id of no line, are no merge line.
        assert!(log.merge("m000002").is_none());
        assert!(log.merge("m060002").is_none());
        // A line appended is found too.
        let mut log = log;
        let next = log.next_id().unwrap();
        assert_eq!(next, "m060002");
        log.append_merge(MergeLine {
            id: next.clone(),
            at: "t".into(),
            by: By::Model,
            reason: None,
            a: "z".into(),
            b: "b".into(),
            a_merged_into_before: None,
        })
        .unwrap();
        assert_eq!(log.merge(&next).map(|m| m.a.as_str()), Some("z"));
    }
}

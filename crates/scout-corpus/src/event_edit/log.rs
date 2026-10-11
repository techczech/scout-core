//! The two append-only files of the records folder, `merge-log.jsonl` and
//! `merge-decisions.jsonl`: their only reader and their only writer.
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
//! other name's contents, which may lie outside the records folder), the
//! file that was read, at the length it was read at. The line with its
//! newline must be at most 4 MiB, and the file with the line at most
//! 256 MiB, so no append leaves a log this module then refuses to read.
//! The object and its newline go out in one `write` call, then the file
//! is synced. A log that is not there is created (create-new, so the name
//! cannot be a link of any kind) with mode 0644, set on the descriptor.
//! Nothing is ever rewritten or removed.
//!
//! [`MergeLog::ready_for_undo`] and [`Decisions::ready`] make the same
//! checks without writing. `unmerge` and `decide` append their line last,
//! after records have changed, so they call these before their first
//! write; `merge` appends its line first, where the append's own checks
//! are already ahead of every write.
//!
//! Ids of the merge log are `m` and six to nine digits, so at most
//! `m999999999` ([`MAX_ID`]). A line with any other id is malformed. When
//! the last line holds the largest id, no further line can be written and
//! `merge` and `unmerge` refuse.

use super::store::{ident_of, links_of, no_follow, read_plain, set_mode};
use anyhow::{bail, Context, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const MERGE_LOG: &str = "merge-log.jsonl";
pub const DECISIONS_LOG: &str = "merge-decisions.jsonl";
/// Largest log file read, and the largest an append may leave.
pub(super) const MAX_LOG_BYTES: u64 = 256 << 20;
/// Largest line, with its newline, read or written.
pub(super) const MAX_LINE_BYTES: u64 = 4 << 20;
/// Largest number of a merge-log id.
pub(super) const MAX_ID: u64 = 999_999_999;

/// The size limits of one log.
#[derive(Debug, Clone, Copy)]
struct Caps {
    file: u64,
    line: u64,
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
    pub reason: Option<String>,
    /// The record merged away.
    pub a: String,
    /// The surviving record.
    pub b: String,
    /// `a`'s `merged_into` before the merge. Always null: a record that is
    /// already merged into another is not merged. A line with any other
    /// value is malformed.
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
struct Jsonl {
    path: PathBuf,
    /// Device and inode, and the length read; `None` when no file is there.
    read: Option<((u64, u64), u64)>,
    /// Each line's JSON object.
    values: Vec<serde_json::Value>,
    caps: Caps,
}

impl Jsonl {
    fn read(dir: &Path, name: &str) -> Result<Jsonl> {
        Jsonl::read_with(dir, name, CAPS)
    }

    fn read_with(dir: &Path, name: &str, caps: Caps) -> Result<Jsonl> {
        let path = dir.join(name);
        let Some(plain) = read_plain(&path, caps.file)? else {
            return Ok(Jsonl {
                path,
                read: None,
                values: vec![],
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
    fn text_of<T: Serialize>(&self, line: &T) -> Result<String> {
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
        let have = self.read.map_or(0, |(_, len)| len);
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
    /// Gives its length.
    fn appendable(&self, meta: &std::fs::Metadata) -> Result<u64> {
        let at = self.path.display();
        if !meta.is_file() {
            bail!("{at}: not a plain file; nothing was changed");
        }
        let links = links_of(meta);
        if links != 1 {
            bail!(
                "{at}: has {links} hard links; an append would also change the file under its other name, which may be outside the records folder, so it is not written; nothing was changed"
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

    /// Every check of [`Jsonl::append`], with no write: for an operation
    /// that appends its line after it has changed records.
    fn ready<T: Serialize>(&self, line: &T) -> Result<()> {
        let at = self.path.display().to_string();
        self.text_of(line)?;
        if self.read.is_none() {
            return match std::fs::symlink_metadata(&self.path) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e).with_context(|| format!("look at {at}")),
                Ok(_) => bail!("{at}: appeared since it was looked for (another tool made it meanwhile); nothing was changed"),
            };
        }
        let file = no_follow(std::fs::OpenOptions::new().read(true))
            .open(&self.path)
            .with_context(|| {
                format!("open {at} (it must be a plain file, not a link; nothing was changed)")
            })?;
        let meta = file.metadata().with_context(|| format!("look at {at}"))?;
        self.appendable(&meta).map(|_| ())
    }

    /// Append `line` and a newline in one `write` call, then sync. Every
    /// check is made before the write, on the open descriptor (see the
    /// notes of this file). A file that was not there is created here,
    /// create-new, mode 0644.
    fn append<T: Serialize>(&mut self, line: &T) -> Result<()> {
        let at = self.path.display().to_string();
        let text = self.text_of(line)?;
        let mut open = std::fs::OpenOptions::new();
        open.append(true);
        let new = self.read.is_none();
        if new {
            open.create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                open.mode(0o644);
            }
        }
        let mut file = no_follow(&mut open).open(&self.path).with_context(|| {
            format!(
                "open {at} to append (it must be a plain file, not a link; nothing was changed)"
            )
        })?;
        let meta = file.metadata().with_context(|| format!("look at {at}"))?;
        let before = self.appendable(&meta)?;
        if new {
            // Whatever the umask made of the mode asked for at the open.
            set_mode(&file, 0o644).with_context(|| format!("set the mode of {at}"))?;
        }
        let wrote = file
            .write(text.as_bytes())
            .with_context(|| format!("append to {at}"))?;
        if wrote != text.len() {
            // Our own partial bytes: take them back, so no half line stays.
            let _ = file.set_len(before);
            bail!("{at}: the line could not be appended whole; nothing was changed");
        }
        file.sync_all().with_context(|| format!("sync {at}"))?;
        self.read = Some((ident_of(&meta), before + wrote as u64));
        Ok(())
    }
}

/// The number of an id: `m` and six to nine digits.
fn number(id: &str) -> Option<u64> {
    let digits = id.strip_prefix('m')?;
    ((6..=9).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_digit()))
        .then(|| digits.parse().ok())
        .flatten()
        .filter(|n| *n <= MAX_ID)
}

/// `merge-log.jsonl`, read and checked.
pub(super) struct MergeLog {
    file: Jsonl,
    pub lines: Vec<LogLine>,
    /// Merge line id → the id of the line that undoes it.
    pub undone: BTreeMap<String, String>,
    last: u64,
}

impl MergeLog {
    /// Read the log. Besides the line format: ids are `m` and six to nine
    /// digits, each greater than the one before, a merge line's
    /// `a_merged_into_before` is null, and an undo line names an earlier
    /// merge line that no earlier line has undone.
    pub fn read(dir: &Path) -> Result<MergeLog> {
        let file = Jsonl::read(dir, MERGE_LOG)?;
        let mut log = MergeLog {
            lines: Vec::new(),
            undone: BTreeMap::new(),
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
                    "id {id:?} is not `m` and six to nine digits, or is not greater than the line before"
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
            log.lines.push(line);
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
            Some(n) => Ok(format!("m{n:06}")),
            None => bail!(
                "{}: the last line has the largest id there is (m{MAX_ID}); move the file away by hand to start a new one; nothing was changed",
                self.file.path.display()
            ),
        }
    }

    /// The checks of an append of `line`, with no write.
    pub fn ready_for_undo(&self, line: &UndoLine) -> Result<()> {
        self.file.ready(line)
    }

    pub fn merge(&self, id: &str) -> Option<&MergeLine> {
        self.merges().find(|m| m.id == id)
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
        self.lines.push(LogLine::Merge(Box::new(line)));
        Ok(())
    }

    pub fn append_undo(&mut self, line: UndoLine) -> Result<()> {
        self.file.append(&line)?;
        self.last = number(&line.id).unwrap_or(self.last);
        self.undone.insert(line.undoes.clone(), line.id.clone());
        self.lines.push(LogLine::Undo(line));
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

    #[test]
    fn an_id_is_m_and_six_to_nine_digits() {
        assert_eq!(number("m000001"), Some(1));
        assert_eq!(number("m999999999"), Some(MAX_ID));
        for bad in [
            "m1",
            "m00001",
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
}

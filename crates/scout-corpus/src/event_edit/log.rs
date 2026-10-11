//! The two append-only files of the records folder, `merge-log.jsonl` and
//! `merge-decisions.jsonl`: their only reader and their only writer.
//!
//! A file is read whole before anything is written. It must be a plain
//! file (not a symlink: the open does not follow one), UTF-8, and every
//! line a complete JSON object of the file's kind, the last line ended by
//! a newline; anything else stops the command with the file and the line
//! named. A line is appended through an open that does not follow a link
//! and only when the file is still the one that was read, at the length
//! it was read at: the object and its newline go out in one `write` call,
//! then the file is synced. Nothing is ever rewritten or removed.

use super::store::{ident_of, no_follow, read_plain};
use crate::events::EventRecord;
use anyhow::{bail, Context, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

pub const MERGE_LOG: &str = "merge-log.jsonl";
pub const DECISIONS_LOG: &str = "merge-decisions.jsonl";
/// Largest log file read.
const MAX_LOG_BYTES: u64 = 256 << 20;

/// Who made a merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum By {
    Dominik,
    Model,
}

/// The two records of a merge, whole.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pair {
    pub a: EventRecord,
    pub b: EventRecord,
}

/// One merge: `a` merged into `b`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MergeLine {
    /// `m000001`, `m000002` …: one sequence for merge and undo lines.
    pub id: String,
    /// When the line was written, UTC.
    pub at: String,
    pub by: By,
    pub reason: Option<String>,
    pub a: String,
    pub b: String,
    /// The two records as they were read before the merge.
    pub before: Pair,
    /// The two records as the merge writes them.
    pub after: Pair,
    /// The folded ids (record ids and monthly ids) the merge moves from
    /// `a` to `b` in the reader's id table.
    pub moved: Vec<String>,
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
}

impl Jsonl {
    fn read(dir: &Path, name: &str) -> Result<Jsonl> {
        let path = dir.join(name);
        let Some(plain) = read_plain(&path, MAX_LOG_BYTES)? else {
            return Ok(Jsonl {
                path,
                read: None,
                values: vec![],
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

    /// Append `line` and a newline in one `write` call, then sync. The
    /// file must be the one that was read, as long as it was then (or
    /// still missing, when it is created here, exclusively).
    fn append<T: Serialize>(&mut self, line: &T) -> Result<()> {
        let at = self.path.display().to_string();
        let mut text = serde_json::to_string(line)?;
        if text.contains('\n') {
            bail!("{at}: a log line would hold a line break");
        }
        text.push('\n');
        let mut open = std::fs::OpenOptions::new();
        open.append(true);
        if self.read.is_none() {
            open.create_new(true);
        }
        let mut file = no_follow(&mut open).open(&self.path).with_context(|| {
            format!(
                "open {at} to append (it must be a plain file, not a link; nothing was changed)"
            )
        })?;
        let meta = file.metadata().with_context(|| format!("look at {at}"))?;
        if !meta.is_file() {
            bail!("{at}: not a plain file; nothing was changed");
        }
        let before = meta.len();
        // The file just opened, not the path: it must be the file that was
        // read, and no longer than it was then.
        if let Some((ident, len)) = self.read {
            if ident_of(&meta) != ident || before != len {
                bail!("{at}: changed since it was read (another tool wrote to it meanwhile); nothing was changed");
            }
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

fn number(id: &str) -> Option<u64> {
    let digits = id.strip_prefix('m')?;
    (digits.len() >= 6 && digits.bytes().all(|b| b.is_ascii_digit()))
        .then(|| digits.parse().ok())
        .flatten()
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
    /// Read the log. Besides the line format: ids are `m` and at least
    /// six digits, each greater than the one before, and an undo line
    /// names an earlier merge line that no earlier line has undone.
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
                    "id {id:?} is not `m` and six digits, or is not greater than the line before"
                )));
            };
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

    /// The id the next line takes.
    pub fn next_id(&self) -> String {
        format!("m{:06}", self.last + 1)
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

    pub fn append(&mut self, line: DecisionLine) -> Result<()> {
        self.file.append(&line)?;
        self.lines.push(line);
        Ok(())
    }
}

//! `scout events set | merge | unmerge | apply-proposals | decide |
//! proposals | remove | restore | create | check`: the edits of lasting
//! event records (spec
//! `archive-scout/docs/specs/2026-10-11-lasting-events-spec.md`, "What he
//! can do to an event" and "Where lasting events come from", step 2).
//! Item-level changes (an item into or out of an event) are not here;
//! they go through `corrections.jsonl`.
//!
//! The design rule is the seed's ([`crate::event_seed`]): one
//! implementation of the record format. This module has no parser and no
//! validator of records.
//!
//! - The folder is read once per operation by the reader
//!   (`events::read_dir_with_bytes`, the index build's read plus the bytes
//!   each record came from). If the reader skips a file, or does not
//!   follow a `merged_into`, the operation stops.
//! - Every record an operation changes is written by the seed's writer
//!   (`render::render_front` / `render_record`), read back by
//!   [`events::parse_bytes`] and compared with the record meant.
//! - The whole folder as it would be (the records as their files have
//!   them, with the changed ones in place) goes through
//!   [`EventSet::from_records`]. The reader must read every record, follow
//!   every `merged_into`, give no warning it does not give now, and its id
//!   table must be exactly the one the operation means:
//!   - `set`, `remove`, `restore`: the table as it is;
//!   - `create`: the table plus the new id;
//!   - `merge a b`: every id held by `a` held by `b`, nothing else moved;
//!   - `unmerge`: `a`, every record merged into `a`, and the monthly ids
//!     those records hold, held by `a`; nothing else moved.
//! - All of that happens before the first write. A refusal leaves the
//!   folder byte for byte as it was.
//!
//! What is preserved. An operation rewrites only the frontmatter of a
//! record whose description it does not set: the bytes after the
//! frontmatter are carried over as they are. A record is rewritten only
//! when its file starts with exactly the frontmatter the writer writes
//! for it; one that has anything else there (a key scout does not read, a
//! comment, another YAML style, CR LF, a byte-order mark) is refused,
//! since rewriting would lose it. `edited`, `origin`, `members`, `themes`
//! and every other field an operation does not name keep their values.
//!
//! Replacing a record ([`store::replace`]): the target's permission bits
//! are read; a new file of a name no record can have
//! (`.scout-edit-<pid>-<time>-<n>.tmp`) is created in the records folder
//! (create-new and no-follow, so it is a file of its own and no link to
//! any other), given those permission bits, written and synced; the
//! target is then read again and must be a plain file holding exactly the
//! bytes the operation read at its start (else the new file is removed
//! and the target is left alone: he or another tool edited it meanwhile);
//! then the new file is renamed over the target. The old file is never
//! opened for writing: the rename gives the record's name a new file, and
//! the old file's bytes stay as they were under any other name it has.
//! A record that `create` makes, and a log made by its first line, has
//! mode 0644, set on the descriptor whatever the umask.
//!
//! Merging. A merge of `a` into `b` changes one field of one record:
//! `a.merged_into` becomes `b`. `unmerge` clears it. Record `b` is never
//! written by either, and nothing else of `a` is. The combined event's
//! ids, span, aliases and the list of records merged into it are worked
//! out by the reader when asked ([`events::EventSet`],
//! [`events::combined`]); see `merge.rs`.
//!
//! Order of writes, and what a crash leaves. A merge writes two things
//! and an undo writes two:
//!
//! - `merge a b`: (1) the log line; (2) record `a`, now `merged_into: b`.
//!   After (1) alone no record has changed.
//! - `unmerge`: (1) record `a`, `merged_into` cleared; (2) the undo line.
//!   That the log can take the line is checked before (1). After (1)
//!   alone the merge is gone from the records and its line is still in
//!   the log without an undo.
//! - `decide`: the merge of an approval (as above), then the decision
//!   line; that `merge-decisions.jsonl` can take the line is checked
//!   before the merge.
//! - A merge line no undo line undoes is therefore in one of two states,
//!   told from record `a`: in effect (`a` is merged into `b`) or not
//!   applied (`a` is merged into nothing: either crash above).
//!   [`EventEditor::check`] reports every line not applied, and
//!   `unmerge <id>` withdraws it (it only appends the undo line). `merge`
//!   refuses `a` while a line for `a` is not applied, and names the line.
//!   A line whose `a` is gone or merged into another record was changed
//!   by other means; `check` reports it and `unmerge` refuses it.
//! - The other operations write one file each.
//! - A crash can leave one `.scout-edit-*.tmp` file; the reader does not
//!   read it, `check` lists it, nothing removes it.
//!
//! Undo writes no value kept from before. `unmerge` clears `merged_into`
//! on the record its line names; `restore` takes `removed` off. Nothing
//! he changed in between is touched.
//!
//! The two logs (`log.rs`, their only reader and writer) are append-only:
//! read whole and checked before any write, opened no-follow, refused
//! when not a plain file, when over 256 MiB, or when a line is malformed
//! or over 4 MiB. A log is appended to only when the open file is a
//! regular file with one link (an append to a hard-linked file would
//! write to its other name too), is the file that was read at the length
//! read, and stays within both limits with the new line; an operation
//! that appends after changing records checks this before its first
//! write. Merge-log ids run to `m999999999`.
//!
//! Numbers taken from files. A merge-log id is at most nine digits and is
//! advanced with checked arithmetic. `a_items` / `b_items` of a proposals
//! line (the tagger's file) are taken only as whole numbers from 0 to
//! 10,000,000 and are added saturating. No other number from a file is
//! used in arithmetic or as a length.
//!
//! Threat model: the seed's. A single-user desktop tool; no hostile
//! process runs beside it.
//!
//! - Holds: the records folder itself is not a symlink; a path with `..`
//!   is refused; a record or log that is a symlink or not a plain file is
//!   not written (the reader refuses such a record, and the logs are
//!   opened no-follow); a log with a second hard link is not appended to;
//!   a record with a second hard link is replaced under its own name
//!   only, its old file not written; a change made to a record between
//!   the read and the check before its rename is seen and stops the
//!   operation; every check runs before the first write.
//! - Not defended: a write to a record in the instant between the check
//!   and the rename (it is lost: the rename replaces the file); another
//!   process replacing the folder itself; a link added to a log, or a write to it, in the instant
//!   between the check on its open descriptor and the write. Writes are
//!   not anchored to a directory descriptor. Of a replaced record only
//!   the permission bits are carried over: not its owner, group, extended
//!   attributes or times.

mod log;
mod merge;
mod proposals;
mod store;

use crate::event_seed::naming;
use crate::event_seed::render::{body_text, render_record};
use crate::event_seed::{preflight_names, refuse_parent_dir, resolve_out_dir};
use crate::events::{self, is_date, is_record_control, EventRecord, EVENTS_DIR};
use crate::normalize::fold;
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};
use store::Snapshot;

pub use log::{By, Decision, DecisionLine, MergeLine, UndoLine, DECISIONS_LOG, MERGE_LOG};
pub use merge::{CheckReport, Merged, Problem, Unmerged};
pub use proposals::{
    ApplyReport, Decided, PlannedMerge, SkippedPair, WaitingPair, WaitingReport, PROPOSALS_DIR,
};

/// A point of an operation a test hook is called at ([`EventEditor::with_hook`]).
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step<'a> {
    /// Everything is read and checked; nothing is written yet.
    Read,
    /// The new bytes of this record file are in its temporary file; the
    /// target is about to be compared with what was read, then replaced.
    Replacing(&'a str),
    /// This file (a record or a log) has just been written.
    Written(&'a str),
}

type HookFn = Box<dyn Fn(Step) -> std::io::Result<()> + Send + Sync>;

/// The records folder of one archive, for editing.
pub struct EventEditor {
    archive: PathBuf,
    dir: PathBuf,
    hook: Option<HookFn>,
    now: Option<String>,
}

/// `scout events set`: the fields to change. `None` leaves a field alone.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FieldEdit {
    pub name: Option<String>,
    /// Replaces the body.
    pub description: Option<String>,
    pub start: Option<String>,
    /// `Some("")` clears it.
    pub end: Option<String>,
    pub add_aliases: Vec<String>,
    pub remove_aliases: Vec<String>,
}

/// `scout events create`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NewEvent {
    pub name: String,
    pub description: Option<String>,
    pub start: Option<String>,
    pub end: Option<String>,
}

/// `scout events merge`: `a` is merged into `b`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeRequest {
    pub a: String,
    pub b: String,
    pub by: By,
    pub reason: Option<String>,
}

/// What `set`, `remove`, `restore` and `create` did (one JSON line).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Changed {
    pub tool: String,
    /// When the operation ran, UTC.
    pub at: String,
    pub id: String,
    /// The record file.
    pub file: String,
    /// False when the record already was as asked; nothing was written.
    pub changed: bool,
    /// The record as it is now.
    pub record: EventRecord,
}

/// `s` trimmed, refused when it holds a control character.
fn plain_text(what: &str, s: &str) -> Result<String> {
    if let Some(c) = s.chars().find(|c| c.is_control() || is_record_control(*c)) {
        bail!(
            "{what} holds a control character (U+{:04X}); give it as one line of plain text (nothing was changed)",
            c as u32
        );
    }
    Ok(s.trim().to_string())
}

fn date(what: &str, s: &str) -> Result<String> {
    let s = s.trim();
    if !is_date(s) {
        bail!("{what} {s:?} is not a date YYYY-MM-DD (nothing was changed)");
    }
    Ok(s.to_string())
}

/// `end` must not be before `start`, when the record has both.
fn span_in_order(r: &EventRecord) -> Result<()> {
    if let (Some(start), Some(end)) = (&r.start, &r.end) {
        if !is_date(start) || !is_date(end) {
            bail!(
                "{}: start {start:?} and end {end:?} are not both dates YYYY-MM-DD; set both (nothing was changed)",
                r.id
            );
        }
        if end < start {
            bail!(
                "{}: end {end} is before start {start} (nothing was changed)",
                r.id
            );
        }
    }
    Ok(())
}

impl EventEditor {
    /// The records folder of `archive`, `<archive>/analysis/events`, under
    /// the seed's path rules: no `..` component, and the folder itself a
    /// real folder, not a symlink (symlinks above it are accepted). It
    /// must exist; no edit creates it. Nothing is written here.
    pub fn open(archive: &Path) -> Result<EventEditor> {
        refuse_parent_dir(archive, "the archive")?;
        if !archive.is_dir() {
            bail!("{}: the archive is not a folder", archive.display());
        }
        let dir = resolve_out_dir(&archive.join(EVENTS_DIR)).context("the records folder")?;
        if !std::fs::symlink_metadata(&dir).is_ok_and(|m| m.is_dir()) {
            bail!(
                "{}: no records folder (run `scout events seed` first)",
                dir.display()
            );
        }
        Ok(EventEditor {
            archive: archive.to_path_buf(),
            dir,
            hook: None,
            now: None,
        })
    }

    /// The records folder, resolved.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn archive(&self) -> &Path {
        &self.archive
    }

    /// Tests: call `hook` at each [`Step`]; an error it gives stops the
    /// operation at that point, as a crash would.
    #[doc(hidden)]
    pub fn with_hook(
        mut self,
        hook: impl Fn(Step) -> std::io::Result<()> + Send + Sync + 'static,
    ) -> Self {
        self.hook = Some(Box::new(hook));
        self
    }

    /// Tests: the time every operation takes as now
    /// (`2026-10-11T12:00:00.000Z`).
    #[doc(hidden)]
    pub fn with_now(mut self, now: &str) -> Self {
        self.now = Some(now.to_string());
        self
    }

    fn step(&self, s: Step) -> std::io::Result<()> {
        match &self.hook {
            Some(h) => h(s),
            None => Ok(()),
        }
    }

    fn now(&self) -> String {
        self.now.clone().unwrap_or_else(|| {
            chrono::Utc::now()
                .format("%Y-%m-%dT%H:%M:%S%.3fZ")
                .to_string()
        })
    }

    /// Today's date, UTC.
    fn today(&self) -> String {
        self.now().chars().take(10).collect()
    }

    fn changed(&self, tool: &str, record: EventRecord, changed: bool) -> Changed {
        Changed {
            tool: tool.to_string(),
            at: self.now(),
            file: self
                .dir
                .join(format!("{}.md", record.id))
                .display()
                .to_string(),
            id: record.id.clone(),
            changed,
            record,
        }
    }

    /// Check one changed record against the folder (the id table stays as
    /// it is) and replace its file. False when the bytes already are as
    /// meant.
    fn replace_one(&self, snap: &Snapshot, new: &EventRecord, new_body: bool) -> Result<bool> {
        let w = snap.rewrite(new, new_body)?;
        snap.verify(&[new], &snap.set.aliases)?;
        self.step(Step::Read)?;
        if !w.changes() {
            return Ok(false);
        }
        store::replace(&snap.dir, &w, &|s| self.step(s))?;
        self.step(Step::Written(&w.file))?;
        Ok(true)
    }

    /// Change fields of the record `id` and set its `edited` to today.
    /// `origin`, `members`, `themes`, `merged_into` and `removed` are left
    /// as they are, and so is the body unless `description` is given.
    ///
    /// Aliases are compared under the reader's fold; removals are made
    /// first, then additions (one the record already has is not added
    /// twice).
    ///
    /// Refused, with nothing changed: no such record; nothing to set; a
    /// name or alias that is empty or holds a control character; a date
    /// that is not `YYYY-MM-DD` (an empty `end` clears it); `end` before
    /// `start`; an alias to remove that the record does not have.
    pub fn set(&self, id: &str, edit: &FieldEdit) -> Result<Changed> {
        if *edit == FieldEdit::default() {
            bail!("nothing to set: give at least one field (nothing was changed)");
        }
        let snap = Snapshot::read(&self.dir)?;
        let mut r = snap.record(id)?.clone();
        if let Some(name) = &edit.name {
            let name = plain_text("the name", name)?;
            if name.is_empty() {
                bail!("the name is empty (nothing was changed)");
            }
            r.name = name;
        }
        if let Some(start) = &edit.start {
            r.start = Some(date("start", start)?);
        }
        if let Some(end) = &edit.end {
            r.end = match end.trim() {
                "" => None,
                end => Some(date("end", end)?),
            };
        }
        if edit.start.is_some() || edit.end.is_some() {
            span_in_order(&r)?;
        }
        for alias in &edit.remove_aliases {
            let f = fold(&plain_text("an alias", alias)?);
            let Some(at) = r.aliases.iter().position(|a| fold(a) == f) else {
                bail!(
                    "{}: has no alias {:?} to remove (nothing was changed)",
                    r.id,
                    alias.trim()
                );
            };
            r.aliases.remove(at);
        }
        for alias in &edit.add_aliases {
            let alias = plain_text("an alias", alias)?;
            if alias.is_empty() {
                bail!("an alias is empty (nothing was changed)");
            }
            if !r.aliases.iter().any(|a| fold(a) == fold(&alias)) {
                r.aliases.push(alias);
            }
        }
        if let Some(description) = &edit.description {
            r.description = body_text(description);
        }
        r.edited = Some(self.today());
        let changed = self.replace_one(&snap, &r, edit.description.is_some())?;
        Ok(self.changed("scout events set", r, changed))
    }

    /// Mark the record `id` removed (`removed: <today>`). The record
    /// stays, and stays the holder of its ids. Removing a removed record
    /// changes nothing. Refused: no such record; a record merged into
    /// another (it is not an event of its own).
    pub fn remove(&self, id: &str) -> Result<Changed> {
        let snap = Snapshot::read(&self.dir)?;
        let mut r = snap.record(id)?.clone();
        if let Some(into) = &r.merged_into {
            bail!(
                "{}: merged into {into}; remove {into}, or undo the merge first (nothing was changed)",
                r.id
            );
        }
        let mut changed = false;
        if r.removed.is_none() {
            r.removed = Some(self.today());
            changed = self.replace_one(&snap, &r, false)?;
        }
        Ok(self.changed("scout events remove", r, changed))
    }

    /// Take `removed` off the record `id`. Restoring a record that is not
    /// removed changes nothing. Refused: no such record.
    pub fn restore(&self, id: &str) -> Result<Changed> {
        let snap = Snapshot::read(&self.dir)?;
        let mut r = snap.record(id)?.clone();
        let mut changed = false;
        if r.removed.is_some() {
            r.removed = None;
            changed = self.replace_one(&snap, &r, false)?;
        }
        Ok(self.changed("scout events restore", r, changed))
    }

    /// Create a record: `origin: dominik`, `edited` today, no members.
    /// Its id is `YYYY-MM-<slug of the name>` (the seed's naming, with its
    /// `-2`, `-3` suffixes), the month taken from `start`, else today. The
    /// file is created as the seed creates one: the name is looked up in
    /// the folder first, and the open is create-exclusive and no-follow.
    ///
    /// Refused: an empty name or one with a control character; a date
    /// that is not `YYYY-MM-DD`; `end` before `start`.
    pub fn create(&self, new: &NewEvent) -> Result<Changed> {
        let name = plain_text("the name", &new.name)?;
        if name.is_empty() {
            bail!("the name is empty (nothing was changed)");
        }
        let start = new.start.as_deref().map(|s| date("start", s)).transpose()?;
        let end = match new.end.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(end) => Some(date("end", end)?),
        };
        let snap = Snapshot::read(&self.dir)?;
        let today = self.today();
        let month = &start.as_deref().unwrap_or(&today)[..7];
        if !naming::is_period(month) {
            bail!("{month}: not a month (nothing was changed)");
        }
        let mut taken = snap.names_in_use()?;
        let picked = naming::assign_id(month, "", &name, &mut taken);
        if !naming::is_slug(&picked.id) {
            bail!("{:?}: not a record id (nothing was changed)", picked.id);
        }
        let record = EventRecord {
            id: picked.id,
            name,
            start,
            end,
            origin: Some("dominik".into()),
            edited: Some(today),
            description: body_text(new.description.as_deref().unwrap_or_default()),
            ..EventRecord::default()
        };
        span_in_order(&record)?;
        let text = render_record(&record);
        let at = self.dir.join(format!("{}.md", record.id));
        match events::parse_bytes(text.as_bytes(), &record.id) {
            Err(why) => bail!(
                "{}: scout would not read the new record ({why}) (nothing was changed)",
                at.display()
            ),
            Ok(back) if back != record => bail!(
                "{}: the new record does not read back as meant (nothing was changed)",
                at.display()
            ),
            Ok(_) => {}
        }
        let mut expected = snap.set.aliases.clone();
        expected.insert(fold(&record.id), record.id.clone());
        snap.verify(&[&record], &expected)?;
        preflight_names(&self.dir, std::iter::once(record.id.as_str())).map_err(|e| {
            anyhow::anyhow!(
                "{}",
                e.to_string()
                    .replace("\nnothing was written", "\nnothing was changed")
            )
        })?;
        self.step(Step::Read)?;
        store::create(&self.dir, &record, text.as_bytes())?;
        self.step(Step::Written(&format!("{}.md", record.id)))?;
        Ok(self.changed("scout events create", record, true))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_date_is_a_calendar_day_written_in_full() {
        for ok in ["2026-10-11", "2024-02-29", "0001-01-01"] {
            assert!(is_date(ok), "{ok}");
        }
        for bad in [
            "2026-10-1",
            "2026-1-01",
            "2026-13-01",
            "2026-02-30",
            "2025-02-29",
            "2026/10/11",
            "2026-10-11 ",
            "２０２６-10-11",
            "2026-10-11T00:00:00Z",
            "",
            "today",
        ] {
            assert!(!is_date(bad), "{bad}");
        }
    }

    #[test]
    fn plain_text_is_trimmed_and_refuses_every_control_character() {
        assert_eq!(
            plain_text("the name", "  a b\u{a0}é 日本 ").unwrap(),
            "a b\u{a0}é 日本"
        );
        for bad in [
            "a\nb",
            "a\rb",
            "a\tb",
            "a\u{0}b",
            "a\u{1b}b",
            "a\u{7f}b",
            "a\u{85}b",
            "a\u{2028}b",
            "a\u{2029}b",
            "a\u{feff}b",
        ] {
            let e = plain_text("the name", bad).unwrap_err().to_string();
            assert!(
                e.contains("the name holds a control character"),
                "{bad:?}: {e}"
            );
        }
    }
}

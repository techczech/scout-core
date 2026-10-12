//! `scout events merges`: the merges of `merge-log.jsonl`, read only.
//!
//! The log has one reader ([`super::log::MergeLog`]) and one statement of
//! when a line is in effect ([`super::merge::state_of`]: record `a` is
//! merged into the line's `b`). Both callers here stand on them:
//!
//! - [`EventEditor::merges`] lists the lines in effect against the records
//!   folder as it is;
//! - [`live_merge_ids`] gives `scout events` the `merge_id` of each record
//!   it lists as merged in.
//!
//! Both get the lines in effect from one function ([`Live::in_effect`]),
//! over the folder as it is now, read as `unmerge` reads it
//! ([`Snapshot::read`], [`MergeLog::read`]): an id the listing gives is
//! an id `unmerge` finds in effect. The index's copy of the records
//! decides nothing here; it only says which entries the listing has, and
//! an entry the folder no longer agrees with gets no id.
//!
//! Whether `unmerge` would accept a merge in effect is asked of
//! `unmerge`'s own checks ([`unmerge_plan`], which `unmerge` runs before
//! its first write), once per merge, over the one snapshot and the one
//! log of the call ([`Live::why_not`]). Where they refuse, the row and
//! the listing's entry carry `undoable: false` and `why_not`, the
//! refusal word for word; where they accept, neither key.
//!
//! Nothing here writes.

use super::log::{By, LogLine, MergeLine, MergeLog};
use super::merge::{state_of, unmerge_plan, LineState};
use super::store::Snapshot;
use super::unmerge_check::Folder;
use super::EventEditor;
use crate::events::EventRecord;
use crate::normalize::fold;
use anyhow::{anyhow, Result};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;

/// The records folder and its merge log as they are now.
pub(super) struct Live {
    pub snap: Snapshot,
    pub log: MergeLog,
}

impl Live {
    /// Is `line` in effect: is the record its `a` names merged into its
    /// `b` in the folder?
    pub(super) fn holds(&self, line: &MergeLine) -> bool {
        state_of(self.snap.record(&line.a).ok(), line) == LineState::InEffect
    }

    /// The merges in effect: the lines no line undoes whose record `a` is
    /// merged into their `b`, oldest first. What `merges` lists and what
    /// `scout events` takes each `merge_id` from.
    fn in_effect(&self) -> impl Iterator<Item = &MergeLine> {
        self.log.live().filter(|l| self.holds(l))
    }

    /// Why `unmerge` would refuse `line` now, in its words; `None` when
    /// it would accept it. `at` is the time the undo line would carry.
    fn why_not(&self, folder: &Folder, line: &MergeLine, at: &str) -> Option<String> {
        unmerge_plan(folder, &self.log, &line.id, at.to_string())
            .err()
            .map(|e| format!("{e:#}"))
    }
}

/// One side of a merge: the record's id as the line has it, and the
/// record's name as it is now (null when the record is gone, which only
/// an undone line can show).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MergeSide {
    pub id: String,
    pub name: Option<String>,
}

/// One merge line: `a` was merged into `b`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MergeInfo {
    /// The line's id: what `unmerge` takes.
    pub id: String,
    pub at: String,
    pub by: By,
    pub reason: Option<String>,
    /// The record merged away.
    pub a: MergeSide,
    /// The surviving record.
    pub b: MergeSide,
    /// On an undone line only: the id and the time of its undo line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undone_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undone_at: Option<String>,
    /// On a merge in effect that `unmerge` would refuse now: `false`,
    /// with `why_not` the refusal, word for word. Both keys are absent
    /// on a merge `unmerge` would accept, and on an undone line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undoable: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why_not: Option<String>,
}

/// `scout events merges` (one JSON line).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MergeList {
    pub tool: String,
    pub at: String,
    /// The records folder.
    pub dir: String,
    /// Undone lines are listed too; the key is absent otherwise.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub all: bool,
    /// Newest first.
    pub merges: Vec<MergeInfo>,
}

impl EventEditor {
    /// Every merge line in effect (not undone, and record `a` is merged
    /// into the line's `b`), newest first. Reads only. A line that is
    /// neither undone nor in effect (`check` reports it as `not-applied`
    /// or `not-as-logged`) is no merge and is not listed.
    pub fn merges(&self) -> Result<Vec<MergeInfo>> {
        Ok(self.merge_list(false)?.merges)
    }

    /// [`EventEditor::merges`] plus the undone lines, each with
    /// `undone_by` and `undone_at`; newest first by the merge line.
    pub fn merges_all(&self) -> Result<Vec<MergeInfo>> {
        Ok(self.merge_list(true)?.merges)
    }

    /// What `scout events merges` prints; `all` also lists undone lines.
    pub fn merge_list(&self, all: bool) -> Result<MergeList> {
        let live = Live {
            snap: Snapshot::read(&self.dir)?,
            log: MergeLog::read(&self.dir)?,
        };
        let (snap, log) = (&live.snap, &live.log);
        let in_effect: BTreeMap<&str, &MergeLine> =
            live.in_effect().map(|l| (l.id.as_str(), l)).collect();
        let undo_at: BTreeMap<&str, &str> = log
            .lines
            .iter()
            .filter_map(|l| match l {
                LogLine::Undo(u) => Some((u.id.as_str(), u.at.as_str())),
                LogLine::Merge(_) => None,
            })
            .collect();
        let side = |id: &str| MergeSide {
            id: id.to_string(),
            name: snap.record(id).ok().map(|r| r.name.clone()),
        };
        let now = self.now();
        let folder = Folder::of(snap);
        let mut merges = Vec::new();
        for line in log.merges() {
            let undone_by = log.undone.get(&line.id);
            let shown = match undone_by {
                Some(_) => all,
                None => in_effect.contains_key(line.id.as_str()),
            };
            if !shown {
                continue;
            }
            let why_not = match undone_by {
                Some(_) => None,
                None => live.why_not(&folder, line, &now),
            };
            merges.push(MergeInfo {
                id: line.id.clone(),
                at: line.at.clone(),
                by: line.by,
                reason: line.reason.clone(),
                a: side(&line.a),
                b: side(&line.b),
                undone_by: undone_by.cloned(),
                undone_at: undone_by
                    .and_then(|u| undo_at.get(u.as_str()))
                    .map(|at| at.to_string()),
                undoable: why_not.as_ref().map(|_| false),
                why_not,
            });
        }
        // Ids only grow along the log.
        merges.reverse();
        Ok(MergeList {
            tool: "scout events merges".into(),
            at: now,
            dir: self.dir.display().to_string(),
            all,
            merges,
        })
    }
}

/// What `scout events` takes from the folder as it is now.
#[derive(Debug, Default)]
pub(crate) struct LiveMergeIds {
    /// By folded id of a record the index has as merged in: the id of
    /// the merge in effect for it now, and why `unmerge` would refuse
    /// that id now (`None`: it would accept it).
    pub ids: BTreeMap<String, (String, Option<String>)>,
    /// The index has a record merged in that the folder does not have
    /// merged in the same way: the folder changed since the index was
    /// built.
    pub stale: bool,
}

/// The `merge_id` of each record the index (`indexed`, by folded id) has
/// as merged in, from the merges in effect in the records folder `dir`
/// now ([`Live::in_effect`], the answer of `scout events merges`).
///
/// A record gets an id only when the folder and the index agree about
/// it: every record of its chain in the index, from the record to the
/// record the listing shows it under, is in the folder with the same
/// `merged_into` (so the merge's `a` and `b` are the entry's record and
/// the record the index has it merged into, and its event is the one
/// listed). Where they do not agree the record has no id and `stale` is
/// set. A record merged by hand has no id and is not stale; nor has any
/// record an id when the folder holds no log. Of two lines in effect for
/// one record (only hand edits leave that) the later one is given.
///
/// The folder is the records folder of `archive`, opened as `unmerge`
/// opens it ([`EventEditor::open`]: the same path rules, and the same
/// path in a refusal). An error says which of the two reads failed.
pub(crate) fn live_merge_ids(
    archive: &Path,
    indexed: &BTreeMap<String, EventRecord>,
) -> Result<LiveMergeIds> {
    let not_read = |e: anyhow::Error| anyhow!("the events folder was not read ({e:#})");
    let editor = EventEditor::open(archive).map_err(not_read)?;
    let dir = editor.dir();
    let live = Live {
        snap: Snapshot::read(dir).map_err(not_read)?,
        log: MergeLog::read(dir).map_err(|e| anyhow!("the merge log was not read ({e:#})"))?,
    };
    let now = &live.snap.set.records;
    let merged_into = |r: &EventRecord| r.merged_into.as_deref().map(fold);
    // Does the folder have this record as the index has it?
    let same =
        |k: &str, r: &EventRecord| now.get(k).is_some_and(|n| merged_into(n) == merged_into(r));
    // The index's chain from `k` to the record it is listed under.
    let agrees = |k: &str| {
        let mut cur = k.to_string();
        // A chain is at most as long as the set.
        for _ in 0..=indexed.len() {
            let Some(r) = indexed.get(&cur) else {
                return false;
            };
            if !same(&cur, r) {
                return false;
            }
            match merged_into(r) {
                Some(next) => cur = next,
                None => return true,
            }
        }
        false
    };
    let in_effect: BTreeMap<String, &MergeLine> =
        live.in_effect().map(|l| (fold(&l.a), l)).collect();
    let at = editor.now();
    let folder = Folder::of(&live.snap);
    let mut out = LiveMergeIds::default();
    for (k, _) in indexed.iter().filter(|(_, r)| r.merged_into.is_some()) {
        if !agrees(k) {
            out.stale = true;
        } else if let Some(line) = in_effect.get(k) {
            out.ids.insert(
                k.clone(),
                (line.id.clone(), live.why_not(&folder, line, &at)),
            );
        }
    }
    Ok(out)
}

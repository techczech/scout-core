//! `scout events merges`: the merges of `merge-log.jsonl`, read only.
//!
//! The log has one reader ([`super::log::MergeLog`]) and one statement of
//! when a line is in effect ([`super::merge::state_of`]: record `a` is
//! merged into the line's `b`). Both callers here stand on them:
//!
//! - [`EventEditor::merges`] lists the lines in effect against the records
//!   folder as it is;
//! - [`live_merge_ids`] gives `scout events` the `merge_id` of each record
//!   it lists as merged in, against the records the listing shows (the
//!   index's copy).
//!
//! Nothing here writes.

use super::log::{By, LogLine, MergeLine, MergeLog};
use super::merge::{state_of, LineState};
use super::store::Snapshot;
use super::EventEditor;
use crate::events::EventRecord;
use crate::normalize::fold;
use anyhow::Result;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;

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
        let snap = Snapshot::read(&self.dir)?;
        let log = MergeLog::read(&self.dir)?;
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
        let mut merges = Vec::new();
        for line in log.merges() {
            let undone_by = log.undone.get(&line.id);
            let shown = match undone_by {
                Some(_) => all,
                None => state_of(snap.record(&line.a).ok(), line) == LineState::InEffect,
            };
            if !shown {
                continue;
            }
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
            });
        }
        // Ids only grow along the log.
        merges.reverse();
        Ok(MergeList {
            tool: "scout events merges".into(),
            at: self.now(),
            dir: self.dir.display().to_string(),
            all,
            merges,
        })
    }
}

/// By folded id of each record merged away: the id of the merge line in
/// effect for it, told against `records` (by folded id) with the log in
/// the records folder `dir`. A record merged by hand has no entry; nor
/// has any record when the folder holds no log. Of two lines in effect
/// for one record (only hand edits leave that) the later one is given.
pub(crate) fn live_merge_ids(
    dir: &Path,
    records: &BTreeMap<String, EventRecord>,
) -> Result<BTreeMap<String, String>> {
    let log = MergeLog::read(dir)?;
    let in_effect = |l: &&MergeLine| state_of(records.get(&fold(&l.a)), l) == LineState::InEffect;
    Ok(log
        .live()
        .filter(in_effect)
        .map(|l| (fold(&l.a), l.id.clone()))
        .collect())
}

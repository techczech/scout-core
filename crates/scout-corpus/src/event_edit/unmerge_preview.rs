//! `scout events unmerge-preview`: what `unmerge <log_id>` would leave,
//! read only.
//!
//! Nothing here decides anything of its own. Each part of the answer is
//! asked of the code that makes it true:
//!
//! - whether the merge is in effect, of the listing's function
//!   ([`Live::holds`]); whether `unmerge` would accept it, of `unmerge`'s
//!   own checks ([`unmerge_plan`], which `unmerge` runs before its first
//!   write), so `why_not` is its refusal word for word;
//! - the records as the undo leaves them, of the reader over the folder
//!   with the plan's record in place ([`Snapshot::with`], what every edit
//!   checks before it writes);
//! - the span of the returning event, of [`combined`] over those records
//!   (what `scout events` lists);
//! - which events each item is in, of the index build's reader and its
//!   resolution ([`EventMembers`]), once over the records as they are and
//!   once over the records as the undo leaves them.
//!
//! What is read: the records folder and the merge log, as `unmerge` reads
//! them; then `analysis/topics/current.jsonl` and `corrections.jsonl`
//! under the archive, as an index build reads them (the layer file
//! streamed once within its line bound, the corrections file by its
//! reader). The index is not read, so a stale index changes nothing here,
//! and no item's own file is opened.
//!
//! Nothing here writes.

use super::merge::unmerge_plan;
use super::merges::Live;
use super::store::Snapshot;
use super::unmerge_check::Folder;
use super::EventEditor;
use crate::events::{combined, merged_in, EventRecord};
use crate::normalize::fold;
use crate::topics::EventMembers;
use anyhow::{bail, Result};
use serde::Serialize;

/// An event of the preview: its record's id and name as they are now.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PreviewEvent {
    pub id: String,
    pub name: String,
}

impl PreviewEvent {
    fn of(r: &EventRecord) -> PreviewEvent {
        PreviewEvent {
            id: r.id.clone(),
            name: r.name.clone(),
        }
    }
}

/// `scout events unmerge-preview` (one JSON line): what
/// `scout events unmerge <log_id>` would leave.
///
/// An item is a tag row of `analysis/topics/current.jsonl` (the last
/// valid row of its key). The index is not read, so the counts are of
/// tag rows: `scout events` counts the indexed documents the rows join,
/// which is the same number where every row joins one document with
/// text, and not otherwise (a row that joins no document counts here and
/// not there).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct UnmergePreview {
    /// The merge asked about, as given (trimmed).
    pub log_id: String,
    /// The event that becomes current again: the merge's record `a`.
    pub returning: PreviewEvent,
    /// The event that stays: the one `a` is part of now, which is the
    /// merge's `b`, or the record at the end of `b`'s own chain.
    pub kept: PreviewEvent,
    /// False when `unmerge` would refuse the merge now; `why_not` is then
    /// its refusal, word for word (what `scout events merges` and `scout
    /// events` give), the four counts are absent and `start` / `end` are
    /// null.
    pub undoable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub why_not: Option<String>,
    /// The returning event's span after the split, as `scout events`
    /// lists it: its own dates widened over the records that stay merged
    /// into it. Null where it has none.
    pub start: Option<String>,
    pub end: Option<String>,
    /// Items in the returning event after the split (those of `in_both`
    /// included).
    ///
    /// Which items those are is the index build's rule, applied to the
    /// records as the undo leaves them: the tag row's `event` and
    /// `events`, then the corrections' `event_add` / `event_remove` in
    /// line order, every id resolved through those records. So a
    /// correction names the event its id resolves to after the split,
    /// whatever it resolved to when it was written:
    ///
    /// - an item of `a` that a correction took out of the kept event by
    ///   the kept event's id (`event_remove: [<kept>]`) is in the
    ///   returning event again after the split: the removal names the
    ///   kept event, which no longer holds `a`'s ids;
    /// - one taken out by `a`'s id or by a monthly id `a` holds stays out
    ///   of both: the removal names the returning event after the split.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub returning_items: Option<usize>,
    /// Items in the kept event after the split (those of `in_both`
    /// included).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kept_items: Option<usize>,
    /// Items in both events after the split.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_both: Option<usize>,
    /// Items in the kept event now, before the split.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kept_items_now: Option<usize>,
}

impl EventEditor {
    /// What [`EventEditor::unmerge`] of `log_id` would leave, worked out
    /// from the files as they are now. Reads only (see the module notes
    /// for what is read and by which code).
    ///
    /// An error, with nothing computed, where `log_id` names no merge in
    /// effect: `unmerge`'s own refusal for no such line, a line already
    /// undone and a record `a` that is gone or merged elsewhere; and for
    /// a line never applied (which `unmerge` would withdraw, moving no
    /// item) an error that says so. A merge made by hand has no line and
    /// so no id to ask with.
    ///
    /// A merge in effect that `unmerge` would refuse now gives
    /// `undoable: false` with `why_not`, and no counts.
    pub fn unmerge_preview(&self, log_id: &str) -> Result<UnmergePreview> {
        let log_id = log_id.trim();
        let live = Live {
            snap: Snapshot::read(&self.dir)?,
            log: super::log::MergeLog::read(&self.dir)?,
        };
        let (snap, log) = (&live.snap, &live.log);
        let plan = unmerge_plan(&Folder::of(snap), log, log_id, self.now());
        let in_effect = log
            .merge(log_id)
            .filter(|l| !log.undone.contains_key(&l.id) && live.holds(l));
        let Some(line) = in_effect else {
            // Not a merge in effect: `unmerge`'s refusal, or its withdrawal.
            let plan = plan?;
            bail!(
                "{log_id}: the merge of {} into {} was never applied ({} is merged into nothing); `scout events unmerge {log_id}` would only withdraw its line, and no item would move",
                plan.line.a, plan.line.b, plan.line.a
            );
        };
        let a = snap.record(&line.a)?;
        // In effect: `a` is merged away, so its event is another record.
        let kept = snap
            .set
            .resolve(&a.id)
            .and_then(|id| snap.record(id).ok())
            .unwrap_or(a);
        let mut out = UnmergePreview {
            log_id: log_id.to_string(),
            returning: PreviewEvent::of(a),
            kept: PreviewEvent::of(kept),
            undoable: true,
            why_not: None,
            start: None,
            end: None,
            returning_items: None,
            kept_items: None,
            in_both: None,
            kept_items_now: None,
        };
        let plan = match plan {
            Ok(plan) => plan,
            Err(e) => {
                out.undoable = false;
                out.why_not = Some(format!("{e:#}"));
                return Ok(out);
            }
        };
        // The records as the undo leaves them, read by the reader.
        let after = snap.with(&[&plan.new_a]);
        let (a_key, kept_key) = (fold(&a.id), fold(&kept.id));
        let merged = merged_in(&after.records);
        let span = combined(
            &plan.new_a,
            merged.get(&a_key).map_or(&[][..], Vec::as_slice),
        );
        (out.start, out.end) = (span.start, span.end);

        let members = EventMembers::read(&self.archive)?;
        let has = |ids: &[String], key: &str| ids.iter().any(|e| fold(e) == key);
        let (mut returning, mut stays, mut both, mut now) = (0, 0, 0, 0);
        for (before, then) in members.under(&snap.set).zip(members.under(&after)) {
            let (in_a, in_kept) = (has(&then, &a_key), has(&then, &kept_key));
            returning += usize::from(in_a);
            stays += usize::from(in_kept);
            both += usize::from(in_a && in_kept);
            now += usize::from(has(&before, &kept_key));
        }
        out.returning_items = Some(returning);
        out.kept_items = Some(stays);
        out.in_both = Some(both);
        out.kept_items_now = Some(now);
        Ok(out)
    }
}

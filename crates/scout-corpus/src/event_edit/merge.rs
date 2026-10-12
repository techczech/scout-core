//! `merge`, `unmerge` and `check`.
//!
//! The rule. A merge of `a` into `b` changes one field of one record:
//! `a.merged_into` becomes `b`. `unmerge` clears it. Neither writes to `b`,
//! and neither writes anything else of `a`. What the combined event shows
//! (which ids it holds, its span, its aliases, the records merged into it)
//! is worked out by the reader each time it is asked
//! ([`crate::events::EventSet`], [`crate::events::combined`]); no record
//! holds a copy of another's values.
//!
//! So a merge that is in effect can be undone at any time, whatever was
//! merged or edited since, and its undo touches `a` only. The order of the
//! writes and the states of a merge line are in the notes of [`super`].

use super::log::{By, MergeLine, MergeLog, Planned, UndoLine, MERGE_LOG};
use super::store::{self, leftover_temps, still_as_read, Snapshot};
use super::{is_date, plain_text, EventEditor, MergeRequest, Step};
use crate::events::EventRecord;
use crate::normalize::fold;
use anyhow::{bail, Result};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// Why two records are not merged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Refusal {
    /// The spec's rule for model runs: a record with `edited` set is not
    /// merged away by a model.
    pub model_rule: bool,
    pub why: String,
}

impl Refusal {
    fn other(why: String) -> Refusal {
        Refusal {
            model_rule: false,
            why,
        }
    }
}

/// `scout events merge` (one JSON line).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Merged {
    pub tool: String,
    pub at: String,
    /// The id of the merge's line in `merge-log.jsonl`.
    pub log_id: String,
    /// The record merged away.
    pub a: String,
    /// The surviving record.
    pub b: String,
    pub by: By,
    pub reason: Option<String>,
    /// The record merged away as it is now: as it was, with `merged_into`.
    pub a_record: EventRecord,
}

/// `scout events unmerge` (one JSON line).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Unmerged {
    pub tool: String,
    pub at: String,
    /// The id of the undo line.
    pub log_id: String,
    /// The merge line undone.
    pub undoes: String,
    pub a: String,
    pub b: String,
    /// True when the merge had never been applied (a run cut short after
    /// its log line): only the undo line was written.
    pub withdrawn: bool,
    /// The record that was merged away, as it is now.
    pub a_record: EventRecord,
}

/// One thing `scout events check` found.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Problem {
    /// In `problems`: `not-applied`, `not-as-logged` or `temporary-file`.
    /// In `information`: `merged-without-log`.
    pub kind: String,
    /// The merge line, when the finding is one line's.
    pub log_id: Option<String>,
    pub a: Option<String>,
    pub b: Option<String>,
    pub detail: String,
    /// What to run, or do.
    pub fix: String,
}

/// `scout events check` (one JSON line).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CheckReport {
    pub tool: String,
    pub at: String,
    /// The records folder.
    pub dir: String,
    /// No problem found (`information` does not count).
    pub ok: bool,
    pub records: usize,
    /// Merge lines in the log, and how many of them an undo line undoes.
    pub merges: usize,
    pub undone: usize,
    /// Merge lines that are in effect.
    pub in_effect: usize,
    pub problems: Vec<Problem>,
    /// Findings that are not faults: a record merged into another with no
    /// merge line in effect for it (`merged_into` set by hand).
    pub information: Vec<Problem>,
}

/// The record `a` as a merge into `b` leaves it: `merged_into: b`, every
/// other value as it was. No file is looked at; `b` is only read.
///
/// Refused: `a` and `b` are one record; either is already merged into
/// another, is removed, or has a `start` or `end` that is not a date; a
/// model's merge of an `a` with `edited` set (his record would be hidden
/// behind another). `edited` on `b` refuses nothing: `b` is not written.
pub(super) fn merged_record(
    a: &EventRecord,
    b: &EventRecord,
    by: By,
) -> Result<EventRecord, Refusal> {
    if fold(&a.id) == fold(&b.id) {
        return Err(Refusal::other(format!(
            "{}: an event cannot be merged into itself",
            a.id
        )));
    }
    for r in [a, b] {
        if let Some(into) = &r.merged_into {
            return Err(Refusal::other(format!(
                "{}: already merged into {into}",
                r.id
            )));
        }
        if r.removed.is_some() {
            return Err(Refusal::other(format!(
                "{}: a removed event; restore it first",
                r.id
            )));
        }
        for (what, d) in [("start", &r.start), ("end", &r.end)] {
            if d.as_deref().is_some_and(|d| !is_date(d)) {
                return Err(Refusal::other(format!(
                    "{}: {what} {:?} is not a date YYYY-MM-DD; set it first",
                    r.id,
                    d.as_deref().unwrap_or_default()
                )));
            }
        }
    }
    if by == By::Model && a.edited.is_some() {
        return Err(Refusal {
            model_rule: true,
            why: format!(
                "{}: edited {}; a model's merge does not merge away a record he edited",
                a.id,
                a.edited.as_deref().unwrap_or_default()
            ),
        });
    }
    Ok(EventRecord {
        merged_into: Some(b.id.clone()),
        ..a.clone()
    })
}

/// The state of a merge line, told from record `a` as it is.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum LineState {
    /// `a` is merged into `b`.
    InEffect,
    /// `a` is merged into nothing.
    NotApplied,
    /// `a` is gone, or is merged into another record.
    Other(String),
}

/// The state of `line`, with `a` the record its `a` names (`None`: no
/// such record). The one statement of when a merge line is in effect:
/// `merge`, `unmerge`, `check`, `merges` and the `merge_id` of `scout
/// events` all ask here.
pub(super) fn state_of(a: Option<&EventRecord>, line: &MergeLine) -> LineState {
    let Some(a) = a else {
        return LineState::Other(format!("the record {} is no longer there", line.a));
    };
    match &a.merged_into {
        Some(into) if fold(into) == fold(&line.b) => LineState::InEffect,
        Some(into) => LineState::Other(format!("{} is now merged into {into}", a.id)),
        None => LineState::NotApplied,
    }
}

fn line_state(snap: &Snapshot, line: &MergeLine) -> LineState {
    state_of(snap.record(&line.a).ok(), line)
}

/// A merge of `a` that is in the log, not undone and not applied: a run
/// cut short after its log line.
pub(super) fn not_applied_line<'l>(
    snap: &Snapshot,
    log: &'l MergeLog,
    a: &str,
) -> Option<&'l MergeLine> {
    log.live()
        .find(|l| fold(&l.a) == fold(a) && line_state(snap, l) == LineState::NotApplied)
}

/// The reader's id table as it is meant to be once `a` is merged into
/// nothing: `a`, every record merged into `a` (directly or through a
/// chain) and each monthly id those records hold now resolve to `a`;
/// nothing else moves.
fn table_without_merge(snap: &Snapshot, a: &EventRecord) -> BTreeMap<String, String> {
    let records = &snap.set.records;
    let a_key = fold(&a.id);
    let reaches_a = |r: &EventRecord| {
        let mut cur = r;
        // A chain is at most as long as the set.
        for _ in 0..=records.len() {
            if fold(&cur.id) == a_key {
                return true;
            }
            match cur
                .merged_into
                .as_deref()
                .and_then(|t| records.get(&fold(t)))
            {
                Some(next) => cur = next,
                None => return false,
            }
        }
        false
    };
    let mut expected = snap.set.aliases.clone();
    let holder = expected.get(&a_key).cloned();
    for (k, r) in records.iter().filter(|(_, r)| reaches_a(r)) {
        // A member that is a record id is not a monthly id (the reader
        // passes over it).
        let members = r.members.iter().map(|m| fold(m));
        let keys = std::iter::once(k.clone()).chain(members.filter(|m| !records.contains_key(m)));
        for key in keys {
            if let Some(have) = expected.get_mut(&key) {
                if Some(&*have) == holder.as_ref() {
                    *have = a.id.clone();
                }
            }
        }
    }
    expected
}

impl EventEditor {
    /// Merge `a` into `b`: append the merge's line to `merge-log.jsonl`,
    /// then rewrite record `a` with `merged_into: b` and nothing else
    /// changed. Record `b` is read and is not written.
    ///
    /// Refused, with nothing changed: what [`merged_record`] refuses;
    /// either record is missing; a merge of `a` that an earlier run logged
    /// and did not apply (`unmerge` that line first); a reason with a
    /// control character; a record `a` whose frontmatter is not as scout
    /// writes it. A cycle cannot be asked for: it would need a `b` that is
    /// merged away. The folder as it would be is still checked for one.
    pub fn merge(&self, req: &MergeRequest) -> Result<Merged> {
        let reason = match &req.reason {
            Some(r) => Some(plain_text("the reason", r)?).filter(|r| !r.is_empty()),
            None => None,
        };
        let snap = Snapshot::read(&self.dir)?;
        let mut log = MergeLog::read(&self.dir)?;
        let (a, b) = (snap.record(&req.a)?, snap.record(&req.b)?);
        let new_a = merged_record(a, b, req.by)
            .map_err(|r| anyhow::anyhow!("{} (nothing was changed)", r.why))?;
        if let Some(open) = not_applied_line(&snap, &log, &a.id) {
            bail!(
                "{}: the merge {} of it into {} was logged and never applied; run `scout events unmerge {}` to withdraw it, then merge again (nothing was changed)",
                a.id, open.id, open.b, open.id
            );
        }
        // An undo clears `merged_into` on `a` and nothing else, so nothing
        // else of `a` may change here.
        let only_merged = EventRecord {
            merged_into: Some(b.id.clone()),
            ..a.clone()
        };
        if new_a != only_merged || a.merged_into.is_some() {
            bail!(
                "{}: the merge would change more of it than merged_into (nothing was changed)",
                a.id
            );
        }
        let wa = snap.rewrite(&new_a, false)?;
        // Meant: every id `a` holds is held by `b`; nothing else moves.
        let mut expected = snap.set.aliases.clone();
        for holder in expected.values_mut() {
            if *holder == a.id {
                *holder = b.id.clone();
            }
        }
        snap.verify(&[&new_a], &expected)?;
        // That the log can take the line: the question `apply-proposals`
        // asks of every merge it plans, answered by the checks the append
        // below makes again on the descriptor it writes to.
        let line = log
            .ready_to_append(Planned::default(), |id| MergeLine {
                id,
                at: self.now(),
                by: req.by,
                reason: reason.clone(),
                a: a.id.clone(),
                b: b.id.clone(),
                a_merged_into_before: a.merged_into.clone(),
            })?
            .line()?;
        self.step(Step::Read)?;
        still_as_read(&snap.dir, &wa.file, &wa.old)?;
        let (log_id, at) = (line.id.clone(), line.at.clone());
        // First the line (nothing has changed if this fails: the append
        // makes every check of the log before it writes), then `a`.
        log.append_merge(line)?;
        let record = || -> Result<()> {
            self.step(Step::Written(MERGE_LOG))?;
            store::replace(&snap.dir, &wa, &|s| self.step(s))?;
            self.step(Step::Written(&wa.file))?;
            Ok(())
        };
        if let Err(e) = record() {
            bail!(
                "{e:#}\nthe merge {log_id} is in {} and may not be applied; `scout events check` says whether it is, `scout events unmerge {log_id}` takes it back",
                log.path().display()
            );
        }
        Ok(Merged {
            tool: "scout events merge".into(),
            at,
            log_id,
            a: new_a.id.clone(),
            b: b.id.clone(),
            by: req.by,
            reason,
            a_record: new_a,
        })
    }

    /// Undo the merge of the log line `log_id`: rewrite record `a` with
    /// `merged_into` cleared and nothing else changed, then append an undo
    /// line. Record `b` is not read for its values and is not written. No
    /// other merge line is looked at: a merge in effect is undone whatever
    /// was merged or edited after it.
    ///
    /// That the log can take the undo line (see `log.rs`) is checked
    /// before the record is written.
    ///
    /// A line that was never applied (a merge cut short after its log
    /// line, or an undo cut short before its own: `a` is merged into
    /// nothing) is withdrawn: only the undo line is written.
    ///
    /// Refused, with nothing changed: no such merge line; already undone;
    /// record `a` is gone; `a` is merged into a record other than `b`
    /// (`merged_into` was changed by other means); the reader would not
    /// read the folder as meant with `a` unmerged (for one, a monthly id
    /// that `a` and another record of the event both list, by a hand
    /// edit); the log cannot take the undo line.
    pub fn unmerge(&self, log_id: &str) -> Result<Unmerged> {
        let log_id = log_id.trim();
        let snap = Snapshot::read(&self.dir)?;
        let mut log = MergeLog::read(&self.dir)?;
        let Some(line) = log.merge(log_id).cloned() else {
            bail!(
                "{log_id}: no such merge in {} (nothing was changed)",
                log.path().display()
            );
        };
        if let Some(by) = log.undone.get(log_id) {
            bail!("{log_id}: already undone, by {by} (nothing was changed)");
        }
        let a = snap.record(&line.a)?;
        let withdrawn = match line_state(&snap, &line) {
            LineState::InEffect => false,
            LineState::NotApplied => true,
            LineState::Other(why) => bail!(
                "{log_id}: {} is no longer merged into {} ({why}); the merge cannot be undone (nothing was changed)",
                a.id, line.b
            ),
        };
        let mut new_a = a.clone();
        let mut write = None;
        if !withdrawn {
            new_a.merged_into = None;
            let wa = snap.rewrite(&new_a, false)?;
            snap.verify(&[&new_a], &table_without_merge(&snap, a))?;
            write = Some(wa);
        }
        let undo = UndoLine {
            id: log.next_id()?,
            at: self.now(),
            undoes: line.id.clone(),
        };
        self.step(Step::Read)?;
        if let Some(w) = &write {
            still_as_read(&snap.dir, &w.file, &w.old)?;
        }
        // The line goes out last; that the log can take it is seen now.
        log.ready_for_undo(&undo)?;
        let (undo_id, at) = (undo.id.clone(), undo.at.clone());
        let mut wrote = false;
        let all = || -> Result<()> {
            if let Some(w) = &write {
                store::replace(&snap.dir, w, &|s| self.step(s))?;
                wrote = true;
                self.step(Step::Written(&w.file))?;
            }
            log.append_undo(undo)?;
            wrote = false;
            self.step(Step::Written(MERGE_LOG))?;
            Ok(())
        };
        if let Err(e) = all() {
            if !wrote {
                return Err(e);
            }
            bail!(
                "{e:#}\nthe undo of {log_id} is not finished: {} is unmerged and the undo line is not in the log; run `scout events unmerge {log_id}` again",
                line.a
            );
        }
        Ok(Unmerged {
            tool: "scout events unmerge".into(),
            at,
            log_id: undo_id,
            undoes: line.id,
            a: new_a.id.clone(),
            b: line.b,
            withdrawn,
            a_record: new_a,
        })
    }

    /// Compare `merge-log.jsonl` with the records. Reads only;
    /// `merge-decisions.jsonl` is read too, so a malformed line in either
    /// log is an error here.
    ///
    /// Problems (`ok` is false with any):
    ///
    /// - `not-applied`: a merge line no undo line undoes whose `a` is
    ///   merged into nothing (a merge cut short after its log line, or an
    ///   undo cut short before its own);
    /// - `not-as-logged`: such a line whose `a` is gone or is merged into
    ///   a record other than its `b`;
    /// - `temporary-file`: a temporary file an edit left.
    ///
    /// Information (`ok` stays true): `merged-without-log`, a record
    /// merged into another with no merge line in effect for it. Setting
    /// `merged_into` by hand is allowed; `unmerge` has no line to undo it
    /// by.
    pub fn check(&self) -> Result<CheckReport> {
        let snap = Snapshot::read(&self.dir)?;
        let log = MergeLog::read(&self.dir)?;
        super::log::Decisions::read(&self.dir)?;
        let mut problems = Vec::new();
        let mut in_effect = 0;
        let mut logged: BTreeSet<(String, String)> = BTreeSet::new();
        for line in log.live() {
            let undo = format!("scout events unmerge {}", line.id);
            let (kind, detail, fix) = match line_state(&snap, line) {
                LineState::InEffect => {
                    in_effect += 1;
                    logged.insert((fold(&line.a), fold(&line.b)));
                    continue;
                }
                LineState::NotApplied => (
                    "not-applied",
                    format!(
                        "the merge of {} into {} is in the log and {} is merged into nothing (a merge cut short after its log line, or an undo cut short before its own)",
                        line.a, line.b, line.a
                    ),
                    format!("`{undo}` withdraws it; then merge again if it is wanted"),
                ),
                LineState::Other(why) => (
                    "not-as-logged",
                    why,
                    "the record was changed by other means; put its merged_into right by hand"
                        .to_string(),
                ),
            };
            problems.push(Problem {
                kind: kind.into(),
                log_id: Some(line.id.clone()),
                a: Some(line.a.clone()),
                b: Some(line.b.clone()),
                detail,
                fix,
            });
        }
        let mut information = Vec::new();
        for r in snap.set.records.values() {
            let Some(into) = &r.merged_into else { continue };
            if !logged.contains(&(fold(&r.id), fold(into))) {
                information.push(Problem {
                    kind: "merged-without-log".into(),
                    log_id: None,
                    a: Some(r.id.clone()),
                    b: Some(into.clone()),
                    detail: format!(
                        "{} is merged into {into} and no merge line in effect says so (merged_into set by hand)",
                        r.id
                    ),
                    fix: "nothing is wrong; to undo it, clear merged_into by hand (`unmerge` has no line for it)"
                        .into(),
                });
            }
        }
        for name in leftover_temps(&self.dir)? {
            problems.push(Problem {
                kind: "temporary-file".into(),
                log_id: None,
                a: None,
                b: None,
                detail: format!(
                    "{}: left by an edit that was cut short",
                    self.dir.join(&name).display()
                ),
                fix: "remove the file".into(),
            });
        }
        Ok(CheckReport {
            tool: "scout events check".into(),
            at: self.now(),
            dir: self.dir.display().to_string(),
            ok: problems.is_empty(),
            records: snap.set.records.len(),
            merges: log.merges().count(),
            undone: log.undone.len(),
            in_effect,
            problems,
            information,
        })
    }
}

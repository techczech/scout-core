//! `merge`, `unmerge` and `check`. The order of the writes and the three
//! states of a merge line are in the notes of [`super`].

use super::log::{By, MergeLine, MergeLog, Pair, UndoLine, MERGE_LOG};
use super::store::{self, leftover_temps, still_as_read, Snapshot};
use super::{is_date, plain_text, EventEditor, MergeRequest, Step};
use crate::events::EventRecord;
use crate::normalize::fold;
use anyhow::{bail, Result};
use serde::Serialize;
use std::collections::BTreeSet;

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
    /// The members `b` gained.
    pub members_added: Vec<String>,
    /// The surviving record as it is now.
    pub record: EventRecord,
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
    /// The two records as they are now.
    pub a_record: EventRecord,
    pub b_record: EventRecord,
}

/// One thing `scout events check` found.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Problem {
    /// `not-applied`, `half-applied`, `not-as-logged`, `merged-without-log`
    /// or `temporary-file`.
    pub kind: String,
    /// The merge line, when the problem is one line's.
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
    /// No problem found.
    pub ok: bool,
    pub records: usize,
    /// Merge lines in the log, and how many of them an undo line undoes.
    pub merges: usize,
    pub undone: usize,
    /// Merge lines that are in effect.
    pub in_effect: usize,
    pub problems: Vec<Problem>,
}

/// `have` with every entry of `more` it does not hold yet (under the
/// reader's fold), in order.
fn with_new(have: &[String], more: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut out = have.to_vec();
    let mut seen: BTreeSet<String> = have.iter().map(|s| fold(s)).collect();
    for m in more {
        if seen.insert(fold(&m)) {
            out.push(m);
        }
    }
    out
}

/// The entries of `after` that `before` does not hold (under the fold).
fn added(before: &[String], after: &[String]) -> Vec<String> {
    let had: BTreeSet<String> = before.iter().map(|s| fold(s)).collect();
    after
        .iter()
        .filter(|s| !had.contains(&fold(s)))
        .cloned()
        .collect()
}

/// `have` without the entries of `gone` (under the fold).
fn without(have: &[String], gone: &[String]) -> Vec<String> {
    let gone: BTreeSet<String> = gone.iter().map(|s| fold(s)).collect();
    have.iter()
        .filter(|s| !gone.contains(&fold(s)))
        .cloned()
        .collect()
}

/// The two records as a merge of `a` into `b` leaves them: `(a, b)`. No
/// file is looked at.
///
/// - `a` gains `merged_into: b` and is otherwise as it was (it keeps its
///   members, so the reader resolves them to `b` through it).
/// - `b.members` gains `a`'s members; `b.end` is widened to cover `a`
///   (`b.end` empty stays empty; `a`'s last known date is its `end`, else
///   its `start`).
/// - `b.aliases` gains `a.name` and `a`'s aliases (none that folds to
///   `b.name` or to an alias `b` has) and `b.start` moves back to
///   `a.start` when that is earlier. Not in a model's merge into a record
///   with `edited` set: there only `members` and `end` change.
/// - `edited` is not set on either record: a merge is in the log.
pub(super) fn merged_pair(
    a: &EventRecord,
    b: &EventRecord,
    by: By,
) -> Result<(EventRecord, EventRecord), Refusal> {
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
    let mut new_a = a.clone();
    new_a.merged_into = Some(b.id.clone());
    let mut new_b = b.clone();
    new_b.members = with_new(&b.members, a.members.iter().cloned());
    let a_last = a.end.as_ref().or(a.start.as_ref());
    if let (Some(end), Some(last)) = (&b.end, a_last) {
        if last > end {
            new_b.end = Some(last.clone());
        }
    }
    if !(by == By::Model && b.edited.is_some()) {
        let names = std::iter::once(a.name.clone())
            .chain(a.aliases.iter().cloned())
            .filter(|n| fold(n) != fold(&b.name));
        new_b.aliases = with_new(&b.aliases, names);
        new_b.start = match (&b.start, &a.start) {
            (Some(have), Some(other)) if other < have => Some(other.clone()),
            (None, Some(other)) => Some(other.clone()),
            (have, _) => have.clone(),
        };
    }
    Ok((new_a, new_b))
}

/// The state of a merge line, told from the two records as they are.
#[derive(Debug, PartialEq, Eq)]
enum LineState {
    /// `a` is merged into `b` and `b` has every member the merge gave it.
    InEffect,
    /// `a` is merged into `b`; `b` lacks members the merge gives it.
    Half(Vec<String>),
    /// `a` is merged into nothing and `b` has none of those members.
    NotApplied,
    /// Anything else: a record is gone, or was changed by other means.
    Other(String),
}

fn line_state(snap: &Snapshot, line: &MergeLine) -> LineState {
    let (Ok(a), Ok(b)) = (snap.record(&line.a), snap.record(&line.b)) else {
        let gone = if snap.has(&line.a) { &line.b } else { &line.a };
        return LineState::Other(format!("the record {gone} is no longer there"));
    };
    let gave = added(&line.before.b.members, &line.after.b.members);
    let have: BTreeSet<String> = b.members.iter().map(|m| fold(m)).collect();
    let missing: Vec<String> = gave
        .iter()
        .filter(|m| !have.contains(&fold(m)))
        .cloned()
        .collect();
    match &a.merged_into {
        Some(into) if fold(into) == fold(&b.id) && missing.is_empty() => LineState::InEffect,
        Some(into) if fold(into) == fold(&b.id) => LineState::Half(missing),
        Some(into) => LineState::Other(format!("{} is now merged into {into}", a.id)),
        None if missing.len() == gave.len() => LineState::NotApplied,
        None => LineState::Other(format!(
            "{} is no longer merged into {}, which still has its members",
            a.id, b.id
        )),
    }
}

/// The surviving record of `line` as the log's merges into it leave its
/// names and dates: the record as it was before the first logged merge
/// into it, then every merge line into it that no line undoes (but for
/// `without`), in log order, each made by [`merged_pair`] from the `a` its
/// line holds. With one merge into the record this is the line's `after`
/// (and its `before`, without it).
fn as_merges_leave(log: &MergeLog, line: &MergeLine, without: Option<&str>) -> EventRecord {
    let mut into = log.merges().filter(|m| fold(&m.b) == fold(&line.b));
    let mut b = into.next().map_or(&line.before.b, |m| &m.before.b).clone();
    for m in log.merges().filter(|m| fold(&m.b) == fold(&line.b)) {
        if log.undone.contains_key(&m.id) || without == Some(m.id.as_str()) {
            continue;
        }
        // The rule for model merges looked at `edited` as it was then.
        b.edited = m.before.b.edited.clone();
        if let Ok((_, next)) = merged_pair(&m.before.a, &b, m.by) {
            b = next;
        }
    }
    b
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

impl EventEditor {
    /// Merge `a` into `b` (see [`merged_pair`] for what changes) and
    /// append the merge's line to `merge-log.jsonl`.
    ///
    /// Refused, with nothing changed: `a` and `b` are one record; either
    /// is missing, already merged into another, or removed; either has a
    /// `start` or `end` that is not a date; a model's merge of an `a` with
    /// `edited` set; a merge of `a` that an earlier run logged and did not
    /// apply (the line's state is not-applied: `unmerge` it first); a
    /// reason with a control
    /// character. A cycle cannot be asked for: it would need a `b` that is
    /// merged away. The folder as it would be is still checked for one.
    pub fn merge(&self, req: &MergeRequest) -> Result<Merged> {
        let reason = match &req.reason {
            Some(r) => Some(plain_text("the reason", r)?).filter(|r| !r.is_empty()),
            None => None,
        };
        let snap = Snapshot::read(&self.dir)?;
        let mut log = MergeLog::read(&self.dir)?;
        let (a, b) = (snap.record(&req.a)?, snap.record(&req.b)?);
        let (new_a, new_b) = merged_pair(a, b, req.by)
            .map_err(|r| anyhow::anyhow!("{} (nothing was changed)", r.why))?;
        if let Some(open) = not_applied_line(&snap, &log, &a.id) {
            bail!(
                "{}: the merge {} of it into {} was logged and never applied; run `scout events unmerge {}` to withdraw it, then merge again (nothing was changed)",
                a.id, open.id, open.b, open.id
            );
        }
        let (wa, wb) = (snap.rewrite(&new_a, false)?, snap.rewrite(&new_b, false)?);
        // Meant: every id `a` holds is held by `b`; nothing else moves.
        let mut expected = snap.set.aliases.clone();
        let mut moved = Vec::new();
        for (k, holder) in expected.iter_mut() {
            if *holder == a.id {
                *holder = b.id.clone();
                moved.push(k.clone());
            }
        }
        snap.verify(&[&new_a, &new_b], &expected)?;
        let line = MergeLine {
            id: log.next_id(),
            at: self.now(),
            by: req.by,
            reason: reason.clone(),
            a: a.id.clone(),
            b: b.id.clone(),
            before: Pair {
                a: a.clone(),
                b: b.clone(),
            },
            after: Pair {
                a: new_a.clone(),
                b: new_b.clone(),
            },
            moved,
        };
        self.step(Step::Read)?;
        still_as_read(&snap.dir, &wa.file, &wa.old)?;
        still_as_read(&snap.dir, &wb.file, &wb.old)?;
        let (log_id, at) = (line.id.clone(), line.at.clone());
        // First the line (nothing has changed if this fails), then `a`,
        // then `b`.
        log.append_merge(line)?;
        let records = || -> Result<()> {
            self.step(Step::Written(MERGE_LOG))?;
            store::replace(&snap.dir, &wa, &|s| self.step(s))?;
            self.step(Step::Written(&wa.file))?;
            if wb.changes() {
                store::replace(&snap.dir, &wb, &|s| self.step(s))?;
                self.step(Step::Written(&wb.file))?;
            }
            Ok(())
        };
        if let Err(e) = records() {
            bail!(
                "{e:#}\nthe merge {log_id} is in {} and is not finished; `scout events check` says how far it got, `scout events unmerge {log_id}` takes it back",
                log.path().display()
            );
        }
        Ok(Merged {
            tool: "scout events merge".into(),
            at,
            log_id,
            a: new_a.id,
            b: new_b.id.clone(),
            by: req.by,
            reason,
            members_added: added(&b.members, &new_b.members),
            record: new_b,
        })
    }

    /// Undo the merge of the log line `log_id`: `a` is no longer merged
    /// into `b`, and `b` loses exactly the members the merge gave it.
    /// `b`'s names and dates go back only where they still are what the
    /// merges set. With this the one merge into `b`: the aliases the merge
    /// added that `b` still has are taken off, and `start` and `end` go
    /// back to the line's `before` when they still equal its `after` (a
    /// value he changed since is kept). With several merges into `b`,
    /// "what the merges set" is worked out from the log
    /// ([`as_merges_leave`]): `b` before the first of them with every
    /// merge in effect made again, once with this one and once without;
    /// an alias only the first holds is taken off, and a date that equals
    /// the first becomes the second. So undoing merges in any order ends
    /// where it began, and a date still covers the records left in the
    /// event. An undo line is appended.
    ///
    /// A line that was never applied (a merge cut short after its log
    /// line: `a` is merged into nothing and `b` has none of the members)
    /// is withdrawn: only the undo line is written.
    ///
    /// Refused, with nothing changed: no such merge line; already undone;
    /// a record of the line is gone; `b` has since been merged into
    /// another record (undo that merge first; not asked of a line that
    /// was never applied); `a` is no longer merged into `b` (it is merged
    /// into another record, or into none while `b` still has its
    /// members).
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
        let (a, b) = (snap.record(&line.a)?, snap.record(&line.b)?);
        let state = line_state(&snap, &line);
        if let (Some(into), false) = (&b.merged_into, state == LineState::NotApplied) {
            let later = log
                .live()
                .filter(|l| fold(&l.a) == fold(&b.id) && fold(&l.b) == fold(into))
                .last()
                .map(|l| format!(" (`scout events unmerge {}`)", l.id))
                .unwrap_or_default();
            bail!(
                "{log_id}: {} has since been merged into {into}; undo that later merge first{later} (nothing was changed)",
                b.id
            );
        }
        let withdrawn = match &state {
            LineState::InEffect | LineState::Half(_) => false,
            LineState::NotApplied => true,
            LineState::Other(why) => bail!(
                "{log_id}: {} is no longer merged into {} ({why}); the merge cannot be undone (nothing was changed)",
                a.id, b.id
            ),
        };
        let (mut new_a, mut new_b) = (a.clone(), b.clone());
        let mut writes = Vec::new();
        if !withdrawn {
            new_a.merged_into = None;
            let gave = added(&line.before.b.members, &line.after.b.members);
            new_b.members = without(&b.members, &gave);
            // `b` as the merges in effect leave it, and as they leave it
            // without this one.
            let left = as_merges_leave(&log, &line, None);
            let wanted = as_merges_leave(&log, &line, Some(&line.id));
            new_b.aliases = without(&b.aliases, &added(&wanted.aliases, &left.aliases));
            if b.start == left.start {
                new_b.start = wanted.start;
            }
            if b.end == left.end {
                new_b.end = wanted.end;
            }
            let (wa, wb) = (snap.rewrite(&new_a, false)?, snap.rewrite(&new_b, false)?);
            // Meant: the ids the merge moved are held by `a` again.
            let mut expected = snap.set.aliases.clone();
            for k in &line.moved {
                if let Some(holder) = expected.get_mut(k) {
                    *holder = a.id.clone();
                }
            }
            snap.verify(&[&new_a, &new_b], &expected)?;
            // `b` first: while `a` is still merged into it, its ids
            // resolve as they did.
            writes = vec![wb, wa];
        }
        let undo = UndoLine {
            id: log.next_id(),
            at: self.now(),
            undoes: line.id.clone(),
        };
        self.step(Step::Read)?;
        for w in &writes {
            still_as_read(&snap.dir, &w.file, &w.old)?;
        }
        let (undo_id, at) = (undo.id.clone(), undo.at.clone());
        let mut wrote = false;
        let all = || -> Result<()> {
            for w in writes.iter().filter(|w| w.changes()) {
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
                "{e:#}\nthe undo of {log_id} is not finished; run `scout events unmerge {log_id}` again"
            );
        }
        Ok(Unmerged {
            tool: "scout events unmerge".into(),
            at,
            log_id: undo_id,
            undoes: line.id,
            a: new_a.id.clone(),
            b: new_b.id.clone(),
            withdrawn,
            a_record: new_a,
            b_record: new_b,
        })
    }

    /// Compare `merge-log.jsonl` with the records. Reports every merge
    /// line no undo line undoes whose records are not in the state it
    /// describes (`not-applied`, `half-applied`, `not-as-logged`), every
    /// record merged into another with no such line for it
    /// (`merged-without-log`), and every temporary file an edit left
    /// (`temporary-file`). Reads only; `merge-decisions.jsonl` is read
    /// too, so a malformed line in either log is an error here.
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
                        "the merge of {} into {} is in the log; the first is merged into nothing and the second has none of its members (a merge cut short after its log line, or an undo cut short before its own)",
                        line.a, line.b
                    ),
                    format!("`{undo}` withdraws it; then merge again if it is wanted"),
                ),
                LineState::Half(missing) => {
                    logged.insert((fold(&line.a), fold(&line.b)));
                    (
                        "half-applied",
                        format!(
                            "{} is merged into {}, which lacks the members the merge gives it: {}",
                            line.a,
                            line.b,
                            missing.join(", ")
                        ),
                        format!("`{undo}` takes it back; then merge again"),
                    )
                }
                LineState::Other(why) => (
                    "not-as-logged",
                    why,
                    "the records were changed by other means; put them right by hand".to_string(),
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
        for r in snap.set.records.values() {
            let Some(into) = &r.merged_into else { continue };
            if !logged.contains(&(fold(&r.id), fold(into))) {
                problems.push(Problem {
                    kind: "merged-without-log".into(),
                    log_id: None,
                    a: Some(r.id.clone()),
                    b: Some(into.clone()),
                    detail: format!(
                        "{} is merged into {into} and no merge line in effect says so (an undo cut short, or a hand edit)",
                        r.id
                    ),
                    fix: "run the `unmerge` that was cut short again, or clear merged_into by hand"
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
        })
    }
}

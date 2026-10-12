//! `apply-proposals`, `proposals` and `decide`: the merge pass's proposals
//! file (`<records folder>/merge-proposals/<run-id>.jsonl`, written by the
//! tagger, one JSON object per line) turned into merges. This is the
//! file's only reader here; it is never written.
//!
//! Every merge made from a proposal is [`EventEditor::merge`], one pair at
//! a time, with all its checks. Which pairs, and which record of a pair
//! survives, is worked out first on a copy of the records in memory with
//! the function `merge` itself uses ([`merged_record`]), the reader's own
//! resolution ([`EventSet`]) deciding after each pair which event an id
//! is in. As in `merge`, only the record merged away changes; "starts
//! earlier" is by each surviving record's own `start`.

use super::log::{By, Decision, DecisionLine, Decisions, MergeLine, MergeLog};
use super::merge::{merged_record, not_applied_line, Merged};
use super::store::{read_plain, Snapshot};
use super::{plain_text, EventEditor, MergeRequest, Step};
use crate::event_seed::refuse_parent_dir;
use crate::events::{EventRecord, EventSet};
use crate::normalize::fold;
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

/// The proposals folder, inside the records folder.
pub const PROPOSALS_DIR: &str = "merge-proposals";
const MAX_PROPOSALS_BYTES: u64 = 256 << 20;
/// Largest `a_items` / `b_items` taken from a proposals line. The file is
/// not this module's: a count is used only to pick the survivor of a tie.
const MAX_ITEMS: u64 = 10_000_000;

/// One judged pair of a proposals file.
#[derive(Debug, Clone)]
struct Proposal {
    /// Its line in the file, from 1.
    line: usize,
    a: String,
    b: String,
    verdict: String,
    confidence: String,
    reason: String,
    a_items: u64,
    b_items: u64,
}

impl Proposal {
    fn pair(&self) -> (String, String) {
        pair_key(&self.a, &self.b)
    }
}

/// A pair whatever its order.
fn pair_key(a: &str, b: &str) -> (String, String) {
    let (a, b) = (fold(a), fold(b));
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

struct ProposalsFile {
    /// The file's name inside the proposals folder.
    name: String,
    path: PathBuf,
    lines: usize,
    /// Lines with an `error`, and lines without a `verdict`.
    error_lines: usize,
    no_verdict_lines: usize,
    /// Judged lines whose `a_items` or `b_items` is not a whole number
    /// from 0 to [`MAX_ITEMS`].
    malformed_lines: usize,
    proposals: Vec<Proposal>,
}

/// A merge `apply-proposals` makes, or would make.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PlannedMerge {
    /// The record merged away.
    pub a: String,
    /// The surviving record.
    pub b: String,
    pub reason: Option<String>,
    /// The proposal's line in the file.
    pub line: usize,
    /// The merge's line in `merge-log.jsonl` (with `apply`).
    pub log_id: Option<String>,
}

/// A `same` + `confident` pair that is not merged.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SkippedPair {
    pub a: String,
    pub b: String,
    pub line: usize,
    /// `already-one-event`, `no-record`, `model-rule` or `refused`.
    pub why: String,
    pub detail: String,
}

/// `scout events apply-proposals` (one JSON line).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ApplyReport {
    pub tool: String,
    pub at: String,
    pub file: String,
    pub apply: bool,
    /// Lines of the file; of them, lines skipped for an `error` and for
    /// having no `verdict`.
    pub lines: usize,
    pub error_lines: usize,
    pub no_verdict_lines: usize,
    /// Judged lines skipped because `a_items` or `b_items` is not a whole
    /// number from 0 to 10,000,000.
    pub malformed_lines: usize,
    /// Pairs judged `same` and `confident`.
    pub same_confident: usize,
    /// The merges, in the order they are made.
    pub merges: Vec<PlannedMerge>,
    /// Merges made (0 without `apply`).
    pub merged: usize,
    pub skipped: Vec<SkippedPair>,
    /// `skipped` by `why`.
    pub skipped_counts: BTreeMap<String, usize>,
}

/// A pair waiting for his decision.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WaitingPair {
    pub a: String,
    pub b: String,
    pub a_name: String,
    pub b_name: String,
    pub a_items: u64,
    pub b_items: u64,
    pub confidence: String,
    pub reason: String,
    /// `doubtful`, or `model-rule` for a confident pair the rule for
    /// model merges refused.
    pub why: String,
    /// The refusal, for `model-rule`.
    pub detail: Option<String>,
    pub line: usize,
    /// What approving does: the event `from` is merged into `into`.
    pub from: String,
    pub into: String,
}

/// `scout events proposals` (one JSON line).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WaitingReport {
    pub tool: String,
    pub at: String,
    pub file: String,
    pub lines: usize,
    /// Judged lines skipped for a malformed `a_items` or `b_items`.
    pub malformed_lines: usize,
    pub waiting: Vec<WaitingPair>,
    /// Pairs left out: decided, already in one event, a record missing.
    pub decided: usize,
    pub already_one_event: usize,
    pub no_record: usize,
}

/// `scout events decide` (one JSON line).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Decided {
    pub tool: String,
    pub at: String,
    pub file: String,
    pub a: String,
    pub b: String,
    pub decision: Decision,
    /// The merge an approval made.
    pub merge: Option<Merged>,
}

/// Where one candidate pair of the waiting list stands.
enum Standing {
    Waiting(Box<WaitingPair>),
    Decided(Decision),
    AlreadyOneEvent,
    NoRecord(String),
}

/// The records under a run of merges, in memory.
struct Sim<'a> {
    snap: &'a Snapshot,
    log: &'a MergeLog,
    records: BTreeMap<String, EventRecord>,
    set: EventSet,
    /// Items per event, by the folded id of the record that holds it now:
    /// the counts the proposal lines give for every record of the event
    /// that the file names, added up. A survivor's grows with each merge.
    items: BTreeMap<String, u64>,
    /// The records the merges so far have changed, by folded id.
    changed: BTreeMap<String, EventRecord>,
    /// The log lines the merges so far append: how many, and their bytes.
    planned_lines: u64,
    planned_bytes: u64,
    /// The time the planned log lines carry (its length is what counts).
    at: String,
}

/// Which of two events survives a merge: the one whose own record starts
/// earlier (its `start`, not the span of what is merged into it; one
/// without a start counts as latest); on a tie the one with more items;
/// then the one whose id sorts first. Gives `(from, into)`.
fn direction<'r>(
    x: &'r EventRecord,
    y: &'r EventRecord,
    items: &BTreeMap<String, u64>,
) -> (&'r EventRecord, &'r EventRecord) {
    let n = |r: &EventRecord| items.get(&fold(&r.id)).copied().unwrap_or(0);
    let x_survives = match (&x.start, &y.start) {
        (Some(a), Some(b)) if a != b => a < b,
        (Some(_), None) => true,
        (None, Some(_)) => false,
        _ if n(x) != n(y) => n(x) > n(y),
        _ => fold(&x.id) < fold(&y.id),
    };
    if x_survives {
        (y, x)
    } else {
        (x, y)
    }
}

impl<'a> Sim<'a> {
    /// The count of an event is kept under the record that holds it now.
    /// A record the file names that is already merged into another (by an
    /// earlier run over this file, or by any other merge) adds its count
    /// to its holder's, as the merge of it in this run would have. So a
    /// run cut short and taken up again counts as one run does.
    fn new(snap: &'a Snapshot, log: &'a MergeLog, file: &ProposalsFile, at: &str) -> Sim<'a> {
        let mut own: BTreeMap<String, u64> = BTreeMap::new();
        for p in &file.proposals {
            for (id, n) in [(&p.a, p.a_items), (&p.b, p.b_items)] {
                let have = own.entry(fold(id)).or_default();
                *have = (*have).max(n);
            }
        }
        let mut items: BTreeMap<String, u64> = BTreeMap::new();
        for (id, n) in own {
            let holder = snap.set.resolve(&id).map(fold).unwrap_or(id);
            let have = items.entry(holder).or_default();
            *have = have.saturating_add(n);
        }
        Sim {
            snap,
            log,
            records: snap.set.records.clone(),
            set: snap.set.clone(),
            items,
            changed: BTreeMap::new(),
            planned_lines: 0,
            planned_bytes: 0,
            at: at.to_string(),
        }
    }

    /// The events the two ids are in now, or why the pair is not for
    /// merging.
    fn holders(&self, a: &str, b: &str) -> Result<(&EventRecord, &EventRecord), (String, String)> {
        for id in [a, b] {
            if !self.records.contains_key(&fold(id)) {
                return Err((
                    "no-record".into(),
                    format!("{}: no such event record", id.trim()),
                ));
            }
        }
        let holder = |id: &str| {
            self.set
                .resolve(id)
                .and_then(|h| self.records.get(&fold(h)))
        };
        let (Some(ha), Some(hb)) = (holder(a), holder(b)) else {
            return Err(("no-record".into(), "an id resolves to no record".into()));
        };
        if fold(&ha.id) == fold(&hb.id) {
            return Err(("already-one-event".into(), format!("both are in {}", ha.id)));
        }
        Ok((ha, hb))
    }

    /// Try the model's merge of one confident pair; on success the copy
    /// of the records moves on.
    ///
    /// Every refusal [`EventEditor::merge`] would make of this merge that
    /// can be told from what was read is made here, on the records and
    /// the log as the merges before it leave them: the pair, the rule for
    /// model merges, a merge logged and not applied, the record's
    /// frontmatter, the reader's id table with the merge made
    /// ([`Snapshot::verify`]), the next log id, and the size of the log
    /// line and of the log with it. What is left to the merge itself is
    /// what only the moment of the write can tell (a file changed since
    /// it was read, a write that fails).
    fn merge(&mut self, p: &Proposal) -> Result<PlannedMerge, SkippedPair> {
        let skip = |(why, detail): (String, String)| SkippedPair {
            a: p.a.clone(),
            b: p.b.clone(),
            line: p.line,
            why,
            detail,
        };
        let refused = |detail: String| skip(("refused".into(), detail));
        let (ha, hb) = self.holders(&p.a, &p.b).map_err(skip)?;
        let (from, into) = direction(ha, hb, &self.items);
        let reason = plain_text("the reason", &p.reason)
            .map_err(|_| refused("the proposal's reason holds a control character".into()))?;
        let new_a = merged_record(from, into, By::Model).map_err(|r| {
            let why = if r.model_rule {
                "model-rule"
            } else {
                "refused"
            };
            skip((why.into(), r.why))
        })?;
        if let Some(open) = not_applied_line(self.snap, self.log, &from.id) {
            return Err(refused(format!(
                "{}: the merge {} of it was logged and never applied; `scout events unmerge {}` withdraws it",
                from.id, open.id, open.id
            )));
        }
        self.snap
            .rewrite(&new_a, false)
            .map_err(|e| refused(e.to_string()))?;
        let into_id = into.id.clone();
        // Meant, as in `merge`: every id `from` holds is held by `into`.
        let mut expected = self.set.aliases.clone();
        for holder in expected.values_mut() {
            if *holder == new_a.id {
                *holder = into_id.clone();
            }
        }
        let with: Vec<&EventRecord> = self
            .changed
            .values()
            .filter(|r| fold(&r.id) != fold(&new_a.id))
            .chain(std::iter::once(&new_a))
            .collect();
        self.snap
            .verify(&with, &expected)
            .map_err(|e| refused(e.to_string()))?;
        let planned = PlannedMerge {
            a: new_a.id.clone(),
            b: into_id.clone(),
            reason: Some(reason).filter(|r| !r.is_empty()),
            line: p.line,
            log_id: None,
        };
        let line = MergeLine {
            id: self
                .log
                .id_after(self.planned_lines)
                .map_err(|e| refused(e.to_string()))?,
            at: self.at.clone(),
            by: By::Model,
            reason: planned.reason.clone(),
            a: planned.a.clone(),
            b: planned.b.clone(),
            a_merged_into_before: None,
        };
        let bytes = self
            .log
            .fits_after(&line, self.planned_bytes)
            .map_err(|e| refused(e.to_string()))?;
        self.planned_lines += 1;
        self.planned_bytes = self.planned_bytes.saturating_add(bytes);
        self.changed.insert(fold(&new_a.id), new_a.clone());
        let moved = self.items.get(&fold(&new_a.id)).copied().unwrap_or(0);
        let have = self.items.entry(fold(&into_id)).or_default();
        *have = have.saturating_add(moved);
        self.records.insert(fold(&new_a.id), new_a);
        self.set = EventSet::from_records(
            self.snap.dir.clone(),
            self.records.values().cloned().collect(),
        );
        Ok(planned)
    }

    /// Every `same` + `confident` pair of the file, in file order.
    fn run(mut self, file: &ProposalsFile) -> (Vec<PlannedMerge>, Vec<SkippedPair>) {
        let (mut merges, mut skipped) = (Vec::new(), Vec::new());
        for p in file.proposals.iter().filter(|p| is_confident_same(p)) {
            match self.merge(p) {
                Ok(m) => merges.push(m),
                Err(s) => skipped.push(s),
            }
        }
        (merges, skipped)
    }
}

fn is_confident_same(p: &Proposal) -> bool {
    p.verdict == "same" && p.confidence == "confident"
}

impl EventEditor {
    /// Read a proposals file. It must be a plain file named `*.jsonl`
    /// directly inside `<records folder>/merge-proposals/` (itself a real
    /// folder): the path given is only used to find that folder and the
    /// file's name, and the file opened is the one of that name in that
    /// folder, without following a link.
    ///
    /// A line that is not a JSON object stops the read (the file may still
    /// be being written), as does a judged line whose `a`, `b` or
    /// `verdict` is not text. A line with an `error`, and one without a
    /// `verdict`, is skipped and counted. So is a judged line whose
    /// `a_items` or `b_items` is there and is not a whole number from 0
    /// to 10,000,000 (`malformed_lines`); one that is missing or null is 0.
    fn read_proposals(&self, given: &Path) -> Result<ProposalsFile> {
        refuse_parent_dir(given, "the proposals file")?;
        let folder = self.dir.join(PROPOSALS_DIR);
        if !std::fs::symlink_metadata(&folder).is_ok_and(|m| m.is_dir()) {
            bail!(
                "{}: no proposals folder (a real folder, not a link)",
                folder.display()
            );
        }
        let name = given
            .file_name()
            .and_then(|n| n.to_str())
            .filter(|n| n.ends_with(".jsonl") && !n.starts_with('.'))
            .with_context(|| {
                format!(
                    "{}: not a proposals file (a name ending .jsonl)",
                    given.display()
                )
            })?
            .to_string();
        let mut absolute = if given.is_absolute() {
            PathBuf::new()
        } else {
            std::env::current_dir().context("the current folder")?
        };
        absolute.extend(given.components().filter(|c| *c != Component::CurDir));
        let parent = absolute.parent().unwrap_or(Path::new("/"));
        let inside = std::fs::canonicalize(parent).is_ok_and(|p| p == folder);
        if !inside {
            bail!(
                "{}: not inside {}; a proposals file is read only from there",
                given.display(),
                folder.display()
            );
        }
        let path = folder.join(&name);
        let Some(plain) = read_plain(&path, MAX_PROPOSALS_BYTES)? else {
            bail!("{}: no such proposals file", path.display());
        };
        let at = path.display();
        let Ok(text) = std::str::from_utf8(&plain.bytes) else {
            bail!("{at}: not valid UTF-8");
        };
        let mut file = ProposalsFile {
            name,
            path: path.clone(),
            lines: 0,
            error_lines: 0,
            no_verdict_lines: 0,
            malformed_lines: 0,
            proposals: Vec::new(),
        };
        for (i, raw) in text.lines().enumerate() {
            if raw.trim().is_empty() {
                continue;
            }
            let line = i + 1;
            file.lines += 1;
            let v: serde_json::Value = serde_json::from_str(raw).map_err(|e| {
                anyhow::anyhow!(
                    "{at}: line {line}: not JSON ({e}); is the file still being written? (nothing was changed)"
                )
            })?;
            let Some(o) = v.as_object() else {
                bail!("{at}: line {line}: not a JSON object (nothing was changed)");
            };
            let present = |k: &str| o.get(k).filter(|v| !v.is_null());
            if present("error").is_some() {
                file.error_lines += 1;
                continue;
            }
            if present("verdict").is_none() {
                file.no_verdict_lines += 1;
                continue;
            }
            let text_of = |k: &str| -> Result<String> {
                match o.get(k).and_then(|v| v.as_str()) {
                    Some(s) => Ok(s.to_string()),
                    None => bail!("{at}: line {line}: {k:?} is not text (nothing was changed)"),
                }
            };
            let optional = |k: &str| -> Result<String> {
                match present(k) {
                    None => Ok(String::new()),
                    Some(_) => text_of(k),
                }
            };
            // Missing or null is 0; anything else must be a whole number
            // in range.
            let count = |k: &str| match present(k) {
                None => Some(0),
                Some(v) => v.as_u64().filter(|n| *n <= MAX_ITEMS),
            };
            let proposal = Proposal {
                line,
                a: text_of("a")?,
                b: text_of("b")?,
                verdict: text_of("verdict")?,
                confidence: optional("confidence")?,
                reason: optional("reason")?,
                a_items: count("a_items").unwrap_or(0),
                b_items: count("b_items").unwrap_or(0),
            };
            if count("a_items").is_none() || count("b_items").is_none() {
                file.malformed_lines += 1;
                continue;
            }
            file.proposals.push(proposal);
        }
        Ok(file)
    }

    /// Merge every pair the file judges `same` and `confident`, as a
    /// model's merge, in file order: the event that starts later into the
    /// one that starts earlier, by the `start` of each event's own
    /// surviving record (a tie: the one with more items survives,
    /// by the line's `a_items` / `b_items`; then the id that sorts first).
    /// A pair is taken as the two events its ids are in by then, so
    /// A~B and B~C end in one event, the earliest. An event's item count
    /// is that of every record of it the file names, added up, whether
    /// the records were merged in this run or before it: a run cut short
    /// and taken up again picks the survivors one run would. Skipped and
    /// counted: a pair already in one event, a pair naming a missing
    /// record, a merge the rule for model merges refuses (`a` has
    /// `edited` set), and any other refusal of [`EventEditor::merge`]
    /// that can be told before the write (the size of the log line and of
    /// the log among them).
    ///
    /// The run without `apply` and the run with it work out the same
    /// merges and the same skipped pairs, by the same code; `apply` then
    /// makes the merges.
    ///
    /// Without `apply` nothing is written. With it each merge is made by
    /// [`EventEditor::merge`], whole, with its own log line; an error in
    /// one stops the run, the merges before it stay, and a second run
    /// makes the rest.
    pub fn apply_proposals(&self, file: &Path, apply: bool) -> Result<ApplyReport> {
        let file = self.read_proposals(file)?;
        let snap = Snapshot::read(&self.dir)?;
        let log = MergeLog::read(&self.dir)?;
        let (mut merges, skipped) = Sim::new(&snap, &log, &file, &self.now()).run(&file);
        let mut merged = 0;
        if apply {
            let total = merges.len();
            for m in &mut merges {
                let done = self.merge(&MergeRequest {
                    a: m.a.clone(),
                    b: m.b.clone(),
                    by: By::Model,
                    reason: m.reason.clone(),
                });
                match done {
                    Ok(done) => m.log_id = Some(done.log_id),
                    Err(e) => bail!(
                        "{e:#}\n{}: line {}: the merge of {} into {} stopped the run; {merged} of {total} merges were made before it and stay (each is whole and in the log); run again for the rest",
                        file.path.display(), m.line, m.a, m.b
                    ),
                }
                merged += 1;
            }
        }
        let mut skipped_counts: BTreeMap<String, usize> = BTreeMap::new();
        for s in &skipped {
            *skipped_counts.entry(s.why.clone()).or_default() += 1;
        }
        Ok(ApplyReport {
            tool: "scout events apply-proposals".into(),
            at: self.now(),
            file: file.path.display().to_string(),
            apply,
            lines: file.lines,
            error_lines: file.error_lines,
            no_verdict_lines: file.no_verdict_lines,
            malformed_lines: file.malformed_lines,
            same_confident: file
                .proposals
                .iter()
                .filter(|p| is_confident_same(p))
                .count(),
            merges,
            merged,
            skipped,
            skipped_counts,
        })
    }

    /// Every candidate of the waiting list with where it stands: the
    /// pairs judged `same` and `doubtful`, and the `same` + `confident`
    /// pairs the rule for model merges refuses, each pair once.
    fn standings(
        &self,
        file: &ProposalsFile,
        decisions: &Decisions,
    ) -> Result<Vec<(Proposal, Standing)>> {
        let snap = Snapshot::read(&self.dir)?;
        let log = MergeLog::read(&self.dir)?;
        let sim = Sim::new(&snap, &log, file, &self.now());
        let items = sim.items.clone();
        let (_, skipped) = sim.run(file);
        let ruled: BTreeMap<usize, String> = skipped
            .into_iter()
            .filter(|s| s.why == "model-rule")
            .map(|s| (s.line, s.detail))
            .collect();
        let decided: BTreeMap<(String, String), Decision> = decisions
            .lines
            .iter()
            .filter(|d| d.file == file.name)
            .map(|d| (pair_key(&d.a, &d.b), d.decision))
            .collect();
        // The records as they are on disk, not after the confident merges.
        let now = Sim::new(&snap, &log, file, &self.now());
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for p in &file.proposals {
            let why = match (p.verdict.as_str(), p.confidence.as_str()) {
                ("same", "doubtful") => "doubtful",
                ("same", "confident") if ruled.contains_key(&p.line) => "model-rule",
                _ => continue,
            };
            if !seen.insert(p.pair()) {
                continue;
            }
            let standing = if let Some(d) = decided.get(&p.pair()) {
                Standing::Decided(*d)
            } else {
                match now.holders(&p.a, &p.b) {
                    Err((kind, detail)) if kind == "no-record" => Standing::NoRecord(detail),
                    Err(_) => Standing::AlreadyOneEvent,
                    Ok((ha, hb)) => {
                        let (from, into) = direction(ha, hb, &items);
                        let name = |id: &str| snap.record(id).map(|r| r.name.clone());
                        Standing::Waiting(Box::new(WaitingPair {
                            a: p.a.clone(),
                            b: p.b.clone(),
                            a_name: name(&p.a)?,
                            b_name: name(&p.b)?,
                            a_items: p.a_items,
                            b_items: p.b_items,
                            confidence: p.confidence.clone(),
                            reason: p.reason.clone(),
                            why: why.into(),
                            detail: ruled.get(&p.line).cloned(),
                            line: p.line,
                            from: from.id.clone(),
                            into: into.id.clone(),
                        }))
                    }
                }
            };
            out.push((p.clone(), standing));
        }
        Ok(out)
    }

    /// The pairs of a proposals file still waiting for him: judged `same`
    /// and `doubtful`, or `same` and `confident` but refused by the rule
    /// for model merges; without the pairs `merge-decisions.jsonl` holds a
    /// decision for (for this file), the pairs whose two records are
    /// already in one event, and the pairs naming a record that is not
    /// there. Reads only.
    pub fn proposals(&self, file: &Path) -> Result<WaitingReport> {
        let file = self.read_proposals(file)?;
        let decisions = Decisions::read(&self.dir)?;
        let mut report = WaitingReport {
            tool: "scout events proposals".into(),
            at: self.now(),
            file: file.path.display().to_string(),
            lines: file.lines,
            malformed_lines: file.malformed_lines,
            waiting: Vec::new(),
            decided: 0,
            already_one_event: 0,
            no_record: 0,
        };
        for (_, standing) in self.standings(&file, &decisions)? {
            match standing {
                Standing::Waiting(w) => report.waiting.push(*w),
                Standing::Decided(_) => report.decided += 1,
                Standing::AlreadyOneEvent => report.already_one_event += 1,
                Standing::NoRecord(_) => report.no_record += 1,
            }
        }
        Ok(report)
    }

    /// Decide a waiting pair (in either order). `Approve` merges it as his
    /// merge, with the proposal's reason, in the direction
    /// `apply-proposals` would take; `Dismiss` merges nothing. Either way
    /// one line goes to `merge-decisions.jsonl`, after the merge; that
    /// the file can take the line (see `log.rs`) is checked before the
    /// merge.
    ///
    /// Refused, with nothing changed: the pair is not waiting (no such
    /// proposal, already decided, already in one event, a record
    /// missing); `merge-decisions.jsonl` cannot take the line; an
    /// approval whose merge is refused.
    pub fn decide(&self, file: &Path, a: &str, b: &str, decision: Decision) -> Result<Decided> {
        let file = self.read_proposals(file)?;
        let mut decisions = Decisions::read(&self.dir)?;
        let want = pair_key(a, b);
        let at = file.path.display();
        let found = self
            .standings(&file, &decisions)?
            .into_iter()
            .find(|(p, _)| p.pair() == want);
        let (p, w) = match found {
            Some((p, Standing::Waiting(w))) => (p, w),
            Some((_, Standing::Decided(d))) => bail!(
                "{at}: this pair is already decided ({}) (nothing was changed)",
                serde_json::to_string(&d)?.trim_matches('"')
            ),
            Some((_, Standing::AlreadyOneEvent)) => {
                bail!("{at}: the two are already in one event (nothing was changed)")
            }
            Some((_, Standing::NoRecord(why))) => bail!("{at}: {why} (nothing was changed)"),
            None => bail!(
                "{at}: no proposal waiting for {} and {} (nothing was changed)",
                a.trim(),
                b.trim()
            ),
        };
        let line = DecisionLine {
            at: self.now(),
            file: file.name.clone(),
            a: p.a.clone(),
            b: p.b.clone(),
            decision,
        };
        decisions.ready(&line)?;
        let merge = match decision {
            Decision::Approve => Some(self.merge(&MergeRequest {
                a: w.from.clone(),
                b: w.into.clone(),
                by: By::Dominik,
                reason: Some(p.reason.clone()),
            })?),
            Decision::Dismiss => {
                self.step(Step::Read)?;
                None
            }
        };
        let stamp = line.at.clone();
        if let Err(e) = decisions.append(line) {
            match &merge {
                Some(m) => bail!(
                    "{e:#}\nthe merge was made ({}) and the decision was not recorded; the pair is no longer waiting",
                    m.log_id
                ),
                None => return Err(e),
            }
        }
        Ok(Decided {
            tool: "scout events decide".into(),
            at: stamp,
            file: file.path.display().to_string(),
            a: p.a,
            b: p.b,
            decision,
            merge,
        })
    }
}

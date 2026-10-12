//! Seam tests for `scout events apply-proposals`, `proposals` and
//! `decide`: which pairs of a proposals file are merged and in which
//! direction, chains, the rule for model merges, what waits for him, his
//! decisions, and where a proposals file may be read from. The proposals
//! file is never written; a refusal leaves every file as it was.

mod event_edit_support;
use event_edit_support::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

const LATE: &str = "2026-12-late";
const TWIN_A: &str = "2026-12-twin-a";
const TWIN_B: &str = "2026-12-twin-b";
const FILE: &str = "2026-10-11-merge-1.jsonl";

fn line(
    a: &str,
    b: &str,
    verdict: &str,
    confidence: &str,
    reason: &str,
    items: (u64, u64),
) -> Value {
    json!({
        "a": a, "b": b, "a_name": "A", "b_name": "B", "a_items": items.0, "b_items": items.1,
        "a_span": ["2026-01-01", "2026-01-02"], "signals": ["alias-token"],
        "verdict": verdict, "confidence": confidence, "reason": reason,
        "model": "test", "judged_at": NOW,
    })
}

/// The fixture with three more events and a proposals file of eleven
/// lines (see the test below for what each is).
fn with_proposals() -> (Fx, PathBuf) {
    let fx = fixture();
    put(
        &fx.dir,
        &rec(LATE, "Late", ("2026-12-02", "2026-12-05"), &["2026-12/E09"]),
    );
    put(
        &fx.dir,
        &rec(
            TWIN_A,
            "Twin A",
            ("2026-12-01", "2026-12-01"),
            &["2026-12/E01"],
        ),
    );
    put(
        &fx.dir,
        &rec(
            TWIN_B,
            "Twin B",
            ("2026-12-01", "2026-12-03"),
            &["2026-12/E02"],
        ),
    );
    let lines = [
        line(ALPHA, BETA, "same", "confident", "r1: one story", (3, 5)),
        line(BETA, LATE, "same", "confident", "r2: its sequel", (5, 2)),
        line(DELTA, GAMMA, "same", "confident", "r3", (1, 1)),
        line(TWIN_A, TWIN_B, "same", "confident", "", (2, 7)),
        line(ALPHA, "2026-01-nobody", "same", "confident", "r5", (3, 1)),
        line(BETA, ALPHA, "same", "confident", "r6: again", (5, 3)),
        line(ALPHA, DELTA, "same", "doubtful", "r7: maybe", (3, 1)),
        line(GAMMA, TWIN_B, "different", "confident", "r8", (1, 7)),
        json!({"a": GAMMA, "b": TWIN_A, "error": "timeout"}),
        json!({"a": GAMMA, "b": TWIN_A, "verdict": null}),
        line(GAMMA, LATE, "same", "doubtful", "r11: perhaps", (1, 2)),
    ];
    let folder = fx.dir.join(PROPOSALS_DIR);
    fs::create_dir(&folder).unwrap();
    let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
    let file = folder.join(FILE);
    fs::write(&file, format!("{text}\n")).unwrap();
    fs::write(folder.join("2026-10-11-merge-1-ledger.json"), "{}\n").unwrap();
    (fx, file)
}

fn pairs(merges: &[scout_corpus::event_edit::PlannedMerge]) -> Vec<(&str, &str, usize)> {
    merges
        .iter()
        .map(|m| (m.a.as_str(), m.b.as_str(), m.line))
        .collect()
}

fn skips(r: &scout_corpus::event_edit::ApplyReport) -> Vec<(usize, &str)> {
    r.skipped.iter().map(|s| (s.line, s.why.as_str())).collect()
}

#[test]
fn confident_pairs_are_merged_later_into_earlier_and_chains_end_in_the_earliest() {
    let (fx, file) = with_proposals();
    let before = fx.tree();
    let e = fx.editor();
    // Dry run: the plan, and nothing written.
    let dry = e.apply_proposals(&file, false).unwrap();
    assert!(fx.tree() == before);
    assert_eq!(
        (
            dry.apply,
            dry.merged,
            dry.lines,
            dry.error_lines,
            dry.no_verdict_lines,
            dry.same_confident
        ),
        (false, 0, 11, 1, 1, 6)
    );
    // Line 1: beta (October) into alpha (September). Line 2: beta is in
    // alpha by then, so late goes into alpha. Line 4: the twins start the
    // same day; the one with more items survives.
    let plan = vec![(BETA, ALPHA, 1), (LATE, ALPHA, 2), (TWIN_A, TWIN_B, 4)];
    assert_eq!(pairs(&dry.merges), plan);
    assert!(dry.merges.iter().all(|m| m.log_id.is_none()));
    assert_eq!(
        skips(&dry),
        vec![
            (3, "model-rule"),
            (5, "no-record"),
            (6, "already-one-event")
        ]
    );
    assert!(
        dry.skipped[0]
            .detail
            .contains("2026-11-gamma: edited 2026-10-10"),
        "{:?}",
        dry.skipped[0]
    );
    assert_eq!(dry.skipped_counts.get("model-rule"), Some(&1));
    // Apply: the same plan, each merge logged as the model's.
    let done = e.apply_proposals(&file, true).unwrap();
    assert_eq!((done.apply, done.merged), (true, 3));
    assert_eq!(pairs(&done.merges), plan);
    assert_eq!(skips(&done), skips(&dry));
    let ids: Vec<&str> = done
        .merges
        .iter()
        .map(|m| m.log_id.as_deref().unwrap())
        .collect();
    assert_eq!(ids, ["m000001", "m000002", "m000003"]);
    let log = fx.log(MERGE_LOG);
    let logged: Vec<(&str, &str, &str, &Value)> = log
        .iter()
        .map(|l| {
            (
                l["a"].as_str().unwrap(),
                l["b"].as_str().unwrap(),
                l["by"].as_str().unwrap(),
                &l["reason"],
            )
        })
        .collect();
    assert_eq!(
        logged,
        vec![
            (BETA, ALPHA, "model", &json!("r1: one story")),
            (LATE, ALPHA, "model", &json!("r2: its sequel")),
            (TWIN_A, TWIN_B, "model", &Value::Null),
        ]
    );
    let set = fx.set();
    for id in [BETA, LATE, "2026-10/E04", "2026-12/E09"] {
        assert_eq!(set.resolve(id), Some(ALPHA), "{id}");
    }
    assert_eq!(set.resolve(TWIN_A), Some(TWIN_B));
    assert_eq!(set.resolve(GAMMA), Some(GAMMA));
    // The surviving record's file is never written; the event's span is
    // derived from the records merged into it.
    assert_eq!(fx.bytes(ALPHA), before[&fx.dir.join(format!("{ALPHA}.md"))]);
    let alpha = events::combined(&set.records[ALPHA], &events::merged_in(&set.records)[ALPHA]);
    assert_eq!(
        (alpha.start.as_deref(), alpha.end.as_deref()),
        (Some("2026-09-05"), Some("2026-12-05"))
    );
    let from: Vec<&str> = alpha.merged_from.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(from, [BETA, LATE]);
    assert!(e.check().unwrap().ok);
    // Only records and the log changed; the proposals file never does.
    assert_eq!(fs::read(&file).unwrap(), before[&file]);
    // A second run finds every pair done.
    let state = fx.tree();
    let again = e.apply_proposals(&file, true).unwrap();
    assert_eq!((again.merged, again.merges.len()), (0, 0));
    assert_eq!(
        skips(&again),
        vec![
            (1, "already-one-event"),
            (2, "already-one-event"),
            (3, "model-rule"),
            (4, "already-one-event"),
            (5, "no-record"),
            (6, "already-one-event")
        ]
    );
    assert!(fx.tree() == state);
}

#[test]
fn a_chain_ends_in_the_earliest_event_whatever_the_order_of_its_lines() {
    for order in [[0, 1, 2], [2, 1, 0], [1, 2, 0]] {
        let fx = fixture();
        put(
            &fx.dir,
            &rec(LATE, "Late", ("2026-12-02", "2026-12-05"), &["2026-12/E09"]),
        );
        let all = [
            line(LATE, BETA, "same", "confident", "x", (1, 1)),
            line(BETA, ALPHA, "same", "confident", "y", (1, 1)),
            line(ALPHA, DELTA, "same", "confident", "z", (1, 1)),
        ];
        let folder = fx.dir.join(PROPOSALS_DIR);
        fs::create_dir(&folder).unwrap();
        let text: String = order.iter().map(|i| format!("{}\n", all[*i])).collect();
        fs::write(folder.join(FILE), text).unwrap();
        let r = fx
            .editor()
            .apply_proposals(&folder.join(FILE), true)
            .unwrap();
        assert_eq!(r.merged, 3, "{order:?}");
        let set = fx.set();
        for id in [
            LATE,
            BETA,
            ALPHA,
            DELTA,
            "2026-12/E09",
            "2026-10/E04",
            "2026-09/E01",
        ] {
            assert_eq!(set.resolve(id), Some(DELTA), "{order:?} {id}");
        }
        // Delta's own record is as it was; the span is derived.
        assert_eq!(
            fx.bytes(DELTA),
            render_record(&rec(
                DELTA,
                "Delta",
                ("2026-08-01", "2026-08-02"),
                &["2026-08/E07"]
            ))
            .into_bytes()
        );
        let d = events::combined(&set.records[DELTA], &events::merged_in(&set.records)[DELTA]);
        assert_eq!(
            (d.start.as_deref(), d.end.as_deref(), d.merged_from.len()),
            (Some("2026-08-01"), Some("2026-12-05"), 3)
        );
        assert!(fx.editor().check().unwrap().ok);
    }
}

#[test]
fn the_waiting_list_and_his_decisions() {
    let (fx, file) = with_proposals();
    let e = fx.editor();
    let waiting = |e: &EventEditor| -> Vec<(usize, String, String, String)> {
        e.proposals(&file)
            .unwrap()
            .waiting
            .into_iter()
            .map(|w| (w.line, w.why, w.from, w.into))
            .collect()
    };
    let s = |t: (usize, &str, &str, &str)| (t.0, t.1.to_string(), t.2.to_string(), t.3.to_string());
    // Before anything is applied: the two doubtful pairs, and the
    // confident pair the rule for model merges refuses.
    let before = fx.tree();
    let r = e.proposals(&file).unwrap();
    assert!(fx.tree() == before);
    assert_eq!(
        (r.lines, r.decided, r.already_one_event, r.no_record),
        (11, 0, 0, 0)
    );
    assert_eq!(
        waiting(&e),
        vec![
            s((3, "model-rule", GAMMA, DELTA)),
            s((7, "doubtful", ALPHA, DELTA)),
            s((11, "doubtful", LATE, GAMMA))
        ]
    );
    let w = &r.waiting[1];
    assert_eq!(
        (
            w.a.as_str(),
            w.b.as_str(),
            w.a_name.as_str(),
            w.b_name.as_str()
        ),
        (ALPHA, DELTA, "Alpha story", "Delta")
    );
    assert_eq!(
        (
            w.a_items,
            w.b_items,
            w.confidence.as_str(),
            w.reason.as_str()
        ),
        (3, 1, "doubtful", "r7: maybe")
    );
    assert!(r.waiting[0]
        .detail
        .as_deref()
        .unwrap()
        .contains("a model's merge does not merge away a record he edited"));
    e.apply_proposals(&file, true).unwrap();
    // Late is in alpha now: approving line 11 would merge gamma into alpha.
    assert_eq!(waiting(&e)[2], s((11, "doubtful", GAMMA, ALPHA)));
    // Dismiss, the pair given the other way round.
    let d = e
        .decide(&file, &format!(" {DELTA} "), ALPHA, Decision::Dismiss)
        .unwrap();
    assert_eq!(
        (d.a.as_str(), d.b.as_str(), d.decision, d.merge.is_none()),
        (ALPHA, DELTA, Decision::Dismiss, true)
    );
    assert_eq!(
        fx.log(DECISIONS_LOG),
        vec![json!({"at": NOW, "file": FILE, "a": ALPHA, "b": DELTA, "decision": "dismiss"})]
    );
    assert_eq!(fx.log(MERGE_LOG).len(), 3);
    assert_eq!(e.proposals(&file).unwrap().decided, 1);
    refused(&fx, "this pair is already decided (dismiss)", || {
        e.decide(&file, ALPHA, DELTA, Decision::Approve)
    });
    // Approve: his merge, with the proposal's reason, then the decision.
    let d = e.decide(&file, GAMMA, LATE, Decision::Approve).unwrap();
    let m = d.merge.unwrap();
    assert_eq!(
        (m.a.as_str(), m.b.as_str(), m.by, m.reason.as_deref()),
        (GAMMA, ALPHA, By::Dominik, Some("r11: perhaps"))
    );
    let log = fx.log(MERGE_LOG);
    assert_eq!(
        (log.len(), &log[3]["by"], &log[3]["id"]),
        (4, &json!("dominik"), &json!("m000004"))
    );
    assert_eq!(
        fx.log(DECISIONS_LOG)[1],
        json!({"at": NOW, "file": FILE, "a": GAMMA, "b": LATE, "decision": "approve"})
    );
    assert_eq!(fx.set().resolve("2026-11/E02"), Some(ALPHA));
    // Gamma is merged away, so line 3 is no longer the model rule's: it
    // is a confident pair (alpha into delta) the next apply makes.
    let r = e.proposals(&file).unwrap();
    assert_eq!((r.waiting.len(), r.decided), (0, 2));
    assert_eq!(
        pairs(&e.apply_proposals(&file, false).unwrap().merges),
        vec![(ALPHA, DELTA, 3)]
    );
    refused(
        &fx,
        "no proposal waiting for 2026-08-delta and 2026-11-gamma",
        || e.decide(&file, DELTA, GAMMA, Decision::Approve),
    );
    // A decision for another file's pair of the same ids does not count here.
    let other = fx.dir.join(PROPOSALS_DIR).join("2026-11-01-merge-2.jsonl");
    fs::write(
        &other,
        format!(
            "{}\n",
            line(DELTA, ALPHA, "same", "doubtful", "again", (1, 3))
        ),
    )
    .unwrap();
    assert_eq!(e.proposals(&other).unwrap().waiting.len(), 1);
    assert!(e.check().unwrap().ok);
}

#[test]
fn decide_refuses_what_is_not_waiting_and_an_approval_whose_merge_is_refused() {
    let (fx, file) = with_proposals();
    let e = fx.editor();
    let decide = |a: &str, b: &str, d: Decision| e.decide(&file, a, b, d);
    refused(
        &fx,
        "no proposal waiting for 2026-09-alpha and 2026-11-gamma",
        || decide(ALPHA, GAMMA, Decision::Dismiss),
    );
    // Judged different; judged confident and not refused; never judged.
    refused(&fx, "no proposal waiting", || {
        decide(GAMMA, TWIN_B, Decision::Approve)
    });
    refused(&fx, "no proposal waiting", || {
        decide(ALPHA, BETA, Decision::Approve)
    });
    refused(&fx, "no proposal waiting", || {
        decide(GAMMA, TWIN_A, Decision::Dismiss)
    });
    // The merge of an approval is refused: no decision is recorded.
    e.remove(DELTA).unwrap();
    refused(
        &fx,
        "2026-08-delta: a removed event; restore it first",
        || decide(ALPHA, DELTA, Decision::Approve),
    );
    e.restore(DELTA).unwrap();
    // Already in one event, by his own merge.
    e.merge(&merge_req(ALPHA, DELTA, By::Dominik)).unwrap();
    refused(&fx, "the two are already in one event", || {
        decide(ALPHA, DELTA, Decision::Dismiss)
    });
    assert_eq!(e.proposals(&file).unwrap().already_one_event, 1);
    // A record that is gone.
    fs::remove_file(fx.dir.join(format!("{LATE}.md"))).unwrap();
    refused(&fx, "2026-12-late: no such event record", || {
        decide(GAMMA, LATE, Decision::Dismiss)
    });
    assert_eq!(e.proposals(&file).unwrap().no_record, 1);
    // A malformed decisions log stops both commands before any write.
    fs::write(
        fx.dir.join(DECISIONS_LOG),
        "{\"at\":\"x\",\"file\":\"f\",\"a\":\"a\",\"b\":\"b\",\"decision\":\"maybe\"}\n",
    )
    .unwrap();
    refused(
        &fx,
        "merge-decisions.jsonl: line 1: not a decision line",
        || decide(DELTA, GAMMA, Decision::Approve),
    );
    refused(
        &fx,
        "merge-decisions.jsonl: line 1: not a decision line",
        || e.proposals(&file),
    );
    assert!(fx.log(MERGE_LOG).len() == 1);
}

#[cfg(unix)]
#[test]
fn a_proposals_file_is_read_only_from_the_proposals_folder() {
    use std::os::unix::fs::symlink;
    let (fx, file) = with_proposals();
    let e = fx.editor();
    let folder = fx.dir.join(PROPOSALS_DIR);
    let text = fs::read(&file).unwrap();
    let all = |p: &Path, why: &str| {
        refused(&fx, why, || e.apply_proposals(p, true));
        refused(&fx, why, || e.proposals(p));
        refused(&fx, why, || e.decide(p, ALPHA, DELTA, Decision::Dismiss));
    };
    // Elsewhere: beside the records, outside the archive.
    let outside = tempfile::tempdir().unwrap();
    for elsewhere in [
        fx.dir.join(FILE),
        outside.path().join(FILE),
        fx.root().join(FILE),
    ] {
        fs::write(&elsewhere, &text).unwrap();
        all(&elsewhere, "a proposals file is read only from there");
        fs::remove_file(&elsewhere).unwrap();
    }
    all(
        &folder.join("sub/..").join(FILE),
        "the proposals file path has a `..` component",
    );
    all(
        &folder.join("2026-10-11-merge-1-ledger.json"),
        "not a proposals file (a name ending .jsonl)",
    );
    all(&folder.join("no-such.jsonl"), "no such proposals file");
    // A link inside the folder is not followed, wherever it leads.
    fs::write(outside.path().join("real.jsonl"), &text).unwrap();
    symlink(outside.path().join("real.jsonl"), folder.join("link.jsonl")).unwrap();
    all(&folder.join("link.jsonl"), "link.jsonl: not a plain file");
    fs::remove_file(folder.join("link.jsonl")).unwrap();
    // A folder linked to the proposals folder is that folder: the file
    // read is the one inside the records folder.
    symlink(&folder, outside.path().join("alias")).unwrap();
    assert_eq!(
        e.proposals(&outside.path().join("alias").join(FILE))
            .unwrap()
            .waiting
            .len(),
        3
    );
    // A file still being written (the last line cut), a line that is not
    // an object, a judged line without its ids.
    for (tail, why) in [
        ("{\"a\":\"x\",\"b\":", "line 13: not JSON"),
        ("[1,2]\n", "line 13: not a JSON object"),
        (
            "{\"a\":1,\"b\":\"y\",\"verdict\":\"same\"}\n",
            "line 13: \"a\" is not text",
        ),
        (
            "{\"a\":\"x\",\"b\":\"y\",\"verdict\":7}\n",
            "line 13: \"verdict\" is not text",
        ),
    ] {
        let mut cut = text.clone();
        cut.extend_from_slice(tail.as_bytes());
        fs::write(folder.join("cut.jsonl"), cut).unwrap();
        all(&folder.join("cut.jsonl"), why);
    }
    fs::remove_file(folder.join("cut.jsonl")).unwrap();
    // The proposals folder itself a link: refused.
    let moved = outside.path().join("moved");
    fs::rename(&folder, &moved).unwrap();
    symlink(&moved, &folder).unwrap();
    all(&file, "no proposals folder (a real folder, not a link)");
}

#[test]
fn a_reason_with_a_control_character_is_skipped_not_cleaned() {
    let fx = fixture();
    let folder = fx.dir.join(PROPOSALS_DIR);
    fs::create_dir(&folder).unwrap();
    let file = folder.join(FILE);
    fs::write(
        &file,
        format!(
            "{}\n",
            line(ALPHA, BETA, "same", "confident", "two\nlines", (1, 1))
        ),
    )
    .unwrap();
    let before = fx.tree();
    let r = fx.editor().apply_proposals(&file, true).unwrap();
    assert_eq!((r.merged, skips(&r)), (0, vec![(1, "refused")]));
    assert!(r.skipped[0].detail.contains("control character"));
    assert!(fx.tree() == before);
}

#[test]
fn a_run_cut_short_keeps_its_whole_merges_and_a_second_run_makes_the_rest() {
    let (fx, file) = with_proposals();
    // The second merge "crashes" after its log line.
    let n = Arc::new(AtomicUsize::new(0));
    let crashing = fx.editor().with_hook(move |s| {
        if s == Step::Written(MERGE_LOG) && n.fetch_add(1, Ordering::SeqCst) == 1 {
            return Err(std::io::Error::other("power cut"));
        }
        Ok(())
    });
    let err = crashing
        .apply_proposals(&file, true)
        .unwrap_err()
        .to_string();
    assert!(err.contains("power cut"), "{err}");
    assert!(err.contains("line 2: the merge of 2026-12-late into 2026-09-alpha stopped the run; 1 of 3 merges were made before it and stay"), "{err}");
    let e = fx.editor();
    let c = e.check().unwrap();
    assert_eq!(
        (c.in_effect, c.problems.len(), c.problems[0].kind.as_str()),
        (1, 1, "not-applied")
    );
    // The plan says why line 2 cannot go on, and names the way out.
    let dry = e.apply_proposals(&file, false).unwrap();
    assert_eq!(pairs(&dry.merges), vec![(TWIN_A, TWIN_B, 4)]);
    let stuck = dry.skipped.iter().find(|s| s.line == 2).unwrap();
    assert_eq!(stuck.why, "refused");
    assert!(
        stuck
            .detail
            .contains("`scout events unmerge m000002` withdraws it"),
        "{stuck:?}"
    );
    assert!(e.unmerge("m000002").unwrap().withdrawn);
    let rest = e.apply_proposals(&file, true).unwrap();
    assert_eq!(
        pairs(&rest.merges),
        vec![(LATE, ALPHA, 2), (TWIN_A, TWIN_B, 4)]
    );
    assert!(e.check().unwrap().ok);
    assert_eq!(fx.set().resolve("2026-12/E09"), Some(ALPHA));
}

/// Three events that start the same day, so the item counts decide who
/// survives: a (5 items), b (4) and c (7). The file pairs a with b, then
/// b with c.
fn tied() -> (Fx, PathBuf) {
    let fx = fixture();
    for (id, member) in [
        ("2027-01-a", "2027-01/E01"),
        ("2027-01-b", "2027-01/E02"),
        ("2027-01-c", "2027-01/E03"),
    ] {
        put(
            &fx.dir,
            &rec(id, id, ("2027-01-05", "2027-01-06"), &[member]),
        );
    }
    let folder = fx.dir.join(PROPOSALS_DIR);
    fs::create_dir(&folder).unwrap();
    let file = folder.join(FILE);
    let lines = [
        line("2027-01-a", "2027-01-b", "same", "confident", "r1", (5, 4)),
        line("2027-01-b", "2027-01-c", "same", "confident", "r2", (4, 7)),
    ];
    let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
    fs::write(&file, text).unwrap();
    (fx, file)
}

#[test]
fn a_run_taken_up_again_picks_the_survivor_one_run_would() {
    // One run: b goes into a (5 over 4); the event a holds then has 9
    // items, so c (7) goes into a.
    let (fx, file) = tied();
    let whole = fx.editor().apply_proposals(&file, true).unwrap();
    let plan = vec![("2027-01-b", "2027-01-a", 1), ("2027-01-c", "2027-01-a", 2)];
    assert_eq!(pairs(&whole.merges), plan);
    let one_run = fx.set().aliases;

    // Two sessions: the first is cut short after its first merge. The
    // second finds b in a already, and counts a's event as 5 + 4, not as
    // a's own 5 (which c's 7 would beat).
    let (fx, file) = tied();
    let n = Arc::new(AtomicUsize::new(0));
    let crashing = fx.editor().with_hook(move |s| {
        if s == Step::Read && n.fetch_add(1, Ordering::SeqCst) == 1 {
            return Err(std::io::Error::other("power cut"));
        }
        Ok(())
    });
    let err = crashing
        .apply_proposals(&file, true)
        .unwrap_err()
        .to_string();
    assert!(err.contains("1 of 2 merges were made before it"), "{err}");
    assert_eq!(fx.set().resolve("2027-01-b"), Some("2027-01-a"));
    assert_eq!(fx.set().resolve("2027-01-c"), Some("2027-01-c"));
    let e = fx.editor();
    let dry = e.apply_proposals(&file, false).unwrap();
    assert_eq!(pairs(&dry.merges), vec![("2027-01-c", "2027-01-a", 2)]);
    assert_eq!(skips(&dry), vec![(1, "already-one-event")]);
    let rest = e.apply_proposals(&file, true).unwrap();
    assert_eq!(pairs(&rest.merges), pairs(&dry.merges));
    assert_eq!(fx.set().aliases, one_run);
    assert!(e.check().unwrap().ok);
}

#[test]
fn the_dry_run_skips_what_the_log_could_not_take_and_the_run_does_the_same() {
    // A reason that makes the merge's log line longer than a log line may
    // be: the merge itself would stop at its append. The dry run says so,
    // and the run skips the pair instead of stopping at it.
    let (fx, file) = with_proposals();
    let long = "x".repeat(4 << 20);
    let text = fs::read_to_string(&file).unwrap();
    assert!(text.contains("r2: its sequel"));
    fs::write(&file, text.replace("r2: its sequel", &long)).unwrap();
    let before = fx.tree();
    let e = fx.editor();
    let dry = e.apply_proposals(&file, false).unwrap();
    assert!(fx.tree() == before);
    assert_eq!(
        pairs(&dry.merges),
        vec![(BETA, ALPHA, 1), (TWIN_A, TWIN_B, 4)]
    );
    let stuck = dry.skipped.iter().find(|s| s.line == 2).unwrap();
    assert_eq!(stuck.why, "refused");
    assert!(
        stuck
            .detail
            .contains("over the limit of 4194304 for one line"),
        "{}",
        &stuck.detail[..200.min(stuck.detail.len())]
    );
    assert_eq!(dry.merges.len() + dry.skipped.len(), dry.same_confident);
    // The run: the same merges, the same skipped pairs, nothing stopped.
    let done = e.apply_proposals(&file, true).unwrap();
    assert_eq!(pairs(&done.merges), pairs(&dry.merges));
    assert_eq!(skips(&done), skips(&dry));
    assert_eq!(done.merged, 2);
    assert_eq!(fx.log(MERGE_LOG).len(), 2);
    assert_eq!(fx.set().resolve(LATE), Some(LATE));
    assert!(e.check().unwrap().ok);
}

#[test]
fn the_dry_run_refuses_a_merge_the_readers_id_table_would_refuse() {
    // A monthly id two records list: the reader's warning about it would
    // name another event once one of them is merged away, so `merge`
    // refuses the pair. The dry run must say the same, not plan it.
    let fx = fixture();
    // It sorts first, so it holds the id; it starts last, so it is the
    // one merged away.
    let mut twice = rec(
        "2026-01-twice",
        "Twice",
        ("2026-12-30", "2026-12-31"),
        &["2026-01/E01", "2026-11/E02"],
    );
    twice.themes = strs(&["T"]);
    put(&fx.dir, &twice);
    let folder = fx.dir.join(PROPOSALS_DIR);
    fs::create_dir(&folder).unwrap();
    let file = folder.join(FILE);
    let l = line(DELTA, "2026-01-twice", "same", "confident", "r", (1, 1));
    fs::write(&file, format!("{l}\n")).unwrap();
    let e = fx.editor();
    let direct = e
        .merge(&merge_req("2026-01-twice", DELTA, By::Model))
        .unwrap_err()
        .to_string();
    assert!(direct.contains("the reader would warn"), "{direct}");
    let before = fx.tree();
    for apply in [false, true] {
        let r = e.apply_proposals(&file, apply).unwrap();
        assert_eq!((r.merges.len(), skips(&r)), (0, vec![(1, "refused")]));
        assert!(r.skipped[0].detail.contains("the reader would warn"));
        assert!(fx.tree() == before);
    }
}

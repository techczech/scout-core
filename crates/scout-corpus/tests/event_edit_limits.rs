//! What the edits refuse of their logs and of the proposals file: a log
//! with a second hard link is never appended to (and the refusal comes
//! before any record changes), a line too long for the log, an id past
//! the last one, and item counts of a proposals line that are not whole
//! numbers in range. A refusal leaves every file as it was.

mod event_edit_support;
use event_edit_support::*;

const FILE: &str = "2026-10-11-merge-1.jsonl";
const LINE_LIMIT: usize = 4 << 20;

fn merge(fx: &Fx, a: &str, b: &str) -> String {
    fx.editor()
        .merge(&merge_req(a, b, By::Dominik))
        .unwrap()
        .log_id
}

fn proposals(fx: &Fx, lines: &[Value]) -> PathBuf {
    let folder = fx.dir.join(PROPOSALS_DIR);
    fs::create_dir_all(&folder).unwrap();
    let text: String = lines.iter().map(|l| format!("{l}\n")).collect();
    fs::write(folder.join(FILE), text).unwrap();
    folder.join(FILE)
}

fn pair(a: &str, b: &str, confidence: &str) -> Value {
    json!({"a": a, "b": b, "verdict": "same", "confidence": confidence, "reason": "one story"})
}

#[cfg(unix)]
#[test]
fn a_log_with_a_second_hard_link_is_never_appended_to() {
    let fx = fixture();
    let outside = tempfile::tempdir().unwrap();
    let victim = outside.path().join("victim.jsonl");
    let why = "merge-log.jsonl: has 2 hard links";
    // The merge log, linked from outside the records folder.
    let m = merge(&fx, GAMMA, DELTA);
    fs::hard_link(fx.dir.join(MERGE_LOG), &victim).unwrap();
    let was = fs::read(&victim).unwrap();
    let e = fx.editor();
    refused(&fx, why, || e.merge(&merge_req(BETA, ALPHA, By::Dominik)));
    // An undo appends last: it is refused before either record changes.
    refused(&fx, why, || e.unmerge(&m));
    assert_eq!(fs::read(&victim).unwrap(), was);
    // Reading only is not refused.
    assert!(e.check().unwrap().ok);
    // The link made after the log was read, before the first write.
    fs::remove_file(&victim).unwrap();
    let (log, v) = (fx.dir.join(MERGE_LOG), victim.clone());
    let racing = fx.editor().with_hook(move |s| match s {
        Step::Read => fs::hard_link(&log, &v),
        _ => Ok(()),
    });
    let before = fx.tree();
    let err = format!("{:#}", racing.unmerge(&m).unwrap_err());
    assert!(err.contains(why), "{err}");
    assert!(fx.tree() == before);
    assert_eq!(fs::read(&victim).unwrap(), was);
    fs::remove_file(&victim).unwrap();
    let racing = {
        let (log, v) = (fx.dir.join(MERGE_LOG), victim.clone());
        fx.editor().with_hook(move |s| match s {
            Step::Read => fs::hard_link(&log, &v),
            _ => Ok(()),
        })
    };
    let err = format!(
        "{:#}",
        racing
            .merge(&merge_req(BETA, ALPHA, By::Dominik))
            .unwrap_err()
    );
    assert!(err.contains(why), "{err}");
    assert!(fx.tree() == before);
    assert_eq!(fs::read(&victim).unwrap(), was);
    // Without the second name the same commands go through.
    fs::remove_file(&victim).unwrap();
    fx.editor().unmerge(&m).unwrap();

    // The decisions log: an approval is refused before its merge.
    let fx = fixture();
    let file = proposals(&fx, &[pair(ALPHA, BETA, "doubtful")]);
    let victim = outside.path().join("decisions.jsonl");
    fs::write(fx.dir.join(DECISIONS_LOG), "").unwrap();
    fs::hard_link(fx.dir.join(DECISIONS_LOG), &victim).unwrap();
    let e = fx.editor();
    let why = "merge-decisions.jsonl: has 2 hard links";
    refused(&fx, why, || e.decide(&file, ALPHA, BETA, Decision::Approve));
    refused(&fx, why, || e.decide(&file, ALPHA, BETA, Decision::Dismiss));
    assert_eq!(fs::read(&victim).unwrap(), b"");
    assert!(!fx.dir.join(MERGE_LOG).exists());
    assert_eq!(fx.record(BETA).merged_into, None);
    fs::remove_file(&victim).unwrap();
    e.decide(&file, ALPHA, BETA, Decision::Approve).unwrap();
    assert_eq!(fx.record(BETA).merged_into.as_deref(), Some(ALPHA));
    assert_eq!(fx.log(DECISIONS_LOG).len(), 1);
}

#[test]
fn a_line_over_four_mebibytes_is_not_written_to_a_log_and_stops_the_read_of_one() {
    let fx = fixture();
    let e = fx.editor();
    let long = "x".repeat(LINE_LIMIT);
    refused(&fx, "over the limit of 4194304 for one line", || {
        e.merge(&MergeRequest {
            reason: Some(long.clone()),
            ..merge_req(BETA, ALPHA, By::Dominik)
        })
    });
    assert!(!fx.dir.join(MERGE_LOG).exists());
    // Such a line in the log by other means: no command goes on.
    let m = merge(&fx, BETA, ALPHA);
    let good = fs::read_to_string(fx.dir.join(MERGE_LOG)).unwrap();
    fs::write(
        fx.dir.join(MERGE_LOG),
        good.replace("\"one story\"", &format!("\"{long}\"")),
    )
    .unwrap();
    let why = "merge-log.jsonl: line 1: longer than 4194304 bytes";
    refused(&fx, why, || e.unmerge(&m));
    refused(&fx, why, || e.merge(&merge_req(GAMMA, DELTA, By::Dominik)));
    refused(&fx, why, || e.check());
}

#[test]
fn an_id_past_the_last_is_malformed_and_the_last_id_stops_merge_and_unmerge() {
    let fx = fixture();
    merge(&fx, BETA, ALPHA);
    let good = fs::read_to_string(fx.dir.join(MERGE_LOG)).unwrap();
    let e = fx.editor();
    for id in [
        "m18446744073709551615",
        "m18446744073709551616",
        "m1000000000",
        "m0000000001",
    ] {
        fs::write(
            fx.dir.join(MERGE_LOG),
            good.replace("\"m000001\"", &format!("\"{id}\"")),
        )
        .unwrap();
        let why = format!("line 1: id {id:?} is not `m` and six to nine digits");
        refused(&fx, &why, || e.merge(&merge_req(GAMMA, DELTA, By::Dominik)));
        refused(&fx, &why, || e.unmerge(id));
        refused(&fx, &why, || e.check());
    }
    // The largest id is read; after it no line can be written.
    fs::write(
        fx.dir.join(MERGE_LOG),
        good.replace("\"m000001\"", "\"m999999999\""),
    )
    .unwrap();
    assert!(e.check().unwrap().ok);
    let why = "the last line has the largest id there is (m999999999)";
    refused(&fx, why, || e.merge(&merge_req(GAMMA, DELTA, By::Dominik)));
    refused(&fx, why, || e.unmerge("m999999999"));
    // One below it: the next line takes the last id.
    fs::write(
        fx.dir.join(MERGE_LOG),
        good.replace("\"m000001\"", "\"m999999998\""),
    )
    .unwrap();
    assert_eq!(e.unmerge("m999999998").unwrap().log_id, "m999999999");
}

#[test]
fn item_counts_that_are_not_whole_numbers_in_range_skip_the_line() {
    let fx = fixture();
    let with = |a: &str, b: &str, a_items: Value, b_items: Value| {
        let mut l = pair(a, b, "confident");
        l["a_items"] = a_items;
        l["b_items"] = b_items;
        l
    };
    let huge: Value = serde_json::from_str("18446744073709551615").unwrap();
    let file = proposals(
        &fx,
        &[
            // Two counts whose sum does not fit: skipped, not added.
            with(ALPHA, BETA, huge.clone(), huge.clone()),
            with(ALPHA, BETA, json!(10_000_001), json!(1)),
            with(ALPHA, BETA, json!(1), json!(-1)),
            with(ALPHA, BETA, json!(1.5), json!(1)),
            with(ALPHA, BETA, json!("7"), json!(1)),
            with(ALPHA, BETA, json!([1]), json!(1)),
            with(ALPHA, BETA, json!(true), json!(1)),
            // Read: the largest count, and a count left out or null.
            with(DELTA, ALPHA, json!(10_000_000), json!(null)),
            pair(GAMMA, BETA, "doubtful"),
        ],
    );
    let e = fx.editor();
    let before = fx.tree();
    let dry = e.apply_proposals(&file, false).unwrap();
    assert!(fx.tree() == before);
    let seen = serde_json::to_value(&dry).unwrap();
    assert_eq!(seen["malformed_lines"], json!(7));
    assert_eq!((dry.lines, dry.same_confident), (9, 1));
    assert_eq!(dry.merges.len(), 1);
    assert_eq!(
        (
            dry.merges[0].a.as_str(),
            dry.merges[0].b.as_str(),
            dry.merges[0].line
        ),
        (ALPHA, DELTA, 8)
    );
    assert!(dry.skipped.is_empty());
    let waiting = e.proposals(&file).unwrap();
    assert_eq!(
        serde_json::to_value(&waiting).unwrap()["malformed_lines"],
        json!(7)
    );
    assert_eq!(waiting.waiting.len(), 1);
    assert_eq!(
        (waiting.waiting[0].a_items, waiting.waiting[0].b_items),
        (0, 0)
    );
    // With --apply the skipped lines merge nothing.
    let done = e.apply_proposals(&file, true).unwrap();
    assert_eq!(done.merged, 1);
    assert_eq!(fx.record(BETA).merged_into, None);
    assert_eq!(fx.record(ALPHA).merged_into.as_deref(), Some(DELTA));
}

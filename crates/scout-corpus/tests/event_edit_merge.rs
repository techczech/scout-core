//! Seam tests for `scout events merge`, `unmerge` and `check`: a merge
//! writes `merged_into` on the record merged away and nothing else (the
//! surviving record's file is never written), the rule for model merges,
//! every crash window with what `check` says of it and how `unmerge`
//! recovers, a record edited meanwhile, and the log file's own rules. A
//! refusal leaves every file as it was. Undo in every order is in
//! `event_edit_undo.rs`; what the listing derives is in
//! `event_edit_listing.rs`.

mod event_edit_support;
use event_edit_support::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

fn by_hand(fx: &Fx, a: &str, b: &str) -> String {
    fx.editor()
        .merge(&merge_req(a, b, By::Dominik))
        .unwrap()
        .log_id
}

/// The archive's files without the merge log.
fn but_the_log(fx: &Fx) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
    let mut t = fx.tree();
    t.remove(&fx.dir.join(MERGE_LOG));
    t
}

/// An editor whose `n`-th matching step fails, once, as a crash would.
fn crashing(fx: &Fx, at: &'static str) -> EventEditor {
    let done = Arc::new(AtomicBool::new(false));
    fx.editor().with_hook(move |s| {
        if s == Step::Written(at) && !done.swap(true, Ordering::SeqCst) {
            return Err(std::io::Error::other("power cut"));
        }
        Ok(())
    })
}

fn kinds(fx: &Fx) -> Vec<String> {
    let c = fx.editor().check().unwrap();
    assert_eq!(c.ok, c.problems.is_empty());
    c.problems.into_iter().map(|p| p.kind).collect()
}

#[test]
fn merge_writes_merged_into_on_a_and_nothing_else() {
    let fx = fixture();
    let before = fx.tree();
    let (a, b_bytes) = (fx.record(BETA), fx.bytes(ALPHA));
    let body = body_bytes(&fx.dir, BETA);
    let m = fx
        .editor()
        .merge(&merge_req(&format!(" {BETA}"), ALPHA, By::Dominik))
        .unwrap();
    assert_eq!(
        (m.log_id.as_str(), m.a.as_str(), m.b.as_str(), m.at.as_str()),
        ("m000001", BETA, ALPHA, NOW)
    );
    // `a`: as it was, with `merged_into`; its body bytes carried over.
    let want = EventRecord {
        merged_into: Some(ALPHA.into()),
        ..a.clone()
    };
    assert_eq!((fx.record(BETA), &m.a_record), (want.clone(), &want));
    assert_eq!(fx.bytes(BETA), render_record(&want).into_bytes());
    assert_eq!(body_bytes(&fx.dir, BETA), body);
    assert_eq!(want.edited, None);
    // `b`: the file is the one that was there. Nothing of `a` is in it.
    assert_eq!(fx.bytes(ALPHA), b_bytes);
    assert_eq!(fx.record(ALPHA).members, strs(&["2026-09/E01"]));
    // Every other file is as it was; one file is new, the log.
    let mut after = fx.tree();
    assert!(after.remove(&fx.dir.join(MERGE_LOG)).is_some());
    after.insert(
        fx.dir.join(format!("{BETA}.md")),
        before[&fx.dir.join(format!("{BETA}.md"))].clone(),
    );
    assert!(after == before);
    // The reader resolves every id of `a` to `b`, through `a`.
    let set = fx.set();
    for id in [BETA, ALPHA, "2026-10/E04", "2026-09/E01"] {
        assert_eq!(set.resolve(id), Some(ALPHA), "{id}");
    }
    assert_eq!(set.resolve("2026-11/E02"), Some(GAMMA));
    // The line: who, when, which two records, and nothing of either.
    let log = fx.log(MERGE_LOG);
    assert_eq!(
        log,
        vec![json!({
            "id": "m000001", "at": NOW, "by": "dominik", "reason": "one story",
            "a": BETA, "b": ALPHA, "a_merged_into_before": null,
        })]
    );
    let c = fx.editor().check().unwrap();
    assert!(c.ok && c.information.is_empty());
    assert_eq!((c.records, c.merges, c.undone, c.in_effect), (4, 1, 0, 1));
}

#[test]
fn merge_refuses_and_changes_nothing() {
    let fx = fixture();
    let e = fx.editor();
    let merge = |a: &str, b: &str, by: By| e.merge(&merge_req(a, b, by));
    refused(&fx, "an event cannot be merged into itself", || {
        merge(ALPHA, " 2026-09-ALPHA ", By::Dominik)
    });
    refused(&fx, "2026-01-nobody: no such event record", || {
        merge("2026-01-nobody", ALPHA, By::Dominik)
    });
    refused(&fx, "2026-01-nobody: no such event record", || {
        merge(ALPHA, "2026-01-nobody", By::Dominik)
    });
    refused(&fx, "2026-09/E01: no such event record", || {
        merge("2026-09/E01", BETA, By::Dominik)
    });
    refused(&fx, "the reason holds a control character", || {
        e.merge(&MergeRequest {
            reason: Some("two\nlines".into()),
            ..merge_req(ALPHA, BETA, By::Dominik)
        })
    });
    // The rule for model merges: his record is not merged away.
    refused(
        &fx,
        "2026-11-gamma: edited 2026-10-10; a model's merge does not merge away a record he edited",
        || merge(GAMMA, ALPHA, By::Model),
    );
    assert!(fx.log(MERGE_LOG).is_empty() && !fx.dir.join(MERGE_LOG).exists());
    // Already merged away, as `a` and as `b`.
    by_hand(&fx, BETA, ALPHA);
    refused(
        &fx,
        "2026-10-beta: already merged into 2026-09-alpha",
        || merge(BETA, DELTA, By::Dominik),
    );
    refused(
        &fx,
        "2026-10-beta: already merged into 2026-09-alpha",
        || merge(DELTA, BETA, By::Dominik),
    );
    // Which closes off a cycle: `alpha` into `beta` needs a `beta` that
    // is not merged away.
    refused(
        &fx,
        "2026-10-beta: already merged into 2026-09-alpha",
        || merge(ALPHA, BETA, By::Dominik),
    );
    // A date that is not one: the reader does not read such a record, so
    // no edit goes on over the folder.
    let mut vague = rec(
        "2026-01-vague",
        "Vague",
        ("early 2026", "2026-01-09"),
        &["2026-01/E01"],
    );
    put(&fx.dir, &vague);
    refused(
        &fx,
        "2026-01-vague.md: cannot read the record (start \"early 2026\" is not a date YYYY-MM-DD); fix or remove it first",
        || merge("2026-01-vague", DELTA, By::Dominik),
    );
    refused(
        &fx,
        "is not a date YYYY-MM-DD); fix or remove it first",
        || merge(DELTA, "2026-01-vague", By::Dominik),
    );
    refused(&fx, "cannot read the record", || {
        merge(BETA, GAMMA, By::Dominik)
    });
    // A monthly id two records list: the reader's warning about it would
    // name another event after the merge, and the merge is refused.
    vague.start = Some("2026-01-01".into());
    vague.members = strs(&["2026-01/E01", "2026-11/E02"]);
    put(&fx.dir, &vague);
    assert!(!events::read_dir(&fx.dir)
        .unwrap()
        .unwrap()
        .warnings
        .is_empty());
    refused(&fx, "the reader would warn", || {
        merge("2026-01-vague", DELTA, By::Dominik)
    });
}

#[test]
fn a_model_does_not_merge_away_a_record_he_edited_and_may_merge_into_one() {
    let fx = fixture();
    let e = fx.editor();
    // Gamma has `edited` set.
    refused(
        &fx,
        "a model's merge does not merge away a record he edited",
        || e.merge(&merge_req(GAMMA, ALPHA, By::Model)),
    );
    // Into it: allowed, and his record's file is not written.
    let gamma = fx.bytes(GAMMA);
    let m = e.merge(&merge_req(BETA, GAMMA, By::Model)).unwrap();
    assert_eq!(fx.bytes(GAMMA), gamma);
    assert_eq!(fx.log(MERGE_LOG)[0]["by"], json!("model"));
    assert_eq!(fx.set().resolve("2026-10/E04"), Some(GAMMA));
    e.unmerge(&m.log_id).unwrap();
    assert_eq!(fx.bytes(GAMMA), gamma);
    // He may merge his own record away.
    e.merge(&merge_req(GAMMA, ALPHA, By::Dominik)).unwrap();
    assert_eq!(fx.record(GAMMA).merged_into.as_deref(), Some(ALPHA));
    assert_eq!(fx.record(GAMMA).edited.as_deref(), Some("2026-10-10"));
}

#[test]
fn a_merge_cut_short_is_reported_by_check_and_taken_back_by_unmerge() {
    // After the log line, before `a` is rewritten: no record has changed.
    let fx = fixture();
    let before = fx.tree();
    let err = crashing(&fx, MERGE_LOG)
        .merge(&merge_req(BETA, ALPHA, By::Dominik))
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("power cut") && err.contains("the merge m000001 is in"),
        "{err}"
    );
    assert!(
        err.contains("`scout events unmerge m000001` takes it back"),
        "{err}"
    );
    assert!(but_the_log(&fx) == before);
    assert_eq!(fx.log(MERGE_LOG).len(), 1);
    let c = fx.editor().check().unwrap();
    assert!(!c.ok);
    assert_eq!((c.in_effect, c.problems.len()), (0, 1));
    assert_eq!(
        (c.problems[0].kind.as_str(), c.problems[0].log_id.as_deref()),
        ("not-applied", Some("m000001"))
    );
    assert!(c.problems[0].fix.contains("scout events unmerge m000001"));
    let e = fx.editor();
    refused(&fx, "the merge m000001 of it into 2026-09-alpha was logged and never applied; run `scout events unmerge m000001`", || {
        e.merge(&merge_req(BETA, ALPHA, By::Dominik))
    });
    refused(&fx, "was logged and never applied", || {
        e.merge(&merge_req(BETA, DELTA, By::Dominik))
    });
    let u = e.unmerge("m000001").unwrap();
    assert!(u.withdrawn);
    assert!(but_the_log(&fx) == before);
    assert_eq!(kinds(&fx), Vec::<String>::new());
    assert_eq!(by_hand(&fx, BETA, ALPHA), "m000003");
    assert_eq!(kinds(&fx), Vec::<String>::new());

    // After `a` was rewritten: the merge is whole, whatever the caller
    // was told.
    let fx = fixture();
    let before = fx.tree();
    let file: &'static str = Box::leak(format!("{BETA}.md").into_boxed_str());
    assert!(crashing(&fx, file)
        .merge(&merge_req(BETA, ALPHA, By::Dominik))
        .is_err());
    assert_eq!(fx.set().resolve("2026-10/E04"), Some(ALPHA));
    let c = fx.editor().check().unwrap();
    assert!(c.ok);
    assert_eq!(c.in_effect, 1);
    assert!(!fx.editor().unmerge("m000001").unwrap().withdrawn);
    assert!(but_the_log(&fx) == before);
}

#[test]
fn an_undo_cut_short_is_reported_by_check_and_finished_by_running_it_again() {
    // After `a` was rewritten, before the undo line.
    let fx = fixture();
    let before = fx.tree();
    let id = by_hand(&fx, BETA, ALPHA);
    let file: &'static str = Box::leak(format!("{BETA}.md").into_boxed_str());
    let err = crashing(&fx, file).unmerge(&id).unwrap_err().to_string();
    assert!(
        err.contains("the undo of m000001 is not finished")
            && err.contains("run `scout events unmerge m000001` again"),
        "{err}"
    );
    // The records are already as before the merge; the reader warns of
    // nothing.
    assert!(but_the_log(&fx) == before);
    fx.set();
    assert_eq!(fx.log(MERGE_LOG).len(), 1);
    assert_eq!(kinds(&fx), vec!["not-applied".to_string()]);
    let u = fx.editor().unmerge(&id).unwrap();
    assert!(u.withdrawn);
    assert!(but_the_log(&fx) == before);
    assert_eq!(kinds(&fx), Vec::<String>::new());
    assert_eq!(fx.log(MERGE_LOG).len(), 2);

    // After the undo line: all done.
    let fx = fixture();
    let before = fx.tree();
    let id = by_hand(&fx, BETA, ALPHA);
    assert!(crashing(&fx, MERGE_LOG).unmerge(&id).is_err());
    assert_eq!(kinds(&fx), Vec::<String>::new());
    assert!(but_the_log(&fx) == before);
    assert_eq!(fx.log(MERGE_LOG).len(), 2);
}

#[test]
fn a_record_edited_while_a_merge_runs() {
    let his = |id: &str| format!("---\nid: \"{id}\"\nname: \"His edit, made meanwhile\"\n---\n");
    // `a`, before the first write: nothing changes, no log line is left.
    let fx = fixture();
    let (path, text) = (fx.dir.join(format!("{BETA}.md")), his(BETA));
    let (p, t) = (path.clone(), text.clone());
    let e = fx.editor().with_hook(move |s| match s {
        Step::Read => fs::write(&p, &t),
        _ => Ok(()),
    });
    let mut before = fx.tree();
    let err = e
        .merge(&merge_req(BETA, ALPHA, By::Dominik))
        .unwrap_err()
        .to_string();
    assert!(err.contains("changed since it was read"), "{err}");
    before.insert(path, text.into_bytes());
    assert!(fx.tree() == before);

    // `b`, at any point: the merge does not write it, so his edit stands
    // and the merge is made.
    let fx = fixture();
    let (path, text) = (fx.dir.join(format!("{ALPHA}.md")), his(ALPHA));
    let (p, t) = (path.clone(), text.clone());
    let e = fx.editor().with_hook(move |s| match s {
        Step::Read => fs::write(&p, &t),
        _ => Ok(()),
    });
    e.merge(&merge_req(BETA, ALPHA, By::Dominik)).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), text);
    assert_eq!(fx.record(BETA).merged_into.as_deref(), Some(ALPHA));
    assert_eq!(kinds(&fx), Vec::<String>::new());
    fx.editor().unmerge("m000001").unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), text);

    // `a`, after the log line and before its rename: the merge stops, his
    // edit stands, the line is not applied and is withdrawn.
    let fx = fixture();
    let (path, text) = (fx.dir.join(format!("{BETA}.md")), his(BETA));
    let (p, t) = (path.clone(), text.clone());
    let e = fx.editor().with_hook(move |s| match s {
        Step::Replacing(f) if f.starts_with(BETA) => fs::write(&p, &t),
        _ => Ok(()),
    });
    let err = e
        .merge(&merge_req(BETA, ALPHA, By::Dominik))
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("changed since it was read") && err.contains("the merge m000001 is in"),
        "{err}"
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), text);
    assert_eq!(kinds(&fx), vec!["not-applied".to_string()]);
    assert!(fs::read_dir(&fx.dir).unwrap().all(|e| !e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".scout-edit-")));
    assert!(fx.editor().unmerge("m000001").unwrap().withdrawn);
    assert_eq!(fs::read_to_string(&path).unwrap(), text);
    assert_eq!(kinds(&fx), Vec::<String>::new());
}

#[test]
fn a_malformed_merge_log_stops_every_command_before_any_write() {
    let fx = fixture();
    by_hand(&fx, BETA, ALPHA);
    let good = fs::read_to_string(fx.dir.join(MERGE_LOG)).unwrap();
    let undo =
        |id: &str, undoes: &str| format!("{}\n", json!({"id": id, "at": NOW, "undoes": undoes}));
    let line2 = good.replace("\"m000001\"", "\"m000002\"");
    // A line without one of its keys: a key whose value may be null must
    // still be there.
    let without = |line: &str, key: &str| {
        let mut v: Value = serde_json::from_str(line.trim_end()).unwrap();
        assert!(v.as_object_mut().unwrap().remove(key).is_some(), "{key}");
        format!("{v}\n")
    };
    for key in ["id", "at", "by", "reason", "a", "b", "a_merged_into_before"] {
        fs::write(
            fx.dir.join(MERGE_LOG),
            format!("{good}{}", without(&line2, key)),
        )
        .unwrap();
        let why = format!("line 2: not a merge line (missing field `{key}`)");
        refused(&fx, &why, || fx.editor().check());
        refused(&fx, &why, || {
            fx.editor().merge(&merge_req(GAMMA, DELTA, By::Dominik))
        });
    }
    for key in ["id", "at"] {
        let undo = undo("m000002", "m000001");
        fs::write(
            fx.dir.join(MERGE_LOG),
            format!("{good}{}", without(&undo, key)),
        )
        .unwrap();
        let why = format!("line 2: not an undo line (missing field `{key}`)");
        refused(&fx, &why, || fx.editor().unmerge("m000001"));
    }
    // A null where the key allows one is a line as written.
    let null_reason = good.replace("\"reason\":\"one story\"", "\"reason\":null");
    assert_ne!(null_reason, good);
    fs::write(fx.dir.join(MERGE_LOG), &null_reason).unwrap();
    assert!(fx.editor().check().unwrap().ok);
    fs::write(fx.dir.join(MERGE_LOG), &good).unwrap();
    for (text, why) in [
        (format!("{good}not json\n"), "line 2: not JSON"),
        (format!("{good}[1]\n"), "line 2: not a JSON object"),
        (format!("{good}\n"), "line 2: not JSON"),
        (
            good.trim_end().to_string(),
            "line 1: the last line is not ended by a newline",
        ),
        (
            format!("{good}{{\"id\":\"m000002\"}}\n"),
            "line 2: not a merge line",
        ),
        (
            format!(
                "{good}{}",
                good.replace("\"by\":\"dominik\"", "\"by\":\"someone\"")
                    .replace("m000001", "m000002")
            ),
            "line 2: not a merge line",
        ),
        (
            format!("{good}{}", line2.replace("\"a\":", "\"extra\":1,\"a\":")),
            "line 2: not a merge line",
        ),
        (
            format!("{good}{good}"),
            "line 2: id \"m000001\" is not `m` and its number in six digits or more with no zero too many, or is not greater",
        ),
        (
            good.replace("\"m000001\"", "\"m1\""),
            "line 1: id \"m1\" is not `m` and its number in six digits or more with no zero too many",
        ),
        // One spelling per number: a zero too many is not an id.
        (
            good.replace("\"m000001\"", "\"m0000001\""),
            "line 1: id \"m0000001\" is not `m` and its number in six digits or more with no zero too many",
        ),
        (
            good.replace("\"m000001\"", "\"m000000\""),
            "line 1: id \"m000000\" is not `m` and its number in six digits or more with no zero too many",
        ),
        (
            format!("{good}{}", undo("m000002", "m000007")),
            "line 2: undoes \"m000007\", which no earlier line is",
        ),
        (
            format!(
                "{good}{}{}",
                undo("m000002", "m000001"),
                undo("m000003", "m000001")
            ),
            "line 3: undoes \"m000001\" a second time",
        ),
        (
            format!(
                "{good}{}",
                undo("m000002", "m000001").replace("}\n", ",\"x\":1}\n")
            ),
            "line 2: not an undo line",
        ),
        (
            good.replace(
                "\"a_merged_into_before\":null",
                "\"a_merged_into_before\":\"2026-08-delta\"",
            ),
            "line 1: a_merged_into_before is \"2026-08-delta\", where a merge line has null",
        ),
        // A line that holds records, as a tool that copied values wrote.
        (
            good.replace("\"a\":", "\"added_members\":[],\"a\":"),
            "line 1: not a merge line",
        ),
        ("\u{0}\n".to_string(), "line 1: not JSON"),
    ] {
        fs::write(fx.dir.join(MERGE_LOG), &text).unwrap();
        let e = fx.editor();
        refused(&fx, why, || e.merge(&merge_req(GAMMA, DELTA, By::Dominik)));
        refused(&fx, why, || e.unmerge("m000001"));
        refused(&fx, why, || e.check());
    }
    fs::write(fx.dir.join(MERGE_LOG), [0xff, 0xfe, b'\n']).unwrap();
    refused(&fx, "not valid UTF-8", || fx.editor().check());
    // The decisions log is checked by `check` too.
    fs::write(fx.dir.join(MERGE_LOG), &good).unwrap();
    fs::write(fx.dir.join(DECISIONS_LOG), "{\"at\":1}\n").unwrap();
    refused(
        &fx,
        "merge-decisions.jsonl: line 1: not a decision line",
        || fx.editor().check(),
    );
}

#[cfg(unix)]
#[test]
fn a_log_that_is_not_a_plain_file_is_never_written() {
    use std::os::unix::fs::symlink;
    let fx = fixture();
    let outside = tempfile::tempdir().unwrap();
    let victim = outside.path().join("victim.jsonl");
    let e = fx.editor();
    let why = "merge-log.jsonl: not a plain file";
    // A link to a file, a dangling link, a folder.
    fs::write(&victim, "").unwrap();
    symlink(&victim, fx.dir.join(MERGE_LOG)).unwrap();
    refused(&fx, why, || e.merge(&merge_req(BETA, ALPHA, By::Dominik)));
    assert_eq!(fs::read(&victim).unwrap(), b"");
    fs::remove_file(fx.dir.join(MERGE_LOG)).unwrap();
    symlink(
        outside.path().join("not-there.jsonl"),
        fx.dir.join(MERGE_LOG),
    )
    .unwrap();
    refused(&fx, why, || e.merge(&merge_req(BETA, ALPHA, By::Dominik)));
    refused(&fx, why, || e.check());
    assert!(!outside.path().join("not-there.jsonl").exists());
    fs::remove_file(fx.dir.join(MERGE_LOG)).unwrap();
    fs::create_dir(fx.dir.join(MERGE_LOG)).unwrap();
    refused(&fx, why, || e.merge(&merge_req(BETA, ALPHA, By::Dominik)));
    fs::remove_dir(fx.dir.join(MERGE_LOG)).unwrap();
    // The log put in place as a link after it was read: the append does
    // not follow it, and no record has changed.
    let (log, v) = (fx.dir.join(MERGE_LOG), victim.clone());
    let racing = fx.editor().with_hook(move |s| match s {
        Step::Read => symlink(&v, &log),
        _ => Ok(()),
    });
    let before = fx.tree();
    let err = format!(
        "{:#}",
        racing
            .merge(&merge_req(BETA, ALPHA, By::Dominik))
            .unwrap_err()
    );
    assert!(err.contains("merge-log.jsonl to append"), "{err}");
    assert_eq!(fs::read(&victim).unwrap(), b"");
    let mut after = fx.tree();
    after.remove(&fx.dir.join(MERGE_LOG));
    assert!(after == before);
}

#[test]
fn a_log_another_tool_appended_to_meanwhile_is_not_appended_to() {
    let fx = fixture();
    by_hand(&fx, GAMMA, DELTA);
    let log = fx.dir.join(MERGE_LOG);
    let l = log.clone();
    let racing = fx.editor().with_hook(move |s| match s {
        Step::Read => {
            let mut text = fs::read_to_string(&l)?;
            text.push_str("{\"theirs\":true}\n");
            fs::write(&l, text)
        }
        _ => Ok(()),
    });
    let records = but_the_log(&fx);
    let err = racing
        .merge(&merge_req(BETA, ALPHA, By::Dominik))
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("merge-log.jsonl: changed since it was read"),
        "{err}"
    );
    assert!(but_the_log(&fx) == records);
    assert!(fs::read_to_string(&log)
        .unwrap()
        .ends_with("{\"theirs\":true}\n"));
}

#[test]
fn check_reports_lines_that_are_not_in_effect_hand_merges_and_temporary_files() {
    // `merged_into` set by hand: information, not a fault.
    let fx = fixture();
    let mut beta = fx.record(BETA);
    beta.merged_into = Some(ALPHA.into());
    put(&fx.dir, &beta);
    let before = fx.tree();
    let c = fx.editor().check().unwrap();
    assert!(c.ok && c.problems.is_empty());
    let info: Vec<(&str, Option<&str>, Option<&str>)> = c
        .information
        .iter()
        .map(|p| (p.kind.as_str(), p.a.as_deref(), p.b.as_deref()))
        .collect();
    assert_eq!(info, vec![("merged-without-log", Some(BETA), Some(ALPHA))]);
    // A temporary file an edit left: a problem.
    fs::write(fx.dir.join(".scout-edit-123-abc-0.tmp"), "half").unwrap();
    let c = fx.editor().check().unwrap();
    assert!(!c.ok);
    assert_eq!(c.information.len(), 1);
    assert_eq!(kinds(&fx), vec!["temporary-file".to_string()]);
    // Check reads only.
    fs::remove_file(fx.dir.join(".scout-edit-123-abc-0.tmp")).unwrap();
    assert!(fx.tree() == before);

    // A logged merge whose `a` was then merged elsewhere by hand.
    let fx = fixture();
    let id = by_hand(&fx, BETA, ALPHA);
    let mut beta = fx.record(BETA);
    beta.merged_into = Some(DELTA.into());
    put(&fx.dir, &beta);
    let c = fx.editor().check().unwrap();
    assert!(!c.ok);
    assert_eq!(
        (c.problems[0].kind.as_str(), c.problems[0].log_id.as_deref()),
        ("not-as-logged", Some(id.as_str()))
    );
    assert_eq!(c.information[0].b.as_deref(), Some(DELTA));
    refused(
        &fx,
        "2026-10-beta is no longer merged into 2026-09-alpha (2026-10-beta is now merged into 2026-08-delta)",
        || fx.editor().unmerge(&id),
    );
    // Unmerged by hand: the line is not applied, and is withdrawn.
    beta.merged_into = None;
    put(&fx.dir, &beta);
    assert_eq!(kinds(&fx), vec!["not-applied".to_string()]);
    assert!(fx.editor().unmerge(&id).unwrap().withdrawn);
    let c = fx.editor().check().unwrap();
    assert!(c.ok && c.information.is_empty());
    assert_eq!((c.merges, c.undone, c.in_effect), (1, 1, 0));
}

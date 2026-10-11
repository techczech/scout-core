//! Seam tests for `scout events merge`, `unmerge` and `check`: what a
//! merge changes and logs, the rule for model merges, undo (after later
//! edits, in nested order), every crash window with what `check` says of
//! it and how `unmerge` recovers, a record edited meanwhile, and the
//! log file's own rules. A refusal leaves every file as it was.

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
fn merge_folds_a_into_b_and_logs_both_records_whole() {
    let fx = fixture();
    let before = fx.tree();
    let (a, b) = (fx.record(BETA), fx.record(ALPHA));
    let bodies = (body_bytes(&fx.dir, BETA), body_bytes(&fx.dir, ALPHA));
    let m = fx
        .editor()
        .merge(&merge_req(&format!(" {BETA}"), ALPHA, By::Dominik))
        .unwrap();
    assert_eq!(
        (m.log_id.as_str(), m.a.as_str(), m.b.as_str(), m.at.as_str()),
        ("m000001", BETA, ALPHA, NOW)
    );
    assert_eq!(m.members_added, strs(&["2026-10/E04"]));
    let (na, nb) = (fx.record(BETA), fx.record(ALPHA));
    assert_eq!(nb, m.record);
    // `a`: merged, otherwise as it was. `b`: the members, the names (none
    // it has under the fold, none that is its own name), the wider end.
    assert_eq!(
        na,
        EventRecord {
            merged_into: Some(ALPHA.into()),
            ..a.clone()
        }
    );
    assert_eq!(
        nb,
        EventRecord {
            members: strs(&["2026-09/E01", "2026-10/E04"]),
            aliases: strs(&["The Alpha", "Shared", "Beta follow-up", "Beta"]),
            end: Some("2026-10-09".into()),
            ..b.clone()
        }
    );
    assert_eq!((&nb.edited, &na.edited), (&None, &None));
    assert_eq!(
        (body_bytes(&fx.dir, BETA), body_bytes(&fx.dir, ALPHA)),
        bodies
    );
    let set = fx.set();
    for id in [BETA, ALPHA, "2026-10/E04", "2026-09/e01"] {
        assert_eq!(set.resolve(id), Some(ALPHA), "{id}");
    }
    assert_eq!(set.resolve("2026-11/E02"), Some(GAMMA));
    // The log line.
    let log = fx.log(MERGE_LOG);
    assert_eq!(log.len(), 1);
    assert_eq!(
        log[0],
        json!({
            "id": "m000001", "at": NOW, "by": "dominik", "reason": "one story",
            "a": BETA, "b": ALPHA,
            "before": {"a": a, "b": b}, "after": {"a": na, "b": nb},
            "moved": ["2026-10-beta", "2026-10/e04"],
        })
    );
    assert!(!log[0]["before"]["a"]
        .as_object()
        .unwrap()
        .contains_key("removed"));
    // Three files changed: the two records and the log.
    let after = fx.tree();
    let changed: Vec<&PathBuf> = after
        .keys()
        .filter(|k| before.get(*k) != after.get(*k))
        .collect();
    assert_eq!(changed.len(), 3, "{changed:?}");
    assert_eq!(kinds(&fx), Vec::<String>::new());
    // The other way about: an earlier `a` moves `b.start` back.
    let fx = fixture();
    by_hand(&fx, DELTA, BETA);
    let b = fx.record(BETA);
    assert_eq!(
        (b.start.as_deref(), b.end.as_deref()),
        (Some("2026-08-01"), Some("2026-10-09"))
    );
    assert_eq!(b.aliases, strs(&["Beta", "shared", "ALPHA STORY", "Delta"]));
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
    // Dates the merge cannot compare.
    let mut vague = rec(
        "2026-01-vague",
        "Vague",
        ("early 2026", "2026-01-09"),
        &["2026-01/E01"],
    );
    put(&fx.dir, &vague);
    refused(
        &fx,
        "2026-01-vague: start \"early 2026\" is not a date YYYY-MM-DD; set it first",
        || merge("2026-01-vague", DELTA, By::Dominik),
    );
    refused(&fx, "is not a date YYYY-MM-DD; set it first", || {
        merge(DELTA, "2026-01-vague", By::Dominik)
    });
    // A member two events would hold: the reader's own table says no.
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
fn a_models_merge_into_a_record_he_edited_appends_members_and_extends_end_only() {
    let fx = fixture();
    let late = rec(
        "2026-12-late",
        "Late",
        ("2026-10-20", "2026-12-05"),
        &["2026-12/E09"],
    );
    put(&fx.dir, &late);
    let was = fx.record(GAMMA);
    let e = fx.editor();
    e.merge(&merge_req("2026-12-late", GAMMA, By::Model))
        .unwrap();
    assert_eq!(
        fx.record(GAMMA),
        EventRecord {
            members: strs(&["2026-11/E02", "2026-12/E09"]),
            end: Some("2026-12-05".into()),
            ..was.clone()
        }
    );
    assert_eq!(fx.log(MERGE_LOG)[0]["by"], "model");
    // An earlier `a` does not move his start either.
    e.merge(&merge_req(DELTA, GAMMA, By::Model)).unwrap();
    let g = fx.record(GAMMA);
    assert_eq!(
        (g.start, g.aliases, g.edited, g.name),
        (
            was.start.clone(),
            was.aliases.clone(),
            was.edited.clone(),
            was.name.clone()
        )
    );
    assert_eq!(
        g.members,
        strs(&["2026-11/E02", "2026-12/E09", "2026-08/E07"])
    );
    assert_eq!(
        body_bytes(&fx.dir, GAMMA),
        format!("\n{GAMMA_BODY}\n").into_bytes()
    );
    // His own merge into it does change its names and start, not `edited`.
    e.merge(&merge_req(ALPHA, GAMMA, By::Dominik)).unwrap();
    let g = fx.record(GAMMA);
    assert_eq!(
        g.aliases,
        strs(&["His alias", "Alpha story", "The Alpha", "Shared"])
    );
    assert_eq!(
        (g.start.as_deref(), g.edited.as_deref()),
        (Some("2026-09-05"), Some("2026-10-10"))
    );
    // A model's merge into a record nobody edited is a full merge.
    e.merge(&merge_req(GAMMA, BETA, By::Dominik)).unwrap();
    let fx = fixture();
    fx.editor()
        .merge(&merge_req(DELTA, ALPHA, By::Model))
        .unwrap();
    let a = fx.record(ALPHA);
    assert_eq!(
        (a.start.as_deref(), a.aliases),
        (Some("2026-08-01"), strs(&["The Alpha", "Shared", "Delta"]))
    );
}

#[test]
fn unmerge_gives_back_both_records_byte_for_byte() {
    let fx = fixture();
    let before = fx.tree();
    let e = fx.editor();
    let id = by_hand(&fx, BETA, ALPHA);
    let u = e.unmerge(&format!(" {id} ")).unwrap();
    assert_eq!(
        (u.log_id.as_str(), u.undoes.as_str(), u.withdrawn),
        ("m000002", "m000001", false)
    );
    assert_eq!((u.a.as_str(), u.b.as_str()), (BETA, ALPHA));
    assert_eq!(
        (&u.a_record, &u.b_record),
        (&fx.record(BETA), &fx.record(ALPHA))
    );
    assert!(but_the_log(&fx) == before);
    let log = fx.log(MERGE_LOG);
    assert_eq!(log.len(), 2);
    assert_eq!(
        log[1],
        json!({"id": "m000002", "at": NOW, "undoes": "m000001"})
    );
    assert_eq!(kinds(&fx), Vec::<String>::new());
    refused(&fx, "m000001: already undone, by m000002", || {
        e.unmerge("m000001")
    });
    refused(&fx, "m000002: no such merge", || e.unmerge("m000002"));
    refused(&fx, "m000009: no such merge", || e.unmerge("m000009"));
    // The pair can be merged again, under the next id, and undone again.
    assert_eq!(by_hand(&fx, BETA, ALPHA), "m000003");
    assert_eq!(e.unmerge("m000003").unwrap().log_id, "m000004");
    assert!(but_the_log(&fx) == before);
    let c = e.check().unwrap();
    assert_eq!(
        (c.ok, c.merges, c.undone, c.in_effect, c.records),
        (true, 2, 2, 0, 4)
    );
}

#[test]
fn unmerge_keeps_what_he_changed_since_the_merge() {
    let fx = fixture();
    let e = fx.editor();
    // delta into beta: beta gains a member, the alias "Delta", and its
    // start moves back to 2026-08-01.
    let id = by_hand(&fx, DELTA, BETA);
    e.set(
        BETA,
        &FieldEdit {
            name: Some("His name".into()),
            end: Some("2026-12-31".into()),
            add_aliases: strs(&["Mine"]),
            remove_aliases: strs(&["Beta"]),
            description: Some("His words.".into()),
            ..Default::default()
        },
    )
    .unwrap();
    e.unmerge(&id).unwrap();
    let b = fx.record(BETA);
    // Gone: what the merge gave (member, alias, start; the start is still
    // the merge's, so it goes back). Kept: his name, his alias, his
    // removal, his end, his words.
    assert_eq!(b.members, strs(&["2026-10/E04"]));
    assert_eq!(b.aliases, strs(&["shared", "ALPHA STORY", "Mine"]));
    assert_eq!(
        (b.start.as_deref(), b.end.as_deref()),
        (Some("2026-10-02"), Some("2026-12-31"))
    );
    assert_eq!(
        (b.name.as_str(), b.description.as_str(), b.edited.as_deref()),
        ("His name", "His words.", Some(TODAY))
    );
    assert_eq!(fx.record(DELTA).merged_into, None);
    assert_eq!(fx.set().resolve("2026-08/E07"), Some(DELTA));
    // A start he set since is his: it stays.
    let fx = fixture();
    let e = fx.editor();
    let id = by_hand(&fx, DELTA, BETA);
    e.set(
        BETA,
        &FieldEdit {
            start: Some("2026-07-15".into()),
            ..Default::default()
        },
    )
    .unwrap();
    e.unmerge(&id).unwrap();
    assert_eq!(fx.record(BETA).start.as_deref(), Some("2026-07-15"));
}

#[test]
fn nested_merges_are_undone_latest_first() {
    let fx = fixture();
    let before = fx.tree();
    let e = fx.editor();
    let m1 = by_hand(&fx, BETA, ALPHA);
    let m2 = by_hand(&fx, ALPHA, DELTA);
    let m3 = by_hand(&fx, GAMMA, DELTA);
    let set = fx.set();
    for id in [
        BETA,
        ALPHA,
        GAMMA,
        "2026-10/E04",
        "2026-09/E01",
        "2026-11/E02",
    ] {
        assert_eq!(set.resolve(id), Some(DELTA), "{id}");
    }
    assert_eq!(fx.record(BETA).merged_into.as_deref(), Some(ALPHA));
    assert_eq!(
        fx.record(DELTA).members,
        strs(&["2026-08/E07", "2026-09/E01", "2026-10/E04", "2026-11/E02"])
    );
    // The first merge's survivor is merged away: undo that merge first.
    refused(
        &fx,
        "m000001: 2026-09-alpha has since been merged into 2026-08-delta; undo that later merge first (`scout events unmerge m000002`)",
        || e.unmerge(&m1),
    );
    // Two merges into one survivor are independent: either order.
    e.unmerge(&m2).unwrap();
    let set = fx.set();
    assert_eq!(set.resolve("2026-10/E04"), Some(ALPHA));
    assert_eq!(set.resolve("2026-11/E02"), Some(DELTA));
    assert_eq!(
        fx.record(DELTA).members,
        strs(&["2026-08/E07", "2026-11/E02"])
    );
    e.unmerge(&m1).unwrap();
    e.unmerge(&m3).unwrap();
    assert!(but_the_log(&fx) == before);
    assert_eq!(fx.log(MERGE_LOG).len(), 6);
    assert_eq!(kinds(&fx), Vec::<String>::new());
}

#[test]
fn a_merge_cut_short_is_reported_by_check_and_taken_back_by_unmerge() {
    // After the log line: no record has changed.
    let fx = fixture();
    let before = fx.tree();
    let err = crashing(&fx, MERGE_LOG)
        .merge(&merge_req(BETA, ALPHA, By::Dominik))
        .unwrap_err();
    assert!(err.to_string().contains("power cut"), "{err}");
    assert!(but_the_log(&fx) == before);
    assert_eq!(fx.log(MERGE_LOG).len(), 1);
    let c = fx.editor().check().unwrap();
    assert!(!c.ok);
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

    // After `a`: the id table is already the merged one, read without a
    // warning; `b` has not been widened yet.
    let fx = fixture();
    let before = fx.tree();
    let file: &'static str = Box::leak(format!("{BETA}.md").into_boxed_str());
    let err = crashing(&fx, file)
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
    let set = fx.set();
    assert_eq!(set.resolve("2026-10/E04"), Some(ALPHA));
    assert_eq!(fx.bytes(ALPHA), before[&fx.dir.join(format!("{ALPHA}.md"))]);
    let c = fx.editor().check().unwrap();
    assert_eq!(c.problems.len(), 1);
    assert_eq!(c.problems[0].kind, "half-applied");
    assert!(c.problems[0]
        .detail
        .contains("lacks the members the merge gives it: 2026-10/E04"));
    let e = fx.editor();
    refused(
        &fx,
        "2026-10-beta: already merged into 2026-09-alpha",
        || e.merge(&merge_req(BETA, ALPHA, By::Dominik)),
    );
    assert!(!e.unmerge("m000001").unwrap().withdrawn);
    assert!(but_the_log(&fx) == before);
    assert_eq!(kinds(&fx), Vec::<String>::new());
    assert_eq!(by_hand(&fx, BETA, ALPHA), "m000003");
}

#[test]
fn an_undo_cut_short_is_reported_by_check_and_finished_by_running_it_again() {
    for (file, kind, withdrawn) in [
        (format!("{ALPHA}.md"), "half-applied", false),
        (format!("{BETA}.md"), "not-applied", true),
        (MERGE_LOG.to_string(), "", false),
    ] {
        let fx = fixture();
        let before = fx.tree();
        let id = by_hand(&fx, BETA, ALPHA);
        let file: &'static str = Box::leak(file.into_boxed_str());
        let res = crashing(&fx, file).unmerge(&id);
        if file == MERGE_LOG {
            // The undo line was written before the "crash": all done.
            assert!(res.is_err());
            assert_eq!(kinds(&fx), Vec::<String>::new());
            assert!(but_the_log(&fx) == before);
            continue;
        }
        let err = res.unwrap_err().to_string();
        assert!(
            err.contains(
                "the undo of m000001 is not finished; run `scout events unmerge m000001` again"
            ),
            "{err}"
        );
        // A state the reader reads without a warning.
        fx.set();
        assert_eq!(kinds(&fx), vec![kind.to_string()], "{file}");
        let u = fx.editor().unmerge(&id).unwrap();
        assert_eq!(u.withdrawn, withdrawn, "{file}");
        assert!(but_the_log(&fx) == before, "{file}");
        assert_eq!(kinds(&fx), Vec::<String>::new());
        assert_eq!(fx.log(MERGE_LOG).len(), 2);
    }
}

#[test]
fn a_record_edited_while_a_merge_runs_stops_it() {
    let his = |id: &str| format!("---\nid: \"{id}\"\nname: \"His edit, made meanwhile\"\n---\n");
    // Before the first write: nothing changes, and no log line is left.
    for target in [ALPHA, BETA] {
        let fx = fixture();
        let (path, text) = (fx.dir.join(format!("{target}.md")), his(target));
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
        assert!(fx.tree() == before, "{target}");
    }
    // The second record, after the first was replaced: the merge stops in
    // the half-applied state, says so, and his edit stands.
    let fx = fixture();
    let (path, text) = (fx.dir.join(format!("{ALPHA}.md")), his(ALPHA));
    let (p, t) = (path.clone(), text.clone());
    let e = fx.editor().with_hook(move |s| match s {
        Step::Replacing(f) if f.starts_with(ALPHA) => fs::write(&p, &t),
        _ => Ok(()),
    });
    let err = e
        .merge(&merge_req(BETA, ALPHA, By::Dominik))
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("changed since it was read") && err.contains("is not finished"),
        "{err}"
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), text);
    assert_eq!(fx.record(BETA).merged_into.as_deref(), Some(ALPHA));
    assert_eq!(kinds(&fx), vec!["half-applied".to_string()]);
    assert!(fs::read_dir(&fx.dir).unwrap().all(|e| !e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".scout-edit-")));
    // His record is no longer as the writer writes it, so the undo leaves
    // it alone and says why; nothing is lost.
    refused(&fx, "the frontmatter is not as scout writes it", || {
        fx.editor().unmerge("m000001")
    });
}

#[test]
fn a_malformed_merge_log_stops_every_command_before_any_write() {
    let fx = fixture();
    by_hand(&fx, BETA, ALPHA);
    let good = fs::read_to_string(fx.dir.join(MERGE_LOG)).unwrap();
    let undo =
        |id: &str, undoes: &str| format!("{}\n", json!({"id": id, "at": NOW, "undoes": undoes}));
    let line2 = good.replace("\"m000001\"", "\"m000002\"");
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
            format!(
                "{good}{}",
                line2.replace("\"moved\"", "\"extra\":1,\"moved\"")
            ),
            "line 2: not a merge line",
        ),
        (
            format!("{good}{good}"),
            "line 2: id \"m000001\" is not `m` and six digits, or is not greater",
        ),
        (
            good.replace("\"m000001\"", "\"m1\""),
            "line 1: id \"m1\" is not `m` and six digits",
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
fn check_reports_a_merge_no_log_line_explains_and_a_leftover_temporary_file() {
    let fx = fixture();
    let mut beta = fx.record(BETA);
    beta.merged_into = Some(ALPHA.into());
    put(&fx.dir, &beta);
    fs::write(fx.dir.join(".scout-edit-123-abc-0.tmp"), "half").unwrap();
    let before = fx.tree();
    let c = fx.editor().check().unwrap();
    assert!(!c.ok);
    let got: Vec<(&str, Option<&str>, Option<&str>)> = c
        .problems
        .iter()
        .map(|p| (p.kind.as_str(), p.a.as_deref(), p.b.as_deref()))
        .collect();
    assert_eq!(
        got,
        vec![
            ("merged-without-log", Some(BETA), Some(ALPHA)),
            ("temporary-file", None, None)
        ]
    );
    // Check reads only.
    assert!(fx.tree() == before);
    // A logged merge whose records were then changed by hand.
    let fx = fixture();
    let id = by_hand(&fx, BETA, ALPHA);
    let mut beta = fx.record(BETA);
    beta.merged_into = None;
    put(&fx.dir, &beta);
    let c = fx.editor().check().unwrap();
    assert_eq!(c.problems.len(), 1);
    assert_eq!(
        (c.problems[0].kind.as_str(), c.problems[0].log_id.as_deref()),
        ("not-as-logged", Some(id.as_str()))
    );
    refused(
        &fx,
        "2026-10-beta is no longer merged into 2026-09-alpha",
        || fx.editor().unmerge(&id),
    );
}

#[test]
fn undoing_one_of_several_merges_into_an_event_leaves_dates_that_cover_the_rest() {
    let fx = fixture();
    let mut late = rec(
        "2026-12-late",
        "Late",
        ("2026-12-02", "2026-12-05"),
        &["2026-12/E09"],
    );
    late.aliases = strs(&["Beta"]);
    put(&fx.dir, &late);
    let before = fx.tree();
    let e = fx.editor();
    // Late widens alpha's end to December; beta, merged after it, changes
    // no date and adds only the names late did not bring.
    let m1 = by_hand(&fx, "2026-12-late", ALPHA);
    let m2 = by_hand(&fx, BETA, ALPHA);
    let a = fx.record(ALPHA);
    assert_eq!(a.end.as_deref(), Some("2026-12-05"));
    assert_eq!(
        a.aliases,
        strs(&["The Alpha", "Shared", "Late", "Beta", "Beta follow-up"])
    );
    // Undo the first: the end goes back as far as beta allows, and the
    // alias beta also brings stays.
    e.unmerge(&m1).unwrap();
    let a = fx.record(ALPHA);
    assert_eq!(
        (a.start.as_deref(), a.end.as_deref()),
        (Some("2026-09-05"), Some("2026-10-09"))
    );
    assert_eq!(
        a.aliases,
        strs(&["The Alpha", "Shared", "Beta", "Beta follow-up"])
    );
    assert_eq!(a.members, strs(&["2026-09/E01", "2026-10/E04"]));
    assert_eq!(kinds(&fx), Vec::<String>::new());
    e.unmerge(&m2).unwrap();
    assert!(but_the_log(&fx) == before);
}

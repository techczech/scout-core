//! The undo rule of `scout events unmerge`: it clears `merged_into` on the
//! record merged away and writes nothing else. So a merge and its undo
//! leave that record's file byte for byte as it was, the surviving
//! record's file is never written by either, and merges into one record
//! are undone in any order, whatever was merged or edited in between.

mod event_edit_support;
use event_edit_support::*;
use std::collections::BTreeMap;

fn merge(fx: &Fx, a: &str, b: &str) -> String {
    fx.editor()
        .merge(&merge_req(a, b, By::Dominik))
        .unwrap()
        .log_id
}

fn unmerge(fx: &Fx, id: &str) {
    let u = fx.editor().unmerge(id).unwrap();
    assert!(!u.withdrawn);
    assert_eq!(u.a_record.merged_into, None);
    assert_eq!(fx.record(&u.a), u.a_record);
}

/// The archive's files without the merge log.
fn but_the_log(fx: &Fx) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut t = fx.tree();
    t.remove(&fx.dir.join(MERGE_LOG));
    t
}

fn check_ok(fx: &Fx) {
    let c = fx.editor().check().unwrap();
    assert!(c.ok && c.information.is_empty(), "{c:?}");
}

/// Each monthly id of the fixture with the event it is in.
fn holders(fx: &Fx) -> Vec<&'static str> {
    let set = fx.set();
    let of = |m: &str| match set.resolve(m).unwrap() {
        ALPHA => ALPHA,
        BETA => BETA,
        GAMMA => GAMMA,
        DELTA => DELTA,
        other => panic!("{other}"),
    };
    // alpha's, beta's, gamma's, delta's.
    ["2026-09/E01", "2026-10/E04", "2026-11/E02", "2026-08/E07"]
        .into_iter()
        .map(of)
        .collect()
}

#[test]
fn a_merge_and_its_undo_leave_both_files_byte_for_byte() {
    let fx = fixture();
    let before = fx.tree();
    let id = merge(&fx, BETA, ALPHA);
    assert_eq!(fx.bytes(ALPHA), before[&fx.dir.join(format!("{ALPHA}.md"))]);
    assert_ne!(fx.bytes(BETA), before[&fx.dir.join(format!("{BETA}.md"))]);
    let u = fx.editor().unmerge(&format!(" {id} ")).unwrap();
    assert_eq!(
        (
            u.log_id.as_str(),
            u.undoes.as_str(),
            u.a.as_str(),
            u.b.as_str(),
            u.withdrawn
        ),
        ("m000002", "m000001", BETA, ALPHA, false)
    );
    assert!(but_the_log(&fx) == before);
    assert_eq!(
        fx.log(MERGE_LOG)[1],
        json!({"id": "m000002", "at": NOW, "undoes": "m000001"})
    );
    check_ok(&fx);
    // Undone once.
    refused(&fx, "m000001: already undone, by m000002", || {
        fx.editor().unmerge("m000001")
    });
    refused(&fx, "m000002: no such merge", || {
        fx.editor().unmerge("m000002")
    });
    refused(&fx, "m000009: no such merge", || {
        fx.editor().unmerge("m000009")
    });
    // Merged again, into another record, and undone again.
    let again = merge(&fx, BETA, DELTA);
    assert_eq!(again, "m000003");
    unmerge(&fx, &again);
    assert!(but_the_log(&fx) == before);
}

#[test]
fn three_merges_into_one_record_are_undone_in_every_order() {
    let orders = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    for order in orders {
        let fx = fixture();
        let before = fx.tree();
        let alpha = fx.bytes(ALPHA);
        let merged = [BETA, GAMMA, DELTA];
        let ids: Vec<String> = merged.iter().map(|a| merge(&fx, a, ALPHA)).collect();
        assert_eq!(fx.bytes(ALPHA), alpha);
        assert_eq!(holders(&fx), [ALPHA; 4]);
        let mut still: Vec<&str> = merged.to_vec();
        for i in order {
            unmerge(&fx, &ids[i]);
            still.retain(|a| *a != merged[i]);
            // The survivor's file is the one that was always there; each
            // record merged away is its first self again once unmerged.
            assert_eq!(fx.bytes(ALPHA), alpha, "{order:?}");
            for a in merged {
                let file = fx.dir.join(format!("{a}.md"));
                assert_eq!(
                    fx.bytes(a) != before[&file],
                    still.contains(&a),
                    "{order:?} {a}"
                );
            }
            let in_alpha = |a| if still.contains(&a) { ALPHA } else { a };
            assert_eq!(
                holders(&fx),
                [ALPHA, in_alpha(BETA), in_alpha(GAMMA), in_alpha(DELTA)],
                "{order:?}"
            );
            check_ok(&fx);
        }
        assert!(but_the_log(&fx) == before, "{order:?}");
        assert_eq!(fx.log(MERGE_LOG).len(), 6);
    }
}

#[test]
fn his_edits_to_the_survivor_between_merges_are_never_touched() {
    for order in [[0, 1, 2], [2, 1, 0], [1, 2, 0], [1, 0, 2]] {
        let fx = fixture();
        let before = fx.tree();
        let e = fx.editor();
        let m1 = merge(&fx, BETA, ALPHA);
        // He renames the event, narrows its end to before beta's, takes
        // off an alias beta also carries and adds one of his own.
        e.set(
            ALPHA,
            &FieldEdit {
                name: Some("Alpha, his name".into()),
                end: Some("2026-09-10".into()),
                remove_aliases: strs(&["Shared"]),
                add_aliases: strs(&["Beta follow-up", "His"]),
                ..FieldEdit::default()
            },
        )
        .unwrap();
        let m2 = merge(&fx, GAMMA, ALPHA);
        e.set(
            ALPHA,
            &FieldEdit {
                start: Some("2026-09-06".into()),
                description: Some("His description.".into()),
                ..FieldEdit::default()
            },
        )
        .unwrap();
        let m3 = merge(&fx, DELTA, ALPHA);
        let his = fx.bytes(ALPHA);
        let r = fx.record(ALPHA);
        assert_eq!(
            (r.name.as_str(), r.start.as_deref(), r.end.as_deref()),
            ("Alpha, his name", Some("2026-09-06"), Some("2026-09-10"))
        );
        assert_eq!(r.aliases, strs(&["The Alpha", "Beta follow-up", "His"]));
        assert_eq!(r.members, strs(&["2026-09/E01"]));
        let ids = [m1, m2, m3];
        for i in order {
            unmerge(&fx, &ids[i]);
            assert_eq!(fx.bytes(ALPHA), his, "{order:?}");
            check_ok(&fx);
        }
        // Everything but his record is as it was at the start.
        let mut after = but_the_log(&fx);
        let file = fx.dir.join(format!("{ALPHA}.md"));
        assert_eq!(after.insert(file.clone(), before[&file].clone()), Some(his));
        assert!(after == before, "{order:?}");
    }
}

#[test]
fn a_chain_is_undone_from_either_end() {
    for inner_first in [true, false] {
        let fx = fixture();
        let before = fx.tree();
        // delta into beta, then beta (with delta) into alpha.
        let inner = merge(&fx, DELTA, BETA);
        let outer = merge(&fx, BETA, ALPHA);
        assert_eq!(holders(&fx), [ALPHA, ALPHA, GAMMA, ALPHA]);
        let (alpha, beta) = (fx.bytes(ALPHA), fx.bytes(BETA));
        if inner_first {
            // Delta leaves while beta is itself merged away: beta's file
            // is not written.
            unmerge(&fx, &inner);
            assert_eq!(fx.bytes(BETA), beta);
            assert_eq!(holders(&fx), [ALPHA, ALPHA, GAMMA, DELTA]);
            check_ok(&fx);
            unmerge(&fx, &outer);
        } else {
            // Beta leaves and takes delta with it.
            unmerge(&fx, &outer);
            assert_eq!(holders(&fx), [ALPHA, BETA, GAMMA, BETA]);
            assert_eq!(fx.record(DELTA).merged_into.as_deref(), Some(BETA));
            check_ok(&fx);
            unmerge(&fx, &inner);
        }
        assert_eq!(fx.bytes(ALPHA), alpha);
        assert_eq!(holders(&fx), [ALPHA, BETA, GAMMA, DELTA]);
        assert!(but_the_log(&fx) == before);
        check_ok(&fx);
    }
}

#[test]
fn an_undo_does_not_depend_on_what_became_of_the_survivor() {
    let fx = fixture();
    let before = fx.tree();
    let e = fx.editor();
    let m1 = merge(&fx, BETA, ALPHA);
    // The survivor is merged away in turn, and that record removed.
    let m2 = merge(&fx, ALPHA, DELTA);
    e.remove(DELTA).unwrap();
    assert_eq!(holders(&fx), [DELTA, DELTA, GAMMA, DELTA]);
    let (alpha, delta) = (fx.bytes(ALPHA), fx.bytes(DELTA));
    unmerge(&fx, &m1);
    assert_eq!((fx.bytes(ALPHA), fx.bytes(DELTA)), (alpha, delta.clone()));
    assert_eq!(holders(&fx), [DELTA, BETA, GAMMA, DELTA]);
    check_ok(&fx);
    unmerge(&fx, &m2);
    assert_eq!(fx.bytes(DELTA), delta);
    e.restore(DELTA).unwrap();
    assert!(but_the_log(&fx) == before);
}

#[test]
fn unmerge_refuses_what_it_cannot_undo_and_changes_nothing() {
    let fx = fixture();
    let id = merge(&fx, BETA, ALPHA);
    // `a` is gone.
    let file = fx.dir.join(format!("{BETA}.md"));
    let kept = fs::read(&file).unwrap();
    fs::remove_file(&file).unwrap();
    refused(&fx, "2026-10-beta: no such event record", || {
        fx.editor().unmerge(&id)
    });
    fs::write(&file, &kept).unwrap();
    // `a` is not as the writer writes it: clearing the field would lose
    // what he put there.
    let mut text = String::from_utf8(kept.clone()).unwrap();
    text = text.replacen("---\n", "---\n# his comment\n", 1);
    fs::write(&file, &text).unwrap();
    refused(&fx, "the frontmatter is not as scout writes it", || {
        fx.editor().unmerge(&id)
    });
    fs::write(&file, &kept).unwrap();
    // A monthly id that `a` and the survivor both list, by a hand edit:
    // apart, the reader could give it to one only, and would say so.
    let alpha = fx.record(ALPHA);
    let mut both = alpha.clone();
    both.members.push("2026-10/E04".into());
    put(&fx.dir, &both);
    fx.set();
    refused(&fx, "the reader would warn", || fx.editor().unmerge(&id));
    put(&fx.dir, &alpha);
    unmerge(&fx, &id);
    check_ok(&fx);
}

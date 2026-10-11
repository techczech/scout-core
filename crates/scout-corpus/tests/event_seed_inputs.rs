//! `scout events seed` and its inputs. The tag rows are read by the
//! function an index build reads them with, so the seed and the index
//! always agree on which event an item is in. The briefing files, which
//! only the seed reads, stop the run when they are not what they must be.

mod event_seed_support;
use event_seed_support::*;
use scout_corpus::topics;
use std::collections::BTreeSet;

const K: &str = "x:1";

/// The event the index has the item `K` in, read as a build reads it.
fn index_event(root: &Path) -> Option<String> {
    let layer = topics::read_layer(root).unwrap().expect("a layer");
    layer.items.get(K).and_then(|i| i.info.event.clone())
}

/// Lay `lines` down as the tag rows of an archive without briefing files,
/// seed it, and check that the seed made a record for exactly the event
/// the index has `K` in (`want`), dated from the row the index kept.
fn seed_and_index_agree(case: &str, lines: &[Vec<u8>], want: Option<(&str, &str)>) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let mut bytes = Vec::new();
    for l in lines {
        bytes.extend_from_slice(l);
        bytes.push(b'\n');
    }
    let p = root.join(LAYER);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(&p, bytes).unwrap();

    let before = index_event(root);
    // Ids are the same event when they are equal under the index's fold.
    assert_eq!(
        before.as_deref().map(topics::fold),
        want.map(|w| topics::fold(w.0)),
        "{case}: the index"
    );

    let dry = seed(&opts(root, None, false)).unwrap();
    assert_eq!(
        dry.no_briefing_entry,
        strs(&want.map(|w| w.0).into_iter().collect::<Vec<_>>()),
        "{case}"
    );
    let Some((event, date)) = want else {
        assert_eq!((dry.monthly_events, dry.to_create), (0, 0), "{case}");
        return;
    };
    let (r, set) = applied(root);
    assert_eq!(
        (r.monthly_events, r.created, r.one_item),
        (1, 1, 1),
        "{case}"
    );
    // One record, holding the index's event and nothing else.
    let members: BTreeSet<&str> = set
        .records
        .values()
        .flat_map(|r| r.members.iter().map(String::as_str))
        .collect();
    assert_eq!(members, BTreeSet::from([event]), "{case}: the seed");
    let id = set.resolve(event).expect("a record").to_string();
    let record = rec(&set, &id);
    assert_eq!(record.start.as_deref(), Some(date), "{case}");
    assert_eq!(record.end.as_deref(), Some(date), "{case}");
    // The index, read again with the records there, has the item in that
    // record, and a second run has nothing to do.
    let layer = topics::read_layer(root).unwrap().unwrap();
    assert_eq!(layer.items[K].info.events, Some(vec![id]), "{case}");
    let again = seed(&opts(root, None, true)).unwrap();
    assert_eq!((again.created, again.already_in_a_record), (0, 1), "{case}");
}

fn line(v: Value) -> Vec<u8> {
    v.to_string().into_bytes()
}

#[test]
fn the_seed_and_the_index_keep_the_same_row_of_a_key() {
    let e01 = || line(row(K, Some("2026-09/E01"), Some("2026-09-03"), &["A"]));
    let e02 = || row(K, Some("2026-09/E02"), Some("2026-09-20"), &["B"]);
    let first = Some(("2026-09/E01", "2026-09-03"));

    // A later row of the key with a wrongly typed field is not a row: the
    // earlier, valid row stands (the review's case).
    let mut wrong_themes = e02();
    wrong_themes["themes"] = json!("wrong type");
    seed_and_index_agree("wrongly typed themes", &[e01(), line(wrong_themes)], first);

    // A later line that is not JSON.
    let mut cut = line(e02());
    cut.pop();
    seed_and_index_agree("bad JSON", &[e01(), cut, b"not json".to_vec()], first);

    // A later row whose `event` is not text.
    for bad in [
        json!(7),
        json!(["2026-09/E02"]),
        json!({"id": "2026-09/E02"}),
    ] {
        let mut wrong_event = e02();
        wrong_event["event"] = bad.clone();
        seed_and_index_agree(
            &format!("wrongly typed event {bad}"),
            &[e01(), line(wrong_event)],
            first,
        );
    }

    // Other wrongly typed fields, a line that is not an object, a line
    // that is not UTF-8, and a line over the reader's cap.
    let mut wrong_date = e02();
    wrong_date["date"] = json!(20260920);
    let mut wrong_key = e02();
    wrong_key["key"] = json!([K]);
    let mut not_utf8 = line(e02());
    not_utf8.extend_from_slice(b" \xff");
    let mut too_long = e02();
    too_long["event_name"] = json!("n".repeat(topics::MAX_LINE_BYTES));
    seed_and_index_agree(
        "other bad lines",
        &[
            e01(),
            line(wrong_date),
            line(wrong_key),
            line(json!([e02()])),
            not_utf8,
            line(too_long),
        ],
        first,
    );

    // Two valid rows of one key: the later one counts, and the earlier
    // row's event gets no record.
    seed_and_index_agree(
        "two valid rows",
        &[e01(), line(e02())],
        Some(("2026-09/E02", "2026-09-20")),
    );
    // A valid row after a bad one, and a bad one after that.
    let mut wrong = e02();
    wrong["entities"] = json!("x");
    seed_and_index_agree(
        "valid, bad, valid, bad",
        &[
            line(row(K, Some("2026-08/E05"), Some("2026-08-01"), &[])),
            line(wrong.clone()),
            e01(),
            line(wrong),
        ],
        first,
    );
    // Only bad rows: the index has no such item and the seed no event.
    let mut only = e02();
    only["themes"] = json!("wrong type");
    seed_and_index_agree("no valid row", &[line(only)], None);
    // The id forms the index takes for one event: any letter case, any
    // whitespace around. The seed writes the member in one spelling and
    // the index, reading the record, has the item in it.
    for written in [
        "2026-09/e01",
        " 2026-09/E01 ",
        "\t2026-09/e01\u{a0}",
        "\u{2003}2026-09/E01\n",
    ] {
        seed_and_index_agree(
            &format!("written {written:?}"),
            &[line(row(K, Some(written), Some("2026-09-03"), &[]))],
            first,
        );
    }
    seed_and_index_agree(
        "a quarter in lower case",
        &[line(row(K, Some("2022-q2/e7"), Some("2022-05-03"), &[]))],
        Some(("2022-Q2/E7", "2022-05-03")),
    );
    // A blank event is no event to either.
    seed_and_index_agree(
        "blank event",
        &[line(row(K, Some("  "), Some("2026-09-03"), &[]))],
        None,
    );
    // A later valid row without an event takes the item out of its event.
    seed_and_index_agree(
        "later row has no event",
        &[e01(), line(row(K, None, Some("2026-09-21"), &[]))],
        None,
    );
}

#[test]
fn a_malformed_briefing_file_stops_the_run_before_any_write_with_the_file_named() {
    let ev = |v: Value| json!({"month": "2026-09", "events": [v]}).to_string();
    let ok = json!({"id": "E01", "name": "Fine", "aliases": [], "description": "d"});
    let with = |k: &str, v: Value| {
        let mut e = ok.clone();
        e[k] = v;
        ev(e)
    };
    let cases: Vec<(String, &str)> = vec![
        ("{not json".into(), "not JSON"),
        ("".into(), "not JSON"),
        ("[]".into(), "not a JSON object"),
        ("\"2026-09\"".into(), "not a JSON object"),
        (
            json!({"month": 202609, "events": []}).to_string(),
            "\"month\" is 202609, the file name says 2026-09",
        ),
        (
            json!({"month": null, "events": []}).to_string(),
            "\"month\" is null, the file name says 2026-09",
        ),
        (
            json!({"month": "2026-09"}).to_string(),
            "no \"events\" list",
        ),
        (
            json!({"month": "2026-09", "events": {"E01": {}}}).to_string(),
            "no \"events\" list",
        ),
        (ev(json!("E01")), "event 1 of the list is not an object"),
        (ev(json!(["E01"])), "event 1 of the list is not an object"),
        (
            json!({"events": [ok.clone(), null]}).to_string(),
            "event 2 of the list is not an object",
        ),
        (
            ev(json!({"name": "No id"})),
            "event id null is not E<number>",
        ),
        (with("id", json!(1)), "event id 1 is not E<number>"),
        (
            with("id", json!(["E01"])),
            "event id [\"E01\"] is not E<number>",
        ),
        (with("id", json!("E")), "event id \"E\" is not E<number>"),
        (with("id", json!("")), "event id \"\" is not E<number>"),
        (
            with("id", json!("E01/../../x")),
            "event id \"E01/../../x\" is not E<number>",
        ),
        (
            with("id", json!("2026-09/E01")),
            "event id \"2026-09/E01\" is not E<number>",
        ),
        (
            json!({"events": [ok.clone(), ok.clone()]}).to_string(),
            "event 2026-09/E01 appears twice",
        ),
        (
            json!({"events": [ok.clone(), {"id": " e01 "}]}).to_string(),
            "event 2026-09/E01 appears twice",
        ),
        (
            with("id", json!("E 01")),
            "event id \"E 01\" is not E<number>",
        ),
        (
            with("name", json!(7)),
            "event 2026-09/E01: \"name\" is not text",
        ),
        (
            with("name", json!(["Fine"])),
            "event 2026-09/E01: \"name\" is not text",
        ),
        (
            with("description", json!({"text": "d"})),
            "event 2026-09/E01: \"description\" is not text",
        ),
        (
            with("description", json!(false)),
            "event 2026-09/E01: \"description\" is not text",
        ),
        (
            with("aliases", json!("one")),
            "event 2026-09/E01: \"aliases\" is not a list of text",
        ),
        (
            with("aliases", json!(["one", 2])),
            "event 2026-09/E01: \"aliases\" is not a list of text",
        ),
        (
            with("aliases", json!([["one"]])),
            "event 2026-09/E01: \"aliases\" is not a list of text",
        ),
    ];
    for (text, why) in cases {
        for apply in [true, false] {
            // A sound briefing file and tag rows beside the bad one: none
            // of their events is seeded either.
            let tmp = tempfile::tempdir().unwrap();
            let root = tmp.path();
            put_briefing(root, "2026-08", &[("E01", "Sound", None, &[])]);
            put_rows(
                root,
                &[row(K, Some("2026-10/E01"), Some("2026-10-01"), &[])],
            );
            let bad = put(
                root,
                "analysis/context/briefings/2026-09-events.json",
                &text,
            );
            let before = tree(root);
            let e = seed(&opts(root, None, apply)).unwrap_err().to_string();
            assert!(
                e.starts_with(&format!("{}: {why}", bad.display())),
                "{text}: {e}"
            );
            assert_eq!(tree(root), before, "{text}");
            assert!(!root.join(events::EVENTS_DIR).exists(), "{text}");
            // Into another out folder: it is not made.
            let scratch = tempfile::tempdir().unwrap();
            let out = scratch.path().join("out");
            assert!(seed(&opts(root, Some(&out), apply)).is_err());
            assert!(names(scratch.path()).is_empty(), "{text}");
        }
    }
}

#[test]
fn absent_and_null_briefing_fields_are_empty_not_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    put(
        root,
        "analysis/context/briefings/2026-09-events.json",
        &json!({"events": [
            {"id": "E01"},
            {"id": "E02", "name": null, "aliases": null, "description": null},
            {"id": "E03", "name": " A\n name ", "aliases": [" b  c ", ""], "description": " d \n"},
        ]})
        .to_string(),
    );
    let (r, set) = applied(root);
    assert_eq!((r.briefing_events, r.created), (3, 3));
    // Ids as the index compares them: letter case and outer whitespace
    // do not count, and the member is written in the one spelling.
    let tmp2 = tempfile::tempdir().unwrap();
    put(
        tmp2.path(),
        "analysis/context/briefings/2022-Q2-events.json",
        &json!({"events": [{"id": "e01", "name": "Lower"}, {"id": " E02\t", "name": "Spaced"}]})
            .to_string(),
    );
    put_rows(
        tmp2.path(),
        &[row("k", Some("2022-q2/E02"), Some("2022-05-05"), &[])],
    );
    let (r2, set2) = applied(tmp2.path());
    assert_eq!((r2.created, r2.no_briefing_entry.len()), (2, 0));
    assert_eq!(rec(&set2, "2022-04-lower").members, ["2022-Q2/E01"]);
    let spaced = rec(&set2, "2022-04-spaced");
    assert_eq!(spaced.members, ["2022-Q2/E02"]);
    assert_eq!(spaced.start.as_deref(), Some("2022-05-05"));
    // No name: the monthly id stands in.
    let unnamed = rec(&set, set.resolve("2026-09/E02").unwrap());
    assert_eq!(unnamed.name, "2026-09/E02");
    assert!(unnamed.aliases.is_empty() && unnamed.description.is_empty());
    let named = rec(&set, "2026-09-a-name");
    assert_eq!(
        (
            named.name.as_str(),
            named.aliases.clone(),
            named.description.as_str()
        ),
        ("A name", strs(&["b c"]), "d")
    );
}

#[cfg(unix)]
#[test]
fn a_briefing_name_that_is_not_a_plain_file_stops_the_run() {
    use std::os::unix::fs::symlink;
    let elsewhere = tempfile::tempdir().unwrap();
    let real = put(
        elsewhere.path(),
        "real.json",
        &briefing("2026-09", &[("E01", "Through a link", None, &[])]),
    );
    let name = "analysis/context/briefings/2026-09-events.json";
    for kind in ["link to a file", "dangling link", "folder"] {
        for apply in [true, false] {
            let tmp = tempfile::tempdir().unwrap();
            let root = tmp.path();
            put_briefing(root, "2026-08", &[("E01", "Sound", None, &[])]);
            let at = root.join(name);
            match kind {
                "link to a file" => symlink(&real, &at).unwrap(),
                "dangling link" => symlink(elsewhere.path().join("gone.json"), &at).unwrap(),
                _ => fs::create_dir(&at).unwrap(),
            }
            let before = (tree(root), tree(elsewhere.path()));
            let e = seed(&opts(root, None, apply)).unwrap_err().to_string();
            assert_eq!(e, format!("{}: not a plain file", at.display()), "{kind}");
            assert_eq!((tree(root), tree(elsewhere.path())), before, "{kind}");
            assert!(!root.join(events::EVENTS_DIR).exists(), "{kind}");
        }
    }
    // A superseded name is not a briefing file, whatever it is.
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    put_briefing(root, "2026-08", &[("E01", "Sound", None, &[])]);
    symlink(
        &real,
        root.join("analysis/context/briefings/2026-09-events.superseded-1.json"),
    )
    .unwrap();
    put(
        root,
        "analysis/context/briefings/2026-07-events.superseded-1.json",
        "{not json",
    );
    assert_eq!(seed(&opts(root, None, true)).unwrap().created, 1);
}

/// Rows that spell one event differently are one event with all of them
/// as items; an id the index does not take for that event is not one.
#[test]
fn rows_spelling_one_event_differently_seed_one_record_the_index_puts_them_all_in() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    put_rows(
        root,
        &[
            row("a", Some("2026-09/E01"), Some("2026-09-03"), &[]),
            row("b", Some(" 2026-09/e01\t"), Some("2026-09-20"), &[]),
            row("c", Some("2026-09 / E01"), Some("2026-09-28"), &[]),
            row("d", Some("2026-09/E1"), Some("2026-09-29"), &[]),
            // A month the index does not accept: not a date here either.
            row("e", Some("2026-09/e01"), Some("2026-13-01"), &[]),
        ],
    );
    let (r, set) = applied(root);
    // E01 (a, b, e) and E1 (d); c names no monthly event.
    assert_eq!((r.monthly_events, r.created, r.one_item), (2, 2, 1));
    let id = set.resolve("2026-09/E01").unwrap().to_string();
    let other = set.resolve("2026-09/E1").unwrap().to_string();
    assert_ne!(id, other);
    let record = rec(&set, &id);
    assert_eq!(record.members, ["2026-09/E01"]);
    assert_eq!(
        (record.start.as_deref(), record.end.as_deref()),
        (Some("2026-09-03"), Some("2026-09-20"))
    );
    let layer = topics::read_layer(root).unwrap().unwrap();
    let events = |k: &str| layer.items[k].info.events.clone().unwrap();
    for k in ["a", "b", "e"] {
        assert_eq!(events(k), std::slice::from_ref(&id), "{k}");
    }
    assert_eq!(events("d"), [other]);
    assert_eq!(events("c"), ["2026-09 / E01"]);
    assert_eq!(layer.items["e"].info.month, None);
    assert_eq!(seed(&opts(root, None, true)).unwrap().created, 0);
}

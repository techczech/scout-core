//! Reading the monthly briefings (`scout_corpus::briefings`): the list,
//! one briefing with its events linked to lasting event records, every
//! refusal, and that nothing in the archive changes. Each test builds its
//! own archive in a temp folder.

mod event_seed_support;
use event_seed_support::{json, put, tree, Path, Value};
use scout_corpus::briefings::{
    briefing, briefings, BriefingListing, EventLink, NoBriefing, PeriodKind, MAX_BRIEFING_BYTES,
};

const DIR: &str = "analysis/context/briefings";

fn doc(period: &str, events: Value) -> Value {
    json!({
        "meta": {"month": period, "model": "test-model", "generated": "2026-10-01T08:00:00.000Z",
                 "items": 12, "cost_usd": 0.5},
        "overview": format!("What happened in {period}."),
        "events": events,
        "names": [{"name": "Full Name", "aliases": ["FN"], "note": "a thing"}],
        "debates": [{"title": "A debate", "summary": "Two sides.", "when": "mid-month"}],
        "engagement": "Mostly E01.",
        "problems": [],
    })
}

fn event(id: &str, name: &str) -> Value {
    json!({"id": id, "name": name, "date": "2026-09-03", "aliases": ["also"],
           "description": format!("About {name}."), "items": ["m0018", "m0020"]})
}

fn put_doc(root: &Path, period: &str, doc: &Value) {
    put(
        root,
        &format!("{DIR}/{period}-briefing.json"),
        &doc.to_string(),
    );
}

fn record(root: &Path, id: &str, rest: &str) {
    put(
        root,
        &format!("analysis/events/{id}.md"),
        &format!("---\nid: {id}\nname: Name of {id}\n{rest}---\n\nBody.\n"),
    );
}

/// The error of `briefing`, causes joined as the CLI joins them.
fn refusal(root: &Path, period: &str) -> String {
    format!("{:#}", briefing(root, period).unwrap_err())
}

/// The entries of `dir` by name: a plain file with its bytes, anything
/// else with its kind (nothing but a plain file is opened: reading a FIFO
/// would wait).
fn entries(dir: &Path) -> Vec<(String, String, Vec<u8>)> {
    let mut out: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| {
            let e = e.unwrap();
            let t = e.file_type().unwrap();
            let bytes = if t.is_file() {
                std::fs::read(e.path()).unwrap()
            } else {
                Vec::new()
            };
            (
                e.file_name().into_string().unwrap(),
                format!("{t:?}"),
                bytes,
            )
        })
        .collect();
    out.sort();
    out
}

fn periods(rows: &[BriefingListing]) -> Vec<String> {
    rows.iter()
        .map(|r| match r {
            BriefingListing::Briefing(b) => b.period.clone(),
            BriefingListing::Unreadable { error } => format!("ERROR {error}"),
        })
        .collect()
}

#[test]
fn the_list_is_newest_first_skips_superseded_files_and_keeps_going_past_a_bad_one() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    for p in ["2026-09", "2025-12", "2022-Q2", "2022-07", "2022-03"] {
        put_doc(
            root,
            p,
            &doc(p, json!([event("E01", "One"), event("E02", "Two")])),
        );
    }
    put(root, &format!("{DIR}/2026-10-briefing.json"), "{ not json");
    // Not briefings: a superseded file, a superseded folder, the other
    // files of a period, a period that is none.
    let old = doc("2026-09", json!([])).to_string();
    for name in [
        "2026-09-briefing.superseded-20261011-1603.json",
        "2026-08.superseded-20261011-1603/2026-08-briefing.json",
        "2026-09-events.json",
        "2026-09.md",
        "2026-13-briefing.json",
    ] {
        put(root, &format!("{DIR}/{name}"), &old);
    }
    let before = tree(root);

    let rows = briefings(root).unwrap();
    let got = periods(&rows);
    assert!(got[0].starts_with("ERROR ") && got[0].contains("2026-10-briefing.json: not JSON"));
    assert_eq!(
        got[1..],
        ["2026-09", "2025-12", "2022-07", "2022-Q2", "2022-03"]
    );
    assert!(!got[0].contains('\n'));

    let BriefingListing::Briefing(q) = &rows[4] else {
        panic!("{:?}", rows[4])
    };
    assert_eq!(q.kind, PeriodKind::Quarter);
    assert_eq!(
        serde_json::to_value(&rows[1]).unwrap(),
        json!({"period": "2026-09", "kind": "month", "model": "test-model",
               "generated": "2026-10-01T08:00:00.000Z", "items": 12, "events": 2})
    );
    // An error row holds the error and nothing else.
    let bad = serde_json::to_value(&rows[0]).unwrap();
    assert_eq!(
        bad.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["error"]
    );
    assert_eq!(tree(root), before);
}

#[test]
fn a_briefing_links_each_event_to_the_record_that_holds_it_now() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    put_doc(
        root,
        "2026-09",
        &doc(
            "2026-09",
            json!([
                event("E01", "Plain"),
                event("e02", "Merged away"),
                event("E03", "Removed"),
                event("E04", "Held by none"),
                // Only an id: everything else absent or null.
                {"id": "E05", "name": null},
            ]),
        ),
    );
    record(root, "2026-09-plain", "members: [2026-09/E01]\n");
    record(
        root,
        "2026-09-merged-away",
        "members: [2026-09/E02]\nmerged_into: 2026-09-plain\n",
    );
    record(
        root,
        "2026-09-removed",
        "members: [2026-09/E03]\nremoved: 2026-10-02\n",
    );
    let before = tree(root);

    let b = briefing(root, "2026-09").unwrap();
    assert_eq!((b.period.as_str(), b.kind), ("2026-09", PeriodKind::Month));
    assert_eq!((b.model.as_str(), b.items), ("test-model", Some(12)));
    assert_eq!(b.generated, "2026-10-01T08:00:00.000Z");
    assert_eq!(b.overview, "What happened in 2026-09.");
    assert_eq!(b.engagement, "Mostly E01.");
    assert_eq!(
        (b.names[0].name.as_str(), b.names[0].note.as_str()),
        ("Full Name", "a thing")
    );
    assert_eq!(b.names[0].aliases, ["FN"]);
    assert_eq!(b.debates[0].title, "A debate");

    let ids: Vec<&str> = b.events.iter().map(|e| e.monthly_id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "2026-09/E01",
            "2026-09/E02",
            "2026-09/E03",
            "2026-09/E04",
            "2026-09/E05"
        ]
    );
    let link = |id: &str, removed| {
        Some(EventLink {
            id: id.to_string(),
            name: format!("Name of {id}"),
            removed,
        })
    };
    assert_eq!(b.events[0].record, link("2026-09-plain", false));
    // The merged-away record's member leads to the record that holds it.
    assert_eq!(b.events[1].record, link("2026-09-plain", false));
    assert_eq!(b.events[2].record, link("2026-09-removed", true));
    assert_eq!(b.events[3].record, None);
    let e = &b.events[0];
    assert_eq!((e.name.as_str(), e.date.as_str()), ("Plain", "2026-09-03"));
    assert_eq!(
        (e.aliases.as_slice(), e.description.as_str()),
        (&["also".to_string()][..], "About Plain.")
    );
    let bare = &b.events[4];
    assert_eq!(
        (
            bare.name.as_str(),
            bare.date.as_str(),
            bare.description.as_str()
        ),
        ("", "", "")
    );
    assert!(bare.aliases.is_empty());

    // The shape the CLI prints; the briefing's own item ids are not in it.
    let v = serde_json::to_value(&b).unwrap();
    assert_eq!(
        v.as_object().unwrap().keys().collect::<Vec<_>>(),
        [
            "debates",
            "engagement",
            "events",
            "generated",
            "items",
            "kind",
            "model",
            "names",
            "overview",
            "period"
        ]
    );
    assert_eq!(
        v["events"][2],
        json!({"monthly_id": "2026-09/E03", "name": "Removed", "date": "2026-09-03",
               "aliases": ["also"], "description": "About Removed.",
               "record": {"id": "2026-09-removed", "name": "Name of 2026-09-removed", "removed": true}})
    );
    assert_eq!(v["events"][3]["record"], Value::Null);
    assert!(!v.to_string().contains("m0018"));
    assert_eq!(tree(root), before);
}

#[test]
fn absent_parts_are_empty_and_a_quarter_reads_by_its_own_period() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    put_doc(
        root,
        "2022-Q2",
        &json!({"meta": {"month": "2022-Q2"}, "events": [], "overview": null, "names": null}),
    );
    // No events: the records folder is not needed.
    let b = briefing(root, "2022-Q2").unwrap();
    assert_eq!(b.kind, PeriodKind::Quarter);
    assert_eq!(
        (b.model.as_str(), b.generated.as_str(), b.items),
        ("", "", None)
    );
    assert_eq!((b.overview.as_str(), b.engagement.as_str()), ("", ""));
    assert!(b.events.is_empty() && b.names.is_empty() && b.debates.is_empty());
}

#[test]
fn no_briefings_folder_is_an_empty_list_and_no_briefing() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    assert!(briefings(root).unwrap().is_empty());
    let e = briefing(root, "2026-09").unwrap_err();
    assert_eq!(
        e.downcast_ref::<NoBriefing>(),
        Some(&NoBriefing("2026-09".into()))
    );
    assert_eq!(e.to_string(), "No briefing for 2026-09.");
    // A folder with other periods, and a superseded file of this one.
    put_doc(root, "2026-08", &doc("2026-08", json!([])));
    put(
        root,
        &format!("{DIR}/2026-09-briefing.superseded-1.json"),
        "{}",
    );
    assert_eq!(refusal(root, "2026-09"), "No briefing for 2026-09.");
    // An archive that is not there is not an empty archive.
    let gone = root.join("nowhere");
    assert!(format!("{:#}", briefings(&gone).unwrap_err()).contains("the archive is not a folder"));
    let up = root.join("a/../b");
    assert!(format!("{:#}", briefings(&up).unwrap_err()).contains("`..` component"));
}

#[test]
fn a_period_of_another_shape_is_refused_before_any_file_is_looked_at() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    // What an unvalidated join would open.
    put(
        root,
        "analysis/context/secret-briefing.json",
        &doc("2026-09", json!([])).to_string(),
    );
    put(
        root,
        &format!("{DIR}/x-briefing.json"),
        &doc("x", json!([])).to_string(),
    );
    for p in [
        "",
        "x",
        "../secret",
        "2026-9",
        "2026-13",
        "2026-00",
        "2026-Q5",
        "2026-q1",
        "2026-09 ",
        "2026-09/..",
        "2026-09-briefing.json",
        "２０２６-09",
    ] {
        assert_eq!(
            refusal(root, p),
            format!("\"{p}\" is not a month (YYYY-MM) or a quarter (YYYY-Qn).")
        );
    }
    // Checked first: even an archive that is not there does not matter.
    assert!(refusal(&root.join("nowhere"), "nope").starts_with("\"nope\" is not a month"));
}

#[test]
fn a_file_that_is_not_a_briefing_is_refused_with_the_file_and_the_field() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let good = |events: Value| doc("2026-09", events).to_string();
    let with = |key: &str, v: Value| {
        let mut d = doc("2026-09", json!([event("E01", "One")]));
        d[key] = v;
        d.to_string()
    };
    let cases: Vec<(String, &str)> = vec![
        ("{ not json".into(), "not JSON"),
        ("[1, 2]".into(), "not a JSON object"),
        (json!({"events": []}).to_string(), "no \"meta\" object"),
        (
            doc("2026-08", json!([])).to_string(),
            "\"meta.month\" is \"2026-08\", the file name says 2026-09",
        ),
        (with("events", json!({"E01": {}})), "no \"events\" list"),
        (with("events", Value::Null), "no \"events\" list"),
        (with("overview", json!(["a"])), "\"overview\" is not text"),
        (with("engagement", json!(3)), "\"engagement\" is not text"),
        (with("names", json!("x")), "\"names\" is not a list"),
        (
            with("names", json!(["x"])),
            "name 1 of the list is not an object",
        ),
        (
            with("debates", json!([{"title": "t"}, {"summary": 1}])),
            "debate 2: \"summary\" is not text",
        ),
        (
            with("meta", json!({"month": "2026-09", "model": 5})),
            "\"meta.model\" is not text",
        ),
        (
            with("meta", json!({"month": "2026-09", "items": -1})),
            "\"meta.items\" is not a whole number",
        ),
        (good(json!(["E01"])), "event 1 of the list is not an object"),
        (
            good(json!([{"id": "E01", "name": 7}])),
            "event 2026-09/E01: \"name\" is not text",
        ),
        (
            good(json!([{"id": "E01", "date": 20260903}])),
            "event 2026-09/E01: \"date\" is not text",
        ),
        (
            good(json!([{"id": "E01", "aliases": "a"}])),
            "event 2026-09/E01: \"aliases\" is not a list of text",
        ),
        (
            good(json!([{"id": "E01", "description": {}}])),
            "event 2026-09/E01: \"description\" is not text",
        ),
        (
            good(json!([{"id": "1"}])),
            "event id \"1\" is not E<number>",
        ),
        (
            good(json!([{"name": "no id"}])),
            "event id null is not E<number>",
        ),
        (
            good(json!([{"id": "E01"}, {"id": " e01 "}])),
            "event 2026-09/E01 appears twice",
        ),
    ];
    let file = root.join(DIR).join("2026-09-briefing.json");
    for (text, why) in cases {
        put(root, &format!("{DIR}/2026-09-briefing.json"), &text);
        let got = refusal(root, "2026-09");
        assert!(got.starts_with(&format!("{}: ", file.display())), "{got}");
        assert!(got.contains(why), "{why}: {got}");
        // The list says the same of it, on one row.
        assert_eq!(periods(&briefings(root).unwrap()), [format!("ERROR {got}")]);
    }
}

#[test]
fn a_file_over_four_mebibytes_is_refused_with_its_size() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let mut d = doc("2026-09", json!([]));
    d["overview"] = json!("x".repeat(MAX_BRIEFING_BYTES as usize));
    put_doc(root, "2026-09", &d);
    let bytes = std::fs::metadata(root.join(DIR).join("2026-09-briefing.json"))
        .unwrap()
        .len();
    let got = refusal(root, "2026-09");
    assert!(
        got.ends_with(&format!(
            "2026-09-briefing.json: {bytes} bytes, larger than the limit of 4194304 bytes (4 MiB); it was not read"
        )),
        "{got}"
    );
    assert_eq!(periods(&briefings(root).unwrap()), [format!("ERROR {got}")]);
}

#[cfg(unix)]
#[test]
fn a_symlink_a_folder_and_a_special_file_under_a_briefing_name_are_refused() {
    use std::os::unix::fs::symlink;
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let dir = root.join(DIR);
    // A whole, valid briefing, reached only through a link.
    let real = put(
        root,
        "elsewhere/real.json",
        &doc("2026-09", json!([])).to_string(),
    );
    std::fs::create_dir_all(&dir).unwrap();
    symlink(&real, dir.join("2026-09-briefing.json")).unwrap();
    symlink(dir.join("nothing"), dir.join("2026-08-briefing.json")).unwrap();
    std::fs::create_dir(dir.join("2026-07-briefing.json")).unwrap();
    let fifo = std::ffi::CString::new(dir.join("2026-06-briefing.json").to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    put_doc(root, "2026-05", &doc("2026-05", json!([])));
    let before = entries(&dir);

    for p in ["2026-09", "2026-08", "2026-07", "2026-06"] {
        let got = refusal(root, p);
        assert!(
            got.ends_with(&format!("{p}-briefing.json: not a plain file")),
            "{got}"
        );
    }
    let got = periods(&briefings(root).unwrap());
    assert_eq!(got.len(), 5);
    assert!(got[..4]
        .iter()
        .all(|r| r.starts_with("ERROR ") && r.ends_with("not a plain file")));
    assert_eq!(got[4], "2026-05");
    assert_eq!(entries(&dir), before);
}

/// `analysis`, `context` or `briefings` as a link to a folder outside the
/// archive that holds a valid briefing: nothing there is read. The same
/// for a file in a folder's place.
#[cfg(unix)]
#[test]
fn a_link_in_place_of_a_folder_on_the_way_to_the_briefings_is_refused() {
    use std::os::unix::fs::symlink;
    let parts = ["analysis", "context", "briefings"];
    for n in 0..parts.len() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("archive");
        // Outside the archive: the folder the link leads to, with the rest
        // of the way and a whole briefing under it.
        let outside = tmp.path().join("outside");
        let mut rest = parts[n + 1..].to_vec();
        rest.push("2026-09-briefing.json");
        put(
            &outside,
            &rest.join("/"),
            &doc("2026-09", json!([])).to_string(),
        );
        let at = root.join(parts[..=n].join("/"));
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        let want = format!(
            "{}: not a plain folder; the briefings were not read",
            at.display()
        );

        symlink(&outside, &at).unwrap();
        // The link does lead to a briefing.
        assert!(root.join(DIR).join("2026-09-briefing.json").is_file());
        assert_eq!(format!("{:#}", briefings(&root).unwrap_err()), want);
        assert_eq!(refusal(&root, "2026-09"), want);

        std::fs::remove_file(&at).unwrap();
        std::fs::write(&at, "a file").unwrap();
        assert_eq!(format!("{:#}", briefings(&root).unwrap_err()), want);
        assert_eq!(refusal(&root, "2026-09"), want);

        // Not there: no briefings, as before.
        std::fs::remove_file(&at).unwrap();
        assert!(briefings(&root).unwrap().is_empty());
        assert_eq!(refusal(&root, "2026-09"), "No briefing for 2026-09.");
    }
}

/// The archive path is the caller's choice, a link or not.
#[cfg(unix)]
#[test]
fn an_archive_reached_through_a_link_is_read() {
    let tmp = tempfile::tempdir().unwrap();
    let real = tmp.path().join("real");
    put_doc(
        &real,
        "2026-09",
        &doc("2026-09", json!([event("E01", "One")])),
    );
    record(&real, "2026-09-one", "members: [2026-09/E01]\n");
    let link = tmp.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();

    assert_eq!(periods(&briefings(&link).unwrap()), ["2026-09"]);
    let b = briefing(&link, "2026-09").unwrap();
    assert_eq!(b.events[0].record.as_ref().unwrap().id, "2026-09-one");
}

/// The records reader decodes a YAML escape after its look for control
/// characters, so a record's name can hold one; the link's does not.
#[test]
fn a_linked_record_name_holds_no_control_characters() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    put_doc(
        root,
        "2026-09",
        &doc("2026-09", json!([event("E01", "One")])),
    );
    put(
        root,
        "analysis/events/2026-09-one.md",
        "---\nid: 2026-09-one\nname: \"a\\0b \\e[31m c\\td\\Ne\"\nmembers: [2026-09/E01]\n---\n\nBody.\n",
    );
    // The reader does hand the characters on.
    let set = event_seed_support::events::read(root).unwrap().unwrap();
    assert!(set.refused.is_empty());
    assert!(set.records.values().next().unwrap().name.contains('\0'));

    let b = briefing(root, "2026-09").unwrap();
    let link = b.events[0].record.as_ref().unwrap();
    assert_eq!(link.id, "2026-09-one");
    assert_eq!(link.name, "ab [31m c d e");
    assert!(!link.name.chars().any(char::is_control));
    assert!(!serde_json::to_string(&b).unwrap().contains("\\u0000"));
}

#[test]
fn a_records_folder_that_cannot_answer_fails_the_read_instead_of_dropping_links() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    put_doc(
        root,
        "2026-09",
        &doc("2026-09", json!([event("E01", "One")])),
    );
    // No records folder.
    let got = refusal(root, "2026-09");
    assert!(
        got.ends_with("analysis/events: no records folder (run `scout events seed` first)"),
        "{got}"
    );
    // The list does not need it.
    assert_eq!(periods(&briefings(root).unwrap()), ["2026-09"]);

    // A record the reader skips could be the one that holds the event.
    record(root, "2026-09-one", "members: [2026-09/E01]\n");
    put(
        root,
        "analysis/events/broken.md",
        "---\nid: [unclosed\n---\n",
    );
    put(
        root,
        "analysis/events/other-name.md",
        "---\nid: something-else\nmembers: []\n---\n",
    );
    let before = tree(root);
    let got = refusal(root, "2026-09");
    let lines: Vec<&str> = got.lines().collect();
    assert_eq!(lines.len(), 2, "{got}");
    assert!(
        lines[0].contains("analysis/events/broken.md: cannot read the record ("),
        "{got}"
    );
    assert!(
        lines[1].contains("other-name.md: cannot read the record (id \"something-else\" does not match the file name)"),
        "{got}"
    );
    assert_eq!(tree(root), before);

    std::fs::remove_file(root.join("analysis/events/broken.md")).unwrap();
    std::fs::remove_file(root.join("analysis/events/other-name.md")).unwrap();
    let b = briefing(root, "2026-09").unwrap();
    assert_eq!(b.events[0].record.as_ref().unwrap().id, "2026-09-one");
}

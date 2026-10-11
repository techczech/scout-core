//! `scout events seed` at the library seam ([`scout_corpus::event_seed`]):
//! what it reads and the records it writes. Every record is read back
//! with the reader itself. What it refuses is in `event_seed_refusals.rs`.

mod event_seed_support;
use event_seed_support::*;

#[test]
fn a_dry_run_writes_nothing_not_even_the_out_folder() {
    let a = archive();
    let before = tree(a.path());
    let r = seed(&opts(a.path(), None, false)).unwrap();
    assert!(!r.apply);
    assert_eq!((r.to_create, r.created), (10, 0));
    assert_eq!((r.monthly_events, r.briefing_events), (10, 8));
    assert_eq!((r.no_items, r.briefing_events_without_items), (1, 1));
    assert_eq!(r.notes.briefing_events_without_items, 1);
    assert!(!a.path().join("analysis/events").exists());
    assert_eq!(tree(a.path()), before);
}

#[test]
fn apply_writes_one_record_per_monthly_event_with_the_spec_frontmatter() {
    let a = archive();
    let out = a.path().join("analysis/events");
    let (r, set) = applied(a.path());
    assert_eq!(r.created, 10);
    let long = format!("2026-09-long-{}", "word-".repeat(11));
    let long = long[..8 + SLUG_MAX].trim_end_matches('-');
    let ids: Vec<&str> = set.records.keys().map(String::as_str).collect();
    assert_eq!(
        ids,
        [
            "2022-04-quarter-story",
            "2026-08-2026-08-e01",
            "2026-09-event-e10",
            "2026-09-gemini-launch",
            "2026-09-gemini-launch-2",
            "2026-09-he-said-it-s-1-naive-cafe-100-sic-more",
            long,
            "2026-09-nobody-tagged-this",
            "2026-09-only-in-the-tag-rows",
            "2026-10-gemini-launch",
        ]
    );
    for (k, rec) in &set.records {
        assert_eq!(&rec.id, k);
        assert_eq!(rec.origin.as_deref(), Some("model"));
        assert_eq!((&rec.edited, &rec.merged_into), (&None, &None));
        assert_eq!(rec.members.len(), 1);
        assert_eq!(set.resolve(&rec.members[0]), Some(k.as_str()));
    }
    assert_eq!(
        rec(&set, "2026-09-he-said-it-s-1-naive-cafe-100-sic-more"),
        &EventRecord {
            id: "2026-09-he-said-it-s-1-naive-cafe-100-sic-more".into(),
            name: AWKWARD.into(),
            aliases: strs(&["O'Reilly: the 'book'", "Ünïcode — alias"]),
            start: Some("2026-09-03".into()),
            end: Some("2026-10-02".into()),
            members: strs(&["2026-09/E01"]),
            themes: strs(&["A", "B", "C"]),
            origin: Some("model".into()),
            edited: None,
            merged_into: None,
            description: "Line one: with a colon.\n\n--- not a fence\nLine \"two\".".into(),
        }
    );
    // The field order of the spec; the empty fields written empty.
    let raw = fs::read_to_string(out.join("2026-09-gemini-launch.md")).unwrap();
    let keys: Vec<&str> = raw
        .lines()
        .skip(1)
        .take_while(|l| *l != "---")
        .filter(|l| !l.starts_with(' '))
        .map(|l| l.split(':').next().unwrap())
        .collect();
    assert_eq!(
        keys,
        [
            "id",
            "name",
            "aliases",
            "start",
            "end",
            "members",
            "themes",
            "origin",
            "edited",
            "merged_into"
        ]
    );
    assert!(
        raw.ends_with("\nedited:\nmerged_into:\n---\n\nAbout Gemini launch.\n"),
        "{raw}"
    );
    // A quarter event: its id from the quarter's first month; no dated
    // item, so the quarter's first and last day.
    let q = rec(&set, "2022-04-quarter-story");
    assert_eq!(
        (q.start.as_deref(), q.end.as_deref(), q.members.clone()),
        (
            Some("2022-04-01"),
            Some("2022-06-30"),
            strs(&["2022-Q2/E01"])
        )
    );
    // Tag rows naming an event no briefing file holds: named from the
    // row, else by the monthly id; no description.
    assert_eq!(
        rec(&set, "2026-09-only-in-the-tag-rows").name,
        "Only in the tag rows"
    );
    let bare = rec(&set, "2026-08-2026-08-e01");
    assert_eq!(
        (bare.name.as_str(), bare.description.as_str()),
        ("2026-08/E01", "")
    );
    let all = format!("{:?}", set.records);
    assert!(!all.contains("OLD NAME") && !all.contains("superseded folder"));
    let c = &r.collisions;
    assert_eq!(c.len(), 1);
    assert_eq!(
        (c[0].id.as_str(), c[0].base.as_str(), c[0].member.as_str()),
        (
            "2026-09-gemini-launch-2",
            "2026-09-gemini-launch",
            "2026-09/E03"
        )
    );
    assert_eq!(r.fallback_slugs.len(), 1);
    assert_eq!(r.fallback_slugs[0].id, "2026-09-event-e10");
    assert_eq!(r.no_briefing_entry, ["2026-08/E01", "2026-09/E99"]);
    // The counts.
    let v = serde_json::to_value(&r).unwrap();
    for (k, want) in [
        ("monthly_events", 10),
        ("briefing_events", 8),
        ("monthly_events_with_items", 9),
        ("briefing_events_without_items", 1),
        ("already_in_a_record", 0),
        ("to_create", 10),
        ("no_items", 1),
        ("one_item", 8),
        ("created", 10),
        ("skipped", 0),
    ] {
        assert_eq!(v[k], json!(want), "{k}");
    }
    assert_eq!(
        v["notes"],
        json!({"collisions": 1, "fallback_slugs": 1, "truncated_slugs": 1, "non_ascii_names": 2,
               "no_briefing_entry": 2, "month_dates": 2, "briefing_events_without_items": 1})
    );
    assert_eq!(
        (v["tool"].as_str(), v["apply"].as_bool()),
        (Some("scout events seed"), Some(true))
    );
    // Nothing but records is in the folder, and no file is left elsewhere.
    assert_eq!(names(&out).len(), 10);
    assert!(names(&out).iter().all(|n| n.ends_with(".md")));
}

#[test]
fn an_event_without_tagged_items_is_dated_by_its_month_or_quarter() {
    let a = archive();
    put_briefing(
        a.path(),
        "2023-Q4",
        &[(
            "E07",
            "Quiet quarter",
            Some("Nobody saved anything."),
            &["QQ"],
        )],
    );
    put_briefing(a.path(), "2024-02", &[("E01", "Leap month", None, &[])]);
    let (r, set) = applied(a.path());
    assert_eq!((r.briefing_events, r.created, r.no_items), (10, 12, 3));
    assert_eq!(r.notes.briefing_events_without_items, 3);
    assert_eq!(
        rec(&set, "2026-09-nobody-tagged-this"),
        &EventRecord {
            id: "2026-09-nobody-tagged-this".into(),
            name: "Nobody tagged this".into(),
            aliases: vec![],
            start: Some("2026-09-01".into()),
            end: Some("2026-09-30".into()),
            members: strs(&["2026-09/E04"]),
            themes: vec![],
            origin: Some("model".into()),
            edited: None,
            merged_into: None,
            description: "About Nobody tagged this.".into(),
        }
    );
    let q = rec(&set, "2023-10-quiet-quarter");
    assert_eq!(
        (
            q.aliases.clone(),
            q.start.as_deref(),
            q.end.as_deref(),
            q.members.clone()
        ),
        (
            strs(&["QQ"]),
            Some("2023-10-01"),
            Some("2023-12-31"),
            strs(&["2023-Q4/E07"])
        )
    );
    let leap = rec(&set, "2024-02-leap-month");
    assert_eq!(
        (leap.start.as_deref(), leap.end.as_deref()),
        (Some("2024-02-01"), Some("2024-02-29"))
    );
    // Every briefing event is a member of exactly one record.
    let members: Vec<&String> = set.records.values().flat_map(|r| &r.members).collect();
    let mut once = members.clone();
    once.sort();
    once.dedup();
    assert_eq!(once.len(), members.len());
    for id in [
        "2026-09/E04",
        "2023-Q4/E07",
        "2024-02/E01",
        "2026-09/E01",
        "2022-Q2/E01",
    ] {
        assert!(set.resolve(id).is_some(), "{id}");
    }
    // Once it has a record, a second run leaves it alone, items or not.
    let again = seed(&opts(a.path(), None, true)).unwrap();
    assert_eq!(
        (again.created, again.to_create, again.already_in_a_record),
        (0, 0, 12)
    );
}

#[test]
fn a_second_run_writes_no_record_and_changes_none() {
    let a = archive();
    let out = a.path().join("analysis/events");
    applied(a.path());
    let first = tree(&out);
    for apply in [true, false, true] {
        let r = seed(&opts(a.path(), None, apply)).unwrap();
        assert_eq!((r.created, r.to_create), (0, 0));
        assert_eq!((r.already_in_a_record, r.skipped), (10, 10));
        assert_eq!(tree(&out), first);
    }
    assert_eq!(first.len(), 10);
}

#[test]
fn a_record_he_changed_is_never_rewritten_and_its_members_are_not_seeded_again() {
    let a = archive();
    let out = a.path().join("analysis/events");
    // He holds two monthly events in one record of his own, under an id
    // the seed would have used.
    let mine = put(
        &out,
        "2026-09-gemini-launch.md",
        "---\nid: 2026-09-gemini-launch\nname: \"Gemini: my name\"\nmembers: [2026-09/E02, 2026-10/E01]\norigin: dominik\nedited: 2026-10-11\n---\n\nMy words.\n",
    );
    let before = fs::read(&mine).unwrap();
    let (r, set) = applied(a.path());
    assert_eq!(fs::read(&mine).unwrap(), before);
    assert_eq!((r.already_in_a_record, r.created), (2, 8));
    assert!(!set.records.contains_key("2026-10-gemini-launch"));
    // 2026-09/E03 slugs to the id his record has: it takes -2.
    assert_eq!(
        rec(&set, "2026-09-gemini-launch-2").members,
        ["2026-09/E03"]
    );
    assert_eq!(set.resolve("2026-10/E01"), Some("2026-09-gemini-launch"));
    // A monthly event arriving later is added without touching the rest.
    let all = tree(&out);
    let mut layer = fs::read_to_string(a.path().join(LAYER)).unwrap();
    layer.push_str(&format!(
        "{}\n",
        row("x:20", Some("2026-10/E02"), Some("2026-10-10"), &[])
    ));
    put(a.path(), LAYER, &layer);
    let again = seed(&opts(a.path(), None, true)).unwrap();
    assert_eq!(again.created, 1);
    let now = tree(&out);
    assert_eq!(now.len(), all.len() + 1);
    for (p, bytes) in &all {
        assert_eq!(&now[p], bytes, "{}", p.display());
    }
    read_all(&out);
}

#[test]
fn awkward_names_reach_the_reader_as_the_briefing_has_them() {
    // Name in the briefing -> name the record must be read with (runs of
    // whitespace, line ends among them, are one space in a name).
    let cases: Vec<(&str, &str)> = vec![
        ("\"quoted\" and 'single'", "\"quoted\" and 'single'"),
        ("key: value", "key: value"),
        ("ends with a colon:", "ends with a colon:"),
        ("# hash first", "# hash first"),
        ("- leading dash", "- leading dash"),
        ("---", "---"),
        ("!bang", "!bang"),
        ("!!binary YQ==", "!!binary YQ=="),
        ("&anchor", "&anchor"),
        ("*star", "*star"),
        ("line one\nline two\r\nthree", "line one line two three"),
        ("  padded\t name  ", "padded name"),
        ("emoji 🎉 👩‍👩‍👧", "emoji 🎉 👩‍👩‍👧"),
        ("back\\slash \\n", "back\\slash \\n"),
        ("nul\u{0}and bell\u{7}", "nul\u{0}and bell\u{7}"),
        ("bom\u{feff}in", "bom\u{feff}in"),
        ("123", "123"),
        ("1e3", "1e3"),
        ("true", "true"),
        ("no", "no"),
        ("null", "null"),
        ("~", "~"),
        ("2026-10-11", "2026-10-11"),
        ("[a, b]", "[a, b]"),
        ("{a: b}", "{a: b}"),
        ("| block", "| block"),
        ("> fold", "> fold"),
        ("%YAML 1.2", "%YAML 1.2"),
        ("日本語 — ünï", "日本語 — ünï"),
    ];
    let tmp = tempfile::tempdir().unwrap();
    let ids: Vec<String> = (1..=cases.len()).map(|i| format!("E{i:02}")).collect();
    let events: Vec<Value> = cases
        .iter()
        .zip(&ids)
        .map(|((name, _), id)| json!({"id": id, "name": name, "aliases": [name], "description": name}))
        .collect();
    put(
        tmp.path(),
        "analysis/context/briefings/2026-09-events.json",
        &json!({"month": "2026-09", "events": events}).to_string(),
    );
    let (r, set) = applied(tmp.path());
    assert_eq!(r.created, cases.len());
    for ((raw, want), id) in cases.iter().zip(&ids) {
        let member = format!("2026-09/{id}");
        let rec = rec(&set, set.resolve(&member).unwrap());
        assert_eq!(rec.name, *want, "{raw:?}");
        assert_eq!(rec.aliases, [want.to_string()], "{raw:?}");
        assert_eq!(rec.description, raw.trim(), "{raw:?}");
        assert_eq!(rec.members, [member]);
    }
    // Idempotent over these too.
    assert_eq!(seed(&opts(tmp.path(), None, true)).unwrap().created, 0);
}

#[test]
fn names_that_try_to_leave_the_folder_give_plain_file_names_inside_it() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    put_briefing(
        root,
        "2026-09",
        &[
            ("E01", "../../../outside", None, &[]),
            ("E02", "/etc/passwd", None, &[]),
            ("E03", "..", None, &[]),
            ("E04", "a\\..\\b\u{0}c", None, &[]),
        ],
    );
    let out = root.join("deep/out");
    let r = seed(&opts(root, Some(&out), true)).unwrap();
    assert_eq!(r.created, 4);
    let set = read_all(&out);
    let ids: Vec<&str> = set.records.keys().map(String::as_str).collect();
    assert_eq!(
        ids,
        [
            "2026-09-a-b-c",
            "2026-09-etc-passwd",
            "2026-09-event-e03",
            "2026-09-outside"
        ]
    );
    assert_eq!(rec(&set, "2026-09-a-b-c").name, "a\\..\\b\u{0}c");
    let stray: Vec<_> = tree(root)
        .into_keys()
        .filter(|p| !p.starts_with(root.join("analysis")) && !p.starts_with(&out))
        .collect();
    assert!(stray.is_empty(), "{stray:?}");
}

#[test]
fn out_takes_the_records_and_the_archive_is_untouched() {
    let a = archive();
    let before = tree(a.path());
    let scratch = tempfile::tempdir().unwrap();
    let real = scratch.path().join("real");
    fs::create_dir(&real).unwrap();
    let r = seed(&opts(a.path(), Some(&real.join("x").join("..")), true)).unwrap();
    assert_eq!(r.created, 10);
    // Links in the folders above (the temp folder's own, on macOS) are
    // resolved; the report names the real folder.
    assert_eq!(PathBuf::from(&r.out), fs::canonicalize(&real).unwrap());
    assert_eq!(tree(a.path()), before);
    assert_eq!(read_all(&real).records.len(), 10);
    assert_eq!(names(&real).len(), 10);
    assert_eq!(names(scratch.path()), ["real"]);
    // A missing out folder is made, with the folders above it, but not by
    // a dry run.
    let deep = scratch.path().join("a/b");
    assert_eq!(
        seed(&opts(a.path(), Some(&deep), false)).unwrap().to_create,
        10
    );
    assert!(!scratch.path().join("a").exists());
    assert_eq!(
        seed(&opts(a.path(), Some(&deep), true)).unwrap().created,
        10
    );
    assert_eq!(read_all(&deep).records.len(), 10);
    assert_eq!(tree(a.path()), before);
}

/// The whole real archive, when `SCOUT_SEED_REAL_ARCHIVE` names it: every
/// file written is a record the reader reads, one per monthly event, and a
/// second run writes none. Records go to a temporary folder; the archive
/// is only read.
#[test]
fn on_the_real_archive_every_written_file_is_read_by_the_reader() {
    let Some(archive) = std::env::var_os("SCOUT_SEED_REAL_ARCHIVE") else {
        eprintln!("SCOUT_SEED_REAL_ARCHIVE not set: skipped");
        return;
    };
    let archive = PathBuf::from(archive);
    let scratch = tempfile::tempdir().unwrap();
    let out = scratch.path().join("events");
    let r = seed(&opts(&archive, Some(&out), true)).unwrap();
    assert!(r.created > 0 && r.created == r.to_create);
    let set = read_all(&out);
    assert_eq!(set.records.len(), r.created);
    assert_eq!(names(&out).len(), r.created);
    assert_eq!(set.report().members, r.created);
    assert!(set.warnings.is_empty(), "{:?}", set.warnings);
    for rec in set.records.values() {
        let bytes = fs::read(out.join(format!("{}.md", rec.id))).unwrap();
        assert_eq!(&events::parse_bytes(&bytes, &rec.id).unwrap(), rec);
        assert_eq!(set.resolve(&rec.members[0]), Some(rec.id.as_str()));
    }
    let again = seed(&opts(&archive, Some(&out), true)).unwrap();
    assert_eq!((again.created, again.already_in_a_record), (0, r.created));
}

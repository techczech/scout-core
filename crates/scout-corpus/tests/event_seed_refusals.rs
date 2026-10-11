//! `scout events seed`: the two things it must never do. It never writes
//! a record the reader would skip (nor counts on one already there), and
//! it never writes anywhere but a new file of the out folder. Each refusal
//! leaves every file as it was.

mod event_seed_support;
use event_seed_support::*;

#[test]
fn a_record_the_reader_skips_stops_the_run_with_the_readers_reason() {
    for (name, text, why) in [
        ("broken.md", "---\nid: [unclosed\n---\n", "bad YAML"),
        ("no-frontmatter.md", "just prose\n", "no frontmatter"),
        (
            "other-name.md",
            "---\nid: something-else\nmembers: []\n---\n",
            "id \"something-else\" does not match the file name",
        ),
        (
            "bad-members.md",
            "---\nid: bad-members\nmembers: 2026-09/E01\n---\n",
            "members is not a list",
        ),
        (
            "name-list.md",
            "---\nid: name-list\nname: [a, b]\nmembers: [2026-09/E01]\n---\n",
            "name is not a single value",
        ),
        // An explicit YAML tag is not applied by the reader: the id is the
        // text as written, which is not the file's name.
        (
            "a.md",
            "---\nid: !!binary YQ==\nmembers: [2026-09/E01]\n---\n",
            "id \"YQ==\" does not match the file name",
        ),
        // The whole frontmatter is checked: a valid head that holds the
        // member, a `...` line, then anything.
        (
            "doc-end.md",
            "---\nid: doc-end\nmembers: [2026-09/E01]\n...\nname: [unterminated\n---\n",
            "document end marker",
        ),
        (
            "doc-end-clean.md",
            "---\nid: doc-end-clean\nmembers: [2026-09/E01]\n...\n---\n",
            "document end marker",
        ),
        (
            "two-documents.md",
            "---\nid: two-documents\nmembers: [2026-09/E01]\n--- second\n---\n",
            "more than one document",
        ),
        (
            "alias.md",
            "---\nx: &m [2026-09/E01]\nid: alias\nmembers: *m\n---\n",
            "aliases are not read",
        ),
    ] {
        for apply in [true, false] {
            let a = archive();
            let out = a.path().join("analysis/events");
            let file = put(&out, name, text);
            let before = tree(a.path());
            let e = seed(&opts(a.path(), None, apply)).unwrap_err().to_string();
            let file = fs::canonicalize(file).unwrap();
            assert!(
                e.contains(&format!("{}: cannot read the record (", file.display())),
                "{name}: {e}"
            );
            assert!(e.contains(why), "{name}: {e}");
            assert!(e.contains("nothing was written"), "{name}: {e}");
            assert_eq!(tree(a.path()), before, "{name}");
        }
    }
    // Two files with one id: the second is named too. Too large a file.
    let a = archive();
    let out = a.path().join("analysis/events");
    put(&out, "twin  id.md", "---\nid: twin  id\n---\n");
    put(&out, "twin id.md", "---\nid: twin id\n---\n");
    put(
        &out,
        "big.md",
        &format!("---\nid: big\n---\n{}", "x".repeat(1 << 20)),
    );
    let before = tree(a.path());
    let e = seed(&opts(a.path(), None, true)).unwrap_err().to_string();
    assert!(e.contains("repeated or empty id"), "{e}");
    assert!(
        e.contains("big.md: cannot read the record (larger than 1 MiB)"),
        "{e}"
    );
    assert!(e.contains("these 2 records are not read by scout"), "{e}");
    assert_eq!(tree(a.path()), before);
}

#[test]
fn a_record_too_large_for_the_reader_stops_the_run_before_any_write() {
    // The description alone is exactly the cap; with its frontmatter the
    // file would be over it.
    let huge = "d".repeat(events::MAX_RECORD_BYTES as usize);
    let fill = |root: &Path| {
        put_briefing(
            root,
            "2026-11",
            &[
                ("E01", "Fits", None, &[]),
                ("E02", "Far too long", Some(&huge), &[]),
            ],
        );
    };
    // Into a folder that holds a record and other files: all unchanged.
    let a = archive();
    fill(a.path());
    let out = a.path().join("analysis/events");
    put(
        &out,
        "mine.md",
        "---\nid: mine\nmembers: [2026-09/E02]\n---\n",
    );
    put(&out, "notes.txt", "keep\n");
    let before = tree(a.path());
    for apply in [true, false] {
        let e = seed(&opts(a.path(), None, apply)).unwrap_err().to_string();
        assert!(
            e.contains("2026-11/E02 (\"Far too long\"): scout would not read its record 2026-11-far-too-long.md (larger than 1 MiB)"),
            "{e}"
        );
        assert!(e.contains("nothing was written"), "{e}");
        assert!(!e.contains("2026-11/E01"), "{e}");
        assert_eq!(tree(a.path()), before);
    }
    // Into a folder that is not there: it is not made.
    let a = archive();
    fill(a.path());
    let scratch = tempfile::tempdir().unwrap();
    let out = scratch.path().join("new/out");
    assert!(seed(&opts(a.path(), Some(&out), true)).is_err());
    assert!(names(scratch.path()).is_empty());
    // One byte under: a record of exactly the cap is written and read.
    let a = archive();
    let fits = "---\nid: \"2026-11-at-the-cap\"\nname: \"At the cap\"\naliases: []\nstart: \"2026-11-01\"\nend: \"2026-11-30\"\nmembers:\n  - \"2026-11/E01\"\nthemes: []\norigin: \"model\"\nedited:\nmerged_into:\n---\n\n\n";
    let room = events::MAX_RECORD_BYTES as usize - fits.len();
    put_briefing(
        a.path(),
        "2026-11",
        &[("E01", "At the cap", Some(&"d".repeat(room)), &[])],
    );
    let (r, set) = applied(a.path());
    assert_eq!(r.created, 11);
    let at = a.path().join("analysis/events/2026-11-at-the-cap.md");
    assert_eq!(fs::metadata(at).unwrap().len(), events::MAX_RECORD_BYTES);
    assert_eq!(rec(&set, "2026-11-at-the-cap").description.len(), room);
}

#[cfg(unix)]
#[test]
fn an_out_folder_that_is_a_symlink_or_not_a_folder_is_refused_before_anything() {
    use std::os::unix::fs::symlink;
    let a = archive();
    let scratch = tempfile::tempdir().unwrap();
    let s = scratch.path();
    let real = s.join("real");
    fs::create_dir(&real).unwrap();
    symlink(&real, s.join("link")).unwrap();
    symlink(s.join("nowhere"), s.join("dangling")).unwrap();
    let file = put(s, "file", "keep\n");
    for apply in [true, false] {
        for (out, why) in [
            (s.join("link"), "the out folder is a symlink"),
            (s.join("link/../link"), "the out folder is a symlink"),
            (s.join("dangling"), "the out folder is a symlink"),
            (file.clone(), "the out folder is not a folder"),
        ] {
            let e = seed(&opts(a.path(), Some(&out), apply)).unwrap_err();
            assert!(e.to_string().contains(why), "{}: {e}", out.display());
        }
    }
    assert!(names(&real).is_empty());
    assert_eq!(names(s), ["dangling", "file", "link", "real"]);
    assert_eq!(fs::read_to_string(&file).unwrap(), "keep\n");
    // A symlink in a folder ABOVE the out folder is accepted.
    let r = seed(&opts(a.path(), Some(&s.join("link/sub")), true)).unwrap();
    assert_eq!(r.created, 10);
    assert_eq!(
        PathBuf::from(&r.out),
        fs::canonicalize(&real).unwrap().join("sub")
    );
    assert_eq!(read_all(&real.join("sub")).records.len(), 10);
    // The archive's own events folder as a link out of the archive.
    let elsewhere = tempfile::tempdir().unwrap();
    symlink(elsewhere.path(), a.path().join("analysis/events")).unwrap();
    for apply in [true, false] {
        let e = seed(&opts(a.path(), None, apply)).unwrap_err();
        assert!(e.to_string().contains("the out folder is a symlink"), "{e}");
    }
    assert!(names(elsewhere.path()).is_empty());
}

#[cfg(unix)]
#[test]
fn links_planted_in_the_out_folder_are_left_alone_and_nothing_outside_changes() {
    use std::os::unix::fs::{symlink, MetadataExt};
    let a = archive();
    let out = a.path().join("analysis/events");
    fs::create_dir_all(&out).unwrap();
    let outside = tempfile::tempdir().unwrap();
    let o = outside.path();
    let victim = put(o, "victim.txt", "keep\n");
    let victim2 = put(o, "victim2.txt", "keep 2\n");
    // Under names that are not records: a hard link and two symlinks.
    fs::hard_link(&victim, out.join("seed-log.jsonl")).unwrap();
    symlink(&victim2, out.join("seed-log.json")).unwrap();
    symlink(o, out.join("elsewhere")).unwrap();
    // Under the names of records this run wants to create.
    symlink(&victim2, out.join("2026-10-gemini-launch.md")).unwrap();
    symlink(o.join("not-there.md"), out.join("2022-04-quarter-story.md")).unwrap();
    fs::hard_link(&victim, out.join("2026-09-gemini-launch-2.md")).unwrap();
    let before = (tree(o), names(&out), names(o));
    // A record name that is a link is an existing record the reader does
    // not read: the run stops, naming all three, with nothing written.
    for apply in [true, false] {
        let e = seed(&opts(a.path(), None, apply)).unwrap_err().to_string();
        for why in [
            "2026-10-gemini-launch.md: cannot read the record (not a plain file)",
            "2022-04-quarter-story.md: cannot read the record (not a plain file)",
            "2026-09-gemini-launch-2.md: cannot read the record (no frontmatter)",
            "these 3 records are not read by scout",
        ] {
            assert!(e.contains(why), "{why}\n{e}");
        }
        assert_eq!((tree(o), names(&out), names(o)), before);
    }
    // With only the links under other names left, the run goes through
    // and touches none of them.
    for f in [
        "2026-10-gemini-launch.md",
        "2022-04-quarter-story.md",
        "2026-09-gemini-launch-2.md",
    ] {
        fs::remove_file(out.join(f)).unwrap();
    }
    let r = seed(&opts(a.path(), None, true)).unwrap();
    assert_eq!(r.created, 10);
    assert_eq!(tree(o), before.0, "nothing outside the out folder changed");
    assert_eq!(
        names(o),
        before.2,
        "nothing was created through the folder link"
    );
    assert_eq!(fs::metadata(&victim).unwrap().nlink(), 2);
    let others: Vec<String> = names(&out)
        .into_iter()
        .filter(|n| !n.ends_with(".md"))
        .collect();
    assert_eq!(others, ["elsewhere", "seed-log.json", "seed-log.jsonl"]);
    let set = read_all(&out);
    assert_eq!(set.records.len(), 10);
    for id in set.records.keys() {
        let m = fs::symlink_metadata(out.join(format!("{id}.md"))).unwrap();
        assert!(m.is_file() && m.nlink() == 1, "{id} is a file of its own");
    }
}

/// A hard-linked regular file is read like any other: a valid record
/// under a record name counts as one (there is no link-count check), and
/// neither it nor the file it shares its bytes with is written.
#[cfg(unix)]
#[test]
fn a_hard_linked_valid_record_is_read_as_a_record_and_never_written() {
    use std::os::unix::fs::MetadataExt;
    let a = archive();
    let out = a.path().join("analysis/events");
    fs::create_dir_all(&out).unwrap();
    let outside = tempfile::tempdir().unwrap();
    let text = "---\nid: mine\nmembers: [2026-09/E02]\n---\n";
    let original = put(outside.path(), "kept-elsewhere.md", text);
    fs::hard_link(&original, out.join("mine.md")).unwrap();
    let r = seed(&opts(a.path(), None, true)).unwrap();
    assert_eq!((r.already_in_a_record, r.created), (1, 9));
    let set = read_all(&out);
    assert_eq!(set.resolve("2026-09/E02"), Some("mine"));
    assert_eq!(fs::read_to_string(&original).unwrap(), text);
    assert_eq!(fs::metadata(&original).unwrap().nlink(), 2);
    assert_eq!(names(outside.path()), ["kept-elsewhere.md"]);
}

#[test]
fn a_name_in_use_under_another_case_or_ending_is_not_taken_again() {
    // `UPPER.MD` is not a record to the reader, and on a file system that
    // ignores case it is where `upper.md` would land.
    let tmp = tempfile::tempdir().unwrap();
    put_briefing(tmp.path(), "2026-09", &[("E01", "Story", None, &[])]);
    let out = tmp.path().join("analysis/events");
    put(&out, "2026-09-STORY.MD", "not a record\n");
    put(&out, "2026-09-story-2", "a file without an ending\n");
    let before = tree(&out);
    let (r, set) = applied(tmp.path());
    assert_eq!(r.created, 1);
    assert_eq!(rec(&set, "2026-09-story-3").members, ["2026-09/E01"]);
    for (p, bytes) in &before {
        assert_eq!(&fs::read(p).unwrap(), bytes);
    }
    assert_eq!(seed(&opts(tmp.path(), None, true)).unwrap().created, 0);
}

#[test]
fn a_briefing_file_that_is_not_one_stops_the_run() {
    let cases: Vec<(&str, String, &str)> = vec![
        ("2026-09-events.json", "{not json".into(), "not JSON"),
        (
            "2026-09-events.json",
            briefing("2026-10", &[("E01", "x", None, &[])]),
            "\"month\" is \"2026-10\", the file name says 2026-09",
        ),
        (
            "2026-09-events.json",
            "{\"month\":\"2026-09\"}".into(),
            "no \"events\" list",
        ),
        (
            "2026-09-events.json",
            briefing("2026-09", &[("1", "x", None, &[])]),
            "event id \"1\" is not E<number>",
        ),
        (
            "2026-09-events.json",
            briefing(
                "2026-09",
                &[("E01", "x", None, &[]), ("E01", "y", None, &[])],
            ),
            "event 2026-09/E01 appears twice",
        ),
    ];
    for (name, text, why) in cases {
        let tmp = tempfile::tempdir().unwrap();
        put(
            tmp.path(),
            &format!("analysis/context/briefings/{name}"),
            &text,
        );
        let before = tree(tmp.path());
        let e = seed(&opts(tmp.path(), None, true)).unwrap_err().to_string();
        assert!(e.contains(why), "{why}: {e}");
        assert_eq!(tree(tmp.path()), before);
    }
    // An archive path that is not a folder is refused, not read as empty.
    let tmp = tempfile::tempdir().unwrap();
    let e = seed(&opts(&tmp.path().join("typo"), None, true)).unwrap_err();
    assert!(e.to_string().contains("the archive is not a folder"), "{e}");
    assert!(names(tmp.path()).is_empty());
    // An archive with neither briefings nor tag rows: nothing to create.
    let r = seed(&opts(tmp.path(), None, true)).unwrap();
    assert_eq!((r.monthly_events, r.created), (0, 0));
}

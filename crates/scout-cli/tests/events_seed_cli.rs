//! CLI seam for `scout events seed`: one JSON line of counts on stdout,
//! nothing written without `--apply`, a refusal as a non-zero exit with
//! the file and the reader's reason on stderr, and an index build that
//! reads every record the seed wrote. It needs no registry and no index.

use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

/// `scout` with a registry and data folder that do not exist.
fn scout(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_scout"))
        .args(args)
        .env("SCOUT_CONFIG", home.join("no-such-corpora.toml"))
        .env("SCOUT_DATA_DIR", home.join("data"))
        .output()
        .unwrap()
}

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    v.sort();
    v
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// An archive with three works, a briefing of three events (one named so
/// that only quoting keeps it text) and tag rows for two of them.
fn archive(root: &Path) {
    for id in 1..=3 {
        write(
            root,
            &format!("readings/works/w-{id}.md"),
            &format!(
                "---\ntitle: W{id}\nauthor: A\ntype: tweet\nsource_system: x\nsource_id: \"{id}\"\n---\n\n> A proof, number {id}.\n\nhighlighted_at: 2026-09-0{id}\n\n---\n"
            ),
        );
    }
    write(
        root,
        "analysis/context/briefings/2026-09-events.json",
        &json!({"month": "2026-09", "events": [
            {"id": "E01", "name": "Fields: the \"declaration\" #1", "aliases": ["- dash", "null"], "description": "First.\n---\nStill the first."},
            {"id": "E02", "name": "!!binary YQ==", "description": "Second."},
            {"id": "E03", "name": "Nobody tagged this", "description": "Third."},
        ]})
        .to_string(),
    );
    let rows = [
        json!({"key": "x:1", "date": "2026-09-01", "themes": ["Maths"], "entities": [], "event": "2026-09/E01", "event_name": "Fields"}),
        json!({"key": "x:2", "date": "2026-09-02", "themes": ["Maths"], "entities": [], "event": "2026-09/E01", "event_name": "Fields"}),
        json!({"key": "x:3", "date": "2026-09-03", "themes": [], "entities": [], "event": "2026-09/E02", "event_name": "Binary"}),
    ];
    let layer: String = rows.iter().map(|r| format!("{r}\n")).collect();
    write(root, "analysis/topics/current.jsonl", &layer);
}

#[test]
fn seed_prints_one_json_line_writes_only_with_apply_and_the_build_reads_every_record() {
    let tmp = tempfile::tempdir().unwrap();
    let h = tmp.path().join("archive");
    archive(&h);
    let out = h.join("analysis/events");
    let a = h.to_str().unwrap();

    // Dry run: counts, a note on stderr, nothing written.
    let dry = scout(tmp.path(), &["events", "seed", "--archive", a]);
    assert_eq!(dry.status.code(), Some(0), "{}", text(&dry.stderr));
    let line = text(&dry.stdout);
    assert!(line.starts_with('{') && line.ends_with("}\n"), "{line}");
    assert_eq!(line.trim_end().lines().count(), 1);
    let j: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(
        (&j["apply"], &j["to_create"], &j["created"]),
        (&json!(false), &json!(3), &json!(0))
    );
    assert!(
        text(&dry.stderr).contains("dry run: nothing written; 3 records would be created in"),
        "{}",
        text(&dry.stderr)
    );
    assert!(!out.exists());

    // Apply: the counts, and the records and nothing else in the folder.
    let wet = scout(tmp.path(), &["events", "seed", "--archive", a, "--apply"]);
    assert_eq!(wet.status.code(), Some(0), "{}", text(&wet.stderr));
    assert_eq!(text(&wet.stdout).trim_end().lines().count(), 1);
    assert_eq!(text(&wet.stderr), "");
    let j: Value = serde_json::from_slice(&wet.stdout).unwrap();
    let mut keys: Vec<&str> = j.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort();
    let mut want = vec![
        "date",
        "tool",
        "version",
        "archive",
        "out",
        "apply",
        "monthly_events",
        "briefing_events",
        "monthly_events_with_items",
        "briefing_events_without_items",
        "already_in_a_record",
        "to_create",
        "no_items",
        "one_item",
        "created",
        "skipped",
        "notes",
        "collisions",
        "fallback_slugs",
        "no_briefing_entry",
    ];
    want.sort();
    assert_eq!(keys, want);
    assert_eq!(j["tool"], "scout events seed");
    assert_eq!(j["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(j["out"], fs::canonicalize(&out).unwrap().to_str().unwrap());
    for (k, n) in [
        ("monthly_events", 3),
        ("to_create", 3),
        ("created", 3),
        ("skipped", 0),
        ("no_items", 1),
        ("one_item", 1),
    ] {
        assert_eq!(j[k], json!(n), "{k}");
    }
    assert_eq!(
        names(&out),
        [
            "2026-09-binary-yq.md",
            "2026-09-fields-the-declaration-1.md",
            "2026-09-nobody-tagged-this.md"
        ]
    );

    // A second run creates nothing.
    let again = scout(tmp.path(), &["events", "seed", "--archive", a, "--apply"]);
    let j: Value = serde_json::from_slice(&again.stdout).unwrap();
    assert_eq!(
        (
            &j["created"],
            &j["to_create"],
            &j["already_in_a_record"],
            &j["skipped"]
        ),
        (&json!(0), &json!(0), &json!(3), &json!(3))
    );
    assert_eq!(names(&out).len(), 3);

    // The index build reads all three and skips none; the names are the
    // briefing's, whatever YAML could have made of them.
    let cfg = tmp.path().join("corpora.toml");
    fs::write(
        &cfg,
        format!(
            "[[corpus]]\nid = \"highlights\"\nname = \"H\"\nkind = \"highlight-scout-archive\"\npath = \"{}\"\n",
            h.display()
        ),
    )
    .unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_scout"))
            .args(args)
            .env("SCOUT_CONFIG", &cfg)
            .env("SCOUT_DATA_DIR", tmp.path().join("data"))
            .output()
            .unwrap()
    };
    let b = run(&["index", "build", "--json"]);
    assert!(b.status.success(), "{}", text(&b.stderr));
    let j: Value = serde_json::from_slice(&b.stdout).unwrap();
    let e = &j["reports"][0]["topics"]["events"];
    assert_eq!(
        (
            &e["records"],
            &e["skipped"],
            &e["members"],
            &e["warnings_total"]
        ),
        (&json!(3), &json!(0), &json!(3), &json!(0))
    );
    let l: Value =
        serde_json::from_slice(&run(&["events", "--min-items", "0", "--json"]).stdout).unwrap();
    let mut listed: Vec<(String, String, u64)> = l["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["id"].as_str().unwrap().to_string(),
                e["name"].as_str().unwrap().to_string(),
                e["items"].as_u64().unwrap(),
            )
        })
        .collect();
    listed.sort();
    assert_eq!(
        listed,
        [
            (
                "2026-09-binary-yq".to_string(),
                "!!binary YQ==".to_string(),
                1
            ),
            (
                "2026-09-fields-the-declaration-1".to_string(),
                "Fields: the \"declaration\" #1".to_string(),
                2
            ),
            (
                "2026-09-nobody-tagged-this".to_string(),
                "Nobody tagged this".to_string(),
                0
            ),
        ]
    );
}

#[test]
fn a_refusal_is_a_non_zero_exit_with_the_file_and_the_reason_and_no_write() {
    let tmp = tempfile::tempdir().unwrap();
    let h = tmp.path().join("archive");
    archive(&h);
    let a = h.to_str().unwrap();
    // A record the reader skips, already in the out folder.
    let out = tmp.path().join("out");
    write(
        &out,
        "a.md",
        "---\nid: !!binary YQ==\nmembers: [2026-09/E01]\n---\n",
    );
    let o = out.to_str().unwrap();
    for apply in [&["--apply"][..], &[]] {
        let mut args = vec!["events", "seed", "--archive", a, "--out", o];
        args.extend(apply);
        let r = scout(tmp.path(), &args);
        assert_eq!(r.status.code(), Some(2));
        assert_eq!(text(&r.stdout), "");
        let err = text(&r.stderr);
        assert!(
            err.contains("a.md: cannot read the record (id \"YQ==\" does not match the file name)"),
            "{err}"
        );
        assert!(err.contains("nothing was written"), "{err}");
        assert_eq!(names(&out), ["a.md"]);
    }
    // A record the seed would write and the reader would skip.
    write(
        &h,
        "analysis/context/briefings/2026-10-events.json",
        &json!({"month": "2026-10", "events": [
            {"id": "E01", "name": "Too long", "description": "d".repeat(1 << 20)},
        ]})
        .to_string(),
    );
    let fresh = tmp.path().join("fresh");
    let r = scout(
        tmp.path(),
        &[
            "events",
            "seed",
            "--archive",
            a,
            "--out",
            fresh.to_str().unwrap(),
            "--apply",
        ],
    );
    assert_eq!(r.status.code(), Some(2));
    assert_eq!(text(&r.stdout), "");
    let err = text(&r.stderr);
    assert!(
        err.contains("2026-10/E01 (\"Too long\"): scout would not read its record 2026-10-too-long.md (larger than 1 MiB)"),
        "{err}"
    );
    assert!(!fresh.exists());
    // Usage: the archive must be named; an out folder that is a file.
    let r = scout(tmp.path(), &["events", "seed"]);
    assert_eq!(r.status.code(), Some(2));
    assert!(text(&r.stderr).contains("--archive"), "{}", text(&r.stderr));
    let file = tmp.path().join("file");
    fs::write(&file, "keep\n").unwrap();
    let r = scout(
        tmp.path(),
        &[
            "events",
            "seed",
            "--archive",
            a,
            "--out",
            file.to_str().unwrap(),
        ],
    );
    assert_eq!(r.status.code(), Some(2));
    assert!(text(&r.stderr).contains("the out folder is not a folder"));
    assert_eq!(fs::read_to_string(&file).unwrap(), "keep\n");
    // A `..` in either path: refused, dry run too, nothing made.
    let before = names(tmp.path());
    let up_out = tmp.path().join("fresh/../up");
    let up_archive = h.join("analysis/..");
    for apply in [&["--apply"][..], &[]] {
        for (archive, out, why) in [
            (
                a,
                up_out.to_str().unwrap(),
                "the out path has a `..` component",
            ),
            (
                up_archive.to_str().unwrap(),
                fresh.to_str().unwrap(),
                "the archive path has a `..` component",
            ),
        ] {
            let mut args = vec!["events", "seed", "--archive", archive, "--out", out];
            args.extend(apply);
            let r = scout(tmp.path(), &args);
            assert_eq!(r.status.code(), Some(2));
            assert_eq!(text(&r.stdout), "");
            assert!(text(&r.stderr).contains(why), "{}", text(&r.stderr));
        }
    }
    assert_eq!(names(tmp.path()), before);
    assert!(!h.join("analysis/events").exists());
}

/// A refusal of several faults is several lines, and the only line
/// breaks are the ones between the faults: a line feed in a file name
/// starts no line.
#[cfg(unix)]
#[test]
fn a_refusal_of_several_faults_breaks_lines_only_between_the_faults() {
    let tmp = tempfile::tempdir().unwrap();
    let h = tmp.path().join("archive");
    archive(&h);
    let out = tmp.path().join("out");
    let bad = "---\nid: !!binary YQ==\nmembers: [2026-09/E01]\n---\n";
    write(&out, "a.md", bad);
    write(&out, "b\nFAKE: all records are fine.md", bad);
    let r = scout(
        tmp.path(),
        &[
            "events",
            "seed",
            "--archive",
            h.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ],
    );
    assert_eq!(r.status.code(), Some(2));
    assert_eq!(text(&r.stdout), "");
    let err = text(&r.stderr);
    let lines: Vec<&str> = err.lines().collect();
    assert_eq!(lines.len(), 3, "{err:?}");
    assert!(lines[0].starts_with("scout: "), "{err:?}");
    assert!(
        lines[..2]
            .iter()
            .all(|l| l.contains("cannot read the record")),
        "{err:?}"
    );
    assert!(
        err.contains("b\u{fffd}FAKE: all records are fine.md: cannot read the record"),
        "{err:?}"
    );
    assert_eq!(
        lines[2],
        "these 2 records are not read by scout; fix or remove before seeding (nothing was written)"
    );
}

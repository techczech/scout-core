//! CLI seam for `scout briefings list|show`: one JSON line on stdout that
//! is the library's value, a refusal as exit 2 with the reason on stderr
//! and nothing on stdout, and no file of the archive changed. It needs no
//! registry and no index.

use scout_corpus::briefings::{briefing, briefings};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
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

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Every file under `root` with its bytes.
fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).unwrap() {
            let e = e.unwrap();
            if e.file_type().unwrap().is_dir() {
                stack.push(e.path());
            } else {
                out.insert(e.path(), fs::read(e.path()).unwrap());
            }
        }
    }
    out
}

/// Two briefings (a month with two events, a quarter with none), one file
/// that is no briefing, and a record that holds the month's first event.
fn archive(root: &Path) {
    let dir = "analysis/context/briefings";
    write(
        root,
        &format!("{dir}/2026-09-briefing.json"),
        &json!({
            "meta": {"month": "2026-09", "model": "test-model", "generated": "2026-10-01T08:00:00.000Z", "items": 40},
            "overview": "A month.",
            "events": [
                {"id": "E01", "name": "Launch\u{1b}[31m", "date": "2026-09-03", "aliases": ["the launch"], "description": "It launched.", "items": ["m0001"]},
                {"id": "E02", "name": "Letter", "date": "2026-09-10", "aliases": [], "description": "A letter.", "items": []},
            ],
            "names": [{"name": "Thing", "aliases": ["T"], "note": "a thing"}],
            "debates": [{"title": "Debate", "summary": "Two sides.", "when": "late"}],
            "engagement": "Mostly the launch.",
        })
        .to_string(),
    );
    write(
        root,
        &format!("{dir}/2022-Q2-briefing.json"),
        &json!({"meta": {"month": "2022-Q2"}, "events": []}).to_string(),
    );
    write(root, &format!("{dir}/2026-10-briefing.json"), "[]");
    write(
        root,
        "analysis/events/2026-09-launch.md",
        "---\nid: 2026-09-launch\nname: The launch\nmembers: [2026-09/E01]\n---\n\nIt launched.\n",
    );
}

#[test]
fn list_and_show_print_the_librarys_value_as_one_json_line_and_change_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let h = tmp.path().join("archive");
    archive(&h);
    let a = h.to_str().unwrap();
    let before = tree(&h);

    let list = scout(tmp.path(), &["briefings", "list", "--archive", a, "--json"]);
    assert!(list.status.success(), "{}", text(&list.stderr));
    let out = text(&list.stdout);
    assert_eq!(out.lines().count(), 1, "{out}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        v["briefings"],
        serde_json::to_value(briefings(&h).unwrap()).unwrap()
    );
    let rows = v["briefings"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert!(rows[0]["error"]
        .as_str()
        .unwrap()
        .ends_with("2026-10-briefing.json: not a JSON object"));
    assert_eq!(
        rows[1],
        json!({"period": "2026-09", "kind": "month", "model": "test-model",
               "generated": "2026-10-01T08:00:00.000Z", "items": 40, "events": 2})
    );
    assert_eq!(
        rows[2],
        json!({"period": "2022-Q2", "kind": "quarter", "model": "", "generated": "", "items": null, "events": 0})
    );
    // Without --json the output is the same line.
    let plain = scout(tmp.path(), &["briefings", "list", "--archive", a]);
    assert_eq!(plain.stdout, list.stdout);

    let show = scout(
        tmp.path(),
        &["briefings", "show", "2026-09", "--archive", a, "--json"],
    );
    assert!(show.status.success(), "{}", text(&show.stderr));
    let out = text(&show.stdout);
    assert_eq!(out.lines().count(), 1, "{out}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        v,
        serde_json::to_value(briefing(&h, "2026-09").unwrap()).unwrap()
    );
    assert_eq!(
        v["events"],
        json!([
            {"monthly_id": "2026-09/E01", "name": "Launch[31m", "date": "2026-09-03", "aliases": ["the launch"],
             "description": "It launched.", "record": {"id": "2026-09-launch", "name": "The launch", "removed": false}},
            {"monthly_id": "2026-09/E02", "name": "Letter", "date": "2026-09-10", "aliases": [],
             "description": "A letter.", "record": null},
        ])
    );
    assert!(!out.contains("m0001"));
    assert_eq!(tree(&h), before);
    assert!(!tmp.path().join("data").exists());
}

#[test]
fn a_refusal_is_exit_2_with_one_line_on_stderr_and_nothing_on_stdout() {
    let tmp = tempfile::tempdir().unwrap();
    let h = tmp.path().join("archive");
    archive(&h);
    let a = h.to_str().unwrap();
    fs::remove_file(h.join("analysis/events/2026-09-launch.md")).unwrap();
    fs::remove_dir(h.join("analysis/events")).unwrap();
    for (period, why) in [
        (
            "../2026-09",
            "scout: \"../2026-09\" is not a month (YYYY-MM) or a quarter (YYYY-Qn).\n".to_string(),
        ),
        // A line feed in the argument does not start a line of its own.
        (
            "2026\n09",
            "scout: \"2026\u{fffd}09\" is not a month (YYYY-MM) or a quarter (YYYY-Qn).\n"
                .to_string(),
        ),
        ("2026-08", "scout: No briefing for 2026-08.\n".to_string()),
        (
            "2026-10",
            format!(
                "scout: {}: not a JSON object\n",
                h.join("analysis/context/briefings/2026-10-briefing.json")
                    .display()
            ),
        ),
        // Events, and no records folder to link them through.
        (
            "2026-09",
            format!(
                "scout: {}: no records folder (run `scout events seed` first)\n",
                h.join("analysis/events").display()
            ),
        ),
    ] {
        let o = scout(
            tmp.path(),
            &["briefings", "show", period, "--archive", a, "--json"],
        );
        assert_eq!(o.status.code(), Some(2), "{period}");
        assert!(o.stdout.is_empty(), "{period}");
        assert_eq!(text(&o.stderr), why);
    }
    // A briefing without events needs no records folder.
    let o = scout(
        tmp.path(),
        &["briefings", "show", "2022-Q2", "--archive", a],
    );
    assert!(o.status.success(), "{}", text(&o.stderr));
}

//! CLI seam for the record edits of `scout events`: each command prints
//! one JSON line on stdout and exits 0; a refusal exits 2 with the reason
//! on stderr, nothing on stdout and no file changed; `check` exits 1 when
//! the log and the records disagree. They need no registry and no index.

use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const ALPHA: &str = "2026-09-alpha-story";
const BETA: &str = "2026-10-beta";

struct Fx {
    tmp: tempfile::TempDir,
    archive: PathBuf,
    dir: PathBuf,
}

impl Fx {
    fn new() -> Fx {
        let tmp = tempfile::tempdir().unwrap();
        let archive = tmp.path().join("archive");
        let dir = archive.join("analysis/events");
        fs::create_dir_all(&dir).unwrap();
        // The folder as the commands name it (symlinks above it resolved).
        let dir = dir.canonicalize().unwrap();
        Fx { tmp, archive, dir }
    }

    /// `scout events <args> --archive <archive>`.
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_scout"))
            .arg("events")
            .args(args)
            .arg("--archive")
            .arg(&self.archive)
            .env("SCOUT_CONFIG", self.tmp.path().join("no-such-corpora.toml"))
            .env("SCOUT_DATA_DIR", self.tmp.path().join("data"))
            .output()
            .unwrap()
    }

    /// The one JSON line of a command that must succeed.
    fn ok(&self, args: &[&str]) -> Value {
        let out = self.run(args);
        let (stdout, stderr) = (text(&out.stdout), text(&out.stderr));
        assert_eq!(out.status.code(), Some(0), "{args:?}: {stderr}");
        assert_eq!(stdout.lines().count(), 1, "{args:?}: {stdout}");
        serde_json::from_str(&stdout).unwrap()
    }

    /// A command that must be refused: exit 2, the reason on stderr,
    /// nothing on stdout, no file changed.
    fn refused(&self, args: &[&str], why: &str) {
        let before = self.tree();
        let out = self.run(args);
        let stderr = text(&out.stderr);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {stderr}");
        assert!(out.stdout.is_empty(), "{args:?}: {}", text(&out.stdout));
        assert!(stderr.contains(why), "{args:?}: want {why:?} in {stderr}");
        assert!(self.tree() == before, "{args:?}: a file changed");
    }

    fn tree(&self) -> BTreeMap<PathBuf, Vec<u8>> {
        let mut out = BTreeMap::new();
        let mut stack = vec![self.archive.clone()];
        while let Some(d) = stack.pop() {
            for e in fs::read_dir(&d).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    out.insert(p.clone(), fs::read(&p).unwrap());
                }
            }
        }
        out
    }

    fn lines(&self, name: &str) -> Vec<Value> {
        fs::read_to_string(self.dir.join(name))
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn file_of(dir: &Path, id: &str) -> String {
    dir.join(format!("{id}.md")).display().to_string()
}

#[test]
fn every_edit_prints_one_json_line_and_a_refusal_exits_2_with_nothing_changed() {
    let fx = Fx::new();
    // create
    let a = fx.ok(&[
        "create",
        "--name",
        "Alpha story",
        "--start",
        "2026-09-05",
        "--end",
        "2026-09-20",
        "--description",
        "First words.",
    ]);
    assert_eq!(
        (a["tool"].as_str(), a["id"].as_str(), a["changed"].as_bool()),
        (Some("scout events create"), Some(ALPHA), Some(true))
    );
    assert_eq!(a["file"], json!(file_of(&fx.dir, ALPHA)));
    assert_eq!(
        (&a["record"]["origin"], &a["record"]["members"]),
        (&json!("dominik"), &json!([]))
    );
    assert!(a["record"].get("removed").is_none());
    let b = fx.ok(&["create", "--name", "Beta", "--start", "2026-10-02"]);
    assert_eq!(b["id"], BETA);
    fx.refused(&["create", "--name", " "], "the name is empty");
    fx.refused(
        &["create", "--name", "x", "--start", "October"],
        "is not a date YYYY-MM-DD",
    );

    // set
    fx.ok(&[
        "set",
        ALPHA,
        "--name",
        "Alpha: the story",
        "--add-alias",
        "Alpha",
        "--add-alias",
        "A",
        "--description",
        "New words.",
    ]);
    // Removals are made before additions; an alias folds as ids do.
    let s = fx.ok(&[
        "set",
        ALPHA,
        "--remove-alias",
        " a ",
        "--add-alias",
        "alpha",
    ]);
    assert_eq!(s["tool"], "scout events set");
    assert_eq!(
        (
            &s["record"]["name"],
            &s["record"]["aliases"],
            &s["record"]["description"]
        ),
        (
            &json!("Alpha: the story"),
            &json!(["Alpha"]),
            &json!("New words.")
        )
    );
    let s = fx.ok(&["set", ALPHA, "--end", ""]);
    assert_eq!(s["record"]["end"], Value::Null);
    fx.ok(&["set", ALPHA, "--end", "2026-09-20"]);
    fx.refused(&["set", ALPHA], "nothing to set");
    fx.refused(
        &["set", ALPHA, "--end", "2026-09-01"],
        "end 2026-09-01 is before start 2026-09-05",
    );
    fx.refused(
        &["set", ALPHA, "--name", "two\nlines"],
        "the name holds a control character",
    );
    fx.refused(
        &["set", "2026-01-nobody", "--name", "x"],
        "no such event record",
    );

    // merge (his, by default), check, unmerge
    fx.refused(
        &["merge", BETA, ALPHA, "--by", "model"],
        "a model's merge does not merge away a record he edited",
    );
    fx.refused(&["merge", BETA, ALPHA, "--by", "someone"], "invalid value");
    let m = fx.ok(&["merge", BETA, ALPHA, "--reason", "one story"]);
    assert_eq!(
        (m["tool"].as_str(), m["log_id"].as_str(), m["by"].as_str()),
        (Some("scout events merge"), Some("m000001"), Some("dominik"))
    );
    assert_eq!(
        (&m["a"], &m["b"], &m["reason"]),
        (&json!(BETA), &json!(ALPHA), &json!("one story"))
    );
    assert_eq!(
        (&m["a_record"]["id"], &m["a_record"]["merged_into"]),
        (&json!(BETA), &json!(ALPHA))
    );
    assert!(m.get("record").is_none() && m.get("members_added").is_none());
    let log = fx.lines("merge-log.jsonl");
    assert_eq!(log.len(), 1);
    let mut keys: Vec<&str> = log[0]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort();
    assert_eq!(
        keys,
        ["a", "a_merged_into_before", "at", "b", "by", "id", "reason"]
    );
    fx.refused(&["merge", BETA, ALPHA], "already merged into");
    fx.refused(&["remove", BETA], "merged into 2026-09-alpha-story");
    let c = fx.ok(&["check"]);
    assert_eq!(
        (
            &c["ok"],
            &c["merges"],
            &c["in_effect"],
            &c["problems"],
            &c["information"]
        ),
        (&json!(true), &json!(1), &json!(1), &json!([]), &json!([]))
    );
    let u = fx.ok(&["unmerge", "m000001"]);
    assert_eq!(
        (&u["tool"], &u["log_id"], &u["undoes"], &u["withdrawn"]),
        (
            &json!("scout events unmerge"),
            &json!("m000002"),
            &json!("m000001"),
            &json!(false)
        )
    );
    assert_eq!(fx.lines("merge-log.jsonl")[1].as_object().unwrap().len(), 3);
    fx.refused(&["unmerge", "m000001"], "already undone");
    fx.refused(&["unmerge", "m000007"], "no such merge");

    // remove, restore
    let r = fx.ok(&["remove", BETA]);
    assert_eq!(
        (r["tool"].as_str(), r["changed"].as_bool()),
        (Some("scout events remove"), Some(true))
    );
    assert!(r["record"]["removed"].as_str().unwrap().starts_with("20"));
    assert_eq!(fx.ok(&["remove", BETA])["changed"], false);
    let r = fx.ok(&["restore", BETA]);
    assert_eq!(
        (r["tool"].as_str(), r["changed"].as_bool()),
        (Some("scout events restore"), Some(true))
    );
    assert!(r["record"].get("removed").is_none());

    // proposals: list, dry run, decide
    let folder = fx.dir.join("merge-proposals");
    fs::create_dir(&folder).unwrap();
    let file = folder.join("run-1.jsonl");
    let pair = |verdict: &str, confidence: &str| json!({"a": ALPHA, "b": BETA, "a_items": 2, "b_items": 1, "verdict": verdict, "confidence": confidence, "reason": "same thing"});
    fs::write(&file, format!("{}\n", pair("same", "doubtful"))).unwrap();
    let f = file.to_str().unwrap();
    let p = fx.ok(&["proposals", f, "--json"]);
    assert_eq!(p["tool"], "scout events proposals");
    assert_eq!(p["waiting"].as_array().unwrap().len(), 1);
    assert_eq!(
        (
            &p["waiting"][0]["from"],
            &p["waiting"][0]["into"],
            &p["waiting"][0]["why"]
        ),
        (&json!(BETA), &json!(ALPHA), &json!("doubtful"))
    );
    // `--json` changes nothing: the output is always the one line.
    assert_eq!(fx.ok(&["proposals", f])["waiting"], p["waiting"]);
    let before = fx.tree();
    let out = fx.run(&["apply-proposals", f]);
    assert_eq!(out.status.code(), Some(0));
    assert!(text(&out.stderr)
        .contains("dry run: nothing written; 0 merges would be made (add --apply)"));
    let dry: Value = serde_json::from_str(&text(&out.stdout)).unwrap();
    assert_eq!(
        (&dry["tool"], &dry["apply"], &dry["same_confident"]),
        (
            &json!("scout events apply-proposals"),
            &json!(false),
            &json!(0)
        )
    );
    assert!(fx.tree() == before);
    fx.refused(&["decide", f, ALPHA, BETA, "maybe"], "invalid value");
    fx.refused(
        &["decide", f, ALPHA, "2026-01-nobody", "approve"],
        "no proposal waiting",
    );
    let outside = fx.tmp.path().join("run-1.jsonl");
    fs::copy(&file, &outside).unwrap();
    fx.refused(
        &["apply-proposals", outside.to_str().unwrap(), "--apply"],
        "a proposals file is read only from there",
    );
    let d = fx.ok(&["decide", f, BETA, ALPHA, "approve"]);
    assert_eq!(
        (
            &d["tool"],
            &d["decision"],
            &d["merge"]["log_id"],
            &d["merge"]["by"]
        ),
        (
            &json!("scout events decide"),
            &json!("approve"),
            &json!("m000003"),
            &json!("dominik")
        )
    );
    let decisions = fx.lines("merge-decisions.jsonl");
    assert_eq!(decisions.len(), 1);
    assert_eq!(
        (
            &decisions[0]["file"],
            &decisions[0]["a"],
            &decisions[0]["b"],
            &decisions[0]["decision"]
        ),
        (
            &json!("run-1.jsonl"),
            &json!(ALPHA),
            &json!(BETA),
            &json!("approve")
        )
    );
    assert_eq!(decisions[0].as_object().unwrap().len(), 5);
    fx.refused(
        &["decide", f, ALPHA, BETA, "dismiss"],
        "already decided (approve)",
    );
    assert_eq!(fx.ok(&["proposals", f])["waiting"], json!([]));
    // A confident pair is the model's merge: applied with --apply.
    fx.ok(&["unmerge", "m000003"]);
    let gamma = fx.ok(&["create", "--name", "Gamma", "--start", "2026-11-01"])["id"]
        .as_str()
        .unwrap()
        .to_string();
    let file2 = folder.join("run-2.jsonl");
    fs::write(&file2, format!("{}\n", json!({"a": gamma, "b": ALPHA, "verdict": "same", "confidence": "confident", "reason": "x"}))).unwrap();
    let applied = fx.ok(&["apply-proposals", file2.to_str().unwrap(), "--apply"]);
    // Gamma is his (created by him, `edited` set): the model does not
    // merge it away, and the pair waits for him instead.
    assert_eq!(
        (&applied["merged"], &applied["skipped"][0]["why"]),
        (&json!(0), &json!("model-rule"))
    );
    assert_eq!(
        fx.ok(&["proposals", file2.to_str().unwrap()])["waiting"][0]["why"],
        "model-rule"
    );

    // check: exit 1 with its line when something is off.
    fs::write(fx.dir.join(".scout-edit-1-a-0.tmp"), "half").unwrap();
    let out = fx.run(&["check"]);
    assert_eq!(out.status.code(), Some(1));
    let c: Value = serde_json::from_str(&text(&out.stdout)).unwrap();
    assert_eq!(
        (&c["ok"], &c["problems"][0]["kind"]),
        (&json!(false), &json!("temporary-file"))
    );
    // A malformed log: every command stops, exit 2.
    fs::write(fx.dir.join("merge-log.jsonl"), "not json\n").unwrap();
    fx.refused(&["check"], "merge-log.jsonl: line 1: not JSON");
    fx.refused(&["merge", BETA, ALPHA], "merge-log.jsonl: line 1: not JSON");
}

#[test]
fn merges_prints_the_merges_in_effect_and_with_all_the_undone_ones() {
    let fx = Fx::new();
    fx.ok(&["create", "--name", "Alpha story", "--start", "2026-09-05"]);
    fx.ok(&["create", "--name", "Beta", "--start", "2026-10-02"]);
    let dir = fx.dir.display().to_string();
    // No log yet: an empty list, and `--json` changes nothing.
    let none = fx.ok(&["merges"]);
    assert_eq!(
        (&none["tool"], &none["dir"], &none["merges"]),
        (&json!("scout events merges"), &json!(dir), &json!([]))
    );
    assert!(none.get("all").is_none());
    assert_eq!(fx.ok(&["merges", "--json"])["merges"], json!([]));

    let m = fx.ok(&["merge", BETA, ALPHA, "--reason", "one story"]);
    let before = fx.tree();
    let live = fx.ok(&["merges", "--json"]);
    assert_eq!(
        live["merges"],
        json!([{
            "id": "m000001", "at": m["at"], "by": "dominik", "reason": "one story",
            "a": {"id": BETA, "name": "Beta"}, "b": {"id": ALPHA, "name": "Alpha story"},
        }])
    );
    assert_eq!(fx.ok(&["merges", "--all"])["merges"], live["merges"]);

    let u = fx.ok(&["unmerge", "m000001"]);
    assert_eq!(fx.ok(&["merges"])["merges"], json!([]));
    let all = fx.ok(&["merges", "--all"]);
    assert_eq!(all["all"], json!(true));
    assert_eq!(
        all["merges"],
        json!([{
            "id": "m000001", "at": m["at"], "by": "dominik", "reason": "one story",
            "a": {"id": BETA, "name": "Beta"}, "b": {"id": ALPHA, "name": "Alpha story"},
            "undone_by": "m000002", "undone_at": u["at"],
        }])
    );
    // Merged again: the new line is first, the undone one after it.
    fx.ok(&["merge", BETA, ALPHA]);
    let ids = |v: &Value| -> Vec<String> {
        v["merges"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["id"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(ids(&fx.ok(&["merges"])), ["m000003"]);
    assert_eq!(ids(&fx.ok(&["merges", "--all"])), ["m000003", "m000001"]);
    // Reading writes nothing.
    fx.ok(&["unmerge", "m000003"]);
    let state = fx.tree();
    fx.ok(&["merges", "--all"]);
    assert!(fx.tree() == state && state != before);
    // A malformed log stops it like every command that reads the log.
    fs::write(fx.dir.join("merge-log.jsonl"), "not json\n").unwrap();
    fx.refused(&["merges"], "merge-log.jsonl: line 1: not JSON");
}

#[test]
fn unmerge_preview_prints_what_the_undo_would_leave_and_writes_nothing() {
    let fx = Fx::new();
    fx.ok(&["create", "--name", "Alpha story", "--start", "2026-09-05"]);
    fx.ok(&[
        "create",
        "--name",
        "Beta",
        "--start",
        "2026-10-02",
        "--end",
        "2026-10-09",
    ]);
    // Three tag rows: one of Beta, one of Alpha, one of Beta that a
    // correction also puts in Alpha.
    let topics = fx.archive.join("analysis/topics");
    fs::create_dir_all(&topics).unwrap();
    let rows = [("x:1", BETA), ("x:2", ALPHA), ("x:3", BETA)];
    let layer: String = rows
        .iter()
        .map(|(key, event)| format!("{}\n", json!({"key": key, "event": event})))
        .collect();
    fs::write(topics.join("current.jsonl"), layer).unwrap();
    fs::write(
        topics.join("corrections.jsonl"),
        format!("{}\n", json!({"key": "x:3", "event_add": [ALPHA]})),
    )
    .unwrap();
    fx.ok(&["merge", BETA, ALPHA]);

    let before = fx.tree();
    let p = fx.ok(&["unmerge-preview", "m000001"]);
    assert_eq!(
        p,
        json!({
            "log_id": "m000001",
            "returning": {"id": BETA, "name": "Beta"},
            "kept": {"id": ALPHA, "name": "Alpha story"},
            "undoable": true,
            "start": "2026-10-02", "end": "2026-10-09",
            "returning_items": 2, "kept_items": 2, "in_both": 1, "kept_items_now": 3,
        })
    );
    assert_eq!(fx.ok(&["unmerge-preview", "m000001", "--json"]), p);
    assert!(fx.tree() == before, "the preview wrote");
    // No merge in effect of that id: exit 2 in `unmerge`'s words.
    fx.refused(&["unmerge-preview", "m000007"], "m000007: no such merge");
    fx.ok(&["unmerge", "m000001"]);
    fx.refused(&["unmerge-preview", "m000001"], "m000001: already undone");
}

#[test]
fn the_archive_is_required_and_its_path_is_checked() {
    let fx = Fx::new();
    let scout = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_scout"))
            .args(args)
            .env("SCOUT_CONFIG", fx.tmp.path().join("no-such-corpora.toml"))
            .env("SCOUT_DATA_DIR", fx.tmp.path().join("data"))
            .output()
            .unwrap()
    };
    let out = scout(&["events", "remove", "x"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(text(&out.stderr).contains("--archive"));
    let dotted = format!("{}/sub/..", fx.archive.display());
    let out = scout(&["events", "check", "--archive", &dotted]);
    assert_eq!(out.status.code(), Some(2));
    assert!(text(&out.stderr).contains("the archive path has a `..` component"));
    let empty = fx.tmp.path().join("empty");
    fs::create_dir(&empty).unwrap();
    let out = scout(&[
        "events",
        "create",
        "--name",
        "x",
        "--archive",
        empty.to_str().unwrap(),
    ]);
    assert_eq!(out.status.code(), Some(2));
    assert!(text(&out.stderr).contains("no records folder"));
    assert!(fs::read_dir(&empty).unwrap().next().is_none());
}

#[cfg(unix)]
#[test]
fn apply_proposals_refuses_a_log_with_a_second_name_the_same_with_and_without_apply() {
    let fx = Fx::new();
    // Two records of the model's (one made here is his, and the model
    // does not merge his away).
    for (id, name, start) in [
        (ALPHA, "Alpha story", "2026-09-05"),
        (BETA, "Beta", "2026-10-02"),
    ] {
        let record = scout_corpus::events::EventRecord {
            id: id.into(),
            name: name.into(),
            start: Some(start.into()),
            end: Some(start.into()),
            origin: Some("model".into()),
            description: format!("About {name}."),
            ..Default::default()
        };
        fs::write(
            fx.dir.join(format!("{id}.md")),
            scout_corpus::event_seed::render_record(&record),
        )
        .unwrap();
    }
    let folder = fx.dir.join("merge-proposals");
    fs::create_dir_all(&folder).unwrap();
    let file = folder.join("run-1.jsonl");
    fs::write(
        &file,
        format!(
            "{}\n",
            json!({"a": ALPHA, "b": BETA, "verdict": "same", "confidence": "confident", "reason": "one story"})
        ),
    )
    .unwrap();
    let f = file.to_str().unwrap();
    // An empty log, linked from outside the records folder.
    let log = fx.dir.join("merge-log.jsonl");
    let other = fx.tmp.path().join("other-name.jsonl");
    fs::write(&log, "").unwrap();
    fs::hard_link(&log, &other).unwrap();
    let why = "merge-log.jsonl: has 2 hard links";
    fx.refused(&["apply-proposals", f], why);
    fx.refused(&["apply-proposals", f, "--apply"], why);
    let (dry, run) = (
        fx.run(&["apply-proposals", f]),
        fx.run(&["apply-proposals", f, "--apply"]),
    );
    assert_eq!(text(&dry.stderr), text(&run.stderr));
    assert!(
        text(&dry.stderr).contains("1 merges are planned and none was made"),
        "{}",
        text(&dry.stderr)
    );
    assert!(!text(&dry.stderr).contains("dry run"));
    assert_eq!(fs::read(&other).unwrap(), b"");
    // With one name: the plan, then the merge.
    fs::remove_file(&other).unwrap();
    let planned = fx.ok(&["apply-proposals", f]);
    assert_eq!(planned["merges"].as_array().unwrap().len(), 1);
    let made = fx.ok(&["apply-proposals", f, "--apply"]);
    assert_eq!(made["merged"], 1);
    assert_eq!(fx.lines("merge-log.jsonl").len(), 1);
}

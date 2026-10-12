//! CLI seam for `scout correct`: each command prints one JSON line on
//! stdout and exits 0; a refusal exits 2 with the reason on stderr,
//! nothing on stdout and no file changed. They need no registry and no
//! index, and what they print is the facade's result.

use scout_corpus::corrections::CorrectionEditor;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const MATHS: &str = "AI and mathematics";
const EVENT: &str = "2026-09-gemini-launch";

struct Fx {
    tmp: tempfile::TempDir,
    archive: PathBuf,
}

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

impl Fx {
    fn new() -> Fx {
        let tmp = tempfile::tempdir().unwrap();
        let archive = tmp.path().join("archive");
        write(
            &archive,
            "analysis/topics/current.jsonl",
            concat!(
                r#"{"key":"x:1","date":"2026-09-01","themes":["Vibe coding"],"entities":["Terence Tao"],"event":null}"#,
                "\n",
                r#"{"key":"x:2","date":"2026-09-02","themes":[],"entities":[],"event":"2026-09/E02"}"#,
                "\n"
            ),
        );
        write(
            &archive,
            "analysis/context/themes.json",
            &json!({"themes": [{"name": MATHS}, {"name": "Vibe coding"}]}).to_string(),
        );
        write(
            &archive,
            &format!("analysis/events/{EVENT}.md"),
            &format!("---\nid: {EVENT}\nname: Gemini launch\nmembers: [2026-09/E02]\norigin: model\n---\n\nA launch.\n"),
        );
        Fx { tmp, archive }
    }

    /// `scout correct <args> --archive <archive>`.
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_scout"))
            .arg("correct")
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

    fn lines(&self) -> Vec<Value> {
        fs::read_to_string(self.archive.join("analysis/topics/corrections.jsonl"))
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }
}

#[test]
fn every_correction_prints_one_json_line_and_a_refusal_exits_2_with_nothing_written() {
    let fx = Fx::new();
    let before = fx.tree();
    // Nothing is there to list yet, and listing writes nothing.
    let l = fx.ok(&["list", "--key", "x:1", "--json"]);
    assert_eq!(l["tool"], "scout correct list");
    assert_eq!(l["corrections"], json!([]));
    assert_eq!(l["model"], l["effective"]);
    assert!(fx.tree() == before);

    let c = fx.ok(&["theme-add", "ai and mathematics", "--key", "x:1"]);
    assert_eq!(
        (
            &c["tool"],
            &c["id"],
            &c["op"],
            &c["value"],
            &c["key"],
            &c["reindex_needed"]
        ),
        (
            &json!("scout correct theme-add"),
            &json!("c000001"),
            &json!("theme_add"),
            &json!(MATHS),
            &json!("x:1"),
            &json!(true)
        )
    );
    assert_eq!(c["tags"]["themes"], json!(["Vibe coding", MATHS]));
    assert_eq!(c["tags"]["corrected"], json!(["themes"]));
    let c = fx.ok(&["theme-remove", "Vibe coding", "--key", "x:1"]);
    assert_eq!(c["tags"]["themes"], json!([MATHS]));
    let c = fx.ok(&[
        "entity-add",
        "GPT-6 Astra",
        "--type",
        "model",
        "--key",
        "x:1",
    ]);
    assert_eq!(
        (&c["entity_type"], &c["id"]),
        (&json!("model"), &json!("c000003"))
    );
    let c = fx.ok(&["entity-remove", "terence tao", "--key", "x:1"]);
    assert_eq!(c["tags"]["entities"], json!(["GPT-6 Astra"]));
    let c = fx.ok(&["event-add", "2026-09/E02", "--key", "x:1"]);
    assert_eq!(
        (&c["value"], &c["tags"]["events"]),
        (&json!(EVENT), &json!([EVENT]))
    );
    let c = fx.ok(&["event-remove", EVENT, "--key", "x:2"]);
    assert_eq!(c["tags"]["events"], json!([]));
    assert_eq!(c["tags"]["event"], Value::Null);
    let u = fx.ok(&["undo", "c000006"]);
    assert_eq!(
        (&u["tool"], &u["op"], &u["undoes"], &u["id"]),
        (
            &json!("scout correct undo"),
            &json!("undo"),
            &json!("c000006"),
            &json!("c000007")
        )
    );
    assert_eq!(u["tags"]["events"], json!([EVENT]));
    assert_eq!(u["tags"].get("corrected"), None);

    // The file: one line per command, in the documented shape.
    let lines = fx.lines();
    assert_eq!(lines.len(), 7);
    // Keys in the order written (a parsed object does not keep it).
    let raw = fs::read_to_string(fx.archive.join("analysis/topics/corrections.jsonl")).unwrap();
    let raw: Vec<&str> = raw.lines().collect();
    let at = lines[0]["at"].as_str().unwrap();
    assert_eq!(
        raw[0],
        format!(
            r#"{{"id":"c000001","at":"{at}","date":"{}","by":"dominik","key":"x:1","theme_add":["AI and mathematics"]}}"#,
            &at[..10]
        )
    );
    assert!(
        raw[2].ends_with(r#""key":"x:1","entity_add":["GPT-6 Astra"],"entity_type":"model"}"#),
        "{}",
        raw[2]
    );
    assert!(
        raw[6].ends_with(r#""by":"dominik","key":"x:2","undoes":"c000006"}"#),
        "{}",
        raw[6]
    );
    assert_eq!(lines[4]["event_add"], json!([EVENT]));

    // The listing is the facade's, newest first.
    let l = fx.ok(&["list", "--key", "x:1"]);
    let facade = CorrectionEditor::open(&fx.archive)
        .unwrap()
        .list("x:1")
        .unwrap();
    assert_eq!(l, serde_json::to_value(&facade).unwrap());
    let ids: Vec<&str> = l["corrections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["c000005", "c000004", "c000003", "c000002", "c000001"]);
    assert_eq!(l["model"]["themes"], json!(["Vibe coding"]));
    assert_eq!(l["effective"]["themes"], json!([MATHS]));

    // Refusals.
    fx.refused(
        &["theme-add", "Not listed", "--key", "x:1"],
        "not a theme of",
    );
    fx.refused(
        &["theme-add", MATHS, "--key", "x:1"],
        "already has the theme",
    );
    fx.refused(
        &["theme-remove", "Vibe coding", "--key", "x:1"],
        "has no theme",
    );
    fx.refused(&["theme-add", MATHS, "--key", "x:404"], "x:404: no tag row");
    fx.refused(
        &["entity-add", "A", "--type", "animal", "--key", "x:1"],
        "not an entity type",
    );
    fx.refused(
        &["event-add", "2031-01/E01", "--key", "x:1"],
        "no event record holds",
    );
    fx.refused(
        &["event-add", EVENT, "--key", "x:1"],
        "x:1: already in the event",
    );
    fx.refused(&["undo", "c000006"], "c000006: already undone, by c000007");
    fx.refused(&["undo", "c000040"], "no such correction");
    fx.refused(&["list", "--key", "x:404"], "no tag row");
    // A usage error is exit 2 as well, with nothing written.
    let before = fx.tree();
    let out = fx.run(&["entity-add", "A", "--key", "x:1"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(fx.tree() == before);
}

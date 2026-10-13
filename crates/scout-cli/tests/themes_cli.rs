//! CLI seam for `scout themes list`: one JSON line on stdout that is the
//! library's value, a refusal as exit 2 with the reason on one stderr
//! line and nothing on stdout, and no file of the archive changed. It
//! needs no registry and no index.

use scout_corpus::themes::themes_list;
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

const LIST: &str = "analysis/context/themes.json";

fn list(home: &Path, archive: &Path, json: bool) -> Output {
    let mut args = vec!["themes", "list", "--archive", archive.to_str().unwrap()];
    if json {
        args.push("--json");
    }
    scout(home, &args)
}

#[test]
fn list_prints_the_librarys_value_as_one_json_line_and_changes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let h = tmp.path().join("archive");
    write(
        &h,
        LIST,
        &json!({"meta": {}, "themes": [
            {"id": "T02", "name": "Vibe coding", "definition": "Writing code\u{1b}[31m with a model."},
            {"id": "T01", "name": "AI and mathematics"},
        ]})
        .to_string(),
    );
    let before = tree(&h);

    let o = list(tmp.path(), &h, true);
    assert!(o.status.success(), "{}", text(&o.stderr));
    assert!(o.stderr.is_empty());
    let out = text(&o.stdout);
    assert_eq!(out.lines().count(), 1, "{out}");
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        v["themes"],
        serde_json::to_value(themes_list(&h).unwrap()).unwrap()
    );
    assert_eq!(
        v,
        json!({"themes": [
            {"name": "Vibe coding", "id": "T02", "definition": "Writing code[31m with a model."},
            {"name": "AI and mathematics", "id": "T01", "definition": null},
        ]})
    );
    // Without --json the output is the same line.
    assert_eq!(list(tmp.path(), &h, false).stdout, o.stdout);
    assert_eq!(tree(&h), before);
    assert!(!tmp.path().join("data").exists());
}

#[test]
fn a_refusal_is_exit_2_with_one_line_on_stderr_and_nothing_on_stdout() {
    let tmp = tempfile::tempdir().unwrap();
    let h = tmp.path().join("archive");
    fs::create_dir_all(&h).unwrap();
    let at = h.join(LIST);
    let refused = |why: String| {
        let o = list(tmp.path(), &h, true);
        assert_eq!(o.status.code(), Some(2), "{why}");
        assert!(o.stdout.is_empty(), "{why}");
        assert_eq!(text(&o.stderr), format!("scout: {why}\n"));
    };
    refused("No theme list in this archive.".to_string());
    for (bad, why) in [
        ("{\"themes\": 1}", "no `themes` list (nothing was changed)"),
        (
            "{\"themes\": [{\"name\": 3}]}",
            "theme 1 has no text `name` (nothing was changed)",
        ),
        (
            "{\"themes\": [{\"name\": \"A\", \"id\": 1}]}",
            "theme 1: `id` is not text (nothing was changed)",
        ),
    ] {
        write(&h, LIST, bad);
        refused(format!("{}: {why}", at.display()));
    }
    #[cfg(unix)]
    {
        fs::remove_file(&at).unwrap();
        write(tmp.path(), "outside/themes.json", "{\"themes\": []}");
        std::os::unix::fs::symlink(tmp.path().join("outside/themes.json"), &at).unwrap();
        refused(format!(
            "{}: not a plain file (a symlink, a folder or a special file); nothing was changed",
            at.display()
        ));
        fs::remove_dir_all(h.join("analysis/context")).unwrap();
        std::os::unix::fs::symlink(tmp.path().join("outside"), h.join("analysis/context")).unwrap();
        refused(format!(
            "{}: not a plain folder; the theme list was not read",
            h.join("analysis/context").display()
        ));
    }
}

//! CLI seam: with no `--in`, commands run over every indexed corpus and name
//! the unindexed ones on stderr; exit 3 only when none is indexed.

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn scout(cfg: &Path, data: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_scout"))
        .args(args)
        .env("SCOUT_CONFIG", cfg)
        .env("SCOUT_DATA_DIR", data)
        .output()
        .unwrap()
}

fn corpus(id: &str, kind: &str, path: &Path) -> String {
    format!(
        "[[corpus]]\nid = \"{id}\"\nname = \"{id}\"\nkind = \"{kind}\"\npath = \"{}\"\ninclude = [\"**/*.md\"]\nlink = \"writeflex://open?path={{path}}&line={{line}}\"\n\n",
        path.display()
    )
}

fn snapshot(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut v: Vec<(String, Vec<u8>)> = fs::read_dir(root)
        .unwrap()
        .map(|e| {
            let p = e.unwrap().path();
            (p.display().to_string(), fs::read(&p).unwrap())
        })
        .collect();
    v.sort();
    v
}

#[test]
fn no_in_runs_over_indexed_corpora_and_names_the_rest() {
    let tmp = tempfile::tempdir().unwrap();
    let (a, b, data) = (
        tmp.path().join("a"),
        tmp.path().join("b"),
        tmp.path().join("data"),
    );
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    fs::write(
        a.join("x.md"),
        "---\ntitle: X\ndate: 2016\n---\nA generative metaphor, then another metaphor.\n",
    )
    .unwrap();
    fs::write(b.join("y.md"), "---\ntitle: Y\n---\nmetaphor\n").unwrap();
    let cfg = tmp.path().join("corpora.toml");
    fs::write(
        &cfg,
        format!(
            "{}{}{}",
            corpus("alpha", "markdown-folder", &a),
            corpus("beta", "markdown-folder", &b),
            corpus("hl", "highlight-scout-archive", &b)
        ),
    )
    .unwrap();
    let before = snapshot(&a);

    // Nothing indexed: exit 3, naming the build command.
    let o = scout(&cfg, &data, &["kwic", "metaphor"]);
    assert_eq!(o.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&o.stderr).contains("scout index build"));

    assert!(scout(&cfg, &data, &["index", "build", "alpha"])
        .status
        .success());
    for args in [
        vec!["kwic", "metaphor", "--json"],
        vec!["search", "metaphor", "--json"],
        vec!["profile", "metaphor", "--json"],
        vec!["collocates", "metaphor", "--min", "1", "--json"],
        vec!["ngrams", "--n", "1", "--json"],
        vec!["dist", "metaphor", "--by", "year", "--json"],
        vec!["verify-quote", "another metaphor", "--json"],
        vec![
            "keyness",
            "--a",
            "before:2020",
            "--b",
            "after:2020",
            "--min",
            "1",
            "--json",
        ],
    ] {
        let o = scout(&cfg, &data, &args);
        let err = String::from_utf8_lossy(&o.stderr);
        assert!(
            o.status.code() == Some(0) || o.status.code() == Some(1),
            "{args:?}: {:?} {err}",
            o.status
        );
        let notes: Vec<&str> = err.lines().filter(|l| l.contains("note:")).collect();
        assert_eq!(
            notes,
            vec!["scout: note: using alpha; not indexed: beta, hl"],
            "{args:?}"
        );
        let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
        assert!(v["schema_version"].is_number(), "{args:?}");
    }
    let o = scout(&cfg, &data, &["kwic", "metaphor", "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["corpora"], serde_json::json!(["alpha"]));
    assert_eq!(v["total"], 2);
    // Naming an unindexed corpus is still exit 3.
    assert_eq!(
        scout(&cfg, &data, &["kwic", "metaphor", "--in", "beta"])
            .status
            .code(),
        Some(3)
    );
    // An explicit --in prints no note.
    let o = scout(&cfg, &data, &["kwic", "metaphor", "--in", "alpha"]);
    assert!(o.status.success());
    assert!(!String::from_utf8_lossy(&o.stderr).contains("note:"));
    // Sources are untouched.
    assert_eq!(before, snapshot(&a));
}

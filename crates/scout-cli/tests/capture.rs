use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

fn run(root: &Path, args: &[&str], stdin: Option<&str>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_scout"))
        .args(args)
        .env("SCOUT_EMBEDDER", "hash")
        .env("SCOUT_CONFIG", root.join("corpora.toml"))
        .env("SCOUT_DATA_DIR", root.join("data"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(s) = stdin {
        child.stdin.take().unwrap().write_all(s.as_bytes()).unwrap();
    } else {
        drop(child.stdin.take());
    }
    child.wait_with_output().unwrap()
}
fn json_output(o: Output) -> Value {
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    serde_json::from_slice(&o.stdout).unwrap()
}

#[test]
fn stdin_json_index_build_list_and_where_used_end_to_end() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    let project = root.join("vault/talk");
    fs::create_dir_all(&project).unwrap();
    fs::write(
        root.join("corpora.toml"),
        format!(
            "[[corpus]]\nid='talks'\nname='Talks'\nkind='markdown-folder'\npath={:?}\n",
            root.join("vault").to_str().unwrap()
        ),
    )
    .unwrap();
    let p = project.to_str().unwrap();
    let v = json_output(run(
        root,
        &[
            "capture",
            "--project",
            p,
            "--title",
            "Hebb",
            "--kind",
            "evidence",
            "--by",
            "app:writeflex",
            "--source",
            "scout:highlights:hebb.md:12",
            "--json",
            "-",
        ],
        Some("Neural connections strengthen.\n"),
    ));
    assert_eq!(v.as_object().unwrap().len(), 6);
    assert_eq!(v["corpus"], "talks");
    assert_eq!(v["manifest"], true);
    assert_eq!(v["indexed"], true);
    assert_eq!(v["passages"], 1);
    let file = v["file"].as_str().unwrap();
    assert!(fs::read_to_string(file)
        .unwrap()
        .ends_with("Neural connections strengthen.\n"));
    let list = json_output(run(
        root,
        &["capture", "--list", "--project", p, "--json"],
        None,
    ));
    assert_eq!(list["entries"][0]["by"], "app:writeflex");
    assert_eq!(
        list["entries"][0]["sources"][0],
        json!({"scout":"highlights:hebb.md:12"})
    );
    let name = Path::new(file).file_name().unwrap().to_str().unwrap();
    fs::write(
        project.join("talk-outline.md"),
        format!("# Talk\nSee research/{name}\n"),
    )
    .unwrap();
    let hits = json_output(run(
        root,
        &["capture", "--where-used", file, "--json"],
        None,
    ));
    assert_eq!(hits["hits"].as_array().unwrap().len(), 1);
    assert_eq!(hits["hits"][0]["line"], 2);
    let plain = run(root, &["capture", "--list", "--project", p], None);
    assert!(plain.status.success());
    let line: Value = serde_json::from_slice(&plain.stdout).unwrap();
    assert_eq!(line, list["entries"][0]);
    let found = json_output(run(
        root,
        &["search", "connections", "--fts", "--json"],
        None,
    ));
    assert_eq!(found["total_documents"], 1);
}

#[test]
fn invalid_cli_combinations_fail_before_writing() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    let project = root.join("project");
    fs::create_dir(&project).unwrap();
    let p = project.to_str().unwrap();
    for args in [
        vec!["capture", "--project", p, "-"],
        vec!["capture", "--list", "--where-used", "thing"],
        vec!["capture", "--project", p, "--list", "--title", "Title"],
    ] {
        let o = run(root, &args, None);
        assert_eq!(o.status.code(), Some(2));
    }
    assert!(!project.join("research").exists());
}

#[test]
fn capture_refreshes_existing_vectors_and_preserves_incremental_passage_ids() {
    use scout_corpus::semantic::store::{freshness, Freshness};
    use scout_corpus::{Corpus, Embedder, HashEmbedder, Registry};
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    let project = root.join("vault");
    fs::create_dir(&project).unwrap();
    fs::write(
        project.join("z-last.md"),
        "Existing neural connections strengthen through repeated practice and experience.\n",
    )
    .unwrap();
    let config = format!(
        "[[corpus]]\nid='talks'\nname='Talks'\nkind='markdown-folder'\npath={:?}\n",
        project.to_str().unwrap()
    );
    fs::write(root.join("corpora.toml"), &config).unwrap();
    json_output(run(
        root,
        &["index", "build", "talks", "--semantic", "--json"],
        None,
    ));
    let cfg = Registry::parse(&config).unwrap().corpora.remove(0);
    let corpus = Corpus::from_config(cfg, &root.join("data/indexes"));
    let conn = scout_corpus::index::open_existing(&corpus.config, corpus.index_path()).unwrap();
    let old: i64 = conn
        .query_row(
            "SELECT id FROM passages WHERE rel_path='z-last.md'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    drop(conn);
    let result = json_output(run(root, &["capture", "--project", project.to_str().unwrap(), "--title", "New connections", "--kind", "evidence", "--json", "-"], Some("Neural connections strengthen in the brain through repeated learning and experience.\n")));
    assert_eq!(result["schema_version"], 1);
    assert_eq!(result["passages"], 1);
    let conn = scout_corpus::index::open_existing(&corpus.config, corpus.index_path()).unwrap();
    let after: i64 = conn
        .query_row(
            "SELECT id FROM passages WHERE rel_path='z-last.md'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        old, after,
        "capture must use an incremental build, never force=true"
    );
    assert_eq!(
        freshness(
            &conn,
            &corpus.vectors_path(),
            &HashEmbedder::default().model_id()
        )
        .unwrap(),
        Freshness::Current
    );
    let searched = run(root, &["search", "connections", "--hybrid", "--json"], None);
    assert!(!String::from_utf8_lossy(&searched.stderr).contains("full text only"));
    let found = json_output(searched);
    assert_eq!(found["mode"], "hybrid");
    let rebuilt = json_output(run(root, &["index", "build", "talks", "--json"], None));
    assert_eq!(rebuilt["reports"][0]["vectors"]["embedded"], 0);
}

#[test]
fn where_used_json_empty_has_schema_skipped_and_exit_one() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    let output = run(
        root,
        &["capture", "--where-used", "absent.md", "--json"],
        None,
    );
    assert_eq!(output.status.code(), Some(1));
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        value,
        json!({"schema_version":1,"file":"absent.md","hits":[],"skipped":0})
    );
    let project = root.join("project");
    fs::create_dir(&project).unwrap();
    let list = json_output(run(
        root,
        &[
            "capture",
            "--list",
            "--project",
            project.to_str().unwrap(),
            "--json",
        ],
        None,
    ));
    assert_eq!(
        list,
        json!({"schema_version":1,"project":project.to_str().unwrap(),"entries":[]})
    );
}

#[test]
fn nested_capture_and_explicit_moved_receipt_end_to_end() {
    let t = tempfile::tempdir().unwrap();
    let root = t.path();
    let project = root.join("project");
    fs::create_dir(&project).unwrap();
    let p = project.to_str().unwrap();
    let r = json_output(run(
        root,
        &[
            "capture",
            "--project",
            p,
            "--into",
            "literature/deep",
            "--kind",
            "evidence",
            "--title",
            "Nested",
            "--json",
            "-",
        ],
        Some("Body\n"),
    ));
    let file = Path::new(r["file"].as_str().unwrap());
    let research = project.canonicalize().unwrap().join("research");
    let old = file.strip_prefix(&research).unwrap().to_str().unwrap();
    fs::create_dir(research.join("moved")).unwrap();
    fs::rename(file, research.join("moved/nested.md")).unwrap();
    let before = json_output(run(
        root,
        &["capture", "--project", p, "--list", "--json"],
        None,
    ));
    assert_eq!(before["entries"][0]["file"], old);
    let changed = json_output(run(
        root,
        &[
            "capture",
            "--project",
            p,
            "--moved",
            old,
            "moved/nested.md",
            "--json",
        ],
        None,
    ));
    assert_eq!(changed["entries"][0]["file"], "moved/nested.md");
    let after = json_output(run(
        root,
        &["capture", "--project", p, "--list", "--json"],
        None,
    ));
    assert_eq!(after, changed);
    let manifest = fs::read(research.join("_manifest.jsonl")).unwrap();
    for (old, new) in [
        ("moved/nested.md", "../escape.md"),
        ("missing.md", "moved/nested.md"),
        ("moved/nested.md", "absent.md"),
    ] {
        assert!(!run(
            root,
            &["capture", "--project", p, "--moved", old, new],
            None
        )
        .status
        .success());
        assert_eq!(
            fs::read(research.join("_manifest.jsonl")).unwrap(),
            manifest
        );
    }
}

//! CLI seam: `corpora add | remove | init-defaults` against a temp HOME and
//! XDG config dir (never the user's registry), and a corpus whose folder
//! disappears after indexing.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::SystemTime;

struct Home {
    _tmp: tempfile::TempDir,
    home: PathBuf,
}

impl Home {
    fn new() -> Home {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().canonicalize().unwrap();
        Home { _tmp: tmp, home }
    }

    fn registry(&self) -> PathBuf {
        self.home.join("xdg/scout/corpora.toml")
    }

    fn data(&self) -> PathBuf {
        self.home.join("data")
    }

    fn scout(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_scout"))
            .args(args)
            .env_remove("SCOUT_CONFIG")
            .env("HOME", &self.home)
            .env("XDG_CONFIG_HOME", self.home.join("xdg"))
            .env("SCOUT_DATA_DIR", self.data())
            .env("SCOUT_EMBEDDER", "hash")
            .output()
            .unwrap()
    }
}

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn mtimes(root: &Path) -> Vec<(PathBuf, SystemTime)> {
    let mut v: Vec<_> = walk(root)
        .into_iter()
        .map(|p| {
            let m = fs::metadata(&p).unwrap().modified().unwrap();
            (p, m)
        })
        .collect();
    v.sort();
    v
}

fn walk(root: &Path) -> Vec<PathBuf> {
    let mut v = vec![root.to_path_buf()];
    for e in fs::read_dir(root).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            v.extend(walk(&p));
        } else {
            v.push(p);
        }
    }
    v
}

#[test]
fn missing_registry_names_corpora_add() {
    let h = Home::new();
    for args in [vec!["corpora", "list"], vec!["search", "metaphor"]] {
        let o = h.scout(&args);
        assert_eq!(o.status.code(), Some(3), "{args:?}");
        assert_eq!(
            err(&o).trim(),
            format!(
                "scout: no corpus registry at {}; add a folder with `scout corpora add <folder>`",
                h.registry().display()
            )
        );
    }
}

#[test]
fn add_build_search_remove_round_trip() {
    let h = Home::new();
    let notes = h.home.join("Notes");
    write(
        &notes,
        "on reading.txt",
        "A plain note on generative metaphor.\n# not a heading\n",
    );
    write(
        &notes,
        "essay.md",
        "---\ntitle: Essay\n---\nAnother metaphor here.\n",
    );

    let o = h.scout(&[
        "corpora",
        "add",
        "~/Notes",
        "--author",
        "B. Reader",
        "--json",
    ]);
    assert!(o.status.success(), "{}", err(&o));
    let cfg: serde_json::Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(cfg["id"], "notes");
    assert_eq!(cfg["path"], "~/Notes", "stored with ~ under home");
    assert_eq!(cfg["kind"], "markdown-folder");
    assert_eq!(cfg["include"], serde_json::json!(["**/*.md", "**/*.txt"]));
    assert_eq!(cfg["default_author"], "B. Reader");
    assert!(h.registry().is_file());
    assert!(!h.data().exists(), "add does not build the index");

    // The human form names the next command.
    let other = h.home.join("other");
    fs::create_dir_all(&other).unwrap();
    let o = h.scout(&["corpora", "add", other.to_str().unwrap()]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(
        out(&o).contains("next: scout index build other"),
        "{}",
        out(&o)
    );

    // Same folder again, and a clashing id: refused, registry untouched.
    let before = fs::read(h.registry()).unwrap();
    let o = h.scout(&["corpora", "add", notes.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(2));
    assert!(
        err(&o).contains("already registered as `notes`"),
        "{}",
        err(&o)
    );
    let o = h.scout(&["corpora", "add", h.home.to_str().unwrap(), "--id", "other"]);
    assert!(
        err(&o).contains("corpus id `other` is already registered"),
        "{}",
        err(&o)
    );
    let o = h.scout(&["corpora", "add", "~/nope"]);
    assert!(!o.status.success());
    assert_eq!(fs::read(h.registry()).unwrap(), before);

    // The .txt without frontmatter is indexed and titled by its stem.
    let o = h.scout(&["index", "build", "notes", "--semantic"]);
    assert!(o.status.success(), "{}", err(&o));
    let o = h.scout(&["search", "generative", "--in", "notes", "--json"]);
    assert!(o.status.success(), "{}", err(&o));
    let res: serde_json::Value = serde_json::from_str(&out(&o)).unwrap();
    let doc = &res["results"][0];
    assert_eq!(doc["title"], "on reading", "{doc}");
    assert!(doc["date"].is_null(), "{doc}");

    // remove --keep-index on one, plain remove on the other.
    let o = h.scout(&["corpora", "add", h.home.join("other").to_str().unwrap()]);
    assert!(!o.status.success());
    let idx = h.data().join("indexes");
    let src = mtimes(&notes);
    let o = h.scout(&["corpora", "remove", "other", "--keep-index", "--json"]);
    assert!(o.status.success(), "{}", err(&o));
    let r: serde_json::Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(r["index_kept"], true);

    assert!(idx.join("notes.sqlite").is_file());
    assert!(idx.join("notes.vectors.sqlite").is_file());
    let o = h.scout(&["corpora", "remove", "notes"]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("removed notes"), "{}", out(&o));
    assert!(!idx.join("notes.sqlite").exists());
    assert!(!idx.join("notes.vectors.sqlite").exists());
    assert_eq!(mtimes(&notes), src, "source folder untouched");
    let o = h.scout(&["corpora", "remove", "notes"]);
    assert_eq!(o.status.code(), Some(2));
    assert!(err(&o).contains("unknown corpus \"notes\""), "{}", err(&o));
    let reg = fs::read_to_string(h.registry()).unwrap();
    assert!(!reg.lines().any(|l| l.trim() == "[[corpus]]"), "{reg}");
}

#[test]
fn deleted_folder_is_marked_and_still_searchable() {
    let h = Home::new();
    let gone = h.home.join("gone");
    write(
        &gone,
        "a.md",
        "---\ntitle: A\n---\nA generative metaphor.\n",
    );
    assert!(h.scout(&["corpora", "add", "~/gone"]).status.success());
    assert!(h.scout(&["index", "build", "gone"]).status.success());
    fs::remove_dir_all(&gone).unwrap();

    let o = h.scout(&["corpora", "list"]);
    assert!(o.status.success(), "{}", err(&o));
    assert!(out(&o).contains("folder missing"), "{}", out(&o));
    let o = h.scout(&["corpora", "list", "--json"]);
    let list: serde_json::Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(list["corpora"][0]["folder_exists"], false);
    assert_eq!(list["corpora"][0]["index_built"], true);

    let o = h.scout(&["search", "generative", "--in", "gone", "--json"]);
    assert!(o.status.success(), "{}", err(&o));
    let res: serde_json::Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(res["results"][0]["title"], "A");
    assert!(h.scout(&["index", "status"]).status.success());
}

#[test]
fn init_defaults_is_generic_and_the_preset_is_hidden() {
    let h = Home::new();
    let o = h.scout(&["corpora", "init-defaults"]);
    assert!(o.status.success(), "{}", err(&o));
    let text = fs::read_to_string(h.registry()).unwrap();
    assert!(text.contains("scout corpora add"), "{text}");
    for private in ["gitrepos", "Lukeš", "writeflex", "dominik"] {
        assert!(
            !text.to_lowercase().contains(&private.to_lowercase()),
            "{private}: {text}"
        );
    }
    let o = h.scout(&["corpora", "list", "--json"]);
    let list: serde_json::Value = serde_json::from_str(&out(&o)).unwrap();
    assert_eq!(list["corpora"], serde_json::json!([]));

    // An add after init-defaults keeps the header.
    let n = h.home.join("n");
    fs::create_dir_all(&n).unwrap();
    assert!(h.scout(&["corpora", "add", "~/n"]).status.success());
    assert!(fs::read_to_string(h.registry()).unwrap().starts_with(&text));

    for args in [
        vec!["--help"],
        vec!["corpora", "--help"],
        vec!["corpora", "init-defaults", "--help"],
        vec!["corpora", "add", "--help"],
    ] {
        let help = out(&h.scout(&args));
        assert!(!help.contains("preset"), "{args:?}: {help}");
        assert!(!help.to_lowercase().contains("dominik"), "{args:?}: {help}");
    }
    let o = h.scout(&["corpora", "init-defaults", "--preset", "nobody"]);
    assert!(!o.status.success());
    assert!(!err(&o).contains("dominik"), "{}", err(&o));

    let o = h.scout(&["corpora", "init-defaults", "--force", "--preset", "dominik"]);
    assert!(o.status.success(), "{}", err(&o));
    let text = fs::read_to_string(h.registry()).unwrap();
    assert!(
        text.contains("id = \"writing\"") && text.contains("id = \"tweets\""),
        "{text}"
    );
}

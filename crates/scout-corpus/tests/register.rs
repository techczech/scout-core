//! Seam: registry read-modify-write (`RegistryStore::add` / `remove`, the
//! bodies of `api::add_corpus` / `api::remove_corpus`) and kind detection.
//! Every test works in a temp dir; the user registry is never read.

use scout_corpus::register::{detect_kind, RegistryStore};
use scout_corpus::{AddCorpus, CorpusKind, Registry};
use std::fs;
use std::path::{Path, PathBuf};

/// A registry in the shape of a long-lived one: leading comments, three
/// entries with sub-tables, links and a tweet unit.
fn three_entry_registry(dir: &Path) -> String {
    let (w, t, h) = (dir.join("w"), dir.join("t"), dir.join("h"));
    for d in [&w, &t, &h] {
        fs::create_dir_all(d).unwrap();
    }
    fs::create_dir_all(h.join("readings/works")).unwrap();
    format!(
        "# Scout corpus registry. One [[corpus]] per corpus.\n# Hand-edited: keep this note.\n\n\
         [[corpus]]\nid = \"writing\"\nname = \"Writing\"\nkind = \"markdown-folder\"\npath = \"{w}\"\n\
         include = [\"**/*.md\"]\nexclude = [\"cache/**\", \"tweets/**\"]\nrequire_frontmatter = [\"genre\"]\n\
         default_author = \"A. Writer\"\nboilerplate = []\nlink = \"app://open?path={{path}}&line={{line}}\"\n\n\
         [corpus.field_map]\nauthor = \"authors\"\npublic_url = [\"published_url\", \"source_url\"]\n\n\
         [[corpus]]\nid = \"tweets\"\nname = \"Tweets\"\nkind = \"markdown-folder\"\npath = \"{t}\"\n\
         document_unit = \"tweet\"\n\n\
         [[corpus]]\nid = \"highlights\"\nname = \"Highlights\"\nkind = \"highlight-scout-archive\"\npath = \"{h}\"\n",
        w = w.display(),
        t = t.display(),
        h = h.display()
    )
}

fn store(dir: &Path) -> (RegistryStore, PathBuf, PathBuf) {
    let reg = dir.join("config/corpora.toml");
    let idx = dir.join("indexes");
    (RegistryStore::new(&reg, &idx), reg, idx)
}

fn add(path: &Path) -> AddCorpus {
    AddCorpus {
        path: path.display().to_string(),
        ..AddCorpus::default()
    }
}

fn load(p: &Path) -> Registry {
    Registry::load_from(p).unwrap()
}

#[test]
fn add_to_missing_registry_creates_it_with_markdown_defaults() {
    let tmp = tempfile::tempdir().unwrap();
    let (s, reg, _) = store(tmp.path());
    let notes = tmp.path().join("My Notes");
    fs::create_dir_all(&notes).unwrap();

    let cfg = s.add(&add(&notes)).unwrap();
    assert_eq!(cfg.id, "my-notes");
    assert_eq!(cfg.name, "My Notes");
    assert_eq!(cfg.kind, CorpusKind::MarkdownFolder);
    assert_eq!(cfg.include, vec!["**/*.md", "**/*.txt"]);
    assert!(cfg.require_frontmatter.is_empty());
    assert!(cfg.link.is_none());
    assert!(cfg.default_author.is_none());

    let text = fs::read_to_string(&reg).unwrap();
    assert!(text.starts_with("# Scout corpus registry."));
    assert!(text.contains("scout corpora add"));
    assert_eq!(load(&reg).corpora, vec![cfg]);
}

#[test]
fn add_appends_and_keeps_existing_entries_and_comments() {
    let tmp = tempfile::tempdir().unwrap();
    let (s, reg, _) = store(tmp.path());
    let original = three_entry_registry(tmp.path());
    fs::create_dir_all(reg.parent().unwrap()).unwrap();
    fs::write(&reg, &original).unwrap();
    let before = load(&reg);
    let notes = tmp.path().join("notes");
    fs::create_dir_all(&notes).unwrap();

    let cfg = s
        .add(&AddCorpus {
            author: Some("B. Reader".into()),
            exclude: vec!["drafts/**".into()],
            ..add(&notes)
        })
        .unwrap();
    let text = fs::read_to_string(&reg).unwrap();
    assert!(
        text.starts_with(&original),
        "original bytes kept as a prefix"
    );
    let after = load(&reg);
    assert_eq!(after.corpora.len(), 4);
    assert_eq!(after.corpora[..3], before.corpora[..], "field-for-field");
    assert_eq!(after.corpora[3], cfg);
    assert_eq!(cfg.default_author.as_deref(), Some("B. Reader"));
    assert_eq!(cfg.exclude, vec!["drafts/**"]);
}

#[test]
fn ids_are_slugged_and_deduplicated_and_clashes_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let (s, reg, _) = store(tmp.path());
    let a = tmp.path().join("a/notes");
    let b = tmp.path().join("b/notes");
    let c = tmp.path().join("c/Poznámky");
    for d in [&a, &b, &c] {
        fs::create_dir_all(d).unwrap();
    }
    assert_eq!(s.add(&add(&a)).unwrap().id, "notes");
    assert_eq!(s.add(&add(&b)).unwrap().id, "notes-2");
    assert_eq!(s.add(&add(&c)).unwrap().id, "poznamky");

    let before = fs::read(&reg).unwrap();
    let d = tmp.path().join("d");
    fs::create_dir_all(&d).unwrap();
    let e = s
        .add(&AddCorpus {
            id: Some("notes".into()),
            ..add(&d)
        })
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("`notes`") && e.contains("already registered"),
        "{e}"
    );
    let e = s
        .add(&AddCorpus {
            id: Some("bad id".into()),
            ..add(&d)
        })
        .unwrap_err()
        .to_string();
    assert!(e.contains("must be ASCII"), "{e}");
    assert_eq!(fs::read(&reg).unwrap(), before);
}

#[test]
fn same_path_twice_is_refused_naming_the_id() {
    let tmp = tempfile::tempdir().unwrap();
    let (s, reg, _) = store(tmp.path());
    let notes = tmp.path().join("notes");
    fs::create_dir_all(&notes).unwrap();
    s.add(&add(&notes)).unwrap();
    let before = fs::read(&reg).unwrap();
    // Same folder, written differently.
    let again = format!("{}/../notes/", notes.display());
    let e = s
        .add(&AddCorpus {
            path: again,
            ..AddCorpus::default()
        })
        .unwrap_err()
        .to_string();
    assert!(e.contains("already registered as `notes`"), "{e}");
    assert_eq!(fs::read(&reg).unwrap(), before);
}

#[test]
fn missing_folder_or_a_file_is_refused_and_registry_untouched() {
    let tmp = tempfile::tempdir().unwrap();
    let (s, reg, _) = store(tmp.path());
    fs::create_dir_all(reg.parent().unwrap()).unwrap();
    let original = three_entry_registry(tmp.path());
    fs::write(&reg, &original).unwrap();

    let e = s
        .add(&add(&tmp.path().join("nope")))
        .unwrap_err()
        .to_string();
    assert!(e.contains("no folder at"), "{e}");
    let file = tmp.path().join("f.md");
    fs::write(&file, "x").unwrap();
    let e = s.add(&add(&file)).unwrap_err().to_string();
    assert!(e.contains("is a file"), "{e}");
    let e = s
        .add(&AddCorpus {
            include: vec!["[".into()],
            ..add(tmp.path())
        })
        .unwrap_err()
        .to_string();
    assert!(e.contains("bad glob"), "{e}");
    assert_eq!(fs::read_to_string(&reg).unwrap(), original);
}

#[test]
fn missing_folder_on_a_fresh_registry_writes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let (s, reg, _) = store(tmp.path());
    assert!(s.add(&add(&tmp.path().join("nope"))).is_err());
    assert!(!reg.exists());
}

#[test]
fn corrupt_registry_is_refused_with_a_parse_error_and_left_alone() {
    let tmp = tempfile::tempdir().unwrap();
    let (s, reg, _) = store(tmp.path());
    fs::create_dir_all(reg.parent().unwrap()).unwrap();
    let corrupt = "# mine\n[[corpus]\nid = \"x\n";
    fs::write(&reg, corrupt).unwrap();
    let e = format!("{:#}", s.add(&add(tmp.path())).unwrap_err());
    assert!(e.contains("parse corpus registry"), "{e}");
    assert_eq!(fs::read_to_string(&reg).unwrap(), corrupt);
}

#[test]
fn interrupted_write_leaves_the_original_and_no_temp_file() {
    let tmp = tempfile::tempdir().unwrap();
    let (s, reg, _) = store(tmp.path());
    fs::create_dir_all(reg.parent().unwrap()).unwrap();
    let original = three_entry_registry(tmp.path());
    fs::write(&reg, &original).unwrap();
    let s = s.with_rename(|_, _| Err(std::io::Error::other("simulated crash before rename")));
    let notes = tmp.path().join("notes");
    fs::create_dir_all(&notes).unwrap();

    assert!(s.add(&add(&notes)).is_err());
    assert!(s.remove("tweets", true).is_err());
    assert_eq!(fs::read_to_string(&reg).unwrap(), original);
    let names: Vec<String> = fs::read_dir(reg.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, vec!["corpora.toml"]);
}

#[test]
fn remove_cuts_the_entry_and_deletes_only_its_index_files() {
    let tmp = tempfile::tempdir().unwrap();
    let (s, reg, idx) = store(tmp.path());
    fs::create_dir_all(reg.parent().unwrap()).unwrap();
    let original = three_entry_registry(tmp.path());
    fs::write(&reg, &original).unwrap();
    fs::create_dir_all(&idx).unwrap();
    for f in [
        "writing.sqlite",
        "writing.sqlite-wal",
        "writing.vectors.sqlite",
        "tweets.sqlite",
        "tweets.vectors.sqlite",
    ] {
        fs::write(idx.join(f), "x").unwrap();
    }
    let before = load(&reg);

    let r = s.remove("writing", false).unwrap();
    assert_eq!(r.id, "writing");
    assert!(!r.index_kept);
    assert_eq!(r.deleted.len(), 3);
    let after = load(&reg);
    assert_eq!(after.corpora, before.corpora[1..]);
    let text = fs::read_to_string(&reg).unwrap();
    assert!(
        text.starts_with(
            "# Scout corpus registry. One [[corpus]] per corpus.\n# Hand-edited: keep this note.\n"
        ),
        "{text}"
    );
    assert!(!idx.join("writing.sqlite").exists());
    assert!(!idx.join("writing.vectors.sqlite").exists());
    assert!(idx.join("tweets.sqlite").exists());
    assert!(tmp.path().join("w").is_dir(), "source folder kept");

    let r = s.remove("tweets", true).unwrap();
    assert!(r.index_kept && r.deleted.is_empty());
    assert!(idx.join("tweets.sqlite").exists());
    assert!(idx.join("tweets.vectors.sqlite").exists());
    assert_eq!(load(&reg).corpora, before.corpora[2..]);

    let e = s.remove("nope", false).unwrap_err().to_string();
    assert!(e.contains("unknown corpus \"nope\""), "{e}");
}

#[test]
fn remove_then_add_back_round_trips() {
    let tmp = tempfile::tempdir().unwrap();
    let (s, reg, _) = store(tmp.path());
    let notes = tmp.path().join("notes");
    fs::create_dir_all(&notes).unwrap();
    s.add(&add(&notes)).unwrap();
    s.remove("notes", false).unwrap();
    assert!(load(&reg).corpora.is_empty());
    assert_eq!(s.add(&add(&notes)).unwrap().id, "notes");
    assert_eq!(load(&reg).corpora.len(), 1);
}

#[test]
fn remove_on_missing_registry_is_registry_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let (s, _, _) = store(tmp.path());
    let e = s.remove("x", false).unwrap_err();
    assert!(e.downcast_ref::<scout_corpus::RegistryMissing>().is_some());
}

#[test]
fn kind_detection_recognises_a_highlight_scout_archive() {
    let tmp = tempfile::tempdir().unwrap();
    let plain = tmp.path().join("plain");
    let archive = tmp.path().join("archive");
    fs::create_dir_all(&plain).unwrap();
    fs::create_dir_all(archive.join("readings/works")).unwrap();
    assert_eq!(detect_kind(&plain), CorpusKind::MarkdownFolder);
    assert_eq!(detect_kind(&archive), CorpusKind::HighlightScoutArchive);

    let (s, _, _) = store(tmp.path());
    let cfg = s.add(&add(&archive)).unwrap();
    assert_eq!(cfg.kind, CorpusKind::HighlightScoutArchive);
    assert!(cfg.include.is_empty(), "the adapter's own default applies");
    let forced = s
        .add(&AddCorpus {
            kind: Some(CorpusKind::MarkdownFolder),
            ..add(&plain)
        })
        .unwrap();
    assert_eq!(forced.kind, CorpusKind::MarkdownFolder);
}

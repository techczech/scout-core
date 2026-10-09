//! Seam: registry read-modify-write (`RegistryStore::add` / `remove`, the
//! bodies of `api::add_corpus` / `api::remove_corpus`) and kind detection.
//! Every test works in a temp dir; the user registry is never read.

use scout_corpus::register::RegistryStore;
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
            ..add(&{
                let fresh = tmp.path().join("fresh");
                fs::create_dir_all(&fresh).unwrap();
                fresh
            })
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
    let notes = tmp.path().join("notes");
    fs::create_dir_all(&notes).unwrap();
    let e = format!("{:#}", s.add(&add(&notes)).unwrap_err());
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
    let mut names = names;
    names.sort();
    assert_eq!(
        names,
        vec![".corpora.toml.lock", "corpora.toml"],
        "the lock file stays; no temp file"
    );
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

fn archive_at(root: &Path) {
    fs::create_dir_all(root.join("readings/works")).unwrap();
    fs::create_dir_all(root.join("readings/fulltext")).unwrap();
    fs::write(
        root.join("readings/works/a-work.md"),
        "---\ntitle: A Work\nsource_system: readwise\nsource_id: \"42\"\n---\n\n> a highlight\n",
    )
    .unwrap();
}

#[test]
fn kind_defaults_to_markdown_folder_even_for_an_archive_layout() {
    let tmp = tempfile::tempdir().unwrap();
    let archive = tmp.path().join("archive");
    archive_at(&archive);
    let (s, _, _) = store(tmp.path());
    let cfg = s.add(&add(&archive)).unwrap();
    assert_eq!(cfg.kind, CorpusKind::MarkdownFolder, "never guessed");
    assert_eq!(cfg.include, vec!["**/*.md", "**/*.txt"]);
}

#[test]
fn an_explicit_archive_kind_needs_readings_works() {
    let tmp = tempfile::tempdir().unwrap();
    let (s, reg, _) = store(tmp.path());
    let plain = tmp.path().join("plain");
    fs::create_dir_all(&plain).unwrap();
    let e = s
        .add(&AddCorpus {
            kind: Some(CorpusKind::HighlightScoutArchive),
            ..add(&plain)
        })
        .unwrap_err()
        .to_string();
    assert!(e.contains("no readings/works/"), "{e}");
    assert!(!reg.exists());

    let archive = tmp.path().join("archive");
    archive_at(&archive);
    let cfg = s
        .add(&AddCorpus {
            kind: Some(CorpusKind::HighlightScoutArchive),
            ..add(&archive)
        })
        .unwrap();
    assert_eq!(cfg.kind, CorpusKind::HighlightScoutArchive);
    assert!(cfg.include.is_empty(), "the adapter's own default applies");
}

/// The entries of a folder, sorted.
fn names_in(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

#[test]
fn a_new_folder_holding_the_registry_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("corpus");
    fs::create_dir_all(src.join("notes")).unwrap();
    // The registry file would live inside the folder being added.
    let reg = src.join("config/corpora.toml");
    let s = RegistryStore::new(&reg, tmp.path().join("indexes"));
    let e = s.add(&add(&src)).unwrap_err().to_string();
    assert!(e.contains("lies inside the source folder"), "{e}");
    assert!(!src.join("config").exists(), "no lock or registry written");
}

#[test]
fn a_registry_inside_a_registered_source_refuses_every_write() {
    let tmp = tempfile::tempdir().unwrap();
    let corpus = tmp.path().join("corpus");
    fs::create_dir_all(&corpus).unwrap();
    let corpus = corpus.canonicalize().unwrap();
    // Written by hand: /corpus/corpora.toml registers /corpus.
    let reg = corpus.join("corpora.toml");
    let text = format!(
        "[[corpus]]\nid = \"corpus\"\nname = \"Corpus\"\nkind = \"markdown-folder\"\npath = \"{}\"\n",
        corpus.display()
    );
    fs::write(&reg, &text).unwrap();
    let idx = tmp.path().join("indexes");
    fs::create_dir_all(&idx).unwrap();
    fs::write(idx.join("corpus.sqlite"), "x").unwrap();
    let s = RegistryStore::new(&reg, &idx);
    let other = tmp.path().join("other");
    fs::create_dir_all(&other).unwrap();

    let e = s.add(&add(&other)).unwrap_err().to_string();
    assert!(e.contains("lies inside the source folder"), "{e}");
    let e = s.remove("corpus", false).unwrap_err().to_string();
    assert!(e.contains("lies inside the source folder"), "{e}");
    let e = scout_corpus::registry::init_defaults(&reg, true, scout_corpus::Preset::default())
        .unwrap_err()
        .to_string();
    assert!(e.contains("lies inside the source folder"), "{e}");

    assert_eq!(
        names_in(&corpus),
        vec!["corpora.toml"],
        "no lock or temp file"
    );
    assert_eq!(fs::read_to_string(&reg).unwrap(), text);
    assert!(idx.join("corpus.sqlite").exists());
}

#[test]
fn a_registry_beside_a_source_folder_is_fine() {
    // SCOUT_CONFIG=/data/corpora.toml, then `corpora add /data/notes`.
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let notes = data.join("notes");
    fs::create_dir_all(&notes).unwrap();
    let reg = data.join("corpora.toml");
    let s = RegistryStore::new(&reg, tmp.path().join("indexes"));
    assert_eq!(s.add(&add(&notes)).unwrap().id, "notes");
    assert_eq!(load(&reg).corpora.len(), 1);
    assert_eq!(
        names_in(&notes),
        Vec::<String>::new(),
        "nothing written in the source"
    );
}

#[cfg(unix)]
#[test]
fn a_symlinked_lock_file_is_refused_and_never_followed() {
    let tmp = tempfile::tempdir().unwrap();
    let (s, reg, _) = store(tmp.path());
    let src = tmp.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::create_dir_all(reg.parent().unwrap()).unwrap();
    let planted = src.join("new.md");
    std::os::unix::fs::symlink(&planted, reg.parent().unwrap().join(".corpora.toml.lock")).unwrap();
    let notes = tmp.path().join("notes");
    fs::create_dir_all(&notes).unwrap();
    let e = s.add(&add(&notes)).unwrap_err().to_string();
    assert!(e.contains("symlink"), "{e}");
    assert!(
        !planted.exists(),
        "the dangling link's target was not created"
    );
    assert!(!reg.exists());
}

#[cfg(unix)]
#[test]
fn a_symlinked_registry_is_not_replaced() {
    let tmp = tempfile::tempdir().unwrap();
    let (s, reg, _) = store(tmp.path());
    fs::create_dir_all(reg.parent().unwrap()).unwrap();
    let real = tmp.path().join("dotfiles-corpora.toml");
    fs::write(&real, "# mine\n").unwrap();
    std::os::unix::fs::symlink(&real, &reg).unwrap();
    let notes = tmp.path().join("notes");
    fs::create_dir_all(&notes).unwrap();
    let e = s.add(&add(&notes)).unwrap_err().to_string();
    assert!(e.contains("is a symlink"), "{e}");
    assert!(fs::symlink_metadata(&reg).unwrap().file_type().is_symlink());
    assert_eq!(fs::read_to_string(&real).unwrap(), "# mine\n");
}

#[cfg(unix)]
#[test]
fn removing_through_a_symlinked_registry_deletes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let (_, reg, idx) = store(tmp.path());
    let notes = tmp.path().join("notes");
    fs::create_dir_all(&notes).unwrap();
    // A valid registry holding `notes`, reached through a symlink.
    let real = tmp.path().join("dotfiles-corpora.toml");
    RegistryStore::new(&real, &idx).add(&add(&notes)).unwrap();
    let text = fs::read_to_string(&real).unwrap();
    fs::create_dir_all(reg.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&real, &reg).unwrap();
    fs::create_dir_all(&idx).unwrap();
    fs::write(idx.join("notes.sqlite"), "x").unwrap();
    fs::write(idx.join("notes.vectors.sqlite"), "x").unwrap();

    let e = RegistryStore::new(&reg, &idx)
        .remove("notes", false)
        .unwrap_err()
        .to_string();
    assert!(e.contains("is a symlink"), "{e}");
    assert!(idx.join("notes.sqlite").exists());
    assert!(idx.join("notes.vectors.sqlite").exists());
    assert_eq!(fs::read_to_string(&real).unwrap(), text);
}

#[cfg(unix)]
#[test]
fn an_unsearchable_index_dir_fails_removal_and_keeps_the_entry() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let (s, reg, idx) = store(tmp.path());
    let notes = tmp.path().join("notes");
    fs::create_dir_all(&notes).unwrap();
    s.add(&add(&notes)).unwrap();
    fs::create_dir_all(&idx).unwrap();
    fs::write(idx.join("notes.sqlite"), "x").unwrap();
    let before = fs::read_to_string(&reg).unwrap();

    fs::set_permissions(&idx, fs::Permissions::from_mode(0o000)).unwrap();
    let res = s.remove("notes", false);
    fs::set_permissions(&idx, fs::Permissions::from_mode(0o755)).unwrap();
    let e = format!("{:#}", res.unwrap_err());
    assert!(
        e.contains("left registered") && e.contains("notes.sqlite"),
        "{e}"
    );
    assert_eq!(fs::read_to_string(&reg).unwrap(), before);

    let r = s.remove("notes", false).unwrap();
    assert_eq!(r.deleted.len(), 1);
    assert!(!idx.join("notes.sqlite").exists());
}

#[test]
fn remove_refuses_index_files_inside_a_source_folder_and_keeps_the_entry() {
    let tmp = tempfile::tempdir().unwrap();
    // The index store lies inside a registered source folder: written by
    // hand, since `add` refuses it.
    let data = tmp.path().join("data");
    let other = tmp.path().join("other");
    let idx = data.join("indexes");
    fs::create_dir_all(&idx).unwrap();
    fs::create_dir_all(&other).unwrap();
    let reg = tmp.path().join("config/corpora.toml");
    fs::create_dir_all(reg.parent().unwrap()).unwrap();
    let text = format!(
        "[[corpus]]\nid = \"notes\"\nname = \"Notes\"\nkind = \"markdown-folder\"\npath = \"{}\"\n\n\
         [[corpus]]\nid = \"other\"\nname = \"Other\"\nkind = \"markdown-folder\"\npath = \"{}\"\n",
        data.display(),
        other.display()
    );
    fs::write(&reg, &text).unwrap();
    for f in ["notes.sqlite", "other.sqlite"] {
        fs::write(idx.join(f), "x").unwrap();
    }
    let s = RegistryStore::new(&reg, &idx);

    // Its own source folder.
    let e = s.remove("notes", false).unwrap_err().to_string();
    assert!(
        e.contains("inside the source folder") && e.contains("left registered"),
        "{e}"
    );
    // Another corpus's source folder.
    let e = s.remove("other", false).unwrap_err().to_string();
    assert!(e.contains("of `notes`"), "{e}");
    assert!(idx.join("notes.sqlite").exists() && idx.join("other.sqlite").exists());
    assert_eq!(
        fs::read_to_string(&reg).unwrap(),
        text,
        "both still registered"
    );

    // --keep-index only edits the registry.
    let r = s.remove("other", true).unwrap();
    assert!(r.index_kept && r.deleted.is_empty());
    assert!(idx.join("other.sqlite").exists());
    assert_eq!(load(&reg).corpora.len(), 1);
}

#[test]
fn add_refuses_a_folder_holding_or_inside_the_index_store() {
    let tmp = tempfile::tempdir().unwrap();
    let (s, reg, idx) = store(tmp.path());
    fs::create_dir_all(idx.join("sub")).unwrap();
    let e = s.add(&add(tmp.path())).unwrap_err().to_string();
    assert!(e.contains("holds the index store"), "{e}");
    let e = s.add(&add(&idx.join("sub"))).unwrap_err().to_string();
    assert!(e.contains("inside the index store"), "{e}");
    // An index dir that does not exist yet is still guarded.
    let s2 = RegistryStore::new(&reg, tmp.path().join("data/later/indexes"));
    fs::create_dir_all(tmp.path().join("data")).unwrap();
    let e = s2
        .add(&add(&tmp.path().join("data")))
        .unwrap_err()
        .to_string();
    assert!(e.contains("holds the index store"), "{e}");
    assert!(!reg.exists());
}

#[cfg(unix)]
#[test]
fn a_planted_temp_symlink_is_never_followed() {
    let tmp = tempfile::tempdir().unwrap();
    let (s, reg, _) = store(tmp.path());
    let notes = tmp.path().join("notes");
    fs::create_dir_all(&notes).unwrap();
    s.add(&add(&notes)).unwrap();
    let victim = tmp.path().join("victim.md");
    fs::write(&victim, "precious").unwrap();
    // The temp name an attacker could predict: registry name and pid.
    let dir = reg.parent().unwrap();
    let pid = std::process::id();
    for name in [
        format!(".corpora.toml.{pid}.tmp"),
        format!(".corpora.toml.{pid}.0.tmp"),
        format!(".corpora.toml.{pid}.1.tmp"),
    ] {
        std::os::unix::fs::symlink(&victim, dir.join(name)).unwrap();
    }
    let more = tmp.path().join("more");
    fs::create_dir_all(&more).unwrap();
    s.add(&add(&more)).unwrap();
    s.remove("notes", false).unwrap();
    assert_eq!(fs::read_to_string(&victim).unwrap(), "precious");
    assert_eq!(load(&reg).corpora.len(), 1);
    assert!(fs::symlink_metadata(&reg).unwrap().file_type().is_file());
}

#[test]
fn concurrent_adds_all_land() {
    for _round in 0..5 {
        let tmp = tempfile::tempdir().unwrap();
        let (s, reg, _) = store(tmp.path());
        let dirs: Vec<PathBuf> = (0..8).map(|i| tmp.path().join(format!("c{i}"))).collect();
        for d in &dirs {
            fs::create_dir_all(d).unwrap();
        }
        std::thread::scope(|sc| {
            for d in &dirs {
                let s = s.clone();
                sc.spawn(move || s.add(&add(d)).unwrap());
            }
        });
        let mut ids: Vec<String> = load(&reg).corpora.into_iter().map(|c| c.id).collect();
        ids.sort();
        assert_eq!(ids, (0..8).map(|i| format!("c{i}")).collect::<Vec<_>>());
    }
}

#[test]
fn ids_differing_only_by_case_clash() {
    let tmp = tempfile::tempdir().unwrap();
    let (s, reg, idx) = store(tmp.path());
    let a = tmp.path().join("a");
    let b = tmp.path().join("b/Notes");
    for d in [&a, &b] {
        fs::create_dir_all(d).unwrap();
    }
    s.add(&AddCorpus {
        id: Some("NOTES".into()),
        ..add(&a)
    })
    .unwrap();
    let e = s
        .add(&AddCorpus {
            id: Some("notes".into()),
            ..add(&b)
        })
        .unwrap_err()
        .to_string();
    assert!(e.contains("already registered as `NOTES`"), "{e}");
    assert_eq!(
        s.add(&add(&b)).unwrap().id,
        "notes-2",
        "slug skips the case clash"
    );

    // A registry that already has case-only duplicates loads as it is, but
    // removing one never deletes the index the other shares.
    let c = tmp.path().join("c");
    fs::create_dir_all(&c).unwrap();
    let text = fs::read_to_string(&reg).unwrap()
        + &format!(
            "\n[[corpus]]\nid = \"Notes\"\nname = \"c\"\nkind = \"markdown-folder\"\npath = \"{}\"\n",
            c.display()
        );
    fs::write(&reg, &text).unwrap();
    assert_eq!(load(&reg).corpora.len(), 3);
    fs::create_dir_all(&idx).unwrap();
    fs::write(idx.join("NOTES.sqlite"), "x").unwrap();
    let e = s.remove("Notes", false).unwrap_err().to_string();
    assert!(e.contains("shared with `NOTES`"), "{e}");
    assert_eq!(fs::read_to_string(&reg).unwrap(), text);
    assert!(idx.join("NOTES.sqlite").exists());
    s.remove("Notes", true).unwrap();
    assert_eq!(load(&reg).corpora.len(), 2);
}

#[cfg(unix)]
#[test]
fn a_symlink_is_resolved_before_dot_dot() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    let (s, _, _) = store(&base);
    for d in ["a/notes", "b/sub", "b/notes"] {
        fs::create_dir_all(base.join(d)).unwrap();
    }
    std::os::unix::fs::symlink(base.join("b/sub"), base.join("a/link")).unwrap();
    let cfg = s
        .add(&AddCorpus {
            path: format!("{}/a/link/../notes", base.display()),
            ..AddCorpus::default()
        })
        .unwrap();
    assert_eq!(
        PathBuf::from(&cfg.path),
        base.join("b/notes"),
        "a `..` path is stored canonical"
    );
}

#[cfg(unix)]
#[test]
fn a_folder_added_through_a_symlink_keeps_the_symlink_spelling() {
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    let (s, _, _) = store(&base);
    fs::create_dir_all(base.join("cloud/Shared Notes")).unwrap();
    std::os::unix::fs::symlink(base.join("cloud"), base.join("stable")).unwrap();
    let via_link = base.join("stable/Shared Notes");
    let cfg = s
        .add(&AddCorpus {
            path: format!("{}/./", via_link.display()),
            ..AddCorpus::default()
        })
        .unwrap();
    assert_eq!(
        PathBuf::from(&cfg.path),
        via_link,
        "symlink spelling stored"
    );
    assert_eq!(cfg.id, "shared-notes");
    // The same folder by its real path is still a duplicate.
    let e = s
        .add(&add(&base.join("cloud/Shared Notes")))
        .unwrap_err()
        .to_string();
    assert!(e.contains("already registered as `shared-notes`"), "{e}");
}

#[cfg(unix)]
#[test]
fn a_failed_index_deletion_leaves_the_corpus_registered_for_a_retry() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let (s, reg, idx) = store(tmp.path());
    let notes = tmp.path().join("notes");
    fs::create_dir_all(&notes).unwrap();
    s.add(&add(&notes)).unwrap();
    fs::create_dir_all(&idx).unwrap();
    fs::write(idx.join("notes.sqlite"), "x").unwrap();
    fs::write(idx.join("notes.vectors.sqlite"), "x").unwrap();
    let before = fs::read_to_string(&reg).unwrap();

    fs::set_permissions(&idx, fs::Permissions::from_mode(0o555)).unwrap();
    let e = s.remove("notes", false).unwrap_err().to_string();
    fs::set_permissions(&idx, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        e.contains("left registered") && e.contains("notes.sqlite"),
        "{e}"
    );
    assert_eq!(fs::read_to_string(&reg).unwrap(), before);

    let r = s.remove("notes", false).unwrap();
    assert_eq!(r.deleted.len(), 2);
    assert!(load(&reg).corpora.is_empty());
}

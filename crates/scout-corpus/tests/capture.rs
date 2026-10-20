use scout_corpus::api::{CaptureQuery, Engine, IndexBuildQuery, SearchQuery};
use scout_corpus::Registry;
use std::{fs, path::Path};
use yaml_rust2::{Yaml, YamlLoader};

fn request(project: &Path) -> CaptureQuery {
    CaptureQuery {
        project: project.to_str().unwrap().into(),
        kind: "evidence".into(),
        title: "Hebb's rule: Žluťoučký".into(),
        body: "Connections strengthen.\n\nA second passage.\n".into(),
        ..Default::default()
    }
}
fn engine(root: &Path) -> Engine {
    fs::create_dir_all(root.join("vault/talk")).unwrap();
    let reg = Registry::parse(&format!(
        "[[corpus]]\nid='talks'\nname='Talks'\nkind='markdown-folder'\npath={:?}\n",
        root.join("vault").to_str().unwrap()
    ))
    .unwrap();
    Engine::new(reg, root.join("indexes"))
}
fn front(path: &str) -> (Yaml, String) {
    let text = fs::read_to_string(path).unwrap();
    let parts: Vec<_> = text.splitn(3, "---").collect();
    (
        YamlLoader::load_from_str(parts[1]).unwrap().remove(0),
        parts[2].strip_prefix('\n').unwrap().into(),
    )
}

#[test]
fn capture_writes_frontmatter_body_receipt_and_incremental_searchable_index() {
    let t = tempfile::tempdir().unwrap();
    let e = engine(t.path());
    let project = t.path().join("vault/talk");
    let first = e.index_build(&IndexBuildQuery::default()).unwrap();
    assert!(first.body.reports[0].full);
    let mut q = request(&project);
    q.prompt = Some("A question: \"why\"?\nSecond line".into());
    q.sources = vec![
        "scout:highlights:readings/works/hebb-1949.md:12".into(),
        "https://example.org/paper".into(),
    ];
    q.by = "app:talkweaver".into();
    let result = e.capture(&q).unwrap();
    assert!(result.notes.is_empty());
    let r = result.body;
    assert!(r.file.ends_with("-hebb-s-rule-zlutoucky.md"));
    assert_eq!(r.corpus.as_deref(), Some("talks"));
    assert!(r.manifest && r.indexed);
    assert_eq!(r.passages, 2);
    let (fm, body) = front(&r.file);
    assert_eq!(body, q.body);
    assert_eq!(fm["title"].as_str(), Some(q.title.as_str()));
    assert_eq!(fm["kind"].as_str(), Some("evidence"));
    assert_eq!(fm["project"].as_str(), Some("talk"));
    assert_eq!(fm["prompt"].as_str(), q.prompt.as_deref());
    let created = fm["created"].as_str().unwrap();
    assert!(created.ends_with('Z'));
    assert_eq!(created.len(), 20);
    chrono::DateTime::parse_from_rfc3339(created).unwrap();
    assert_eq!(
        fm["sources"][0]["scout"].as_str(),
        Some("highlights:readings/works/hebb-1949.md:12")
    );
    let receipts = e
        .capture_list(project.to_str().unwrap())
        .unwrap()
        .body
        .entries;
    assert_eq!(receipts.len(), 1);
    assert_eq!(receipts[0].by, "app:talkweaver");
    assert_eq!(receipts[0].created, created);
    assert_eq!(
        receipts[0].file,
        Path::new(&r.file).file_name().unwrap().to_str().unwrap()
    );
    assert_eq!(receipts[0].sources[1]["url"], "https://example.org/paper");
    assert!(
        e.search(&SearchQuery {
            query: "connections".into(),
            ..Default::default()
        })
        .unwrap()
        .body
        .total_documents
            > 0
    );
    let after = e.index_build(&IndexBuildQuery::default()).unwrap();
    assert!(!after.body.reports[0].full);
    assert_eq!(after.body.reports[0].indexed, 0);
}

#[test]
fn clashes_append_without_overwriting_and_unknown_frontmatter_survives() {
    let t = tempfile::tempdir().unwrap();
    let e = engine(t.path());
    let mut q = request(&t.path().join("vault/talk"));
    q.body = "---\ncustom:\n  nested: [one, two]\ntitle: old\n---\nExact body.\n".into();
    let a = e.capture(&q).unwrap().body;
    let before = fs::read(&a.file).unwrap();
    let b = e.capture(&q).unwrap().body;
    let c = e.capture(&q).unwrap().body;
    assert!(b.file.ends_with("-2.md"));
    assert!(c.file.ends_with("-3.md"));
    assert_eq!(fs::read(&a.file).unwrap(), before);
    let (fm, body) = front(&a.file);
    assert_eq!(fm["custom"]["nested"][1].as_str(), Some("two"));
    assert_eq!(body, "Exact body.\n");
    assert_eq!(e.capture_list(&q.project).unwrap().body.entries.len(), 3);
    assert_eq!(
        e.capture_list(&q.project).unwrap().body.entries[0].by,
        "verb"
    );
}

#[test]
fn artefacts_are_copied_outside_project_and_referenced_inside() {
    let t = tempfile::tempdir().unwrap();
    let e = engine(t.path());
    let project = t.path().join("vault/talk");
    let external = t.path().join("widget.html");
    fs::write(&external, "<p>Widget</p>").unwrap();
    let mut q = request(&project);
    q.kind = "widget".into();
    q.artefact = Some(external.to_str().unwrap().into());
    q.sources = vec![format!("file:{}", external.display())];
    let r = e.capture(&q).unwrap().body;
    let (fm, _) = front(&r.file);
    let copied = Path::new(&r.file).with_extension("html");
    assert_eq!(
        fm["artefact"].as_str(),
        copied
            .strip_prefix(project.canonicalize().unwrap())
            .unwrap()
            .to_str()
    );
    assert_eq!(fs::read(&copied).unwrap(), fs::read(&external).unwrap());
    assert_eq!(fm["sources"][0]["file"].as_str(), external.to_str());
    let collision = e.capture(&q).unwrap().body;
    assert!(collision.file.ends_with("-2.md"));
    assert!(Path::new(&collision.file).with_extension("html").exists());
    fs::create_dir(project.join("assets")).unwrap();
    let inside = project.join("assets/picture.png");
    fs::write(&inside, b"picture").unwrap();
    q.artefact = Some(inside.to_str().unwrap().into());
    q.sources = vec![format!("file:{}", inside.display())];
    let r = e.capture(&q).unwrap().body;
    let (fm, _) = front(&r.file);
    assert_eq!(fm["artefact"].as_str(), Some("assets/picture.png"));
    assert_eq!(
        fm["sources"][0]["file"].as_str(),
        Some("assets/picture.png")
    );
    assert!(!Path::new(&r.file).with_extension("png").exists());
}

#[test]
fn outside_every_corpus_writes_and_index_failure_keeps_capture_successful() {
    let t = tempfile::tempdir().unwrap();
    let e = engine(t.path());
    let project = t.path().join("outside");
    fs::create_dir(&project).unwrap();
    let q = request(&project);
    let reply = e.capture(&q).unwrap();
    let r = reply.body;
    assert_eq!(r.corpus, None);
    assert!(r.manifest);
    assert!(!r.indexed);
    assert_eq!(r.passages, 0);
    assert!(!reply.notes.is_empty());
    assert_eq!(
        front(&r.file).0["project"].as_str(),
        project.canonicalize().unwrap().to_str()
    );
    // Existing index-target guard must apply to capture's build too.
    let bad = Engine::new(e.registry().clone(), t.path().join("vault/indexes"));
    let r = bad.capture(&request(&t.path().join("vault/talk"))).unwrap();
    assert!(!r.body.indexed);
    assert!(r.body.manifest);
    assert!(Path::new(&r.body.file).is_file());
    assert!(r.notes.iter().any(|s| s.contains("index build failed")));
}

#[test]
fn path_lookup_chooses_nested_corpus_and_not_text_prefix() {
    let t = tempfile::tempdir().unwrap();
    let e = engine(t.path());
    let nested = t.path().join("vault/talk");
    let extra = format!(
        "[[corpus]]\nid='nested'\nname='Nested'\nkind='markdown-folder'\npath={:?}\n",
        nested.to_str().unwrap()
    );
    let mut reg = e.registry().clone();
    reg.corpora.extend(Registry::parse(&extra).unwrap().corpora);
    assert_eq!(reg.containing(&nested).unwrap().0.id, "nested");
    let outside = t.path().join("vault-other");
    fs::create_dir(&outside).unwrap();
    assert!(reg.containing(&outside).is_none());
    let e = Engine::new(reg, t.path().join("indexes"));
    let r = e.capture(&request(&nested)).unwrap().body;
    assert_eq!(r.corpus.as_deref(), Some("nested"));
    assert_eq!(front(&r.file).0["project"].as_str(), Some("."));
}

#[test]
fn list_and_where_used_read_receipts_and_outline_mentions() {
    let t = tempfile::tempdir().unwrap();
    let e = engine(t.path());
    let project = t.path().join("vault/talk");
    assert!(e
        .capture_list(project.to_str().unwrap())
        .unwrap()
        .body
        .entries
        .is_empty());
    let r = e.capture(&request(&project)).unwrap().body;
    let file = Path::new(&r.file).file_name().unwrap().to_str().unwrap();
    fs::write(
        project.join("talk-outline.md"),
        format!("# Talk\n\nSee research/{file}\n"),
    )
    .unwrap();
    fs::write(t.path().join("vault/piece.md"), format!("Reuse {file}\n")).unwrap();
    let hits = e.capture_where_used(&r.file).unwrap().body.hits;
    assert_eq!(hits.len(), 2);
    assert!(hits
        .iter()
        .any(|h| h.line == 3 && h.file.ends_with("talk-outline.md")));
    assert!(e
        .capture_where_used("absent.png")
        .unwrap()
        .body
        .hits
        .is_empty());
}

#[test]
fn invalid_requests_leave_no_capture_and_existing_files_are_never_overwritten() {
    let t = tempfile::tempdir().unwrap();
    let e = engine(t.path());
    let project = t.path().join("vault/talk");
    let q = request(&project);
    for bad in [
        CaptureQuery {
            kind: "other".into(),
            ..q.clone()
        },
        CaptureQuery {
            title: " ".into(),
            ..q.clone()
        },
        CaptureQuery {
            slug: Some("../escape".into()),
            ..q.clone()
        },
        CaptureQuery {
            by: "unknown".into(),
            ..q.clone()
        },
        CaptureQuery {
            project: t.path().join("missing").display().to_string(),
            ..q.clone()
        },
    ] {
        assert!(e.capture(&bad).is_err());
    }
    assert!(!project.join("research").exists());
}

#[cfg(unix)]
#[test]
fn symlink_containment_and_manifest_failures_do_not_write_through_links() {
    use std::os::unix::fs::symlink;
    let t = tempfile::tempdir().unwrap();
    let e = engine(t.path());
    let project = t.path().join("vault/talk");
    let external = t.path().join("external");
    fs::create_dir(&external).unwrap();
    symlink(&external, project.join("research")).unwrap();
    assert!(e.capture(&request(&project)).is_err());
    assert_eq!(fs::read_dir(&external).unwrap().count(), 0);
    fs::remove_file(project.join("research")).unwrap();
    fs::create_dir(project.join("research")).unwrap();
    let target = external.join("receipt");
    fs::write(&target, "untouched\n").unwrap();
    symlink(&target, project.join("research/_manifest.jsonl")).unwrap();
    let r = e.capture(&request(&project)).unwrap();
    assert!(!r.body.manifest);
    assert!(Path::new(&r.body.file).exists());
    assert_eq!(fs::read_to_string(&target).unwrap(), "untouched\n");
    assert!(r.notes.iter().any(|n| n.contains("manifest not appended")));
}

#[test]
fn missing_provenance_files_are_allowed_and_unfinished_frontmatter_is_refused() {
    let t = tempfile::tempdir().unwrap();
    let e = engine(t.path());
    let project = t.path().join("vault/talk");
    let mut q = request(&project);
    q.body = "---".into();
    assert!(e.capture(&q).is_err());
    q.body = "Text\n".into();
    q.sources = vec!["file:assets/absent.png".into()];
    let r = e.capture(&q).unwrap();
    assert_eq!(
        front(&r.body.file).0["sources"][0]["file"].as_str(),
        Some("assets/absent.png")
    );
}

#[test]
fn concurrent_captures_append_whole_receipts_and_choose_distinct_names() {
    let t = tempfile::tempdir().unwrap();
    let project = t.path().join("project");
    fs::create_dir(&project).unwrap();
    let e = Engine::new(Registry::default(), t.path().join("indexes"));
    let q = request(&project);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let e = e.clone();
            let q = q.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                e.capture(&q).unwrap().body
            })
        })
        .collect();
    let results: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert!(results.iter().all(|r| r.manifest));
    let names: std::collections::BTreeSet<_> = results.iter().map(|r| &r.file).collect();
    assert_eq!(names.len(), 8);
    assert_eq!(e.capture_list(&q.project).unwrap().body.entries.len(), 8);
}

#[test]
fn registry_lookup_expands_home_roots() {
    let home = dirs::home_dir().unwrap();
    let reg =
        Registry::parse("[[corpus]]\nid='home'\nname='Home'\nkind='markdown-folder'\npath='~'\n")
            .unwrap();
    assert_eq!(reg.containing(&home).unwrap().0.id, "home");
    assert_eq!(
        reg.containing(&home).unwrap().1,
        home.canonicalize().unwrap()
    );
}

#[test]
fn bounded_slugs_bom_sources_order_and_lexical_provenance() {
    let t = tempfile::tempdir().unwrap();
    let e = engine(t.path());
    let project = t.path().join("vault/talk");
    let mut q = request(&project);
    q.title = format!("{} tail words", "longword ".repeat(9));
    q.body = "\u{feff}---\ntitle: old\ncustom: first\nsecond: next\nsources:\n  - scout:talks:one:1\n  - https://example.org\n  - assets/missing.png\n---\nBody.\n".into();
    q.sources = vec!["file:assets/../missing.png".into()];
    let result = e.capture(&q).unwrap().body;
    let name = Path::new(&result.file)
        .file_stem()
        .unwrap()
        .to_str()
        .unwrap();
    assert!(name[11..].len() <= 60);
    assert!(!name.ends_with('-'));
    assert_eq!(
        &name[11..],
        "longword-longword-longword-longword-longword-longword"
    );
    let text = fs::read_to_string(&result.file).unwrap();
    assert!(text.starts_with("\u{feff}---\ncustom:"));
    assert!(text.find("second:").unwrap() < text.find("title:").unwrap());
    let (fm, body) = front(&result.file);
    assert_eq!(body, "Body.\n");
    assert_eq!(fm["sources"][0]["scout"].as_str(), Some("talks:one:1"));
    assert_eq!(
        fm["sources"][1]["url"].as_str(),
        Some("https://example.org")
    );
    assert_eq!(
        fm["sources"][2]["file"].as_str(),
        Some("assets/missing.png")
    );
    assert_eq!(fm["sources"][3]["file"].as_str(), Some("missing.png"));
    q.slug = Some("a".repeat(61));
    assert!(e.capture(&q).unwrap_err().to_string().contains("60"));
    q.slug = None;
    q.sources = vec!["file:".into()];
    assert!(e.capture(&q).is_err());
}

#[test]
fn frontmatter_string_sources_take_the_same_prefixes_as_flags() {
    let t = tempfile::tempdir().unwrap();
    let e = engine(t.path());
    let project = t.path().join("vault/talk");
    let mut q = request(&project);
    q.body = "---\nsources:\n  - url:https://f.org/x\n  - file:notes/a.md\n---\nBody.\n".into();
    let result = e.capture(&q).unwrap().body;
    let (fm, _) = front(&result.file);
    assert_eq!(fm["sources"][0]["url"].as_str(), Some("https://f.org/x"));
    assert_eq!(fm["sources"][1]["file"].as_str(), Some("notes/a.md"));
}

#[test]
fn frontmatter_artefact_is_confined_and_markdown_copies_use_file_suffix() {
    let t = tempfile::tempdir().unwrap();
    let e = engine(t.path());
    let project = t.path().join("vault/talk");
    let mut q = request(&project);
    let external = t.path().join("external.MD");
    fs::write(&external, "Artefact").unwrap();
    q.body = format!(
        "---\nartefact: {:?}\n---\nBody\n",
        external.to_str().unwrap()
    );
    assert!(e.capture(&q).is_err());
    q.body = "---\nartefact: ../escape.html\n---\nBody\n".into();
    assert!(e.capture(&q).is_err());
    q.body = "Body\n".into();
    q.artefact = Some(external.display().to_string());
    let result = e.capture(&q).unwrap().body;
    let (fm, _) = front(&result.file);
    let stub = Path::new(&result.file)
        .file_stem()
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(
        fm["artefact"].as_str().unwrap(),
        format!("research/{stub}-file.MD")
    );
    assert_eq!(
        fs::read_to_string(project.join(fm["artefact"].as_str().unwrap())).unwrap(),
        "Artefact"
    );
    assert!(project.join("research/.manifest.lock").exists());
    assert!(!fs::read_dir(project.join("research")).unwrap().any(|e| e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with("._")));
    fs::create_dir_all(project.join("assets")).unwrap();
    fs::write(project.join("assets/ok.html"), "ok").unwrap();
    q.artefact = None;
    q.body = "---\nartefact: assets/ok.html\n---\nBody\n".into();
    let result = e.capture(&q).unwrap().body;
    assert_eq!(
        front(&result.file).0["artefact"].as_str(),
        Some("assets/ok.html")
    );
}

#[test]
fn indexed_and_passages_describe_only_the_stub_and_respect_exclusions() {
    let t = tempfile::tempdir().unwrap();
    let e = engine(t.path());
    let project = t.path().join("vault/talk");
    e.capture(&request(&project)).unwrap();
    let mut q = request(&project);
    q.body = "Single passage.\n".into();
    assert_eq!(e.capture(&q).unwrap().body.passages, 1);
    let mut reg = e.registry().clone();
    reg.corpora[0].exclude = vec!["**/research/**".into()];
    let excluded = Engine::new(reg, t.path().join("excluded-index"));
    let result = excluded.capture(&q).unwrap().body;
    assert!(!result.indexed);
    assert_eq!(result.passages, 0);
}

#[test]
fn where_used_honours_kind_globs_research_exclusion_and_skips_bad_files() {
    let t = tempfile::tempdir().unwrap();
    let e = engine(t.path());
    let root = t.path().join("vault");
    for folder in ["included", "excluded", "research", "archive"] {
        fs::create_dir(root.join(folder)).unwrap();
    }
    for file in [
        "included/hit.md",
        "excluded/no.md",
        "research/self.md",
        "archive/no.md",
        "outside.md",
    ] {
        fs::write(root.join(file), "mention needle.md\n").unwrap();
    }
    fs::write(root.join("included/bad.md"), [255, 254]).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::write(root.join("included/unreadable.md"), "needle.md").unwrap();
        fs::set_permissions(
            root.join("included/unreadable.md"),
            fs::Permissions::from_mode(0o000),
        )
        .unwrap();
    }
    let mut reg = e.registry().clone();
    reg.corpora[0].include = vec![
        "included/**".into(),
        "excluded/**".into(),
        "research/**".into(),
    ];
    reg.corpora[0].exclude = vec!["excluded/**".into()];
    let mut archive = reg.corpora[0].clone();
    archive.id = "archive".into();
    archive.path = root.join("archive").display().to_string();
    archive.kind = scout_corpus::CorpusKind::HighlightScoutArchive;
    archive.include.clear();
    archive.exclude.clear();
    reg.corpora.push(archive);
    let e = Engine::new(reg, t.path().join("indexes"));
    let used = e.capture_where_used("needle.md").unwrap().body;
    assert_eq!(used.hits.len(), 1);
    assert!(used.hits[0].file.ends_with("included/hit.md"));
    assert_eq!(used.schema_version, 1);
    #[cfg(unix)]
    assert_eq!(used.skipped, 2);
    #[cfg(not(unix))]
    assert_eq!(used.skipped, 1);
}

#[test]
fn artefact_stem_and_stub_clashes_are_case_insensitive() {
    let t = tempfile::tempdir().unwrap();
    let e = engine(t.path());
    let project = t.path().join("vault/talk");
    let mut q = request(&project);
    q.slug = Some("case".into());
    let research = project.join("research");
    fs::create_dir(&research).unwrap();
    let date = chrono::Utc::now().format("%Y-%m-%d");
    let planted = research.join(format!("{date}-CASE.MD"));
    fs::write(&planted, "untouched").unwrap();
    let ext = t.path().join("thing.Md");
    fs::write(&ext, "copy").unwrap();
    q.artefact = Some(ext.display().to_string());
    let result = e.capture(&q).unwrap().body;
    assert!(result.file.ends_with("-case-2.md"));
    assert!(front(&result.file).0["artefact"]
        .as_str()
        .unwrap()
        .ends_with("-case-2-file.Md"));
    assert_eq!(fs::read_to_string(planted).unwrap(), "untouched");
}

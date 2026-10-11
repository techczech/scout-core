//! Review fixes for the topic layer (Sol 6.1, 2026-10-11): streamed reading
//! with a per-line cap, invalid UTF-8 lines skipped, field values that
//! round-trip from facets into queries (quotes, apostrophes, slashes,
//! backslashes), regexes beside slash-containing field values, key joins
//! for markdown corpora, and the index-path symlink guard.

use scout_corpus::api::{FacetsQuery, SearchQuery};
use scout_corpus::registry::{CorpusConfig, CorpusKind, DocumentUnit, FieldMap};
use scout_corpus::topics;
use scout_corpus::{Corpus, Engine, Registry, SearchRequest};
use std::fs;
use std::path::Path;

fn write(root: &Path, rel: &str, body: &[u8]) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

fn cfg(root: &Path, kind: CorpusKind) -> CorpusConfig {
    CorpusConfig {
        id: "c".into(),
        name: "C".into(),
        kind,
        path: root.display().to_string(),
        include: if kind == CorpusKind::MarkdownFolder {
            vec!["**/*.md".into()]
        } else {
            vec![]
        },
        exclude: vec![],
        require_frontmatter: vec![],
        field_map: FieldMap::default(),
        default_author: None,
        boilerplate: vec![],
        link: None,
        document_unit: DocumentUnit::File,
    }
}

fn work(id: &str, quote: &str) -> Vec<u8> {
    format!(
        "---\ntitle: W{id}\nauthor: A\ntype: tweet\nsource_system: x\nsource_id: \"{id}\"\n---\n\n> {quote}\n\nhighlighted_at: 2026-09-01\n\n---\n"
    )
    .into_bytes()
}

fn line(key: &str, themes: &[&str], entities: &[&str], event: Option<&str>) -> String {
    serde_json::json!({
        "key": key, "path": null, "date": "2026-09-01", "ai_related": true,
        "themes": themes, "entities": entities, "event": event, "event_name": "E"
    })
    .to_string()
}

fn search_keys(c: &Corpus, q: &str) -> Vec<String> {
    let mut r = SearchRequest::new(q);
    r.limit = 100;
    c.search(&r)
        .unwrap()
        .results
        .into_iter()
        .map(|d| d.rel_path)
        .collect()
}

// ---------- fix 1: streamed reading, per-line cap ----------

#[test]
fn an_oversized_line_is_a_bad_line_and_the_rest_is_read() {
    let src = tempfile::tempdir().unwrap();
    let big = serde_json::json!({
        "key": "x:big", "themes": ["Huge"], "entities": [],
        "relevance": "a".repeat(topics::MAX_LINE_BYTES + 10)
    })
    .to_string();
    let body = format!(
        "{}\n{big}\n{}\n",
        line("x:1", &["T"], &[], None),
        line("x:2", &["T"], &[], None)
    );
    write(src.path(), topics::LAYER_FILE, body.as_bytes());
    // The same cap holds for corrections.
    let big_c = format!(
        "{{\"key\":\"x:1\",\"themes\":[\"C\"],\"note\":\"{}\"}}\n",
        "b".repeat(topics::MAX_LINE_BYTES)
    );
    write(src.path(), topics::CORRECTIONS_FILE, big_c.as_bytes());
    let l = topics::read_layer(src.path()).unwrap().unwrap();
    assert_eq!(l.items.len(), 2);
    assert!(!l.items.contains_key("x:big"));
    assert_eq!(l.bad_lines, 1);
    assert_eq!(l.corrections, 0);
    assert_eq!(l.corrections_bad_lines, 1);
    assert_eq!(l.items["x:1"].info.themes, vec!["T"]);
}

// ---------- fix 4: invalid UTF-8 ----------

#[test]
fn invalid_utf8_lines_are_skipped_and_counted_never_applied() {
    let src = tempfile::tempdir().unwrap();
    let mut body = line("x:1", &["T"], &[], None).into_bytes();
    body.extend_from_slice(b"\n{\"key\":\"x:2\",\"themes\":[\"Bad \xff byte\"]}\n");
    write(src.path(), topics::LAYER_FILE, &body);
    let mut corr = b"{\"key\":\"x:1\",\"themes\":[\"Fixed \xfe\"]}\n".to_vec();
    corr.extend_from_slice(b"{\"key\":\"x:1\",\"entities\":[\"Good\"]}\n");
    write(src.path(), topics::CORRECTIONS_FILE, &corr);
    let l = topics::read_layer(src.path()).unwrap().unwrap();
    assert_eq!(l.items.len(), 1, "the invalid line is not an item");
    assert_eq!(l.bad_lines, 1);
    assert_eq!(l.corrections_bad_lines, 1);
    assert_eq!(l.corrections, 1);
    let x1 = &l.items["x:1"].info;
    assert_eq!(
        x1.themes,
        vec!["T"],
        "the invalid correction is not applied"
    );
    assert_eq!(x1.entities, vec!["Good"]);
}

// ---------- fixes 2 and 3: grammar spans and round-tripping values ----------

const LABELS: &[&str] = &[
    "O’Reilly",
    "GPT “Astra”",
    "say \"hi\"",
    "back\\slash",
    "AI/ML and proofs",
    "Teachers' use, of AI",
    "Mixed ‘quotes’",
];

fn quoted_fixture() -> (tempfile::TempDir, tempfile::TempDir, Corpus) {
    let (src, data) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut layer = String::new();
    for (i, label) in LABELS.iter().enumerate() {
        let id = (i + 1).to_string();
        write(
            src.path(),
            &format!("readings/works/w{id}.md"),
            &work(&id, &format!("A proof about item {id}.")),
        );
        // Every label is both a theme and an entity of one or two items;
        // the event id has a slash.
        let others = [*label, LABELS[(i + 1) % LABELS.len()]];
        layer.push_str(&line(
            &format!("x:{id}"),
            &others,
            &others,
            Some(if i % 2 == 0 {
                "2026-09/E21"
            } else {
                "2026-09/E22"
            }),
        ));
        layer.push('\n');
    }
    write(src.path(), topics::LAYER_FILE, layer.as_bytes());
    let c = Corpus::from_config(
        cfg(src.path(), CorpusKind::HighlightScoutArchive),
        data.path(),
    );
    c.build_index(false).unwrap();
    (src, data, c)
}

#[test]
fn every_facet_value_round_trips_into_a_field_query_with_the_same_count() {
    let (_s, data, c) = quoted_fixture();
    let e = Engine::new(
        Registry {
            corpora: vec![c.config.clone()],
        },
        data.path(),
    );
    let f = e
        .facets(&FacetsQuery {
            query: "proof".into(),
            top: 0,
            ..Default::default()
        })
        .unwrap()
        .body;
    assert_eq!(f.total_documents, LABELS.len());
    let mut checked = 0;
    for (field, facet) in [
        ("theme", &f.facets.theme),
        ("entity", &f.facets.entity),
        ("event", &f.facets.event),
        ("month", &f.facets.month),
    ] {
        assert!(!facet.items.is_empty(), "{field}");
        for it in &facet.items {
            for with_text in [false, true] {
                let fq = topics::field_query(field, &it.value);
                let q = if with_text {
                    format!("proof {fq} /item/i")
                } else {
                    fq
                };
                let r = e
                    .search(&SearchQuery {
                        query: q.clone(),
                        limit: 100,
                        ..Default::default()
                    })
                    .unwrap()
                    .body;
                assert_eq!(r.total_documents, it.count, "{q}");
                checked += 1;
            }
        }
    }
    assert!(checked >= 2 * (2 * LABELS.len() + 3));
}

#[test]
fn matching_folds_quotes_and_apostrophes_on_both_sides() {
    let (_s, _d, c) = quoted_fixture();
    // Straight and curly forms of the same label match each other.
    assert_eq!(search_keys(&c, "entity:O'Reilly").len(), 2);
    assert_eq!(search_keys(&c, "entity:O’Reilly").len(), 2);
    assert_eq!(search_keys(&c, "theme:\"Teachers’ use, of AI\"").len(), 2);
    assert_eq!(
        search_keys(&c, "entity:\"GPT \\\"Astra\\\"\"").len(),
        2,
        "escaped straight quotes match the curly label"
    );
    assert_eq!(search_keys(&c, "entity:\"back\\\\slash\"").len(), 2);
}

#[test]
fn regexes_and_repeated_fields_beside_slash_values() {
    let (_s, _d, c) = quoted_fixture();
    let n = |q: &str| search_keys(&c, q).len();
    // 4 items carry E21 (items 1, 3, 5, 7).
    assert_eq!(n("event:2026-09/E21"), 4);
    assert_eq!(n("event:2026-09/E21 /proof/i"), 4);
    assert_eq!(n("event:2026-09/E21 /item 3/"), 1);
    assert_eq!(n("event:2026-09/E22 event:2026-09/E21"), 4, "last wins");
    assert_eq!(n("theme:\"AI/ML and proofs\" /proof/"), 2);
}

// ---------- fix 5: markdown corpora join by key ----------

#[test]
fn markdown_documents_join_by_frontmatter_key() {
    let (src, data) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    write(
        src.path(),
        "notes/a.md",
        b"---\ntitle: A\nsource_system: blog\nsource_id: \"42\"\n---\nA proof.\n",
    );
    write(
        src.path(),
        "notes/b.md",
        b"---\ntitle: B\n---\nAnother proof.\n",
    );
    write(
        src.path(),
        topics::LAYER_FILE,
        format!(
            "{}\n",
            line("blog:42", &["Proofs"], &[], None)
                .replace("\"path\":null", "\"path\":\"moved.md\"")
        )
        .as_bytes(),
    );
    let c = Corpus::from_config(cfg(src.path(), CorpusKind::MarkdownFolder), data.path());
    let t = c.build_index(false).unwrap().topics.unwrap();
    assert_eq!((t.matched_by_key, t.unmatched), (1, 0));
    assert_eq!(search_keys(&c, "theme:Proofs"), vec!["notes/a.md"]);
}

// ---------- fix 6: index-path symlink guard ----------

#[test]
fn an_index_path_symlinked_into_the_corpus_is_refused_untouched() {
    let (src, data) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    write(src.path(), "readings/works/w1.md", &work("1", "A proof."));
    let inner = src.path().join("inner.sqlite");
    {
        let conn = rusqlite::Connection::open(&inner).unwrap();
        conn.execute_batch("CREATE TABLE keep (x INTEGER); INSERT INTO keep VALUES (1);")
            .unwrap();
    }
    let before = fs::read(&inner).unwrap();
    let c = Corpus::from_config(
        cfg(src.path(), CorpusKind::HighlightScoutArchive),
        data.path(),
    );
    fs::create_dir_all(c.index_path().parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&inner, c.index_path()).unwrap();
    for force in [false, true] {
        let err = c.build_index(force).unwrap_err().to_string();
        assert!(err.contains("symlink"), "{err}");
        assert_eq!(fs::read(&inner).unwrap(), before, "force={force}");
        assert!(fs::symlink_metadata(c.index_path()).is_ok(), "link kept");
    }
    // A symlinked side file (-wal) is refused too.
    fs::remove_file(c.index_path()).unwrap();
    let wal = format!("{}-wal", c.index_path().display());
    std::os::unix::fs::symlink(src.path().join("w.wal"), &wal).unwrap();
    assert!(c.build_index(false).is_err());
    assert!(!src.path().join("w.wal").exists());
}

#[test]
fn an_index_inside_another_registered_corpus_root_is_refused() {
    // Engine builds check every registered corpus root, not only the
    // corpus's own: an index dir inside corpus b must not take a's index.
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    write(a.path(), "readings/works/w1.md", &work("1", "A proof."));
    write(b.path(), "notes/x.md", b"---\ntitle: X\n---\nx\n");
    let mut ca = cfg(a.path(), CorpusKind::HighlightScoutArchive);
    ca.id = "a".into();
    let mut cb = cfg(b.path(), CorpusKind::MarkdownFolder);
    cb.id = "b".into();
    let e = Engine::new(
        Registry {
            corpora: vec![ca, cb],
        },
        b.path().join("idx"),
    );
    let err = e
        .index_build(&scout_corpus::api::IndexBuildQuery {
            ids: vec!["a".into()],
            ..Default::default()
        })
        .unwrap_err()
        .to_string();
    assert!(err.contains("inside"), "{err}");
    assert!(!b.path().join("idx").join("a.sqlite").exists());
}

// ---------- round 2 ----------

#[cfg(unix)]
#[test]
fn an_index_file_hard_linked_to_a_corpus_file_is_refused_untouched() {
    let (src, data) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    write(src.path(), "readings/works/w1.md", &work("1", "A proof."));
    let inner = src.path().join("inner.sqlite");
    {
        let conn = rusqlite::Connection::open(&inner).unwrap();
        conn.execute_batch("CREATE TABLE keep (x INTEGER); INSERT INTO keep VALUES (1);")
            .unwrap();
    }
    let before = fs::read(&inner).unwrap();
    let c = Corpus::from_config(
        cfg(src.path(), CorpusKind::HighlightScoutArchive),
        data.path(),
    );
    fs::create_dir_all(c.index_path().parent().unwrap()).unwrap();
    // Same filesystem is needed for a hard link; skip quietly when the
    // two temp dirs are on different ones.
    if fs::hard_link(&inner, c.index_path()).is_err() {
        return;
    }
    for force in [false, true] {
        let err = c.build_index(force).unwrap_err().to_string();
        assert!(err.contains("hard link"), "{err}");
        assert_eq!(fs::read(&inner).unwrap(), before, "force={force}");
    }
    // A hard-linked side file is refused too.
    fs::remove_file(c.index_path()).unwrap();
    let wal = format!("{}-wal", c.index_path().display());
    fs::hard_link(src.path().join("readings/works/w1.md"), &wal).unwrap();
    let work_before = fs::read(src.path().join("readings/works/w1.md")).unwrap();
    assert!(c.build_index(false).is_err());
    assert_eq!(
        fs::read(src.path().join("readings/works/w1.md")).unwrap(),
        work_before
    );
}

fn tagged_work(id: &str, title: &str, author: &str, tags: &str) -> Vec<u8> {
    format!(
        "---\ntitle: {title}\nauthor: {author}\ntype: article\nsource_system: readwise\nsource_id: \"{id}\"\n---\n\n> A proof in work {id}.\n\nhighlighted_at: 2026-09-01 | tags: {tags} | color: yellow\n\n---\n"
    )
    .into_bytes()
}

#[test]
fn every_field_comparison_folds_both_sides() {
    let (src, data) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    write(
        src.path(),
        "readings/works/w1.md",
        &tagged_work("1", "The “Astra” paper", "Tim O'Reilly", "O'Reilly, plain"),
    );
    write(
        src.path(),
        "readings/works/w2.md",
        &tagged_work("2", "Teachers’ notes", "Seán O’Reilly", "O’Reilly"),
    );
    let c = Corpus::from_config(
        cfg(src.path(), CorpusKind::HighlightScoutArchive),
        data.path(),
    );
    c.build_index(false).unwrap();
    let n = |q: &str| search_keys(&c, q).len();
    // tag: straight and curly on either side.
    assert_eq!(n("tag:O’Reilly"), 2);
    assert_eq!(n("tag:O'Reilly"), 2);
    assert_eq!(n("tag:o'reilly"), 2);
    assert_eq!(n("tag:plain"), 1);
    // author and title (substring), with quotes and apostrophes.
    assert_eq!(n("au:O’Reilly"), 2);
    assert_eq!(n("au:\"o'reilly\""), 2);
    assert_eq!(n("ti:\"\\\"astra\\\"\""), 1);
    assert_eq!(n("ti:“teachers' notes”"), 1);
    // Decomposed and composed forms of the same name.
    assert_eq!(n("au:Sea\u{301}n"), 1);
    // source, type and colour.
    assert_eq!(n("source:READWISE"), 2);
    assert_eq!(n("ty:Articles"), 2);
    assert_eq!(n("co:Yellow"), 2);
}

#[test]
fn constructs_are_read_only_at_token_start() {
    use scout_corpus::search::Plan;
    // A curly-quoted phrase is a phrase: no tag filter comes out of it.
    let p = Plan::parse("“tag:like text”").unwrap();
    assert!(p.tag.is_none());
    assert_eq!(p.any_of.len(), 1);
    assert_eq!(p.any_of[0].len(), 1, "one phrase term");
    // A URL holding `theme:` is one term, not a theme filter.
    let p = Plan::parse("https://example.org/theme:Foo").unwrap();
    assert!(p.filter.theme.is_none());
    assert!(p.filter.is_empty());
    let p = Plan::parse("notes/theme:Foo proof").unwrap();
    assert!(p.filter.is_empty());
    assert!(p.has_positive());
    let p = Plan::parse("x(theme:Foo) theme:Bar").unwrap();
    assert_eq!(p.filter.theme.as_deref(), Some("Bar"));
}

// ---------- round 3 ----------

/// The guard inspects the real files when the index folder's name is not
/// valid UTF-8 (possible on Linux; APFS refuses such names, so this skips
/// on macOS, where `index::path_tests` covers the byte-exact suffix).
#[cfg(unix)]
#[test]
fn the_guard_sees_hard_links_under_a_non_utf8_index_folder() {
    use std::os::unix::ffi::OsStrExt;
    let (src, data) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    write(src.path(), "readings/works/w1.md", &work("1", "A proof."));
    let odd = data
        .path()
        .join(std::ffi::OsStr::from_bytes(b"idx\xff\xfe"));
    if fs::create_dir(&odd).is_err() {
        eprintln!("skipped: this filesystem refuses non-UTF-8 names");
        return;
    }
    let source = src.path().join("readings/works/w1.md");
    let before = fs::read(&source).unwrap();
    let c = Corpus::from_config(cfg(src.path(), CorpusKind::HighlightScoutArchive), &odd);
    for suffix in ["", "-wal", "-journal"] {
        let mut name = c.index_path().as_os_str().to_os_string();
        name.push(suffix);
        if fs::hard_link(&source, &name).is_err() {
            return;
        }
        let err = c.build_index(false).unwrap_err().to_string();
        assert!(err.contains("hard link"), "{suffix}: {err}");
        assert_eq!(fs::read(&source).unwrap(), before, "{suffix}");
        fs::remove_file(&name).unwrap();
    }
    // With nothing in the way, the odd folder works.
    assert_eq!(c.build_index(false).unwrap().docs, 1);
}

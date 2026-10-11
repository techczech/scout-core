//! Seam tests for lasting events: the record reader (malformed files,
//! merges, cycles), resolution of monthly ids, items in several events
//! (the tag row's `events` list, `event_add` / `event_remove`
//! corrections), `event:` queries by lasting and by old monthly id, the
//! event facet with `min_event_items`, the `events` listing, and a corpus
//! without `analysis/events/` behaving as before. The archive is never
//! written.

use scout_corpus::api::{EventsQuery, FacetsQuery, TopicsQuery};
use scout_corpus::registry::{CorpusConfig, CorpusKind, DocumentUnit, FieldMap};
use scout_corpus::{events, topics};
use scout_corpus::{Corpus, Engine, Registry, SearchRequest, SearchResults};
use std::fs;
use std::path::Path;

const FIELDS: &str = "2026-09-fields-declaration";
const FOLLOW: &str = "2026-10-follow-up";
const GEMINI: &str = "2026-09-gemini-launch";
const FIELDS_NAME: &str = "Fields Medallists' declaration: the \"long\" story";

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

fn snapshot(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push((p.display().to_string(), fs::read(&p).unwrap()));
            }
        }
    }
    out.sort();
    out
}

fn doc(n: u32) -> String {
    format!("readings/works/w-{n}.md")
}

fn row(n: u32, date: &str, event: Option<(&str, &str)>, extra: &str) -> String {
    let (event, name) = match event {
        Some((e, n)) => (format!("\"{e}\""), format!("\"{n}\"")),
        None => ("null".into(), "null".into()),
    };
    format!(
        r#"{{"key":"x:{n}","path":"{}","date":"{date}","ai_related":true,"themes":["T"],"entities":[],"event":{event},"event_name":{name}{extra}}}"#,
        doc(n)
    )
}

fn record(root: &Path, id: &str, front: &str, body: &str) {
    write(
        root,
        &format!("{}/{id}.md", events::EVENTS_DIR),
        &format!("---\nid: {id}\n{front}---\n\n{body}\n"),
    );
}

/// Six works; five tagged with an event. With records: the declaration
/// (`2026-09/E21`) and its follow-up (`2026-10/E04`, merged into it), the
/// Gemini launch (`2026-09/E02`); `2026-11/E01` has no record.
fn archive(root: &Path, with_records: bool) {
    for n in 1..=6 {
        write(
            root,
            &doc(n),
            &format!(
                "---\ntitle: W{n}\nauthor: A\ntype: tweet\nsource_system: x\nsource_id: \"{n}\"\n---\n\n> A proof, number {n}.\n\nhighlighted_at: 2026-09-0{n}\n\n---\n"
            ),
        );
    }
    let e21 = Some(("2026-09/E21", "Fields declaration"));
    write(
        root,
        topics::LAYER_FILE,
        &[
            row(1, "2026-09-01", e21, ""),
            row(2, "2026-09-02", e21, ""),
            row(
                3,
                "2026-10-03",
                Some(("2026-10/E04", "Declaration follow-up")),
                r#","events":["2026-09/E02","2026-10/E04",7]"#,
            ),
            row(4, "2026-09-04", Some(("2026-09/E02", "Gemini launch")), ""),
            row(5, "2026-11-05", Some(("2026-11/E01", "Unrecorded")), ""),
            row(6, "2026-09-06", None, ""),
        ]
        .join("\n"),
    );
    if with_records {
        record(
            root,
            FIELDS,
            "name: \"Fields Medallists' declaration: the \\\"long\\\" story\"\naliases:\n  - Fields declaration\nstart: 2026-09-01\nend:\nmembers:\n  - 2026-09/E21\nthemes: [T]\norigin: dominik\nedited: 2026-10-11\nmerged_into:\n",
            "The declaration, and what followed.",
        );
        record(
            root,
            FOLLOW,
            &format!("name: Declaration follow-up\nmembers: [2026-10/E04]\norigin: model\nmerged_into: {FIELDS}\n"),
            "Merged away.",
        );
        record(
            root,
            GEMINI,
            "name: Gemini launch\nstart: 2026-09-04\nend: 2026-09-04\nmembers: [2026-09/E02]\norigin: model\n",
            "A launch.",
        );
        // Not records: files that are not `*.md` are left alone.
        write(root, "analysis/events/notes.jsonl", "{\"created\":3}\n");
        write(root, "analysis/events/.hidden.md", "not a record\n");
    }
}

struct Fx {
    src: tempfile::TempDir,
    data: tempfile::TempDir,
    corpus: Corpus,
}

fn fixture(with_records: bool) -> Fx {
    let (src, data) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    archive(src.path(), with_records);
    let corpus = Corpus::from_config(
        CorpusConfig {
            id: "highlights".into(),
            name: "Highlights".into(),
            kind: CorpusKind::HighlightScoutArchive,
            path: src.path().display().to_string(),
            include: vec![],
            exclude: vec![],
            require_frontmatter: vec![],
            field_map: FieldMap::default(),
            default_author: None,
            boilerplate: vec![],
            link: None,
            document_unit: DocumentUnit::File,
        },
        data.path(),
    );
    Fx { src, data, corpus }
}

fn search(c: &Corpus, q: &str) -> SearchResults {
    let mut r = SearchRequest::new(q);
    r.limit = 100;
    c.search(&r).unwrap()
}

fn hits(c: &Corpus, q: &str) -> Vec<u32> {
    let mut out: Vec<u32> = search(c, q)
        .results
        .iter()
        .map(|d| {
            d.rel_path
                .trim_start_matches("readings/works/w-")
                .trim_end_matches(".md")
                .parse()
                .unwrap()
        })
        .collect();
    out.sort();
    out
}

fn engine(fx: &Fx) -> Engine {
    Engine::new(
        Registry {
            corpora: vec![fx.corpus.config.clone()],
        },
        fx.data.path(),
    )
}

fn event_facet(fx: &Fx, query: &str, min: usize) -> Vec<(String, usize, Option<String>)> {
    engine(fx)
        .facets(&FacetsQuery {
            query: query.into(),
            top: 0,
            min_event_items: min,
            ..Default::default()
        })
        .unwrap()
        .body
        .facets
        .event
        .items
        .into_iter()
        .map(|i| (i.value, i.count, i.name))
        .collect()
}

#[test]
fn without_a_records_folder_events_are_monthly_as_before() {
    let fx = fixture(false);
    assert!(events::read(fx.src.path()).unwrap().is_none());
    let before = snapshot(fx.src.path());
    let t = fx.corpus.build_index(false).unwrap().topics.unwrap();
    assert!(t.events.is_none());
    assert!(!serde_json::to_string(&t).unwrap().contains("\"events\""));
    let c = &fx.corpus;
    assert_eq!(hits(c, "event:2026-09/E21"), vec![1, 2]);
    assert_eq!(hits(c, "event:2026-10/E04"), vec![3]);
    assert!(hits(c, &format!("event:{FIELDS}")).is_empty());
    // A plain item carries `event` only; the item with an `events` list
    // carries it too (monthly ids, nothing to resolve them).
    let r = search(c, "event:2026-09/E21");
    let tags = r.results[0].topics.as_ref().unwrap();
    assert_eq!(tags.events, None);
    assert!(!serde_json::to_string(tags).unwrap().contains("\"events\""));
    assert_eq!(hits(c, "event:2026-09/E02"), vec![3, 4]);
    let r = search(c, "event:2026-10/E04");
    assert_eq!(
        r.results[0].topics.as_ref().unwrap().events.as_deref(),
        Some(&["2026-10/E04".to_string(), "2026-09/E02".to_string()][..])
    );
    assert_eq!(
        event_facet(&fx, "proof", 1),
        vec![
            ("2026-09/E02".into(), 2, Some("Gemini launch".into())),
            ("2026-09/E21".into(), 2, Some("Fields declaration".into())),
            (
                "2026-10/E04".into(),
                1,
                Some("Declaration follow-up".into())
            ),
            ("2026-11/E01".into(), 1, Some("Unrecorded".into())),
        ]
    );
    // The floor works on monthly events too.
    assert_eq!(event_facet(&fx, "proof", 2).len(), 2);
    let l = engine(&fx).events(&EventsQuery::default()).unwrap().body;
    assert_eq!(l.distinct, 4);
    assert!(l.events.iter().all(|e| !e.record && e.members.is_empty()));
    assert_eq!(snapshot(fx.src.path()), before);
}

#[test]
fn monthly_and_merged_ids_resolve_to_the_lasting_event() {
    let fx = fixture(true);
    let before = snapshot(fx.src.path());
    let set = events::read(fx.src.path()).unwrap().unwrap();
    assert!(set.warnings.is_empty(), "{:?}", set.warnings);
    assert_eq!(set.resolve("2026-09/E21"), Some(FIELDS));
    assert_eq!(set.resolve("2026-10/e04"), Some(FIELDS));
    assert_eq!(set.resolve(FOLLOW), Some(FIELDS));
    assert_eq!(set.resolve("2026-11/E01"), None);
    assert_eq!(set.records[FIELDS].name, FIELDS_NAME);
    assert_eq!(set.records[FIELDS].end, None);

    let t = fx.corpus.build_index(false).unwrap().topics.unwrap();
    let e = t.events.unwrap();
    assert_eq!(
        (e.records, e.merged, e.members, e.skipped, e.warnings_total),
        (3, 1, 3, 0, 0)
    );
    let c = &fx.corpus;
    // The lasting id, the old monthly ids of either month and the
    // merged-away id all give every item of the event.
    for q in [FIELDS, "2026-09/E21", "2026-10/E04", "2026-10/e04", FOLLOW] {
        assert_eq!(hits(c, &format!("event:{q}")), vec![1, 2, 3], "{q}");
    }
    // An item in two events (the tag row's `events` list) is in both.
    assert_eq!(hits(c, &format!("event:{GEMINI}")), vec![3, 4]);
    assert_eq!(hits(c, "event:2026-09/E02"), vec![3, 4]);
    // A monthly event without a record keeps its monthly id.
    assert_eq!(hits(c, "event:2026-11/E01"), vec![5]);
    assert!(hits(c, "event:2026-12/E01").is_empty());
    // The month narrows inside the event; text combines as before.
    assert_eq!(hits(c, &format!("event:{FIELDS} month:2026-10")), vec![3]);
    assert_eq!(hits(c, "event:2026-09/E21 month:2026-09"), vec![1, 2]);
    assert_eq!(hits(c, &format!("event:{FIELDS} \"number 2\"")), vec![2]);
    // Slices take the same ids.
    let s = scout_corpus::SliceSpec::parse("event:2026-10/E04").unwrap();
    let conn = rusqlite::Connection::open(fx.corpus.index_path()).unwrap();
    let docs = scout_corpus::filter::filtered_docs(&conn, &s.filter).unwrap();
    assert_eq!(docs.len(), 3);

    let r = search(c, "event:2026-10/E04 month:2026-10");
    let tags = r.results[0].topics.as_ref().unwrap();
    assert_eq!(tags.event.as_deref(), Some("2026-10/E04"));
    assert_eq!(
        tags.events.as_deref(),
        Some(&[FIELDS.to_string(), GEMINI.to_string()][..])
    );
    assert!(tags.corrected.is_empty());
    let untagged = search(c, "\"number 6\"");
    assert_eq!(
        untagged.results[0].topics.as_ref().unwrap().events,
        Some(vec![])
    );

    // Unchanged records: the stored copy is kept. A changed record re-joins.
    assert!(
        !fx.corpus
            .build_index(false)
            .unwrap()
            .topics
            .unwrap()
            .reloaded
    );
    assert_eq!(snapshot(fx.src.path()), before);
    record(
        fx.src.path(),
        GEMINI,
        "name: Gemini launch\nmembers: [2026-09/E02, 2026-11/E01]\n",
        "",
    );
    assert!(
        fx.corpus
            .build_index(false)
            .unwrap()
            .topics
            .unwrap()
            .reloaded
    );
    assert_eq!(hits(c, &format!("event:{GEMINI}")), vec![3, 4, 5]);
    assert_eq!(hits(c, "event:2026-11/E01"), vec![3, 4, 5]);
    // Delete and rebuild: the same answer.
    fx.corpus.build_index(true).unwrap();
    assert_eq!(hits(c, "event:2026-11/E01"), vec![3, 4, 5]);
}

#[test]
fn corrections_add_and_remove_items_and_the_later_line_wins() {
    let fx = fixture(true);
    write(
        fx.src.path(),
        topics::CORRECTIONS_FILE,
        &[
            // An untagged item joins an event, named by an old monthly id.
            r#"{"key":"x:6","event_add":["2026-09/E02"],"by":"Dominik"}"#,
            // Out of its tag row's event, named by the lasting id.
            format!(r#"{{"key":"x:1","event_remove":["{FIELDS}"]}}"#).as_str(),
            // Out of an event the model listed, then back in: the later line wins.
            r#"{"key":"x:3","event_remove":["2026-09/E02"]}"#,
            format!(r#"{{"key":"x:3","event_add":["{GEMINI}","{GEMINI}"]}}"#).as_str(),
            // In, then out again.
            r#"{"key":"x:4","event_add":["2026-11/E01"]}"#,
            r#"{"key":"x:4","event_remove":["2026-11/e01","2099-01/E01"]}"#,
            // One line: removals follow additions.
            format!(r#"{{"key":"x:5","event_add":["{FIELDS}"],"event_remove":["{FOLLOW}"]}}"#)
                .as_str(),
            // Wrongly typed: the whole line is left out.
            r#"{"key":"x:2","event_add":"2026-09/E02","themes":["Nope"]}"#,
            r#"{"key":"x:2","event_remove":[1]}"#,
            r#"{"key":"x:404","event_add":["2026-09/E02"]}"#,
        ]
        .join("\n"),
    );
    let before = snapshot(fx.src.path());
    let t = fx.corpus.build_index(false).unwrap().topics.unwrap();
    assert_eq!(
        (
            t.corrections,
            t.corrections_bad_lines,
            t.corrections_unmatched
        ),
        (7, 2, 1)
    );
    let c = &fx.corpus;
    assert_eq!(hits(c, &format!("event:{GEMINI}")), vec![3, 4, 6]);
    assert_eq!(hits(c, &format!("event:{FIELDS}")), vec![2, 3]);
    assert_eq!(hits(c, "event:2026-09/E21"), vec![2, 3]);
    assert_eq!(hits(c, "event:2026-11/E01"), vec![5]);
    assert_eq!(hits(c, "theme:Nope"), Vec::<u32>::new());

    let tags = |n: u32| {
        search(c, &format!("\"number {n}\"")).results[0]
            .topics
            .clone()
            .unwrap()
    };
    let one = tags(1);
    assert_eq!(one.events, Some(vec![]));
    assert_eq!((one.event, one.event_name), (None, None));
    assert_eq!(one.corrected, vec!["event", "events"]);
    let three = tags(3);
    assert_eq!(three.events, Some(vec![FIELDS.into(), GEMINI.into()]));
    assert_eq!(three.event.as_deref(), Some("2026-10/E04"));
    assert_eq!(three.corrected, vec!["events"]);
    assert_eq!(tags(6).events, Some(vec![GEMINI.into()]));
    assert_eq!(tags(6).event, None);
    assert_eq!(tags(5).events, Some(vec!["2026-11/E01".into()]));
    assert_eq!(tags(2).corrected, Vec::<String>::new());

    assert_eq!(
        event_facet(&fx, "proof", 1),
        vec![
            (GEMINI.into(), 3, Some("Gemini launch".into())),
            (FIELDS.into(), 2, Some(FIELDS_NAME.into())),
            ("2026-11/E01".into(), 1, Some("Unrecorded".into())),
        ]
    );
    assert_eq!(snapshot(fx.src.path()), before);

    // Without records the same corrections work on monthly ids.
    let fx = fixture(false);
    write(
        fx.src.path(),
        topics::CORRECTIONS_FILE,
        &[
            r#"{"key":"x:6","event_add":["2026-09/E02"]}"#,
            r#"{"key":"x:1","event_remove":["2026-09/e21"]}"#,
        ]
        .join("\n"),
    );
    fx.corpus.build_index(false).unwrap();
    assert_eq!(hits(&fx.corpus, "event:2026-09/E02"), vec![3, 4, 6]);
    assert_eq!(hits(&fx.corpus, "event:2026-09/E21"), vec![2]);
}

#[test]
fn the_event_facet_counts_by_lasting_event_and_takes_a_floor() {
    let fx = fixture(true);
    fx.corpus.build_index(false).unwrap();
    // Item 3 counts once in each of its two events.
    let all = event_facet(&fx, "proof", 1);
    assert_eq!(
        all,
        vec![
            (FIELDS.into(), 3, Some(FIELDS_NAME.into())),
            (GEMINI.into(), 2, Some("Gemini launch".into())),
            ("2026-11/E01".into(), 1, Some("Unrecorded".into())),
        ]
    );
    // Every value gives its count back as a query.
    for (id, count, _) in &all {
        let q = topics::field_query("event", id);
        assert_eq!(search(&fx.corpus, &q).total_documents, *count, "{q}");
    }
    // The floor leaves out the one-item event.
    assert_eq!(event_facet(&fx, "proof", 2), all[..2].to_vec());
    assert_eq!(event_facet(&fx, "proof", 3), all[..1].to_vec());
    assert!(event_facet(&fx, "proof", 4).is_empty());
    assert_eq!(event_facet(&fx, "proof", 0), all);
    // The size is the event's size in the corpus, not in the result: one
    // Gemini item in this result, two in the archive, so it stays.
    assert_eq!(
        event_facet(&fx, "\"number 4\"", 2),
        vec![(GEMINI.into(), 1, Some("Gemini launch".into()))]
    );
    assert!(event_facet(&fx, "\"number 5\"", 2).is_empty());
    let e = engine(&fx);
    let with = e
        .facets(&FacetsQuery {
            query: "proof".into(),
            min_event_items: 2,
            ..Default::default()
        })
        .unwrap()
        .body;
    assert_eq!(with.facets.event.distinct, 2);
    assert_eq!(with.min_event_items, Some(2));
    assert_eq!(with.facets.theme.items[0].count, 6);
    let plain = e
        .facets(&FacetsQuery {
            query: "proof".into(),
            ..Default::default()
        })
        .unwrap()
        .body;
    assert!(!serde_json::to_string(&plain)
        .unwrap()
        .contains("min_event_items"));
    // `scout topics events` lists the same values.
    let t = e
        .topics(&TopicsQuery {
            field: "events".into(),
            ..Default::default()
        })
        .unwrap()
        .body;
    assert_eq!(
        t.items
            .iter()
            .map(|i| (i.value.clone(), i.count, i.name.clone()))
            .collect::<Vec<_>>(),
        all
    );
}

#[test]
fn the_events_listing_gives_records_counts_and_months() {
    let fx = fixture(true);
    fx.corpus.build_index(false).unwrap();
    let e = engine(&fx);
    let l = e.events(&EventsQuery::default()).unwrap().body;
    assert_eq!(l.distinct, 3);
    let ids: Vec<&str> = l.events.iter().map(|x| x.id.as_str()).collect();
    assert_eq!(ids, vec![FIELDS, GEMINI, "2026-11/E01"]);
    let v = serde_json::to_value(&l).unwrap();
    assert_eq!(
        v["events"][0],
        serde_json::json!({
            "corpus": "highlights",
            "id": FIELDS,
            "name": FIELDS_NAME,
            "record": true,
            "start": "2026-09-01",
            "end": null,
            "items": 3,
            "months": [{"month": "2026-09", "count": 2}, {"month": "2026-10", "count": 1}],
            "members": ["2026-09/E21"],
            "aliases": ["Fields declaration"],
            "themes": ["T"],
            "origin": "dominik",
            "edited": "2026-10-11",
            "merged_into": null,
            "description": "The declaration, and what followed."
        })
    );
    assert_eq!(
        v["events"][2],
        serde_json::json!({
            "corpus": "highlights", "id": "2026-11/E01", "name": "Unrecorded", "record": false,
            "start": null, "end": null, "items": 1,
            "months": [{"month": "2026-11", "count": 1}],
            "members": [], "aliases": [], "themes": [], "origin": null, "edited": null,
            "merged_into": null, "description": ""
        })
    );
    // One-item events left out; merged records on request.
    let two = e
        .events(&EventsQuery {
            min_items: 2,
            ..Default::default()
        })
        .unwrap()
        .body;
    assert_eq!(two.distinct, 2);
    let merged = e
        .events(&EventsQuery {
            merged: true,
            min_items: 0,
            ..Default::default()
        })
        .unwrap()
        .body;
    let m = merged.events.iter().find(|x| x.id == FOLLOW).unwrap();
    assert_eq!((m.items, m.merged_into.as_deref()), (0, Some(FIELDS)));
    assert_eq!(merged.distinct, 4);
    // A month's list: every event with an item that month, with that
    // month's count beside the total.
    let oct = e
        .events(&EventsQuery {
            month: Some("2026-10".into()),
            ..Default::default()
        })
        .unwrap()
        .body;
    assert_eq!(
        oct.events
            .iter()
            .map(|x| (x.id.as_str(), x.items, x.month_items))
            .collect::<Vec<_>>(),
        vec![(FIELDS, 3, Some(1)), (GEMINI, 2, Some(1))]
    );
    let year = e
        .events(&EventsQuery {
            month: Some("2026".into()),
            top: 1,
            ..Default::default()
        })
        .unwrap()
        .body;
    assert_eq!((year.distinct, year.events.len()), (3, 1));
    assert!(e
        .events(&EventsQuery {
            month: Some("October".into()),
            ..Default::default()
        })
        .is_err());
}

#[test]
fn malformed_records_are_skipped_with_a_warning_and_the_rest_is_indexed() {
    let fx = fixture(true);
    let root = fx.src.path();
    let dir = events::EVENTS_DIR;
    write(
        root,
        &format!("{dir}/bad-yaml.md"),
        "---\nid: bad-yaml\nname: [unclosed\n---\n",
    );
    write(
        root,
        &format!("{dir}/no-id.md"),
        "---\nname: No id\nmembers: [2026-11/E01]\n---\n",
    );
    write(
        root,
        &format!("{dir}/other-name.md"),
        "---\nid: something-else\nmembers: [2026-11/E01]\n---\n",
    );
    write(root, &format!("{dir}/no-frontmatter.md"), "Just prose.\n");
    write(
        root,
        &format!("{dir}/bad-members.md"),
        "---\nid: bad-members\nmembers: 2026-11/E01\n---\n",
    );
    write(
        root,
        &format!("{dir}/bomb.md"),
        "---\nid: bomb\na: &a [x, x]\nb: &b [*a, *a]\nmembers: *b\n---\n",
    );
    fs::write(
        root.join(dir).join("not-utf8.md"),
        b"---\nid: not-utf8\nname: \xff\n---\n",
    )
    .unwrap();
    fs::write(
        root.join(dir).join("huge.md"),
        format!(
            "---\nid: huge\n---\n{}",
            "x".repeat(events::MAX_RECORD_BYTES as usize)
        ),
    )
    .unwrap();
    fs::create_dir_all(root.join(dir).join("a-folder.md")).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        root.join(dir).join(format!("{GEMINI}.md")),
        root.join(dir).join("link.md"),
    )
    .unwrap();
    // Sound files with unusable links between them.
    record(
        root,
        "2026-12-dangling",
        "members: [2026-12/E01]\nmerged_into: 2026-12-nowhere\n",
        "",
    );
    record(root, "2026-12-loop-a", "merged_into: 2026-12-loop-b\n", "");
    record(root, "2026-12-loop-b", "merged_into: 2026-12-loop-a\n", "");
    record(root, "2026-12-thief", "members: [2026-09/E21]\n", "");
    let before = snapshot(root);

    let t = fx.corpus.build_index(false).unwrap().topics.unwrap();
    let e = t.events.unwrap();
    let skipped = if cfg!(unix) { 10 } else { 9 };
    assert_eq!(e.skipped, skipped);
    assert_eq!(e.records, 7);
    assert_eq!(e.warnings_total, skipped + 4);
    let w = e.warnings.join("\n");
    for needle in [
        "analysis/events/bad-yaml.md: bad YAML",
        "analysis/events/no-id.md: no id; skipped",
        "analysis/events/other-name.md: id \"something-else\" does not match the file name; skipped",
        "analysis/events/no-frontmatter.md: no frontmatter; skipped",
        "analysis/events/bad-members.md: members is not a list; skipped",
        "analysis/events/bomb.md: bad YAML (YAML aliases are not read); skipped",
        "analysis/events/not-utf8.md: not valid UTF-8; skipped",
        "analysis/events/huge.md: larger than 1 MiB; skipped",
        "analysis/events/a-folder.md: not a plain file; skipped",
        "analysis/events/2026-12-dangling.md: merged_into \"2026-12-nowhere\" names no record; treated as not merged",
        "analysis/events/2026-12-loop-a.md: merged_into \"2026-12-loop-b\" closes a cycle; treated as not merged",
        "analysis/events/2026-12-loop-b.md: merged_into \"2026-12-loop-a\" closes a cycle; treated as not merged",
        "analysis/events/2026-12-thief.md: member \"2026-09/E21\" already belongs to \"2026-09-fields-declaration\"; kept there",
    ] {
        assert!(w.contains(needle), "{needle}\n{w}");
    }
    #[cfg(unix)]
    assert!(
        w.contains("analysis/events/link.md: not a plain file; skipped"),
        "{w}"
    );
    // The sound records are all there, and nothing from a skipped file is.
    let c = &fx.corpus;
    assert_eq!(hits(c, "event:2026-09/E21"), vec![1, 2, 3]);
    assert_eq!(hits(c, &format!("event:{GEMINI}")), vec![3, 4]);
    assert_eq!(hits(c, "event:2026-11/E01"), vec![5]);
    for gone in [
        "bad-yaml",
        "no-id",
        "something-else",
        "bad-members",
        "bomb",
        "link",
        "2026-12-thief",
    ] {
        assert!(hits(c, &format!("event:{gone}")).is_empty(), "{gone}");
    }
    let l = engine(&fx)
        .events(&EventsQuery {
            min_items: 0,
            merged: true,
            ..Default::default()
        })
        .unwrap()
        .body;
    let listed: Vec<&str> = l
        .events
        .iter()
        .filter(|x| x.record)
        .map(|x| x.id.as_str())
        .collect();
    assert_eq!(listed.len(), 7);
    // The cycle's records are listed as events that are not merged.
    let a = l.events.iter().find(|x| x.id == "2026-12-loop-a").unwrap();
    assert_eq!(a.merged_into, None);
    assert_eq!(snapshot(root), before);
    // The warnings survive an unchanged rebuild.
    let again = fx.corpus.build_index(false).unwrap().topics.unwrap();
    assert!(!again.reloaded);
    assert_eq!(again.events.unwrap().warnings_total, skipped + 4);
}

#[test]
fn an_empty_or_odd_records_folder_never_breaks_the_build() {
    // Empty folder: no records, every item keeps its monthly event.
    let fx = fixture(false);
    fs::create_dir_all(fx.src.path().join(events::EVENTS_DIR)).unwrap();
    let t = fx.corpus.build_index(false).unwrap().topics.unwrap();
    assert_eq!(t.events.unwrap().records, 0);
    assert_eq!(hits(&fx.corpus, "event:2026-09/E21"), vec![1, 2]);
    // A file where the folder should be: as if there were none.
    let fx = fixture(false);
    write(fx.src.path(), events::EVENTS_DIR, "not a folder\n");
    let t = fx.corpus.build_index(false).unwrap().topics.unwrap();
    assert!(t.events.is_none());
    assert_eq!(hits(&fx.corpus, "event:2026-09/E21"), vec![1, 2]);
    // Many warnings: the report keeps the first 50 and the total.
    let fx = fixture(true);
    for i in 0..(events::MAX_WARNINGS + 20) {
        write(
            fx.src.path(),
            &format!("{}/z-bad-{i:03}.md", events::EVENTS_DIR),
            "nothing\n",
        );
    }
    let e = fx
        .corpus
        .build_index(false)
        .unwrap()
        .topics
        .unwrap()
        .events
        .unwrap();
    assert_eq!(e.warnings.len(), events::MAX_WARNINGS);
    assert_eq!(e.warnings_total, events::MAX_WARNINGS + 20);
    assert_eq!(hits(&fx.corpus, "event:2026-09/E21"), vec![1, 2, 3]);
}

/// The ids `scout events --min-items 0 --merged` lists as records, with
/// their item counts.
fn listed_records(fx: &Fx) -> Vec<(String, usize)> {
    let mut out: Vec<(String, usize)> = engine(fx)
        .events(&EventsQuery {
            min_items: 0,
            merged: true,
            ..Default::default()
        })
        .unwrap()
        .body
        .events
        .into_iter()
        .filter(|e| e.record)
        .map(|e| (e.id, e.items))
        .collect();
    out.sort();
    out
}

#[test]
fn records_are_read_and_listed_when_the_corpus_has_no_topic_layer() {
    let none: Vec<(String, usize)> = [FIELDS, GEMINI, FOLLOW]
        .iter()
        .map(|id| (id.to_string(), 0))
        .collect();
    // Never had tags: the records are indexed and reported on their own.
    let fx = fixture(true);
    fs::remove_file(fx.src.path().join(topics::LAYER_FILE)).unwrap();
    assert!(topics::read_layer(fx.src.path()).unwrap().is_none());
    let before = snapshot(fx.src.path());
    let b = fx.corpus.build_index(false).unwrap();
    assert!(b.topics.is_none());
    let e = b.events.clone().unwrap();
    assert_eq!((e.records, e.merged, e.members, e.skipped), (3, 1, 3, 0));
    let json = serde_json::to_value(&b).unwrap();
    assert_eq!(json["events"]["records"], 3);
    assert!(json.get("topics").is_none());
    assert_eq!(listed_records(&fx), none);
    // Without `--min-items 0` nothing is listed: no record has an item.
    let l = engine(&fx).events(&EventsQuery::default()).unwrap().body;
    assert!(l.events.is_empty());
    // The record fields are all there.
    let l = engine(&fx)
        .events(&EventsQuery {
            min_items: 0,
            ..Default::default()
        })
        .unwrap()
        .body;
    assert_eq!(l.distinct, 2, "the merged record needs --merged");
    let f = l.events.iter().find(|e| e.id == FIELDS).unwrap();
    assert_eq!(f.name.as_deref(), Some(FIELDS_NAME));
    assert_eq!(f.members, vec!["2026-09/E21"]);
    assert_eq!(f.description, "The declaration, and what followed.");
    // No tags, so no document is in any event.
    assert!(hits(&fx.corpus, &format!("event:{FIELDS}")).is_empty());
    // An unchanged rebuild keeps the copy and its report.
    let again = fx.corpus.build_index(false).unwrap();
    assert_eq!(again.events, Some(e));
    assert_eq!(listed_records(&fx), none);
    // A changed record is re-read without a layer file.
    record(fx.src.path(), "2026-12-new", "name: New\n", "");
    let b = fx.corpus.build_index(false).unwrap();
    assert_eq!(b.events.unwrap().records, 4);
    assert_eq!(listed_records(&fx).len(), 4);
    fs::remove_file(
        fx.src
            .path()
            .join(events::EVENTS_DIR)
            .join("2026-12-new.md"),
    )
    .unwrap();
    fx.corpus.build_index(false).unwrap();
    assert_eq!(listed_records(&fx), none);
    assert_eq!(snapshot(fx.src.path()), before);

    // Tags taken away after a build: the records stay, the items go.
    let fx = fixture(true);
    let b = fx.corpus.build_index(false).unwrap();
    assert!(b.events.is_none(), "with a layer: reported in topics");
    assert_eq!(b.topics.unwrap().events.unwrap().records, 3);
    assert_eq!(
        listed_records(&fx),
        vec![
            (FIELDS.to_string(), 3),
            (GEMINI.to_string(), 2),
            (FOLLOW.to_string(), 0)
        ]
    );
    fs::remove_file(fx.src.path().join(topics::LAYER_FILE)).unwrap();
    let b = fx.corpus.build_index(false).unwrap();
    assert!(b.topics.is_none());
    assert_eq!(b.events.unwrap().records, 3);
    assert_eq!(listed_records(&fx), none);
    // The tags come back: reported in `topics` again, items counted.
    archive(fx.src.path(), true);
    let b = fx.corpus.build_index(false).unwrap();
    assert!(b.events.is_none());
    assert_eq!(b.topics.unwrap().events.unwrap().records, 3);
    assert_eq!(listed_records(&fx)[0], (FIELDS.to_string(), 3));
    // Neither tags nor records: nothing is kept or reported.
    fs::remove_file(fx.src.path().join(topics::LAYER_FILE)).unwrap();
    fs::remove_dir_all(fx.src.path().join(events::EVENTS_DIR)).unwrap();
    let b = fx.corpus.build_index(false).unwrap();
    assert!(b.topics.is_none() && b.events.is_none());
    assert!(listed_records(&fx).is_empty());
}

#[test]
fn a_record_nested_100_000_deep_is_skipped_with_a_warning_and_the_build_completes() {
    let fx = fixture(true);
    let dir = events::EVENTS_DIR;
    let n = 100_000;
    for (id, front) in [
        ("deep-block", format!("members:\n  {}a\n", "- ".repeat(n))),
        ("deep-flow", format!("members: {}\n", "[".repeat(n))),
        (
            "deep-closed",
            format!("extra: {}{}\n", "[".repeat(n), "]".repeat(n)),
        ),
        ("deep-map", format!("extra: {}\n", "{a: ".repeat(n))),
    ] {
        let text = format!("---\nid: {id}\n{front}---\n");
        assert!(text.len() as u64 <= events::MAX_RECORD_BYTES);
        write(fx.src.path(), &format!("{dir}/{id}.md"), &text);
    }
    // Four levels is already too deep; three is read.
    record(fx.src.path(), "four-deep", "extra: [[[a]]]\n", "");
    record(fx.src.path(), "three-deep", "extra: [[a]]\n", "");
    let before = snapshot(fx.src.path());
    let t = fx.corpus.build_index(false).unwrap().topics.unwrap();
    let e = t.events.unwrap();
    assert_eq!(e.skipped, 5);
    assert_eq!(e.records, 4);
    let w = e.warnings.join("\n");
    for id in ["deep-block", "deep-map", "four-deep"] {
        let needle =
            format!("analysis/events/{id}.md: bad YAML (nested deeper than 3 levels); skipped");
        assert!(w.contains(&needle), "{needle}\n{w}");
    }
    // Bare `[[[[…`: the YAML scanner's own cap of 255 open brackets stops
    // it first, while it looks ahead for a key.
    for id in ["deep-flow", "deep-closed"] {
        let needle = format!("analysis/events/{id}.md: bad YAML (recursion limit exceeded");
        assert!(w.contains(&needle), "{needle}\n{w}");
    }
    assert!(!w.contains("three-deep"), "{w}");
    // The sound records are indexed as ever.
    assert_eq!(hits(&fx.corpus, "event:2026-09/E21"), vec![1, 2, 3]);
    assert_eq!(listed_records(&fx).len(), 4);
    assert_eq!(snapshot(fx.src.path()), before);
}

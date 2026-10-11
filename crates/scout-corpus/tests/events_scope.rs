//! Where lasting events apply, and to which corpus an event belongs.
//!
//! - A corpus without `analysis/events/` reads its topic layer exactly as
//!   before lasting events: a tag row's `events` list and the corrections'
//!   `event_add` / `event_remove` are unknown keys, whatever their type.
//!   The expected values below are what the reader before lasting events
//!   gives for the fixture (its `TopicInfo` had no `events`, its
//!   corrections read `themes`, `entities`, `event`, `event_name` and
//!   `ai_related` and nothing else).
//! - Where any searched corpus has a records folder, an event id names an
//!   event within one corpus: two corpora holding the same id hold two
//!   events, and facets count, name and size them apart. Where none has,
//!   the event facet is the one from before lasting events: one value per
//!   id over the searched corpora together, without `corpus`.

use scout_corpus::api::{self, EventsQuery, FacetsQuery, SearchQuery, TopicsQuery};
use scout_corpus::registry::{CorpusConfig, CorpusKind, DocumentUnit, FieldMap};
use scout_corpus::{events, topics};
use scout_corpus::{Corpus, Engine, Registry};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

fn work(root: &Path, n: u32) {
    write(
        root,
        &format!("readings/works/w-{n}.md"),
        &format!(
            "---\ntitle: W{n}\nauthor: A\ntype: tweet\nsource_system: x\nsource_id: \"{n}\"\n---\n\n> A proof, number {n}.\n\nhighlighted_at: 2026-09-0{n}\n\n---\n"
        ),
    );
}

fn row(n: u32, event: Option<(&str, &str)>, extra: &str) -> String {
    let (event, name) = match event {
        Some((e, n)) => (format!("\"{e}\""), format!("\"{n}\"")),
        None => ("null".into(), "null".into()),
    };
    format!(
        r#"{{"key":"x:{n}","path":"readings/works/w-{n}.md","date":"2026-09-0{n}","ai_related":true,"themes":["T"],"entities":[],"event":{event},"event_name":{name}{extra}}}"#
    )
}

fn config(id: &str, root: &Path) -> CorpusConfig {
    CorpusConfig {
        id: id.into(),
        name: id.into(),
        kind: CorpusKind::HighlightScoutArchive,
        path: root.display().to_string(),
        include: vec![],
        exclude: vec![],
        require_frontmatter: vec![],
        field_map: FieldMap::default(),
        default_author: None,
        boilerplate: vec![],
        link: None,
        document_unit: DocumentUnit::File,
    }
}

/// One built corpus `highlights` over `root`, and the engine that reads it.
struct One {
    _data: tempfile::TempDir,
    engine: Engine,
    report: topics::TopicReport,
}

fn one(root: &Path) -> One {
    let data = tempfile::tempdir().unwrap();
    let cfg = config("highlights", root);
    let report = Corpus::from_config(cfg.clone(), data.path())
        .build_index(false)
        .unwrap()
        .topics
        .unwrap();
    let engine = Engine::new(Registry { corpora: vec![cfg] }, data.path());
    One {
        _data: data,
        engine,
        report,
    }
}

/// `scout search <q> --json`, with the corpus folder's path taken out.
fn search_json(e: &Engine, root: &Path, q: &str) -> String {
    let body = e
        .search(&SearchQuery {
            query: q.into(),
            limit: 100,
            ..Default::default()
        })
        .unwrap()
        .body;
    api::to_json(&body)
        .unwrap()
        .replace(&root.display().to_string(), "<root>")
}

fn facets_json(e: &Engine, q: &str) -> String {
    let body = e
        .facets(&FacetsQuery {
            query: q.into(),
            top: 0,
            ..Default::default()
        })
        .unwrap()
        .body;
    api::to_json(&body).unwrap()
}

fn topics_json(e: &Engine) -> String {
    let body = e
        .topics(&TopicsQuery {
            field: "events".into(),
            ..Default::default()
        })
        .unwrap()
        .body;
    api::to_json(&body).unwrap()
}

/// The works a search returns, as their numbers.
fn hits(e: &Engine, q: &str) -> Vec<u64> {
    let body = e
        .search(&SearchQuery {
            query: q.into(),
            limit: 100,
            ..Default::default()
        })
        .unwrap()
        .body;
    let v = serde_json::to_value(&body).unwrap();
    let mut out: Vec<u64> = v["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            let p = r["rel_path"].as_str().unwrap();
            p.rsplit("w-")
                .next()
                .unwrap()
                .trim_end_matches(".md")
                .parse()
                .unwrap()
        })
        .collect();
    out.sort();
    out
}

/// The `topics` object of the one work `q` finds, as compact JSON.
fn tags(e: &Engine, q: &str) -> String {
    let body = e
        .search(&SearchQuery {
            query: q.into(),
            limit: 100,
            ..Default::default()
        })
        .unwrap()
        .body;
    assert_eq!(body.results.len(), 1, "{q}");
    serde_json::to_string(body.results[0].topics.as_ref().unwrap()).unwrap()
}

const E01: (&str, &str) = ("2026-09/E01", "One");
const E02: (&str, &str) = ("2026-09/E02", "Two");

/// Four works. `lasting` adds the lasting-events fields to the tag rows
/// and the corrections; without it the same lines lack exactly those keys.
fn no_records_archive(root: &Path, lasting: bool) {
    for n in 1..=4 {
        work(root, n);
    }
    let pick = |with: &'static str| if lasting { with } else { "" };
    write(
        root,
        topics::LAYER_FILE,
        &[
            // The review's reproduction: `event: E01` and `events: [E02]`.
            row(1, Some(E01), pick(r#","events":["2026-09/E02"]"#)),
            row(2, Some(E01), ""),
            row(3, Some(E02), ""),
            // Any type, and even twice: an unknown key is never looked at.
            row(4, None, pick(r#","events":"not a list","events":[7]"#)),
        ]
        .join("\n"),
    );
    write(
        root,
        topics::CORRECTIONS_FILE,
        &[
            // The same for a correction line: add and remove.
            format!(
                r#"{{"key":"x:2"{},"by":"D"}}"#,
                pick(r#","event_add":["2026-09/E02"],"event_remove":["2026-09/E01"]"#)
            ),
            // Wrongly typed for lasting events; an unknown key before them,
            // so the line's `themes` still applies.
            format!(
                r#"{{"key":"x:3"{},"themes":["Fixed"]}}"#,
                pick(r#","event_add":"2026-09/E01""#)
            ),
            format!(
                r#"{{"key":"x:4"{},"ai_related":false}}"#,
                pick(r#","event_remove":7"#)
            ),
        ]
        .join("\n"),
    );
}

#[test]
fn without_a_records_folder_the_events_list_and_event_corrections_change_nothing() {
    let (with, plain) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    no_records_archive(with.path(), true);
    no_records_archive(plain.path(), false);
    assert!(events::read(with.path()).unwrap().is_none());
    let (a, b) = (one(with.path()), one(plain.path()));

    // The reproduction: the extra `events` id is not a membership.
    assert_eq!(hits(&a.engine, "event:2026-09/E02"), vec![3]);
    assert_eq!(hits(&a.engine, "event:2026-09/E01"), vec![1, 2]);

    // Every tag object is the one the reader before lasting events gives:
    // no `events`, `corrected` naming only the fields that reader reads.
    for (q, expected) in [
        (
            "\"number 1\"",
            r#"{"key":"x:1","themes":["T"],"entities":[],"event":"2026-09/E01","event_name":"One","ai_related":true,"month":"2026-09","confidence":null,"run_id":null,"prompt_version":null}"#,
        ),
        (
            "\"number 2\"",
            r#"{"key":"x:2","themes":["T"],"entities":[],"event":"2026-09/E01","event_name":"One","ai_related":true,"month":"2026-09","confidence":null,"run_id":null,"prompt_version":null}"#,
        ),
        (
            "\"number 3\"",
            r#"{"key":"x:3","themes":["Fixed"],"entities":[],"event":"2026-09/E02","event_name":"Two","ai_related":true,"month":"2026-09","confidence":null,"run_id":null,"prompt_version":null,"corrected":["themes"]}"#,
        ),
        (
            "\"number 4\"",
            r#"{"key":"x:4","themes":["T"],"entities":[],"event":null,"event_name":null,"ai_related":false,"month":"2026-09","confidence":null,"run_id":null,"prompt_version":null,"corrected":["ai_related"]}"#,
        ),
    ] {
        assert_eq!(tags(&a.engine, q), expected, "{q}");
    }

    // The event facet and the build's counts, as before lasting events.
    let f: Value = serde_json::from_str(&facets_json(&a.engine, "proof")).unwrap();
    assert_eq!(
        f["facets"]["event"],
        json!({"distinct": 2, "items": [
            {"value": "2026-09/E01", "count": 2, "name": "One"},
            {"value": "2026-09/E02", "count": 1, "name": "Two"},
        ]})
    );
    assert_eq!(f["facets"]["theme"]["distinct"], json!(2));
    assert_eq!(f["facets"]["ai"], json!({"true": 3, "false": 1}));
    let r = &a.report;
    assert_eq!(
        (
            r.corrections,
            r.corrections_unmatched,
            r.corrections_bad_lines,
            r.bad_lines
        ),
        (3, 0, 0, 0)
    );
    assert!(r.events.is_none());

    // And byte for byte what the same archive without those keys gives.
    for q in [
        "event:2026-09/E02",
        "event:2026-09/E01",
        "proof",
        "theme:Fixed",
        "ai:false",
    ] {
        assert_eq!(
            search_json(&a.engine, with.path(), q),
            search_json(&b.engine, plain.path(), q),
            "search {q}"
        );
        assert_eq!(
            facets_json(&a.engine, q),
            facets_json(&b.engine, q),
            "facets {q}"
        );
    }
    assert_eq!(topics_json(&a.engine), topics_json(&b.engine));
    let report = |o: &One, root: &Path| {
        serde_json::to_string(&o.report)
            .unwrap()
            .replace(&root.display().to_string(), "<root>")
    };
    assert_eq!(report(&a, with.path()), report(&b, plain.path()));
    for text in [
        search_json(&a.engine, with.path(), "proof"),
        facets_json(&a.engine, "proof"),
        report(&a, with.path()),
    ] {
        assert!(!text.contains("\"events\""), "{text}");
    }
}

#[test]
fn the_same_archive_with_a_records_folder_reads_the_events_list_and_the_corrections() {
    // The other side of the gate: an empty records folder is enough.
    let with = tempfile::tempdir().unwrap();
    no_records_archive(with.path(), true);
    fs::create_dir_all(with.path().join(events::EVENTS_DIR)).unwrap();
    let a = one(with.path());
    // x:1 is in E01 and E02 (its `events` list); x:2 was moved to E02.
    assert_eq!(hits(&a.engine, "event:2026-09/E02"), vec![1, 2, 3]);
    assert_eq!(hits(&a.engine, "event:2026-09/E01"), vec![1]);
    // The two wrongly typed lines are skipped whole.
    assert_eq!(
        (a.report.corrections, a.report.corrections_bad_lines),
        (1, 2)
    );
    assert!(tags(&a.engine, "\"number 2\"").contains(r#""events":["2026-09/E02"]"#));
}

/// Two corpora, each with its own event under the SAME id. `records`
/// says which of alpha and beta has a records folder, holding a record
/// `shared` (different names) for its monthly event; a corpus without one
/// has the monthly id itself.
struct Two {
    _dirs: Vec<tempfile::TempDir>,
    engine: Engine,
}

fn two_corpora(records: [bool; 2], works_in_alpha: u32) -> Two {
    two_corpora_with(records, works_in_alpha, false)
}

/// [`two_corpora`]; `empty_folder` gives beta a records folder with
/// nothing in it.
fn two_corpora_with(records: [bool; 2], works_in_alpha: u32, empty_folder: bool) -> Two {
    let (a, b, data) = (
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    for n in 1..=works_in_alpha {
        work(a.path(), n);
    }
    work(b.path(), 9);
    let rows: Vec<String> = (1..=works_in_alpha)
        .map(|n| row(n, Some(("2026-09/E01", "Alpha story")), ""))
        .collect();
    write(a.path(), topics::LAYER_FILE, &rows.join("\n"));
    write(
        b.path(),
        topics::LAYER_FILE,
        &row(9, Some(("2026-09/E01", "Beta story")), ""),
    );
    for (root, name, has) in [
        (a.path(), "Alpha lasting", records[0]),
        (b.path(), "Beta lasting", records[1]),
    ] {
        if has {
            write(
                root,
                &format!("{}/shared.md", events::EVENTS_DIR),
                &format!("---\nid: shared\nname: {name}\nmembers: [2026-09/E01]\n---\n"),
            );
        }
    }
    if empty_folder {
        fs::create_dir_all(b.path().join(events::EVENTS_DIR)).unwrap();
    }
    let cfgs = vec![config("alpha", a.path()), config("beta", b.path())];
    for c in &cfgs {
        Corpus::from_config(c.clone(), data.path())
            .build_index(false)
            .unwrap();
    }
    let engine = Engine::new(Registry { corpora: cfgs }, data.path());
    Two {
        _dirs: vec![a, b, data],
        engine,
    }
}

fn event_facet(e: &Engine, in_: &[&str], min: usize) -> Value {
    let body = e
        .facets(&FacetsQuery {
            query: "proof".into(),
            in_: in_.iter().map(|s| s.to_string()).collect(),
            top: 0,
            min_event_items: min,
        })
        .unwrap()
        .body;
    serde_json::to_value(&body).unwrap()["facets"]["event"].clone()
}

/// An event value of `scout topics events`: value, count, name, corpus.
type EventItem = (String, usize, Option<String>, Option<String>);

fn topics_events(e: &Engine) -> (usize, Vec<EventItem>) {
    let topics = e
        .topics(&TopicsQuery {
            field: "events".into(),
            ..Default::default()
        })
        .unwrap()
        .body;
    let items = topics
        .items
        .iter()
        .map(|i| (i.value.clone(), i.count, i.name.clone(), i.corpus.clone()))
        .collect();
    (topics.distinct, items)
}

fn item(value: &str, count: usize, name: &str, corpus: Option<&str>) -> EventItem {
    (
        value.into(),
        count,
        Some(name.into()),
        corpus.map(String::from),
    )
}

#[test]
fn with_records_facets_keep_the_events_of_two_corpora_apart_under_one_id() {
    let t = two_corpora([true, true], 1);
    let e = &t.engine;
    // `scout events`: two events, one per corpus.
    let listed = e.events(&EventsQuery::default()).unwrap().body;
    assert_eq!(listed.distinct, 2);
    // Two events of one item each, each with its own name.
    assert_eq!(
        event_facet(e, &[], 1),
        json!({"distinct": 2, "items": [
            {"value": "shared", "count": 1, "name": "Alpha lasting", "corpus": "alpha"},
            {"value": "shared", "count": 1, "name": "Beta lasting", "corpus": "beta"},
        ]})
    );
    // Neither has two items in its corpus, so the floor leaves both out.
    assert_eq!(event_facet(e, &[], 2), json!({"distinct": 0, "items": []}));
    // `scout topics events` lists them the same way.
    assert_eq!(
        topics_events(e),
        (
            2,
            vec![
                item("shared", 1, "Alpha lasting", Some("alpha")),
                item("shared", 1, "Beta lasting", Some("beta")),
            ]
        )
    );
    // One corpus searched: the value has no `corpus`.
    assert_eq!(
        event_facet(e, &["beta"], 1),
        json!({"distinct": 1, "items": [{"value": "shared", "count": 1, "name": "Beta lasting"}]})
    );
    // The query still takes the id in each corpus: both works match, and
    // `--in` narrows to one corpus's event.
    assert_eq!(hits(e, "event:shared"), vec![1, 9]);
    assert_eq!(hits(e, "event:shared in:beta"), vec![9]);
}

#[test]
fn without_any_records_folder_the_event_facet_is_one_value_per_id_as_before() {
    // What `main` gave before lasting events for two corpora sharing a
    // monthly id: one value, both documents, the most frequent layer name
    // (a tie: the alphabetically first), and no `corpus`.
    let t = two_corpora([false, false], 1);
    let e = &t.engine;
    let one = json!({"distinct": 1, "items": [
        {"value": "2026-09/E01", "count": 2, "name": "Alpha story"},
    ]});
    assert_eq!(event_facet(e, &[], 1), one);
    assert_eq!(
        topics_events(e),
        (1, vec![item("2026-09/E01", 2, "Alpha story", None)])
    );
    // The floor measures that one value: two items over the two corpora.
    assert_eq!(event_facet(e, &[], 2), one);
    assert_eq!(event_facet(e, &[], 3), json!({"distinct": 0, "items": []}));
    assert_eq!(
        event_facet(e, &["beta"], 1),
        json!({"distinct": 1, "items": [{"value": "2026-09/E01", "count": 1, "name": "Beta story"}]})
    );
    assert_eq!(hits(e, "event:2026-09/E01"), vec![1, 9]);
    assert_eq!(hits(e, "event:2026-09/E01 in:beta"), vec![9]);
}

#[test]
fn one_records_folder_among_the_searched_corpora_makes_events_per_corpus() {
    // Alpha has records, beta has none.
    let t = two_corpora([true, false], 1);
    let e = &t.engine;
    assert_eq!(
        event_facet(e, &[], 1),
        json!({"distinct": 2, "items": [
            {"value": "2026-09/E01", "count": 1, "name": "Beta story", "corpus": "beta"},
            {"value": "shared", "count": 1, "name": "Alpha lasting", "corpus": "alpha"},
        ]})
    );
    assert_eq!(
        topics_events(e),
        (
            2,
            vec![
                item("2026-09/E01", 1, "Beta story", Some("beta")),
                item("shared", 1, "Alpha lasting", Some("alpha")),
            ]
        )
    );
    // The rule looks at the corpora searched, not at the registry: beta
    // alone has no records folder and one corpus never carries `corpus`.
    assert_eq!(
        event_facet(e, &["beta"], 1),
        json!({"distinct": 1, "items": [{"value": "2026-09/E01", "count": 1, "name": "Beta story"}]})
    );
    // A records folder with nothing in it is a records folder.
    let t = two_corpora_with([false, false], 1, true);
    assert_eq!(
        event_facet(&t.engine, &[], 1),
        json!({"distinct": 2, "items": [
            {"value": "2026-09/E01", "count": 1, "name": "Alpha story", "corpus": "alpha"},
            {"value": "2026-09/E01", "count": 1, "name": "Beta story", "corpus": "beta"},
        ]})
    );
}

#[test]
fn the_event_floor_measures_each_event_in_its_own_corpus() {
    // Alpha's event has two items, beta's one: three under the shared id.
    let t = two_corpora([true, true], 2);
    assert_eq!(
        event_facet(&t.engine, &[], 2),
        json!({"distinct": 1, "items": [
            {"value": "shared", "count": 2, "name": "Alpha lasting", "corpus": "alpha"},
        ]})
    );
    assert_eq!(
        event_facet(&t.engine, &[], 3),
        json!({"distinct": 0, "items": []})
    );
    // Without a floor: by count, then id, then corpus.
    assert_eq!(
        event_facet(&t.engine, &[], 1),
        json!({"distinct": 2, "items": [
            {"value": "shared", "count": 2, "name": "Alpha lasting", "corpus": "alpha"},
            {"value": "shared", "count": 1, "name": "Beta lasting", "corpus": "beta"},
        ]})
    );
}

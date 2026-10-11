//! Seam tests for the topic layer: the reader (missing file, bad lines,
//! corrections, unmatched keys), the join into the derived index, the
//! query fields `theme: entity: event: ai: month:` in search, facet
//! counting (a facet count = the matching query's document total) and the
//! `facets` / `topics` facade calls. The archive is never written.

use scout_corpus::api::{FacetsQuery, SearchQuery, TopicsQuery};
use scout_corpus::registry::{CorpusConfig, CorpusKind, DocumentUnit, FieldMap};
use scout_corpus::topics::{self, TopicField};
use scout_corpus::{Corpus, Engine, Registry, SearchRequest, SearchResults, SliceSpec};
use std::fs;
use std::path::Path;

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

fn work(source: &str, id: Option<&str>, title: &str, quote: &str, at: &str) -> String {
    let id = id
        .map(|i| format!("source_id: \"{i}\"\n"))
        .unwrap_or_default();
    format!(
        "---\ntitle: {title}\nauthor: A\ntype: tweet\nsource_system: {source}\n{id}---\n\n> {quote}\n\nhighlighted_at: {at} | tags: like\n\n---\n"
    )
}

const LAKOFF: &str = "readings/works/lakoff-abc.md";
const FRAMES: &str = "readings/works/frames-1939901126326764018.md";
const ASTRAL: &str = "readings/works/astral-rw-8497389.md";
const PLAIN: &str = "readings/works/plain-untagged.md";

fn archive(root: &Path) {
    write(
        root,
        LAKOFF,
        &work(
            "zotero",
            Some("ABC"),
            "Metaphors",
            "Metaphor is pervasive in thought.",
            "2024-01-15",
        ),
    );
    write(
        root,
        FRAMES,
        &work(
            "x",
            Some("1939901126326764018"),
            "Frames",
            "Tao says frames and metaphor matter for proofs.",
            "2026-09-03",
        ),
    );
    // No source_id: only the layer's path can join it.
    write(
        root,
        ASTRAL,
        &work(
            "readwise",
            None,
            "Willpower",
            "The brain is an inference engine.",
            "2026-09-10",
        ),
    );
    write(
        root,
        PLAIN,
        &work(
            "readwise",
            Some("rw_plain"),
            "Plain",
            "Nothing tagged here, metaphor aside.",
            "2026-09-11",
        ),
    );
}

fn layer_lines() -> String {
    [
        // A first line for ABC that the later one replaces.
        r#"{"key":"zotero:ABC","path":"readings/works/lakoff-abc.md","date":"2024-01-15","month":"2024-01","ai_related":true,"themes":["Old theme"],"entities":[],"event":null,"event_name":null}"#,
        r#"{"key":"zotero:ABC","path":"readings/works/lakoff-abc.md","date":"2024-01-15","month":"2024-01","ai_related":false,"themes":["Metaphor and cognition"],"entities":["George Lakoff"],"event":"2024-01/E02","event_name":"Lakoff lecture","confidence":"high","run_id":"r1","prompt_version":"topics-v2"}"#,
        r#"{"key":"x:1939901126326764018","path":"readings/works/renamed.md","date":"2026-09-03","month":"2026-09","ai_related":true,"themes":["AI and mathematics","Metaphor and cognition"],"entities":["GPT-6 Astra","Terence Tao"],"event":"2026-09/E21","event_name":"Fields Medallists' declaration","confidence":"medium","run_id":"r9","prompt_version":"topics-v2"}"#,
        r#"{"key":"readwise:rw_book_8497389","path":"readings/works/astral-rw-8497389.md","date":"2026-09-10","month":"2026-09","ai_related":true,"themes":["ai and MATHEMATICS"],"entities":["gpt-6 astra"],"event":"2026-09/E21","event_name":null,"confidence":"low","run_id":"r9","prompt_version":"topics-v2"}"#,
        r#"{"key":"x:999","path":"readings/works/gone.md","date":"2026-09-12","month":"2026-09","ai_related":true,"themes":["AI and mathematics"],"entities":[],"event":null,"event_name":null}"#,
        r#"{"key":"x:1000","path":"readings/works/gon"#,
        "",
    ]
    .join("\n")
}

fn cfg(root: &Path) -> CorpusConfig {
    CorpusConfig {
        id: "highlights".into(),
        name: "Highlights".into(),
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

struct Fx {
    src: tempfile::TempDir,
    data: tempfile::TempDir,
    corpus: Corpus,
}

fn fixture(with_layer: bool) -> Fx {
    let (src, data) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    archive(src.path());
    if with_layer {
        write(src.path(), topics::LAYER_FILE, &layer_lines());
    }
    let corpus = Corpus::from_config(cfg(src.path()), data.path());
    Fx { src, data, corpus }
}

fn search(c: &Corpus, q: &str) -> SearchResults {
    let mut r = SearchRequest::new(q);
    r.limit = 100;
    c.search(&r).unwrap()
}

fn keys(r: &SearchResults) -> Vec<String> {
    r.results.iter().map(|d| d.rel_path.clone()).collect()
}

fn engine(fx: &Fx) -> Engine {
    let reg = Registry {
        corpora: vec![fx.corpus.config.clone()],
    };
    Engine::new(reg, fx.data.path())
}

#[test]
fn missing_layer_is_a_no_op() {
    let fx = fixture(false);
    assert!(topics::read_layer(fx.src.path()).unwrap().is_none());
    let r = fx.corpus.build_index(false).unwrap();
    assert!(r.topics.is_none());
    assert_eq!(r.docs, 4);
    // Topic fields select nothing; they are not errors.
    assert!(search(&fx.corpus, "theme:\"AI and mathematics\"")
        .results
        .is_empty());
    assert!(search(&fx.corpus, "metaphor").results[0].topics.is_none());
    let t = engine(&fx).topics(&TopicsQuery::default()).unwrap().body;
    assert_eq!(t.tagged_documents, 0);
    assert!(t.items.is_empty());
}

#[test]
fn reader_counts_bad_lines_and_keeps_the_last_line_per_key() {
    let fx = fixture(true);
    let l = topics::read_layer(fx.src.path()).unwrap().unwrap();
    assert_eq!(l.items.len(), 4);
    assert_eq!(l.bad_lines, 1, "the cut-off last line");
    let abc = &l.items["zotero:ABC"].info;
    assert_eq!(abc.themes, vec!["Metaphor and cognition"]);
    assert_eq!(abc.ai_related, Some(false));
    assert_eq!(abc.month.as_deref(), Some("2024-01"));
    // A missing event name is filled from another item with that event.
    assert_eq!(
        l.items["readwise:rw_book_8497389"]
            .info
            .event_name
            .as_deref(),
        Some("Fields Medallists' declaration")
    );
}

#[test]
fn build_joins_by_key_then_path_and_reports_the_rate() {
    let fx = fixture(true);
    let before = snapshot(fx.src.path());
    let r = fx.corpus.build_index(false).unwrap();
    let t = r.topics.expect("layer report");
    assert_eq!(t.items, 4);
    assert_eq!(t.matched, 3);
    // The X work joins by key although its layer path is stale.
    assert_eq!(t.matched_by_key, 2);
    assert_eq!(t.matched_by_path, 1);
    assert_eq!(t.unmatched, 1);
    assert_eq!(t.unmatched_sample, vec!["x:999"]);
    assert_eq!(t.tagged_documents, 3);
    assert_eq!(t.bad_lines, 1);
    assert!(t.reloaded);
    assert_eq!(snapshot(fx.src.path()), before, "archive untouched");

    // Nothing changed: the stored copy is kept and reported again.
    let again = fx.corpus.build_index(false).unwrap().topics.unwrap();
    assert!(!again.reloaded);
    assert_eq!(
        topics::TopicReport {
            reloaded: true,
            ..again
        },
        t
    );
    // Delete and rebuild: identical results (invariant 4).
    let q = "event:2026-09/E21";
    let a = search(&fx.corpus, q);
    fx.corpus.build_index(true).unwrap();
    let b = search(&fx.corpus, q);
    assert_eq!(
        serde_json::to_string(&a).unwrap(),
        serde_json::to_string(&b).unwrap()
    );
}

#[test]
fn topic_fields_filter_search_and_combine_with_text() {
    let fx = fixture(true);
    fx.corpus.build_index(false).unwrap();
    let c = &fx.corpus;
    assert_eq!(
        keys(&search(c, "theme:\"ai and MATHEMATICS\"")),
        vec![ASTRAL, FRAMES]
    );
    assert_eq!(
        keys(&search(c, "theme:\"AI and mathematics\" Tao")),
        vec![FRAMES]
    );
    assert_eq!(
        keys(&search(c, "entity:\"GPT-6 Astra\"")),
        vec![ASTRAL, FRAMES]
    );
    // Exact names only: a part of a name is not a match.
    assert!(search(c, "entity:Astra").results.is_empty());
    assert_eq!(keys(&search(c, "event:2026-09/e21")), vec![ASTRAL, FRAMES]);
    assert_eq!(keys(&search(c, "ai:false")), vec![LAKOFF]);
    // Untagged documents fail ai:false too.
    assert!(!keys(&search(c, "ai:false metaphor")).contains(&PLAIN.to_string()));
    assert_eq!(keys(&search(c, "month:2024")), vec![LAKOFF]);
    assert_eq!(keys(&search(c, "month:2026-09 -inference")), vec![FRAMES]);
    assert_eq!(
        keys(&search(c, "theme:\"Metaphor and cognition\" y:2024")),
        vec![LAKOFF],
        "y: is the document's date, month: the layer's"
    );
    assert_eq!(
        keys(&search(c, "metaphor theme:\"Metaphor and cognition\"")).len(),
        2
    );
    // tag: keeps its meaning: the reader's own highlight tags.
    assert!(search(c, "tag:\"AI and mathematics\"").results.is_empty());
    assert_eq!(search(c, "tag:like").results.len(), 4);
    // A result carries its tags.
    let d = &search(c, "Tao").results[0];
    let t = d.topics.as_ref().unwrap();
    assert_eq!(t.key, "x:1939901126326764018");
    assert_eq!(t.event.as_deref(), Some("2026-09/E21"));
    // Bad values are usage errors.
    let bad = |q: &str| c.search(&SearchRequest::new(q)).is_err();
    assert!(bad("ai:maybe"));
    assert!(bad("month:2026-13"));
    assert!(bad("theme:"));
}

#[test]
fn slices_take_topic_fields() {
    let s =
        SliceSpec::parse("highlights theme:\"AI and mathematics\" ai:true month:2026-09").unwrap();
    assert_eq!(s.filter.theme.as_deref(), Some("AI and mathematics"));
    assert_eq!(s.filter.ai, Some(true));
    assert_eq!(
        s.filter.spec(),
        "theme:\"AI and mathematics\" ai:true month:2026-09"
    );
}

#[test]
fn corrections_override_fields_and_unknown_keys_are_counted() {
    let fx = fixture(true);
    fx.corpus.build_index(false).unwrap();
    write(
        fx.src.path(),
        topics::CORRECTIONS_FILE,
        &[
            r#"{"key":"x:1939901126326764018","themes":["Proof and verification"],"by":"Dominik","date":"2026-10-11"}"#,
            r#"{"key":"x:1939901126326764018","event":null}"#,
            r#"{"key":"readwise:rw_book_8497389","event":"2024-01/E02","ai_related":false}"#,
            r#"{"key":"x:404","themes":["Nope"]}"#,
            r#"{"key":"zotero:ABC","themes":"not a list"}"#,
            "",
        ]
        .join("\n"),
    );
    let before = snapshot(fx.src.path());
    let t = fx.corpus.build_index(false).unwrap().topics.unwrap();
    assert!(t.reloaded, "a changed corrections file re-joins");
    assert_eq!(t.corrections, 3);
    assert_eq!(t.corrections_unmatched, 1);
    assert_eq!(t.corrections_bad_lines, 1);
    assert_eq!(snapshot(fx.src.path()), before);
    let c = &fx.corpus;
    assert!(search(c, "theme:\"AI and mathematics\" Tao")
        .results
        .is_empty());
    assert_eq!(
        keys(&search(c, "theme:\"Proof and verification\"")),
        vec![FRAMES]
    );
    assert!(search(c, "event:2026-09/E21").results.is_empty());
    assert_eq!(keys(&search(c, "event:2024-01/E02")), vec![ASTRAL, LAKOFF]);
    let d = &search(c, "inference").results[0];
    let tags = d.topics.as_ref().unwrap();
    assert_eq!(tags.corrected, vec!["event", "ai_related"]);
    // The new event's name comes from the layer, not the old event.
    assert_eq!(tags.event_name.as_deref(), Some("Lakoff lecture"));
    // The bad correction left ABC as the layer has it.
    assert_eq!(
        keys(&search(c, "theme:\"Metaphor and cognition\"")),
        vec![LAKOFF]
    );
}

#[test]
fn facet_counts_equal_the_matching_query_totals() {
    let fx = fixture(true);
    fx.corpus.build_index(false).unwrap();
    let e = engine(&fx);
    let f = e
        .facets(&FacetsQuery {
            query: "tag:like".into(),
            top: 0,
            ..Default::default()
        })
        .unwrap()
        .body;
    assert_eq!(f.total_documents, 4);
    assert_eq!(f.tagged_documents, 3);
    let themes: Vec<(&str, usize)> = f
        .facets
        .theme
        .items
        .iter()
        .map(|i| (i.value.as_str(), i.count))
        .collect();
    // Case variants fold into one value, shown in its most frequent
    // spelling (ties: alphabetical, so the capitalised one).
    assert_eq!(
        themes,
        vec![("AI and mathematics", 2), ("Metaphor and cognition", 2)]
    );
    let ev = &f.facets.event.items[0];
    assert_eq!(
        (ev.value.as_str(), ev.count, ev.name.as_deref()),
        ("2026-09/E21", 2, Some("Fields Medallists' declaration"))
    );
    let months: Vec<&str> = f
        .facets
        .month
        .items
        .iter()
        .map(|i| i.value.as_str())
        .collect();
    assert_eq!(months, vec!["2024-01", "2026-09"], "chronological");
    assert_eq!((f.facets.ai.yes, f.facets.ai.no), (2, 1));
    for (field, facet) in [
        ("theme", &f.facets.theme),
        ("entity", &f.facets.entity),
        ("event", &f.facets.event),
        ("month", &f.facets.month),
    ] {
        for it in &facet.items {
            let q = format!("tag:like {field}:\"{}\"", it.value);
            let r = e
                .search(&SearchQuery {
                    query: q.clone(),
                    limit: 100,
                    ..Default::default()
                })
                .unwrap()
                .body;
            assert_eq!(r.total_documents, it.count, "{q}");
        }
    }
    // Narrowed by text.
    let f = e
        .facets(&FacetsQuery {
            query: "event:2026-09/E21 Tao".into(),
            ..Default::default()
        })
        .unwrap()
        .body;
    assert_eq!(f.total_documents, 1);
    assert_eq!(f.facets.entity.items.len(), 2);
    assert!(e
        .facets(&FacetsQuery {
            query: "  ".into(),
            ..Default::default()
        })
        .is_err());
}

#[test]
fn topics_lists_values_with_counts() {
    let fx = fixture(true);
    fx.corpus.build_index(false).unwrap();
    let e = engine(&fx);
    let list = |field: &str, month: Option<&str>| {
        e.topics(&TopicsQuery {
            field: field.into(),
            month: month.map(String::from),
            ..Default::default()
        })
        .unwrap()
        .body
    };
    let t = list("themes", None);
    assert_eq!(t.field, TopicField::Theme);
    assert_eq!(t.tagged_documents, 3);
    assert_eq!(t.distinct, 2);
    let t = list("entities", Some("2026-09"));
    assert_eq!(t.tagged_documents, 2);
    assert_eq!(t.items[0].value, "GPT-6 Astra");
    assert_eq!(t.items[0].count, 2);
    let t = list("events", Some("2024"));
    assert_eq!(t.items.len(), 1);
    assert_eq!(t.items[0].name.as_deref(), Some("Lakoff lecture"));
    assert!(e
        .topics(&TopicsQuery {
            field: "colours".into(),
            ..Default::default()
        })
        .is_err());
    assert!(e
        .topics(&TopicsQuery {
            month: Some("Sept".into()),
            ..Default::default()
        })
        .is_err());
}

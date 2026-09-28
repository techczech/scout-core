//! Seam tests for the app-facing facade (`api::Engine`): request JSON and
//! defaults, background-thread safety, and the cleaning report.

use scout_corpus::api::{self, *};
use scout_corpus::{Engine, Registry};
use std::fs;
use std::path::Path;

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

/// A writing corpus with a triple-encoded entity, a curly apostrophe, a
/// signature line matched by the boilerplate rule and a Markdown link.
fn engine(tmp: &Path) -> (Engine, std::path::PathBuf) {
    let w = tmp.join("writing");
    write(
        &w,
        "a.md",
        "---\ntitle: A\ndate: 2016\ngenre: essay\n---\n\
         Rock &amp;amp;amp; roll is a metaphor.\n\n\
         I don’t mind a metaphor.\n\n\
         *Dominik Lukeš*\n\n\
         See [the essay](https://example.org/x) on metaphor.\n",
    );
    write(
        &w,
        "b.md",
        "---\ntitle: B\ndate: 2021\ngenre: note\n---\n\
         Another &amp;amp;amp; one; we don’t stop.\n\n\
         *Dominik Lukeš*\n",
    );
    let reg = Registry::parse(&format!(
        "[[corpus]]\nid = \"writing\"\nname = \"W\"\nkind = \"markdown-folder\"\npath = \"{}\"\nrequire_frontmatter = [\"genre\"]\nboilerplate = [\"^\\\\*Dominik Lukeš\\\\*$\"]\n",
        w.display()
    ))
    .unwrap();
    (Engine::new(reg, tmp.join("indexes")), w)
}

#[test]
fn requests_deserialise_from_partial_json_with_cli_defaults() {
    let q: KwicQuery =
        serde_json::from_str(r#"{"term": "metaphor", "scope": {"in": ["writing"]}}"#).unwrap();
    assert_eq!(q.width, 8);
    assert_eq!(q.sort, "R1");
    assert_eq!(q.limit, 200);
    assert_eq!(q.scope.in_, vec!["writing"]);
    let c: CollocatesQuery = serde_json::from_str(r#"{"node": "x"}"#).unwrap();
    assert_eq!(
        (c.window, c.score.as_str(), c.min, c.top),
        (5, "logdice", 5, 30)
    );
    let k: KeynessQuery =
        serde_json::from_str(r#"{"a": "before:2015", "b": "after:2020"}"#).unwrap();
    assert_eq!((k.top, k.min), (40, 5));
    let n: NgramsQuery = serde_json::from_str("{}").unwrap();
    assert_eq!((n.n.as_str(), n.top), ("3-5", 50));
    let s: SearchQuery = serde_json::from_str(r#"{"query": "x"}"#).unwrap();
    assert_eq!((s.limit, s.passage), (20, false));
    assert_eq!(
        serde_json::from_str::<ProfileQuery>("{}").unwrap().min_hits,
        3
    );
    assert_eq!(
        serde_json::from_str::<CleanReportQuery>("{}")
            .unwrap()
            .samples,
        5
    );
}

#[test]
fn engine_is_safe_on_background_threads() {
    fn send_sync<T: Send + Sync + Clone + 'static>() {}
    send_sync::<Engine>();
    let tmp = tempfile::tempdir().unwrap();
    let (e, _) = engine(tmp.path());
    e.index_build(&IndexBuildQuery::default()).unwrap();
    let q = KwicQuery {
        term: "metaphor".into(),
        ..Default::default()
    };
    let here = api::to_json(&e.kwic(&q).unwrap().body).unwrap();
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let (e, q) = (e.clone(), q.clone());
            std::thread::spawn(move || api::to_json(&e.kwic(&q).unwrap().body).unwrap())
        })
        .collect();
    for h in handles {
        assert_eq!(h.join().unwrap(), here);
    }
}

#[test]
fn missing_index_notes_and_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let (e, _) = engine(tmp.path());
    let q = KwicQuery {
        term: "metaphor".into(),
        ..Default::default()
    };
    let err = e.kwic(&q).unwrap_err();
    assert!(err
        .downcast_ref::<scout_corpus::NoIndexedCorpus>()
        .is_some());
    assert!(e
        .search(&SearchQuery::default())
        .unwrap_err()
        .to_string()
        .contains("empty query"));
}

#[test]
fn clean_report_counts_rules_with_before_after_samples() {
    let tmp = tempfile::tempdir().unwrap();
    let (e, w) = engine(tmp.path());
    let before = fs::read(w.join("a.md")).unwrap();
    let r = e
        .clean_report(&CleanReportQuery {
            corpus: "writing".into(),
            rules: vec!["^See ".into()],
            ..Default::default()
        })
        .unwrap()
        .body;
    assert_eq!((r.documents, r.schema_version), (2, 1));
    let rule = |id: &str| r.rules.iter().find(|x| x.rule == id).unwrap();

    // Entities: both triple-encoded runs, grouped as one "amp amp amp" change.
    let ent = rule("entities");
    assert_eq!(ent.passages, 2);
    assert_eq!(ent.token_passages, 2);
    let s = &ent.samples[0];
    assert_eq!(s.removed, vec!["amp", "amp", "amp"]);
    assert!(s.added.is_empty());
    assert_eq!(s.occurrences, 2);
    assert_eq!((s.rel_path.as_str(), s.line), ("a.md", 6));
    assert_eq!(s.original, "Rock &amp;amp;amp; roll is a metaphor.");
    assert_eq!(s.after, vec!["rock", "roll", "is", "a", "metaphor"]);

    // Apostrophes: the curly form, counted as the straight one.
    let ap = rule("apostrophes");
    assert_eq!(ap.passages, 2);
    let s = &ap.samples[0];
    assert_eq!(s.removed, vec!["don’t"]);
    assert_eq!(s.added, vec!["don't"]);
    assert_eq!(s.original, "I don’t mind a metaphor.");

    // Boilerplate: the registry rule and a preview rule, each credited.
    let bp = rule("boilerplate");
    assert_eq!(bp.passages, 3);
    let pats = bp.patterns.as_ref().unwrap();
    assert_eq!(pats.len(), 2);
    assert_eq!(
        (pats[0].origin.as_str(), pats[0].lines, pats[0].passages),
        ("registry", 2, 2)
    );
    assert_eq!(
        (pats[1].origin.as_str(), pats[1].lines, pats[1].passages),
        ("preview", 1, 1)
    );
    let sig = bp.samples.iter().find(|s| s.pattern == Some(1)).unwrap();
    assert_eq!(sig.before, vec!["dominik", "lukeš"]);
    assert!(sig.after.is_empty());
    assert_eq!(sig.occurrences, 2);

    // Links: the only link sits on the line the preview rule drops, and a
    // later stage sees only what boilerplate left.
    assert_eq!(rule("links_urls").passages, 0);
    let no_preview = e
        .clean_report(&CleanReportQuery {
            corpus: "writing".into(),
            ..Default::default()
        })
        .unwrap()
        .body;
    let ln = no_preview
        .rules
        .iter()
        .find(|x| x.rule == "links_urls")
        .unwrap();
    assert_eq!(ln.passages, 1);
    assert_eq!(ln.samples[0].removed, vec!["https", "example.org", "x"]);
    assert!(ln.samples[0].added.is_empty());

    // Sample count is capped; the sources are untouched.
    let one = e
        .clean_report(&CleanReportQuery {
            corpus: "writing".into(),
            samples: 0,
            ..Default::default()
        })
        .unwrap()
        .body;
    assert!(one.rules.iter().all(|r| r.samples.is_empty()));
    assert_eq!(fs::read(w.join("a.md")).unwrap(), before);
}

/// Ticket 03: documents where some query terms are only in the title,
/// topics or summary.
fn fields_engine(tmp: &Path) -> Engine {
    let w = tmp.join("writing");
    write(
        &w,
        "essay.md",
        "---\ntitle: \"Repaved paths and generative metaphors\"\ngenre: essay\n---\n\
         Cows wander and people follow their paths.\n\n\
         Nothing to see in this paragraph.\n",
    );
    write(
        &w,
        "both.md",
        "---\ntitle: Notes on metaphor\ngenre: note\n---\n\
         Paths wander.\n\n\
         Paths are a metaphor for learning.\n",
    );
    write(
        &w,
        "topic.md",
        "---\ntitle: Walking\ngenre: note\ntopics: metaphor, walking\n---\n\
         Walking along forest paths.\n",
    );
    write(
        &w,
        "summary.md",
        "---\ntitle: Roads\ngenre: note\nsummary: A metaphor study.\n---\n\
         Roads and paths.\n",
    );
    write(
        &w,
        "neither.md",
        "---\ntitle: Gardens\ngenre: note\n---\nGarden paths only.\n",
    );
    let reg = Registry::parse(&format!(
        "[[corpus]]\nid = \"writing\"\nname = \"W\"\nkind = \"markdown-folder\"\npath = \"{}\"\nrequire_frontmatter = [\"genre\"]\n",
        w.display()
    ))
    .unwrap();
    let e = Engine::new(reg, tmp.join("indexes"));
    e.index_build(&IndexBuildQuery::default()).unwrap();
    e
}

fn search(e: &Engine, q: &str) -> scout_corpus::SearchResults {
    e.search(&SearchQuery {
        query: q.into(),
        ..Default::default()
    })
    .unwrap()
    .body
}

#[test]
fn search_takes_missing_terms_from_title_topics_and_summary() {
    let tmp = tempfile::tempdir().unwrap();
    let e = fields_engine(tmp.path());
    let r = search(&e, "paths metaphor");
    let order: Vec<&str> = r.results.iter().map(|d| d.rel_path.as_str()).collect();
    // A passage holding every term outranks the ones leaning on a field;
    // a document whose fields lack "metaphor" does not match.
    assert_eq!(order[0], "both.md");
    let mut rest = order[1..].to_vec();
    rest.sort();
    assert_eq!(rest, vec!["essay.md", "summary.md", "topic.md"]);

    // The shown passage is the one covering the most terms; "Paths wander."
    // matches too, the title supplying "metaphor".
    let both = &r.results[0];
    assert_eq!(both.hits[0].quote, "Paths are a metaphor for learning.");
    assert!(both.hits[0].field_terms.is_empty());
    assert_eq!(both.hits[1].quote, "Paths wander.");
    assert_eq!(both.hits[1].field_terms.len(), 1);
    assert!(both.score > r.results[1].score);

    // The essay quotes its "paths" passage, as written; the title supplies
    // "metaphor" and matches the whole query.
    let essay = r.results.iter().find(|d| d.rel_path == "essay.md").unwrap();
    assert_eq!(essay.hits.len(), 1);
    assert_eq!(
        essay.hits[0].quote,
        "Cows wander and people follow their paths."
    );
    assert_eq!(
        essay.hits[0].field_terms,
        vec![scout_corpus::search::Term::Prefix("metaphor".into())]
    );
    assert!(essay.title_match);

    // Deterministic output.
    let again = search(&e, "paths metaphor");
    assert_eq!(api::to_json(&again).unwrap(), api::to_json(&r).unwrap());

    // A term found only in fields never matches on its own.
    assert!(search(&e, "metaphor walking")
        .results
        .iter()
        .all(|d| d.rel_path == "topic.md"));
    assert!(search(&e, "study").results.is_empty());
}

#[test]
fn full_passage_outranks_title_only_match_of_equal_text() {
    let tmp = tempfile::tempdir().unwrap();
    let w = tmp.path().join("writing");
    // Same passage text; only one of them holds "metaphor" in the passage.
    write(
        &w,
        "a.md",
        "---\ntitle: Metaphor\ngenre: note\n---\nPaths metaphor.\n",
    );
    write(
        &w,
        "b.md",
        "---\ntitle: Metaphor\ngenre: note\n---\nPaths only.\n",
    );
    let reg = Registry::parse(&format!(
        "[[corpus]]\nid = \"writing\"\nname = \"W\"\nkind = \"markdown-folder\"\npath = \"{}\"\nrequire_frontmatter = [\"genre\"]\n",
        w.display()
    ))
    .unwrap();
    let e = Engine::new(reg, tmp.path().join("indexes"));
    e.index_build(&IndexBuildQuery::default()).unwrap();
    let r = search(&e, "paths metaphor");
    let order: Vec<&str> = r.results.iter().map(|d| d.rel_path.as_str()).collect();
    assert_eq!(order, vec!["a.md", "b.md"]);
}

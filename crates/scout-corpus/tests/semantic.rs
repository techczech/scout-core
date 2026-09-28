//! Seam tests for semantic search (ticket 07): the embedder trait (a fake
//! concept embedder; no model download), the vector store round trip and
//! incremental rebuild, hybrid fusion ordering, and the facade's mode
//! selection (apps without an embedder are unaffected).

use scout_corpus::api::{IndexBuildQuery, SearchQuery};
use scout_corpus::search::quote_is_original;
use scout_corpus::semantic::{store, Embedder, Freshness, HashEmbedder, SearchMode};
use scout_corpus::{Engine, Registry};
use std::fs;
use std::path::Path;
use std::sync::Arc;

/// A fake that embeds by concept: each dimension is one concept, lit by any
/// of its words in either language. "The mind as a machine" and "mysl je
/// počítač" share no word but share a concept.
struct Concepts;

const CONCEPTS: &[&[&str]] = &[
    &["mind", "mysl", "brain", "mozek", "thought"],
    &["machine", "computer", "počítač", "stroj", "computation"],
    &["garden", "tomato", "compost", "zahrada"],
    &["budget", "committee", "money"],
];

impl Embedder for Concepts {
    fn model_id(&self) -> String {
        "concepts-test".into()
    }
    fn embed_passages(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|t| embed(t)).collect())
    }
    fn embed_query(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        Ok(embed(text))
    }
}

fn embed(text: &str) -> Vec<f32> {
    let lower = text.to_lowercase();
    let mut v: Vec<f32> = CONCEPTS
        .iter()
        .map(|ws| ws.iter().filter(|w| lower.contains(*w)).count() as f32)
        .collect();
    v.push(0.05); // never a zero vector
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    v.iter().map(|x| x / n).collect()
}

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

fn doc(title: &str, date: &str, lang: &str, paras: &[&str]) -> String {
    format!(
        "---\ntitle: {title}\ndate: {date}\ngenre: essay\nlang: {lang}\n---\n{}\n",
        paras.join("\n\n")
    )
}

fn fixture(tmp: &Path) -> (Registry, std::path::PathBuf) {
    let w = tmp.join("writing");
    write(
        &w,
        "2005-mysl.md",
        &doc(
            "Nové myšlení",
            "2005-10-11",
            "cs",
            &[
                "Mysl je jako počítač, říká kognitivní věda.",
                "Zahrada, kompost a rajčata na slunci.",
            ],
        ),
    );
    write(
        &w,
        "2016-chess.md",
        &doc(
            "The chess master",
            "2016-06-15",
            "en",
            &["The mind as a machine: every thought a computation by a computer."],
        ),
    );
    write(
        &w,
        "2019-machine.md",
        &doc(
            "Machines",
            "2019-01-01",
            "en",
            &["A washing machine broke; the committee paid from the budget."],
        ),
    );
    write(
        &w,
        "2020-garden.md",
        &doc(
            "Garden",
            "2020-05-01",
            "en",
            &["Tomato compost in the garden."],
        ),
    );
    let reg = Registry::parse(&format!(
        "[[corpus]]\nid = \"writing\"\nname = \"W\"\nkind = \"markdown-folder\"\npath = \"{}\"\n",
        w.display()
    ))
    .unwrap();
    (reg, w)
}

fn engine(tmp: &Path) -> (Engine, std::path::PathBuf) {
    let (reg, w) = fixture(tmp);
    let e = Engine::new(reg, tmp.join("indexes")).with_embedder(Arc::new(Concepts));
    e.index_build(&IndexBuildQuery {
        semantic: true,
        ..Default::default()
    })
    .unwrap();
    (e, w)
}

fn search(e: &Engine, q: &str, mode: SearchMode) -> scout_corpus::SearchResults {
    e.search(&SearchQuery {
        query: q.into(),
        mode,
        ..Default::default()
    })
    .unwrap()
    .body
}

fn paths(r: &scout_corpus::SearchResults) -> Vec<&str> {
    r.results.iter().map(|d| d.rel_path.as_str()).collect()
}

#[test]
fn semantic_finds_the_meaning_without_the_words() {
    let tmp = tempfile::tempdir().unwrap();
    let (e, w) = engine(tmp.path());
    let r = search(&e, "the mind as a machine in:writing", SearchMode::Semantic);
    assert_eq!(r.mode.as_deref(), Some("semantic"));
    // Both mind-as-computer pieces first; the Czech one shares no word.
    let top: Vec<&str> = paths(&r)[..2].to_vec();
    assert!(
        top.contains(&"2005-mysl.md") && top.contains(&"2016-chess.md"),
        "{top:?}"
    );
    let czech = r
        .results
        .iter()
        .find(|d| d.rel_path == "2005-mysl.md")
        .unwrap();
    assert_eq!(
        czech.hits[0].quote,
        "Mysl je jako počítač, říká kognitivní věda."
    );
    assert!(czech.hits[0].semantic_score.unwrap() > 0.9);
    // Quotes are original text, passage-level (invariant 2).
    for d in &r.results {
        for h in &d.hits {
            assert!(quote_is_original(&w.join(&d.rel_path), h).unwrap());
        }
    }
    // Scores descend; ranks are relative to the best.
    assert!(r.results.windows(2).all(|p| p[0].score >= p[1].score));
    assert_eq!(r.results[0].rank, 1.0);
}

#[test]
fn query_fields_filter_semantic_hits() {
    let tmp = tempfile::tempdir().unwrap();
    let (e, _) = engine(tmp.path());
    let r = search(&e, "the mind as a machine lang:cs", SearchMode::Semantic);
    assert_eq!(paths(&r), vec!["2005-mysl.md"]);
    let r = search(
        &e,
        "the mind as a machine after:2010 -thought",
        SearchMode::Semantic,
    );
    assert!(!paths(&r).contains(&"2016-chess.md"));
    assert!(!paths(&r).contains(&"2005-mysl.md"));
}

#[test]
fn hybrid_fuses_words_and_meaning_by_reciprocal_rank() {
    let tmp = tempfile::tempdir().unwrap();
    let (e, _) = engine(tmp.path());
    // Full text: "machine" is in the chess piece and the washing machine.
    let fts = search(&e, "machine", SearchMode::Fts);
    assert_eq!(fts.mode, None);
    let mut f = paths(&fts);
    f.sort();
    assert_eq!(f, vec!["2016-chess.md", "2019-machine.md"]);
    // Hybrid (the default with vectors): the chess piece has the word AND
    // the meaning, so it leads; the Czech piece comes in on meaning alone.
    let auto = search(&e, "machine", SearchMode::Auto);
    assert_eq!(auto.mode.as_deref(), Some("hybrid"));
    assert_eq!(paths(&auto)[0], "2016-chess.md");
    assert!(paths(&auto).contains(&"2005-mysl.md"));
    assert!(paths(&auto).contains(&"2019-machine.md"));
    // RRF: a document in both lists outscores one in only one of them.
    let s = |p: &str| auto.results.iter().find(|d| d.rel_path == p).unwrap().score;
    // (No document in one list only can score more than 1/61.)
    assert!(s("2016-chess.md") > 1.0 / 61.0);
    assert!(s("2005-mysl.md") <= 1.0 / 61.0 + 1e-6);
    assert!(s("2016-chess.md") > s("2005-mysl.md"));
    // A full-text hit keeps its located quote and gains its cosine.
    let chess = &auto.results[0].hits[0];
    assert!(chess.semantic_score.is_some());
    let explicit = search(&e, "machine", SearchMode::Hybrid);
    assert_eq!(
        serde_json::to_string(&explicit).unwrap(),
        serde_json::to_string(&auto).unwrap()
    );
}

#[test]
fn output_is_byte_identical_across_runs() {
    let tmp = tempfile::tempdir().unwrap();
    let (e, _) = engine(tmp.path());
    for mode in [SearchMode::Semantic, SearchMode::Hybrid] {
        let a = serde_json::to_string(&search(&e, "mind machine garden", mode)).unwrap();
        let b = serde_json::to_string(&search(&e, "mind machine garden", mode)).unwrap();
        assert_eq!(a, b);
    }
}

#[test]
fn engine_without_embedder_is_unaffected() {
    let tmp = tempfile::tempdir().unwrap();
    let (e, _) = engine(tmp.path());
    let (reg, _) = fixture(tmp.path());
    let plain = Engine::new(reg, tmp.path().join("indexes"));
    // Vectors exist on disk, but an app without an embedder gets full text,
    // identical to an explicit fts search.
    let a = search(&plain, "machine", SearchMode::Auto);
    assert_eq!(a.mode, None);
    let b = search(&e, "machine", SearchMode::Fts);
    assert_eq!(
        serde_json::to_string(&a).unwrap(),
        serde_json::to_string(&b).unwrap()
    );
    assert!(!serde_json::to_string(&a)
        .unwrap()
        .contains("semantic_score"));
    assert!(plain
        .search(&SearchQuery {
            query: "machine".into(),
            mode: SearchMode::Semantic,
            ..Default::default()
        })
        .is_err());
    assert!(plain
        .index_build(&IndexBuildQuery {
            semantic: true,
            ..Default::default()
        })
        .is_err());
}

#[test]
fn vector_store_round_trip_and_incremental_rebuild() {
    let tmp = tempfile::tempdir().unwrap();
    let (reg, w) = fixture(tmp.path());
    let e = Engine::new(reg, tmp.path().join("indexes"))
        .with_embedder(Arc::new(HashEmbedder::default()));
    // No vectors until asked for: auto search is full text.
    e.index_build(&Default::default()).unwrap();
    assert_eq!(search(&e, "machine", SearchMode::Auto).mode, None);
    assert!(e
        .search(&SearchQuery {
            query: "machine".into(),
            mode: SearchMode::Semantic,
            ..Default::default()
        })
        .is_err());

    let q = IndexBuildQuery {
        semantic: true,
        ..Default::default()
    };
    let r = e.index_build(&q).unwrap().body.reports.remove(0);
    let v = r.vectors.unwrap();
    assert!(v.full);
    assert_eq!(
        (v.passages, v.embedded, v.reused),
        (r.passages as usize, 5, 0)
    );
    assert_eq!(v.dim, 64);

    let dir = tmp.path().join("indexes");
    let vpath = dir.join("writing.vectors.sqlite");
    let fts = rusqlite::Connection::open(dir.join("writing.sqlite")).unwrap();
    let loaded = store::load(&vpath).unwrap();
    assert_eq!(loaded.ids.len(), 5);
    assert_eq!(loaded.data.len(), 5 * 64);
    // The stored vector is the embedder's vector for the passage text.
    let (id, text): (i64, String) = fts
        .query_row(
            "SELECT id, normalized FROM passages ORDER BY id LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    let i = loaded.ids.iter().position(|&x| x == id).unwrap();
    let want = HashEmbedder::default()
        .embed_passages(&[&text])
        .unwrap()
        .remove(0);
    assert_eq!(loaded.get(i), want.as_slice());
    let model = HashEmbedder::default().model_id();
    assert_eq!(
        store::freshness(&fts, &vpath, &model).unwrap(),
        Freshness::Current
    );
    assert!(matches!(
        store::freshness(&fts, &vpath, "other-model").unwrap(),
        Freshness::Stale(_)
    ));

    // No change: nothing embedded (plain builds keep existing vectors up).
    let v = e.index_build(&Default::default()).unwrap().body.reports[0]
        .vectors
        .clone()
        .unwrap();
    assert_eq!((v.full, v.embedded, v.reused, v.removed), (false, 0, 5, 0));

    // One passage edited, one file removed: only the new text is embedded;
    // the unchanged passages of the edited file are reused though their ids
    // moved.
    write(
        &w,
        "2005-mysl.md",
        &doc(
            "Nové myšlení",
            "2005-10-11",
            "cs",
            &[
                "Mysl je jako počítač, říká kognitivní věda.",
                "Zahrada bez kompostu a bez rajčat letos.",
            ],
        ),
    );
    fs::remove_file(w.join("2020-garden.md")).unwrap();
    let v = e.index_build(&Default::default()).unwrap().body.reports[0]
        .vectors
        .clone()
        .unwrap();
    assert_eq!((v.full, v.embedded, v.passages), (false, 1, 4));
    assert_eq!(v.reused, 3);
    let fts = rusqlite::Connection::open(dir.join("writing.sqlite")).unwrap();
    assert_eq!(
        store::freshness(&fts, &vpath, &model).unwrap(),
        Freshness::Current
    );

    // A build by an engine without an embedder leaves the vectors stale;
    // auto search then falls back to full text with a note.
    write(
        &w,
        "2021-new.md",
        &doc("New", "2021-01-01", "en", &["A new machine."]),
    );
    let (reg, _) = fixture_registry(&w);
    Engine::new(reg, &dir)
        .index_build(&Default::default())
        .unwrap();
    let reply = e
        .search(&SearchQuery {
            query: "machine".into(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(reply.body.mode, None);
    assert_eq!(reply.notes.len(), 1, "{:?}", reply.notes);
    assert!(reply.notes[0].contains("index build --semantic"));
}

fn fixture_registry(w: &Path) -> (Registry, ()) {
    let reg = Registry::parse(&format!(
        "[[corpus]]\nid = \"writing\"\nname = \"W\"\nkind = \"markdown-folder\"\npath = \"{}\"\n",
        w.display()
    ))
    .unwrap();
    (reg, ())
}

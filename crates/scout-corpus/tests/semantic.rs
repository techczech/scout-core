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

// Scores are deliberately far apart: every zeta document is a better
// semantic match than alpha's best, despite alpha sorting first by name.
fn cross_corpus_candidates() -> Vec<(scout_corpus::SearchResults, scout_corpus::SearchResults)> {
    let tmp = tempfile::tempdir().unwrap();
    let (e, _) = engine(tmp.path());
    let template = search(&e, "machine", SearchMode::Semantic);
    [
        ("alpha", vec![("weak.md", 0.1)]),
        ("zeta", vec![("strong.md", 0.9), ("medium.md", 0.6)]),
    ]
    .into_iter()
    .map(|(corpus, docs)| {
        let mut sem = template.clone();
        sem.corpora = vec![corpus.into()];
        sem.results = docs
            .into_iter()
            .map(|(path, score)| {
                let mut d = template.results[0].clone();
                d.corpus = corpus.into();
                d.rel_path = path.into();
                d.score = score;
                d.rank = score / if corpus == "alpha" { 0.1 } else { 0.9 };
                d.hits.truncate(1);
                d.hits[0].passage_id = format!("{corpus}:{path}:1");
                d.hits[0].score = score;
                d.hits[0].semantic_score = Some(score);
                d
            })
            .collect();
        sem.total_documents = sem.results.len();
        sem.total_passages = sem.results.len();
        let mut fts = sem.clone();
        fts.results.clear();
        fts.total_documents = 0;
        fts.total_passages = 0;
        fts.mode = None;
        (fts, sem)
    })
    .collect()
}

#[test]
fn cross_corpus_hybrid_merge_ranks_by_match_strength() {
    let r =
        scout_corpus::semantic::merge_hybrid("machine", cross_corpus_candidates(), 20, 3).unwrap();
    assert_eq!(paths(&r), ["strong.md", "medium.md", "weak.md"]);
    assert!(r.results.windows(2).all(|w| w[0].score > w[1].score));
    assert_eq!(r.results[0].rank, 1.0);
    assert!(r.results[1].rank < 1.0);
}

#[test]
fn cross_corpus_limit_does_not_reserve_a_place_for_each_corpus() {
    let parts = cross_corpus_candidates();
    // Reproduce the previous merge so this test explicitly guards its bias.
    let old = scout_corpus::search::merge(
        "machine",
        parts
            .clone()
            .into_iter()
            .map(|(f, s)| scout_corpus::semantic::fuse(f, s, 20, 3))
            .collect(),
        2,
    )
    .unwrap();
    assert_eq!(paths(&old), ["weak.md", "strong.md"]);
    let new = scout_corpus::semantic::merge_hybrid("machine", parts, 2, 3).unwrap();
    assert_eq!(paths(&new), ["strong.md", "medium.md"]);
}

#[test]
fn cross_corpus_word_votes_use_local_ranks_not_bm25_scales() {
    let mut parts = cross_corpus_candidates();
    parts[0].0 = parts[0].1.clone();
    parts[0].0.results[0].score = 30.0;
    // This extra alpha word match pushes strong to fourth place in pooled
    // BM25, making weak strictly beat strong there rather than merely tie.
    let mut word_only = parts[0].0.results[0].clone();
    word_only.rel_path = "word-only.md".into();
    word_only.hits[0].passage_id = "alpha:word-only.md:1".into();
    word_only.score = 9.5;
    parts[0].0.results.push(word_only);
    parts[0].0.total_documents = 2;
    parts[0].0.total_passages = 2;
    parts[1].0 = parts[1].1.clone();
    parts[1].0.results.reverse();
    parts[1].0.results[0].score = 10.0; // medium: first local word match
    parts[1].0.results[1].score = 9.0; // strong: second local word match
    let merged = scout_corpus::semantic::merge_hybrid("machine", parts.clone(), 20, 3).unwrap();
    assert_eq!(
        paths(&merged),
        ["medium.md", "strong.md", "weak.md", "word-only.md"]
    );
    assert!(merged.results[1].score > merged.results[2].score);
    let top = scout_corpus::semantic::merge_hybrid("machine", parts.clone(), 2, 3).unwrap();
    assert!(top.results.iter().all(|d| d.corpus == "zeta"));

    // Scale alpha DOWN tenfold: pooled BM25 would move weak from first
    // to third, flipping its order against strong. Local votes stay identical.
    for d in &mut parts[0].0.results {
        d.score /= 10.0;
    }
    let scaled = scout_corpus::semantic::merge_hybrid("machine", parts.clone(), 20, 3).unwrap();
    assert_eq!(paths(&scaled), paths(&merged));
    assert_eq!(
        merged.results.iter().map(|d| d.score).collect::<Vec<_>>(),
        scaled.results.iter().map(|d| d.score).collect::<Vec<_>>()
    );

    // Equal BM25 scores share a local rank: both zeta documents get 1/61.
    parts[1].0.results[1].score = 10.0;
    let tied = scout_corpus::semantic::merge_hybrid("machine", parts, 20, 3).unwrap();
    assert_eq!(
        paths(&tied),
        ["strong.md", "medium.md", "weak.md", "word-only.md"]
    );
    assert_eq!(tied.results[0].score, 0.032787);
}

#[test]
fn cross_corpus_hybrid_keeps_same_path_documents_distinct() {
    let mut parts = cross_corpus_candidates();
    parts[0].1.results[0].rel_path = "strong.md".into();
    parts[0].1.results[0].hits[0].passage_id = "alpha:strong.md:1".into();
    // Both modalities vote for zeta's strong document, not alpha's namesake.
    parts[1].0 = parts[1].1.clone();
    parts[1].0.results.truncate(1);
    parts[1].0.results[0].score = 10.0;
    parts[1].0.total_documents = 1;
    let r = scout_corpus::semantic::merge_hybrid("machine", parts, 20, 3).unwrap();
    assert_eq!(r.results.len(), 3);
    assert_eq!(r.total_documents, 3);
    assert_eq!(r.results[0].corpus, "zeta");
    assert!(r.results[0].score > 1.0 / 61.0);
    assert_eq!(r.results[2].corpus, "alpha");
    assert_eq!(r.results[2].hits[0].semantic_score, Some(0.1));
}

#[test]
fn engine_cross_corpus_search_uses_global_semantic_evidence() {
    let tmp = tempfile::tempdir().unwrap();
    let alpha = tmp.path().join("alpha");
    let zeta = tmp.path().join("zeta");
    write(
        &alpha,
        "weak.md",
        "Tomato compost in the garden grows all summer.",
    );
    write(
        &zeta,
        "strong.md",
        "The mind and brain carry every thought through life.",
    );
    write(
        &zeta,
        "medium.md",
        "The mind enjoys tomato compost in the garden.",
    );
    let reg = Registry::parse(&format!(
        "[[corpus]]\nid = \"alpha\"\nname = \"A\"\nkind = \"markdown-folder\"\npath = \"{}\"\n\n[[corpus]]\nid = \"zeta\"\nname = \"Z\"\nkind = \"markdown-folder\"\npath = \"{}\"\n",
        alpha.display(), zeta.display()
    )).unwrap();
    let e = Engine::new(reg, tmp.path().join("indexes")).with_embedder(Arc::new(Concepts));
    e.index_build(&IndexBuildQuery {
        semantic: true,
        ..Default::default()
    })
    .unwrap();
    // Czech for mind: no lexical matches, so cosine must decide the order.
    for mode in [SearchMode::Hybrid, SearchMode::Auto, SearchMode::Semantic] {
        let r = search(&e, "mysl", mode);
        assert_eq!(paths(&r), ["strong.md", "medium.md", "weak.md"]);
        assert!(r.results[1].rank > r.results[2].rank);
    }
}

#[test]
fn cross_corpus_equal_evidence_has_equal_votes_and_stable_ties() {
    let mut parts = cross_corpus_candidates();
    parts[0].1.results[0].score = 0.9;
    parts[0].1.results[0].hits[0].semantic_score = Some(0.9);
    let a = scout_corpus::semantic::merge_hybrid("machine", parts.clone(), 20, 3).unwrap();
    parts.reverse();
    let b = scout_corpus::semantic::merge_hybrid("machine", parts, 20, 3).unwrap();
    assert_eq!(a.results[0].corpus, "alpha");
    assert_eq!(a.results[1].corpus, "zeta");
    assert_eq!(a.results[0].score, a.results[1].score);
    assert_eq!(
        serde_json::to_string(&a.results).unwrap(),
        serde_json::to_string(&b.results).unwrap()
    );
}

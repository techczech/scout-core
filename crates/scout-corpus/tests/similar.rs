//! Seam tests for `Engine::similar` ("more like these"): ranking by tf-idf
//! cosine to the seeds' centroid, the "why" (shared terms, closest seed),
//! seed exclusion, tie order and byte-identical output.

use scout_corpus::api::{self, SimilarQuery};
use scout_corpus::{Engine, PassageNotFound, Registry};
use std::fs;
use std::path::Path;

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

fn doc(title: &str, date: &str, paras: &[&str]) -> String {
    format!(
        "---\ntitle: {title}\ndate: {date}\ngenre: essay\n---\n{}\n",
        paras.join("\n\n")
    )
}

/// Two corpora: metaphor passages, gardening passages, and one passage
/// that appears word for word in two documents (a tie).
fn engine(tmp: &Path) -> Engine {
    let (w, n) = (tmp.join("writing"), tmp.join("notes"));
    write(
        &w,
        "2016-seed.md",
        &doc(
            "Repaved paths",
            "2016-06-23",
            &[
                "New analogies inspire a scientist; Schon called these generative metaphors.",
                "Tomatoes need sun, compost and regular watering in the garden.",
            ],
        ),
    );
    write(
        &w,
        "2013-metaphor.md",
        &doc(
            "Metaphor hacking",
            "2013-10-27",
            &[
                "Generative metaphors and analogies reveal new connections in thought.",
                "The committee met on Tuesday to approve the budget.",
            ],
        ),
    );
    write(
        &w,
        "2020-garden.md",
        &doc(
            "Garden diary",
            "2020-05-01",
            &["Compost the beds before the tomatoes go in; watering daily in the sun."],
        ),
    );
    write(
        &n,
        "a.md",
        &doc(
            "Note A",
            "2019",
            &["Conceptual metaphors frame how analogies work."],
        ),
    );
    write(
        &n,
        "b.md",
        &doc(
            "Note B",
            "2019",
            &["Conceptual metaphors frame how analogies work."],
        ),
    );
    let reg = Registry::parse(&format!(
        "[[corpus]]\nid = \"writing\"\nname = \"W\"\nkind = \"markdown-folder\"\npath = \"{}\"\n\n\
         [[corpus]]\nid = \"notes\"\nname = \"N\"\nkind = \"markdown-folder\"\npath = \"{}\"\n",
        w.display(),
        n.display()
    ))
    .unwrap();
    let e = Engine::new(reg, tmp.join("indexes"));
    e.index_build(&Default::default()).unwrap();
    e
}

const SEED: &str = "writing:2016-seed.md:6";
const GARDEN_SEED: &str = "writing:2016-seed.md:8";

fn q(seeds: &[&str]) -> SimilarQuery {
    SimilarQuery {
        seeds: seeds.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    }
}

#[test]
fn metaphor_seed_ranks_metaphor_passages_first_with_shared_terms() {
    let tmp = tempfile::tempdir().unwrap();
    let e = engine(tmp.path());
    let r = e.similar(&q(&[SEED])).unwrap().body;
    assert_eq!(r.seeds.len(), 1);
    assert_eq!(r.seeds[0].title, "Repaved paths");
    assert_eq!(r.corpora, vec!["writing", "notes"]);
    // The seed itself comes first (cosine 1) when not excluded.
    assert_eq!(r.results[0].passage_id, SEED);
    assert_eq!(r.results[0].score, 1.0);
    let ids: Vec<&str> = r.results.iter().map(|s| s.passage_id.as_str()).collect();
    assert_eq!(
        &ids[1..4],
        &["writing:2013-metaphor.md:6", "notes:a.md:6", "notes:b.md:6"],
        "{ids:?}"
    );
    // The gardening and budget passages share no two terms with the seed.
    assert!(!ids
        .iter()
        .any(|i| i.contains("garden") || i.ends_with(":8")));
    let m = &r.results[1];
    assert_eq!(m.shared_terms.len(), 3);
    assert!(m.shared_terms.contains(&"generative".to_string()));
    assert_eq!(m.closest_seed, SEED);
    assert_eq!(m.title, "Metaphor hacking");
    assert!(m.quote.starts_with("Generative metaphors"));
    assert!(m.score > 0.0 && m.score < 1.0);
    for s in &r.results {
        assert!(!s.shared_terms.is_empty() && s.shared_terms.len() <= 3);
        assert!(s.shared_terms.iter().all(|t| t != "and" && t != "the"));
    }
}

#[test]
fn exclude_seeds_ties_and_byte_identical_output() {
    let tmp = tempfile::tempdir().unwrap();
    let e = engine(tmp.path());
    let mut query = q(&[SEED]);
    query.exclude_seeds = true;
    let a = e.similar(&query).unwrap().body;
    assert!(a.results.iter().all(|s| s.passage_id != SEED));
    // Identical text in two documents: equal scores, ordered by passage id.
    let tied: Vec<_> = a.results.iter().filter(|s| s.corpus == "notes").collect();
    assert_eq!(tied.len(), 2);
    assert_eq!(tied[0].score, tied[1].score);
    assert!(tied[0].passage_id < tied[1].passage_id);
    let b = e.similar(&query).unwrap().body;
    assert_eq!(api::to_json(&a).unwrap(), api::to_json(&b).unwrap());
    // `top` truncates after ranking; total_passages counts every candidate.
    query.top = 1;
    let one = e.similar(&query).unwrap().body;
    assert_eq!(one.results.len(), 1);
    assert_eq!(one.results[0].passage_id, a.results[0].passage_id);
    assert_eq!(one.total_passages, a.total_passages);
}

#[test]
fn two_seeds_each_suggestion_names_its_closest_seed() {
    let tmp = tempfile::tempdir().unwrap();
    let e = engine(tmp.path());
    let mut query = q(&[SEED, GARDEN_SEED]);
    query.exclude_seeds = true;
    let r = e.similar(&query).unwrap().body;
    let by_id = |id: &str| r.results.iter().find(|s| s.passage_id == id).unwrap();
    assert_eq!(by_id("writing:2020-garden.md:6").closest_seed, GARDEN_SEED);
    assert_eq!(by_id("writing:2013-metaphor.md:6").closest_seed, SEED);
    assert!(by_id("writing:2020-garden.md:6")
        .shared_terms
        .contains(&"tomatoes".to_string()));
}

#[test]
fn scope_seed_limits_and_missing_passages() {
    let tmp = tempfile::tempdir().unwrap();
    let e = engine(tmp.path());
    // A seed outside --in still seeds; only the named corpora are searched.
    let mut query = q(&[SEED]);
    query.in_ = vec!["notes".into()];
    let r = e.similar(&query).unwrap().body;
    assert_eq!(r.corpora, vec!["notes"]);
    assert!(r.results.iter().all(|s| s.corpus == "notes"));
    assert_eq!(r.results.len(), 2);

    assert!(e.similar(&q(&[])).is_err());
    let eleven: Vec<String> = (0..11).map(|i| format!("writing:x.md:{i}")).collect();
    let err = e
        .similar(&SimilarQuery {
            seeds: eleven,
            ..Default::default()
        })
        .unwrap_err();
    assert!(err.to_string().contains("at most 10"), "{err}");
    let err = e.similar(&q(&["writing:2016-seed.md:99"])).unwrap_err();
    assert!(err.downcast_ref::<PassageNotFound>().is_some());

    let d: SimilarQuery = serde_json::from_str(r#"{"seeds": ["writing:a.md:1"]}"#).unwrap();
    assert_eq!((d.top, d.exclude_seeds, d.in_.len()), (20, false, 0));
}

//! Multi-corpus passage identity (invariant 2 across corpora).
//!
//! Every corpus has its own index, so SQLite row ids restart at 1 in each:
//! the same row id names different passages in different corpora. The
//! fixture is DESIGNED so they collide: `alpha` and `beta` hold the same file
//! names with the same passage layout (so passage N has row id N in both),
//! but different words and lengths, and dates that interleave the two
//! corpora under `date` sort. `c.md` is one passage in each (row 5 in
//! both, adjacent under `date`): a one-token passage in `alpha`, the node
//! deep in a long one in `beta`, which made the stale cache index past the
//! end of `alpha`'s spans (the panic). Every quote the facade returns must be a
//! substring of its OWN passage's original text.

use scout_corpus::api::*;
use scout_corpus::{Engine, Registry};
use std::collections::BTreeMap;
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

/// `alpha`: short passages. `beta`: the same layout, long passages with the
/// node far from the start (a stale `alpha` cache indexes past its end).
fn engine(tmp: &Path) -> Engine {
    let (a, b) = (tmp.join("alpha"), tmp.join("beta"));
    write(
        &a,
        "a.md",
        &doc(
            "Alpha A",
            "2010-01-01",
            &["Metaphor.", "A metaphor of alpha."],
        ),
    );
    write(
        &a,
        "b.md",
        &doc(
            "Alpha B",
            "2012-01-01",
            &["The alpha metaphor is one thing here.", "Plain metaphor."],
        ),
    );
    write(
        &b,
        "a.md",
        &doc(
            "Beta A",
            "2011-01-01",
            &[
                "In beta a long run of different words comes first before the metaphor arrives late.",
                "Beta keeps on talking for a good while and only then says metaphor again.",
            ],
        ),
    );
    write(
        &b,
        "b.md",
        &doc(
            "Beta B",
            "2013-01-01",
            &[
                "The beta metaphor is another matter there.",
                "Quite a different beta sentence with the metaphor near its end.",
            ],
        ),
    );
    write(&a, "c.md", &doc("Alpha C", "2014-01-01", &["Metaphor."]));
    write(
        &b,
        "c.md",
        &doc(
            "Beta C",
            "2015-01-01",
            &["Beta rambles on and on through one two three four five six words to metaphor."],
        ),
    );
    let entry = |id: &str, p: &Path| {
        format!(
            "[[corpus]]\nid = \"{id}\"\nname = \"{id}\"\nkind = \"markdown-folder\"\npath = \"{}\"\n",
            p.display()
        )
    };
    let reg = Registry::parse(&format!("{}\n{}", entry("alpha", &a), entry("beta", &b))).unwrap();
    let e = Engine::new(reg, tmp.join("indexes"));
    e.index_build(&IndexBuildQuery::default()).unwrap();
    e
}

fn scope() -> Scope {
    Scope {
        in_: vec!["alpha".into(), "beta".into()],
        ..Default::default()
    }
}

/// The original text of a passage, by id, cached.
struct Originals<'a> {
    e: &'a Engine,
    cache: BTreeMap<String, String>,
}

impl Originals<'_> {
    fn quote(&mut self, pid: &str) -> &str {
        if !self.cache.contains_key(pid) {
            let q = self
                .e
                .cite(&CiteQuery {
                    passage_id: pid.into(),
                })
                .unwrap()
                .body
                .quote;
            self.cache.insert(pid.to_string(), q);
        }
        &self.cache[pid]
    }

    /// `text` is original text of `pid` itself.
    fn assert_own(&mut self, pid: &str, text: &str, what: &str) {
        let own = self.quote(pid).to_string();
        assert!(
            own.contains(text.trim()),
            "{what}: {text:?} is not in its own passage {pid} ({own:?})"
        );
    }
}

const SORTS: [&str; 8] = ["date", "R1", "R2", "L1", "L2", "L9", "R9", "source"];

#[test]
fn row_ids_really_collide_across_the_two_corpora() {
    // The fixture's premise: both corpora hit on passages with the same
    // line layout, so the same row ids.
    let tmp = tempfile::tempdir().unwrap();
    let e = engine(tmp.path());
    let k = e
        .kwic(&KwicQuery {
            term: "metaphor".into(),
            scope: scope(),
            sort: "source".into(),
            ..Default::default()
        })
        .unwrap()
        .body;
    let lines = |c: &str| -> Vec<String> {
        k.lines
            .iter()
            .filter(|l| l.corpus == c)
            .map(|l| l.passage_id.split_once(':').unwrap().1.to_string())
            .collect()
    };
    assert_eq!(lines("alpha"), lines("beta"));
    assert_eq!(k.total, 10);
}

#[test]
fn every_kwic_line_quotes_its_own_passage_in_every_sort() {
    let tmp = tempfile::tempdir().unwrap();
    let e = engine(tmp.path());
    let mut o = Originals {
        e: &e,
        cache: BTreeMap::new(),
    };
    for sort in SORTS {
        for width in [0, 3, 8] {
            let k = e
                .kwic(&KwicQuery {
                    term: "metaphor".into(),
                    scope: scope(),
                    sort: sort.into(),
                    width,
                    ..Default::default()
                })
                .unwrap()
                .body;
            assert_eq!(k.returned, 10, "sort {sort}");
            for l in &k.lines {
                assert_eq!(l.node.to_lowercase(), "metaphor", "sort {sort}");
                let s = format!("{}{}{}", l.left, l.node, l.right);
                o.assert_own(
                    &l.passage_id,
                    &s,
                    &format!("kwic sort {sort} width {width}"),
                );
            }
        }
    }
}

#[test]
fn ngram_rows_open_concordances_without_panic_or_wrong_quotes() {
    // The apps' path: an n-gram row (across both corpora) opens its
    // concordance. Every gram with the word, every sort.
    let tmp = tempfile::tempdir().unwrap();
    let e = engine(tmp.path());
    let mut o = Originals {
        e: &e,
        cache: BTreeMap::new(),
    };
    let g = e
        .ngrams(&NgramsQuery {
            scope: scope(),
            n: "1-3".into(),
            top: 200,
            containing: Some("metaphor".into()),
            ..Default::default()
        })
        .unwrap()
        .body;
    assert!(!g.grams.is_empty());
    for gram in &g.grams {
        for sort in SORTS {
            let k = e
                .kwic(&KwicQuery {
                    term: gram.gram.clone(),
                    scope: scope(),
                    sort: sort.into(),
                    ..Default::default()
                })
                .unwrap()
                .body;
            assert_eq!(k.total as u64, gram.count, "{:?} sort {sort}", gram.gram);
            for l in &k.lines {
                let s = format!("{}{}{}", l.left, l.node, l.right);
                o.assert_own(&l.passage_id, &s, &format!("{:?} sort {sort}", gram.gram));
            }
        }
    }
}

#[test]
fn ngrams_containing_keeps_only_grams_with_the_word() {
    let tmp = tempfile::tempdir().unwrap();
    let e = engine(tmp.path());
    let q = |containing: Option<&str>| NgramsQuery {
        scope: scope(),
        n: "1-3".into(),
        top: 500,
        containing: containing.map(str::to_string),
        ..Default::default()
    };
    let all = e.ngrams(&q(None)).unwrap().body;
    let with = e.ngrams(&q(Some("Metaphor"))).unwrap().body;
    assert_eq!(with.containing, Some(vec!["metaphor".to_string()]));
    let want: Vec<_> = all
        .grams
        .iter()
        .filter(|g| g.gram.split(' ').any(|w| w == "metaphor"))
        .map(|g| (g.gram.clone(), g.count))
        .collect();
    let got: Vec<_> = with
        .grams
        .iter()
        .map(|g| (g.gram.clone(), g.count))
        .collect();
    assert!(!got.is_empty());
    assert_eq!(got, want);
    // n=1 count agrees with the KWIC total (invariant 3).
    let one = with.grams.iter().find(|g| g.gram == "metaphor").unwrap();
    assert_eq!(one.count, 10);
    // A phrase: grams containing the token sequence.
    let phrase = e.ngrams(&q(Some("the alpha metaphor"))).unwrap().body;
    assert_eq!(
        phrase
            .grams
            .iter()
            .map(|g| g.gram.as_str())
            .collect::<Vec<_>>(),
        vec!["the alpha metaphor"]
    );
    // A word that is nowhere: no grams, no error.
    assert!(e.ngrams(&q(Some("zebra"))).unwrap().body.grams.is_empty());
    // No word tokens at all is a usage error.
    assert!(e.ngrams(&q(Some("  ;; "))).is_err());
    // Partial JSON from an app.
    let j: NgramsQuery = serde_json::from_str(r#"{"containing": "metaphor"}"#).unwrap();
    assert_eq!(j.containing.as_deref(), Some("metaphor"));
    assert_eq!(
        serde_json::from_str::<NgramsQuery>("{}")
            .unwrap()
            .containing,
        None
    );
}

#[test]
fn search_similar_cite_and_verify_quote_their_own_passage() {
    let tmp = tempfile::tempdir().unwrap();
    let e = engine(tmp.path());
    let mut o = Originals {
        e: &e,
        cache: BTreeMap::new(),
    };
    for passage in [false, true] {
        let r = e
            .search(&SearchQuery {
                query: "metaphor".into(),
                in_: scope().in_,
                limit: 50,
                passage,
                ..Default::default()
            })
            .unwrap()
            .body;
        assert_eq!(r.results.len(), 6);
        for d in &r.results {
            for h in &d.hits {
                assert!(h.passage_id.starts_with(&format!("{}:", d.corpus)));
                o.assert_own(&h.passage_id, &h.quote, "search");
            }
        }
    }
    let seeds = ["alpha:b.md:6", "beta:a.md:6"];
    for seed in seeds {
        let s = e
            .similar(&SimilarQuery {
                seeds: vec![seed.into()],
                in_: scope().in_,
                top: 50,
                ..Default::default()
            })
            .unwrap()
            .body;
        assert!(!s.results.is_empty());
        for r in &s.results {
            o.assert_own(&r.passage_id, &r.quote, "similar");
        }
    }
    // cite: the quote is the passage's own source text.
    for (corpus, rel, lines) in [
        ("alpha", "a.md", &[6, 8][..]),
        ("alpha", "b.md", &[6, 8]),
        ("alpha", "c.md", &[6]),
        ("beta", "a.md", &[6, 8]),
        ("beta", "b.md", &[6, 8]),
        ("beta", "c.md", &[6]),
    ] {
        let src = fs::read_to_string(tmp.path().join(corpus).join(rel)).unwrap();
        for &line in lines {
            let pid = format!("{corpus}:{rel}:{line}");
            let q = o.quote(&pid).to_string();
            assert!(src.contains(&q), "cite {pid}: {q:?}");
        }
    }
    // verify-quote: a beta-only sentence is found in beta, never alpha.
    let v = e
        .verify_quote(&VerifyQuoteQuery {
            text: "The beta metaphor is another matter there.".into(),
            in_: scope().in_,
        })
        .unwrap()
        .body;
    assert_eq!(v.matches.len(), 1);
    assert_eq!(v.matches[0].corpus, "beta");
    o.assert_own(&v.matches[0].passage_id, &v.matches[0].original, "verify");
}

#[test]
fn collocates_near_dist_and_profile_stay_per_corpus() {
    let tmp = tempfile::tempdir().unwrap();
    let e = engine(tmp.path());
    let mut o = Originals {
        e: &e,
        cache: BTreeMap::new(),
    };
    // kwic --near: only lines with the collocate, each its own text.
    for near in ["alpha", "beta"] {
        let k = e
            .kwic(&KwicQuery {
                term: "metaphor".into(),
                scope: scope(),
                sort: "date".into(),
                near: Some(near.into()),
                window: 5,
                ..Default::default()
            })
            .unwrap()
            .body;
        assert!(k.total > 0);
        for l in &k.lines {
            assert_eq!(l.corpus, near, "{near}: {:?}", l.passage_id);
            let s = format!("{}{}{}", l.left, l.node, l.right);
            o.assert_own(&l.passage_id, &s, "kwic --near");
        }
    }
    let d = e
        .dist(&DistQuery {
            term: "metaphor".into(),
            by: "corpus".into(),
            scope: scope(),
        })
        .unwrap()
        .body;
    let by: BTreeMap<_, _> = d
        .buckets
        .iter()
        .map(|b| (b.key.as_str(), (b.hits, b.pieces)))
        .collect();
    assert_eq!(by["alpha"], (5, 3));
    assert_eq!(by["beta"], (5, 3));
    let g = e
        .ngrams(&NgramsQuery {
            scope: scope(),
            n: "1".into(),
            containing: Some("metaphor".into()),
            dist: Some("doc".into()),
            ..Default::default()
        })
        .unwrap()
        .body;
    let row = &g.grams[0];
    let keys: Vec<(&str, u64)> = row
        .dist
        .as_ref()
        .unwrap()
        .buckets
        .iter()
        .map(|b| (b.key.as_str(), b.count))
        .collect();
    assert_eq!(
        keys,
        vec![
            ("alpha:a.md", 2),
            ("alpha:b.md", 2),
            ("beta:a.md", 2),
            ("beta:b.md", 2),
            ("alpha:c.md", 1),
            ("beta:c.md", 1),
        ]
    );
    let p = e
        .profile(&ProfileQuery {
            word: "metaphor".into(),
            scope: scope(),
            min_hits: 1,
        })
        .unwrap()
        .body;
    let docs: Vec<(String, String)> = p
        .top_documents
        .iter()
        .map(|d| (d.corpus.clone(), d.rel_path.clone()))
        .collect();
    assert_eq!(docs.len(), 6);
    for (c, rel) in &docs {
        let title = &p
            .top_documents
            .iter()
            .find(|d| &d.corpus == c && &d.rel_path == rel)
            .unwrap()
            .title;
        assert!(
            title.to_lowercase().starts_with(c.as_str()),
            "{c}:{rel} titled {title}"
        );
    }
    let first = p.first_used.unwrap();
    assert_eq!(
        (first.corpus.as_str(), first.title.as_str()),
        ("alpha", "Alpha A")
    );
}

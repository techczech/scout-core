//! Seam tests for T4: collocates (logDice, MI, compare), per-row
//! distributions, keyness, the completed profile, KWIC edge punctuation and
//! citations with a public URL. The key check is invariant 3 for collocates:
//! each row's count equals the lines of its concordance link.

use scout_corpus::colloc::{self, log_dice, mutual_information, CollocRequest, Score};
use scout_corpus::concord::{KwicRequest, Near};
use scout_corpus::keyness::{self, log_likelihood, KeynessRequest};
use scout_corpus::ngrams::NgramRequest;
use scout_corpus::registry::{CorpusConfig, CorpusKind, FieldMap, WRITEFLEX_LINK};
use scout_corpus::rowdist::RowDistBy;
use scout_corpus::{Corpus, DocFilter, ProfileOptions, SearchRequest, Slice, SliceSpec};
use std::fs;
use std::path::Path;

fn cfg(root: &Path, id: &str) -> CorpusConfig {
    CorpusConfig {
        id: id.into(),
        name: id.into(),
        kind: CorpusKind::MarkdownFolder,
        path: root.display().to_string(),
        include: vec!["**/*.md".into()],
        exclude: vec![],
        require_frontmatter: vec!["genre".into()],
        field_map: FieldMap::default(),
        default_author: Some("Dominik Lukeš".into()),
        boilerplate: vec![],
        link: Some(WRITEFLEX_LINK.into()),
    }
}

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

fn doc(date: &str, genre: &str, body: &str) -> String {
    format!("---\ntitle: T {date}\ndate: {date}\ngenre: {genre}\n---\n{body}\n")
}

fn built(src: &Path, data: &Path, id: &str) -> Corpus {
    let c = Corpus::from_config(cfg(src, id), data);
    c.build_index(false).unwrap();
    c
}

/// A small corpus with repeated vocabulary across dates and genres.
fn prose(root: &Path) {
    write(
        root,
        "a/2004.md",
        &doc(
            "2004",
            "essay",
            "The generative metaphor of scaffolding frames teaching. A conceptual metaphor frames thought; the metaphor of scaffolding is a conceptual frame.\n\nScaffolding is a metaphor for support, and support is not scaffolding.",
        ),
    );
    write(
        root,
        "a/2005.md",
        &doc(
            "2005-03",
            "essay",
            "Conceptual metaphor theory treats metaphor as thought. Generative metaphor, conceptual metaphor, dead metaphor: each metaphor frames a problem.\n\n- a list item with metaphor and frame and theory",
        ),
    );
    write(
        root,
        "b/2020.md",
        &doc(
            "2020-06-01",
            "note",
            "Models use metaphor loosely. A model is a metaphor that breaks; every metaphor breaks somewhere, and models break too.\n\nThe metaphor of the model: models frame what we see, a metaphor frames it too.",
        ),
    );
    write(
        root,
        "b/2022.md",
        &doc(
            "2022-01-02",
            "note",
            "Metaphor breaks when stretched. The frame metaphor and the scaffolding metaphor both break; conceptual metaphor theory explains why the metaphor breaks.",
        ),
    );
}

fn kwic_near_total(c: &[Corpus], node: &str, near: &str, window: usize, f: &DocFilter) -> usize {
    let mut k = KwicRequest::new(node);
    k.filter = f.clone();
    k.limit = 0;
    k.near = Some(Near {
        word: near.into(),
        window,
    });
    scout_corpus::concord::kwic(c, &k).unwrap().total
}

#[test]
fn logdice_and_mi_match_hand_computed_values() {
    let src = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    // Passage A: kilo lima kilo mike; passage B: lima november lima kilo.
    write(
        src.path(),
        "x.md",
        &doc(
            "2010",
            "note",
            "kilo lima kilo mike\n\nlima november lima kilo",
        ),
    );
    let c = built(src.path(), data.path(), "t");
    let mut req = CollocRequest::new("kilo");
    req.window = 1;
    req.min_freq = 1;
    let r = c.collocates(&req).unwrap();
    assert_eq!((r.node_freq, r.total_tokens, r.window_span), (3, 8, 2));
    let row = |w: &str| r.rows.iter().find(|x| x.collocate == w).unwrap().clone();
    // lima: f(n,c) = 3 (every kilo has a lima beside it), f(c) = 3.
    // logDice = 14 + log2(2*3 / (3+3)) = 14; MI = log2(3*8 / (3*3*2)) = 0.415
    let lima = row("lima");
    assert_eq!((lima.count, lima.collocate_freq), (3, 3));
    assert_eq!(lima.logdice, 14.0);
    assert_eq!(lima.mi, 0.415);
    // mike: f(n,c) = 1, f(c) = 1. logDice = 14 + log2(2/4) = 13;
    // MI = log2(1*8 / (3*1*2)) = 0.415
    let mike = row("mike");
    assert_eq!((mike.count, mike.logdice, mike.mi), (1, 13.0, 0.415));
    assert_eq!(r.rows.len(), 2, "november is two tokens away");
    // The formulas themselves.
    assert_eq!(log_dice(3, 3, 3), 14.0);
    assert_eq!(mutual_information(1, 3, 1, 8, 2), 0.415);
    // Order: logDice desc (lima 14 before mike 13); MI ties break by count.
    assert_eq!(r.rows[0].collocate, "lima");
    req.score = Score::Mi;
    let r = c.collocates(&req).unwrap();
    assert_eq!(r.rows[0].collocate, "lima", "equal MI: higher count first");

    // The window stays inside the passage: at ±5, mike is near the two
    // passage-A kilos only, never the passage-B one.
    req.window = 5;
    let r = c.collocates(&req).unwrap();
    let m = r.rows.iter().find(|x| x.collocate == "mike").unwrap();
    assert_eq!(m.count, 2);
    let n = r.rows.iter().find(|x| x.collocate == "november").unwrap();
    assert_eq!(n.count, 1);
    // min_freq filters on f(n,c).
    req.min_freq = 3;
    let r = c.collocates(&req).unwrap();
    let names: Vec<&str> = r.rows.iter().map(|x| x.collocate.as_str()).collect();
    assert_eq!(names, vec!["lima"]);
    assert_eq!(r.candidates, 1);
    // Window 0 is a usage error.
    req.window = 0;
    assert!(c.collocates(&req).is_err());
}

#[test]
fn collocate_count_equals_its_concordance_lines() {
    let src = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    prose(src.path());
    let c = built(src.path(), data.path(), "t");
    let both = [c.clone()];
    let filters = [
        DocFilter::default(),
        DocFilter {
            genre: vec!["note".into()],
            ..DocFilter::default()
        },
        DocFilter {
            before: Some("2010".into()),
            ..DocFilter::default()
        },
    ];
    let mut checked = 0;
    for node in [
        "metaphor",
        "scaffolding",
        "conceptual metaphor",
        "the",
        "frame",
    ] {
        for f in &filters {
            for window in [1, 3, 5] {
                for keep in [false, true] {
                    let mut req = CollocRequest::new(node);
                    req.window = window;
                    req.min_freq = 1;
                    req.top = 1000;
                    req.keep_stopwords = keep;
                    req.filter = f.clone();
                    let r = c.collocates(&req).unwrap();
                    assert_eq!(r.candidates, r.rows.len());
                    for row in &r.rows {
                        let lines = kwic_near_total(&both, node, &row.collocate, window, f);
                        assert_eq!(
                            row.count as usize, lines,
                            "{node} ~ {} ±{window} {f:?}",
                            row.collocate
                        );
                        assert_eq!(row.kwic.near.word, row.collocate);
                        assert!(row.kwic.command.contains(&format!("--window {window}")));
                        if !keep {
                            assert!(!scout_corpus::stopwords::is_stopword(&row.collocate));
                        }
                        checked += 1;
                    }
                }
            }
        }
    }
    assert!(checked > 200, "the fixture exercises real rows: {checked}");
    // A multi-word --near is a usage error.
    let mut k = KwicRequest::new("metaphor");
    k.near = Some(Near {
        word: "two words".into(),
        window: 5,
    });
    assert!(c.kwic(&k).is_err());
}

#[test]
fn row_distributions_sum_to_the_row_count() {
    let src = tempfile::tempdir().unwrap();
    let src2 = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    prose(src.path());
    prose(src2.path());
    // An undated document and more documents than DOC_TOP.
    for i in 0..12 {
        write(
            src.path(),
            &format!("c/{i:02}.md"),
            &doc(
                &format!("{}", 2010 + i),
                "note",
                "a conceptual metaphor frames",
            ),
        );
    }
    write(
        src.path(),
        "c/undated.md",
        "---\ntitle: U\ngenre: note\n---\na conceptual metaphor frames\n",
    );
    let a = built(src.path(), data.path(), "one");
    let b = built(src2.path(), data.path(), "two");
    let both = [a, b];
    for by in [RowDistBy::Year, RowDistBy::Doc, RowDistBy::Corpus] {
        let mut req = CollocRequest::new("metaphor");
        req.min_freq = 1;
        req.top = 50;
        req.dist = Some(by);
        let r = colloc::collocates(&both, &req).unwrap();
        assert!(!r.rows.is_empty());
        for row in &r.rows {
            let d = row.dist.as_ref().unwrap();
            assert_eq!(d.total(), row.count, "{by:?} {}", row.collocate);
            match by {
                RowDistBy::Doc => assert!(d.buckets.len() <= scout_corpus::rowdist::DOC_TOP),
                RowDistBy::Corpus => {
                    let keys: Vec<&str> = d.buckets.iter().map(|b| b.key.as_str()).collect();
                    assert_eq!(keys.len(), 2, "every corpus listed: {keys:?}");
                }
                RowDistBy::Year => {}
            }
        }
        let conceptual = r.rows.iter().find(|x| x.collocate == "conceptual").unwrap();
        let d = conceptual.dist.as_ref().unwrap();
        match by {
            RowDistBy::Year => assert_eq!(d.buckets.last().unwrap().key, "unknown"),
            RowDistBy::Doc => {
                assert!(d.other.as_ref().unwrap().documents > 0);
                assert!(d.buckets[0].title.is_some());
            }
            RowDistBy::Corpus => {}
        }

        let n = scout_corpus::ngrams::ngrams(
            &both,
            &NgramRequest {
                n_min: 1,
                n_max: 3,
                top: 200,
                dist: Some(by),
                ..NgramRequest::default()
            },
        )
        .unwrap();
        assert!(!n.grams.is_empty());
        for g in &n.grams {
            assert_eq!(
                g.dist.as_ref().unwrap().total(),
                g.count,
                "{by:?} {}",
                g.gram
            );
        }
    }
    // Without dist, no dist field.
    let json = serde_json::to_value(
        scout_corpus::ngrams::ngrams(&both, &NgramRequest::default()).unwrap(),
    )
    .unwrap();
    assert!(json["grams"][0].get("dist").is_none());
}

#[test]
fn compare_is_symmetric() {
    let src = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    prose(src.path());
    let c = built(src.path(), data.path(), "t");
    let slice = |spec: &str| {
        let s = SliceSpec::parse(spec).unwrap();
        Slice::new(s.spec, vec![c.clone()], s.filter)
    };
    let (a, b) = (slice("before:2010"), slice("after:2020"));
    for score in [Score::LogDice, Score::Mi] {
        let mut req = CollocRequest::new("metaphor");
        req.min_freq = 1;
        req.top = 1000;
        req.score = score;
        let ab = colloc::compare(&a, &b, &req).unwrap();
        let ba = colloc::compare(&b, &a, &req).unwrap();
        assert!(ab.rows.len() > 5);
        assert_eq!(ab.rows.len(), ba.rows.len());
        assert_eq!(
            (ab.a.node_freq, ab.b.node_freq),
            (ba.b.node_freq, ba.a.node_freq)
        );
        for (x, y) in ab.rows.iter().zip(&ba.rows) {
            assert_eq!(x.collocate, y.collocate, "same order both ways");
            assert_eq!(x.a, y.b);
            assert_eq!(x.b, y.a);
            assert_eq!(x.score_diff, y.score_diff.map(|d| -d));
            match (x.ratio, y.ratio) {
                (Some(p), Some(q)) => assert!((p * q - 1.0).abs() < 0.01, "{p} {q}"),
                (Some(p), None) => assert_eq!(p, 0.0),
                (None, Some(q)) => assert_eq!(q, 0.0),
                (None, None) => panic!("a row has a count on some side"),
            }
        }
        // Each side agrees with a plain collocates run over that slice.
        let mut single = req.clone();
        single.filter = a.filter.clone();
        let one = c.collocates(&single).unwrap();
        for row in &one.rows {
            let r = ab
                .rows
                .iter()
                .find(|r| r.collocate == row.collocate)
                .unwrap();
            assert_eq!(r.a.count, row.count);
            assert_eq!(r.a.logdice, Some(row.logdice));
            assert_eq!(r.a.mi, Some(row.mi));
        }
        // And the side counts equal their concordance lines.
        for r in ab.rows.iter().take(10) {
            assert_eq!(
                r.b.count as usize,
                kwic_near_total(
                    std::slice::from_ref(&c),
                    "metaphor",
                    &r.collocate,
                    5,
                    &b.filter
                )
            );
        }
    }
}

#[test]
fn keyness_matches_a_hand_computed_table() {
    let src = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    // A: zeta x10 in 1000 tokens; B: zeta x5 in 2000 tokens.
    let a_body = format!("{}{}", "zeta ".repeat(10), "fa ".repeat(990));
    let b_body = format!("{}{}", "zeta ".repeat(5), "fb ".repeat(1995));
    write(src.path(), "a.md", &doc("2004", "essay", &a_body));
    write(src.path(), "b.md", &doc("2020", "note", &b_body));
    let c = built(src.path(), data.path(), "t");
    let slice = |spec: &str| {
        let s = SliceSpec::parse(spec).unwrap();
        Slice::new(s.spec, vec![c.clone()], s.filter)
    };
    let r = keyness::keyness(
        &slice("genre:essay"),
        &slice("genre:note"),
        &KeynessRequest::default(),
    )
    .unwrap();
    assert_eq!((r.a.tokens, r.b.tokens), (1000, 2000));
    assert_eq!(r.overlap_documents, 0);
    let z = r.a_keys.iter().find(|k| k.word == "zeta").unwrap();
    // G2 = 10 ln 2 = 6.931; %DIFF = 300.
    assert_eq!((z.a, z.b), (10, 5));
    assert_eq!(z.g2, 6.931);
    assert_eq!(z.pct_diff, Some(300.0));
    assert_eq!((z.a_per_million, z.b_per_million), (10000.0, 2500.0));
    assert_eq!(r.a_keys[0].word, "fa", "G2 desc");
    assert_eq!(r.a_keys[0].pct_diff, None, "b = 0");
    assert_eq!(
        r.b_keys.iter().map(|k| k.word.as_str()).collect::<Vec<_>>(),
        vec!["fb"]
    );
    assert!((log_likelihood(10, 5, 1000, 2000) - 6.931_471_8).abs() < 1e-6);
    // Swapping the slices swaps the lists.
    let s = keyness::keyness(
        &slice("genre:note"),
        &slice("genre:essay"),
        &KeynessRequest::default(),
    )
    .unwrap();
    assert_eq!(s.b_keys.len(), r.a_keys.len());
    assert_eq!(
        s.b_keys.iter().find(|k| k.word == "zeta").unwrap().g2,
        6.931
    );
    // min_freq: zeta needs a >= 11 now and drops out.
    let t = keyness::keyness(
        &slice("genre:essay"),
        &slice("genre:note"),
        &KeynessRequest {
            top: 40,
            min_freq: 11,
        },
    )
    .unwrap();
    assert!(!t.a_keys.iter().any(|k| k.word == "zeta"));
    // Overlapping slices are reported.
    let o = keyness::keyness(
        &slice("after:2000"),
        &slice("genre:note"),
        &KeynessRequest::default(),
    )
    .unwrap();
    assert_eq!(o.overlap_documents, 1);
}

#[test]
fn profile_is_complete() {
    let src = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    prose(src.path());
    write(src.path(), "d/one-hit.md", &doc("2019", "note", "metaphor"));
    let c = built(src.path(), data.path(), "t");
    let p = c.profile("metaphor", &DocFilter::default()).unwrap();
    assert_eq!(p.schema_version, 2);
    assert_eq!(p.top_documents_min_hits, 3);
    assert!(p.top_documents.iter().all(|d| d.hits >= 3));
    assert!(!p.top_documents.iter().any(|d| d.rel_path == "d/one-hit.md"));
    let all = c
        .profile_with(
            "metaphor",
            &DocFilter::default(),
            &ProfileOptions {
                min_hits: 1,
                ..ProfileOptions::default()
            },
        )
        .unwrap();
    assert_eq!(
        all.top_documents[0].rel_path, "d/one-hit.md",
        "a 1-token note tops per-million"
    );
    // Collocates: the same rows as `collocates` with top 15.
    let mut req = CollocRequest::new("metaphor");
    req.top = 15;
    req.min_freq = 5;
    let direct = c.collocates(&req).unwrap();
    assert_eq!(p.collocates.status, "ok");
    assert_eq!(p.collocates.items, direct.rows);
    assert!(!p.collocates.items.is_empty());
    // N-grams: 3-5-grams containing the word, top 10.
    assert_eq!(p.ngrams.status, "ok");
    assert!(!p.ngrams.items.is_empty() && p.ngrams.items.len() <= 10);
    for g in &p.ngrams.items {
        assert!(g.gram.split(' ').any(|t| t == "metaphor"), "{}", g.gram);
        assert!((3..=5).contains(&g.n));
    }
    assert_eq!(
        p.frequency,
        c.kwic(&KwicRequest::new("metaphor")).unwrap().total
    );
}

#[test]
fn kwic_context_keeps_edge_punctuation() {
    let src = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    write(
        src.path(),
        "p.md",
        &doc(
            "2020",
            "note",
            "2. Redirection (building scaffolding)\n\n“Scaffolding”, he said.\n\nbuilding a [cognitive scaffolding](https://x.y/z) for readers",
        ),
    );
    let c = built(src.path(), data.path(), "t");
    let get = |width: usize| {
        let mut r = KwicRequest::new("scaffolding");
        r.width = width;
        r.sort = scout_corpus::KwicSort::Source;
        c.kwic(&r).unwrap().lines
    };
    let w3 = get(3);
    let file = fs::read_to_string(src.path().join("p.md")).unwrap();
    for l in &w3 {
        assert!(file.contains(&format!("{}{}{}", l.left, l.node, l.right)));
    }
    assert_eq!(
        (w3[0].left.as_str(), w3[0].right.as_str()),
        ("2. Redirection (building ", ")")
    );
    let w0 = get(0);
    assert_eq!((w0[0].left.as_str(), w0[0].right.as_str()), ("", ")"));
    assert_eq!(
        (
            w0[1].left.as_str(),
            w0[1].node.as_str(),
            w0[1].right.as_str()
        ),
        ("“", "Scaffolding", "”,")
    );
    // A run reaching into letters (a link target) is not taken.
    assert_eq!((w0[2].left.as_str(), w0[2].right.as_str()), ("", ""));
    let w1 = get(1);
    assert_eq!(
        (w1[2].left.as_str(), w1[2].right.as_str()),
        ("[cognitive ", "](https://x.y/z) for")
    );
    assert_eq!(w1[1].right, "”, he");
}

#[test]
fn citation_carries_the_public_url_when_frontmatter_has_one() {
    let src = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    write(
        src.path(),
        "pub.md",
        "---\ntitle: \"Repaved paths and generative metaphors: Expressing human purposes with technology\"\ndate: 2016-06-23\ngenre: essay\npublished_url: https://medium.com/metaphor-hacker/repaved-321b\n---\nSchön called these generative metaphors.\n",
    );
    write(
        src.path(),
        "canon.md",
        "---\ntitle: Canon\ndate: 2016\ngenre: essay\ncanonical_url: https://example.org/canon\n---\nAnother generative metaphors note.\n",
    );
    write(
        src.path(),
        "private.md",
        "---\ntitle: Private\ndate: 2016\ngenre: essay\npublished_url: not a url\n---\nPrivate generative metaphors note.\n",
    );
    let c = built(src.path(), data.path(), "t");
    let res = c
        .search(&SearchRequest::new("generative metaphors"))
        .unwrap();
    let by = |rel: &str| res.results.iter().find(|d| d.rel_path == rel).unwrap();
    let d = by("pub.md");
    let link = d.hits[0].link.clone().unwrap();
    assert_eq!(
        d.public_url.as_deref(),
        Some("https://medium.com/metaphor-hacker/repaved-321b")
    );
    assert!(d.citation.markdown.ends_with(&format!(
        "— Dominik Lukeš, *Repaved paths and generative metaphors: Expressing human purposes with technology*, 23 June 2016 · [archive]({link}) · [public](https://medium.com/metaphor-hacker/repaved-321b)\n"
    )));
    assert!(d.citation.plain.ends_with(&format!(
        "23 June 2016 · archive: {link} · public: https://medium.com/metaphor-hacker/repaved-321b\n"
    )));
    assert_eq!(
        by("canon.md").public_url.as_deref(),
        Some("https://example.org/canon")
    );
    let p = by("private.md");
    assert_eq!(p.public_url, None, "a non-URL value is ignored");
    assert!(!p.citation.markdown.contains("[public]"));
    assert!(p.citation.markdown.contains("· [archive]("));
    // A single-string field map entry parses too.
    let reg = scout_corpus::Registry::parse(
        "[[corpus]]\nid='x'\nname='x'\nkind='markdown-folder'\npath='/tmp'\n[corpus.field_map]\npublic_url='source_url'\n",
    )
    .unwrap();
    assert_eq!(reg.corpora[0].field_map.public_url, vec!["source_url"]);
}

/// Invariant 3 on the real writing index: 20 pseudo-random words with at
/// least 50 hits, their top collocates each checked against `kwic --near`.
/// Needs the user's index; run with `cargo test -- --ignored`.
#[test]
#[ignore]
fn real_writing_collocate_counts_equal_concordance_lines() {
    let c = Corpus::open("writing").expect("writing corpus registered");
    let words = scout_corpus::ngrams::ngrams(
        std::slice::from_ref(&c),
        &NgramRequest {
            n_min: 1,
            n_max: 1,
            top: 5000,
            ..NgramRequest::default()
        },
    )
    .unwrap()
    .grams;
    let pool: Vec<&str> = words
        .iter()
        .filter(|g| g.count >= 50)
        .map(|g| g.gram.as_str())
        .collect();
    let mut seed: u64 = 0x5eed_2026_0927;
    let mut checked = 0;
    for _ in 0..20 {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let w = pool[(seed >> 33) as usize % pool.len()];
        let mut req = CollocRequest::new(w);
        req.top = 5;
        let r = c.collocates(&req).unwrap();
        for row in &r.rows {
            let lines = kwic_near_total(
                std::slice::from_ref(&c),
                w,
                &row.collocate,
                5,
                &DocFilter::default(),
            );
            assert_eq!(row.count as usize, lines, "{w} ~ {}", row.collocate);
            checked += 1;
        }
        eprintln!("{w}: {} rows checked", r.rows.len());
    }
    assert!(checked >= 50, "{checked}");
}

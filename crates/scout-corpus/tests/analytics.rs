//! Seam tests for the analytics views (T3): KWIC, distribution, n-grams,
//! profile and verify_quote. The key check is invariant 3: for the same word
//! and filters, KWIC lines = n-gram count (n=1) = profile frequency = the sum
//! of the distribution buckets.

use scout_corpus::concord::{self, node_tokens, DistBy, KwicRequest, KwicSort};
use scout_corpus::ngrams::{self, parse_n_range, NgramRequest};
use scout_corpus::registry::{CorpusConfig, CorpusKind, FieldMap, WRITEFLEX_LINK};
use scout_corpus::{Corpus, DocFilter};
use std::fs;
use std::path::Path;

const ZPD: &str = "Chaiklin is useful for resisting the common equation of ZPD with any assisted task or with scaffolding in general.";

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
        boilerplate: vec![r"^Originally published at ".into()],
        link: Some(WRITEFLEX_LINK.into()),
    }
}

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

fn fixture(root: &Path) {
    write(
        root,
        "essays/2016-metaphor.md",
        "---\ntitle: Generative metaphor\ndate: 2016-06-23\ngenre: essay\nlang: en\n---\n# On metaphor\n\nA **generative** [metaphor](http://x.org/m) is a metaphor that makes; Schön’s metaphor &amp; more.\nI don’t think a metaphor is only decoration, at the same time.\n\n- a list item: metaphor, metaphor\n\nOriginally published at medium.com with metaphor\n",
    );
    write(
        root,
        "essays/2004-scaffold.md",
        "---\ntitle: Scaffolding\ndate: 2004\ngenre: essay\nlang: en\n---\nScaffolding is a metaphor. Scaffolding helps; don't remove scaffolding too soon.\n\n- **Vygotsky:** The ZPD enters through a narrow criticism. Vygotsky’s point stands. ",
    );
    // The ZPD sentence goes at the end of the list item above.
    let p = root.join("essays/2004-scaffold.md");
    let mut s = fs::read_to_string(&p).unwrap();
    s.push_str(ZPD);
    s.push_str(" Sources follow.\n\nAt the same time, the same time at the same time.\n");
    fs::write(&p, s).unwrap();
    write(
        root,
        "notes/2016-03-note.md",
        "---\ntitle: Note\ndate: 2016-03\ngenre: note\nlang: en\n---\nSalt &amp;amp; pepper &amp;amp;amp; vinegar &amp;amp;amp; oil &amp;amp;amp; metaphor.\n\nAT&amp;amp;T and &lt;b&gt;bold&lt;/b&gt; metaphor at the same time.\n",
    );
    write(
        root,
        "czech/metafora.md",
        "---\ntitle: Metafora\ndate: 2019-01-02\ngenre: essay\nlang: cs\n---\nMetafora je příklad. Metaphor v češtině, don’t.\n",
    );
    write(
        root,
        "undated/u.md",
        "---\ntitle: Undated\ngenre: note\n---\nAn undated metaphor, and scaffolding.\n",
    );
    write(
        root,
        "twins/copy-b.md",
        "---\ntitle: Twin\ndate: 2020-01-01\ngenre: note\n---\nIdentical metaphor passage.\n",
    );
    write(
        root,
        "twins/copy-a.md",
        "---\ntitle: Twin\ndate: 2020-01-01\ngenre: note\n---\nIdentical metaphor passage.\n",
    );
    write(
        root,
        "notes/no-genre.md",
        "---\ntitle: x\n---\nmetaphor outside\n",
    );
}

fn built(src: &Path, data: &Path) -> Corpus {
    let c = Corpus::from_config(cfg(src, "test"), data);
    c.build_index(false).unwrap();
    c
}

fn unigram_count(c: &Corpus, word: &str, filter: &DocFilter) -> u64 {
    let req = NgramRequest {
        n_min: node_tokens(word).len(),
        n_max: node_tokens(word).len(),
        filter: filter.clone(),
        top: 1_000_000,
        containing: Some(node_tokens(word)),
        ..NgramRequest::default()
    };
    let res = c.ngrams(&req).unwrap();
    res.grams
        .iter()
        .find(|g| g.gram == node_tokens(word).join(" "))
        .map(|g| g.count)
        .unwrap_or(0)
}

#[test]
fn counts_agree_across_views() {
    let src = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    fixture(src.path());
    let c = built(src.path(), data.path());
    let filters = vec![
        DocFilter::default(),
        DocFilter {
            genre: vec!["essay".into()],
            ..DocFilter::default()
        },
        DocFilter {
            year: Some(2016),
            ..DocFilter::default()
        },
        DocFilter {
            lang: vec!["cs".into()],
            ..DocFilter::default()
        },
        DocFilter {
            after: Some("2004".into()),
            before: Some("2017".into()),
            ..DocFilter::default()
        },
    ];
    let mut nonzero = 0;
    for word in ["metaphor", "scaffolding", "don't", "same time", "Metafora"] {
        for f in &filters {
            let mut req = KwicRequest::new(word);
            req.filter = f.clone();
            req.limit = 3;
            let k = c.kwic(&req).unwrap();
            let p = c.profile(word, f).unwrap();
            let uni = unigram_count(&c, word, f);
            assert_eq!(k.total as u64, uni, "{word} {f:?}: kwic vs n-gram");
            assert_eq!(k.total, p.frequency, "{word} {f:?}: kwic vs profile");
            assert_eq!(k.pieces, p.pieces);
            for by in [DistBy::Year, DistBy::Corpus, DistBy::Genre, DistBy::Lang] {
                let d = c.distribution(word, by, f).unwrap();
                let sum: usize = d.buckets.iter().map(|b| b.hits).sum();
                assert_eq!(sum, k.total, "{word} {f:?} {by:?}: dist sum");
                assert_eq!(d.total, k.total);
            }
            let py: usize = p.by_year.iter().map(|b| b.hits).sum();
            assert_eq!(py, p.frequency);
            nonzero += (k.total > 0) as usize;
        }
    }
    assert!(nonzero >= 10, "the fixture exercises real counts");

    // Known absolute counts: metaphor in passages of genre documents only
    // (boilerplate line, heading counted, no-genre file excluded).
    let k = c.kwic(&KwicRequest::new("metaphor")).unwrap();
    // essay: heading 1 + paragraph 4 + list item 2; scaffold 1; note 2;
    // cs 1; undated 1; twins 2.
    assert_eq!(k.total, 14);
    assert_eq!(k.pieces, 7);
}

#[test]
fn amp_amp_amp_is_absent_from_ngrams() {
    let src = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    fixture(src.path());
    let c = built(src.path(), data.path());
    let req = NgramRequest {
        n_min: 1,
        n_max: 5,
        top: 1_000_000,
        ..NgramRequest::default()
    };
    let res = c.ngrams(&req).unwrap();
    assert!(!res.grams.is_empty());
    for g in &res.grams {
        assert!(
            !g.gram
                .split(' ')
                .any(|t| t == "amp" || t == "lt" || t == "gt"),
            "entity residue in {:?}",
            g.gram
        );
    }
    assert!(res.grams.iter().any(|g| g.gram == "salt pepper vinegar"));
}

#[test]
fn dont_is_one_token_in_every_view() {
    let src = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    fixture(src.path());
    let c = built(src.path(), data.path());
    assert_eq!(node_tokens("don’t"), vec!["don't"]);
    let mut req = KwicRequest::new("don't");
    req.sort = KwicSort::Source;
    let k = c.kwic(&req).unwrap();
    assert_eq!(k.node_tokens, vec!["don't"]);
    let nodes: Vec<&str> = k.lines.iter().map(|l| l.node.as_str()).collect();
    // Curly originals are quoted curly; the straight one straight.
    assert_eq!(nodes, vec!["don’t", "don't", "don’t"]);
    assert_eq!(unigram_count(&c, "don't", &DocFilter::default()), 3);
    assert_eq!(unigram_count(&c, "don", &DocFilter::default()), 0);
}

fn source_lines(root: &Path, rel: &str) -> Vec<String> {
    fs::read_to_string(root.join(rel))
        .unwrap()
        .split('\n')
        .map(String::from)
        .collect()
}

#[test]
fn kwic_context_is_an_original_substring() {
    let src = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    fixture(src.path());
    let c = built(src.path(), data.path());
    for width in [0, 1, 3, 8] {
        for term in ["metaphor", "same time", "scaffolding", "pepper"] {
            let mut req = KwicRequest::new(term);
            req.width = width;
            let k = c.kwic(&req).unwrap();
            for l in &k.lines {
                let file = fs::read_to_string(src.path().join(&l.rel_path)).unwrap();
                let ctx = format!("{}{}{}", l.left, l.node, l.right);
                assert!(file.contains(&ctx), "context not original: {ctx:?}");
                let line = &source_lines(src.path(), &l.rel_path)[l.line - 1];
                assert!(
                    line.contains(&l.node),
                    "node {:?} not on line {}",
                    l.node,
                    l.line
                );
                assert!(l.passage_id.starts_with("test:"));
                assert!(l
                    .link
                    .as_deref()
                    .unwrap()
                    .ends_with(&format!("&line={}", l.line)));
            }
        }
    }
    // The link-wrapped, emphasised node renders from the original.
    let mut req = KwicRequest::new("generative metaphor");
    req.width = 2;
    let k = c.kwic(&req).unwrap();
    assert_eq!(k.total, 1, "the frontmatter title is not a passage");
    let body = k
        .lines
        .iter()
        .find(|l| l.rel_path == "essays/2016-metaphor.md")
        .unwrap();
    assert_eq!(body.node, "generative** [metaphor");
    assert_eq!(body.left, "A **");
    assert_eq!(body.right, "](http://x.org/m) is a");
    assert_eq!(body.line, 9);
    // Entity-coded context: the right context quotes `&amp;` as written.
    let mut req = KwicRequest::new("Schön's");
    req.width = 2;
    let k = c.kwic(&req).unwrap();
    assert_eq!(k.lines[0].node, "Schön’s");
    // Punctuation glued to the context edge is kept (T4 follow-up).
    assert_eq!(k.lines[0].right, " metaphor &amp; more.");
}

#[test]
fn sort_ties_are_deterministic() {
    let src = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    fixture(src.path());
    let c = built(src.path(), data.path());
    for sort in ["R1", "L1", "L3", "R3", "date", "source"] {
        let mut req = KwicRequest::new("metaphor");
        req.sort = sort.parse().unwrap();
        let a = serde_json::to_string(&c.kwic(&req).unwrap()).unwrap();
        let b = serde_json::to_string(&c.kwic(&req).unwrap()).unwrap();
        assert_eq!(a, b);
        // Twins have identical context and date: path breaks the tie.
        let k = c.kwic(&req).unwrap();
        let twins: Vec<&str> = k
            .lines
            .iter()
            .filter(|l| l.rel_path.starts_with("twins/"))
            .map(|l| l.rel_path.as_str())
            .collect();
        assert_eq!(twins, vec!["twins/copy-a.md", "twins/copy-b.md"], "{sort}");
    }
    // R1 orders by the right-hand token, then outward.
    let mut req = KwicRequest::new("metaphor");
    req.sort = KwicSort::Right(1);
    let k = c.kwic(&req).unwrap();
    let r1: Vec<String> = k
        .lines
        .iter()
        .map(|l| {
            l.right
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_lowercase()
        })
        .collect();
    assert_eq!(r1[0], "", "passage-final nodes (no R1) sort first");
    // Date: 2004 < 2016-03 < 2016-06-23 < … ; undated last.
    let mut req = KwicRequest::new("metaphor");
    req.sort = KwicSort::Date;
    let k = c.kwic(&req).unwrap();
    assert_eq!(k.lines[0].rel_path, "essays/2004-scaffold.md");
    assert_eq!(k.lines.last().unwrap().rel_path, "undated/u.md");
    // A forced rebuild gives byte-identical output.
    let before = serde_json::to_string(&c.kwic(&KwicRequest::new("metaphor")).unwrap()).unwrap();
    c.build_index(true).unwrap();
    let after = serde_json::to_string(&c.kwic(&KwicRequest::new("metaphor")).unwrap()).unwrap();
    assert_eq!(before, after);
    assert!("X9".parse::<KwicSort>().is_err());
    assert!("R0".parse::<KwicSort>().is_err());
}

#[test]
fn limit_truncates_lines_but_not_total() {
    let src = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    fixture(src.path());
    let c = built(src.path(), data.path());
    let mut req = KwicRequest::new("metaphor");
    req.limit = 2;
    let k = c.kwic(&req).unwrap();
    assert_eq!((k.total, k.returned, k.lines.len()), (14, 2, 2));
}

#[test]
fn date_filters_floor_partial_dates() {
    let src = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    fixture(src.path());
    let c = built(src.path(), data.path());
    let hits = |after: Option<&str>, before: Option<&str>| {
        let f = DocFilter {
            after: after.map(String::from),
            before: before.map(String::from),
            ..DocFilter::default()
        };
        c.distribution("metaphor", DistBy::Year, &f)
            .unwrap()
            .buckets
            .iter()
            .filter(|b| b.hits > 0)
            .map(|b| b.key.clone())
            .collect::<Vec<_>>()
    };
    // after inclusive: the doc dated just "2004" counts from 2004.
    assert_eq!(hits(Some("2004"), Some("2005")), vec!["2004"]);
    // before exclusive: 2016-03 is before 2016-04, 2016-06-23 is not.
    let d = c
        .distribution(
            "metaphor",
            DistBy::Year,
            &DocFilter {
                after: Some("2016".into()),
                before: Some("2016-04".into()),
                ..DocFilter::default()
            },
        )
        .unwrap();
    assert_eq!(d.total, 2, "only the 2016-03 note");
    // Undated documents fail date filters but appear as `unknown` otherwise.
    let all = c
        .distribution("metaphor", DistBy::Year, &DocFilter::default())
        .unwrap();
    assert_eq!(all.buckets.last().unwrap().key, "unknown");
    assert!(DocFilter {
        after: Some("soon".into()),
        ..DocFilter::default()
    }
    .validate()
    .is_err());
}

#[test]
fn dist_buckets_carry_denominators() {
    let src = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    fixture(src.path());
    let c = built(src.path(), data.path());
    let d = c
        .distribution("scaffolding", DistBy::Genre, &DocFilter::default())
        .unwrap();
    let keys: Vec<(&str, usize)> = d.buckets.iter().map(|b| (b.key.as_str(), b.hits)).collect();
    assert_eq!(keys, vec![("essay", 4), ("note", 1)]);
    let status = c.status().unwrap();
    let tokens: i64 = d.buckets.iter().map(|b| b.tokens).sum();
    assert_eq!(tokens, status.tokens);
}

#[test]
fn ngram_stopword_rules() {
    let src = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    fixture(src.path());
    let c = built(src.path(), data.path());
    let run = |strict: bool| {
        c.ngrams(&NgramRequest {
            n_min: 3,
            n_max: 5,
            top: 1000,
            strict_stopwords: strict,
            ..NgramRequest::default()
        })
        .unwrap()
    };
    let loose = run(false);
    let g = loose
        .grams
        .iter()
        .find(|g| g.gram == "at the same time")
        .unwrap();
    assert_eq!(g.count, 4);
    assert_eq!(g.pieces, 3);
    // Top is count desc, then gram asc.
    for w in loose.grams.windows(2) {
        assert!(w[0].count > w[1].count || (w[0].count == w[1].count && w[0].gram < w[1].gram));
    }
    // All-stopword grams are gone by default.
    assert!(!loose
        .grams
        .iter()
        .any(|g| g.gram.split(' ').all(scout_corpus::stopwords::is_stopword)));
    assert_eq!(
        loose
            .grams
            .iter()
            .find(|g| g.gram == "is a metaphor")
            .unwrap()
            .count,
        2
    );
    let strict = run(true);
    // n=3 with 2 stopwords goes; "at the same time" (2 of 4) stays even
    // under the strict rule with the built-in list.
    assert!(!strict.grams.iter().any(|g| g.gram == "is a metaphor"));
    assert!(strict.grams.iter().any(|g| g.gram == "at the same time"));
    assert!(strict.grams.iter().any(|g| g.gram == "salt pepper vinegar"));
    // --since/--until: per-year counts, undated excluded.
    let p = c
        .ngrams(&NgramRequest {
            n_min: 4,
            n_max: 4,
            since: Some(2004),
            until: Some(2016),
            top: 5,
            ..NgramRequest::default()
        })
        .unwrap();
    let g = p
        .grams
        .iter()
        .find(|g| g.gram == "at the same time")
        .unwrap();
    let by: Vec<(&str, u64)> = g
        .by_year
        .as_ref()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.as_str(), *v))
        .collect();
    assert_eq!(by, vec![("2004", 2), ("2016", 2)]);
    assert_eq!(parse_n_range("3-5").unwrap(), (3, 5));
    assert_eq!(parse_n_range("2").unwrap(), (2, 2));
    assert!(parse_n_range("0-3").is_err());
    assert!(parse_n_range("3-6").is_err());
    assert!(parse_n_range("5-3").is_err());
}

#[test]
fn profile_reports_first_use() {
    let src = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    fixture(src.path());
    let c = built(src.path(), data.path());
    let p = c.profile("metaphor", &DocFilter::default()).unwrap();
    let f = p.first_used.as_ref().unwrap();
    assert_eq!(f.rel_path, "essays/2004-scaffold.md");
    assert_eq!(f.date.as_deref(), Some("2004"));
    assert_eq!(f.line, 7);
    assert_eq!(p.total_tokens, c.status().unwrap().tokens);
    let json = serde_json::to_value(&p).unwrap();
    assert_eq!(json["collocates"]["status"], "ok");
    assert_eq!(json["ngrams"]["status"], "ok");
    assert_eq!(json["schema_version"], 2);
    assert!(p.top_documents.len() <= concord::PROFILE_TOP_DOCUMENTS);
}

#[test]
fn verify_quote_finds_exact_text_only() {
    let src = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    fixture(src.path());
    let c = built(src.path(), data.path());
    let v = c.verify_quote(ZPD).unwrap();
    assert!(v.found);
    assert_eq!(v.matches.len(), 1);
    let m = &v.matches[0];
    assert_eq!(m.rel_path, "essays/2004-scaffold.md");
    assert_eq!(m.original, ZPD);
    assert_eq!(m.line, 9);
    assert_eq!(m.passage_id, "test:essays/2004-scaffold.md:9");
    assert!(source_lines(src.path(), &m.rel_path)[m.line - 1].contains(ZPD));
    // One-character alterations miss.
    for altered in [
        ZPD.replace("scaffolding", "scafolding"),
        ZPD.replace("ZPD", "ZDP"),
        ZPD.replace("Chaiklin", "chaiklin"),
        ZPD.replace("general.", "general!"),
        ZPD.replace("any assisted", "any  assisted"),
    ] {
        assert!(!c.verify_quote(&altered).unwrap().found, "{altered}");
    }
    // Quote and apostrophe forms unify both ways; the original is returned.
    let v = c.verify_quote("Vygotsky's point stands.").unwrap();
    assert_eq!(v.matches[0].original, "Vygotsky’s point stands.");
    let v = c.verify_quote("don’t remove scaffolding").unwrap();
    assert_eq!(v.matches[0].original, "don't remove scaffolding");
    // Normalisation does not leak in: entities are matched as written.
    assert!(c.verify_quote("Schön’s metaphor &amp; more").unwrap().found);
    assert!(!c.verify_quote("Schön’s metaphor & more").unwrap().found);
    assert!(c.verify_quote("   ").is_err());
}

#[test]
fn multi_corpus_views_merge() {
    let src = tempfile::tempdir().unwrap();
    let src2 = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    fixture(src.path());
    fixture(src2.path());
    let a = built(src.path(), data.path());
    let b = Corpus::from_config(cfg(src2.path(), "other"), data.path());
    b.build_index(false).unwrap();
    let both = [a.clone(), b];
    let k = concord::kwic(&both, &KwicRequest::new("metaphor")).unwrap();
    assert_eq!(k.total, 28);
    let d =
        concord::distribution(&both, "metaphor", DistBy::Corpus, &DocFilter::default()).unwrap();
    let keys: Vec<(&str, usize)> = d.buckets.iter().map(|b| (b.key.as_str(), b.hits)).collect();
    assert_eq!(keys, vec![("other", 14), ("test", 14)]);
    let n = ngrams::ngrams(
        &both,
        &NgramRequest {
            n_min: 1,
            n_max: 1,
            top: 100_000,
            ..NgramRequest::default()
        },
    )
    .unwrap();
    assert_eq!(
        n.grams.iter().find(|g| g.gram == "metaphor").unwrap().count,
        28
    );
    let v = scout_corpus::verify::verify_quote(&both, ZPD).unwrap();
    let corpora: Vec<&str> = v.matches.iter().map(|m| m.corpus.as_str()).collect();
    assert_eq!(corpora, vec!["other", "test"]);
}

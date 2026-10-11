//! Round-trip property: for the 400 labels made by the seeded generator in
//! `packages/scout-query/generated.test.ts` (letters, digits, `:` `/` `\`,
//! straight and curly quotes and apostrophes, every whitespace code point,
//! field names), stored as themes, entities and highlight tags, every
//! facet value written with `topics::field_query` selects exactly the
//! facet's count of documents.

use scout_corpus::api::{FacetsQuery, SearchQuery};
use scout_corpus::normalize::fold;
use scout_corpus::registry::{CorpusConfig, CorpusKind, DocumentUnit, FieldMap};
use scout_corpus::topics;
use scout_corpus::{Corpus, Engine, Registry};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

const DOCS: usize = 100;

fn labels() -> Vec<String> {
    let text = include_str!("../../../packages/scout-query/fixtures/generated-cases.json");
    let f: serde_json::Value = serde_json::from_str(text).unwrap();
    f["quote"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["label"].as_str().unwrap().to_string())
        .collect()
}

/// Can the work-file meta line (`… | tags: a, b`) hold this tag verbatim?
fn tag_safe(l: &str) -> bool {
    !l.contains([',', '|', '\n', '\r']) && !fold(l).is_empty()
}

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

#[test]
fn every_generated_label_round_trips_from_facet_to_query() {
    let labels = labels();
    assert!(labels.len() >= 300);
    let (src, data) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let mut themes = vec![Vec::new(); DOCS];
    let mut entities = vec![Vec::new(); DOCS];
    let mut tags = vec![Vec::new(); DOCS];
    for (i, l) in labels.iter().enumerate() {
        themes[i % DOCS].push(l.clone());
        themes[(i + 37) % DOCS].push(l.clone());
        entities[(i * 3) % DOCS].push(l.clone());
        if tag_safe(l) {
            tags[(i * 7) % DOCS].push(l.clone());
            tags[(i * 7 + 11) % DOCS].push(l.clone());
        }
    }
    let mut layer = String::new();
    for d in 0..DOCS {
        write(
            src.path(),
            &format!("readings/works/w{d:03}.md"),
            &format!(
                "---\ntitle: W{d}\nauthor: A\ntype: tweet\nsource_system: x\nsource_id: \"{d}\"\n---\n\n> A proof in item {d}.\n\nhighlighted_at: 2026-09-01 | tags: {}\n\n---\n",
                tags[d].join(", ")
            ),
        );
        layer.push_str(
            &serde_json::json!({
                "key": format!("x:{d}"), "date": "2026-09-01", "ai_related": true,
                "themes": themes[d], "entities": entities[d], "event": null
            })
            .to_string(),
        );
        layer.push('\n');
    }
    write(src.path(), topics::LAYER_FILE, &layer);
    let cfg = CorpusConfig {
        id: "h".into(),
        name: "H".into(),
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
    };
    let c = Corpus::from_config(cfg.clone(), data.path());
    let t = c.build_index(false).unwrap().topics.unwrap();
    assert_eq!((t.matched_by_key, t.bad_lines), (DOCS, 0));
    let e = Engine::new(Registry { corpora: vec![cfg] }, data.path());
    let total = |q: &str| {
        e.search(&SearchQuery {
            query: q.to_string(),
            limit: 1,
            ..Default::default()
        })
        .unwrap_or_else(|err| panic!("{q:?}: {err}"))
        .body
        .total_documents
    };

    // Themes and entities: every facet value gives back its count.
    let f = e
        .facets(&FacetsQuery {
            query: "proof".into(),
            top: 0,
            ..Default::default()
        })
        .unwrap()
        .body;
    assert_eq!(f.total_documents, DOCS);
    let distinct: BTreeSet<String> = labels
        .iter()
        .map(|l| fold(l))
        .filter(|l| !l.is_empty())
        .collect();
    assert!(distinct.len() >= 300, "{} distinct labels", distinct.len());
    let mut checked = 0;
    for (field, facet) in [("theme", &f.facets.theme), ("entity", &f.facets.entity)] {
        assert_eq!(
            facet.distinct,
            distinct.len(),
            "{field}: every label is a facet value"
        );
        for it in &facet.items {
            let fq = topics::field_query(field, &it.value);
            assert_eq!(total(&fq), it.count, "{fq:?}");
            assert_eq!(total(&format!("proof {fq} /item/")), it.count, "{fq:?}");
            checked += 1;
        }
    }
    // The generated labels themselves (not only the facets' chosen
    // spellings) select the same documents.
    let theme_counts: BTreeMap<String, usize> = f
        .facets
        .theme
        .items
        .iter()
        .map(|i| (fold(&i.value), i.count))
        .collect();
    for l in labels.iter().filter(|l| !fold(l).is_empty()) {
        let fq = topics::field_query("theme", l);
        assert_eq!(total(&fq), theme_counts[&fold(l)], "{fq:?}");
        checked += 1;
    }

    // Tags (no facet): the expected count is the documents given a tag
    // with the same fold.
    let mut tag_docs: BTreeMap<String, BTreeSet<usize>> = BTreeMap::new();
    for (d, ts) in tags.iter().enumerate() {
        for t in ts {
            tag_docs.entry(fold(t)).or_default().insert(d);
        }
    }
    let mut tag_checked = 0;
    for l in labels.iter().filter(|l| tag_safe(l)) {
        let fq = topics::field_query("tag", l);
        assert_eq!(total(&fq), tag_docs[&fold(l)].len(), "{fq:?}");
        tag_checked += 1;
    }
    assert!(checked >= 900, "{checked}");
    assert!(tag_checked >= 150, "{tag_checked} tag labels");
}

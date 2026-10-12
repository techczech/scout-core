//! Seam tests for the theme × month matrix (`Engine::theme_months`,
//! `scout topics theme-months`): every month of the range, every theme
//! present, and each cell equal to the count `facets` gives that theme
//! for the same query with `month:YYYY-MM`. Read-only.

use scout_corpus::api::{FacetsQuery, ThemeMonthsQuery, ThemeMonthsResults};
use scout_corpus::registry::{CorpusConfig, CorpusKind, DocumentUnit, FieldMap};
use scout_corpus::topics;
use scout_corpus::{Corpus, Engine, Registry};
use serde_json::json;
use std::fs;
use std::path::Path;

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

/// One corpus: works `1..=n` of `rows`, each `(date, themes, text)`; a
/// work without a date in `rows` has no tag row.
fn corpus(id: &str, root: &Path, data: &Path, rows: &[(&str, &[&str], &str)]) -> Corpus {
    let mut layer = String::new();
    for (i, (date, themes, text)) in rows.iter().enumerate() {
        let n = i + 1;
        write(
            root,
            &format!("readings/works/w-{n}.md"),
            &format!(
                "---\ntitle: W{n}\nauthor: A\ntype: tweet\nsource_system: {id}\nsource_id: \"{n}\"\n---\n\n> {text}\n\nhighlighted_at: 2020-01-01\n\n---\n"
            ),
        );
        if date.is_empty() {
            continue;
        }
        layer.push_str(
            &json!({"key": format!("{id}:{n}"), "path": format!("readings/works/w-{n}.md"),
                    "date": date, "ai_related": true, "themes": themes, "entities": [],
                    "event": null, "event_name": null})
            .to_string(),
        );
        layer.push('\n');
    }
    write(root, topics::LAYER_FILE, &layer);
    let c = Corpus::from_config(
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
        },
        data,
    );
    c.build_index(false).unwrap();
    c
}

struct Fx {
    _dirs: Vec<tempfile::TempDir>,
    engine: Engine,
}

/// Two corpora. `one`: seven works over 2025-11 to 2026-03 (no item in
/// 2026-01), one untagged, one tagged without themes, one with a theme
/// written in another case. `two`: three works, sharing a theme name.
fn fixture() -> Fx {
    let dirs: Vec<tempfile::TempDir> = (0..3).map(|_| tempfile::tempdir().unwrap()).collect();
    let data = dirs[2].path();
    let one = corpus(
        "one",
        dirs[0].path(),
        data,
        &[
            ("2025-11-03", &["Maths", "Coding"], "A proof about agents."),
            ("2025-12-10", &["Maths"], "A proof, again."),
            ("2025-12-11", &["Coding", "maths"], "Agents write code."),
            ("2026-02-01", &["Coding"], "Agents and a proof."),
            ("2026-02-02", &[], "Tagged, no theme, agents."),
            (
                "2026-03-20",
                &["Coding", "Coding", "Zeta"],
                "A proof at last.",
            ),
            ("", &[], "Untagged proof by agents."),
        ],
    );
    let two = corpus(
        "two",
        dirs[1].path(),
        data,
        &[
            ("2026-02-14", &["Maths"], "Another proof."),
            ("2026-03-01", &["Alpha"], "Agents elsewhere."),
            ("2024-06-01", &["Old"], "An old proof."),
        ],
    );
    let engine = Engine::new(
        Registry {
            corpora: vec![one.config.clone(), two.config.clone()],
        },
        data,
    );
    Fx {
        _dirs: dirs,
        engine,
    }
}

fn matrix(fx: &Fx, q: ThemeMonthsQuery) -> ThemeMonthsResults {
    fx.engine.theme_months(&q).unwrap().body
}

/// Every cell, every theme total and every month total against `facets`
/// for the same query with `month:` (and `theme:` for a total).
fn equals_facets(fx: &Fx, m: &ThemeMonthsResults, query: &str, in_: &[&str]) {
    let in_: Vec<String> = in_.iter().map(|s| s.to_string()).collect();
    for (mi, month) in m.months.iter().enumerate() {
        let f = fx
            .engine
            .facets(&FacetsQuery {
                query: format!("month:{month} {query}"),
                in_: in_.clone(),
                top: 0,
                min_event_items: 1,
            })
            .unwrap()
            .body;
        assert_eq!(m.month_totals[mi], f.tagged_documents, "{month} {query:?}");
        let count = |theme: &str| {
            f.facets
                .theme
                .items
                .iter()
                .find(|i| topics::fold(&i.value) == topics::fold(theme))
                .map_or(0, |i| i.count)
        };
        for (ti, theme) in m.themes.iter().enumerate() {
            assert_eq!(m.counts[ti][mi], count(theme), "{theme} {month} {query:?}");
        }
        // And no theme of the month is missing from the matrix.
        for item in &f.facets.theme.items {
            assert!(
                m.themes
                    .iter()
                    .any(|t| topics::fold(t) == topics::fold(&item.value)),
                "{} {month}",
                item.value
            );
        }
    }
    for (ti, theme) in m.themes.iter().enumerate() {
        assert_eq!(
            m.theme_totals[ti],
            m.counts[ti].iter().sum::<usize>(),
            "{theme}"
        );
        assert_eq!(m.counts[ti].len(), m.months.len());
    }
    assert_eq!(m.documents, m.month_totals.iter().sum::<usize>());
}

#[test]
fn every_cell_is_the_facet_count_of_its_theme_and_month() {
    let fx = fixture();
    // One corpus, no query: every tagged document.
    let m = matrix(
        &fx,
        ThemeMonthsQuery {
            in_: vec!["one".into()],
            ..Default::default()
        },
    );
    assert_eq!(m.corpora, ["one"]);
    assert_eq!(
        m.months,
        ["2025-11", "2025-12", "2026-01", "2026-02", "2026-03"]
    );
    assert_eq!(
        (m.from.as_deref(), m.to.as_deref()),
        (Some("2025-11"), Some("2026-03"))
    );
    // By total, largest first; ties by name. The spelling is the most
    // frequent one; a theme named twice on one item counts once.
    assert_eq!(m.themes, ["Coding", "Maths", "Zeta"]);
    assert_eq!(m.theme_totals, [4, 3, 1]);
    assert_eq!(
        m.counts,
        [
            vec![1, 1, 0, 1, 1],
            vec![1, 2, 0, 0, 0],
            vec![0, 0, 0, 0, 1]
        ]
    );
    // Distinct tagged documents per month: the one without a theme counts
    // here and in no row; the untagged one nowhere.
    assert_eq!(m.month_totals, [1, 2, 0, 2, 1]);
    assert_eq!(m.documents, 6);
    equals_facets(&fx, &m, "", &["one"]);

    // Narrowed by a query, in the search grammar.
    for query in [
        "proof",
        "agents",
        "proof -agents",
        "theme:Coding",
        "nothing-matches",
    ] {
        let m = matrix(
            &fx,
            ThemeMonthsQuery {
                in_: vec!["one".into()],
                from: Some("2025-11".into()),
                to: Some("2026-03".into()),
                query: query.into(),
            },
        );
        assert_eq!(m.months.len(), 5, "{query}");
        assert_eq!(m.query, query);
        equals_facets(&fx, &m, query, &["one"]);
    }
    let m = matrix(
        &fx,
        ThemeMonthsQuery {
            in_: vec!["one".into()],
            query: "proof".into(),
            ..Default::default()
        },
    );
    // Without bounds the range is that of the documents counted.
    assert_eq!(
        m.months,
        ["2025-11", "2025-12", "2026-01", "2026-02", "2026-03"]
    );
    assert_eq!(m.themes, ["Coding", "Maths", "Zeta"]);
    assert_eq!(
        m.counts,
        [
            vec![1, 0, 0, 1, 1],
            vec![1, 1, 0, 0, 0],
            vec![0, 0, 0, 0, 1]
        ]
    );
    assert_eq!(m.month_totals, [1, 1, 0, 1, 1]);
}

#[test]
fn several_corpora_are_added_up_and_a_theme_is_its_name() {
    let fx = fixture();
    let m = matrix(
        &fx,
        ThemeMonthsQuery {
            from: Some("2026-02".into()),
            ..Default::default()
        },
    );
    assert_eq!(m.corpora, ["one", "two"]);
    assert_eq!(m.months, ["2026-02", "2026-03"]);
    // "Maths" of `two` and "Coding" of `one` in February; ties by name.
    assert_eq!(m.themes, ["Coding", "Alpha", "Maths", "Zeta"]);
    assert_eq!(m.counts, [vec![1, 1], vec![0, 1], vec![1, 0], vec![0, 1]]);
    assert_eq!(m.month_totals, [3, 2]);
    equals_facets(&fx, &m, "", &[]);
    // The whole range of both: 2024-06 to 2026-03, every month listed.
    let all = matrix(&fx, ThemeMonthsQuery::default());
    assert_eq!(all.months.len(), 22);
    assert_eq!(
        (all.months[0].as_str(), all.months[21].as_str()),
        ("2024-06", "2026-03")
    );
    assert_eq!(all.themes, ["Coding", "Maths", "Alpha", "Old", "Zeta"]);
    assert_eq!(all.theme_totals, [4, 4, 1, 1, 1]);
    assert_eq!(all.documents, 9);
    equals_facets(&fx, &all, "", &[]);
    let q = matrix(
        &fx,
        ThemeMonthsQuery {
            query: "proof".into(),
            ..Default::default()
        },
    );
    equals_facets(&fx, &q, "proof", &[]);
    // `in:` in the query selects the corpus, as in facets.
    let two = matrix(
        &fx,
        ThemeMonthsQuery {
            query: "in:two proof".into(),
            ..Default::default()
        },
    );
    assert_eq!(two.corpora, ["two"]);
    assert_eq!(two.themes, ["Maths", "Old"]);
}

#[test]
fn bounds_are_months_and_an_empty_range_is_empty() {
    let fx = fixture();
    let q = |from: Option<&str>, to: Option<&str>| ThemeMonthsQuery {
        in_: vec!["one".into()],
        from: from.map(String::from),
        to: to.map(String::from),
        query: String::new(),
    };
    // Bounds beyond the documents: the months asked for, empty ones too.
    let m = matrix(&fx, q(Some("2026-03"), Some("2026-05")));
    assert_eq!(m.months, ["2026-03", "2026-04", "2026-05"]);
    assert_eq!(m.month_totals, [1, 0, 0]);
    assert_eq!(m.themes, ["Coding", "Zeta"]);
    // One bound: the other is the documents'.
    let m = matrix(&fx, q(None, Some("2025-12")));
    assert_eq!(m.months, ["2025-11", "2025-12"]);
    // Nothing in range: no month, no theme, and not an error.
    let m = matrix(&fx, q(Some("2027-01"), None));
    assert!(m.months.is_empty() && m.themes.is_empty() && m.counts.is_empty());
    assert_eq!((m.documents, m.from, m.to), (0, None, None));
    let m = matrix(&fx, q(Some("2027-01"), Some("2027-02")));
    assert_eq!((m.months.len(), m.documents), (2, 0));
    for (from, to, why) in [
        (Some("2026"), None, "--from \"2026\" is not a month YYYY-MM"),
        (
            None,
            Some("2026-13"),
            "--to \"2026-13\" is not a month YYYY-MM",
        ),
        (
            Some("2026-03"),
            Some("2026-02"),
            "--to 2026-02 is before --from 2026-03",
        ),
        (Some("0001-01"), Some("9999-12"), "at most 1800 are listed"),
    ] {
        let e = fx
            .engine
            .theme_months(&q(from, to))
            .unwrap_err()
            .to_string();
        assert!(e.contains(why), "{e}");
    }
    // The reply's JSON shape.
    let v = serde_json::to_value(matrix(&fx, q(Some("2026-02"), Some("2026-03")))).unwrap();
    assert_eq!(
        v,
        json!({"schema_version": 1, "query": "", "corpora": ["one"], "from": "2026-02",
               "to": "2026-03", "months": ["2026-02", "2026-03"], "themes": ["Coding", "Zeta"],
               "counts": [[1, 1], [0, 1]], "theme_totals": [2, 1], "month_totals": [2, 1],
               "documents": 3})
    );
}

#[test]
fn a_query_with_a_month_field_is_refused_and_every_other_field_keeps_the_equality() {
    let fx = fixture();
    let q = |query: &str, in_: &[&str]| ThemeMonthsQuery {
        in_: in_.iter().map(|s| s.to_string()).collect(),
        from: Some("2025-11".into()),
        to: Some("2026-03".into()),
        query: query.into(),
    };
    // `month:` as the engine's parser reads it: alone, among terms, after
    // other fields, as a year, with and without bounds, in any case.
    for query in [
        "month:2026-02",
        "proof month:2026-02",
        "theme:Coding month:2025-12 agents",
        "month:2026",
        "in:one month:2026-02",
        "MONTH:2026-02",
    ] {
        for bounded in [true, false] {
            let mut ask = q(query, &[]);
            if !bounded {
                (ask.from, ask.to) = (None, None);
            }
            let e = fx
                .engine
                .theme_months(&ask)
                .err()
                .unwrap_or_else(|| panic!("{query:?} was accepted"))
                .to_string();
            assert!(
                e.contains("in the query") && e.contains("use --from and --to"),
                "{query}: {e}"
            );
        }
    }
    // A month that is no month is the parser's own refusal, as in search.
    let e = fx.engine.theme_months(&q("month:soon", &[])).unwrap_err();
    assert!(e.to_string().contains("month:"), "{e}");
    // The text `month:` inside a phrase is a phrase, not the field.
    let m = matrix(&fx, q("\"month:2026-02\"", &["one"]));
    assert_eq!(m.documents, 0);
    equals_facets(&fx, &m, "\"month:2026-02\"", &["one"]);

    // Every other field: each cell is still the facet count.
    for query in [
        "theme:Maths",
        "theme:Coding proof",
        "ai:true",
        "ai:false",
        "ty:tweet agents",
        "au:A proof",
        "ti:W3",
        "source:one",
        "after:2025-12 proof",
        "before:2026-02",
        "y:2025",
        "proof OR agents",
        "-proof",
    ] {
        for in_ in [&["one"][..], &[][..]] {
            let m = matrix(&fx, q(query, in_));
            assert_eq!(m.months.len(), 5, "{query}");
            equals_facets(&fx, &m, query, in_);
        }
        // Without bounds too: the range is the documents'.
        let m = matrix(
            &fx,
            ThemeMonthsQuery {
                query: query.into(),
                ..Default::default()
            },
        );
        equals_facets(&fx, &m, query, &[]);
    }
    // At least one of them counted something in more than one month.
    let m = matrix(&fx, q("theme:Coding", &["one"]));
    assert_eq!(m.month_totals, [1, 1, 0, 1, 1]);
}

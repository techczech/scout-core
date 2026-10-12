//! Seam tests for what a filter panel needs of the engine (themes, named
//! entities, events, months, each a tick box with a count):
//!
//! - several values of one topic field: `theme:` / `entity:` / `event:`
//!   terms are ANDed, `month:` terms ORed, every other field keeps its last
//!   value;
//! - a number shown beside a value is the number of documents the search
//!   returns with that value added, for every value of every facet, with
//!   and without a selection;
//! - `facets` on an empty query;
//! - entity types from the tag rows and the corrections to a hit and to
//!   the entity facet;
//! - `sort: newest | oldest`, and relevance untouched by it.
//!
//! The fixture is two corpora: `notes` (a Markdown folder, so documents
//! have their own dates or none; no records folder) and `hl` (a highlights
//! archive with a records folder, so an item can be in two events).

use scout_corpus::api::{self, FacetsQuery, SearchQuery, Sort, ThemeMonthsQuery, TopicsQuery};
use scout_corpus::search::Plan;
use scout_corpus::topics::{self, FacetItem, TopicInfo};
use scout_corpus::{Engine, Registry, SearchMode, SearchResults};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

struct Fx {
    _tmp: tempfile::TempDir,
    engine: Engine,
}

/// `(file, frontmatter date line, text)` of the notes.
const NOTES: &[(&str, &str, &str)] = &[
    ("n1.md", "date: 2026-02-10\n", "A proof about agents."),
    ("n2.md", "date: 2026-02-10\n", "Agents write code."),
    ("n3.md", "date: 2026\n", "A proof again."),
    (
        "n4.md",
        "date: \"2025-12-31T10:00:00Z\"\n",
        "Old proof by agents.",
    ),
    ("n5.md", "", "Undated proof."),
    ("n6.md", "", "Undated agents, untagged."),
    ("n7.md", "date: 2024-06\n", "A proof with no month."),
    ("n8.md", "date: soon\n", "A proof someday."),
    (
        "n9.md",
        "date: \"2025-12-31T08:00:00Z\"\n",
        "A morning proof.",
    ),
];

fn typed(pairs: &[(&str, &str)]) -> (Value, Value) {
    (
        json!(pairs.iter().map(|p| p.0).collect::<Vec<_>>()),
        json!(pairs
            .iter()
            .map(|p| json!({"name": p.0, "type": p.1}))
            .collect::<Vec<_>>()),
    )
}

fn fixture() -> Fx {
    let tmp = tempfile::tempdir().unwrap();
    let (notes, hl) = (tmp.path().join("notes"), tmp.path().join("hl"));
    for (file, date, text) in NOTES {
        write(
            &notes,
            file,
            &format!("---\ntitle: {file}\nauthor: A\n{date}---\n\n{text}\n"),
        );
    }
    let row = |n: u32,
               date: Option<&str>,
               themes: &[&str],
               ents: &[(&str, &str)],
               event: Option<&str>,
               ai: bool| {
        let (entities, entities_typed) = typed(ents);
        json!({"key": format!("n:{n}"), "path": format!("n{n}.md"), "date": date,
               "month": "2022-Q2", "ai_related": ai, "themes": themes,
               "entities": entities, "entities_typed": entities_typed,
               "event": event, "event_name": event.map(|_| "Declaration")})
        .to_string()
    };
    let e01 = Some("2026-02/E01");
    let layer = [
        row(
            1,
            Some("2026-02-10"),
            &["Maths", "Coding"],
            &[("Tao", "person"), ("Astra", "model")],
            e01,
            true,
        ),
        row(
            2,
            Some("2026-02-11"),
            &["Coding"],
            &[("Astra", "product")],
            e01,
            true,
        ),
        row(
            3,
            Some("2026-03-05"),
            &["Maths", "coding"],
            &[("Tao", "person")],
            None,
            true,
        ),
        row(
            4,
            Some("2025-12-31"),
            &["Maths", "Coding", "Zeta"],
            &[("astra", "model"), ("Tao", "org")],
            None,
            true,
        ),
        row(5, Some("2026-03-09"), &["Maths"], &[], None, false),
        // No date and a layer month that is no month: counted in no month.
        row(7, None, &["Coding"], &[("Gamma", "place")], None, true),
        // Types that are not read: a name the row does not list, an item
        // of another shape, a blank type. The row is still a row.
        json!({"key": "n:9", "path": "n9.md", "date": "2025-12-30", "ai_related": true,
               "themes": ["Zeta"], "entities": ["Delta", "Epsilon"],
               "entities_typed": [{"name": "Other", "type": "person"}, "Delta", 7,
                                  {"name": "Delta", "type": " "}, {"name": " epsilon ", "type": " work "}],
               "event": null, "event_name": null})
        .to_string(),
    ]
    .join("\n");
    write(&notes, topics::LAYER_FILE, &layer);
    let numbered = |n: u32, key: &str, change: Value| {
        let mut line = json!({"id": format!("c{n:06}"), "at": "2026-10-12T09:00:00Z",
                              "date": "2026-10-12", "by": "dominik", "key": key});
        for (k, v) in change.as_object().unwrap() {
            line[k] = v.clone();
        }
        line.to_string()
    };
    let corrections = [
        numbered(
            1,
            "n:5",
            json!({"entity_add": ["Ada"], "entity_type": "person"}),
        ),
        numbered(
            2,
            "n:5",
            json!({"entity_add": ["Bob"], "entity_type": "org"}),
        ),
        numbered(3, "n:5", json!({"undoes": "c000002"})),
        numbered(4, "n:7", json!({"entity_remove": ["gamma"]})),
        // The row has the name by now: the line's type is still its type.
        numbered(
            5,
            "n:9",
            json!({"entity_add": ["delta"], "entity_type": "event"}),
        ),
        // A whole list, by hand: a kept name keeps its type.
        json!({"key": "n:2", "entities": ["Astra", "New"]}).to_string(),
    ]
    .join("\n");
    write(&notes, topics::CORRECTIONS_FILE, &corrections);

    for n in 1..=4 {
        write(
            &hl,
            &format!("readings/works/w-{n}.md"),
            &format!(
                "---\ntitle: W{n}\nauthor: B\ntype: tweet\nsource_system: x\nsource_id: \"{n}\"\n---\n\n> A proof, number {n}, by agents.\n\nhighlighted_at: 2026-0{n}-15\n\n---\n"
            ),
        );
    }
    let hrow = |n: u32, date: &str, themes: &[&str], event: Option<&str>, events: &[&str]| {
        json!({"key": format!("x:{n}"), "path": format!("readings/works/w-{n}.md"),
               "date": date, "ai_related": n != 4, "themes": themes,
               "entities": ["Tao"], "entities_typed": [{"name": "Tao", "type": "person"}],
               "event": event, "event_name": event.map(|_| "Monthly"), "events": events})
        .to_string()
    };
    let layer = [
        hrow(
            1,
            "2026-02-20",
            &["Maths"],
            Some("2026-02/E01"),
            &["2026-03-launch"],
        ),
        hrow(
            2,
            "2026-03-02",
            &["Maths", "Coding"],
            Some("2026-03/E02"),
            &[],
        ),
        hrow(3, "2026-02-21", &["Coding"], Some("2026-02/E01"), &[]),
        hrow(4, "2026-04-01", &["Alpha"], None, &[]),
    ]
    .join("\n");
    write(&hl, topics::LAYER_FILE, &layer);
    for (id, member) in [
        ("2026-02-declaration", "2026-02/E01"),
        ("2026-03-launch", "2026-03/E02"),
    ] {
        write(
            &hl,
            &format!("analysis/events/{id}.md"),
            &format!("---\nid: {id}\nname: {id}\nmembers:\n  - {member}\n---\n\nAbout it.\n"),
        );
    }
    let reg = Registry::parse(&format!(
        "[[corpus]]\nid = \"notes\"\nname = \"N\"\nkind = \"markdown-folder\"\npath = \"{}\"\nexclude = [\"analysis/**\"]\n\n\
         [[corpus]]\nid = \"hl\"\nname = \"H\"\nkind = \"highlight-scout-archive\"\npath = \"{}\"\n",
        notes.display(),
        hl.display()
    ))
    .unwrap();
    let engine = Engine::new(reg, tmp.path().join("indexes"));
    engine
        .index_build(&api::IndexBuildQuery::default())
        .unwrap();
    Fx { _tmp: tmp, engine }
}

fn search(fx: &Fx, query: &str, in_: &[&str], sort: Sort, limit: usize) -> SearchResults {
    fx.engine
        .search(&SearchQuery {
            query: query.into(),
            in_: in_.iter().map(|s| s.to_string()).collect(),
            limit,
            mode: SearchMode::Fts,
            sort,
            ..Default::default()
        })
        .unwrap()
        .body
}

/// The documents a search returns, as `corpus:file`, in order.
fn keys(r: &SearchResults) -> Vec<String> {
    r.results
        .iter()
        .map(|d| {
            let file = d.rel_path.rsplit('/').next().unwrap();
            format!("{}:{}", d.corpus, file.trim_end_matches(".md"))
        })
        .collect()
}

fn found(fx: &Fx, query: &str) -> Vec<String> {
    let mut k = keys(&search(fx, query, &[], Sort::Relevance, 100));
    k.sort();
    k
}

fn total(fx: &Fx, query: &str, in_: &[&str]) -> usize {
    search(fx, query, in_, Sort::Relevance, 1).total_documents
}

fn facets(fx: &Fx, query: &str, in_: &[&str]) -> api::FacetsResults {
    fx.engine
        .facets(&FacetsQuery {
            query: query.into(),
            in_: in_.iter().map(|s| s.to_string()).collect(),
            top: 0,
            min_event_items: 1,
        })
        .unwrap()
        .body
}

#[test]
fn repeated_theme_entity_and_event_terms_are_anded_and_month_terms_ored() {
    let fx = fixture();
    // One value, then two: every one must be carried.
    assert_eq!(
        found(&fx, "theme:Maths"),
        ["hl:w-1", "hl:w-2", "notes:n1", "notes:n3", "notes:n4", "notes:n5"]
    );
    assert_eq!(
        found(&fx, "theme:Maths theme:Coding"),
        ["hl:w-2", "notes:n1", "notes:n3", "notes:n4"]
    );
    assert_eq!(
        found(&fx, "theme:Maths theme:coding theme:ZETA"),
        ["notes:n4"]
    );
    assert_eq!(found(&fx, "theme:Zeta theme:Alpha"), [] as [&str; 0]);
    // Before this, the last one won: `theme:Zeta theme:Maths` was
    // `theme:Maths`.
    assert_eq!(found(&fx, "theme:Zeta theme:Maths"), ["notes:n4"]);
    assert_eq!(
        found(&fx, "entity:Tao entity:Astra"),
        ["notes:n1", "notes:n4"]
    );
    // An item in two events (a corpus with a records folder): by lasting
    // id, by monthly id, and both at once.
    assert_eq!(
        found(&fx, "in:hl event:2026-02-declaration"),
        ["hl:w-1", "hl:w-3"]
    );
    assert_eq!(found(&fx, "in:hl event:2026-03/E02"), ["hl:w-1", "hl:w-2"]);
    assert_eq!(
        found(&fx, "in:hl event:2026-02/E01 event:2026-03-launch"),
        ["hl:w-1"]
    );

    // Months: any of them; a year and a month mix.
    assert_eq!(
        found(&fx, "month:2026-02"),
        ["hl:w-1", "hl:w-3", "notes:n1", "notes:n2"]
    );
    assert_eq!(
        found(&fx, "month:2026-02 month:2025-12"),
        ["hl:w-1", "hl:w-3", "notes:n1", "notes:n2", "notes:n4", "notes:n9"]
    );
    assert_eq!(
        found(&fx, "month:2025 month:2026-04"),
        ["hl:w-4", "notes:n4", "notes:n9"]
    );
    assert_eq!(found(&fx, "month:2026 month:2026-02").len(), 8);
    assert_eq!(found(&fx, "month:2026").len(), 8);

    // Mixed with text, `in:` and the other fields.
    assert_eq!(
        found(
            &fx,
            "proof theme:Maths month:2026-03 in:notes theme:Coding month:2026-02"
        ),
        ["notes:n1", "notes:n3"]
    );
    // Every other field keeps its last value.
    assert_eq!(
        found(&fx, "in:hl in:notes theme:Zeta"),
        ["notes:n4", "notes:n9"]
    );
    assert_eq!(found(&fx, "au:B au:A theme:Alpha"), [] as [&str; 0]);
    assert_eq!(found(&fx, "au:A au:B theme:Alpha"), ["hl:w-4"]);
    assert_eq!(found(&fx, "ai:true ai:false theme:Maths"), ["notes:n5"]);
    assert_eq!(found(&fx, "y:2026 y:2024 ti:n1 ti:n7"), ["notes:n7"]);

    // Each term needs a value, and each month must be one.
    let err = |q: &str| {
        let q = SearchQuery {
            query: q.into(),
            ..Default::default()
        };
        format!("{:#}", fx.engine.search(&q).unwrap_err())
    };
    assert!(err("theme:A theme:").contains("theme: needs a value"));
    assert!(err("theme: theme:A").contains("theme: needs a value"));
    assert!(err("month:2026-02 month:2026-13").contains("month:2026-13"));
}

#[test]
fn the_plan_shows_one_value_as_before_and_several_in_a_list() {
    let plan = |q: &str| serde_json::to_value(Plan::parse(q).unwrap()).unwrap()["filters"].clone();
    // One term of a field: the JSON it always was.
    assert_eq!(
        plan("theme:A entity:B event:c month:2026-02 ai:true"),
        json!({"theme": "A", "entity": "B", "event": "c", "ai": true, "month": "2026-02"})
    );
    // Several: the string key is the last one, the list key has them all.
    assert_eq!(
        plan("theme:A theme:\"B c\" month:2026 month:2026-02 entity:E"),
        json!({"theme": "B c", "themes": ["A", "B c"], "entity": "E",
               "month": "2026-02", "months": ["2026", "2026-02"]})
    );
    assert_eq!(
        plan("event:a event:b entity:x entity:y"),
        json!({"entity": "y", "entities": ["x", "y"], "event": "b", "events": ["a", "b"]})
    );
    assert_eq!(plan("proof"), json!({}));
    // The filter written back as terms reads back as itself.
    let p = Plan::parse("theme:A theme:\"B c\" month:2026 month:2026-02 au:x au:y").unwrap();
    assert_eq!(
        p.filter.spec(),
        "au:y theme:A theme:\"B c\" month:2026 month:2026-02"
    );
    assert_eq!(Plan::parse(&p.filter.spec()).unwrap().filter, p.filter);
}

/// The whole property for one query: `rest` plus one `month:` term per
/// month of `months`.
fn counts_are_search_totals(fx: &Fx, rest: &str, months: &[&str], in_: &[&str]) {
    let month_terms: Vec<String> = months.iter().map(|m| format!("month:{m}")).collect();
    let base = format!("{rest} {}", month_terms.join(" "));
    let base = base.trim();
    let what = format!("{base:?} in {in_:?}");
    let f = facets(fx, base, in_);
    let bare = Plan::parse(base).unwrap().is_bare();
    if !bare {
        assert_eq!(f.total_documents, total(fx, base, in_), "{what}");
    }
    // Themes, entities and events: under the whole query, so the count is
    // the search with that value added (AND).
    let with = |field: &str, it: &FacetItem| {
        let q = format!("{base} {}", topics::field_query(field, &it.value));
        match &it.corpus {
            Some(c) => total(fx, &q, &[c.as_str()]),
            None => total(fx, &q, in_),
        }
    };
    for it in &f.facets.theme.items {
        assert_eq!(it.count, with("theme", it), "{what} theme {}", it.value);
    }
    for it in &f.facets.entity.items {
        assert_eq!(it.count, with("entity", it), "{what} entity {}", it.value);
    }
    for it in &f.facets.event.items {
        assert_eq!(it.count, with("event", it), "{what} event {}", it.value);
    }
    // `ai`: the last `ai:` term wins, so picking a value replaces the
    // query's own. A value's count is the search with that value as the
    // `ai:` term: appended (it is then the last), and equally with the
    // query's own `ai:` terms taken out first.
    let no_ai = base
        .split(' ')
        .filter(|t| !t.starts_with("ai:"))
        .collect::<Vec<_>>()
        .join(" ");
    for (count, value) in [(f.facets.ai.yes, "true"), (f.facets.ai.no, "false")] {
        let appended = total(fx, &format!("{base} ai:{value}"), in_);
        assert_eq!(count, appended, "{what} ai:{value}");
        let replaced = total(fx, &format!("{no_ai} ai:{value}"), in_);
        assert_eq!(count, replaced, "{what} ai:{value} replacing");
    }
    // The `ai` counts are the same whichever value the query has picked.
    for value in ["true", "false"] {
        let picked = facets(fx, &format!("{no_ai} ai:{value}"), in_);
        assert_eq!(picked.facets.ai, f.facets.ai, "{what} with ai:{value}");
    }
    // Months: under the rest of the query, without the months picked. So a
    // month's count is the search of the rest with that month alone …
    let selected = if months.is_empty() {
        0
    } else {
        total(fx, base, in_)
    };
    for it in &f.facets.month.items {
        let alone = total(fx, &format!("{rest} month:{}", it.value), in_);
        assert_eq!(it.count, alone, "{what} month {}", it.value);
        // … and what ticking it adds to the months already picked: the
        // search with it added returns that many more, unless a picked
        // month (or its year) already holds it.
        let covered = months
            .iter()
            .any(|m| *m == it.value || (m.len() == 4 && it.value.starts_with(m)));
        let added = total(fx, &format!("{base} month:{}", it.value), in_);
        if covered {
            assert_eq!(added, selected, "{what} month {} again", it.value);
        } else {
            assert_eq!(added, selected + it.count, "{what} + month {}", it.value);
        }
    }
    // The months listed do not shrink when one is picked.
    let unpicked = facets(fx, rest, in_);
    assert_eq!(f.facets.month, unpicked.facets.month, "{what}");
}

#[test]
fn every_facet_count_is_the_total_of_the_search_with_that_value_added() {
    let fx = fixture();
    let rests = [
        "",
        "in:notes",
        "in:hl",
        "proof",
        "agents OR again",
        "proof -about",
        "theme:Maths",
        "theme:Maths theme:Coding",
        "entity:Tao",
        "entity:Tao entity:astra proof",
        "event:2026-02/E01",
        "in:hl event:2026-02-declaration event:2026-03-launch",
        "ai:true theme:coding",
        "ai:true",
        "ai:false",
        "ai:false proof",
        "ai:true ai:false",
        "ai:false theme:Maths ai:true",
        "au:A theme:Maths",
    ];
    let picks: [&[&str]; 6] = [
        &[],
        &["2026-02"],
        &["2026-02", "2026-03"],
        &["2026"],
        &["2026", "2025-12"],
        &["2024-01"],
    ];
    let mut values = 0;
    for rest in rests {
        for months in picks {
            for in_ in [&[][..], &["notes"][..], &["hl"][..]] {
                // `in:` in the query and `--in` naming another corpus is
                // a refusal of its own, not this property.
                if rest.contains("in:") && !in_.is_empty() {
                    continue;
                }
                counts_are_search_totals(&fx, rest, months, in_);
                let f = facets(&fx, rest, in_).facets;
                values += f.theme.items.len() + f.entity.items.len() + f.month.items.len();
            }
        }
    }
    assert!(values > 500, "{values}");

    // The numbers themselves, once: a month picked, the months facet keeps
    // every month of the rest, the others narrow to the pick.
    let f = facets(&fx, "theme:Maths month:2026-02", &[]).facets;
    let months: Vec<(&str, usize)> = f
        .month
        .items
        .iter()
        .map(|i| (i.value.as_str(), i.count))
        .collect();
    assert_eq!(months, [("2025-12", 1), ("2026-02", 2), ("2026-03", 3)]);
    let themes: Vec<(&str, usize)> = f
        .theme
        .items
        .iter()
        .map(|i| (i.value.as_str(), i.count))
        .collect();
    assert_eq!(themes, [("Maths", 2), ("Coding", 1)]);
}

/// The reported case: with `ai:true` in the query the facet still says
/// how many documents `ai:false` would give, and search gives that many.
#[test]
fn the_ai_facet_counts_each_value_as_the_search_that_picks_it() {
    let fx = fixture();
    let open = facets(&fx, "", &[]).facets.ai;
    assert!(open.yes > 0 && open.no > 0, "{open:?}");
    for q in [
        "ai:true",
        "ai:false",
        "ai:true ai:false",
        "ai:false ai:true",
    ] {
        let f = facets(&fx, q, &[]);
        assert_eq!(f.facets.ai, open, "{q}");
        assert_eq!(f.facets.ai.yes, total(&fx, &format!("{q} ai:true"), &[]));
        assert_eq!(f.facets.ai.no, total(&fx, &format!("{q} ai:false"), &[]));
    }
    // The totals and the other facets stay the whole query's.
    let f = facets(&fx, "ai:true", &[]);
    assert_eq!(f.total_documents, total(&fx, "ai:true", &[]));
    assert_eq!(f.total_documents, open.yes);
    assert_eq!(total(&fx, "ai:true ai:false", &[]), open.no);
    // With months picked too: each facet is counted without its own field
    // and with the other's terms.
    let f = facets(&fx, "ai:true month:2026-02", &[]);
    assert_eq!(f.facets.ai.no, total(&fx, "month:2026-02 ai:false", &[]));
    assert_eq!(f.facets.ai.yes, total(&fx, "month:2026-02 ai:true", &[]));
    for it in &f.facets.month.items {
        let alone = total(&fx, &format!("ai:true month:{}", it.value), &[]);
        assert_eq!(it.count, alone, "month {}", it.value);
    }
}

#[test]
fn facets_on_an_empty_query_count_every_tagged_document() {
    let fx = fixture();
    let all = facets(&fx, "", &[]);
    // notes: n1 n2 n3 n4 n5 n7 n9 tagged (n6, n8 not); hl: four.
    assert_eq!((all.total_documents, all.tagged_documents), (11, 11));
    assert_eq!(all.corpora, ["notes", "hl"]);
    let (notes, hl) = (facets(&fx, "in:notes", &[]), facets(&fx, "", &["hl"]));
    assert_eq!((notes.total_documents, notes.tagged_documents), (7, 7));
    assert_eq!((hl.total_documents, hl.tagged_documents), (4, 4));
    assert_eq!(facets(&fx, "  ", &["notes"]).total_documents, 7);
    // The sum over the corpora, and what `scout topics` counts.
    let topics_total = |in_: &[&str]| {
        fx.engine
            .topics(&TopicsQuery {
                in_: in_.iter().map(|s| s.to_string()).collect(),
                ..Default::default()
            })
            .unwrap()
            .body
    };
    assert_eq!(topics_total(&[]).tagged_documents, 11);
    for (one, in_) in [(&notes, "notes"), (&hl, "hl")] {
        let t = topics_total(&[in_]);
        assert_eq!(t.tagged_documents, one.tagged_documents);
        assert_eq!(t.items, one.facets.theme.items, "{in_}");
    }
    let by = |f: &api::FacetsResults| -> std::collections::BTreeMap<String, usize> {
        f.facets
            .theme
            .items
            .iter()
            .map(|i| (topics::fold(&i.value), i.count))
            .collect()
    };
    let mut sum = by(&notes);
    for (k, v) in by(&hl) {
        *sum.entry(k).or_default() += v;
    }
    assert_eq!(by(&all), sum);
    let months =
        |f: &api::FacetsResults| -> usize { f.facets.month.items.iter().map(|i| i.count).sum() };
    // n7 has tags and no month.
    assert_eq!((months(&all), months(&notes), months(&hl)), (10, 6, 4));
    assert_eq!(all.facets.ai.yes + all.facets.ai.no, 11);
    // The same documents `theme-months` counts without a query.
    let m = fx
        .engine
        .theme_months(&ThemeMonthsQuery::default())
        .unwrap()
        .body;
    assert_eq!(m.documents, 10);
    let listed: Vec<(String, usize)> = m
        .months
        .iter()
        .cloned()
        .zip(m.month_totals.iter().copied())
        .filter(|(_, n)| *n > 0)
        .collect();
    let facet: Vec<(String, usize)> = all
        .facets
        .month
        .items
        .iter()
        .map(|i| (i.value.clone(), i.count))
        .collect();
    assert_eq!(listed, facet);

    // Search itself still refuses an empty query and lists nothing for a
    // corpus alone, unless a date order is asked for.
    let empty = fx.engine.search(&SearchQuery::default());
    assert_eq!(format!("{:#}", empty.unwrap_err()), "empty query");
    assert_eq!(total(&fx, "in:notes", &[]), 0);
}

#[test]
fn entity_types_go_from_the_rows_and_corrections_to_hits_and_the_entity_facet() {
    let fx = fixture();
    assert_eq!(scout_corpus::index::ENGINE_VERSION, "scout-corpus/14");
    let r = search(&fx, "in:notes", &[], Sort::Oldest, 100);
    let info = |file: &str| -> TopicInfo {
        r.results
            .iter()
            .find(|d| d.rel_path == file)
            .and_then(|d| d.topics.clone())
            .unwrap_or_else(|| panic!("{file}"))
    };
    let types =
        |t: &TopicInfo| -> Vec<(String, String)> { t.entity_types.clone().into_iter().collect() };
    let pairs = |p: &[(&str, &str)]| -> Vec<(String, String)> {
        p.iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    };
    // From the row: by folded name; `entities` is still the names.
    let n1 = info("n1.md");
    assert_eq!(n1.entities, ["Tao", "Astra"]);
    assert_eq!(types(&n1), pairs(&[("astra", "model"), ("tao", "person")]));
    assert_eq!(n1.entity_type("ASTRA"), Some("model"));
    assert_eq!(n1.entity_type("Nobody"), None);
    // A numbered `entity_add` gives its type; an undone one gives nothing.
    let n5 = info("n5.md");
    assert_eq!(n5.entities, ["Ada"]);
    assert_eq!(types(&n5), pairs(&[("ada", "person")]));
    assert_eq!(n5.corrected, ["entities"]);
    // A name taken off has no type left.
    let n7 = info("n7.md");
    assert!(n7.entities.is_empty() && n7.entity_types.is_empty());
    // A whole list by hand: the kept name keeps its type, the new has none.
    let n2 = info("n2.md");
    assert_eq!(n2.entities, ["Astra", "New"]);
    assert_eq!(types(&n2), pairs(&[("astra", "product")]));
    // Unreadable types are passed over and the row stays; a correction's
    // type is the name's type also when the row has the name.
    let n9 = info("n9.md");
    assert_eq!(n9.entities, ["Delta", "Epsilon"]);
    assert_eq!(
        types(&n9),
        pairs(&[("delta", "event"), ("epsilon", "work")])
    );

    // The hit's JSON: `entities` an array of strings, the types beside it;
    // no `entity_types` key on an item without a typed entity.
    let as_json = |file: &str| serde_json::to_value(info(file)).unwrap();
    let j = as_json("n1.md");
    assert_eq!(j["entities"], json!(["Tao", "Astra"]));
    assert_eq!(
        j["entity_types"],
        json!({"astra": "model", "tao": "person"})
    );
    assert!(as_json("n7.md").get("entity_types").is_none());
    // Tags stored before types were read still read, and write back as
    // they were.
    let old = r#"{"key":"x:1","themes":[],"entities":["Tao"],"event":null,"event_name":null,"ai_related":null,"month":null,"confidence":null,"run_id":null,"prompt_version":null}"#;
    let t: TopicInfo = serde_json::from_str(old).unwrap();
    assert!(t.entity_types.is_empty());
    assert_eq!(serde_json::to_string(&t).unwrap(), old);

    // The facet: the most frequent type among the counted documents …
    let entity = |q: &str, in_: &[&str]| -> Vec<(String, usize, Option<String>)> {
        facets(&fx, q, in_)
            .facets
            .entity
            .items
            .into_iter()
            .map(|i| (i.value, i.count, i.kind))
            .collect()
    };
    let row = |v: &str, n: usize, k: Option<&str>| (v.to_string(), n, k.map(String::from));
    assert_eq!(
        entity("", &["notes"]),
        [
            row("Astra", 3, Some("model")),
            row("Tao", 3, Some("person")),
            row("Ada", 1, Some("person")),
            row("Delta", 1, Some("event")),
            row("Epsilon", 1, Some("work")),
            row("New", 1, None),
        ]
    );
    // … within the result: here Astra is a product once and a model once,
    // Tao a person once and an organisation once: the first in name order.
    assert_eq!(
        entity("agents month:2026-02 month:2025-12", &["notes"]),
        [
            row("Astra", 3, Some("model")),
            row("Tao", 2, Some("org")),
            row("New", 1, None)
        ]
    );
    assert_eq!(
        entity("theme:Coding -proof", &["notes"]),
        [row("Astra", 1, Some("product")), row("New", 1, None)]
    );
    // Across corpora a name is one value, typed by all its documents.
    assert_eq!(entity("", &[])[0], row("Tao", 7, Some("person")));
    // As JSON: `type` on a typed value, no key on an untyped one, and the
    // other facets' values as they were.
    let f = serde_json::to_value(facets(&fx, "theme:Coding -proof", &["notes"]).facets).unwrap();
    assert_eq!(
        f["entity"]["items"],
        json!([{"value": "Astra", "count": 1, "type": "product"}, {"value": "New", "count": 1}])
    );
    assert_eq!(
        f["theme"]["items"],
        json!([{"value": "Coding", "count": 1}])
    );
    // `scout topics entities` carries it too.
    let t = fx
        .engine
        .topics(&TopicsQuery {
            field: "entities".into(),
            in_: vec!["notes".into()],
            top: 2,
            ..Default::default()
        })
        .unwrap()
        .body;
    assert_eq!(
        t.items
            .iter()
            .map(|i| i.kind.as_deref())
            .collect::<Vec<_>>(),
        [Some("model"), Some("person")]
    );
}

#[test]
fn newest_and_oldest_order_by_the_document_date_with_undated_last_and_ties_by_path() {
    let fx = fixture();
    let listed = |q: &str, sort: Sort, limit: usize| search(&fx, q, &["notes"], sort, limit);
    // Every document of the corpus, without a term or a filter. n1 and n2
    // share a day (path order in both directions); n4 and n9 share a day
    // and differ by the hour; n3 has a year (its first day); n5 and n6
    // have no date and n8 one that is no date: last, by path, both ways.
    let newest = listed("", Sort::Newest, 100);
    assert_eq!(
        keys(&newest),
        [
            "notes:n1", "notes:n2", "notes:n3", "notes:n4", "notes:n9", "notes:n7", "notes:n5",
            "notes:n6", "notes:n8"
        ]
    );
    let oldest = listed("in:notes", Sort::Oldest, 100);
    assert_eq!(
        keys(&oldest),
        [
            "notes:n7", "notes:n9", "notes:n4", "notes:n3", "notes:n1", "notes:n2", "notes:n5",
            "notes:n6", "notes:n8"
        ]
    );
    assert_eq!((newest.total_documents, newest.total_passages), (9, 9));
    // Each result carries what a list shows: title, author, date.
    let d = &newest.results[3];
    assert_eq!(
        (
            d.title.as_str(),
            d.author.as_deref(),
            d.date.as_deref(),
            d.date_display.as_deref()
        ),
        (
            "n4.md",
            Some("A"),
            Some("2025-12-31T10:00:00Z"),
            Some("31 December 2025")
        )
    );
    assert_eq!(newest.results[6].date, None);
    // The reply says which order it is in; relevance says nothing.
    assert_eq!(serde_json::to_value(&newest).unwrap()["sort"], "newest");
    assert_eq!(serde_json::to_value(&oldest).unwrap()["sort"], "oldest");

    // Filters alone, and paging by `limit`: the first of the same order.
    assert_eq!(
        keys(&listed("theme:Maths", Sort::Newest, 100)),
        ["notes:n1", "notes:n3", "notes:n4", "notes:n5"]
    );
    let page = listed("theme:Maths", Sort::Oldest, 2);
    assert_eq!(keys(&page), ["notes:n4", "notes:n3"]);
    assert_eq!(page.total_documents, 4);
    // With a term: the same documents as by relevance, in date order.
    let by_date = listed("proof", Sort::Newest, 100);
    let by_score = listed("proof", Sort::Relevance, 100);
    assert_eq!(
        keys(&by_date),
        ["notes:n1", "notes:n3", "notes:n4", "notes:n9", "notes:n7", "notes:n5", "notes:n8"]
    );
    let sorted = |r: &SearchResults| {
        let mut k = keys(r);
        k.sort();
        k
    };
    assert_eq!(sorted(&by_date), sorted(&by_score));
    assert_eq!(by_date.total_passages, by_score.total_passages);
    // Scores are the relevance search's own, whatever the order.
    for d in &by_date.results {
        let same = by_score
            .results
            .iter()
            .find(|r| r.rel_path == d.rel_path)
            .unwrap();
        assert_eq!((d.score, d.rank), (same.score, same.rank), "{}", d.rel_path);
    }

    // Several corpora: one order over them all, ties by corpus then path.
    let both = search(&fx, "", &[], Sort::Newest, 100);
    assert_eq!(
        keys(&both),
        [
            "hl:w-4", "hl:w-3", "hl:w-2", "notes:n1", "notes:n2", "hl:w-1", "notes:n3", "notes:n4",
            "notes:n9", "notes:n7", "notes:n5", "notes:n6", "notes:n8"
        ]
    );
    assert_eq!(
        keys(&search(&fx, "", &[], Sort::Oldest, 3)),
        ["notes:n7", "notes:n9", "notes:n4"]
    );
    assert_eq!(both.total_documents, 13);

    // A date order is a full-text search.
    for mode in [SearchMode::Semantic, SearchMode::Hybrid] {
        let q = SearchQuery {
            query: "proof".into(),
            sort: Sort::Newest,
            mode,
            ..Default::default()
        };
        let err = format!("{:#}", fx.engine.search(&q).unwrap_err());
        assert!(
            err.contains("does not combine with semantic or hybrid"),
            "{err}"
        );
    }
}

#[test]
fn relevance_is_the_default_and_its_reply_is_what_it_was() {
    let fx = fixture();
    // The default of the request, of its JSON and of the engine's own.
    assert_eq!(SearchQuery::default().sort, Sort::Relevance);
    assert_eq!(scout_corpus::SearchRequest::new("x").sort, Sort::Relevance);
    let q: SearchQuery = serde_json::from_str(r#"{"query": "proof"}"#).unwrap();
    assert_eq!(q.sort, Sort::Relevance);
    let q: SearchQuery = serde_json::from_str(r#"{"query": "proof", "sort": "newest"}"#).unwrap();
    assert_eq!(q.sort, Sort::Newest);
    assert!(serde_json::from_str::<SearchQuery>(r#"{"sort": "latest"}"#).is_err());
    assert_eq!("oldest".parse::<Sort>().unwrap(), Sort::Oldest);
    assert!("Newest".parse::<Sort>().is_err());

    // A relevance reply has no `sort` key, and is the same bytes whether
    // the order is left out or named.
    for query in [
        "proof",
        "agents OR again",
        "theme:Maths",
        "proof theme:Coding in:notes",
        "in:notes",
    ] {
        let named = fx
            .engine
            .search(&SearchQuery {
                query: query.into(),
                sort: Sort::Relevance,
                ..Default::default()
            })
            .unwrap()
            .body;
        let left_out: SearchQuery = serde_json::from_value(json!({"query": query})).unwrap();
        let left_out = fx.engine.search(&left_out).unwrap().body;
        let text = api::to_json(&named).unwrap();
        assert_eq!(text, api::to_json(&left_out).unwrap(), "{query}");
        assert!(serde_json::to_value(&named).unwrap().get("sort").is_none());
        // Relevance order: rank, then score, corpus, path.
        let order: Vec<(f64, f64, &str, &str)> = named
            .results
            .iter()
            .map(|d| (-d.rank, -d.score, d.corpus.as_str(), d.rel_path.as_str()))
            .collect();
        let mut want = order.clone();
        want.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(order, want, "{query}");
    }
    // A filter alone by relevance: path order, score 0, as before.
    let r = search(&fx, "theme:Maths", &["notes"], Sort::Relevance, 100);
    assert_eq!(keys(&r), ["notes:n1", "notes:n3", "notes:n4", "notes:n5"]);
    assert!(r.results.iter().all(|d| d.score == 0.0 && d.rank == 0.0));
}

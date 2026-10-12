//! The read-only additions of engine `scout-corpus/14` (an event's page in
//! the desktop app reads them):
//!
//! - `EventEditor::merges` and the `merge_id` of a listed `merged_from`
//!   entry: direct and chained merges, a record merged by hand (no log
//!   line), an undone merge;
//! - `own_aliases`, `month_spans`, `undated_items` in `scout events`, each
//!   absent where it has nothing to say;
//! - records without items, with their `origin` and `edited`;
//! - an event's other events (the event facet of `event:<id>`);
//! - `mentioned_works`: the hit's `topics.mentioned_works` and the `works`
//!   facet, a key named as a document where the index has it;
//! - a corpus with none of this lists, counts and searches with no new
//!   key;
//! - with `SCOUT_SEED_REAL_ARCHIVE`: the real archive's merges and works,
//!   from a scratch index (the archive is only read).

mod event_edit_support;
use event_edit_support::*;
use scout_corpus::api::{EventsQuery, FacetsQuery, SearchQuery, Sort};
use scout_corpus::registry::{CorpusConfig, CorpusKind, DocumentUnit, FieldMap};
use scout_corpus::{topics, Corpus, Engine, Registry};

fn config(root: &Path) -> CorpusConfig {
    CorpusConfig {
        id: "highlights".into(),
        name: "Highlights".into(),
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
    }
}

struct Indexed {
    fx: Fx,
    _data: tempfile::TempDir,
    corpus: Corpus,
    engine: Engine,
}

/// A work of key `x:<n>`; `saved` is the day of its one highlight (none:
/// the document has no date).
fn work(root: &Path, n: usize, saved: Option<&str>) {
    let p = root.join(format!("readings/works/w-{n}.md"));
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    let saved = saved
        .map(|d| format!("highlighted_at: {d}\n\n"))
        .unwrap_or_default();
    fs::write(
        p,
        format!("---\ntitle: W{n}\nauthor: A\ntype: tweet\nsource_system: x\nsource_id: \"{n}\"\n---\n\n> A proof, number {n}.\n\n{saved}---\n"),
    )
    .unwrap();
}

/// The fixture's four records over `rows` (one work per row, keyed
/// `x:<n>` by position from 1; `saved` gives each work's highlight day).
fn indexed(rows: &[Value], saved: &[Option<&str>]) -> Indexed {
    let fx = fixture();
    let data = tempfile::tempdir().unwrap();
    let root = fx.root();
    let mut layer = String::new();
    for (i, row) in rows.iter().enumerate() {
        work(root, i + 1, saved[i]);
        layer.push_str(&format!("{row}\n"));
    }
    let file = root.join(topics::LAYER_FILE);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, layer).unwrap();
    let corpus = Corpus::from_config(config(root), data.path());
    let engine = Engine::new(
        Registry {
            corpora: vec![config(root)],
        },
        data.path(),
    );
    Indexed {
        fx,
        _data: data,
        corpus,
        engine,
    }
}

fn row(n: usize, date: Option<&str>, event: &str) -> Value {
    let mut v = json!({"key": format!("x:{n}"), "themes": ["T"], "entities": [], "event": event});
    if let Some(d) = date {
        v["date"] = json!(d);
    }
    v
}

/// Seven works. Alpha: 1 and 2 (September days), 6 (no month at all) and
/// 7 (a layer month, no day); 2 is in beta too. Beta: 3. Delta: 5. Gamma
/// (his, edited) has none. Works 1 and 2 mention work 3; work 1 also a key
/// no document has.
fn page() -> Indexed {
    let mut rows = vec![
        row(1, Some("2026-09-17"), "2026-09/E01"),
        row(2, Some("2026-09-06"), "2026-09/E01"),
        row(3, Some("2026-10-03"), "2026-10/E04"),
        row(4, Some("2026-12-01"), "2026-12/E09"),
        row(5, Some("2026-08-01"), "2026-08/E07"),
        row(6, None, "2026-09/E01"),
        row(7, None, "2026-09/E01"),
    ];
    rows[0]["mentioned_works"] = json!(["x:3", "rw:gone"]);
    rows[1]["mentioned_works"] = json!(["X:3"]);
    rows[1]["events"] = json!(["2026-10/E04"]);
    rows[6]["month"] = json!("2026-09");
    let saved = [
        Some("2026-09-01"),
        Some("2026-09-02"),
        Some("2026-09-03"),
        Some("2026-09-04"),
        Some("2026-09-05"),
        // Work 6 has a document date and no layer month; work 7 the
        // other way round.
        Some("2026-01-01"),
        None,
    ];
    indexed(&rows, &saved)
}

impl Indexed {
    fn build(&self) {
        self.corpus.build_index(false).unwrap();
    }

    fn listing(&self, merged: bool) -> Value {
        self.build();
        let q = EventsQuery {
            min_items: 0,
            merged,
            ..Default::default()
        };
        serde_json::to_value(self.engine.events(&q).unwrap().body).unwrap()
    }

    fn row(&self, id: &str) -> Value {
        self.listing(true)["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["id"] == id)
            .unwrap_or_else(|| panic!("{id} is not listed"))
            .clone()
    }

    fn merge(&self, a: &str, b: &str, reason: Option<&str>) -> String {
        let mut req = merge_req(a, b, By::Dominik);
        req.reason = reason.map(String::from);
        self.fx.editor().merge(&req).unwrap().log_id
    }

    fn facets(&self, query: &str) -> Value {
        let q = FacetsQuery {
            query: query.into(),
            top: 0,
            ..Default::default()
        };
        serde_json::to_value(self.engine.facets(&q).unwrap().body).unwrap()
    }

    fn search(&self, query: &str, sort: Sort) -> Value {
        let q = SearchQuery {
            query: query.into(),
            sort,
            limit: 50,
            ..Default::default()
        };
        serde_json::to_value(self.engine.search(&q).unwrap().body).unwrap()
    }
}

/// `(id, a, b, undone_by)` of each listed merge.
fn brief(
    l: &[scout_corpus::event_edit::MergeInfo],
) -> Vec<(String, String, String, Option<String>)> {
    l.iter()
        .map(|m| {
            (
                m.id.clone(),
                m.a.id.clone(),
                m.b.id.clone(),
                m.undone_by.clone(),
            )
        })
        .collect()
}

#[test]
fn merges_lists_the_lines_in_effect_and_the_listing_names_each_one() {
    let ix = page();
    let e = ix.fx.editor();
    let start = ix.fx.tree();
    assert_eq!(e.merges().unwrap(), vec![]);
    assert_eq!(e.merges_all().unwrap(), vec![]);
    // No merged record: the listing has no `merged_from`, and reads no log.
    assert!(ix.row(ALPHA).get("merged_from").is_none());

    // A chain (delta into beta, beta into alpha), then one undone.
    let inner = ix.merge(DELTA, BETA, Some("one story"));
    let outer = ix.merge(BETA, ALPHA, None);
    let live = e.merges().unwrap();
    let s = |v: &[(&str, &str, &str, Option<&str>)]| {
        v.iter()
            .map(|(i, a, b, u)| {
                (
                    i.to_string(),
                    a.to_string(),
                    b.to_string(),
                    u.map(String::from),
                )
            })
            .collect::<Vec<_>>()
    };
    // Newest first.
    assert_eq!(
        brief(&live),
        s(&[(&outer, BETA, ALPHA, None), (&inner, DELTA, BETA, None)])
    );
    assert_eq!(
        serde_json::to_value(&live).unwrap(),
        json!([
            {"id": outer, "at": NOW, "by": "dominik", "reason": null,
             "a": {"id": BETA, "name": "Beta follow-up"}, "b": {"id": ALPHA, "name": "Alpha story"}},
            {"id": inner, "at": NOW, "by": "dominik", "reason": "one story",
             "a": {"id": DELTA, "name": "Delta"}, "b": {"id": BETA, "name": "Beta follow-up"}},
        ])
    );
    assert_eq!(e.merges_all().unwrap(), live);
    // Each record merged in, direct or through the chain, with the line
    // that merged it: undoing delta's takes delta out of alpha.
    assert_eq!(
        ix.row(ALPHA)["merged_from"],
        json!([
            {"id": DELTA, "name": "Delta", "merge_id": inner},
            {"id": BETA, "name": "Beta follow-up", "merge_id": outer},
        ])
    );

    // A record merged by hand: no line, so no `merge_id`, and `merges`
    // does not list it (`check` names it as information).
    let mut gamma = ix.fx.record(GAMMA);
    gamma.merged_into = Some(ALPHA.into());
    put(&ix.fx.dir, &gamma);
    assert_eq!(e.merges().unwrap(), live);
    let from = ix.row(ALPHA)["merged_from"].clone();
    assert_eq!(
        from,
        json!([
            {"id": DELTA, "name": "Delta", "merge_id": inner},
            {"id": BETA, "name": "Beta follow-up", "merge_id": outer},
            {"id": GAMMA, "name": "Gamma"},
        ])
    );
    let check = e.check().unwrap();
    assert_eq!((check.in_effect, check.information.len()), (2, 1));

    // Undone: out of `merges`, in `merges_all` with its undo line.
    let undo = e.unmerge(&inner).unwrap().log_id;
    assert_eq!(
        brief(&e.merges().unwrap()),
        s(&[(&outer, BETA, ALPHA, None)])
    );
    let all = e.merges_all().unwrap();
    assert_eq!(
        brief(&all),
        s(&[
            (&outer, BETA, ALPHA, None),
            (&inner, DELTA, BETA, Some(&undo))
        ])
    );
    assert_eq!(all[1].undone_at.as_deref(), Some(NOW));
    assert_eq!(all[0].undone_at, None);
    let v = serde_json::to_value(&all).unwrap();
    assert!(v[0].get("undone_by").is_none() && v[0].get("undone_at").is_none());
    assert_eq!(
        (&v[1]["undone_by"], &v[1]["undone_at"]),
        (&json!(undo), &json!(NOW))
    );
    assert_eq!(
        ix.row(ALPHA)["merged_from"],
        json!([
            {"id": BETA, "name": "Beta follow-up", "merge_id": outer},
            {"id": GAMMA, "name": "Gamma"},
        ])
    );
    // The line the listing names is the one `unmerge` takes.
    e.unmerge(&outer).unwrap();
    assert_eq!(e.merges().unwrap(), vec![]);
    assert_eq!(
        ix.row(ALPHA)["merged_from"],
        json!([{"id": GAMMA, "name": "Gamma"}])
    );

    // A line logged and never applied is no merge: not listed.
    let cut = ix
        .fx
        .editor()
        .with_hook(|s| match s {
            Step::Written(f) if f == MERGE_LOG => Err(std::io::Error::other("stopped")),
            _ => Ok(()),
        })
        .merge(&merge_req(DELTA, ALPHA, By::Dominik));
    assert!(cut.is_err());
    assert_eq!(e.merges().unwrap(), vec![]);
    assert_eq!(e.merges_all().unwrap().len(), 2);
    assert_eq!(e.check().unwrap().problems[0].kind, "not-applied");

    // Reading wrote nothing: only the edits above changed the folder.
    gamma.merged_into = None;
    put(&ix.fx.dir, &gamma);
    let mut after = ix.fx.tree();
    assert!(after.remove(&ix.fx.dir.join(MERGE_LOG)).is_some());
    assert!(after == start);
}

#[test]
fn a_listing_made_from_a_stale_index_names_only_lines_still_in_effect() {
    let ix = page();
    let m = ix.merge(BETA, ALPHA, None);
    assert_eq!(ix.row(ALPHA)["merged_from"][0]["merge_id"], json!(m));
    // Undone in the folder, the index not rebuilt: the index still shows
    // beta merged in, and the log has no line in effect for it.
    ix.fx.editor().unmerge(&m).unwrap();
    let q = EventsQuery::default();
    let stale = serde_json::to_value(ix.engine.events(&q).unwrap().body).unwrap();
    let alpha = stale["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == ALPHA)
        .unwrap();
    assert_eq!(
        alpha["merged_from"],
        json!([{"id": BETA, "name": "Beta follow-up"}])
    );

    // A log that cannot be read costs the ids and a note, not the listing.
    let m = ix.merge(BETA, ALPHA, None);
    ix.build();
    let log = ix.fx.dir.join(MERGE_LOG);
    let good = fs::read(&log).unwrap();
    fs::write(&log, b"not json\n").unwrap();
    let reply = ix.engine.events(&q).unwrap();
    assert!(
        reply
            .notes
            .iter()
            .any(|n| n.contains("merge log was not read")),
        "{:?}",
        reply.notes
    );
    let alpha = reply.body.events.iter().find(|x| x.id == ALPHA).unwrap();
    assert_eq!(alpha.merged_from.len(), 1);
    assert_eq!(alpha.merged_from[0].merge_id, None);
    fs::write(&log, good).unwrap();
    let reply = ix.engine.events(&q).unwrap();
    assert!(reply.notes.is_empty(), "{:?}", reply.notes);
    let alpha = reply.body.events.iter().find(|x| x.id == ALPHA).unwrap();
    assert_eq!(alpha.merged_from[0].merge_id.as_deref(), Some(m.as_str()));
}

#[test]
fn own_aliases_are_given_only_where_the_combined_list_differs() {
    let ix = page();
    // Nothing merged in: `aliases` are the record's own.
    let alone = ix.row(ALPHA);
    assert_eq!(alone["aliases"], json!(["The Alpha", "Shared"]));
    assert!(alone.get("own_aliases").is_none());
    let m = ix.merge(BETA, ALPHA, None);
    let both = ix.row(ALPHA);
    assert_eq!(
        (&both["aliases"], &both["own_aliases"]),
        (
            &json!(["The Alpha", "Shared", "Beta follow-up", "Beta"]),
            &json!(["The Alpha", "Shared"])
        )
    );
    // A record merged away is listed as itself.
    assert!(ix.row(BETA).get("own_aliases").is_none());
    // Each own alias is one `set --remove-alias` removes; a merged-in one
    // is not.
    let e = ix.fx.editor();
    let remove = |alias: &str| {
        e.set(
            ALPHA,
            &FieldEdit {
                remove_aliases: strs(&[alias]),
                ..FieldEdit::default()
            },
        )
    };
    assert!(remove("Beta").is_err());
    remove("Shared").unwrap();
    remove("The Alpha").unwrap();
    // A survivor with no alias of its own: an empty list, not an absent key.
    let row = ix.row(ALPHA);
    assert_eq!(
        (&row["aliases"], &row["own_aliases"]),
        (&json!(["Beta follow-up", "Beta", "shared"]), &json!([]))
    );
    e.unmerge(&m).unwrap();
    assert!(ix.row(ALPHA).get("own_aliases").is_none());
}

#[test]
fn each_month_has_its_first_and_last_day_and_items_without_a_month_are_counted() {
    let ix = page();
    let alpha = ix.row(ALPHA);
    // Four items: two September days, one of September without a day, one
    // without a month.
    assert_eq!(alpha["items"], json!(4));
    assert_eq!(alpha["months"], json!([{"month": "2026-09", "count": 3}]));
    assert_eq!(
        alpha["month_spans"],
        json!({"2026-09": {"first": "2026-09-06", "last": "2026-09-17"}})
    );
    assert_eq!(alpha["undated_items"], json!(1));
    // One item: first and last are its day. No undated item: no key.
    let delta = ix.row(DELTA);
    assert_eq!(
        delta["month_spans"],
        json!({"2026-08": {"first": "2026-08-01", "last": "2026-08-01"}})
    );
    assert!(delta.get("undated_items").is_none());
    // Merged in: the months of all, each with its own days.
    ix.merge(BETA, ALPHA, None);
    let alpha = ix.row(ALPHA);
    assert_eq!(
        alpha["months"],
        json!([{"month": "2026-09", "count": 3}, {"month": "2026-10", "count": 1}])
    );
    assert_eq!(
        alpha["month_spans"],
        json!({
            "2026-09": {"first": "2026-09-06", "last": "2026-09-17"},
            "2026-10": {"first": "2026-10-03", "last": "2026-10-03"},
        })
    );
    assert_eq!(
        (&alpha["items"], &alpha["undated_items"]),
        (&json!(5), &json!(1))
    );

    // The undated item is reached by `event:<id>`, which lists every item
    // (`items`), and told by its null `topics.month`; no `month:` value
    // selects it. Date order is the document's own date, not the layer's:
    // work 6 (no layer month, saved in January) is first when oldest
    // first, and work 7 (no document date) is last in both orders.
    let titles = |v: &Value| -> Vec<String> {
        v["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["title"].as_str().unwrap().to_string())
            .collect()
    };
    let oldest = ix.search(&format!("event:{ALPHA}"), Sort::Oldest);
    assert_eq!(oldest["total_documents"], json!(5));
    assert_eq!(titles(&oldest), ["W6", "W1", "W2", "W3", "W7"]);
    let newest = ix.search(&format!("event:{ALPHA}"), Sort::Newest);
    assert_eq!(titles(&newest), ["W3", "W2", "W1", "W6", "W7"]);
    let no_month: Vec<&str> = oldest["results"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["topics"]["month"].is_null())
        .map(|r| r["title"].as_str().unwrap())
        .collect();
    assert_eq!(no_month, ["W6"]);
    assert_eq!(
        ix.search(&format!("event:{ALPHA} month:2026"), Sort::Oldest)["total_documents"],
        json!(4)
    );
}

#[test]
fn a_record_without_items_is_listed_from_min_items_0_with_its_origin() {
    let ix = page();
    ix.build();
    let list = |min_items: usize, month: Option<&str>| -> Vec<(String, usize)> {
        let q = EventsQuery {
            min_items,
            month: month.map(String::from),
            ..Default::default()
        };
        let body = ix.engine.events(&q).unwrap().body;
        body.events.into_iter().map(|e| (e.id, e.items)).collect()
    };
    let s = |l: &[(&str, usize)]| {
        l.iter()
            .map(|(id, n)| (id.to_string(), *n))
            .collect::<Vec<_>>()
    };
    // Work 4's monthly event has no record: listed under its monthly id.
    let with_items = s(&[(ALPHA, 4), (BETA, 2), (DELTA, 1), ("2026-12/E09", 1)]);
    assert_eq!(list(1, None), with_items);
    let mut all = with_items.clone();
    all.push((GAMMA.to_string(), 0));
    assert_eq!(list(0, None), all);
    // His record, kept visible by what the row says of it.
    let gamma = ix.row(GAMMA);
    assert_eq!(
        (
            &gamma["origin"],
            &gamma["edited"],
            &gamma["items"],
            &gamma["record"]
        ),
        (
            &json!("dominik"),
            &json!("2026-10-10"),
            &json!(0),
            &json!(true)
        )
    );
    assert_eq!(ix.row(ALPHA)["origin"], json!("model"));
    for absent in ["month_spans", "undated_items", "own_aliases", "merged_from"] {
        assert!(gamma.get(absent).is_none(), "{absent}");
    }
    // `min_items` counts an event's items in the whole corpus, whatever
    // `month` is: alpha's four pass a floor of 4 though three are in the
    // month. With `month` an event without an item there is not listed,
    // so a record without items never is.
    assert_eq!(list(4, Some("2026-09")), s(&[(ALPHA, 4)]));
    assert_eq!(list(0, Some("2026-09")), s(&[(ALPHA, 4), (BETA, 2)]));
    assert_eq!(list(0, Some("2026-11")), s(&[]));
}

#[test]
fn the_event_facet_of_an_event_lists_the_event_and_the_events_sharing_items() {
    let ix = page();
    ix.build();
    let events = |q: &str| ix.facets(q)["facets"]["event"].clone();
    // The event itself, with all its items, then each other event with
    // the items the two share.
    assert_eq!(
        events(&format!("event:{ALPHA}")),
        json!({"distinct": 2, "items": [
            {"value": ALPHA, "count": 4, "name": "Alpha story"},
            {"value": BETA, "count": 1, "name": "Beta follow-up"},
        ]})
    );
    assert_eq!(
        events(&format!("event:{BETA}")),
        json!({"distinct": 2, "items": [
            {"value": BETA, "count": 2, "name": "Beta follow-up"},
            {"value": ALPHA, "count": 1, "name": "Alpha story"},
        ]})
    );
    // An event that shares no item: itself alone.
    assert_eq!(
        events(&format!("event:{DELTA}")),
        json!({"distinct": 1, "items": [{"value": DELTA, "count": 1, "name": "Delta"}]})
    );
}

#[test]
fn mentioned_works_are_on_the_hit_and_counted_as_a_facet() {
    let ix = page();
    ix.build();
    let hits = ix.search(&format!("event:{ALPHA}"), Sort::Oldest);
    let topics = |title: &str| -> Value {
        hits["results"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["title"] == title)
            .unwrap()["topics"]
            .clone()
    };
    // As written, in the row's order.
    assert_eq!(topics("W1")["mentioned_works"], json!(["x:3", "rw:gone"]));
    assert_eq!(topics("W2")["mentioned_works"], json!(["X:3"]));
    assert!(topics("W6").get("mentioned_works").is_none());
    assert!(topics("W1").get("day").is_none());

    // Counted over the query's documents: a key under the fold, with the
    // documents mentioning it (the value shown is its most frequent
    // spelling, of equals the first); a key that is a document of the
    // corpus names it, by the spelling the document has (`key`); one that
    // is none stands alone.
    let w3 = ix.fx.root().join("readings/works/w-3.md");
    let all = ix.facets("");
    assert_eq!(
        all["facets"]["works"],
        json!({"distinct": 2, "items": [
            {"value": "X:3", "count": 2, "corpus": "highlights", "key": "x:3",
             "title": "W3", "rel_path": "readings/works/w-3.md",
             "path": w3.display().to_string()},
            {"value": "rw:gone", "count": 1},
        ]})
    );
    // Narrowed by the query like any facet counted under the whole query.
    let beta = ix.facets(&format!("event:{BETA}"));
    assert_eq!(beta["facets"]["works"]["items"][0]["count"], json!(1));
    assert_eq!(beta["facets"]["works"]["distinct"], json!(1));
    // No counted document mentions a work: no key.
    assert!(ix.facets(&format!("event:{DELTA}"))["facets"]
        .get("works")
        .is_none());
    // `top` cuts it like themes; no query field selects by it.
    let q = FacetsQuery {
        top: 1,
        ..Default::default()
    };
    let top = ix.engine.facets(&q).unwrap().body.facets.works.unwrap();
    assert_eq!((top.distinct, top.items.len()), (2, 1));
    let unknown = ix.engine.search(&SearchQuery {
        query: "work:x:3".into(),
        ..Default::default()
    });
    assert_eq!(unknown.map(|r| r.body.total_documents).unwrap_or(0), 0);
}

/// One listed event as `scout events` wrote it before engine 14.
#[derive(serde::Serialize)]
struct Before {
    corpus: String,
    id: String,
    name: Option<String>,
    record: bool,
    start: Option<String>,
    end: Option<String>,
    items: usize,
    months: Vec<scout_corpus::api::MonthCount>,
    members: Vec<String>,
    aliases: Vec<String>,
    themes: Vec<String>,
    origin: Option<String>,
    edited: Option<String>,
    merged_into: Option<String>,
    description: String,
}

/// Every key this version adds, anywhere in a reply, as JSON writes it.
const NEW_KEYS: [&str; 11] = [
    "\"merge_id\":",
    "\"own_aliases\":",
    "\"month_spans\":",
    "\"undated_items\":",
    "\"mentioned_works\":",
    "\"works\":",
    "\"day\":",
    "\"undone_by\":",
    "\"undone_at\":",
    "\"first\":",
    "\"last\":",
];

#[test]
fn a_corpus_with_none_of_it_lists_counts_and_searches_with_no_new_key() {
    // Rows as before the tagger's v3: no `mentioned_works`; a layer
    // month and no day; every item with a month; nothing merged.
    let rows: Vec<Value> = [
        (1, "2026-09", "2026-09/E01"),
        (2, "2026-09", "2026-09/E01"),
        (3, "2026-10", "2026-10/E04"),
    ]
    .iter()
    .map(|(n, month, event)| {
        let mut r = row(*n, None, event);
        r["month"] = json!(month);
        r["entities"] = json!(["Tao"]);
        r["entities_typed"] = json!([{"name": "Tao", "type": "person"}]);
        r
    })
    .collect();
    let ix = indexed(&rows, &[Some("2026-09-01"); 3]);
    ix.build();
    let q = EventsQuery {
        min_items: 0,
        merged: true,
        ..Default::default()
    };
    let body = ix.engine.events(&q).unwrap().body;
    let now = serde_json::to_string(&body.events).unwrap();
    let set = ix.fx.set();
    let before: Vec<Before> = body
        .events
        .iter()
        .map(|l| {
            let r = set.records[l.id.as_str()].clone();
            Before {
                corpus: "highlights".into(),
                id: r.id,
                name: Some(r.name),
                record: true,
                start: r.start,
                end: r.end,
                items: l.items,
                months: l.months.clone(),
                members: r.members,
                aliases: r.aliases,
                themes: r.themes,
                origin: r.origin,
                edited: r.edited,
                merged_into: r.merged_into,
                description: r.description,
            }
        })
        .collect();
    assert_eq!(before.len(), 4);
    assert_eq!(now, serde_json::to_string(&before).unwrap());

    // Facets: the five facets there were, each item with the keys it had.
    let facets = ix.facets("");
    let keys = |v: &Value| v.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
    assert_eq!(
        keys(&facets["facets"]),
        ["ai", "entity", "event", "month", "theme"]
    );
    assert_eq!(
        facets["facets"]["entity"]["items"],
        json!([{"value": "Tao", "count": 3, "type": "person"}])
    );
    assert_eq!(
        facets["facets"]["event"]["items"][0],
        json!({"value": ALPHA, "count": 2, "name": "Alpha story"})
    );
    // A search hit's topics: the keys they had.
    let hits = ix.search("proof", Sort::Relevance);
    let topics = &hits["results"][0]["topics"];
    assert_eq!(
        keys(topics),
        [
            "ai_related",
            "confidence",
            "entities",
            "entity_types",
            "event",
            "event_name",
            "events",
            "key",
            "month",
            "prompt_version",
            "run_id",
            "themes"
        ]
    );
    let merges = serde_json::to_string(&ix.fx.editor().merge_list(false).unwrap()).unwrap();
    assert_eq!(
        merges,
        format!(
            r#"{{"tool":"scout events merges","at":"{NOW}","dir":{},"merges":[]}}"#,
            json!(ix.fx.editor().dir().display().to_string())
        )
    );
    for text in [
        now,
        serde_json::to_string(&facets).unwrap(),
        serde_json::to_string(&hits).unwrap(),
        serde_json::to_string(&ix.search("", Sort::Oldest)).unwrap(),
    ] {
        for key in NEW_KEYS {
            assert!(!text.contains(key), "{key} in {text}");
        }
    }
}

fn real_archive() -> Option<PathBuf> {
    match std::env::var_os("SCOUT_SEED_REAL_ARCHIVE") {
        Some(a) => Some(PathBuf::from(a)),
        None => {
            eprintln!("SCOUT_SEED_REAL_ARCHIVE not set: skipped");
            None
        }
    }
}

/// The real archive, read only: its merges, and from a scratch index of
/// it (the index folder is under `SCOUT_EDIT_SCRATCH` when set, else a
/// temporary folder; nothing is written into the archive, and the
/// installed index is not opened) the `merge_id` of every record a listed
/// event is merged from, and the works its tag rows mention.
#[test]
fn on_the_real_archive_merges_have_ids_and_works_are_counted() {
    let Some(archive) = real_archive() else {
        return;
    };
    let before = tree(&archive.join("analysis/events"));
    let e = EventEditor::open(&archive).unwrap();
    let live = e.merges().unwrap();
    let all = e.merges_all().unwrap();
    let check = e.check().unwrap();
    eprintln!(
        "real archive: {} merges in effect, {} with the undone ones; check: {} in effect, {} merged without a log line",
        live.len(),
        all.len(),
        check.in_effect,
        check.information.len()
    );
    assert_eq!(live.len(), check.in_effect);
    assert_eq!(live.len(), 55);
    assert!(live
        .iter()
        .all(|m| m.a.name.is_some() && m.b.name.is_some()));
    assert!(live.windows(2).all(|w| w[0].id > w[1].id));

    let scratch = match std::env::var_os("SCOUT_EDIT_SCRATCH") {
        Some(dir) => {
            fs::create_dir_all(&dir).unwrap();
            tempfile::Builder::new()
                .prefix("e4-index-")
                .tempdir_in(dir)
                .unwrap()
        }
        None => tempfile::tempdir().unwrap(),
    };
    let corpus = Corpus::from_config(config(&archive), scratch.path());
    assert!(corpus.index_path().starts_with(scratch.path()));
    let report = corpus.build_index(false).unwrap();
    let engine = Engine::new(
        Registry {
            corpora: vec![config(&archive)],
        },
        scratch.path(),
    );
    let reply = engine
        .events(&EventsQuery {
            min_items: 0,
            ..Default::default()
        })
        .unwrap();
    assert!(reply.notes.is_empty(), "{:?}", reply.notes);
    let listed = &reply.body.events;
    let from: Vec<_> = listed.iter().flat_map(|l| &l.merged_from).collect();
    let ids: std::collections::BTreeSet<&str> =
        from.iter().filter_map(|m| m.merge_id.as_deref()).collect();
    eprintln!(
        "real archive: {} events listed, {} merged-in records, {} with a merge_id; {} with own_aliases, {} with month_spans, {} with undated_items",
        listed.len(),
        from.len(),
        ids.len(),
        listed.iter().filter(|l| l.own_aliases.is_some()).count(),
        listed.iter().filter(|l| !l.month_spans.is_empty()).count(),
        listed.iter().filter(|l| l.undated_items.is_some()).count()
    );
    assert!(from.iter().all(|m| m.merge_id.is_some()));
    // Each line in effect is named once, by the record it merged away.
    let live_ids: std::collections::BTreeSet<&str> = live.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, live_ids);
    assert_eq!(from.len(), live.len());

    // Mentioned works: the rows that carry any, and the most mentioned.
    let tagged = topics::load(
        &scout_corpus::index::open_existing(&corpus.config, corpus.index_path()).unwrap(),
    )
    .unwrap();
    let with_works = tagged
        .values()
        .filter(|t| !t.mentioned_works.is_empty())
        .count();
    let facets = engine
        .facets(&FacetsQuery {
            top: 5,
            ..Default::default()
        })
        .unwrap()
        .body;
    let works = facets.facets.works.expect("the works facet");
    eprintln!(
        "real archive: {} documents indexed, {} tagged, {} carry mentioned_works; {} distinct works; top five:",
        report.docs,
        tagged.len(),
        with_works,
        works.distinct
    );
    for w in &works.items {
        eprintln!(
            "  {:>4}  {}  {}",
            w.count,
            w.value,
            w.title.as_deref().unwrap_or("(no document of that key)")
        );
    }
    assert!(with_works > 0 && works.items.len() == 5);
    assert!(works.items.iter().all(|w| w.count > 0));
    // The archive's records folder is as it was.
    assert!(tree(&archive.join("analysis/events")) == before);
}

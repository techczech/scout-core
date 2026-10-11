//! What `scout events` shows of a merged event, over an index build: its
//! span, its aliases and the records merged into it are derived from the
//! records (no record holds another's values), for direct and chained
//! merges and after each undo; an old monthly id selects the surviving
//! event with the items of all; the seed finds every monthly id held; and
//! a folder with no merged record lists exactly as it did before the
//! derivation.

mod event_edit_support;
use event_edit_support::*;
use scout_corpus::api::{EventsQuery, FacetsQuery, MonthCount};
use scout_corpus::event_seed::{seed, SeedOptions};
use scout_corpus::registry::{CorpusConfig, CorpusKind, DocumentUnit, FieldMap};
use scout_corpus::{topics, Corpus, Engine, Registry, SearchRequest};
use serde::Serialize;

/// The fixture's records with five works: two in alpha, one in beta, one
/// in gamma, one in delta.
struct Indexed {
    fx: Fx,
    _data: tempfile::TempDir,
    corpus: Corpus,
    engine: Engine,
}

fn indexed() -> Indexed {
    let fx = fixture();
    let data = tempfile::tempdir().unwrap();
    let root = fx.root();
    let rows = [
        ("1", "2026-09-06", "2026-09/E01", "Alpha"),
        ("2", "2026-09-07", "2026-09/E01", "Alpha"),
        ("3", "2026-10-03", "2026-10/E04", "Beta"),
        ("4", "2026-11-02", "2026-11/E02", "Gamma"),
        ("5", "2026-08-01", "2026-08/E07", "Delta"),
    ];
    let mut layer = String::new();
    for (n, date, event, name) in rows {
        let p = root.join(format!("readings/works/w-{n}.md"));
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(
            p,
            format!("---\ntitle: W{n}\nauthor: A\ntype: tweet\nsource_system: x\nsource_id: \"{n}\"\n---\n\n> A proof, number {n}.\n\nhighlighted_at: 2026-09-01\n\n---\n"),
        )
        .unwrap();
        let row = json!({"key": format!("x:{n}"), "date": date, "themes": ["T"], "entities": [], "event": event, "event_name": name});
        layer.push_str(&format!("{row}\n"));
    }
    let file = root.join(topics::LAYER_FILE);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, layer).unwrap();
    let config = CorpusConfig {
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
    };
    let corpus = Corpus::from_config(config.clone(), data.path());
    let engine = Engine::new(
        Registry {
            corpora: vec![config],
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

impl Indexed {
    /// Build the index and list every event, as JSON, by id.
    fn listing(&self, merged: bool) -> Value {
        self.corpus.build_index(false).unwrap();
        let q = EventsQuery {
            min_items: 0,
            merged,
            ..Default::default()
        };
        serde_json::to_value(self.engine.events(&q).unwrap().body).unwrap()
    }

    fn row(&self, id: &str) -> Value {
        let body = self.listing(true);
        let rows = body["events"].as_array().unwrap();
        rows.iter()
            .find(|x| x["id"] == id)
            .unwrap_or_else(|| panic!("{id} is not listed"))
            .clone()
    }

    fn ids(&self) -> Vec<String> {
        let body = self.listing(false);
        let mut v: Vec<String> = body["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x["id"].as_str().unwrap().to_string())
            .collect();
        v.sort();
        v
    }

    fn total(&self, q: &str) -> usize {
        self.corpus
            .search(&SearchRequest::new(q))
            .unwrap()
            .total_documents
    }

    fn merge(&self, a: &str, b: &str) -> String {
        let m = self.fx.editor().merge(&merge_req(a, b, By::Dominik));
        m.unwrap().log_id
    }

    /// The seed, dry: `(monthly ids already in a record, records to
    /// create)`.
    fn seed(&self) -> (usize, usize) {
        let r = seed(&SeedOptions {
            archive: self.fx.root().to_path_buf(),
            out: None,
            apply: false,
        })
        .unwrap();
        (r.already_in_a_record, r.to_create)
    }
}

/// The span, own dates, aliases and merged-in ids of a listed event.
fn shown(row: &Value) -> (Value, Value, Value, Value, Value, Vec<String>) {
    let from = row
        .get("merged_from")
        .and_then(|l| l.as_array())
        .map(|l| l.iter().map(|m| m["id"].as_str().unwrap().into()).collect())
        .unwrap_or_default();
    let own = |k: &str| row.get(k).cloned().unwrap_or(json!("absent"));
    (
        row["start"].clone(),
        row["end"].clone(),
        own("own_start"),
        own("own_end"),
        row["aliases"].clone(),
        from,
    )
}

#[test]
fn a_merged_event_lists_the_span_and_aliases_of_all_its_records() {
    let ix = indexed();
    let start = ix.fx.tree();
    assert_eq!(ix.seed(), (4, 0));
    let alone = ix.row(ALPHA);
    assert_eq!(
        shown(&alone),
        (
            json!("2026-09-05"),
            json!("2026-09-20"),
            json!("absent"),
            json!("absent"),
            json!(["The Alpha", "Shared"]),
            vec![]
        )
    );
    assert_eq!(alone["items"], json!(2));

    // Direct: beta (2 to 9 October) into alpha.
    let m1 = ix.merge(BETA, ALPHA);
    let row = ix.row(ALPHA);
    assert_eq!(
        shown(&row),
        (
            json!("2026-09-05"),
            json!("2026-10-09"),
            json!("absent"),
            json!("2026-09-20"),
            // Beta's name and its alias "Beta"; not "shared" (alpha has
            // "Shared") nor "ALPHA STORY" (alpha's name).
            json!(["The Alpha", "Shared", "Beta follow-up", "Beta"]),
            strs(&[BETA])
        )
    );
    assert_eq!(
        row["merged_from"],
        json!([{"id": BETA, "name": "Beta follow-up"}])
    );
    // The rest is alpha's own record: the monthly ids of beta are in
    // beta's record, and resolve to alpha through it.
    assert_eq!(row["members"], json!(["2026-09/E01"]));
    assert_eq!(
        (&row["name"], &row["edited"], &row["description"]),
        (&alone["name"], &alone["edited"], &alone["description"])
    );
    assert_eq!(row["items"], json!(3));
    for q in [
        "event:2026-10/E04",
        "event:2026-09/E01",
        &format!("event:{BETA}"),
        &format!("event:{ALPHA}"),
    ] {
        assert_eq!(ix.total(q), 3, "{q}");
    }
    let facets = ix.engine.facets(&FacetsQuery {
        query: "event:2026-10/E04".into(),
        top: 0,
        ..Default::default()
    });
    let facet: Vec<(String, usize)> = facets
        .unwrap()
        .body
        .facets
        .event
        .items
        .into_iter()
        .map(|i| (i.value, i.count))
        .collect();
    assert_eq!(facet, vec![(ALPHA.to_string(), 3)]);
    assert_eq!(ix.ids(), strs(&[DELTA, ALPHA, GAMMA]));
    assert_eq!(ix.seed(), (4, 0));
    // The record merged away, listed on request: its own values.
    let away = ix.row(BETA);
    assert_eq!(
        shown(&away),
        (
            json!("2026-10-02"),
            json!("2026-10-09"),
            json!("absent"),
            json!("absent"),
            json!(["Beta", "shared", "ALPHA STORY"]),
            vec![]
        )
    );
    assert_eq!(
        (&away["merged_into"], &away["items"]),
        (&json!(ALPHA), &json!(0))
    );

    // A second record, earlier than alpha: the start moves back.
    let m2 = ix.merge(DELTA, ALPHA);
    let row = ix.row(ALPHA);
    assert_eq!(
        shown(&row),
        (
            json!("2026-08-01"),
            json!("2026-10-09"),
            json!("2026-09-05"),
            json!("2026-09-20"),
            json!(["The Alpha", "Shared", "Delta", "Beta follow-up", "Beta"]),
            strs(&[DELTA, BETA])
        )
    );
    assert_eq!(row["items"], json!(4));
    assert_eq!(ix.total("event:2026-08/E07"), 4);
    assert_eq!(ix.seed(), (4, 0));

    // Undone out of order: beta first. What is left is alpha and delta.
    ix.fx.editor().unmerge(&m1).unwrap();
    let row = ix.row(ALPHA);
    assert_eq!(
        shown(&row),
        (
            json!("2026-08-01"),
            json!("2026-09-20"),
            json!("2026-09-05"),
            json!("absent"),
            json!(["The Alpha", "Shared", "Delta"]),
            strs(&[DELTA])
        )
    );
    assert_eq!(
        (row["items"].clone(), ix.total("event:2026-10/E04")),
        (json!(3), 1)
    );
    assert_eq!(ix.seed(), (4, 0));
    ix.fx.editor().unmerge(&m2).unwrap();
    assert_eq!(ix.row(ALPHA), alone);
    assert_eq!(ix.seed(), (4, 0));
    let mut after = ix.fx.tree();
    assert!(after.remove(&ix.fx.dir.join(MERGE_LOG)).is_some());
    assert!(after == start);
}

#[test]
fn a_chain_and_an_open_ended_survivor() {
    let ix = indexed();
    // delta into beta, then beta into gamma: delta is in gamma through
    // beta.
    let inner = ix.merge(DELTA, BETA);
    let row = ix.row(BETA);
    assert_eq!(
        shown(&row),
        (
            json!("2026-08-01"),
            json!("2026-10-09"),
            json!("2026-10-02"),
            json!("absent"),
            json!(["Beta", "shared", "ALPHA STORY", "Delta"]),
            strs(&[DELTA])
        )
    );
    let outer = ix.merge(BETA, GAMMA);
    let row = ix.row(GAMMA);
    assert_eq!(
        shown(&row),
        (
            json!("2026-08-01"),
            json!("2026-11-03"),
            json!("2026-11-01"),
            json!("absent"),
            json!([
                "His alias",
                "Delta",
                "Beta follow-up",
                "Beta",
                "shared",
                "ALPHA STORY"
            ]),
            strs(&[DELTA, BETA])
        )
    );
    assert_eq!(row["items"], json!(3));
    for q in [
        "event:2026-08/E07",
        "event:2026-10/E04",
        &format!("event:{DELTA}"),
    ] {
        assert_eq!(ix.total(q), 3, "{q}");
    }
    // Beta, merged away, is listed as itself, without delta.
    assert_eq!(shown(&ix.row(BETA)).5, Vec::<String>::new());
    assert_eq!(ix.seed(), (4, 0));

    // He opens gamma's end: it stays open whatever is merged in.
    let e = ix.fx.editor();
    e.set(
        GAMMA,
        &FieldEdit {
            end: Some(String::new()),
            ..FieldEdit::default()
        },
    )
    .unwrap();
    let m3 = ix.merge(ALPHA, GAMMA);
    let row = ix.row(GAMMA);
    assert_eq!(
        (&row["start"], &row["end"], row.get("own_end")),
        (&json!("2026-08-01"), &Value::Null, None)
    );
    assert_eq!(shown(&row).5, strs(&[DELTA, ALPHA, BETA]));
    assert_eq!(row["items"], json!(5));

    // The inner merge undone while the chain stands: delta leaves alone.
    e.unmerge(&inner).unwrap();
    let row = ix.row(GAMMA);
    assert_eq!(
        (&row["start"], &row["own_start"], shown(&row).5),
        (
            &json!("2026-09-05"),
            &json!("2026-11-01"),
            strs(&[ALPHA, BETA])
        )
    );
    assert_eq!(
        (row["items"].clone(), ix.total("event:2026-08/E07")),
        (json!(4), 1)
    );
    e.unmerge(&m3).unwrap();
    e.unmerge(&outer).unwrap();
    let row = ix.row(GAMMA);
    assert_eq!(
        shown(&row),
        (
            json!("2026-11-01"),
            Value::Null,
            json!("absent"),
            json!("absent"),
            json!(["His alias"]),
            vec![]
        )
    );
    assert_eq!(ix.ids(), strs(&[DELTA, ALPHA, BETA, GAMMA]));
    assert_eq!(ix.seed(), (4, 0));
}

/// One listed event as it was written before spans and aliases were
/// derived: these keys, in this order, from the record alone.
#[derive(Serialize)]
struct Before {
    corpus: String,
    id: String,
    name: Option<String>,
    record: bool,
    start: Option<String>,
    end: Option<String>,
    items: usize,
    months: Vec<MonthCount>,
    members: Vec<String>,
    aliases: Vec<String>,
    themes: Vec<String>,
    origin: Option<String>,
    edited: Option<String>,
    merged_into: Option<String>,
    description: String,
}

#[test]
fn a_folder_with_no_merged_record_lists_as_it_did_before() {
    let ix = indexed();
    ix.corpus.build_index(false).unwrap();
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
    for key in ["own_start", "own_end", "merged_from"] {
        assert!(!now.contains(key), "{key}");
    }
}

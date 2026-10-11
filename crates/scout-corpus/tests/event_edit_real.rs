//! The edits against an index and against the real archive:
//!
//! - a removed event in an index build (not listed, no facet value, no
//!   item has it; restored, it is back);
//! - when `SCOUT_SEED_REAL_ARCHIVE` names the real archive: every record
//!   there reads as it did before the `removed` field (the writer still
//!   writes each file's bytes from what the reader reads), and a merge and
//!   its undo on a COPY of its records folder, with an index build
//!   between (the merge writes one record; the listing derives the rest). The archive itself is only read. The copy goes to
//!   `SCOUT_EDIT_SCRATCH` when set, else to a temporary folder.

mod event_edit_support;
use event_edit_support::*;
use scout_corpus::api::{EventsQuery, FacetsQuery};
use scout_corpus::registry::{CorpusConfig, CorpusKind, DocumentUnit, FieldMap};
use scout_corpus::{topics, Corpus, Engine, Registry, SearchRequest};

fn corpus(root: &Path, data: &Path) -> Corpus {
    Corpus::from_config(
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
        },
        data,
    )
}

fn engine(c: &Corpus, data: &Path) -> Engine {
    Engine::new(
        Registry {
            corpora: vec![c.config.clone()],
        },
        data,
    )
}

fn work(root: &Path, n: usize, system: &str, id: &str) {
    let p = root.join(format!("readings/works/w-{n}.md"));
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(
        p,
        format!("---\ntitle: W{n}\nauthor: A\ntype: tweet\nsource_system: {system}\nsource_id: \"{id}\"\n---\n\n> A proof, number {n}.\n\nhighlighted_at: 2026-09-01\n\n---\n"),
    )
    .unwrap();
}

fn total(c: &Corpus, q: &str) -> usize {
    c.search(&SearchRequest::new(q)).unwrap().total_documents
}

/// `(id, items)` of every listed event.
fn listed(e: &Engine, removed: bool) -> Vec<(String, usize)> {
    let q = EventsQuery {
        min_items: 0,
        removed,
        ..Default::default()
    };
    let mut v: Vec<(String, usize)> = e
        .events(&q)
        .unwrap()
        .body
        .events
        .into_iter()
        .map(|x| (x.id, x.items))
        .collect();
    v.sort();
    v
}

fn facet(e: &Engine, query: &str) -> Vec<(String, usize)> {
    let q = FacetsQuery {
        query: query.into(),
        top: 0,
        ..Default::default()
    };
    e.facets(&q)
        .unwrap()
        .body
        .facets
        .event
        .items
        .into_iter()
        .map(|i| (i.value, i.count))
        .collect()
}

#[test]
fn a_removed_event_is_not_listed_is_no_facet_value_and_no_item_has_it() {
    let fx = fixture();
    let data = tempfile::tempdir().unwrap();
    let root = fx.root();
    // Three works: two in alpha, one of them in gamma as well; one in beta.
    let rows = [
        json!({"key": "x:1", "date": "2026-09-06", "themes": ["T"], "entities": [], "event": "2026-09/E01", "event_name": "Alpha"}),
        json!({"key": "x:2", "date": "2026-09-07", "themes": ["T"], "entities": [], "event": "2026-09/E01", "event_name": "Alpha", "events": ["2026-11/E02"]}),
        json!({"key": "x:3", "date": "2026-10-03", "themes": ["U"], "entities": [], "event": "2026-10/E04", "event_name": "Beta"}),
    ];
    for n in 1..=3 {
        work(root, n, "x", &n.to_string());
    }
    let layer = root.join(topics::LAYER_FILE);
    fs::create_dir_all(layer.parent().unwrap()).unwrap();
    fs::write(
        &layer,
        rows.iter().map(|r| format!("{r}\n")).collect::<String>(),
    )
    .unwrap();
    let c = corpus(root, data.path());
    let e = engine(&c, data.path());
    let s = |l: &[(&str, usize)]| {
        l.iter()
            .map(|(id, n)| (id.to_string(), *n))
            .collect::<Vec<_>>()
    };
    c.build_index(false).unwrap();
    let all = s(&[(DELTA, 0), (ALPHA, 2), (BETA, 1), (GAMMA, 1)]);
    assert_eq!(listed(&e, false), all);
    assert_eq!(
        (
            total(&c, "event:2026-09/E01"),
            total(&c, &format!("event:{ALPHA}"))
        ),
        (2, 2)
    );

    // Remove alpha: its items keep their themes and their other events.
    fx.editor().remove(ALPHA).unwrap();
    c.build_index(false).unwrap();
    assert_eq!(listed(&e, false), s(&[(DELTA, 0), (BETA, 1), (GAMMA, 1)]));
    assert_eq!(facet(&e, "theme:T"), s(&[(GAMMA, 1)]));
    for q in ["event:2026-09/E01", &format!("event:{ALPHA}")] {
        assert_eq!(total(&c, q), 0, "{q}");
    }
    assert_eq!(
        (total(&c, "theme:T"), total(&c, &format!("event:{GAMMA}"))),
        (2, 1)
    );
    let hit = c.search(&SearchRequest::new("theme:T")).unwrap();
    for d in &hit.results {
        let t = serde_json::to_value(d).unwrap()["topics"].clone();
        assert_eq!(t["event"], Value::Null, "{t}");
        assert!(
            !t["events"].as_array().unwrap().contains(&json!(ALPHA)),
            "{t}"
        );
        // Not a correction of the item.
        assert!(!t.to_string().contains("\"corrected\":[\"event\"]"), "{t}");
    }
    // On request it is listed, without items, with its date.
    assert_eq!(
        listed(&e, true),
        s(&[(DELTA, 0), (ALPHA, 0), (BETA, 1), (GAMMA, 1)])
    );
    let q = EventsQuery {
        min_items: 0,
        removed: true,
        ..Default::default()
    };
    let body = serde_json::to_value(e.events(&q).unwrap().body).unwrap();
    assert_eq!(body["removed"], json!(true));
    let row = body["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|x| x["id"] == ALPHA)
        .unwrap();
    assert_eq!(row["removed"], json!(TODAY));
    // Without the flag the reply has no `removed` key anywhere.
    let plain = serde_json::to_string(
        &e.events(&EventsQuery {
            min_items: 0,
            ..Default::default()
        })
        .unwrap()
        .body,
    )
    .unwrap();
    assert!(!plain.contains("removed"), "{plain}");
    // The seed still sees its member as held: resolution is unchanged.
    assert_eq!(fx.set().resolve("2026-09/E01"), Some(ALPHA));

    // An event with another merged into it, removed: both are gone.
    fx.editor()
        .merge(&merge_req(BETA, GAMMA, By::Dominik))
        .unwrap();
    fx.editor().remove(GAMMA).unwrap();
    c.build_index(false).unwrap();
    assert_eq!(listed(&e, false), s(&[(DELTA, 0)]));
    assert_eq!(total(&c, "event:2026-10/E04"), 0);

    // Restored: all as before the removals (beta now in gamma).
    fx.editor().restore(ALPHA).unwrap();
    fx.editor().restore(GAMMA).unwrap();
    c.build_index(false).unwrap();
    assert_eq!(listed(&e, false), s(&[(DELTA, 0), (ALPHA, 2), (GAMMA, 2)]));
    assert_eq!(total(&c, "event:2026-09/E01"), 2);
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

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let t = e.file_type().unwrap();
        if t.is_dir() {
            copy_dir(&e.path(), &to.join(e.file_name()));
        } else if t.is_file() {
            fs::copy(e.path(), to.join(e.file_name())).unwrap();
        }
    }
}

/// Every record of the real archive reads as it did before `removed`: no
/// file skipped, no warning, no record removed, and the writer writes
/// each file's exact bytes from the record the reader reads from it.
#[test]
fn on_the_real_archive_every_record_reads_unchanged() {
    let Some(archive) = real_archive() else {
        return;
    };
    let dir = archive.join(events::EVENTS_DIR);
    let set = read_all(&dir);
    let files = fs::read_dir(&dir)
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".md")
        })
        .count();
    assert_eq!(set.records.len(), files);
    assert!(files >= 1712, "{files} records");
    eprintln!("real archive: {files} records read");
    for r in set.records.values() {
        assert_eq!(r.removed, None, "{}", r.id);
        let bytes = fs::read(dir.join(format!("{}.md", r.id))).unwrap();
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            render_record(r),
            "{}",
            r.id
        );
    }
    // Opening the folder for editing and checking it writes nothing.
    let c = EventEditor::open(&archive).unwrap().check().unwrap();
    assert_eq!(c.records, files);
}

/// A merge and its undo on a copy of the real records folder: the old
/// monthly id resolves to the surviving event with the items of both, and
/// after the undo the folder is the copy again, but for the log.
#[test]
fn on_a_copy_of_the_real_records_a_merge_and_its_undo() {
    const MATHS: &str = "2026-10-mathematicians-backlash-and-ahm-statement";
    const FIELDS: &str = "2026-09-fields-medallists-declaration-on-ai-and-mathematics";
    let Some(archive) = real_archive() else {
        return;
    };
    let scratch = match std::env::var_os("SCOUT_EDIT_SCRATCH") {
        Some(dir) => {
            fs::create_dir_all(&dir).unwrap();
            tempfile::Builder::new()
                .prefix("run-")
                .tempdir_in(dir)
                .unwrap()
        }
        None => tempfile::tempdir().unwrap(),
    };
    let root = scratch.path().join("archive");
    let dir = root.join(events::EVENTS_DIR);
    copy_dir(&archive.join(events::EVENTS_DIR), &dir);
    let copy = tree(&dir);
    let set = read_all(&dir);
    let (maths, fields) = (set.records[MATHS].clone(), set.records[FIELDS].clone());
    let old_id = maths.members[0].clone();
    // A scratch corpus: the real tag rows of the two events, each with a
    // small work of its key.
    let mut rows = String::new();
    let mut n = 0;
    let (mut in_maths, mut in_fields) = (0, 0);
    for l in fs::read_to_string(archive.join(topics::LAYER_FILE))
        .unwrap()
        .lines()
    {
        let Ok(v) = serde_json::from_str::<Value>(l) else {
            continue;
        };
        let Some(holder) = v["event"].as_str().and_then(|e| set.resolve(e)) else {
            continue;
        };
        if holder != MATHS && holder != FIELDS {
            continue;
        }
        let (system, id) = v["key"].as_str().unwrap().split_once(':').unwrap();
        n += 1;
        work(&root, n, system, id);
        rows.push_str(l);
        rows.push('\n');
        if holder == MATHS {
            in_maths += 1
        } else {
            in_fields += 1
        }
    }
    assert!(in_maths > 0 && in_fields > 0, "{in_maths} {in_fields}");
    fs::create_dir_all(root.join("analysis/topics")).unwrap();
    fs::write(root.join(topics::LAYER_FILE), rows).unwrap();
    let data = scratch.path().join("data");
    let c = corpus(&root, &data);
    c.build_index(false).unwrap();
    assert_eq!(total(&c, &format!("event:{old_id}")), in_maths);
    assert_eq!(total(&c, &format!("event:{FIELDS}")), in_fields);

    let e = EventEditor::open(&root).unwrap();
    let m = e
        .merge(&MergeRequest {
            a: MATHS.into(),
            b: FIELDS.into(),
            by: By::Dominik,
            reason: Some("one story".into()),
        })
        .unwrap();
    // Only the record merged away was written.
    let fields_file = dir.join(format!("{FIELDS}.md"));
    assert_eq!(m.a_record.merged_into.as_deref(), Some(FIELDS));
    assert_eq!(fs::read(&fields_file).unwrap(), copy[&fields_file]);
    c.build_index(false).unwrap();
    // The old monthly id, the merged-away id and the surviving id all
    // select the one event, with the items of both.
    for q in [
        format!("event:{old_id}"),
        format!("event:{MATHS}"),
        format!("event:{FIELDS}"),
    ] {
        assert_eq!(total(&c, &q), in_maths + in_fields, "{q}");
    }
    let eng = engine(&c, &data);
    assert_eq!(
        facet(&eng, &format!("event:{old_id}"))[0],
        (FIELDS.to_string(), in_maths + in_fields)
    );
    let row = listed(&eng, false)
        .into_iter()
        .find(|(id, _)| id == FIELDS)
        .unwrap();
    assert_eq!(row.1, in_maths + in_fields);
    assert!(listed(&eng, false).iter().all(|(id, _)| id != MATHS));
    let now = read_all(&dir);
    assert_eq!(now.resolve(&old_id), Some(FIELDS));
    assert_eq!(now.records[FIELDS], fields);
    // The listing derives the event's span and aliases from both records.
    let q = EventsQuery {
        min_items: 0,
        ..Default::default()
    };
    let shown = eng.events(&q).unwrap().body.events;
    let shown = shown.iter().find(|x| x.id == FIELDS).unwrap();
    let all = events::combined(&fields, &[&maths]);
    assert_eq!((&shown.start, &shown.end), (&all.start, &all.end));
    assert_eq!(shown.aliases, all.aliases);
    assert!(shown.aliases.contains(&maths.name));
    assert_eq!(shown.merged_from.len(), 1);
    assert_eq!(shown.merged_from[0].id, MATHS);
    assert_eq!(shown.members, fields.members);
    eprintln!(
        "{FIELDS}: own {:?} to {:?}, shown {:?} to {:?}",
        fields.start, fields.end, shown.start, shown.end
    );
    assert!(e.check().unwrap().ok);
    eprintln!("copy of the real records: {old_id} -> {FIELDS}, {in_maths} + {in_fields} items");

    // Undo: the folder is the copy again, but for the log.
    e.unmerge(&m.log_id).unwrap();
    let mut after = tree(&dir);
    assert!(after.remove(&dir.join(MERGE_LOG)).is_some());
    assert!(after == copy);
    assert_eq!(log_lines(&dir.join(MERGE_LOG)).len(), 2);
    c.build_index(false).unwrap();
    assert_eq!(total(&c, &format!("event:{old_id}")), in_maths);
    assert_eq!(total(&c, &format!("event:{FIELDS}")), in_fields);

    // The tagger's proposals file, read in place in the copy: a dry run
    // plans its confident merges and writes nothing.
    let folder = dir.join(PROPOSALS_DIR);
    if let Some(file) = fs::read_dir(&folder).ok().and_then(|rd| {
        rd.map(|e| e.unwrap().path())
            .find(|p| p.extension().is_some_and(|x| x == "jsonl"))
    }) {
        let state = tree(&dir);
        let dry = e.apply_proposals(&file, false).unwrap();
        let waiting = e.proposals(&file).unwrap();
        assert!(tree(&dir) == state);
        eprintln!(
            "proposals {}: {} lines, {} same+confident, {} merges planned, skipped {:?}, {} waiting",
            file.display(), dry.lines, dry.same_confident, dry.merges.len(), dry.skipped_counts, waiting.waiting.len()
        );
        assert_eq!(dry.merges.len() + dry.skipped.len(), dry.same_confident);
    }
}

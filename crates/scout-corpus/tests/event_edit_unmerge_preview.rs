//! `EventEditor::unmerge_preview`: what it says an undo would leave is
//! what the undo leaves. Each test of the matrix (spec
//! `docs/specs/2026-10-13-unmerge-preview-and-engine-follow-ups.md`, E6)
//! computes the preview, runs the real `unmerge`, builds the index over
//! the folder as it then is and compares `scout events` and a search
//! with the preview ([`World::split`]); the numbers each test then names
//! are the state it is about. The preview writes nothing, and `unmerge`
//! refuses a returning record that carries `removed`.

mod event_edit_support;
use event_edit_support::*;
use scout_corpus::api::EventsQuery;
use scout_corpus::event_edit::UnmergePreview;
use scout_corpus::registry::{CorpusConfig, CorpusKind, DocumentUnit, FieldMap};
use scout_corpus::{topics, Corpus, Engine, Registry, SearchRequest};

const A: &str = "2026-09-alpha";
const B: &str = "2026-10-beta";
const C: &str = "2026-08-gamma";
/// The monthly ids the three records hold.
const A_M: &str = "2026-09/E01";
const B_M: &str = "2026-10/E04";
const C_M: &str = "2026-08/E02";

/// An archive with records, tag rows, corrections and one work per
/// document named, and an index over it kept outside the archive.
struct World {
    tmp: tempfile::TempDir,
    _data: tempfile::TempDir,
    corpus: Corpus,
    engine: Engine,
}

fn records() -> Vec<EventRecord> {
    vec![
        rec(A, "Alpha", ("2026-09-05", "2026-09-20"), &[A_M]),
        rec(B, "Beta", ("2026-10-02", "2026-10-09"), &[B_M]),
        rec(C, "Gamma", ("2026-08-01", "2026-08-03"), &[C_M]),
    ]
}

/// A tag row of the work `n` in the monthly event `event`.
fn row(n: usize, event: &str) -> Value {
    json!({"key": format!("x:{n}"), "date": "2026-09-06", "themes": ["T"], "entities": [], "event": event, "event_name": "E"})
}

/// A hand-written correction of the work `n`.
fn fix(n: usize, field: &str, id: &str) -> Value {
    json!({"key": format!("x:{n}"), field: [id]})
}

fn jsonl(lines: &[Value]) -> String {
    lines.iter().map(|l| format!("{l}\n")).collect()
}

impl World {
    /// `layer` is the layer file as written; a work is written for each
    /// of `docs` (`x:<n>`).
    fn new(records: &[EventRecord], layer: &str, corrections: &[Value], docs: &[usize]) -> World {
        let tmp = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let dir = root.join(events::EVENTS_DIR);
        fs::create_dir_all(&dir).unwrap();
        for r in records {
            put(&dir, r);
        }
        fs::create_dir_all(root.join("readings/works")).unwrap();
        for n in docs {
            fs::write(
                root.join(format!("readings/works/w-{n}.md")),
                format!("---\ntitle: W{n}\nauthor: A\ntype: tweet\nsource_system: x\nsource_id: \"{n}\"\n---\n\n> A proof, number {n}.\n\nhighlighted_at: 2026-09-01\n\n---\n"),
            )
            .unwrap();
        }
        let file = root.join(topics::LAYER_FILE);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, layer).unwrap();
        if !corrections.is_empty() {
            fs::write(root.join(topics::CORRECTIONS_FILE), jsonl(corrections)).unwrap();
        }
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
        World {
            tmp,
            _data: data,
            corpus,
            engine,
        }
    }

    /// Rows `1..=rows.len()`, one work each.
    fn of(records: &[EventRecord], rows: &[Value], corrections: &[Value]) -> World {
        let docs: Vec<usize> = (1..=rows.len()).collect();
        World::new(records, &jsonl(rows), corrections, &docs)
    }

    fn root(&self) -> &Path {
        self.tmp.path()
    }

    fn editor(&self) -> EventEditor {
        EventEditor::open(self.root()).unwrap().with_now(NOW)
    }

    fn merge(&self, a: &str, b: &str) -> String {
        let m = self.editor().merge(&merge_req(a, b, By::Dominik));
        m.unwrap().log_id
    }

    fn correct(&self, lines: &[Value]) {
        use std::io::Write;
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.root().join(topics::CORRECTIONS_FILE))
            .unwrap();
        f.write_all(jsonl(lines).as_bytes()).unwrap();
    }

    /// Build the index over the folder as it is and give the row
    /// `scout events` lists for `id` (merged and removed records too).
    fn listed(&self, id: &str) -> Value {
        self.corpus.build_index(false).unwrap();
        let q = EventsQuery {
            min_items: 0,
            merged: true,
            removed: true,
            ..Default::default()
        };
        let body = serde_json::to_value(self.engine.events(&q).unwrap().body).unwrap();
        let rows = body["events"].as_array().unwrap();
        rows.iter()
            .find(|x| x["id"] == id)
            .unwrap_or_else(|| panic!("{id} is not listed"))
            .clone()
    }

    fn total(&self, q: &str) -> usize {
        self.corpus
            .search(&SearchRequest::new(q))
            .unwrap()
            .total_documents
    }

    /// The whole comparison of one matrix row: the preview of `log_id`
    /// (which writes nothing), the index as it is now, the real
    /// `unmerge`, the index rebuilt, and every value of the preview
    /// against what the engine then lists and finds.
    fn split(&self, log_id: &str) -> UnmergePreview {
        let before = tree(self.root());
        let p = self.editor().unmerge_preview(log_id).unwrap();
        assert!(tree(self.root()) == before, "the preview wrote");
        assert!(p.undoable && p.why_not.is_none(), "{p:?}");
        let n = |v: Option<usize>| json!(v.expect("a count"));
        assert_eq!(
            self.listed(&p.kept.id)["items"],
            n(p.kept_items_now),
            "kept_items_now"
        );

        let u = self.editor().unmerge(log_id).unwrap();
        assert_eq!(
            (u.a.as_str(), u.withdrawn),
            (p.returning.id.as_str(), false)
        );
        let (a, kept) = (self.listed(&p.returning.id), self.listed(&p.kept.id));
        assert_eq!(a["items"], n(p.returning_items), "returning_items");
        assert_eq!(kept["items"], n(p.kept_items), "kept_items");
        assert_eq!(
            (&a["name"], &kept["name"]),
            (&json!(p.returning.name), &json!(p.kept.name))
        );
        assert_eq!(
            (
                &a["start"],
                &a["end"],
                &a["merged_into"],
                &kept["merged_into"]
            ),
            (&json!(p.start), &json!(p.end), &Value::Null, &Value::Null),
            "the span of the returning event"
        );
        let both = format!("event:{} event:{}", p.returning.id, p.kept.id);
        assert_eq!(self.total(&both), p.in_both.unwrap(), "in_both");
        p
    }
}

/// `(returning_items, kept_items, in_both, kept_items_now)`.
fn counts(p: &UnmergePreview) -> (usize, usize, usize, usize) {
    let n = |v: Option<usize>| v.expect("a count");
    (
        n(p.returning_items),
        n(p.kept_items),
        n(p.in_both),
        n(p.kept_items_now),
    )
}

/// Row 1: A merged into B; every item in one event by its tag row.
#[test]
fn row_1_the_items_of_the_returning_event_return_and_the_kept_event_keeps_its_own() {
    let w = World::of(
        &records(),
        &[row(1, A_M), row(2, A_M), row(3, B_M), row(4, C_M)],
        &[],
    );
    let id = w.merge(A, B);
    let p = w.split(&id);
    assert_eq!(counts(&p), (2, 1, 0, 3));
    assert_eq!(
        (
            p.log_id.as_str(),
            p.returning.id.as_str(),
            p.kept.id.as_str()
        ),
        (id.as_str(), A, B)
    );
    assert_eq!(
        (p.returning.name.as_str(), p.kept.name.as_str()),
        ("Alpha", "Beta")
    );
    // A's own span again: nothing is merged into it.
    assert_eq!(
        (p.start.as_deref(), p.end.as_deref()),
        (Some("2026-09-05"), Some("2026-09-20"))
    );
}

/// Row 2: a tag row with `event` in A and a further event of B in its
/// `events` list (and one the other way round).
#[test]
fn row_2_an_item_with_a_second_event_in_its_events_list_is_counted_in_both() {
    let mut two = row(1, A_M);
    two["events"] = json!([B_M]);
    let mut other_way = row(2, B_M);
    other_way["events"] = json!([A_M, "2026-01/E77"]);
    let w = World::of(&records(), &[two, other_way, row(3, A_M), row(4, B_M)], &[]);
    let id = w.merge(A, B);
    let p = w.split(&id);
    assert_eq!(counts(&p), (3, 3, 2, 4));
}

/// Row 3: after the merge, an item of A is added to B by a correction.
#[test]
fn row_3_an_item_of_the_returning_event_added_to_the_kept_one_by_correction_is_in_both() {
    let w = World::of(&records(), &[row(1, A_M), row(2, A_M), row(3, B_M)], &[]);
    let id = w.merge(A, B);
    // By the kept record's id, and by its monthly id.
    w.correct(&[fix(1, "event_add", B)]);
    w.correct(&[fix(2, "event_add", B_M)]);
    let p = w.split(&id);
    assert_eq!(counts(&p), (2, 3, 2, 3));
}

/// Row 4: after the merge, an item of A is taken out of B by a
/// correction. The engine's rule, pinned here and stated on
/// `UnmergePreview::returning_items`: a correction's id is resolved
/// through the records as they are after the split. Taken out by B's id,
/// the item is in A again; taken out by A's id or A's monthly id, it
/// stays out of both.
#[test]
fn row_4_an_item_taken_out_by_the_kept_events_id_returns_and_one_taken_out_by_its_own_stays_out() {
    let rows = [
        row(1, A_M),
        row(2, A_M),
        row(3, A_M),
        row(4, A_M),
        row(5, B_M),
    ];
    let w = World::of(&records(), &rows, &[]);
    let id = w.merge(A, B);
    w.correct(&[
        fix(1, "event_remove", B),
        fix(2, "event_remove", A),
        fix(3, "event_remove", A_M),
    ]);
    // Now: only the untouched item of A and B's own are in the event.
    assert_eq!(w.listed(B)["items"], json!(2));
    let p = w.split(&id);
    assert_eq!(counts(&p), (2, 1, 0, 2));
    // Item 1 is back in A; items 2 and 3 are in no event.
    assert_eq!(w.total(&format!("event:{A}")), 2);
    let hit = |n: usize| {
        let r = w.corpus.search(&SearchRequest::new(format!("number {n}")));
        let r = serde_json::to_value(r.unwrap()).unwrap();
        r["results"][0]["topics"]["events"].clone()
    };
    assert_eq!(
        (hit(1), hit(2), hit(3), hit(4)),
        (json!([A]), json!([]), json!([]), json!([A]))
    );
}

/// Row 5: an item that was never A's, added to B by hand.
#[test]
fn row_5_an_item_added_to_the_kept_event_by_hand_stays_in_it() {
    let rows = [row(1, A_M), row(2, B_M), row(3, C_M), row(4, C_M)];
    // One before the merge, one after it.
    let w = World::of(&records(), &rows, &[fix(3, "event_add", B)]);
    let id = w.merge(A, B);
    w.correct(&[fix(4, "event_add", B_M)]);
    let p = w.split(&id);
    assert_eq!(counts(&p), (1, 3, 0, 4));
    assert_eq!(w.total(&format!("event:{B} event:{C}")), 2);
}

/// Row 6: C merged into A, A merged into B; A is split off.
#[test]
fn row_6_what_is_merged_into_the_returning_event_goes_with_it() {
    let rows = [row(1, A_M), row(2, C_M), row(3, C_M), row(4, B_M)];
    let w = World::of(&records(), &rows, &[]);
    w.merge(C, A);
    let id = w.merge(A, B);
    let p = w.split(&id);
    assert_eq!(counts(&p), (3, 1, 0, 4));
    // A's span covers C, which stays merged into it.
    assert_eq!(
        (p.start.as_deref(), p.end.as_deref()),
        (Some("2026-08-01"), Some("2026-09-20"))
    );
    assert_eq!(w.listed(C)["merged_into"], json!(A));
}

/// Row 6, the kept side: the event that stays is the current holder, the
/// record at the end of B's own chain.
#[test]
fn row_6_the_kept_event_is_the_holder_at_the_end_of_the_chain() {
    let rows = [row(1, A_M), row(2, B_M), row(3, C_M)];
    let w = World::of(&records(), &rows, &[]);
    let id = w.merge(A, B);
    w.merge(B, C);
    let p = w.split(&id);
    assert_eq!((p.returning.id.as_str(), p.kept.id.as_str()), (A, C));
    assert_eq!(p.kept.name, "Gamma");
    assert_eq!(counts(&p), (1, 2, 0, 3));
}

/// Row 7: the returning record carries `removed` (a hand edit: `remove`
/// refuses a merged record). Split off it would be a removed event and
/// its items in no event, so `unmerge` refuses it, with nothing written,
/// and the preview and both listings say `undoable: false` in its words.
#[test]
fn row_7_a_returning_record_that_carries_removed_is_refused_by_unmerge_and_shown_not_undoable() {
    let w = World::of(&records(), &[row(1, A_M), row(2, B_M)], &[]);
    let id = w.merge(A, B);
    let dir = w.root().join(events::EVENTS_DIR);
    let mut a = events::parse_bytes(&fs::read(dir.join(format!("{A}.md"))).unwrap(), A).unwrap();
    a.removed = Some("2026-10-11".into());
    put(&dir, &a);
    // Merged away, `removed` has no effect: the item is in B.
    assert_eq!(w.listed(B)["items"], json!(2));

    let before = tree(w.root());
    let e = w.editor().unmerge(&id).unwrap_err();
    let why = format!("{e:#}");
    assert!(
        why.contains(&format!("{id}: {A} carries removed 2026-10-11"))
            && why.contains(&format!("run `scout events restore {A}` first"))
            && why.contains("nothing was changed"),
        "{why}"
    );
    assert!(tree(w.root()) == before, "a refused unmerge wrote");

    let p = w.editor().unmerge_preview(&id).unwrap();
    assert!(tree(w.root()) == before, "the preview wrote");
    assert_eq!(
        (p.undoable, p.why_not.as_deref()),
        (false, Some(why.as_str()))
    );
    assert_eq!((p.returning.id.as_str(), p.kept.id.as_str()), (A, B));
    assert_eq!(
        (p.returning_items, p.kept_items, p.in_both, p.kept_items_now),
        (None, None, None, None)
    );
    assert_eq!((&p.start, &p.end), (&None, &None));
    let line = serde_json::to_value(&p).unwrap();
    for absent in ["returning_items", "kept_items", "in_both", "kept_items_now"] {
        assert!(line.get(absent).is_none(), "{line}");
    }
    // The two listings mirror it, word for word.
    let merges = w.editor().merges().unwrap();
    assert_eq!(
        (merges[0].undoable, merges[0].why_not.as_deref()),
        (Some(false), Some(why.as_str()))
    );
    let from = w.listed(B)["merged_from"][0].clone();
    assert_eq!(
        (&from["merge_id"], &from["undoable"], &from["why_not"]),
        (&json!(id), &json!(false), &json!(why))
    );

    // Restored, it is undoable again, and the preview is the outcome.
    w.editor().restore(A).unwrap();
    assert!(w.editor().merges().unwrap()[0].undoable.is_none());
    let p = w.split(&id);
    assert_eq!(counts(&p), (1, 1, 0, 2));
}

/// Row 8: no merge in effect of that id. The error is `unmerge`'s own,
/// and nothing is computed or written.
#[test]
fn row_8_an_id_that_names_no_merge_in_effect_is_the_error_unmerge_gives() {
    let w = World::of(&records(), &[row(1, A_M), row(2, B_M)], &[]);
    // Merged by hand: no line, so no id to ask with.
    let dir = w.root().join(events::EVENTS_DIR);
    let mut c = events::parse_bytes(&fs::read(dir.join(format!("{C}.md"))).unwrap(), C).unwrap();
    c.merged_into = Some(B.into());
    put(&dir, &c);
    let same = |id: &str, needle: &str| {
        let before = tree(w.root());
        let p = format!("{:#}", w.editor().unmerge_preview(id).unwrap_err());
        assert!(tree(w.root()) == before, "{id}: the preview wrote");
        let u = format!("{:#}", w.editor().unmerge(id).unwrap_err());
        assert!(tree(w.root()) == before, "{id}: a refused unmerge wrote");
        assert_eq!(p, u, "{id}");
        assert!(p.contains(needle), "{id}: {p}");
    };
    // No log at all, then an id the log does not hold.
    same("m000001", "no such merge");
    let id = w.merge(A, B);
    same("m000009", "no such merge");
    same(C, "no such merge");
    // Changed by other means: A is merged elsewhere.
    let mut a = events::parse_bytes(&fs::read(dir.join(format!("{A}.md"))).unwrap(), A).unwrap();
    a.merged_into = Some(C.into());
    put(&dir, &a);
    same(&id, "is no longer merged into");
    a.merged_into = Some(B.into());
    put(&dir, &a);
    // Undone.
    w.editor().unmerge(&id).unwrap();
    same(&id, "already undone");

    // Logged and never applied: `unmerge` would withdraw the line, which
    // moves no item; the preview says so and computes nothing.
    let stopped = w
        .editor()
        .with_hook(|s| match s {
            Step::Written(MERGE_LOG) => Err(std::io::Error::other("stop")),
            _ => Ok(()),
        })
        .merge(&merge_req(A, B, By::Dominik));
    assert!(stopped.is_err());
    let before = tree(w.root());
    let e = format!("{:#}", w.editor().unmerge_preview("m000003").unwrap_err());
    assert!(
        e.contains("was never applied") && e.contains("would only withdraw its line"),
        "{e}"
    );
    assert!(tree(w.root()) == before);
    assert!(w.editor().unmerge("m000003").unwrap().withdrawn);
}

/// Row 9: lines of the layer file the reader passes over (not JSON, a
/// wrongly typed field, no key, not UTF-8), an earlier row of a repeated
/// key, an `events` value that is not a list of ids, and correction
/// lines it does not apply (malformed, an unknown key, a numbered line
/// taken back). Each names A or B and has a work; the build passes them
/// over, and so does the preview.
#[test]
fn row_9_lines_the_reader_passes_over_are_passed_over_as_the_index_build_does() {
    let mut odd_events = row(4, B_M);
    odd_events["events"] = json!([7, A_M, null]);
    let mut not_a_list = row(5, B_M);
    not_a_list["events"] = json!(A_M);
    let mut layer = Vec::new();
    let mut push = |bytes: &[u8]| {
        layer.extend_from_slice(bytes);
        layer.push(b'\n');
    };
    push(row(1, A_M).to_string().as_bytes());
    // Not rows.
    push(br#"{"key":"x:6","event":"2026-09/E01""#);
    push(br#"{"key":"x:7","themes":"T","event":"2026-09/E01"}"#);
    push(br#"{"event":"2026-09/E01"}"#);
    push(b"{\"key\":\"x:8\",\"event\":\"2026-09/E01\",\"event_name\":\"\xff\"}");
    push(b"");
    // The last valid row of a key is the one kept.
    push(row(2, A_M).to_string().as_bytes());
    push(row(2, B_M).to_string().as_bytes());
    push(row(3, B_M).to_string().as_bytes());
    push(br#"{"key":"x:3","themes":7,"event":"2026-09/E01"}"#);
    push(odd_events.to_string().as_bytes());
    push(not_a_list.to_string().as_bytes());
    let numbered = |id: &str, n: usize, rest: Value| {
        let mut l =
            json!({"id": id, "at": NOW, "date": TODAY, "by": "dominik", "key": format!("x:{n}")});
        l.as_object_mut()
            .unwrap()
            .extend(rest.as_object().unwrap().clone());
        l
    };
    let corrections = [
        json!("not an object"),
        // A wrongly typed field: the whole line is bad.
        json!({"key": "x:1", "event_add": B}),
        // No tag row of that key.
        fix(9, "event_add", A),
        // Taken back by an undo line: not applied.
        numbered("c000001", 3, json!({"event_add": [A]})),
        numbered("c000002", 3, json!({"undoes": "c000001"})),
        // A numbered line `scout correct` does not write: bad.
        numbered("c000003", 5, json!({"event_add": [A], "note": "x"})),
        // Applied.
        numbered("c000004", 5, json!({"event_add": [C]})),
    ];
    let w = World::new(
        &records(),
        std::str::from_utf8(&layer[..0]).unwrap(),
        &corrections,
        &[1, 2, 3, 4, 5, 6, 7, 8, 9],
    );
    fs::write(w.root().join(topics::LAYER_FILE), &layer).unwrap();
    let id = w.merge(A, B);
    let p = w.split(&id);
    // A: 1, and 4 by its events list. B: 2, 3, 4, 5.
    assert_eq!(counts(&p), (2, 4, 1, 5));
    assert_eq!(w.total(&format!("event:{B} event:{C}")), 1);
}

/// Row 10: the counts over a large layer file with a line over the
/// reader's line bound in it: the long line is a bad line to the preview
/// as to the build, and the other rows count. (What the reader holds in
/// memory, and that it reads regular files only, is not shown here: see
/// `layer_files_regular_only.rs` and the tests below.) Prints the time
/// one preview takes over 15,000 tag rows.
#[test]
fn row_10_counts_over_a_large_layer_file_with_an_over_long_line() {
    const ROWS: usize = 15_000;
    let mut layer = String::new();
    // Works exist for the rows of A and B only; the rest are tag rows of
    // a monthly event no record holds.
    let mut docs = Vec::new();
    for n in 1..=ROWS {
        let event = match n % 500 {
            0 => A_M,
            1 => B_M,
            _ => "2026-01/E99",
        };
        if event != "2026-01/E99" {
            docs.push(n);
        }
        let mut r = row(n, event);
        if n % 1000 == 0 {
            r["events"] = json!([B_M]);
        }
        layer.push_str(&format!("{r}\n"));
    }
    // One line of A over the bound: a bad line to every reader.
    let long = ROWS + 1;
    let mut r = row(long, A_M);
    r["themes"] = json!(["t".repeat(topics::MAX_LINE_BYTES)]);
    layer.push_str(&format!("{r}\n"));
    docs.push(long);
    let corrections: Vec<Value> = (1..=200).map(|n| fix(n * 7, "event_add", B)).collect();
    let w = World::new(&records(), &layer, &corrections, &docs);
    let id = w.merge(A, B);

    let started = std::time::Instant::now();
    let timed = w.editor().unmerge_preview(&id).unwrap();
    let took = started.elapsed();
    eprintln!(
        "unmerge_preview over {ROWS} tag rows ({} bytes) and {} corrections: {took:?}",
        layer.len(),
        corrections.len()
    );
    // 30 rows of A (15 of them also in B by `events`), 30 of B, and the
    // corrections add B to rows that have no work: counted here, where an
    // item is a tag row. `scout events` counts the works.
    let a_rows = ROWS / 500;
    assert_eq!(timed.returning_items, Some(a_rows));
    // (None of the corrected rows is a row of A; one, 1001, is B's.)
    let added = (1..=200).map(|n| n * 7).filter(|n| n % 500 > 1).count();
    assert_eq!(added, 199);
    assert_eq!(timed.kept_items, Some(a_rows / 2 + a_rows + added));
    assert_eq!(timed.in_both, Some(a_rows / 2));
    assert!(took < std::time::Duration::from_secs(20), "{took:?}");

    // Against the engine, over the rows that have works: take the
    // corrections away (their rows have none) and compare.
    fs::remove_file(w.root().join(topics::CORRECTIONS_FILE)).unwrap();
    let p = w.split(&id);
    assert_eq!(
        counts(&p),
        (a_rows, a_rows / 2 + a_rows, a_rows / 2, 2 * a_rows)
    );
}

/// What is at each path under `root`, without opening anything that is
/// not a regular file (a FIFO would wait): the kind, and for a regular
/// file its bytes and time.
#[cfg(unix)]
fn shape(root: &Path) -> Vec<(PathBuf, String, Vec<u8>, Option<std::time::SystemTime>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            let m = fs::symlink_metadata(&p).unwrap();
            let kind = format!("{:?}", m.file_type());
            if m.is_dir() {
                stack.push(p.clone());
                out.push((p, kind, vec![], None));
            } else if m.is_file() {
                let bytes = fs::read(&p).unwrap();
                out.push((p, kind, bytes, m.modified().ok()));
            } else {
                out.push((p, kind, vec![], None));
            }
        }
    }
    out.sort();
    out
}

/// A layer file that is not a regular file stops the preview with an
/// error, at once, and nothing is written: a link to `/dev/zero` (bytes
/// without end), a FIFO nobody writes to (a plain open waits), a
/// directory. For the tag rows and for the corrections file. No count is
/// given without a file that is there and was not read.
#[cfg(unix)]
#[test]
fn a_layer_file_that_is_not_a_regular_file_stops_the_preview_and_nothing_is_written() {
    use std::os::unix::fs::symlink;
    /// Puts the thing at the path.
    type Make = fn(&Path);
    let bad: [(&str, Make); 3] = [
        ("a link to /dev/zero", |p| symlink("/dev/zero", p).unwrap()),
        ("a FIFO", |p| {
            use std::os::unix::ffi::OsStrExt;
            let c = std::ffi::CString::new(p.as_os_str().as_bytes()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        }),
        ("a directory", |p| fs::create_dir(p).unwrap()),
    ];
    for file in [topics::LAYER_FILE, topics::CORRECTIONS_FILE] {
        for (what, make) in bad {
            let w = World::of(
                &records(),
                &[row(1, A_M), row(2, B_M)],
                &[fix(2, "event_add", A)],
            );
            let id = w.merge(A, B);
            assert_eq!(
                counts(&w.editor().unmerge_preview(&id).unwrap()),
                (2, 1, 1, 2)
            );
            let path = w.root().join(file);
            fs::remove_file(&path).unwrap();
            make(&path);
            let before = shape(w.root());

            let (editor, log_id) = (w.editor(), id.clone());
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(
                    editor
                        .unmerge_preview(&log_id)
                        .map_err(|e| format!("{e:#}")),
                );
            });
            let answer = rx
                .recv_timeout(std::time::Duration::from_secs(20))
                .unwrap_or_else(|_| panic!("{file} as {what}: the preview did not answer"));
            let e = answer.expect_err(&format!("{file} as {what}"));
            assert!(
                e.contains("is not a regular file") && e.contains(&path.display().to_string()),
                "{file} as {what}: {e}"
            );
            assert!(
                shape(w.root()) == before,
                "{file} as {what}: a preview wrote"
            );
        }
    }
}

/// A layer file that is a link to a regular file is read through the
/// link: the preview is the one over the files themselves.
#[cfg(unix)]
#[test]
fn a_layer_file_that_links_to_a_regular_file_is_previewed_as_the_file() {
    let w = World::of(
        &records(),
        &[row(1, A_M), row(2, B_M)],
        &[fix(2, "event_add", A)],
    );
    let id = w.merge(A, B);
    let plain = w.editor().unmerge_preview(&id).unwrap();
    assert_eq!(counts(&plain), (2, 1, 1, 2));
    let elsewhere = tempfile::tempdir().unwrap();
    for (n, file) in [topics::LAYER_FILE, topics::CORRECTIONS_FILE]
        .iter()
        .enumerate()
    {
        let (at, target) = (w.root().join(file), elsewhere.path().join(n.to_string()));
        fs::rename(&at, &target).unwrap();
        std::os::unix::fs::symlink(&target, &at).unwrap();
        assert_eq!(w.editor().unmerge_preview(&id).unwrap(), plain, "{file}");
    }
}

/// The preview writes nothing: the archive is byte for byte the same,
/// with no file added, after previews of every kind of answer. The
/// snapshot is taken before the first call.
#[test]
fn the_preview_leaves_the_archive_byte_for_byte_as_it_was() {
    let w = World::of(
        &records(),
        &[row(1, A_M), row(2, B_M), row(3, C_M)],
        &[fix(1, "event_add", C)],
    );
    let m1 = w.merge(A, B);
    let m2 = w.merge(C, B);
    w.editor().unmerge(&m2).unwrap();
    let dir = w.root().join(events::EVENTS_DIR);
    // The times of the files too: nothing is rewritten in place.
    let stamps = |root: &Path| -> Vec<(PathBuf, std::time::SystemTime, u64)> {
        let mut out: Vec<_> = tree(root)
            .into_keys()
            .map(|p| {
                let m = fs::metadata(&p).unwrap();
                (p, m.modified().unwrap(), m.len())
            })
            .collect();
        out.sort();
        out
    };
    let (before, stamped) = (tree(w.root()), stamps(w.root()));

    let p = w.editor().unmerge_preview(&m1).unwrap();
    assert_eq!(counts(&p), (1, 1, 0, 2));
    assert!(w.editor().unmerge_preview(&m2).is_err());
    assert!(w.editor().unmerge_preview("m000009").is_err());
    // Twice gives the same answer.
    assert_eq!(
        w.editor().unmerge_preview(&format!(" {m1} ")).unwrap().kept,
        p.kept
    );
    assert_eq!(w.editor().unmerge_preview(&m1).unwrap(), p);
    assert!(tree(w.root()) == before, "a preview wrote");
    assert_eq!(stamps(w.root()), stamped);

    // Not undoable: still nothing written.
    let mut a = events::parse_bytes(&fs::read(dir.join(format!("{A}.md"))).unwrap(), A).unwrap();
    a.removed = Some("2026-10-11".into());
    put(&dir, &a);
    let (before, stamped) = (tree(w.root()), stamps(w.root()));
    assert!(!w.editor().unmerge_preview(&m1).unwrap().undoable);
    assert!(tree(w.root()) == before, "a preview wrote");
    assert_eq!(stamps(w.root()), stamped);
}

/// What an item is here. The preview does not read the index, so it
/// counts tag rows; `scout events` counts the works the rows join. A row
/// that joins no work is counted by the preview alone.
#[test]
fn an_item_is_a_tag_row_whether_or_not_a_work_joins_it() {
    let rows = [row(1, A_M), row(2, A_M), row(3, B_M)];
    // No work for row 2.
    let w = World::new(&records(), &jsonl(&rows), &[], &[1, 3]);
    let id = w.merge(A, B);
    let p = w.editor().unmerge_preview(&id).unwrap();
    assert_eq!(counts(&p), (2, 1, 0, 3));
    w.editor().unmerge(&id).unwrap();
    assert_eq!(w.listed(A)["items"], json!(1));
}

/// With no layer file there are no items, and the span is still said.
#[test]
fn an_archive_without_a_layer_file_has_no_items_to_move() {
    let w = World::of(&records(), &[], &[]);
    fs::remove_file(w.root().join(topics::LAYER_FILE)).unwrap();
    let id = w.merge(A, B);
    let p = w.split(&id);
    assert_eq!(counts(&p), (0, 0, 0, 0));
    assert_eq!(p.start.as_deref(), Some("2026-09-05"));
}

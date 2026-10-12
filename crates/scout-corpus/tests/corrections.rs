//! Seam tests for `scout correct`: the one writer of
//! `analysis/topics/corrections.jsonl`. Every operation appends one line;
//! what the writer says the item's tags are is what the reader has (an
//! index build on the temp archive, then a search hit's `topics`); a
//! refusal leaves every file of the archive as it was.

use scout_corpus::corrections::{Corrected, CorrectionEditor, MAX_FILE_BYTES, MAX_NAME_CHARS};
use scout_corpus::registry::{CorpusConfig, CorpusKind, DocumentUnit, FieldMap};
use scout_corpus::topics::{self, TopicInfo};
use scout_corpus::{events, Corpus, SearchRequest};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const NOW: &str = "2026-10-12T08:00:00.000Z";
const FIELDS: &str = "2026-09-fields-declaration";
const FOLLOW: &str = "2026-10-follow-up";
const GEMINI: &str = "2026-09-gemini-launch";
const GONE: &str = "2026-08-removed-event";
const MATHS: &str = "AI and mathematics";
const VIBE: &str = "Vibe coding and AI-assisted programming";

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

fn record(root: &Path, id: &str, front: &str) {
    write(
        root,
        &format!("{}/{id}.md", events::EVENTS_DIR),
        &format!("---\nid: {id}\n{front}---\n\nAbout it.\n"),
    );
}

struct Fx {
    src: tempfile::TempDir,
    _data: tempfile::TempDir,
    corpus: Corpus,
}

/// Four works. x:1 and x:2 are in the Fields declaration (`2026-09/E21`),
/// x:3 in the Gemini launch (`2026-09/E02`), x:4 has no tags to speak of.
/// Records: the two events, a follow-up merged into the declaration, and
/// a removed event. The theme list has three themes.
fn fixture(with_records: bool) -> Fx {
    let (src, data) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let root = src.path();
    for n in 1..=4 {
        write(
            root,
            &format!("readings/works/w-{n}.md"),
            &format!(
                "---\ntitle: W{n}\nauthor: A\ntype: tweet\nsource_system: x\nsource_id: \"{n}\"\n---\n\n> A proof, number {n}.\n\nhighlighted_at: 2026-09-0{n}\n\n---\n"
            ),
        );
    }
    let row = |n: u32, themes: Value, entities: Value, event: Value, name: Value| {
        json!({"key": format!("x:{n}"), "path": format!("readings/works/w-{n}.md"),
               "date": format!("2026-09-0{n}"), "ai_related": true, "themes": themes,
               "entities": entities, "event": event, "event_name": name})
        .to_string()
    };
    write(
        root,
        topics::LAYER_FILE,
        &[
            row(
                1,
                json!([MATHS]),
                json!(["Terence Tao"]),
                json!("2026-09/E21"),
                json!("Fields declaration"),
            ),
            row(
                2,
                json!([MATHS, VIBE]),
                json!([]),
                json!("2026-09/E21"),
                json!("Fields declaration"),
            ),
            row(
                3,
                json!(["Off the list"]),
                json!(["GPT-6 Astra"]),
                json!("2026-09/E02"),
                json!("Gemini launch"),
            ),
            row(4, json!([]), json!([]), Value::Null, Value::Null),
            String::new(),
        ]
        .join("\n"),
    );
    write(
        root,
        "analysis/context/themes.json",
        &json!({"meta": {}, "themes": [
            {"id": "T01", "name": MATHS, "definition": "d"},
            {"id": "T02", "name": VIBE},
            {"id": "T03", "name": "Metaphor and cognition"},
        ]})
        .to_string(),
    );
    if with_records {
        record(
            root,
            FIELDS,
            "name: Fields declaration\nstart: 2026-09-01\nmembers: [2026-09/E21]\norigin: model\n",
        );
        record(
            root,
            FOLLOW,
            &format!(
                "name: Follow-up\nmembers: [2026-10/E04]\norigin: model\nmerged_into: {FIELDS}\n"
            ),
        );
        record(
            root,
            GEMINI,
            "name: Gemini launch\nstart: 2026-09-04\nmembers: [2026-09/E02]\norigin: model\n",
        );
        record(
            root,
            GONE,
            "name: Removed\nmembers: [2026-08/E01]\norigin: model\nremoved: 2026-10-11\n",
        );
    }
    let corpus = Corpus::from_config(
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
        data.path(),
    );
    Fx {
        src,
        _data: data,
        corpus,
    }
}

/// Every file under `root` with its bytes.
fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).unwrap() {
            let e = e.unwrap();
            let t = e.file_type().unwrap();
            if t.is_dir() {
                stack.push(e.path());
            } else if t.is_symlink() {
                out.insert(e.path(), b"<link>".to_vec());
            } else {
                out.insert(e.path(), fs::read(e.path()).unwrap());
            }
        }
    }
    out
}

impl Fx {
    fn root(&self) -> &Path {
        self.src.path()
    }

    fn editor(&self) -> CorrectionEditor {
        CorrectionEditor::open(self.root()).unwrap().with_now(NOW)
    }

    fn file(&self) -> PathBuf {
        self.root().join(topics::CORRECTIONS_FILE)
    }

    fn lines(&self) -> Vec<Value> {
        let Ok(text) = fs::read_to_string(self.file()) else {
            return vec![];
        };
        assert!(text.is_empty() || text.ends_with('\n'), "{text:?}");
        text.lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    /// The item's tags as the READER has them: an index build of the
    /// archive, then the `topics` of the work's search hit.
    fn reader(&self, n: u32) -> TopicInfo {
        self.corpus.build_index(false).unwrap();
        let mut r = SearchRequest::new(format!("\"number {n}\""));
        r.limit = 10;
        self.corpus.search(&r).unwrap().results[0]
            .topics
            .clone()
            .expect("tags")
    }

    fn hits(&self, q: &str) -> Vec<String> {
        self.corpus.build_index(false).unwrap();
        let mut r = SearchRequest::new(q);
        r.limit = 100;
        let mut keys: Vec<String> = self
            .corpus
            .search(&r)
            .unwrap()
            .results
            .iter()
            .filter_map(|d| d.topics.as_ref().map(|t| t.key.clone()))
            .collect();
        keys.sort();
        keys
    }

    /// The operation wrote one line more, and the reader has the tags the
    /// writer reports.
    fn agreed(&self, n: u32, lines_before: usize, done: &Corrected) -> TopicInfo {
        assert_eq!(self.lines().len(), lines_before + 1, "{}", done.id);
        assert!(done.reindex_needed);
        assert_eq!(done.key, format!("x:{n}"));
        let read = self.reader(n);
        assert_eq!(read, done.tags, "{}: writer and reader disagree", done.id);
        read
    }

    /// `op` must fail with `needle` and leave every file as it was.
    fn refused<T: std::fmt::Debug>(&self, needle: &str, op: impl FnOnce() -> anyhow::Result<T>) {
        let before = tree(self.root());
        let e = format!("{:#}", op().expect_err(needle));
        assert!(e.contains(needle), "want {needle:?} in {e}");
        assert!(tree(self.root()) == before, "{needle}: a file changed");
    }
}

fn strs(l: &[&str]) -> Vec<String> {
    l.iter().map(|s| s.to_string()).collect()
}

#[test]
fn every_operation_appends_one_line_and_the_reader_agrees() {
    let fx = fixture(true);
    let e = fx.editor();
    assert!(!fx.file().exists());
    let model4 = fx.reader(4);
    assert!(model4.themes.is_empty() && model4.corrected.is_empty());

    // A theme, given in another case: written as the list spells it.
    let c = e.add_theme("x:4", "ai AND  mathematics").unwrap();
    assert_eq!(
        (c.id.as_str(), c.op.as_str(), c.value.as_str()),
        ("c000001", "theme_add", MATHS)
    );
    assert_eq!(c.tool, "scout correct theme-add");
    assert_eq!(
        c.file,
        fx.file().canonicalize().unwrap().display().to_string()
    );
    assert_eq!(
        fx.lines()[0],
        json!({"id": "c000001", "at": NOW, "date": "2026-10-12", "by": "dominik",
               "key": "x:4", "theme_add": [MATHS]})
    );
    let t = fx.agreed(4, 0, &c);
    assert_eq!((t.themes, t.corrected), (strs(&[MATHS]), strs(&["themes"])));
    assert_eq!(
        fx.hits(&format!("theme:\"{MATHS}\"")),
        ["x:1", "x:2", "x:4"]
    );

    // Out again, and a model's theme off another item (off the list too).
    let c = e
        .remove_theme("x:2", "vibe coding and ai-assisted programming")
        .unwrap();
    assert_eq!(c.value, VIBE);
    assert_eq!(fx.agreed(2, 1, &c).themes, [MATHS]);
    let c = e.remove_theme("x:3", "Off the list").unwrap();
    assert!(fx.agreed(3, 2, &c).themes.is_empty());
    assert_eq!(
        fx.lines()[2],
        json!({"id": "c000003", "at": NOW, "date": "2026-10-12", "by": "dominik",
               "key": "x:3", "theme_remove": ["Off the list"]})
    );

    // An entity: free text, trimmed, with its type on the line.
    let c = e.add_entity("x:4", "  O’Reilly Media ", "org").unwrap();
    assert_eq!(c.entity_type.as_deref(), Some("org"));
    assert_eq!(
        fx.lines()[3],
        json!({"id": "c000004", "at": NOW, "date": "2026-10-12", "by": "dominik",
               "key": "x:4", "entity_add": ["O’Reilly Media"], "entity_type": "org"})
    );
    let t = fx.agreed(4, 3, &c);
    assert_eq!(t.entities, ["O’Reilly Media"]);
    assert_eq!(t.corrected, strs(&["themes", "entities"]));
    assert_eq!(fx.hits("entity:\"o'reilly media\""), ["x:4"]);
    // Removed under the fold, written as the item has it.
    let c = e.remove_entity("x:1", "terence  TAO").unwrap();
    assert_eq!(c.value, "Terence Tao");
    assert!(fx.agreed(1, 4, &c).entities.is_empty());

    // Into an event, by a monthly id: the record's id is written.
    let c = e.add_to_event("x:4", "2026-09/e02").unwrap();
    assert_eq!(c.value, GEMINI);
    assert_eq!(
        fx.lines()[5],
        json!({"id": "c000006", "at": NOW, "date": "2026-10-12", "by": "dominik",
               "key": "x:4", "event_add": [GEMINI]})
    );
    let t = fx.agreed(4, 5, &c);
    assert_eq!(t.events, Some(strs(&[GEMINI])));
    assert_eq!(t.corrected, strs(&["themes", "entities", "events"]));
    assert_eq!(fx.hits(&format!("event:{GEMINI}")), ["x:3", "x:4"]);

    // Out of its model event, named by a record merged into it.
    let c = e.remove_from_event("x:2", FOLLOW).unwrap();
    assert_eq!(c.value, FIELDS);
    let t = fx.agreed(2, 6, &c);
    assert_eq!(
        (t.events, t.event, t.event_name),
        (Some(vec![]), None, None)
    );
    assert_eq!(t.corrected, strs(&["themes", "event", "events"]));
    assert_eq!(fx.hits(&format!("event:{FIELDS}")), ["x:1"]);

    // No other item moved, and the model's rows were never written.
    assert_eq!(fx.reader(1).themes, [MATHS]);
    let build = fx.corpus.build_index(false).unwrap().topics.unwrap();
    assert_eq!(
        (
            build.corrections,
            build.corrections_bad_lines,
            build.corrections_undone
        ),
        (7, 0, 0)
    );
}

#[test]
fn a_refusal_writes_nothing() {
    let fx = fixture(true);
    let e = fx.editor();
    // One correction first, so the file exists and must stay as it is.
    e.add_theme("x:4", MATHS).unwrap();
    let long = "n".repeat(MAX_NAME_CHARS + 1);
    let fits = "é".repeat(MAX_NAME_CHARS);

    fx.refused("x:404: no tag row", || e.add_theme("x:404", MATHS));
    fx.refused("x:404: no tag row", || e.remove_entity("x:404", "A"));
    fx.refused("the key is empty", || e.add_theme("  ", MATHS));
    fx.refused("the key holds a control character", || {
        e.add_theme("x:\n4", MATHS)
    });
    fx.refused("not a theme of analysis/context/themes.json", || {
        e.add_theme("x:4", "Off the list")
    });
    fx.refused("the theme is empty", || e.add_theme("x:4", " "));
    fx.refused("x:4: already has the theme \"AI and mathematics\"", || {
        e.add_theme("x:4", "AI AND MATHEMATICS")
    });
    fx.refused("x:4: has no theme \"Metaphor and cognition\"", || {
        e.remove_theme("x:4", "Metaphor and cognition")
    });
    fx.refused("the entity holds a control character (U+0007)", || {
        e.add_entity("x:4", "a\u{7}b", "person")
    });
    fx.refused("the entity holds a control character (U+2028)", || {
        e.add_entity("x:4", "a\u{2028}b", "person")
    });
    fx.refused("the entity is empty", || {
        e.add_entity("x:4", "   ", "person")
    });
    fx.refused("longer than 200 characters", || {
        e.add_entity("x:4", &long, "person")
    });
    fx.refused("\"animal\": not an entity type", || {
        e.add_entity("x:4", "A", "animal")
    });
    fx.refused("x:1: already has the entity \"Terence Tao\"", || {
        e.add_entity("x:1", "terence tao", "person")
    });
    fx.refused("x:4: has no entity \"Terence Tao\"", || {
        e.remove_entity("x:4", "Terence Tao")
    });
    fx.refused("2031-01/E09: no event record holds this id", || {
        e.add_to_event("x:4", "2031-01/E09")
    });
    fx.refused(
        "2026-08-removed-event: a removed event; restore it first",
        || e.add_to_event("x:4", "2026-08/E01"),
    );
    fx.refused(
        "x:1: already in the event 2026-09-fields-declaration",
        || e.add_to_event("x:1", FOLLOW),
    );
    fx.refused("x:4: not in the event 2026-09-gemini-launch", || {
        e.remove_from_event("x:4", GEMINI)
    });
    fx.refused("c000009: no such correction", || e.undo("c000009"));
    fx.refused("m000001: no such correction", || e.undo("m000001"));
    // 200 characters fit, whatever their bytes.
    assert_eq!(e.add_entity("x:4", &fits, "other").unwrap().value, fits);

    // Paths: `..`, an archive that is no folder, a topics folder that is a
    // link or is missing.
    let before = tree(fx.root());
    let dotted = fx.root().join("readings").join("..");
    let err = |r: anyhow::Result<CorrectionEditor>| format!("{:#}", r.err().unwrap());
    assert!(err(CorrectionEditor::open(&dotted)).contains("`..` component"));
    assert!(err(CorrectionEditor::open(&fx.root().join("nowhere"))).contains("not a folder"));
    let other = tempfile::tempdir().unwrap();
    fs::create_dir_all(other.path().join("analysis")).unwrap();
    assert!(err(CorrectionEditor::open(other.path())).contains("no topics folder"));
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            fx.root().join("analysis/topics"),
            other.path().join("analysis/topics"),
        )
        .unwrap();
        assert!(
            err(CorrectionEditor::open(other.path())).contains("the topics folder is a symlink")
        );
    }
    assert!(tree(fx.root()) == before);

    // No theme list: a theme is not added, but one is still removed.
    fs::remove_file(fx.root().join("analysis/context/themes.json")).unwrap();
    fx.refused("no theme list", || e.add_theme("x:1", VIBE));
    e.remove_theme("x:4", MATHS).unwrap();
    for (bad, why) in [
        ("{", "not JSON"),
        ("{\"themes\": {}}", "no `themes` list"),
        (
            "{\"themes\": [{\"name\": 3}]}",
            "theme 1 has no text `name`",
        ),
    ] {
        write(fx.root(), "analysis/context/themes.json", bad);
        fx.refused(why, || e.add_theme("x:1", VIBE));
    }
}

#[test]
fn undo_takes_back_exactly_one_correction_of_each_kind() {
    let fx = fixture(true);
    let e = fx.editor();
    let model: Vec<TopicInfo> = (1..=4).map(|n| fx.reader(n)).collect();
    let made = [
        e.add_theme("x:1", VIBE).unwrap(),
        e.remove_theme("x:1", MATHS).unwrap(),
        e.add_entity("x:1", "GPT-6 Astra", "model").unwrap(),
        e.remove_entity("x:1", "Terence Tao").unwrap(),
        e.add_to_event("x:1", GEMINI).unwrap(),
        e.remove_from_event("x:1", FIELDS).unwrap(),
    ];
    let all = fx.reader(1);
    assert_eq!(
        (all.themes.clone(), all.entities.clone(), all.events.clone()),
        (strs(&[VIBE]), strs(&["GPT-6 Astra"]), Some(strs(&[GEMINI])))
    );
    assert_eq!(
        all.corrected,
        strs(&["themes", "entities", "event", "events"])
    );
    // Each undone alone, in the order made: each is the last change of its
    // own theme, entity or event.
    for (i, c) in made.iter().enumerate() {
        let u = e.undo(&c.id).unwrap();
        assert_eq!(
            (u.op.as_str(), u.undoes.as_deref(), u.value.as_str()),
            ("undo", Some(c.id.as_str()), c.value.as_str())
        );
        assert_eq!(u.tool, "scout correct undo");
        assert_eq!(
            fx.lines()[6 + i],
            json!({"id": format!("c{:06}", 7 + i), "at": NOW, "date": "2026-10-12",
                   "by": "dominik", "key": "x:1", "undoes": c.id})
        );
        fx.agreed(1, 6 + i, &u);
    }
    // All undone: the reader has the model's tags again, nothing marked.
    for n in 1..=4 {
        assert_eq!(fx.reader(n), model[n as usize - 1]);
    }
    let l = e.list("x:1").unwrap();
    assert_eq!(l.effective, l.model);
    assert_eq!(l.model, model[0]);
    let build = fx.corpus.build_index(false).unwrap().topics.unwrap();
    assert_eq!((build.corrections, build.corrections_undone), (0, 6));

    // Not undone twice; an undo line is not undone; twelve lines stay.
    fx.refused("c000001: already undone, by c000007", || e.undo("c000001"));
    fx.refused(
        "c000007 is the undo of c000001; an undo is not undone",
        || e.undo(" c000007 "),
    );
    // The same correction made again is a new line, undone on its own.
    let again = e.add_theme("x:1", VIBE).unwrap();
    assert_eq!(again.id, "c000013");
    assert_eq!(fx.agreed(1, 12, &again).themes, [MATHS, VIBE]);
}

#[test]
fn undo_is_last_change_first_per_item_and_value() {
    let fx = fixture(true);
    let e = fx.editor();
    // x:4: the theme in, out, in again; an entity; another theme. x:1: the
    // same theme name on another item.
    let c1 = e.add_theme("x:4", MATHS).unwrap();
    let c2 = e.remove_theme("x:4", MATHS).unwrap();
    let c3 = e.add_theme("x:4", MATHS).unwrap();
    let c4 = e.add_entity("x:4", "Astra", "model").unwrap();
    let c5 = e.add_theme("x:4", VIBE).unwrap();
    let c6 = e.remove_theme("x:1", MATHS).unwrap();
    assert_eq!(fx.reader(4).themes, [MATHS, VIBE]);

    fx.refused(
        "c000001: c000002 (theme_remove \"AI and mathematics\") later changed the same theme of x:4; undo is last change first",
        || e.undo(&c1.id),
    );
    fx.refused("c000002: c000003 (theme_add", || e.undo(&c2.id));
    let l = e.list("x:4").unwrap();
    let undoable: Vec<(&str, bool)> = l
        .corrections
        .iter()
        .map(|c| (c.id.as_deref().unwrap(), c.undoable))
        .collect();
    // Newest first; only the last change of each value can go.
    assert_eq!(
        undoable,
        [
            ("c000005", true),
            ("c000004", true),
            ("c000003", true),
            ("c000002", false),
            ("c000001", false)
        ]
    );
    assert!(l.corrections[3]
        .why_not
        .as_deref()
        .unwrap()
        .contains("c000003"));

    // Corrections of other values, fields and items do not stand in the
    // way: the first theme chain unwinds under the entity and the other
    // theme, and under x:1's change of the same theme name.
    let u = e.undo(&c3.id).unwrap();
    assert_eq!(fx.agreed(4, 6, &u).themes, [VIBE]);
    let u = e.undo(&c2.id).unwrap();
    assert_eq!(fx.agreed(4, 7, &u).themes, [MATHS, VIBE]);
    let u = e.undo(&c1.id).unwrap();
    let t = fx.agreed(4, 8, &u);
    assert_eq!((t.themes, t.entities), (strs(&[VIBE]), strs(&["Astra"])));
    assert!(fx.reader(1).themes.is_empty());
    e.undo(&c6.id).unwrap();
    e.undo(&c4.id).unwrap();
    let u = e.undo(&c5.id).unwrap();
    let t = fx.agreed(4, 11, &u);
    assert!(t.themes.is_empty() && t.entities.is_empty() && t.corrected.is_empty());

    // Events: the same event under another id is the same value.
    let a = e.add_to_event("x:4", GEMINI).unwrap();
    let r = e.remove_from_event("x:4", "2026-09/E02").unwrap();
    fx.refused("c000013: c000014 (event_remove", || e.undo(&a.id));
    e.undo(&r.id).unwrap();
    assert_eq!(fx.reader(4).events, Some(strs(&[GEMINI])));

    // A later hand-written line with the whole list touches every theme.
    let t = e.add_theme("x:3", MATHS).unwrap();
    let mut text = fs::read_to_string(fx.file()).unwrap();
    text.push_str("{\"key\":\"x:3\",\"themes\":[\"By hand\"],\"by\":\"Dominik\"}\n");
    text.push_str("{\"key\":\"x:2\",\"themes\":[\"By hand\"]}\n");
    fs::write(fx.file(), text).unwrap();
    fx.refused(
        "c000016: line 17 (a hand-written line) later changed the same theme of x:3",
        || e.undo(&t.id),
    );
    // It does not touch the item's entities or events, nor another item.
    let ent = e.add_entity("x:2", "E", "other").unwrap();
    e.undo(&ent.id).unwrap();
    assert_eq!(fx.reader(3).themes, ["By hand"]);
}

#[test]
fn a_hand_written_file_in_the_old_shape_reads_as_before_and_is_appended_to() {
    let fx = fixture(true);
    let old = [
        // The old shape: whole lists, provenance keys, an unmatched key.
        r#"{"key":"x:1","themes":["Hand theme"],"entities":["A","B"],"by":"Dominik","date":"2026-10-11","note":"n"}"#,
        r#"{"key":"x:404","ai_related":false}"#,
        r#"{"key":"x:2","event_add":["2026-09-gemini-launch"],"event_remove":["2026-09/E21"]}"#,
        r#"{"key":"x:3","ai_related":null,"event":null}"#,
        // The new keys on a line without a numbered id are not read, as
        // before: unknown keys, whatever their type.
        r#"{"key":"x:4","theme_add":["AI and mathematics"],"entity_add":7,"undoes":"c000001","id":"mine"}"#,
        r#"{"key":"x:4","theme_remove":[1],"id":7}"#,
        "",
    ]
    .join("\n");
    write(fx.root(), topics::CORRECTIONS_FILE, &old);
    let build = fx.corpus.build_index(false).unwrap().topics.unwrap();
    assert_eq!(
        (
            build.corrections,
            build.corrections_unmatched,
            build.corrections_bad_lines,
            build.corrections_undone
        ),
        (5, 1, 0, 0)
    );
    let one = fx.reader(1);
    assert_eq!(
        (one.themes.clone(), one.entities.clone()),
        (strs(&["Hand theme"]), strs(&["A", "B"]))
    );
    assert_eq!(one.corrected, strs(&["themes", "entities"]));
    assert_eq!(fx.reader(2).events, Some(strs(&[GEMINI])));
    let three = fx.reader(3);
    assert_eq!((three.event, three.ai_related), (None, None));
    let four = fx.reader(4);
    assert!(four.themes.is_empty() && four.entities.is_empty() && four.corrected.is_empty());

    // Appended to: the old bytes stay, the patch applies to the list the
    // hand-written line left.
    let e = fx.editor();
    let c = e.add_theme("x:1", VIBE).unwrap();
    assert_eq!(c.id, "c000001");
    let bytes = fs::read(fx.file()).unwrap();
    assert!(bytes.starts_with(old.as_bytes()));
    assert_eq!(fx.agreed(1, 6, &c).themes, ["Hand theme", VIBE]);
    let c = e.remove_entity("x:1", "a").unwrap();
    assert_eq!(fx.agreed(1, 7, &c).entities, ["B"]);
    // The listing shows the hand-written line, which `undo` does not take.
    let l = e.list("x:1").unwrap();
    let ops: Vec<(&str, bool)> = l
        .corrections
        .iter()
        .map(|c| (c.op.as_str(), c.undoable))
        .collect();
    assert_eq!(
        ops,
        [
            ("entity_remove", true),
            ("theme_add", true),
            ("hand-written", false)
        ]
    );
    assert_eq!(l.corrections[2].fields, ["entities", "themes"]);
    assert_eq!((l.corrections[2].line, &l.corrections[2].id), (1, &None));
    assert_eq!(l.model.themes, [MATHS]);
    assert_eq!(l.effective, fx.reader(1));
}

#[test]
fn a_file_with_a_line_the_writer_cannot_read_is_not_appended_to() {
    let good = r#"{"id":"c000001","at":"t","date":"d","by":"dominik","key":"x:4","theme_add":["AI and mathematics"]}"#;
    for (text, why, reader_bad) in [
        ("{\"key\":\"x:1\",\n".to_string(), "line 1: not JSON", 1),
        ("[1]\n".to_string(), "line 1: not a JSON object", 1),
        (
            format!("{good}\n{{\"themes\":[\"A\"]}}\n"),
            "line 2: the reader does not read it",
            1,
        ),
        (
            format!("{good}\n{{\"key\":\"x:1\",\"themes\":\"A\"}}\n"),
            "line 2: the reader does not read it",
            1,
        ),
        (format!("{good}\n\n"), "line 2: not JSON", 0),
        (
            good.to_string(),
            "line 1: the last line is not ended by a newline",
            0,
        ),
        (format!("{good}\n\u{feff}\n"), "line 2: not JSON", 1),
        (
            format!(
                "{good}\n{}\n",
                good.replace("c000001", "c000002")
                    .replace("\"theme_add\"", "\"note\":\"x\",\"theme_add\"")
            ),
            "line 2: a numbered line with the key \"note\"",
            1,
        ),
        (
            format!("{good}\n{}\n", good.replace("\"x:4\"", "\"x:1\"")),
            "line 2: id \"c000001\" is not greater than the numbered line before",
            1,
        ),
        (
            format!(
                "{good}\n{}\n",
                r#"{"id":"c000002","at":"t","date":"d","by":"dominik","key":"x:1","undoes":"c000001"}"#
            ),
            "line 2: undoes \"c000001\", which no earlier correction of its key is",
            1,
        ),
        (
            format!(
                "{good}\n{}\n",
                r#"{"id":"c000002","at":"t","date":"d","by":"dominik","key":"x:4","theme_add":["A","B"]}"#
            ),
            "line 2: \"theme_add\" is not a list of one name",
            1,
        ),
        (
            format!(
                "{good}\n{}\n",
                r#"{"id":"c000002","date":"d","by":"dominik","key":"x:4","theme_add":["A"]}"#
            ),
            "line 2: a numbered line without a text \"at\"",
            1,
        ),
    ] {
        let fx = fixture(true);
        write(fx.root(), topics::CORRECTIONS_FILE, &text);
        let e = fx.editor();
        fx.refused(why, || e.add_theme("x:1", VIBE));
        fx.refused(why, || e.undo("c000001"));
        fx.refused(why, || e.list("x:1"));
        fx.refused("nothing was changed", || e.add_entity("x:1", "E", "person"));
        // The reader builds all the same, and counts the line it skips.
        let build = fx.corpus.build_index(false).unwrap().topics.unwrap();
        assert_eq!(build.corrections_bad_lines, reader_bad, "{why}");
    }
}

#[cfg(unix)]
#[test]
fn a_linked_corrections_file_is_not_written() {
    use std::os::unix::fs::PermissionsExt;
    let fx = fixture(true);
    let e = fx.editor();
    e.add_theme("x:4", MATHS).unwrap();
    // A new file has the default mode: what the umask leaves of 0666.
    let mode = fs::metadata(fx.file()).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode & 0o111, 0, "{mode:o}");
    assert_eq!(mode & 0o600, 0o600, "{mode:o}");

    // A second name outside the archive: an append would write there too.
    let outside = tempfile::tempdir().unwrap();
    let other = outside.path().join("other-name.jsonl");
    fs::hard_link(fx.file(), &other).unwrap();
    let before = fs::read(&other).unwrap();
    fx.refused("has 2 hard links", || e.add_theme("x:4", VIBE));
    fx.refused("has 2 hard links", || e.undo("c000001"));
    assert_eq!(fs::read(&other).unwrap(), before);
    // Reading it is harmless.
    assert_eq!(e.list("x:4").unwrap().effective.themes, [MATHS]);
    fs::remove_file(&other).unwrap();
    e.add_theme("x:4", VIBE).unwrap();

    // A symlink in its place, to a file outside: neither read nor written.
    let target = outside.path().join("target.jsonl");
    fs::rename(fx.file(), &target).unwrap();
    std::os::unix::fs::symlink(&target, fx.file()).unwrap();
    let before = fs::read(&target).unwrap();
    fx.refused("not a plain file", || e.add_entity("x:4", "E", "person"));
    fx.refused("not a plain file", || e.list("x:4"));
    assert_eq!(fs::read(&target).unwrap(), before);
    // A folder in its place.
    fs::remove_file(fx.file()).unwrap();
    fs::create_dir(fx.file()).unwrap();
    fx.refused("not a plain file", || e.add_entity("x:4", "E", "person"));
}

#[test]
fn a_line_over_the_readers_line_limit_and_a_last_id_are_refused() {
    let fx = fixture(true);
    let e = fx.editor();
    // A key whose tag row is exactly the reader's line limit: the row is
    // read, and a correction line for it would be longer than the limit.
    let key = format!("x:{}", "k".repeat(topics::MAX_LINE_BYTES - 12));
    let row = format!("{{\"key\":\"{key}\"}}");
    assert_eq!(row.len(), topics::MAX_LINE_BYTES);
    let mut rows = fs::read_to_string(fx.root().join(topics::LAYER_FILE)).unwrap();
    rows.push_str(&row);
    rows.push('\n');
    write(fx.root(), topics::LAYER_FILE, &rows);
    assert!(topics::read_layer(fx.root())
        .unwrap()
        .unwrap()
        .items
        .contains_key(&key));
    fx.refused("over the limit of 1048576 for one line", || {
        e.add_entity(&key, "E", "person")
    });
    assert!(!fx.file().exists());

    // The largest id: no further line.
    let last = r#"{"id":"c999999999","at":"t","date":"d","by":"dominik","key":"x:4","theme_add":["AI and mathematics"]}"#;
    write(fx.root(), topics::CORRECTIONS_FILE, &format!("{last}\n"));
    assert_eq!(fx.reader(4).themes, [MATHS]);
    fx.refused(
        "the last correction has the largest id there is (c999999999)",
        || e.add_theme("x:4", VIBE),
    );
    fx.refused("the largest id there is", || e.undo("c999999999"));
    // Ten digits is no numbered id: the line is a hand-written one, whose
    // `theme_add` is not read.
    write(
        fx.root(),
        topics::CORRECTIONS_FILE,
        &format!("{}\n", last.replace("c999999999", "c1000000000")),
    );
    assert!(fx.reader(4).themes.is_empty());
    assert_eq!(e.add_theme("x:4", VIBE).unwrap().id, "c000001");
}

#[test]
fn an_item_in_several_events_and_out_of_its_model_event() {
    let fx = fixture(true);
    let e = fx.editor();
    // x:1 is in the declaration by its tag row; now in the launch too.
    let c = e.add_to_event("x:1", GEMINI).unwrap();
    let t = fx.agreed(1, 0, &c);
    assert_eq!(t.events, Some(strs(&[FIELDS, GEMINI])));
    assert_eq!(t.event.as_deref(), Some("2026-09/E21"));
    assert_eq!(fx.hits(&format!("event:{GEMINI}")), ["x:1", "x:3"]);
    assert_eq!(fx.hits("event:2026-09/E21"), ["x:1", "x:2"]);
    // Out of the model's event: the other stays, `event` is null.
    let c = e.remove_from_event("x:1", "2026-09/E21").unwrap();
    assert_eq!(c.value, FIELDS);
    let t = fx.agreed(1, 1, &c);
    assert_eq!(
        (t.events, t.event, t.event_name),
        (Some(strs(&[GEMINI])), None, None)
    );
    assert_eq!(fx.hits(&format!("event:{FIELDS}")), ["x:2"]);
    // Back in, as a correction: the tag row's event is its event again.
    fx.refused("x:1: not in the event 2026-09-fields-declaration", || {
        e.remove_from_event("x:1", FIELDS)
    });
    let c = e.add_to_event("x:1", FIELDS).unwrap();
    let t = fx.agreed(1, 2, &c);
    assert_eq!(t.events, Some(strs(&[GEMINI, FIELDS])));
    assert_eq!(t.event.as_deref(), Some("2026-09/E21"));
    // Undone newest first: the model's one event, nothing marked.
    for id in ["c000003", "c000002", "c000001"] {
        e.undo(id).unwrap();
    }
    let t = fx.reader(1);
    assert_eq!((t.events, t.corrected), (Some(strs(&[FIELDS])), vec![]));
}

#[test]
fn without_a_records_folder_events_are_refused_and_themes_and_entities_work() {
    let fx = fixture(false);
    let e = fx.editor();
    for op in [
        CorrectionEditor::add_to_event,
        CorrectionEditor::remove_from_event,
    ] {
        fx.refused(
            "no records folder; without one the reader reads no event_add or event_remove",
            || op(&e, "x:1", "2026-09/E21"),
        );
    }
    assert!(!fx.file().exists());
    // The reader reads the theme and entity keys of a numbered line with
    // or without the folder.
    let c = e.add_theme("x:4", VIBE).unwrap();
    let t = fx.agreed(4, 0, &c);
    assert_eq!((t.themes, t.events), (strs(&[VIBE]), None));
    let c = e.add_entity("x:4", "Astra", "model").unwrap();
    assert_eq!(fx.agreed(4, 1, &c).entities, ["Astra"]);
    let c = e.remove_theme("x:1", MATHS).unwrap();
    assert!(fx.agreed(1, 2, &c).themes.is_empty());
    let u = e.undo("c000001").unwrap();
    assert!(fx.agreed(4, 3, &u).themes.is_empty());
    assert_eq!(fx.hits("entity:Astra"), ["x:4"]);
}

#[test]
fn the_listing_gives_the_model_tags_the_tags_in_effect_and_every_line() {
    let fx = fixture(true);
    let e = fx.editor();
    let empty = e.list("x:2").unwrap();
    assert_eq!(empty.tool, "scout correct list");
    assert_eq!(empty.model, empty.effective);
    assert!(empty.corrections.is_empty());
    assert_eq!(empty.model, fx.reader(2));
    fx.refused("x:404: no tag row", || e.list("x:404"));

    e.add_theme("x:2", "Metaphor and cognition").unwrap();
    e.add_entity("x:2", "Astra", "model").unwrap();
    e.add_theme("x:1", VIBE).unwrap();
    e.undo("c000001").unwrap();
    let before = tree(fx.root());
    let l = e.list("x:2").unwrap();
    assert!(tree(fx.root()) == before, "list wrote");
    assert_eq!(l.model.themes, [MATHS, VIBE]);
    assert!(l.model.corrected.is_empty() && l.model.entities.is_empty());
    assert_eq!(l.effective, fx.reader(2));
    assert_eq!(l.effective.entities, ["Astra"]);
    let v = serde_json::to_value(&l.corrections).unwrap();
    assert_eq!(
        v,
        json!([
            {"id": "c000004", "line": 4, "at": NOW, "date": "2026-10-12", "by": "dominik",
             "op": "undo", "value": null, "undoes": "c000001", "undone_by": null,
             "undoable": false,
             "why_not": "c000004 is the undo of c000001; an undo is not undone, make the correction again (nothing was changed)"},
            {"id": "c000002", "line": 2, "at": NOW, "date": "2026-10-12", "by": "dominik",
             "op": "entity_add", "value": "Astra", "entity_type": "model", "undoes": null,
             "undone_by": null, "undoable": true, "why_not": null},
            {"id": "c000001", "line": 1, "at": NOW, "date": "2026-10-12", "by": "dominik",
             "op": "theme_add", "value": "Metaphor and cognition", "undoes": null,
             "undone_by": "c000004", "undoable": false,
             "why_not": "c000001: already undone, by c000004 (nothing was changed)"},
        ])
    );
}

/// For every line `list` shows of `key`: `undoable` and `why_not` are
/// what `undo` then does. A line said not undoable is refused in the
/// words of its `why_not`, with nothing written; one said undoable is
/// undone, on a copy of the archive, so this archive stays as it is.
/// Gives `(id, undoable)`, newest first.
fn listing_is_what_undo_does(fx: &Fx, key: &str) -> Vec<(String, bool)> {
    let e = fx.editor();
    let listed = e.list(key).unwrap();
    let mut out = Vec::new();
    for c in &listed.corrections {
        assert_eq!(c.undoable, c.why_not.is_none(), "{c:?}");
        let Some(id) = &c.id else {
            assert!(!c.undoable, "a line without an id cannot be named: {c:?}");
            continue;
        };
        out.push((id.clone(), c.undoable));
        if let Some(why) = &c.why_not {
            let before = tree(fx.root());
            let refusal = format!("{:#}", e.undo(id).expect_err(why));
            assert_eq!(&refusal, why, "{id}: list and undo disagree");
            assert!(tree(fx.root()) == before, "{id}: a refused undo wrote");
            continue;
        }
        // On a copy: the same files under another root.
        let copy = tempfile::tempdir().unwrap();
        for (path, bytes) in tree(fx.root()) {
            let to = copy.path().join(path.strip_prefix(fx.root()).unwrap());
            fs::create_dir_all(to.parent().unwrap()).unwrap();
            fs::write(to, bytes).unwrap();
        }
        let there = CorrectionEditor::open(copy.path()).unwrap().with_now(NOW);
        let done = there
            .undo(id)
            .unwrap_or_else(|e| panic!("{id}: listed undoable, and undo says {e:#}"));
        assert_eq!(done.undoes.as_deref(), Some(id.as_str()));
    }
    out
}

fn flags(l: &[(String, bool)]) -> Vec<(&str, bool)> {
    l.iter().map(|(id, u)| (id.as_str(), *u)).collect()
}

#[cfg(unix)]
#[test]
fn undoable_in_the_listing_is_the_answer_undo_gives() {
    use std::os::unix::fs::PermissionsExt;
    let fx = fixture(true);
    let e = fx.editor();
    // The states the listing already told apart: a later change of the
    // same value, an undo line, a line undone, a hand-written line.
    e.add_theme("x:4", MATHS).unwrap();
    e.remove_theme("x:4", MATHS).unwrap();
    e.add_entity("x:4", "Astra", "model").unwrap();
    e.add_to_event("x:4", GEMINI).unwrap();
    e.undo("c000003").unwrap();
    let mut text = fs::read_to_string(fx.file()).unwrap();
    text.push_str("{\"key\":\"x:4\",\"ai_related\":false}\n");
    fs::write(fx.file(), &text).unwrap();
    assert_eq!(
        flags(&listing_is_what_undo_does(&fx, "x:4")),
        [
            ("c000005", false),
            ("c000004", true),
            ("c000003", false),
            ("c000002", true),
            ("c000001", false)
        ]
    );

    // A second name for the file: every undo is refused at its append,
    // so no line is undoable, and the listing says why in undo's words.
    let outside = tempfile::tempdir().unwrap();
    let other = outside.path().join("other-name.jsonl");
    fs::hard_link(fx.file(), &other).unwrap();
    let linked = listing_is_what_undo_does(&fx, "x:4");
    assert!(linked.iter().all(|(_, undoable)| !undoable), "{linked:?}");
    let l = e.list("x:4").unwrap();
    let why = |id: &str| {
        l.corrections
            .iter()
            .find(|c| c.id.as_deref() == Some(id))
            .and_then(|c| c.why_not.clone())
            .unwrap()
    };
    assert!(
        why("c000004").contains("has 2 hard links"),
        "{}",
        why("c000004")
    );
    // The first refusal of the undo is the one given: an undone line is
    // refused for that before the file is looked at.
    assert!(
        why("c000003").contains("already undone"),
        "{}",
        why("c000003")
    );
    fs::remove_file(&other).unwrap();

    // A file that may not be written.
    fs::set_permissions(fx.file(), fs::Permissions::from_mode(0o444)).unwrap();
    if fs::OpenOptions::new().append(true).open(fx.file()).is_err() {
        let locked = listing_is_what_undo_does(&fx, "x:4");
        assert!(locked.iter().all(|(_, undoable)| !undoable), "{locked:?}");
    }
    fs::set_permissions(fx.file(), fs::Permissions::from_mode(0o644)).unwrap();

    // No id left for the undo line: the correction that holds the last
    // id is in effect, and cannot be undone.
    let last = r#"{"id":"c999999999","at":"t","date":"d","by":"dominik","key":"x:2","entity_add":["Astra"],"entity_type":"model"}"#;
    fs::write(fx.file(), format!("{text}{last}\n")).unwrap();
    assert_eq!(fx.reader(2).entities, ["Astra"]);
    assert_eq!(
        flags(&listing_is_what_undo_does(&fx, "x:2")),
        [("c999999999", false)]
    );
    let l = e.list("x:2").unwrap();
    let why = l.corrections[0].why_not.as_deref().unwrap();
    assert!(
        why.contains("the last correction has the largest id there is (c999999999)"),
        "{why}"
    );
    // And every other line of the file is in the same case.
    let stuck = listing_is_what_undo_does(&fx, "x:4");
    assert!(stuck.iter().all(|(_, undoable)| !undoable), "{stuck:?}");
}

#[test]
fn undo_is_refused_for_an_item_that_has_no_tag_row_any_more() {
    let fx = fixture(true);
    let e = fx.editor();
    e.add_theme("x:4", MATHS).unwrap();
    e.add_theme("x:1", VIBE).unwrap();
    listing_is_what_undo_does(&fx, "x:4");
    // The model's rows are written again without x:4.
    let rows = fs::read_to_string(fx.root().join(topics::LAYER_FILE)).unwrap();
    let kept: String = rows
        .lines()
        .filter(|l| !l.contains("\"key\":\"x:4\""))
        .map(|l| format!("{l}\n"))
        .collect();
    assert_ne!(kept, rows);
    write(fx.root(), topics::LAYER_FILE, &kept);
    // Undo, like every operation on that key, is refused before a write.
    let why = "x:4: no tag row in analysis/topics/current.jsonl";
    fx.refused(why, || e.undo("c000001"));
    fx.refused(why, || e.add_theme("x:4", VIBE));
    fx.refused(why, || e.list("x:4"));
    assert_eq!(fx.lines().len(), 2);
    // The line stays, and the reader does with it what it does with any
    // correction for a key without a tag row: counts it, applies it to
    // nothing. The other item's correction is read as before.
    let layer = topics::read_layer(fx.root()).unwrap().unwrap();
    assert!(!layer.items.contains_key("x:4"));
    assert_eq!(
        (
            layer.corrections,
            layer.corrections_unmatched,
            layer.corrections_bad_lines
        ),
        (1, 1, 0)
    );
    assert_eq!(fx.reader(1).themes, [MATHS, VIBE]);
    // The other item's corrections are undone as ever.
    assert_eq!(
        flags(&listing_is_what_undo_does(&fx, "x:1")),
        [("c000002", true)]
    );
    e.undo("c000002").unwrap();
    assert_eq!(fx.reader(1).themes, [MATHS]);
    // When the row is back, so is the undo.
    write(fx.root(), topics::LAYER_FILE, &rows);
    assert_eq!(fx.reader(4).themes, [MATHS]);
    e.undo("c000001").unwrap();
    assert!(fx.reader(4).themes.is_empty());
}

/// A numbered line with the usual provenance for `key`, and `rest` as its
/// other keys.
fn numbered(id: &str, key: &str, rest: &str) -> String {
    format!(r#"{{"id":"{id}","at":"t","date":"d","by":"dominik","key":"{key}",{rest}}}"#)
}

/// The review's reproduction: `c000002`, then `c000001` with a `note`.
/// The reader applies the first and counts the second; the writer refuses
/// the file and names that same line.
#[test]
fn an_out_of_order_numbered_line_with_a_note_is_skipped_by_the_reader_and_named_by_the_writer() {
    let fx = fixture(true);
    let text = format!(
        "{}\n{}\n",
        numbered("c000002", "x:4", r#""theme_add":["AI and mathematics"]"#),
        numbered(
            "c000001",
            "x:4",
            r#""note":"by hand","entity_add":["Terence Tao"],"entity_type":"person""#
        ),
    );
    write(fx.root(), topics::CORRECTIONS_FILE, &text);
    let build = fx.corpus.build_index(false).unwrap().topics.unwrap();
    assert_eq!(
        (
            build.corrections,
            build.corrections_bad_lines,
            build.corrections_unmatched,
            build.corrections_undone
        ),
        (1, 1, 0, 0)
    );
    let four = fx.reader(4);
    assert_eq!(four.themes, [MATHS]);
    assert!(four.entities.is_empty(), "{four:?}");
    assert_eq!(four.corrected, ["themes"]);
    let e = fx.editor();
    let why = "line 2: a numbered line with the key \"note\"";
    fx.refused(why, || e.add_theme("x:1", VIBE));
    fx.refused(why, || e.undo("c000002"));
    fx.refused(why, || e.list("x:4"));
    // Out of order alone is enough.
    let text = format!(
        "{}\n{}\n",
        numbered("c000002", "x:4", r#""theme_add":["AI and mathematics"]"#),
        numbered(
            "c000001",
            "x:4",
            r#""entity_add":["Terence Tao"],"entity_type":"person""#
        ),
    );
    write(fx.root(), topics::CORRECTIONS_FILE, &text);
    let build = fx.corpus.build_index(false).unwrap().topics.unwrap();
    assert_eq!((build.corrections, build.corrections_bad_lines), (1, 1));
    assert!(fx.reader(4).entities.is_empty());
    fx.refused(
        "line 2: id \"c000001\" is not greater than the numbered line before",
        || e.add_theme("x:1", VIBE),
    );
}

/// One table, both paths. Line 1 is a valid correction (`c000002`: x:4
/// gets a theme). Line 2 is a numbered line of one rejection kind, which
/// would change x:4 if it were applied. Line 3, where given, is a
/// numbered undo of line 2.
///
/// Reader: line 1 applied, every other line a bad line, x:4 as line 1
/// leaves it. Writer: every command refuses, naming line 2 and the
/// reason. The reason is the reader's too: it comes from the one function
/// both ask.
#[test]
fn every_kind_of_rejected_numbered_line_is_judged_alike_by_the_reader_and_the_writer() {
    let first = numbered("c000002", "x:4", r#""theme_add":["AI and mathematics"]"#);
    let person = r#""entity_add":["E"],"entity_type":"person""#;
    let cases: Vec<(&str, String, Option<String>, &str)> = vec![
        (
            "a key `scout correct` does not write",
            numbered("c000003", "x:4", &format!(r#""note":"n",{person}"#)),
            None,
            "a numbered line with the key \"note\", which `scout correct` does not write",
        ),
        (
            "a whole-list key on a numbered line",
            numbered("c000003", "x:4", r#""themes":["Replaced"]"#),
            None,
            "a numbered line with the key \"themes\", which `scout correct` does not write",
        ),
        (
            "more than one change",
            numbered(
                "c000003",
                "x:4",
                r#""theme_add":["B"],"theme_remove":["AI and mathematics"]"#,
            ),
            None,
            "a numbered line with more than one change",
        ),
        (
            "a change and an undo",
            numbered("c000003", "x:4", r#""theme_add":["B"],"undoes":"c000002""#),
            None,
            "a numbered line with more than one change",
        ),
        (
            "no change",
            numbered("c000003", "x:4", r#""entity_type":"person""#),
            None,
            "a numbered line with no change and no `undoes`",
        ),
        (
            "an id below an earlier numbered id",
            numbered("c000001", "x:4", person),
            None,
            "id \"c000001\" is not greater than the numbered line before",
        ),
        (
            "an id equal to an earlier numbered id",
            numbered("c000002", "x:4", person),
            None,
            "id \"c000002\" is not greater than the numbered line before",
        ),
        (
            "a list of two names",
            numbered("c000003", "x:4", r#""theme_add":["B","C"]"#),
            None,
            "\"theme_add\" is not a list of one name",
        ),
        (
            "a name that is not in a list",
            numbered("c000003", "x:4", r#""theme_remove":"AI and mathematics""#),
            None,
            "\"theme_remove\" is not a list of one name",
        ),
        (
            "a blank name",
            numbered(
                "c000003",
                "x:4",
                r#""entity_add":[" "],"entity_type":"person""#,
            ),
            None,
            "\"entity_add\" is not a list of one name",
        ),
        (
            "an entity type that is none",
            numbered(
                "c000003",
                "x:4",
                r#""entity_add":["E"],"entity_type":"thing""#,
            ),
            None,
            "`entity_type` is not an entity type",
        ),
        (
            "an entity type that is a number",
            numbered("c000003", "x:4", r#""entity_add":["E"],"entity_type":7"#),
            None,
            "`entity_type` is not an entity type",
        ),
        (
            "an entity type that is null",
            numbered("c000003", "x:4", r#""entity_add":["E"],"entity_type":null"#),
            None,
            "`entity_type` is not an entity type",
        ),
        (
            "an entity type in a list",
            numbered(
                "c000003",
                "x:4",
                r#""entity_add":["E"],"entity_type":["person"]"#,
            ),
            None,
            "`entity_type` is not an entity type",
        ),
        (
            "a wrongly typed entity type on another change",
            numbered("c000003", "x:4", r#""theme_add":["B"],"entity_type":7"#),
            None,
            "`entity_type` is not an entity type",
        ),
        (
            "an entity without its type",
            numbered("c000003", "x:4", r#""entity_add":["E"]"#),
            None,
            "`entity_type` goes with `entity_add`, and only with it",
        ),
        (
            "no `at`",
            numbered("c000003", "x:4", person).replace(r#""at":"t","#, ""),
            None,
            "a numbered line without a text \"at\"",
        ),
        (
            "a `by` that is not text",
            numbered("c000003", "x:4", person).replace(r#""by":"dominik""#, r#""by":7"#),
            None,
            "a numbered line without a text \"by\"",
        ),
        (
            "an `undoes` that is not a correction id",
            numbered("c000003", "x:4", r#""undoes":"m000002""#),
            None,
            "`undoes` is not a correction id",
        ),
        (
            "an `undoes` spelt with a zero too many",
            numbered("c000003", "x:4", r#""undoes":"c0000002""#),
            None,
            "`undoes` is not a correction id",
        ),
        (
            "an `undoes` spelt without its zeros",
            numbered("c000003", "x:4", r#""undoes":"c2""#),
            None,
            "`undoes` is not a correction id",
        ),
        (
            "an `undoes` spelt with a capital",
            numbered("c000003", "x:4", r#""undoes":"C000002""#),
            None,
            "`undoes` is not a correction id",
        ),
        (
            "an undo of no line",
            numbered("c000003", "x:4", r#""undoes":"c000001""#),
            None,
            "undoes \"c000001\", which no earlier correction of its key is",
        ),
        (
            "an undo of another item's correction",
            numbered("c000003", "x:1", r#""undoes":"c000002""#),
            None,
            "undoes \"c000002\", which no earlier correction of its key is",
        ),
        (
            "a rejected line is not undone either",
            numbered("c000003", "x:4", &format!(r#""note":"n",{person}"#)),
            Some(numbered("c000004", "x:4", r#""undoes":"c000003""#)),
            "a numbered line with the key \"note\", which `scout correct` does not write",
        ),
    ];
    for (kind, second, third, why) in cases {
        let fx = fixture(true);
        let mut text = format!("{first}\n{second}\n");
        let mut bad = 1;
        if let Some(third) = &third {
            text.push_str(&format!("{third}\n"));
            bad += 1;
        }
        write(fx.root(), topics::CORRECTIONS_FILE, &text);
        // The reader.
        let build = fx.corpus.build_index(false).unwrap().topics.unwrap();
        assert_eq!(
            (
                build.corrections,
                build.corrections_bad_lines,
                build.corrections_unmatched,
                build.corrections_undone
            ),
            (1, bad, 0, 0),
            "{kind}"
        );
        let four = fx.reader(4);
        assert_eq!(four.themes, [MATHS], "{kind}");
        assert!(four.entities.is_empty(), "{kind}: {four:?}");
        assert_eq!(four.corrected, ["themes"], "{kind}");
        assert_eq!(fx.reader(1).corrected, Vec::<String>::new(), "{kind}");
        // The writer: the same line, the same reason.
        let e = fx.editor();
        let named = format!("line 2: {why}");
        fx.refused(&named, || e.add_theme("x:1", VIBE));
        fx.refused(&named, || e.undo("c000002"));
        fx.refused(&named, || e.list("x:4"));
    }

    // A second undo of one correction: line 3 is the rejected line.
    let fx = fixture(true);
    let undo = |id: &str| numbered(id, "x:4", r#""undoes":"c000002""#);
    write(
        fx.root(),
        topics::CORRECTIONS_FILE,
        &format!("{first}\n{}\n{}\n", undo("c000003"), undo("c000004")),
    );
    let build = fx.corpus.build_index(false).unwrap().topics.unwrap();
    assert_eq!(
        (
            build.corrections,
            build.corrections_bad_lines,
            build.corrections_undone
        ),
        (0, 1, 1)
    );
    assert!(fx.reader(4).themes.is_empty());
    let e = fx.editor();
    fx.refused("line 3: undoes \"c000002\" a second time", || {
        e.add_theme("x:4", MATHS)
    });

    // An `id` of another form makes no numbered line: both read it as a
    // hand-written line, on which the patch keys are not read.
    let fx = fixture(true);
    for (n, id) in [
        "c1",
        "c0000000001",
        "C000003",
        "x000003",
        // A number has one spelling: these are no ids either.
        "c0000003",
        "c000000003",
        "c0999999",
        "c000000",
    ]
    .iter()
    .enumerate()
    {
        let hand = numbered(id, "x:4", r#""theme_add":["B"]"#);
        write(
            fx.root(),
            topics::CORRECTIONS_FILE,
            &format!("{first}\n{hand}\n"),
        );
        let build = fx.corpus.build_index(false).unwrap().topics.unwrap();
        assert_eq!(
            (build.corrections, build.corrections_bad_lines),
            (2, 0),
            "{id}"
        );
        assert_eq!(fx.reader(4).themes, [MATHS], "{id}");
        let e = fx.editor();
        let l = e.list("x:4").unwrap();
        assert_eq!(l.corrections[0].op, "hand-written", "{id}");
        assert_eq!(l.corrections[0].id, None, "{id}");
        if n == 0 {
            let c = e.add_entity("x:4", "E", "person").unwrap();
            assert_eq!(c.id, "c000003");
            fx.agreed(4, 2, &c);
        }
    }
}

/// The review's reproduction: `c0000001`, then `c0000002` undoing it.
/// Neither id is the one spelling of its number, so neither line is a
/// numbered line, for the reader and for the writer alike: two
/// hand-written lines, nothing applied, nothing undone, and the listing
/// says the same. The same two lines under their canonical ids are a
/// correction and its undo, for both; so are ids past six digits.
#[test]
fn a_numbered_id_has_one_spelling_for_the_reader_the_writer_and_the_listing() {
    // (first id, second id, the id the second line's `undoes` names,
    // are they numbered lines)
    let cases = [
        ("c0000001", "c0000002", "c0000001", false),
        ("c00000001", "c00000002", "c00000001", false),
        ("c1", "c2", "c1", false),
        ("C000001", "C000002", "C000001", false),
        ("c000001", "c000002", "c000001", true),
        ("c999999", "c1000000", "c999999", true),
        ("c1000000", "c1000001", "c1000000", true),
        ("c999999998", "c999999999", "c999999998", true),
    ];
    for (first, second, undoes, numbered_lines) in cases {
        let fx = fixture(true);
        let text = format!(
            "{}\n{}\n",
            numbered(first, "x:4", r#""theme_add":["AI and mathematics"]"#),
            numbered(second, "x:4", &format!(r#""undoes":"{undoes}""#)),
        );
        write(fx.root(), topics::CORRECTIONS_FILE, &text);
        // The reader.
        let build = fx.corpus.build_index(false).unwrap().topics.unwrap();
        let read = (
            build.corrections,
            build.corrections_bad_lines,
            build.corrections_unmatched,
            build.corrections_undone,
        );
        let four = fx.reader(4);
        assert!(four.themes.is_empty(), "{first}: {four:?}");
        assert!(four.corrected.is_empty(), "{first}: {four:?}");
        // The writer's listing: newest first.
        let l = fx.editor().list("x:4").unwrap();
        assert_eq!(l.effective, four, "{first}");
        let rows: Vec<_> = l
            .corrections
            .iter()
            .map(|c| {
                (
                    c.op.as_str(),
                    c.id.as_deref(),
                    c.undoes.as_deref(),
                    c.undone_by.as_deref(),
                    c.undoable,
                )
            })
            .collect();
        if numbered_lines {
            // A correction and its undo: the reader took the correction
            // back, and the listing names the undo line on it.
            assert_eq!(read, (0, 0, 0, 1), "{first}");
            assert_eq!(
                rows,
                [
                    ("undo", Some(second), Some(first), None, false),
                    ("theme_add", Some(first), None, Some(second), false),
                ],
                "{first}"
            );
        } else {
            // Two hand-written lines: the reader undid nothing, and the
            // listing shows no undo and nothing undone.
            assert_eq!(read, (2, 0, 0, 0), "{first}");
            assert_eq!(
                rows,
                [
                    ("hand-written", None, None, None, false),
                    ("hand-written", None, None, None, false),
                ],
                "{first}"
            );
            // And `undo` does not take the spelling.
            let e = fx.editor();
            fx.refused("no such correction", || e.undo(first));
            // The next line the writer adds is the first numbered one.
            let c = e.add_theme("x:4", MATHS).unwrap();
            assert_eq!(c.id, "c000001", "{first}");
            fx.agreed(4, 2, &c);
        }
    }
    // A canonical id is not found under another spelling of its number.
    let fx = fixture(true);
    let e = fx.editor();
    assert_eq!(e.add_theme("x:4", MATHS).unwrap().id, "c000001");
    for other in ["c0000001", "c1", "C000001"] {
        fx.refused("no such correction", || e.undo(other));
    }
    e.undo("c000001").unwrap();
}

/// A corrections file over the limit the writer keeps it under is not
/// read at all: the build completes, applies no correction, and its
/// report says so. Back under the limit, the corrections apply again.
#[test]
fn a_corrections_file_over_the_size_limit_is_not_read_and_the_build_says_so() {
    let fx = fixture(true);
    let e = fx.editor();
    e.add_theme("x:4", MATHS).unwrap();
    let ok = fx.corpus.build_index(false).unwrap().topics.unwrap();
    assert_eq!(
        (ok.corrections, &ok.corrections_skipped, ok.warnings),
        (1, &None, 0)
    );
    let json = serde_json::to_value(&ok).unwrap();
    assert!(json.get("corrections_skipped").is_none() && json.get("warnings").is_none());
    assert_eq!(fx.reader(4).themes, [MATHS]);

    // One byte over (a sparse tail: no megabytes are written).
    let len = fs::metadata(fx.file()).unwrap().len();
    let over = MAX_FILE_BYTES + 1;
    let f = fs::OpenOptions::new().write(true).open(fx.file()).unwrap();
    f.set_len(over).unwrap();
    let report = fx.corpus.build_index(false).unwrap();
    let t = report.topics.unwrap();
    assert!(t.reloaded);
    assert_eq!(
        (
            t.corrections,
            t.corrections_unmatched,
            t.corrections_bad_lines,
            t.corrections_undone
        ),
        (0, 0, 0, 0)
    );
    assert_eq!(t.warnings, 1);
    let skipped = t.corrections_skipped.clone().expect("skipped");
    assert_eq!((skipped.bytes, skipped.limit), (over, MAX_FILE_BYTES));
    assert_eq!(
        skipped.reason,
        format!(
            "the corrections file is {over} bytes, larger than the limit of {MAX_FILE_BYTES} bytes; it was not read and NO correction was applied"
        )
    );
    let json = serde_json::to_value(&t).unwrap();
    assert_eq!(json["corrections_skipped"]["bytes"], over);
    assert_eq!(json["warnings"], 1);
    let four = fx.reader(4);
    assert!(
        four.themes.is_empty() && four.corrected.is_empty(),
        "{four:?}"
    );
    // An unchanged rebuild keeps saying so.
    let again = fx.corpus.build_index(false).unwrap().topics.unwrap();
    assert!(!again.reloaded);
    assert_eq!(
        (again.warnings, again.corrections_skipped),
        (1, Some(skipped))
    );
    // The writer does not read it either.
    let err = e.add_theme("x:1", VIBE).unwrap_err().to_string();
    assert!(err.contains("larger than"), "{err}");

    // Back at its length: read again.
    f.set_len(len).unwrap();
    let t = fx.corpus.build_index(false).unwrap().topics.unwrap();
    assert!(t.reloaded);
    assert_eq!(
        (t.corrections, &t.corrections_skipped, t.warnings),
        (1, &None, 0)
    );
    assert_eq!(fx.reader(4).themes, [MATHS]);
}

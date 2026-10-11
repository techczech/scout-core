//! Shared by the `scout events seed` tests: a fixture archive, and the
//! out folder read back with the reader itself ([`events::read_dir`]),
//! never with a second parser.
#![allow(dead_code, unused_imports)]

pub use scout_corpus::event_seed::{seed, SeedOptions, SeedReport, SLUG_MAX};
pub use scout_corpus::events::{self, EventRecord, EventSet};
pub use serde_json::{json, Value};
use std::collections::BTreeMap;
pub use std::fs;
pub use std::path::{Path, PathBuf};

pub const AWKWARD: &str = "He said: \"it's #1\" — naïve café, 100% [sic] & more";

pub fn put(root: &Path, rel: &str, text: &str) -> PathBuf {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(&p, text).unwrap();
    p
}

/// One briefing event: id, name, description, aliases.
pub type Ev<'a> = (&'a str, &'a str, Option<&'a str>, &'a [&'a str]);

pub fn briefing(month: &str, list: &[Ev]) -> String {
    let events: Vec<Value> = list
        .iter()
        .map(|(id, name, description, aliases)| {
            json!({
                "id": id, "name": name, "date": format!("{}-01-01", &month[..4]),
                "aliases": aliases,
                "description": description.map(String::from).unwrap_or(format!("About {name}.")),
                "items": [],
            })
        })
        .collect();
    json!({"month": month, "model": "test", "events": events}).to_string()
}

pub fn put_briefing(root: &Path, month: &str, list: &[Ev]) {
    put(
        root,
        &format!("analysis/context/briefings/{month}-events.json"),
        &briefing(month, list),
    );
}

pub fn row(key: &str, event: Option<&str>, date: Option<&str>, themes: &[&str]) -> Value {
    json!({
        "key": key, "path": format!("readings/works/{key}.md"), "date": date,
        "month": date.map(|d| &d[..7]), "themes": themes, "entities": [],
        "event": event, "event_name": null,
    })
}

pub const LAYER: &str = "analysis/topics/current.jsonl";

pub fn put_rows(root: &Path, rows: &[Value]) {
    let mut text: String = rows.iter().map(|r| format!("{r}\n")).collect();
    text.push_str("not json\n");
    put(root, LAYER, &text);
}

/// Every file under `root` with its bytes (links are not followed).
pub fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = fs::read_dir(&d) else { continue };
        for e in rd {
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

pub fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    v.sort();
    v
}

/// The fixture archive: ten monthly events, eight of them in briefing
/// files, two only in tag rows; superseded files that must not be read.
pub fn archive() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let long = format!("Long {}", "word ".repeat(30));
    put_briefing(
        root,
        "2026-09",
        &[
            (
                "E01",
                AWKWARD,
                Some("Line one: with a colon.\n\n--- not a fence\nLine \"two\"."),
                &["O'Reilly: the 'book'", "Ünïcode — alias"],
            ),
            ("E02", "Gemini launch", None, &[]),
            ("E03", "Gemini Launch!", None, &[]),
            ("E04", "Nobody tagged this", None, &[]),
            ("E10", "日本語", None, &[]),
            ("E11", &long, None, &[]),
        ],
    );
    put_briefing(root, "2026-10", &[("E01", "Gemini launch", None, &[])]);
    put_briefing(root, "2022-Q2", &[("E01", "Quarter story", None, &[])]);
    put(
        root,
        "analysis/context/briefings/2026-09-events.superseded-20261011-1603.json",
        &briefing(
            "2026-09",
            &[
                ("E01", "OLD NAME", None, &[]),
                ("E99", "Old only", None, &[]),
            ],
        ),
    );
    put(
        root,
        "analysis/context/briefings/2026-08.superseded-20261011-1603/2026-08-events.json",
        &briefing("2026-08", &[("E01", "In a superseded folder", None, &[])]),
    );
    let mut named = row("x:11", Some("2026-09/E99"), Some("2026-09-09"), &[]);
    named["event_name"] = json!("Only in the tag rows");
    put_rows(
        root,
        &[
            row("x:1", Some("2026-09/E01"), Some("2026-09-03"), &["A", "B"]),
            row("x:2", Some("2026-09/E01"), Some("2026-09-20"), &["A"]),
            row("x:3", Some("2026-09/E01"), Some("2026-10-02"), &["A", "C"]),
            row("x:4", Some("2026-09/E02"), Some("2026-09-05"), &["A"]),
            row("x:5", Some("2026-09/E03"), Some("2026-09-06"), &[]),
            row("x:6", Some("2026-09/E10"), Some("2026-09-07"), &[]),
            row("x:7", Some("2026-09/E11"), Some("2026-09-08"), &[]),
            row("x:8", Some("2026-10/E01"), Some("2026-10-09"), &[]),
            row("x:9", Some("2022-Q2/E01"), None, &[]),
            row("x:10", Some("2026-08/E01"), Some("2026-08-01"), &[]),
            named,
            row("x:12", None, Some("2026-09-10"), &[]),
        ],
    );
    tmp
}

pub fn opts(root: &Path, out: Option<&Path>, apply: bool) -> SeedOptions {
    SeedOptions {
        archive: root.to_path_buf(),
        out: out.map(Path::to_path_buf),
        apply,
    }
}

/// The out folder as the reader has it. Every `.md` file there must be a
/// record it reads: none skipped, as many records as files.
pub fn read_all(dir: &Path) -> EventSet {
    let set = events::read_dir(dir).unwrap().expect("the out folder");
    assert!(set.refused.is_empty(), "{:?}", set.refused);
    assert_eq!(set.skipped, 0);
    let md = names(dir).iter().filter(|n| n.ends_with(".md")).count();
    assert_eq!(set.records.len(), md);
    set
}

/// Seed with `apply` into the archive's own folder and read it back.
pub fn applied(root: &Path) -> (SeedReport, EventSet) {
    let r = seed(&opts(root, None, true)).unwrap();
    let set = read_all(&root.join(events::EVENTS_DIR));
    (r, set)
}

pub fn rec<'a>(set: &'a EventSet, id: &str) -> &'a EventRecord {
    set.records.get(id).unwrap_or_else(|| panic!("no {id}"))
}

pub fn strs(l: &[&str]) -> Vec<String> {
    l.iter().map(|s| s.to_string()).collect()
}

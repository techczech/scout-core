//! Shared by the `scout events` edit tests: a records folder written by
//! the seed's writer, read back with the reader itself, and compared
//! byte for byte.
#![allow(dead_code, unused_imports)]

pub use scout_corpus::event_edit::{
    By, Decision, EventEditor, FieldEdit, MergeRequest, NewEvent, Step, DECISIONS_LOG, MERGE_LOG,
    PROPOSALS_DIR,
};
pub use scout_corpus::event_seed::render_record;
pub use scout_corpus::events::{self, EventRecord, EventSet};
pub use serde_json::{json, Value};
use std::collections::BTreeMap;
pub use std::fs;
pub use std::path::{Path, PathBuf};

pub const ALPHA: &str = "2026-09-alpha";
pub const BETA: &str = "2026-10-beta";
pub const GAMMA: &str = "2026-11-gamma";
pub const DELTA: &str = "2026-08-delta";
/// The time every test editor takes as now.
pub const NOW: &str = "2026-10-12T08:00:00.000Z";
pub const TODAY: &str = "2026-10-12";
pub const GAMMA_BODY: &str =
    "His own words: \"kept\".\n\n--- not a fence\n\ttabbed, and a last line";

pub fn strs(l: &[&str]) -> Vec<String> {
    l.iter().map(|s| s.to_string()).collect()
}

pub fn rec(id: &str, name: &str, span: (&str, &str), members: &[&str]) -> EventRecord {
    EventRecord {
        id: id.into(),
        name: name.into(),
        start: Some(span.0.into()),
        end: Some(span.1.into()),
        members: strs(members),
        themes: strs(&["T"]),
        origin: Some("model".into()),
        description: format!("About {name}."),
        ..EventRecord::default()
    }
}

pub fn put(dir: &Path, r: &EventRecord) {
    fs::write(dir.join(format!("{}.md", r.id)), render_record(r)).unwrap();
}

/// An archive whose records folder holds four events: alpha (September),
/// beta (October), gamma (November; edited by him, with a body of his
/// own) and delta (August).
pub struct Fx {
    pub tmp: tempfile::TempDir,
    pub dir: PathBuf,
}

pub fn fixture() -> Fx {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join(events::EVENTS_DIR);
    fs::create_dir_all(&dir).unwrap();
    let mut alpha = rec(
        ALPHA,
        "Alpha story",
        ("2026-09-05", "2026-09-20"),
        &["2026-09/E01"],
    );
    alpha.aliases = strs(&["The Alpha", "Shared"]);
    let mut beta = rec(
        BETA,
        "Beta follow-up",
        ("2026-10-02", "2026-10-09"),
        &["2026-10/E04"],
    );
    beta.aliases = strs(&["Beta", "shared", "ALPHA STORY"]);
    let mut gamma = rec(
        GAMMA,
        "Gamma",
        ("2026-11-01", "2026-11-03"),
        &["2026-11/E02"],
    );
    gamma.origin = Some("dominik".into());
    gamma.edited = Some("2026-10-10".into());
    gamma.aliases = strs(&["His alias"]);
    gamma.description = GAMMA_BODY.into();
    let delta = rec(
        DELTA,
        "Delta",
        ("2026-08-01", "2026-08-02"),
        &["2026-08/E07"],
    );
    for r in [&alpha, &beta, &gamma, &delta] {
        put(&dir, r);
    }
    // Not records.
    fs::write(dir.join("notes.txt"), "mine\n").unwrap();
    Fx { tmp, dir }
}

impl Fx {
    pub fn root(&self) -> &Path {
        self.tmp.path()
    }

    pub fn editor(&self) -> EventEditor {
        EventEditor::open(self.root()).unwrap().with_now(NOW)
    }

    pub fn set(&self) -> EventSet {
        read_all(&self.dir)
    }

    pub fn record(&self, id: &str) -> EventRecord {
        self.set()
            .records
            .get(id)
            .unwrap_or_else(|| panic!("no {id}"))
            .clone()
    }

    pub fn bytes(&self, id: &str) -> Vec<u8> {
        fs::read(self.dir.join(format!("{id}.md"))).unwrap()
    }

    pub fn tree(&self) -> BTreeMap<PathBuf, Vec<u8>> {
        tree(self.root())
    }

    /// The lines of a log of the records folder (none when it is missing).
    pub fn log(&self, name: &str) -> Vec<Value> {
        log_lines(&self.dir.join(name))
    }
}

pub fn log_lines(path: &Path) -> Vec<Value> {
    let Ok(text) = fs::read_to_string(path) else {
        return vec![];
    };
    assert!(text.is_empty() || text.ends_with('\n'), "{text:?}");
    text.lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
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

/// The folder as the reader has it: every `.md` file a record it reads,
/// no warning.
pub fn read_all(dir: &Path) -> EventSet {
    let set = events::read_dir(dir).unwrap().expect("the records folder");
    assert!(set.refused.is_empty(), "{:?}", set.refused);
    assert!(set.warnings.is_empty(), "{:?}", set.warnings);
    assert!(set.unmerged.is_empty(), "{:?}", set.unmerged);
    set
}

/// The bytes of a record file after its frontmatter block, found without
/// parsing: the writer's frontmatter for the record the reader read is
/// what the file starts with.
pub fn body_bytes(dir: &Path, id: &str) -> Vec<u8> {
    let bytes = fs::read(dir.join(format!("{id}.md"))).unwrap();
    let r = events::parse_bytes(&bytes, id).unwrap();
    let mut front = r.clone();
    front.description = String::new();
    let head = render_record(&front);
    let head = head.strip_suffix('\n').unwrap();
    assert!(bytes.starts_with(head.as_bytes()), "{id}: not as written");
    bytes[head.len()..].to_vec()
}

/// Run `op`, which must fail with `needle` in its message and leave every
/// file under the archive as it was.
pub fn refused<T: std::fmt::Debug>(fx: &Fx, needle: &str, op: impl FnOnce() -> anyhow::Result<T>) {
    let before = fx.tree();
    let e = format!("{:#}", op().expect_err(needle));
    assert!(e.contains(needle), "want {needle:?} in {e}");
    assert!(fx.tree() == before, "{needle}: the folder changed");
}

pub fn merge_req(a: &str, b: &str, by: By) -> MergeRequest {
    MergeRequest {
        a: a.into(),
        b: b.into(),
        by,
        reason: Some("one story".into()),
    }
}

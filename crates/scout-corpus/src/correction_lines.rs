//! Numbered lines of `corrections.jsonl`: the ONE judgement of them, and
//! the reading of the file line by line.
//!
//! A numbered line is one whose `id` is a canonical numbered id: `c` and
//! the number (1 to 999,999,999) written with at least six digits and no
//! more than it needs (`c000001`, `c1000000`), as [`correction_number`]
//! reads it and `scout correct` writes it. The reader, the writer and
//! `scout correct list` all tell a numbered line by that one function. Whether such a line is valid is decided
//! here and nowhere else ([`Sequence::judge`]): the reader
//! ([`crate::topics`]) skips and counts a line this function rejects, and
//! the writer ([`crate::corrections`]) refuses to append to a file that
//! holds one, naming the line and the reason given here. So the two agree
//! by construction: there is no numbered line the reader applies and the
//! writer would not have written, and none the writer accepts and the
//! reader skips.
//!
//! A numbered line is valid when:
//!
//! - it has a text `at`, `date`, `by` and `key`;
//! - besides those and `id` it has exactly one change: one of
//!   `theme_add`, `theme_remove`, `entity_add`, `entity_remove`,
//!   `event_add`, `event_remove` holding a list of one non-blank name, or
//!   `undoes` holding a numbered id; and no other key but `entity_type`;
//! - `entity_type` (one of [`ENTITY_TYPES`]) is there exactly when the
//!   change is `entity_add`;
//! - its id is greater than the id of every earlier numbered line, valid
//!   or not;
//! - an `undoes` names an earlier VALID correction (not an undo line) of
//!   the same `key` that no earlier line has undone.
//!
//! A line whose `id` is absent or of another form (`c1`, `C000001`, and
//! `c0000001` with a zero too many) is not a numbered line:
//! it is a hand-written line, read as before numbered lines existed, and
//! nothing here judges it.
//!
//! Reading ([`read`]): the lines go past twice. The first pass finds
//! which corrections an undo line takes back; the second applies every
//! other line in order. Nothing of a rejected line is kept, and of a
//! valid one only what a later line can refer to: per valid numbered
//! correction its number and key ([`Kept`] counts them). A file of a
//! million lines that are not corrections keeps nothing.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha1::{Digest, Sha1};
use std::collections::BTreeMap;
use std::io::{Read, Seek};
use std::path::Path;

/// Largest corrections file read by the reader, and the largest an append
/// of the writer may leave: one limit for both. A larger file is not read
/// at all ([`Skipped`]).
pub const MAX_FILE_BYTES: u64 = 64 << 20;

/// The entity types of the topic layer (`entities_typed[].type`).
pub const ENTITY_TYPES: &[&str] = &[
    "person", "org", "model", "product", "work", "event", "place", "other",
];

type Object = serde_json::Map<String, Value>;

/// What one correction does. Its name is the key the line carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Op {
    ThemeAdd,
    ThemeRemove,
    EntityAdd,
    EntityRemove,
    EventAdd,
    EventRemove,
}

pub(crate) const OPS: [Op; 6] = [
    Op::ThemeAdd,
    Op::ThemeRemove,
    Op::EntityAdd,
    Op::EntityRemove,
    Op::EventAdd,
    Op::EventRemove,
];

impl Op {
    /// The key of the line, and the op's name in every report.
    pub fn name(self) -> &'static str {
        match self {
            Op::ThemeAdd => "theme_add",
            Op::ThemeRemove => "theme_remove",
            Op::EntityAdd => "entity_add",
            Op::EntityRemove => "entity_remove",
            Op::EventAdd => "event_add",
            Op::EventRemove => "event_remove",
        }
    }
}

/// The number of a numbered correction's id: the canonical spelling
/// only ([`crate::numbered_id`]), so one id per number.
pub(crate) fn correction_number(id: &str) -> Option<u64> {
    crate::numbered_id::number('c', id)
}

/// The id of the numbered correction `n`.
pub(crate) fn correction_id(n: u64) -> String {
    crate::numbered_id::write('c', n)
}

/// The id of a numbered line, when `m` is one.
pub(crate) fn numbered(m: &Object) -> Option<&str> {
    m.get("id")
        .and_then(|v| v.as_str())
        .filter(|id| correction_number(id).is_some())
}

/// A valid numbered line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Numbered {
    /// A correction.
    Change {
        op: Op,
        value: String,
        entity_type: Option<String>,
    },
    /// An undo line.
    Undo { undoes: String },
}

/// A string value of a numbered line that must be there.
fn required<'a>(m: &'a Object, k: &str) -> Result<&'a str, String> {
    m.get(k)
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("a numbered line without a text {k:?}"))
}

/// The shape of a numbered line, apart from the lines before it: its
/// keys, its one change and its values. Only [`Sequence::judge`] calls
/// this; it is not the whole judgement.
fn shape(m: &Object) -> Result<Numbered, String> {
    for k in ["at", "date", "by", "key"] {
        required(m, k)?;
    }
    let mut found: Option<Numbered> = None;
    let mut put = |w: Numbered| match found.replace(w) {
        None => Ok(()),
        Some(_) => Err("a numbered line with more than one change".to_string()),
    };
    for (k, v) in m {
        match k.as_str() {
            "id" | "at" | "date" | "by" | "key" | "entity_type" => {}
            "undoes" => {
                let id = v
                    .as_str()
                    .filter(|id| correction_number(id).is_some())
                    .ok_or("`undoes` is not a correction id")?;
                put(Numbered::Undo { undoes: id.into() })?;
            }
            other => {
                let Some(op) = OPS.into_iter().find(|op| op.name() == other) else {
                    return Err(format!(
                        "a numbered line with the key {other:?}, which `scout correct` does not write"
                    ));
                };
                let value = match v.as_array().map(Vec::as_slice) {
                    Some([Value::String(s)]) if !s.trim().is_empty() => s.clone(),
                    _ => return Err(format!("{other:?} is not a list of one name")),
                };
                put(Numbered::Change {
                    op,
                    value,
                    entity_type: None,
                })?;
            }
        }
    }
    let etype = match m.get("entity_type") {
        None => None,
        Some(Value::String(t)) if ENTITY_TYPES.contains(&t.as_str()) => Some(t.clone()),
        Some(_) => return Err("`entity_type` is not an entity type".into()),
    };
    match found {
        None => Err("a numbered line with no change and no `undoes`".into()),
        Some(Numbered::Change { op, value, .. }) => {
            if (op == Op::EntityAdd) != etype.is_some() {
                return Err("`entity_type` goes with `entity_add`, and only with it".into());
            }
            Ok(Numbered::Change {
                op,
                value,
                entity_type: etype,
            })
        }
        Some(_) if etype.is_some() => Err("an undo line with an `entity_type`".into()),
        Some(w) => Ok(w),
    }
}

/// The numbered lines of one file, judged in file order. This is the one
/// judgement of a numbered line (see the module notes): the reader and
/// the writer each feed every numbered line of the file, in order, to
/// [`Sequence::judge`].
#[derive(Debug, Default)]
pub(crate) struct Sequence {
    /// The largest number of any numbered line so far, valid or not.
    last: u64,
    /// The key of every valid correction so far, by its number.
    changes: BTreeMap<u64, String>,
    /// Correction number → the number of the undo line that took it back.
    undone: BTreeMap<u64, u64>,
}

impl Sequence {
    /// Judge the next numbered line (`m` has a numbered `id`): what it
    /// is, or why it is not a line `scout correct` writes. A rejected
    /// line is neither applied nor undone, and its number still counts
    /// for the order of the ids after it.
    pub(crate) fn judge(&mut self, m: &Object) -> Result<Numbered, String> {
        let id = numbered(m).ok_or("not a numbered line")?;
        let n = correction_number(id).unwrap_or(0);
        let ascending = n > self.last;
        self.last = self.last.max(n);
        let what = shape(m)?;
        if !ascending {
            return Err(format!(
                "id {id:?} is not greater than the numbered line before"
            ));
        }
        let key = required(m, "key")?.trim();
        match &what {
            Numbered::Change { .. } => {
                self.changes.insert(n, key.to_string());
            }
            Numbered::Undo { undoes } => {
                // Ids ascend, so a number names at most one line.
                let target = correction_number(undoes).unwrap_or(0);
                if self.changes.get(&target).map(String::as_str) != Some(key) {
                    return Err(format!(
                        "undoes {undoes:?}, which no earlier correction of its key is"
                    ));
                }
                if self.undone.contains_key(&target) {
                    return Err(format!("undoes {undoes:?} a second time"));
                }
                self.undone.insert(target, n);
            }
        }
        Ok(what)
    }

    /// The largest number of a numbered line so far.
    pub(crate) fn last(&self) -> u64 {
        self.last
    }

    /// Correction id → the id of the undo line that took it back.
    pub(crate) fn undone_ids(&self) -> BTreeMap<String, String> {
        let id = |n: &u64| correction_id(*n);
        self.undone.iter().map(|(c, u)| (id(c), id(u))).collect()
    }

    fn is_undone(&self, id: &str) -> bool {
        correction_number(id).is_some_and(|n| self.undone.contains_key(&n))
    }

    fn kept(&self) -> Kept {
        Kept {
            corrections: self.changes.len(),
            undone: self.undone.len(),
        }
    }
}

/// What one non-blank line of the corrections file was read as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LineRead {
    /// Applied to its item.
    Applied,
    /// Its key has no tag row; not applied.
    UnknownKey,
    /// Not a JSON object, no `key`, a wrongly typed field, over the line
    /// cap, not UTF-8, or a numbered line [`Sequence::judge`] rejects.
    Bad,
    /// An undo line that took an earlier line back.
    Undo,
    /// A numbered line an undo line took back; not applied.
    Undone,
}

/// How many lines were read as each [`LineRead`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    pub applied: usize,
    pub unknown_key: usize,
    pub bad: usize,
    pub undo: usize,
    pub undone: usize,
}

impl Counts {
    pub(crate) fn add(&mut self, r: LineRead) {
        *match r {
            LineRead::Applied => &mut self.applied,
            LineRead::UnknownKey => &mut self.unknown_key,
            LineRead::Bad => &mut self.bad,
            LineRead::Undo => &mut self.undo,
            LineRead::Undone => &mut self.undone,
        } += 1;
    }
}

/// What reading a file kept in memory while its lines went past: the
/// entries of the two collections of [`Sequence`]. Nothing else of any
/// line outlives the line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Kept {
    /// Valid numbered corrections (their number and key).
    pub corrections: usize,
    /// Corrections taken back (two numbers each).
    pub undone: usize,
}

fn object(line: Option<&str>) -> Option<Object> {
    match serde_json::from_str::<Value>(line?) {
        Ok(Value::Object(m)) => Some(m),
        _ => None,
    }
}

/// Read the lines of a corrections file. `pass` feeds every non-blank
/// line, in order, to the function it is given (`None`: a line that is
/// not valid UTF-8 or is over the line cap); it is called twice and must
/// give the same lines both times. `apply` applies one line that is to be
/// applied (a hand-written line, or a valid numbered correction no undo
/// line takes back) and says what became of it; `each` is told what every
/// line was read as, in order.
///
/// No line is kept after the function it was fed to returns.
pub(crate) fn read<E>(
    mut pass: impl FnMut(&mut dyn FnMut(Option<&str>)) -> Result<(), E>,
    mut apply: impl FnMut(&Object) -> LineRead,
    mut each: impl FnMut(LineRead),
) -> Result<Kept, E> {
    // First: which corrections does an undo line take back?
    let mut first = Sequence::default();
    pass(&mut |line| {
        if let Some(m) = object(line).filter(|m| numbered(m).is_some()) {
            let _ = first.judge(&m);
        }
    })?;
    // Then every line in order, judged again as it is met.
    let mut seq = Sequence::default();
    pass(&mut |line| {
        let Some(m) = object(line) else {
            return each(LineRead::Bad);
        };
        let Some(id) = numbered(&m) else {
            return each(apply(&m));
        };
        each(match seq.judge(&m) {
            Err(_) => LineRead::Bad,
            Ok(Numbered::Undo { .. }) => LineRead::Undo,
            Ok(Numbered::Change { .. }) if first.is_undone(id) => LineRead::Undone,
            Ok(Numbered::Change { .. }) => apply(&m),
        });
    })?;
    Ok(first.kept())
}

/// Why a corrections file was not read at all. The build goes on with no
/// correction applied, and says so in its report.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Skipped {
    /// In words, with the size and the limit.
    pub reason: String,
    /// The size of the file, in bytes.
    pub bytes: u64,
    /// The limit ([`MAX_FILE_BYTES`]).
    pub limit: u64,
}

/// A corrections file as opened for reading.
pub(crate) enum Opened {
    Missing,
    /// Larger than the limit: not read.
    TooLarge(Skipped),
    /// The open file and its length at that moment; exactly that many
    /// bytes are read.
    File(std::fs::File, u64),
}

/// Open the corrections file at `path` and measure it on the open file.
pub(crate) fn open(path: &Path, limit: u64) -> Result<Opened> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Opened::Missing),
        Err(e) => return Err(e).with_context(|| format!("read {}", path.display())),
    };
    let bytes = file
        .metadata()
        .with_context(|| format!("read {}", path.display()))?
        .len();
    if bytes > limit {
        return Ok(Opened::TooLarge(Skipped {
            reason: format!(
                "the corrections file is {bytes} bytes, larger than the limit of {limit} bytes; it was not read and NO correction was applied"
            ),
            bytes,
            limit,
        }));
    }
    Ok(Opened::File(file, bytes))
}

/// What stands in the hash of the layer files for a corrections file that
/// was not read: its size, so the index is rebuilt when the file changes.
pub(crate) fn hash_skipped(hash: &mut Sha1, s: &Skipped) {
    hash.update(format!("corrections not read: {} bytes", s.bytes).as_bytes());
}

/// The first `len` bytes of `file`, from its start, for one pass.
pub(crate) fn rewound(
    file: &mut std::fs::File,
    len: u64,
) -> std::io::Result<std::io::BufReader<std::io::Take<&mut std::fs::File>>> {
    file.rewind()?;
    Ok(std::io::BufReader::with_capacity(64 * 1024, file.take(len)))
}

/// The two passes of [`read`] must have read the same bytes.
pub(crate) fn same_bytes(path: &Path, first: &Sha1, second: &Sha1) -> Result<()> {
    if first.clone().finalize() != second.clone().finalize() {
        bail!(
            "{}: the file changed while it was read; run the build again",
            path.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obj(v: Value) -> Object {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn a_numbered_line_is_one_change_with_every_key_of_its_kind() {
        let base = serde_json::json!({"id": "c000001", "at": "t", "date": "d", "by": "dominik", "key": "x:1"});
        let with = |extra: Value| {
            let mut m = obj(base.clone());
            m.extend(obj(extra));
            shape(&m)
        };
        assert_eq!(
            with(serde_json::json!({"theme_add": ["T"]})).unwrap(),
            Numbered::Change {
                op: Op::ThemeAdd,
                value: "T".into(),
                entity_type: None
            }
        );
        assert!(with(serde_json::json!({"entity_add": ["E"], "entity_type": "person"})).is_ok());
        assert_eq!(
            with(serde_json::json!({"undoes": "c000001"})).unwrap(),
            Numbered::Undo {
                undoes: "c000001".into()
            }
        );
        for (extra, why) in [
            (serde_json::json!({}), "no change and no `undoes`"),
            (
                serde_json::json!({"theme_add": ["A", "B"]}),
                "not a list of one name",
            ),
            (
                serde_json::json!({"theme_add": "A"}),
                "not a list of one name",
            ),
            (
                serde_json::json!({"theme_add": [" "]}),
                "not a list of one name",
            ),
            (
                serde_json::json!({"theme_add": ["A"], "theme_remove": ["B"]}),
                "more than one change",
            ),
            (
                serde_json::json!({"theme_add": ["A"], "undoes": "c000001"}),
                "more than one change",
            ),
            (serde_json::json!({"themes": ["A"]}), "does not write"),
            (
                serde_json::json!({"note": "x", "theme_add": ["A"]}),
                "does not write",
            ),
            (
                serde_json::json!({"undoes": "m000001"}),
                "not a correction id",
            ),
            (
                serde_json::json!({"entity_add": ["E"]}),
                "goes with `entity_add`",
            ),
            (
                serde_json::json!({"entity_add": ["E"], "entity_type": "thing"}),
                "not an entity type",
            ),
            (
                serde_json::json!({"theme_add": ["A"], "entity_type": "person"}),
                "goes with `entity_add`",
            ),
            (
                serde_json::json!({"undoes": "c000001", "entity_type": "person"}),
                "an undo line with",
            ),
        ] {
            let e = with(extra.clone()).unwrap_err();
            assert!(e.contains(why), "{extra}: {e}");
        }
        for missing in ["at", "date", "by", "key"] {
            let mut m = obj(base.clone());
            m.extend(obj(serde_json::json!({"theme_add": ["T"]})));
            m.remove(missing);
            let e = shape(&m).unwrap_err();
            assert!(e.contains(missing), "{missing}: {e}");
        }
    }

    fn line(id: &str, key: &str, change: Value) -> Object {
        let mut m =
            obj(serde_json::json!({"id": id, "at": "t", "date": "d", "by": "dominik", "key": key}));
        m.extend(obj(change));
        m
    }

    #[test]
    fn the_sequence_judges_ids_in_order_and_an_undo_by_what_came_before() {
        let add = || serde_json::json!({"theme_add": ["T"]});
        let undo = |id: &str| serde_json::json!({"undoes": id});
        let mut s = Sequence::default();
        assert!(s.judge(&line("c000002", "x:1", add())).is_ok());
        // Not greater than every earlier numbered id.
        for id in ["c000001", "c000002"] {
            let e = s.judge(&line(id, "x:1", add())).unwrap_err();
            assert!(e.contains("is not greater than"), "{e}");
        }
        // A rejected line still counts for the order, and is not undone.
        let e = s
            .judge(&line(
                "c000009",
                "x:1",
                serde_json::json!({"note": "n", "theme_add": ["T"]}),
            ))
            .unwrap_err();
        assert!(e.contains("does not write"), "{e}");
        assert!(s.judge(&line("c000005", "x:1", add())).is_err());
        let e = s
            .judge(&line("c000010", "x:1", undo("c000009")))
            .unwrap_err();
        assert!(e.contains("no earlier correction of its key"), "{e}");
        // Another key's correction, a later one, an undo line: not undone.
        let e = s
            .judge(&line("c000011", "x:2", undo("c000002")))
            .unwrap_err();
        assert!(e.contains("no earlier correction of its key"), "{e}");
        assert!(s.judge(&line("c000012", "x:1", undo("c000099"))).is_err());
        assert!(s.judge(&line("c000013", "x:1", undo("c000002"))).is_ok());
        let e = s
            .judge(&line("c000014", "x:1", undo("c000002")))
            .unwrap_err();
        assert!(e.contains("a second time"), "{e}");
        let e = s
            .judge(&line("c000015", "x:1", undo("c000013")))
            .unwrap_err();
        assert!(e.contains("no earlier correction of its key"), "{e}");
        assert_eq!(s.last(), 15);
        assert_eq!(
            s.undone_ids(),
            BTreeMap::from([("c000002".to_string(), "c000013".to_string())])
        );
        assert_eq!(
            s.kept(),
            Kept {
                corrections: 1,
                undone: 1
            }
        );
    }
}

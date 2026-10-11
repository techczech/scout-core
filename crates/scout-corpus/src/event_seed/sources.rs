//! What the seed reads from the archive (never written): the monthly
//! events of the briefing files and the tag rows that name an event.

use super::naming::{is_period, split_monthly_id};
use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

/// The folder of briefing files, relative to the archive root.
pub const BRIEFINGS_DIR: &str = "analysis/context/briefings";

/// One event of a briefing file.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct MonthlyEvent {
    /// Whitespace runs as one space, trimmed; may be empty.
    pub name: String,
    pub aliases: Vec<String>,
    /// Trimmed.
    pub description: String,
}

/// One tag row that names an event.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct TaggedItem {
    /// `YYYY-MM-DD`, when the row's date starts with one.
    pub date: Option<String>,
    pub themes: Vec<String>,
    pub event_name: Option<String>,
}

/// Whitespace runs to one space, none at the ends.
fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The period of a briefing events file name (`2026-09-events.json`,
/// `2022-Q2-events.json`); `None` for any other name, and for every name
/// holding `.superseded-`.
fn events_file_period(name: &str) -> Option<&str> {
    if name.contains(".superseded-") {
        return None;
    }
    name.strip_suffix("-events.json").filter(|p| is_period(p))
}

/// Monthly events from the briefings folder, by monthly id
/// (`2026-09/E21`). Only plain files directly in the folder are read (no
/// link is followed, no folder walked). A file that is not what a briefing
/// file must be stops the run.
pub(crate) fn read_monthly_events(archive: &Path) -> Result<BTreeMap<String, MonthlyEvent>> {
    let dir = archive.join(BRIEFINGS_DIR);
    let mut out = BTreeMap::new();
    let rd = match std::fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e).with_context(|| format!("read {}", dir.display())),
    };
    let mut files = Vec::new();
    for e in rd {
        let e = e.with_context(|| format!("read {}", dir.display()))?;
        let name = e.file_name();
        let Some(period) = name.to_str().and_then(events_file_period) else {
            continue;
        };
        if e.file_type().is_ok_and(|t| t.is_file()) {
            files.push((period.to_string(), e.path()));
        }
    }
    files.sort();
    for (period, file) in files {
        let at = file.display();
        let bytes = std::fs::read(&file).with_context(|| format!("read {at}"))?;
        let doc: Value = match serde_json::from_slice(&bytes) {
            Ok(v) => v,
            Err(e) => bail!("{at}: not JSON ({e})"),
        };
        if let Some(month) = doc.get("month") {
            if month.as_str() != Some(period.as_str()) {
                bail!("{at}: \"month\" is {month}, the file name says {period}");
            }
        }
        let Some(events) = doc.get("events").and_then(Value::as_array) else {
            bail!("{at}: no \"events\" list");
        };
        for ev in events {
            let number = ev.get("id").and_then(Value::as_str).unwrap_or_default();
            let id = format!("{period}/{number}");
            if split_monthly_id(&id).is_none() {
                bail!(
                    "{at}: event id {} is not E<number>",
                    ev.get("id").unwrap_or(&Value::Null)
                );
            }
            let text = |k: &str| ev.get(k).and_then(Value::as_str).unwrap_or_default();
            let aliases = ev
                .get("aliases")
                .and_then(Value::as_array)
                .map(|l| {
                    l.iter()
                        .filter_map(Value::as_str)
                        .map(one_line)
                        .filter(|a| !a.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            let event = MonthlyEvent {
                name: one_line(text("name")),
                aliases,
                description: text("description").trim().to_string(),
            };
            if out.insert(id.clone(), event).is_some() {
                bail!("{at}: event {id} appears twice");
            }
        }
    }
    Ok(out)
}

/// The tag rows of `analysis/topics/current.jsonl` grouped by their
/// `event`, rows in the order their keys first appear (the last row per
/// key counts, as the reader has it). Lines that are not a JSON object
/// with a `key` are passed over. Corrections are not applied here: the
/// reader applies them at query time.
pub(crate) fn read_tagged_items(archive: &Path) -> Result<BTreeMap<String, Vec<TaggedItem>>> {
    let file = archive.join(crate::topics::LAYER_FILE);
    let bytes = match std::fs::read(&file) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(e).with_context(|| format!("read {}", file.display())),
    };
    let text = String::from_utf8_lossy(&bytes);
    let mut rows: Vec<Value> = Vec::new();
    let mut at: HashMap<String, usize> = HashMap::new();
    for line in text.split('\n') {
        let Ok(row) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        let Some(key) = row.get("key").and_then(Value::as_str).map(str::trim) else {
            continue;
        };
        if key.is_empty() {
            continue;
        }
        match at.get(key) {
            Some(&i) => rows[i] = row,
            None => {
                at.insert(key.to_string(), rows.len());
                rows.push(row);
            }
        }
    }
    let mut by_event: BTreeMap<String, Vec<TaggedItem>> = BTreeMap::new();
    for row in &rows {
        let Some(event) = row.get("event").and_then(Value::as_str).map(str::trim) else {
            continue;
        };
        if split_monthly_id(event).is_none() {
            continue;
        }
        let date = row
            .get("date")
            .and_then(Value::as_str)
            .filter(|d| starts_with_a_date(d))
            .map(|d| d[..10].to_string());
        let themes = row
            .get("themes")
            .and_then(Value::as_array)
            .map(|l| {
                l.iter()
                    .filter_map(Value::as_str)
                    .map(String::from)
                    .collect()
            })
            .unwrap_or_default();
        let event_name = row
            .get("event_name")
            .and_then(Value::as_str)
            .map(one_line)
            .filter(|n| !n.is_empty());
        by_event
            .entry(event.to_string())
            .or_default()
            .push(TaggedItem {
                date,
                themes,
                event_name,
            });
    }
    Ok(by_event)
}

/// `YYYY-MM-DD` at the start (ASCII digits).
fn starts_with_a_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 10
        && b[..10].iter().enumerate().all(|(i, c)| match i {
            4 | 7 => *c == b'-',
            _ => c.is_ascii_digit(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_current_briefing_event_files_are_named() {
        assert_eq!(events_file_period("2026-09-events.json"), Some("2026-09"));
        assert_eq!(events_file_period("2022-Q2-events.json"), Some("2022-Q2"));
        for no in [
            "2026-09-events.superseded-20261011-1603.json",
            "2026-09.superseded-1-events.json",
            "2026-09-briefing.json",
            "2026-13-events.json",
            "x2026-09-events.json",
            "2026-09-events.json.bak",
        ] {
            assert_eq!(events_file_period(no), None, "{no}");
        }
        assert!(starts_with_a_date("2026-09-03T10:00:00Z"));
        assert!(!starts_with_a_date("2026-9-3"));
        assert!(!starts_with_a_date("2026/09/03"));
        assert_eq!(one_line("  a \n\t b\u{a0}c "), "a b c");
    }
}

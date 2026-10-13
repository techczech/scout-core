//! What the seed reads from the archive (never written): the monthly
//! events of the briefing files and the tag rows that name an event.
//!
//! The tag rows come from the topic layer's own reader. The briefing
//! `<period>-events.json` files have no other reader in this workspace
//! (the tagger in `tools/topic-tagger` writes them), so their rules are
//! here, and they are strict: a file that is not what a briefing file
//! must be stops the run with the file named; none is skipped.

use super::naming::{is_period, monthly_id};
use super::render::body_text;
use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

/// The folder of briefing files, relative to the archive root.
pub const BRIEFINGS_DIR: &str = "analysis/context/briefings";

/// One event of a briefing file.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct MonthlyEvent {
    /// Whitespace runs as one space, trimmed; may be empty.
    pub name: String,
    pub aliases: Vec<String>,
    /// A record body: [`body_text`] (no control characters, trimmed).
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
pub(crate) fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The period of a file of the briefings folder named `<period><suffix>`
/// (`2026-09-events.json`, `2022-Q2-briefing.json`); `None` for any other
/// name, and for every name holding `.superseded-`.
pub(crate) fn file_period<'a>(name: &'a str, suffix: &str) -> Option<&'a str> {
    if name.contains(".superseded-") {
        return None;
    }
    name.strip_suffix(suffix).filter(|p| is_period(p))
}

/// The period of a briefing events file name.
fn events_file_period(name: &str) -> Option<&str> {
    file_period(name, "-events.json")
}

/// Monthly events from the briefings folder, by monthly id
/// (`2026-09/E21`). Only entries directly in the folder are looked at (no
/// folder is walked) and only plain files are read (no link is followed).
/// Anything under a briefing file's name that is not what a briefing file
/// must be stops the run, with the file named: not a plain file, not JSON,
/// not an object, a `month` that is not the file name's period, no
/// `events` list, an event that is not an object, an event `id` that is
/// not `E<number>` or is there twice (ids compared as the index compares
/// them: `e01` is `E01`), a `name` or `description` that is
/// not text, `aliases` that is not a list of text. (`name`, `description`
/// and `aliases` may be absent or null.)
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
        // The entry's own type: a link is not followed.
        if !e.file_type().is_ok_and(|t| t.is_file()) {
            bail!("{}: not a plain file", e.path().display());
        }
        files.push((period.to_string(), e.path()));
    }
    files.sort();
    for (period, file) in files {
        let bytes = std::fs::read(&file).with_context(|| format!("read {}", file.display()))?;
        for (id, event) in parse_briefing(&bytes, &period)
            .map_err(|why| anyhow::anyhow!("{}: {why}", file.display()))?
        {
            // One period, one file name: an id cannot come from two files.
            out.insert(id, event);
        }
    }
    Ok(out)
}

/// The events of one briefing file's bytes by monthly id, or what is
/// wrong with the file.
fn parse_briefing(
    bytes: &[u8],
    period: &str,
) -> std::result::Result<BTreeMap<String, MonthlyEvent>, String> {
    let doc: Value = serde_json::from_slice(bytes).map_err(|e| format!("not JSON ({e})"))?;
    let Some(doc) = doc.as_object() else {
        return Err("not a JSON object".into());
    };
    if let Some(month) = doc.get("month") {
        if month.as_str() != Some(period) {
            return Err(format!("\"month\" is {month}, the file name says {period}"));
        }
    }
    let Some(events) = doc.get("events").and_then(Value::as_array) else {
        return Err("no \"events\" list".into());
    };
    let mut out = BTreeMap::new();
    for (i, ev) in events.iter().enumerate() {
        let (id, event) = parse_event(ev, i, period)?;
        if out.insert(id.clone(), event).is_some() {
            return Err(format!("event {id} appears twice"));
        }
    }
    Ok(out)
}

/// Entry `i` (from 0) of a briefing's `events` list as its monthly id and
/// what the briefing says of it, or what is wrong with it. The one reading
/// of a briefing event: the seed's and the briefing read's
/// (`crate::briefings`).
pub(crate) fn parse_event(
    ev: &Value,
    i: usize,
    period: &str,
) -> std::result::Result<(String, MonthlyEvent), String> {
    let Some(ev) = ev.as_object() else {
        return Err(format!("event {} of the list is not an object", i + 1));
    };
    // The id as the index would compare it (`e01`, ` E01 ` are E01).
    let raw = ev.get("id").unwrap_or(&Value::Null);
    let number = crate::normalize::fold(raw.as_str().unwrap_or_default());
    let Some(id) = monthly_id(&format!("{period}/{number}")) else {
        return Err(format!("event id {raw} is not E<number>"));
    };
    // Text, or absent or null (empty).
    let text = |k: &str| match ev.get(k) {
        None | Some(Value::Null) => Ok(""),
        Some(Value::String(s)) => Ok(s.as_str()),
        Some(_) => Err(format!("event {id}: \"{k}\" is not text")),
    };
    let aliases = match ev.get("aliases") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(l)) => l
            .iter()
            .map(|a| a.as_str().map(one_line))
            .collect::<Option<Vec<String>>>()
            .ok_or_else(|| format!("event {id}: \"aliases\" is not a list of text"))?
            .into_iter()
            .filter(|a| !a.is_empty())
            .collect(),
        Some(_) => return Err(format!("event {id}: \"aliases\" is not a list of text")),
    };
    let event = MonthlyEvent {
        name: one_line(text("name")?),
        aliases,
        description: body_text(text("description")?),
    };
    Ok((id, event))
}

/// The tag rows of `analysis/topics/current.jsonl` grouped by their
/// `event` (as [`monthly_id`] spells it), each event's rows in key order. The rows are the ones
/// [`crate::topics::read_tag_rows`] gives, which is how an index build
/// reads the file: a line that is not a valid row is passed over and the
/// last valid row of a key counts. Nothing is parsed here. Corrections
/// are not read (see the module notes of `event_seed`).
pub(crate) fn read_tagged_items(archive: &Path) -> Result<BTreeMap<String, Vec<TaggedItem>>> {
    let mut by_event: BTreeMap<String, Vec<TaggedItem>> = BTreeMap::new();
    let Some(rows) = crate::topics::read_tag_rows(archive)? else {
        return Ok(by_event);
    };
    for row in rows.items.values() {
        // Which monthly event the row is in: the index's own comparison.
        let Some(event) = row.info.event.as_deref().and_then(monthly_id) else {
            continue;
        };
        let date = row
            .date
            .as_deref()
            .filter(|d| starts_with_a_date(d) && crate::topics::valid_month(&d[..7]))
            .map(|d| d[..10].to_string());
        let event_name = row
            .info
            .event_name
            .as_deref()
            .map(one_line)
            .filter(|n| !n.is_empty());
        by_event.entry(event).or_default().push(TaggedItem {
            date,
            themes: row.info.themes.clone(),
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

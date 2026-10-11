//! What a merged event shows beyond its own record, derived from the
//! records each time it is asked for. A merge is one field,
//! `merged_into` on the record merged away; the record merged into holds
//! no copy of the other's members, aliases or dates. So the span and the
//! aliases of the combined event, and the list of records merged into it,
//! are worked out here, by one function each, for every caller.

use super::EventRecord;
use crate::normalize::fold;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// A record merged into an event, directly or through a chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MergedFrom {
    pub id: String,
    pub name: String,
}

/// What an event shows of its own record and the records merged into it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Combined {
    pub start: Option<String>,
    pub end: Option<String>,
    pub aliases: Vec<String>,
    /// The records merged in, in id order; empty for an event nothing is
    /// merged into.
    pub merged_from: Vec<MergedFrom>,
}

/// Is `s` a calendar date written `YYYY-MM-DD`?
pub(crate) fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b.iter().enumerate().all(|(i, c)| {
            if i == 4 || i == 7 {
                *c == b'-'
            } else {
                c.is_ascii_digit()
            }
        })
        && chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()
}

/// A span widened to cover another record's span: `(start, end)`. The one
/// rule for the span of a combined event.
///
/// - `start`: moves back to the other's `start` when that is earlier, and
///   an empty start takes it.
/// - `end`: an empty end stays empty (the event is open); else it moves on
///   to the other's last known date (its `end`, else its `start`) when
///   that is later.
/// - A `start` or `end` of the other that is not a date `YYYY-MM-DD` is
///   passed over, and a value of the span that is not one is left as it
///   is.
pub fn cover(
    span: (&Option<String>, &Option<String>),
    other: (&Option<String>, &Option<String>),
) -> (Option<String>, Option<String>) {
    let date = |d: &'_ Option<String>| d.clone().filter(|d| is_date(d));
    let (o_start, o_end) = (date(other.0), date(other.1));
    let o_last = o_end.or_else(|| o_start.clone());
    let start = match (span.0, o_start) {
        (None, Some(o)) => Some(o),
        (Some(have), Some(o)) if is_date(have) && o < *have => Some(o),
        (have, _) => have.clone(),
    };
    let end = match (span.1, o_last) {
        (Some(have), Some(last)) if is_date(have) && last > *have => Some(last),
        (have, _) => have.clone(),
    };
    (start, end)
}

/// By folded id of each record that is not merged away: the records merged
/// into it, directly or through a chain, in id order. Records nothing is
/// merged into have no entry. `records` is a set's or an index's, where
/// every `merged_into` names a record and closes no cycle.
pub fn merged_in(records: &BTreeMap<String, EventRecord>) -> BTreeMap<String, Vec<&EventRecord>> {
    let mut out: BTreeMap<String, Vec<&EventRecord>> = BTreeMap::new();
    for r in records.values().filter(|r| r.merged_into.is_some()) {
        let mut cur = r;
        // A chain is at most as long as the set.
        for _ in 0..records.len() {
            match cur.merged_into.as_deref().map(fold) {
                Some(next) => match records.get(&next) {
                    Some(n) => cur = n,
                    None => break,
                },
                None => break,
            }
        }
        if cur.merged_into.is_none() {
            out.entry(fold(&cur.id)).or_default().push(r);
        }
    }
    out
}

/// The span, aliases and merged-in list of the event whose own record is
/// `own`, with `merged` the records merged into it ([`merged_in`]).
///
/// - Span: `own`'s dates widened by [`cover`] over each merged record.
/// - Aliases: `own`'s, as they are; then the name and the aliases of each
///   merged record, in the order of `merged`, leaving out any that folds
///   to `own`'s name or to an alias already there.
///
/// With nothing merged in these are `own`'s values, unchanged.
pub fn combined(own: &EventRecord, merged: &[&EventRecord]) -> Combined {
    let mut span = (own.start.clone(), own.end.clone());
    let mut aliases = own.aliases.clone();
    let mut seen: BTreeSet<String> = aliases.iter().map(|a| fold(a)).collect();
    seen.insert(fold(&own.name));
    for r in merged {
        span = cover((&span.0, &span.1), (&r.start, &r.end));
        for n in std::iter::once(&r.name).chain(&r.aliases) {
            let f = fold(n);
            if !f.is_empty() && seen.insert(f) {
                aliases.push(n.clone());
            }
        }
    }
    Combined {
        start: span.0,
        end: span.1,
        aliases,
        merged_from: merged
            .iter()
            .map(|r| MergedFrom {
                id: r.id.clone(),
                name: r.name.clone(),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(id: &str, into: Option<&str>) -> EventRecord {
        EventRecord {
            id: id.into(),
            name: id.to_uppercase(),
            merged_into: into.map(String::from),
            ..EventRecord::default()
        }
    }

    fn dated(id: &str, span: (&str, &str), aliases: &[&str], into: Option<&str>) -> EventRecord {
        let date = |d: &str| (!d.is_empty()).then(|| d.to_string());
        EventRecord {
            start: date(span.0),
            end: date(span.1),
            aliases: aliases.iter().map(|a| a.to_string()).collect(),
            ..rec(id, into)
        }
    }

    #[test]
    fn a_combined_event_covers_and_names_every_record_merged_into_it() {
        let records: BTreeMap<String, EventRecord> = [
            dated("b", ("2026-03-01", "2026-03-05"), &["One", "B"], None),
            // Into b through c; open-ended, so its start is its last date.
            dated("a", ("2026-04-02", ""), &["one", "Two"], Some("c")),
            dated("c", ("2026-02-10", "2026-02-11"), &["b"], Some("b")),
            // Dates that are not dates are passed over.
            dated("d", ("early 2020", "2030"), &[], Some("b")),
            dated("e", ("2026-01-01", "2026-12-31"), &["Own"], None),
        ]
        .into_iter()
        .map(|r| (fold(&r.id), r))
        .collect();
        let merged = merged_in(&records);
        assert_eq!(merged.keys().collect::<Vec<_>>(), ["b"]);
        let all = combined(&records["b"], &merged["b"]);
        assert_eq!(
            (all.start.as_deref(), all.end.as_deref()),
            (Some("2026-02-10"), Some("2026-04-02"))
        );
        // Own first, as they are; then each record's name and aliases in
        // id order, none that folds to b's name or to one already there.
        assert_eq!(all.aliases, ["One", "B", "A", "Two", "C", "D"]);
        let from: Vec<&str> = all.merged_from.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(from, ["a", "c", "d"]);
        // Nothing merged in: the record's own values, whatever they are.
        let own = dated("x", ("soon", ""), &["X", "x", "Dup", "dup"], None);
        let alone = combined(&own, &[]);
        assert_eq!(
            (alone.start, alone.end, alone.aliases),
            (own.start.clone(), own.end.clone(), own.aliases.clone())
        );
        assert!(alone.merged_from.is_empty());
        // An open end stays open; an empty start takes the earliest.
        let open = dated("o", ("", ""), &[], None);
        let all = combined(&open, &[&records["e"], &records["b"]]);
        assert_eq!((all.start.as_deref(), all.end), (Some("2026-01-01"), None));
    }
}

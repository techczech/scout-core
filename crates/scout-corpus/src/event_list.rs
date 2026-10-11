//! `scout events`: every event of the searched corpora with its record
//! fields, its item count and its count per month. An event with a record
//! (`analysis/events/<id>.md`) is listed under its lasting id; a monthly
//! event no record holds is listed under its monthly id with the layer's
//! name. Items are the tagged documents search can return, as in
//! `scout topics`; a document counts once in each of its events.

use crate::api::{Engine, Outcome, Reply};
use crate::events::{self, EventRecord};
use crate::index;
use crate::normalize::fold;
use crate::topics::valid_month;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Schema version of `scout events --json`.
pub const SCHEMA_VERSION: u32 = 1;

/// `scout events`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EventsQuery {
    /// Only events with an item in this layer month (`YYYY-MM` or `YYYY`).
    pub month: Option<String>,
    #[serde(rename = "in")]
    pub in_: Vec<String>,
    /// Leave out events with fewer items (all months counted); 0 also
    /// lists records without items.
    pub min_items: usize,
    /// Also list records merged into another (they hold no items).
    pub merged: bool,
    /// Also list removed records (`removed` set; they hold no items).
    pub removed: bool,
    /// Events kept (0 = all).
    pub top: usize,
}

impl Default for EventsQuery {
    fn default() -> Self {
        EventsQuery {
            month: None,
            in_: vec![],
            min_items: 1,
            merged: false,
            removed: false,
            top: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MonthCount {
    pub month: String,
    pub count: usize,
}

/// One event of one corpus.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct EventListing {
    pub corpus: String,
    /// The lasting id, or the monthly id of an event without a record.
    pub id: String,
    pub name: Option<String>,
    /// Does `analysis/events/<id>.md` exist? The fields below `months` come
    /// from it and are empty without it.
    pub record: bool,
    pub start: Option<String>,
    pub end: Option<String>,
    /// Tagged documents in the event, all months.
    pub items: usize,
    /// Of those, the documents in the query's `month` (only with `month`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub month_items: Option<usize>,
    /// Items per layer month, chronological; an item without a month is in
    /// `items` only.
    pub months: Vec<MonthCount>,
    pub members: Vec<String>,
    pub aliases: Vec<String>,
    pub themes: Vec<String>,
    pub origin: Option<String>,
    pub edited: Option<String>,
    pub merged_into: Option<String>,
    /// The date the event was removed; the key is absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub removed: Option<String>,
    pub description: String,
}

/// `events --json`.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct EventsResults {
    pub schema_version: u32,
    pub month: Option<String>,
    pub corpora: Vec<String>,
    pub min_items: usize,
    pub merged: bool,
    /// Removed records are listed too; the key is absent otherwise.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub removed: bool,
    /// Events before `top`.
    pub distinct: usize,
    pub events: Vec<EventListing>,
}

impl Outcome for EventsResults {
    fn has_results(&self) -> bool {
        !self.events.is_empty()
    }
}

#[derive(Default)]
struct Tally {
    spelling: String,
    items: usize,
    months: BTreeMap<String, usize>,
    names: BTreeMap<String, usize>,
}

fn in_month(have: &str, want: &str) -> bool {
    have == want || (want.len() == 4 && have.starts_with(want))
}

impl Engine {
    /// Every event with its record fields and counts (see the module
    /// notes). Ordered by items, most first, then id.
    pub fn events(&self, q: &EventsQuery) -> Result<Reply<EventsResults>> {
        let month = q.month.as_deref().map(str::trim);
        if let Some(m) = month {
            if !valid_month(m) {
                bail!("month:{m}: expected YYYY-MM or YYYY");
            }
        }
        let mut notes = vec![];
        let corpora = self.scope(&q.in_, &mut notes)?;
        let mut out = Vec::new();
        for c in &corpora {
            let conn = index::open_existing(&c.config, c.index_path())?;
            let records = events::load(&conn)?;
            let mut tally: BTreeMap<String, Tally> = BTreeMap::new();
            for d in crate::filter::load_docs(&conn)?.values() {
                let Some(t) = d.topic.as_ref().filter(|_| d.tokens > 0) else {
                    continue;
                };
                for id in crate::events::dedup(t.event_ids().iter().cloned()) {
                    let e = tally.entry(fold(&id)).or_default();
                    e.items += 1;
                    if let Some(m) = &t.month {
                        *e.months.entry(m.clone()).or_default() += 1;
                    }
                    // The layer's name belongs to the tag row's own id.
                    if let (Some(own), Some(n)) = (&t.event, &t.event_name) {
                        if fold(own) == fold(&id) {
                            *e.names.entry(n.clone()).or_default() += 1;
                        }
                    }
                    if e.spelling.is_empty() {
                        e.spelling = id;
                    }
                }
            }
            for (k, r) in &records {
                let hidden =
                    (r.merged_into.is_some() && !q.merged) || (r.removed.is_some() && !q.removed);
                if hidden {
                    tally.remove(k);
                } else {
                    tally.entry(k.clone()).or_default();
                }
            }
            for (k, t) in tally {
                let record = records.get(&k);
                let month_items = month.map(|m| {
                    t.months
                        .iter()
                        .filter(|(have, _)| in_month(have, m))
                        .map(|(_, c)| *c)
                        .sum::<usize>()
                });
                if t.items < q.min_items || month_items == Some(0) {
                    continue;
                }
                out.push(listing(c.id(), record, t, month_items));
            }
        }
        out.sort_by(|a, b| {
            b.items
                .cmp(&a.items)
                .then_with(|| a.id.cmp(&b.id))
                .then_with(|| a.corpus.cmp(&b.corpus))
        });
        let distinct = out.len();
        if q.top > 0 {
            out.truncate(q.top);
        }
        Ok(Reply {
            body: EventsResults {
                schema_version: SCHEMA_VERSION,
                month: month.map(String::from),
                corpora: corpora.iter().map(|c| c.id().to_string()).collect(),
                min_items: q.min_items,
                merged: q.merged,
                removed: q.removed,
                distinct,
                events: out,
            },
            notes,
        })
    }
}

fn listing(
    corpus: &str,
    record: Option<&EventRecord>,
    t: Tally,
    month_items: Option<usize>,
) -> EventListing {
    // Without a record: the layer's most frequent name (ties: the first).
    let layer_name = || {
        let max = t.names.values().max()?;
        t.names
            .iter()
            .find(|(_, c)| *c == max)
            .map(|(n, _)| n.clone())
    };
    let r = record.cloned().unwrap_or_default();
    EventListing {
        corpus: corpus.to_string(),
        id: record.map_or_else(|| t.spelling.clone(), |r| r.id.clone()),
        name: record.map(|r| r.name.clone()).or_else(layer_name),
        record: record.is_some(),
        start: r.start,
        end: r.end,
        items: t.items,
        month_items,
        months: t
            .months
            .iter()
            .map(|(month, count)| MonthCount {
                month: month.clone(),
                count: *count,
            })
            .collect(),
        members: r.members,
        aliases: r.aliases,
        themes: r.themes,
        origin: r.origin,
        edited: r.edited,
        merged_into: r.merged_into,
        removed: r.removed,
        description: r.description,
    }
}

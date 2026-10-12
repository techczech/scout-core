//! `scout events`: every event of the searched corpora with its record
//! fields, its item count and its count per month. An event with a record
//! (`analysis/events/<id>.md`) is listed under its lasting id; a monthly
//! event no record holds is listed under its monthly id with the layer's
//! name. Items are the tagged documents search can return, as in
//! `scout topics`; a document counts once in each of its events.
//!
//! A record that others are merged into is listed as the combined event
//! ([`events::combined`]): `start` / `end` cover the merged records,
//! `aliases` take in their names and aliases, `merged_from` names them
//! (each with the `merge_id` of the merge in effect for it now, when
//! there is one: the answer of `scout events merges` over the corpus's
//! records folder as it is, asked only for a corpus that has a merged
//! record; an entry the folder no longer agrees with has none, and the
//! reply then carries `stale`), and
//! `own_start` / `own_end` / `own_aliases` give the record's own value
//! where the combined one differs. `members`, `themes`, `origin`, `edited` and the
//! description are the record's own. A record merged away (listed with
//! `merged`) is listed with its own values only.

use crate::api::{Engine, Outcome, Reply};
use crate::events::{self, EventRecord, MergedFrom, EVENTS_DIR};
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

/// The earliest and the latest day of an event's items in one month.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MonthSpan {
    pub first: String,
    pub last: String,
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
    /// The event's span: the record's own dates, widened to cover the
    /// records merged into it.
    pub start: Option<String>,
    pub end: Option<String>,
    /// The record's own `start` / `end`, only when the one above differs
    /// (null: the record has none); the key is absent otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub own_start: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub own_end: Option<Option<String>>,
    /// Tagged documents in the event, all months.
    pub items: usize,
    /// Of those, the documents in the query's `month` (only with `month`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub month_items: Option<usize>,
    /// Items per layer month, chronological; an item without a month is in
    /// `items` only.
    pub months: Vec<MonthCount>,
    /// By month of `months`: the earliest and the latest day
    /// (`YYYY-MM-DD`) of the event's items in that month, from each
    /// item's tag-row date ([`crate::topics::TopicInfo::day`]). A month
    /// none of whose items has a day has no entry; the key is absent when
    /// no month has one.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub month_spans: BTreeMap<String, MonthSpan>,
    /// The items that have no layer month: in `items`, in no month of
    /// `months`. The key is absent when there are none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub undated_items: Option<usize>,
    /// The record's own members (the monthly ids of the records merged
    /// into it are in their records).
    pub members: Vec<String>,
    /// The record's own aliases, then the names and aliases of the
    /// records merged into it.
    pub aliases: Vec<String>,
    /// The record's own aliases (the ones `set --remove-alias` can
    /// remove), only when `aliases` differs from them; the key is absent
    /// otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub own_aliases: Option<Vec<String>>,
    /// The records merged into this one, directly or through a chain, in
    /// id order; the key is absent when there are none.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub merged_from: Vec<MergedFrom>,
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
    /// The events folder of a searched corpus changed since its index was
    /// built (a record listed as merged in is not merged in that way in
    /// the folder now): rebuild the index. The key is absent otherwise.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub stale: bool,
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
    /// Month → (earliest, latest) day of its items that have one.
    spans: BTreeMap<String, (String, String)>,
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
        let mut stale = false;
        for c in &corpora {
            let conn = index::open_existing(&c.config, c.index_path())?;
            let records = events::load(&conn)?;
            let merged_in = events::merged_in(&records);
            // The merge of each record merged in, from the folder as it
            // is now: read only where there is one to show. A folder or
            // a log that cannot be read costs the ids, never the listing.
            let merge_ids = if merged_in.is_empty() {
                BTreeMap::new()
            } else {
                let dir = c.config.root().join(EVENTS_DIR);
                match crate::event_edit::live_merge_ids(&dir, &records) {
                    Ok(live) => {
                        stale |= live.stale;
                        live.ids
                    }
                    Err(e) => {
                        notes.push(format!("scout: note: no merge_id for {}: {e:#}", c.id()));
                        BTreeMap::new()
                    }
                }
            };
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
                        if let Some(day) = t.day.as_ref().filter(|d| d.starts_with(m.as_str())) {
                            let span = e
                                .spans
                                .entry(m.clone())
                                .or_insert_with(|| (day.clone(), day.clone()));
                            if *day < span.0 {
                                span.0 = day.clone();
                            }
                            if *day > span.1 {
                                span.1 = day.clone();
                            }
                        }
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
                let merged = merged_in.get(&k).map_or(&[][..], Vec::as_slice);
                out.push(listing(c.id(), record, merged, &merge_ids, t, month_items));
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
                stale,
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
    merged: &[&EventRecord],
    merge_ids: &BTreeMap<String, String>,
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
    // A record merged away is listed as itself.
    let merged = if r.merged_into.is_some() { &[] } else { merged };
    let mut all = events::combined(&r, merged);
    for m in all.merged_from.iter_mut() {
        m.merge_id = merge_ids.get(&fold(&m.id)).cloned();
    }
    let dated: usize = t.months.values().sum();
    let own = |own: &Option<String>, shown: &Option<String>| (own != shown).then(|| own.clone());
    EventListing {
        corpus: corpus.to_string(),
        id: record.map_or_else(|| t.spelling.clone(), |r| r.id.clone()),
        name: record.map(|r| r.name.clone()).or_else(layer_name),
        record: record.is_some(),
        own_start: own(&r.start, &all.start),
        own_end: own(&r.end, &all.end),
        start: all.start,
        end: all.end,
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
        month_spans: t
            .spans
            .into_iter()
            .map(|(month, (first, last))| (month, MonthSpan { first, last }))
            .collect(),
        undated_items: (t.items > dated).then(|| t.items - dated),
        members: r.members,
        own_aliases: (r.aliases != all.aliases).then(|| r.aliases.clone()),
        aliases: all.aliases,
        merged_from: all.merged_from,
        themes: r.themes,
        origin: r.origin,
        edited: r.edited,
        merged_into: r.merged_into,
        removed: r.removed,
        description: r.description,
    }
}

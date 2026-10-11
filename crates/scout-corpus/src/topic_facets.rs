//! Facet counting over the topic layer: documents per theme, entity, event
//! and month (`scout facets`, `scout topics`). Re-exported from
//! [`crate::topics`].

use crate::events::EventRecord;
use crate::filter::DocInfo;
use crate::normalize::fold;
use anyhow::{bail, Result};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// One facet value and the documents carrying it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FacetItem {
    pub value: String,
    pub count: usize,
    /// Events: the event's name (its record's, else the layer's).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Facet {
    /// Distinct values before `top`.
    pub distinct: usize,
    pub items: Vec<FacetItem>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct AiCounts {
    #[serde(rename = "true")]
    pub yes: usize,
    #[serde(rename = "false")]
    pub no: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Facets {
    pub theme: Facet,
    pub entity: Facet,
    pub event: Facet,
    pub month: Facet,
    pub ai: AiCounts,
}

/// Which facet a listing asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TopicField {
    Theme,
    Entity,
    Event,
    Month,
}

impl std::str::FromStr for TopicField {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        Ok(match s.trim().to_lowercase().as_str() {
            "theme" | "themes" => TopicField::Theme,
            "entity" | "entities" => TopicField::Entity,
            "event" | "events" => TopicField::Event,
            "month" | "months" => TopicField::Month,
            other => bail!("unknown topic field {other:?}: use themes, entities, events or months"),
        })
    }
}

/// Spellings per case-folded value, and the documents per value.
#[derive(Debug, Default)]
struct ValueCounts {
    /// folded value -> (spelling -> documents, documents)
    values: BTreeMap<String, (BTreeMap<String, usize>, usize)>,
}

impl ValueCounts {
    fn add_doc<'a>(&mut self, vals: impl IntoIterator<Item = &'a String>) {
        let mut seen = BTreeSet::new();
        for v in vals {
            let f = fold(v);
            if f.is_empty() || !seen.insert(f.clone()) {
                continue;
            }
            let e = self.values.entry(f).or_default();
            *e.0.entry(v.trim().to_string()).or_default() += 1;
            e.1 += 1;
        }
    }

    /// Items by count desc, then value; the displayed spelling is the most
    /// frequent one (ties: alphabetically first). `top` 0 keeps all. Only
    /// folded values passing `keep` are counted in.
    fn facet(&self, top: usize, chronological: bool, keep: impl Fn(&str) -> bool) -> Facet {
        let mut items: Vec<FacetItem> = self
            .values
            .iter()
            .filter(|(folded, _)| keep(folded))
            .map(|(_, (spellings, count))| {
                let max = spellings.values().max().copied().unwrap_or(0);
                let value = spellings
                    .iter()
                    .find(|(_, c)| **c == max)
                    .map(|(s, _)| s.clone())
                    .unwrap_or_default();
                FacetItem {
                    value,
                    count: *count,
                    name: None,
                }
            })
            .collect();
        if chronological {
            items.sort_by(|a, b| a.value.cmp(&b.value));
        } else {
            items.sort_by(|a, b| b.count.cmp(&a.count).then(a.value.cmp(&b.value)));
        }
        let distinct = items.len();
        if top > 0 {
            items.truncate(top);
        }
        Facet { distinct, items }
    }
}

/// Counts documents per theme, entity, event and month. Feed it the
/// documents of a result set (several corpora may feed one counter); each
/// document counts once per value, so once in each of its events. Values
/// match case-insensitively, as the query fields do, so a facet's count is
/// the document total of the matching `theme:` / `entity:` / `event:` /
/// `month:` query.
#[derive(Debug, Default)]
pub struct FacetCounter {
    pub documents: usize,
    pub tagged: usize,
    theme: ValueCounts,
    entity: ValueCounts,
    event: ValueCounts,
    month: ValueCounts,
    event_names: BTreeMap<String, BTreeMap<String, usize>>,
    /// Names from event records, by folded id; they lead the layer's names.
    record_names: BTreeMap<String, String>,
    /// Event sizes over whole corpora, by folded id (see `event_floor`).
    event_sizes: BTreeMap<String, usize>,
    min_event_items: usize,
    ai: AiCounts,
}

impl FacetCounter {
    pub fn add(&mut self, d: &DocInfo) {
        self.documents += 1;
        let Some(t) = &d.topic else {
            return;
        };
        self.tagged += 1;
        self.theme.add_doc(&t.themes);
        self.entity.add_doc(&t.entities);
        self.event.add_doc(t.event_ids());
        self.month.add_doc(t.month.iter());
        // The layer's name belongs to the tag row's own event id; it names
        // a facet value only where that id is the value (no record).
        if let (Some(e), Some(n)) = (&t.event, &t.event_name) {
            if t.has_event(e) {
                *self
                    .event_names
                    .entry(fold(e))
                    .or_default()
                    .entry(n.clone())
                    .or_default() += 1;
            }
        }
        match t.ai_related {
            Some(true) => self.ai.yes += 1,
            Some(false) => self.ai.no += 1,
            None => {}
        }
    }

    /// Name events from a corpus's records (their names win over the
    /// layer's `event_name`).
    pub fn add_event_records(&mut self, records: &BTreeMap<String, EventRecord>) {
        for (k, r) in records {
            self.record_names
                .entry(k.clone())
                .or_insert_with(|| r.name.clone());
        }
    }

    /// Leave out of the event facet every event with fewer than `min`
    /// items. An event's size is its documents over the whole of the
    /// corpora fed through [`FacetCounter::add_event_size`], not its count
    /// in the result set. `min` 0 or 1 keeps every event.
    pub fn event_floor(&mut self, min: usize) {
        self.min_event_items = min;
    }

    /// Count one document of a corpus towards its events' sizes.
    pub fn add_event_size(&mut self, d: &DocInfo) {
        if let Some(t) = &d.topic {
            let mut seen = BTreeSet::new();
            for e in t.event_ids() {
                let f = fold(e);
                if !f.is_empty() && seen.insert(f.clone()) {
                    *self.event_sizes.entry(f).or_default() += 1;
                }
            }
        }
    }

    fn event_facet(&self, top: usize) -> Facet {
        let min = self.min_event_items;
        let mut f = self.event.facet(top, false, |folded| {
            min <= 1 || self.event_sizes.get(folded).copied().unwrap_or(0) >= min
        });
        for it in f.items.iter_mut() {
            let key = fold(&it.value);
            it.name = self.record_names.get(&key).cloned().or_else(|| {
                let ns = self.event_names.get(&key)?;
                let max = ns.values().max()?;
                ns.iter().find(|(_, c)| *c == max).map(|(n, _)| n.clone())
            });
        }
        f
    }

    /// All four facets; `top` cuts themes, entities and events (0 = all);
    /// months are all listed, chronologically.
    pub fn facets(&self, top: usize) -> Facets {
        Facets {
            theme: self.theme.facet(top, false, |_| true),
            entity: self.entity.facet(top, false, |_| true),
            event: self.event_facet(top),
            month: self.month.facet(0, true, |_| true),
            ai: self.ai.clone(),
        }
    }

    /// One facet (`scout topics`); months chronological, others by count.
    pub fn facet(&self, field: TopicField, top: usize) -> Facet {
        match field {
            TopicField::Theme => self.theme.facet(top, false, |_| true),
            TopicField::Entity => self.entity.facet(top, false, |_| true),
            TopicField::Event => self.event_facet(top),
            TopicField::Month => self.month.facet(top, true, |_| true),
        }
    }
}

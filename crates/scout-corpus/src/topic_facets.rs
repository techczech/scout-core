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
    /// Entities: the entity's type, when a counted document gives it one
    /// (the most frequent type among the counted documents; ties: the
    /// first in type-name order).
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Events: the event's name (its record's, else the layer's).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Events, and only when more than one corpus was searched and any of
    /// them has a records folder: the corpus the event belongs to (the
    /// same id in two corpora is then two values).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub corpus: Option<String>,
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
                    kind: None,
                    name: None,
                    corpus: None,
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

/// The events of one corpus: an event id names an event only within its
/// corpus (as `scout events` lists them), so nothing here is shared.
#[derive(Debug, Default)]
struct CorpusEvents {
    /// The corpus id; empty for a counter never told its corpus.
    corpus: String,
    counts: ValueCounts,
    /// Layer names by folded monthly id.
    names: BTreeMap<String, BTreeMap<String, usize>>,
    /// Names from this corpus's event records, by folded id; they lead the
    /// layer's names.
    record_names: BTreeMap<String, String>,
    /// Event sizes over the whole corpus, by folded id (see `event_floor`).
    sizes: BTreeMap<String, usize>,
}

/// Counts documents per theme, entity, event and month. Feed it the
/// documents of a result set; each document counts once per value, so once
/// in each of its events. Values match case-insensitively, as the query
/// fields do, so a facet's count is the document total of the matching
/// `theme:` / `entity:` / `event:` / `month:` query.
///
/// Several corpora may feed one counter. Themes, entities and months are
/// counted across them. Events are too, one value per id, unless the
/// caller names each corpus with [`FacetCounter::corpus`] before feeding
/// it: then an event belongs to its corpus, so the same id in two corpora
/// gives two values, each counted, named and sized (the `event_floor`)
/// within its own corpus, and each carrying `corpus`. With one corpus
/// named, or none, no value carries `corpus`. The engine names corpora
/// exactly when a searched corpus has a records folder.
#[derive(Debug, Default)]
pub struct FacetCounter {
    pub documents: usize,
    pub tagged: usize,
    theme: ValueCounts,
    entity: ValueCounts,
    /// Folded entity name -> (type -> documents giving it that type).
    entity_kinds: BTreeMap<String, BTreeMap<String, usize>>,
    month: ValueCounts,
    /// One entry per corpus fed, in feeding order.
    events: Vec<CorpusEvents>,
    /// The entry of the corpus being fed.
    feeding: usize,
    min_event_items: usize,
    ai: AiCounts,
}

impl FacetCounter {
    /// What follows (documents, records, sizes) belongs to corpus `id`.
    /// Naming a corpus again goes back to it.
    pub fn corpus(&mut self, id: &str) {
        self.feeding = match self.events.iter().position(|e| e.corpus == id) {
            Some(at) => at,
            None => {
                self.events.push(CorpusEvents {
                    corpus: id.to_string(),
                    ..CorpusEvents::default()
                });
                self.events.len() - 1
            }
        };
    }

    /// The current corpus's events (an unnamed one when none was named).
    fn current(&mut self) -> &mut CorpusEvents {
        if self.events.is_empty() {
            self.events.push(CorpusEvents::default());
        }
        &mut self.events[self.feeding]
    }

    pub fn add(&mut self, d: &DocInfo) {
        self.documents += 1;
        let Some(t) = &d.topic else {
            return;
        };
        self.tagged += 1;
        self.theme.add_doc(&t.themes);
        self.entity.add_doc(&t.entities);
        // Once per document and name, as the count is.
        let typed: BTreeSet<String> = t.entities.iter().map(|e| fold(e)).collect();
        for name in typed {
            if let Some(kind) = t.entity_types.get(&name) {
                *self
                    .entity_kinds
                    .entry(name)
                    .or_default()
                    .entry(kind.clone())
                    .or_default() += 1;
            }
        }
        self.month.add_doc(t.layer_month().map(String::from).iter());
        let ev = self.current();
        ev.counts.add_doc(t.event_ids());
        // The layer's name belongs to the tag row's own event id; it names
        // a facet value only where that id is the value (no record).
        if let (Some(e), Some(n)) = (&t.event, &t.event_name) {
            if t.has_event(e) {
                *ev.names
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

    /// Name the current corpus's events from its records (their names win
    /// over the layer's `event_name`).
    pub fn add_event_records(&mut self, records: &BTreeMap<String, EventRecord>) {
        let ev = self.current();
        for (k, r) in records {
            ev.record_names.insert(k.clone(), r.name.clone());
        }
    }

    /// Leave out of the event facet every event with fewer than `min`
    /// items. An event's size is its documents over the whole of its own
    /// corpus, fed through [`FacetCounter::add_event_size`], not its count
    /// in the result set. `min` 0 or 1 keeps every event.
    pub fn event_floor(&mut self, min: usize) {
        self.min_event_items = min;
    }

    /// Count one document of the current corpus towards its events' sizes.
    pub fn add_event_size(&mut self, d: &DocInfo) {
        if let Some(t) = &d.topic {
            let ev = self.current();
            let mut seen = BTreeSet::new();
            for e in t.event_ids() {
                let f = fold(e);
                if !f.is_empty() && seen.insert(f.clone()) {
                    *ev.sizes.entry(f).or_default() += 1;
                }
            }
        }
    }

    fn event_facet(&self, top: usize) -> Facet {
        let min = self.min_event_items;
        let several = self.events.len() > 1;
        let mut items = Vec::new();
        for ev in &self.events {
            let f = ev.counts.facet(0, false, |folded| {
                min <= 1 || ev.sizes.get(folded).copied().unwrap_or(0) >= min
            });
            for mut it in f.items {
                let key = fold(&it.value);
                it.name = ev.record_names.get(&key).cloned().or_else(|| {
                    let ns = ev.names.get(&key)?;
                    let max = ns.values().max()?;
                    ns.iter().find(|(_, c)| *c == max).map(|(n, _)| n.clone())
                });
                it.corpus = several.then(|| ev.corpus.clone());
                items.push(it);
            }
        }
        items.sort_by(|a, b| {
            b.count
                .cmp(&a.count)
                .then_with(|| a.value.cmp(&b.value))
                .then_with(|| a.corpus.cmp(&b.corpus))
        });
        let distinct = items.len();
        if top > 0 {
            items.truncate(top);
        }
        Facet { distinct, items }
    }

    /// The entity facet, each value with its type: the most frequent one
    /// among the counted documents (ties: the first in type-name order).
    fn entity_facet(&self, top: usize) -> Facet {
        let mut f = self.entity.facet(top, false, |_| true);
        for it in f.items.iter_mut() {
            it.kind = self.entity_kinds.get(&fold(&it.value)).and_then(|kinds| {
                let max = kinds.values().max()?;
                kinds
                    .iter()
                    .find(|(_, c)| *c == max)
                    .map(|(k, _)| k.clone())
            });
        }
        f
    }

    /// All four facets; `top` cuts themes, entities and events (0 = all);
    /// months are all listed, chronologically.
    pub fn facets(&self, top: usize) -> Facets {
        Facets {
            theme: self.theme.facet(top, false, |_| true),
            entity: self.entity_facet(top),
            event: self.event_facet(top),
            month: self.month.facet(0, true, |_| true),
            ai: self.ai.clone(),
        }
    }

    /// One facet (`scout topics`); months chronological, others by count.
    pub fn facet(&self, field: TopicField, top: usize) -> Facet {
        match field {
            TopicField::Theme => self.theme.facet(top, false, |_| true),
            TopicField::Entity => self.entity_facet(top),
            TopicField::Event => self.event_facet(top),
            TopicField::Month => self.month.facet(top, true, |_| true),
        }
    }
}

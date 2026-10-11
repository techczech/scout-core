//! Human output for the topic-layer commands (`facets`, `topics`) and the
//! topic line of `index build`. Formatting only.

use scout_corpus::api::{EventsResults, FacetItem, FacetsResults, TopicsResults};
use scout_corpus::events::EventsReport;
use scout_corpus::topics::{Facet, TopicReport};

fn rows(items: &[FacetItem]) {
    let w = items
        .iter()
        .map(|i| i.count.to_string().len())
        .max()
        .unwrap_or(1);
    for i in items {
        // An event of a query over several corpora names its corpus.
        let corpus = i
            .corpus
            .as_deref()
            .map(|c| format!("  [{c}]"))
            .unwrap_or_default();
        match &i.name {
            Some(n) => println!("  {:>w$}  {}  {}{corpus}", i.count, i.value, n),
            None => println!("  {:>w$}  {}{corpus}", i.count, i.value),
        }
    }
}

fn section(title: &str, f: &Facet) {
    if f.items.is_empty() {
        return;
    }
    if f.distinct > f.items.len() {
        println!("{title} (top {} of {})", f.items.len(), f.distinct);
    } else {
        println!("{title} ({})", f.distinct);
    }
    rows(&f.items);
}

pub fn print_facets(r: &FacetsResults) {
    println!(
        "{} documents ({} tagged) for {:?} in {}",
        r.total_documents,
        r.tagged_documents,
        r.query,
        r.corpora.join(", ")
    );
    let f = &r.facets;
    section("themes", &f.theme);
    section("entities", &f.entity);
    section("events", &f.event);
    section("months", &f.month);
    if r.tagged_documents > 0 {
        println!("ai: {} true · {} false", f.ai.yes, f.ai.no);
    }
}

pub fn print_topics(r: &TopicsResults) {
    let field = serde_json::to_value(r.field)
        .ok()
        .and_then(|v| v.as_str().map(String::from))
        .unwrap_or_default();
    let month = r
        .month
        .as_deref()
        .map(|m| format!(" in {m}"))
        .unwrap_or_default();
    println!(
        "{} {field} values over {} tagged documents{month} ({})",
        r.distinct,
        r.tagged_documents,
        r.corpora.join(", ")
    );
    rows(&r.items);
}

pub fn print_events(r: &EventsResults) {
    let month = r
        .month
        .as_deref()
        .map(|m| format!(" with items in {m}"))
        .unwrap_or_default();
    println!("{} events{month} ({})", r.distinct, r.corpora.join(", "));
    let w = r
        .events
        .iter()
        .map(|e| e.items.to_string().len())
        .max()
        .unwrap_or(1);
    for e in &r.events {
        let span = match (&e.start, &e.end) {
            (Some(a), Some(b)) if a != b => format!("  {a} to {b}"),
            (Some(a), _) => format!("  {a}"),
            _ => String::new(),
        };
        let merged = e
            .merged_into
            .as_deref()
            .map(|m| format!("  (merged into {m})"))
            .unwrap_or_default();
        let in_month = e
            .month_items
            .map(|n| format!("  [{n} in month]"))
            .unwrap_or_default();
        println!(
            "  {:>w$}  {}  {}{span}{in_month}{merged}",
            e.items,
            e.id,
            e.name.as_deref().unwrap_or("")
        );
    }
}

/// The `index build` lines for a corpus's event records.
pub fn print_events_report(e: &EventsReport) {
    println!(
        "{:<10} events: {} records ({} merged), {} monthly events held, {} files skipped",
        "", e.records, e.merged, e.members, e.skipped
    );
    for w in &e.warnings {
        println!("{:<10} warning: {w}", "");
    }
    if e.warnings_total > e.warnings.len() {
        println!(
            "{:<10} … and {} more warnings",
            "",
            e.warnings_total - e.warnings.len()
        );
    }
}

/// The `index build` line for a corpus with a topic layer.
pub fn print_topic_report(t: &TopicReport) {
    let mut extra = Vec::new();
    if t.bad_lines > 0 {
        extra.push(format!("{} bad lines", t.bad_lines));
    }
    if t.corrections + t.corrections_unmatched + t.corrections_bad_lines > 0 {
        extra.push(format!(
            "{} corrections ({} unmatched, {} bad)",
            t.corrections, t.corrections_unmatched, t.corrections_bad_lines
        ));
    }
    if !t.reloaded {
        extra.push("unchanged".into());
    }
    let extra = if extra.is_empty() {
        String::new()
    } else {
        format!(" · {}", extra.join(" · "))
    };
    println!(
        "{:<10} topics: {} items, {} matched ({} by key, {} by path), {} unmatched → {} tagged documents{extra}",
        "",
        t.items,
        t.matched,
        t.matched_by_key,
        t.matched_by_path,
        t.unmatched,
        t.tagged_documents
    );
    if let Some(e) = &t.events {
        print_events_report(e);
    }
    if !t.unmatched_sample.is_empty() {
        println!(
            "{:<10} unmatched e.g. {}",
            "",
            t.unmatched_sample.join(", ")
        );
    }
}

//! Human output for the topic-layer commands (`facets`, `topics`) and the
//! topic line of `index build`. Formatting only.
//!
//! Every name, id, date and path printed here comes from an archive file
//! or an argument, and goes through [`crate::term::safe`].

use crate::term::safe;
use scout_corpus::api::{
    EventsResults, FacetItem, FacetsResults, ThemeMonthsResults, TopicsResults,
};
use scout_corpus::events::EventsReport;
use scout_corpus::topics::{Facet, TopicReport};

/// A list of names on one line, safe for a terminal.
fn list(items: &[String]) -> String {
    safe(&items.join(", ")).into_owned()
}

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
            .map(|c| format!("  [{}]", safe(c)))
            .unwrap_or_default();
        match &i.name {
            Some(n) => println!("  {:>w$}  {}  {}{corpus}", i.count, safe(&i.value), safe(n)),
            None => println!("  {:>w$}  {}{corpus}", i.count, safe(&i.value)),
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
        list(&r.corpora)
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
        .map(|m| format!(" in {}", safe(m)))
        .unwrap_or_default();
    println!(
        "{} {field} values over {} tagged documents{month} ({})",
        r.distinct,
        r.tagged_documents,
        list(&r.corpora)
    );
    rows(&r.items);
}

pub fn print_theme_months(r: &ThemeMonthsResults) {
    let span = match (&r.from, &r.to) {
        (Some(a), Some(b)) => format!(" from {} to {}", safe(a), safe(b)),
        _ => String::new(),
    };
    println!(
        "{} themes over {} months{span}, {} tagged documents ({})",
        r.themes.len(),
        r.months.len(),
        r.documents,
        list(&r.corpora)
    );
    let w = r
        .theme_totals
        .iter()
        .map(|n| n.to_string().len())
        .max()
        .unwrap_or(1);
    for (i, theme) in r.themes.iter().enumerate() {
        // The months with a count, so a row stays one readable line.
        let cells: Vec<String> = r
            .months
            .iter()
            .zip(&r.counts[i])
            .filter(|(_, n)| **n > 0)
            .map(|(m, n)| format!("{}:{n}", safe(m)))
            .collect();
        println!(
            "  {:>w$}  {}  {}",
            r.theme_totals[i],
            safe(theme),
            cells.join(" ")
        );
    }
}

pub fn print_events(r: &EventsResults) {
    let month = r
        .month
        .as_deref()
        .map(|m| format!(" with items in {}", safe(m)))
        .unwrap_or_default();
    println!("{} events{month} ({})", r.distinct, list(&r.corpora));
    let w = r
        .events
        .iter()
        .map(|e| e.items.to_string().len())
        .max()
        .unwrap_or(1);
    for e in &r.events {
        let span = match (&e.start, &e.end) {
            (Some(a), Some(b)) if a != b => format!("  {} to {}", safe(a), safe(b)),
            (Some(a), _) => format!("  {}", safe(a)),
            _ => String::new(),
        };
        let merged = e
            .merged_into
            .as_deref()
            .map(|m| format!("  (merged into {})", safe(m)))
            .unwrap_or_default();
        let removed = e
            .removed
            .as_deref()
            .map(|d| format!("  (removed {})", safe(d)))
            .unwrap_or_default();
        let in_month = e
            .month_items
            .map(|n| format!("  [{n} in month]"))
            .unwrap_or_default();
        println!(
            "  {:>w$}  {}  {}{span}{in_month}{merged}{removed}",
            e.items,
            safe(&e.id),
            safe(e.name.as_deref().unwrap_or(""))
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
        println!("{:<10} warning: {}", "", safe(w));
    }
    if e.warnings_total > e.warnings.len() {
        println!(
            "{:<10} … and {} more warnings",
            "",
            e.warnings_total - e.warnings.len()
        );
    }
}

/// The `index build` line for a corpus with a topic layer, and a second
/// line when its corrections file was not read.
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
    if t.corrections_undone > 0 {
        extra.push(format!("{} corrections undone", t.corrections_undone));
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
    if let Some(skipped) = &t.corrections_skipped {
        println!(
            "{:<10} WARNING: corrections NOT applied: {}",
            "",
            safe(&skipped.reason)
        );
    }
    if let Some(e) = &t.events {
        print_events_report(e);
    }
    if !t.unmatched_sample.is_empty() {
        println!("{:<10} unmatched e.g. {}", "", list(&t.unmatched_sample));
    }
}

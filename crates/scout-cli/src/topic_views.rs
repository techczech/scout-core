//! Human output for the topic-layer commands (`facets`, `topics`) and the
//! topic line of `index build`. Formatting only.

use scout_corpus::api::{FacetItem, FacetsResults, TopicsResults};
use scout_corpus::topics::{Facet, TopicReport};

fn rows(items: &[FacetItem]) {
    let w = items
        .iter()
        .map(|i| i.count.to_string().len())
        .max()
        .unwrap_or(1);
    for i in items {
        match &i.name {
            Some(n) => println!("  {:>w$}  {}  {}", i.count, i.value, n),
            None => println!("  {:>w$}  {}", i.count, i.value),
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
    if !t.unmatched_sample.is_empty() {
        println!(
            "{:<10} unmatched e.g. {}",
            "",
            t.unmatched_sample.join(", ")
        );
    }
}

//! Human output for the analytics views: compact and aligned (J9 frame).
//! Formatting only; every number comes from `scout-corpus`.

use anyhow::Result;
use scout_corpus::concord::{Distribution, KwicResults, Profile};
use scout_corpus::ngrams::NgramResults;
use scout_corpus::verify::VerifyResult;
use std::process::ExitCode;

/// Print `value` as pretty JSON, or run the human printer.
pub fn emit<T: serde::Serialize>(json: bool, value: &T, human: impl FnOnce()) -> Result<()> {
    if json {
        println!("{}", serde_json::to_string_pretty(value)?);
    } else {
        human();
    }
    Ok(())
}

/// Exit 0 with results, 1 without.
pub fn code(any: bool) -> ExitCode {
    if any {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn flat(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn keep_tail(s: &str, n: usize) -> String {
    let c: Vec<char> = s.chars().collect();
    if c.len() <= n {
        s.to_string()
    } else {
        format!("…{}", c[c.len() - (n - 1)..].iter().collect::<String>())
    }
}

fn keep_head(s: &str, n: usize) -> String {
    let c: Vec<char> = s.chars().collect();
    if c.len() <= n {
        s.to_string()
    } else {
        format!("{}…", c[..n - 1].iter().collect::<String>())
    }
}

fn pad_left(s: &str, n: usize) -> String {
    let len = s.chars().count();
    format!("{}{}", " ".repeat(n.saturating_sub(len)), s)
}

fn pad_right(s: &str, n: usize) -> String {
    let len = s.chars().count();
    format!("{}{}", s, " ".repeat(n.saturating_sub(len)))
}

pub fn print_kwic(r: &KwicResults) {
    println!(
        "{} lines in {} pieces for {:?} in {} · sort {} · showing {}",
        r.total,
        r.pieces,
        r.term,
        r.corpora.join(", "),
        r.sort,
        r.returned
    );
    const SIDE: usize = 48;
    for l in &r.lines {
        let left = pad_left(&keep_tail(&flat(&l.left), SIDE), SIDE);
        let right = pad_right(&keep_head(&flat(&l.right), SIDE), SIDE);
        let date = l.date.as_deref().unwrap_or("—");
        println!(
            "{left}  [{}]  {right}  {:<10} {}:{}",
            flat(&l.node),
            date,
            l.rel_path,
            l.line
        );
    }
}

pub fn print_dist(d: &Distribution) {
    println!(
        "{} hits in {} pieces for {:?} by {} in {}",
        d.total,
        d.pieces,
        d.term,
        serde_json::to_value(d.by)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_default(),
        d.corpora.join(", ")
    );
    let w = d
        .buckets
        .iter()
        .map(|b| b.key.chars().count())
        .max()
        .unwrap_or(4)
        .max(4);
    println!(
        "{}  {:>6}  {:>6}  {:>10}  {:>10}",
        pad_right("key", w),
        "hits",
        "pieces",
        "per-mill",
        "tokens"
    );
    for b in &d.buckets {
        println!(
            "{}  {:>6}  {:>6}  {:>10.1}  {:>10}",
            pad_right(&b.key, w),
            b.hits,
            b.pieces,
            b.per_million,
            b.tokens
        );
    }
}

pub fn print_ngrams(r: &NgramResults) {
    let period = match (r.since, r.until) {
        (None, None) => String::new(),
        (s, u) => format!(
            " · {}–{}",
            s.map(|y| y.to_string()).unwrap_or_default(),
            u.map(|y| y.to_string()).unwrap_or_default()
        ),
    };
    println!(
        "top {} {}–{}-grams in {} ({} docs, {} tokens{}){}",
        r.grams.len(),
        r.n_min,
        r.n_max,
        r.corpora.join(", "),
        r.documents,
        r.total_tokens,
        period,
        if r.strict_stopwords {
            " · strict stopwords"
        } else {
            ""
        }
    );
    for g in &r.grams {
        let years = g
            .by_year
            .as_ref()
            .map(|m| {
                format!(
                    "  [{}]",
                    m.iter()
                        .map(|(y, c)| format!("{y}:{c}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                )
            })
            .unwrap_or_default();
        println!(
            "{:>7}  {:>5} pieces  {}{}",
            g.count, g.pieces, g.gram, years
        );
    }
}

pub fn print_profile(p: &Profile) {
    println!("{:?} in {}", p.word, p.corpora.join(", "));
    println!(
        "  frequency   {}  ({:.1} per million of {} tokens)",
        p.frequency, p.per_million, p.total_tokens
    );
    println!("  pieces      {}", p.pieces);
    match &p.first_used {
        Some(f) => println!(
            "  first used  {}  {}  ({}:{})",
            f.date_display
                .as_deref()
                .or(f.date.as_deref())
                .unwrap_or("—"),
            f.title,
            f.rel_path,
            f.line
        ),
        None => println!("  first used  —"),
    }
    let years: Vec<String> = p
        .by_year
        .iter()
        .filter(|b| b.hits > 0)
        .map(|b| format!("{}:{}", b.key, b.hits))
        .collect();
    println!("  by year     {}", years.join(" "));
    if !p.top_documents.is_empty() {
        println!("  top documents (per million)");
        for d in &p.top_documents {
            println!(
                "    {:>9.1}  {:>3} hits  {:<10}  {}",
                d.per_million,
                d.hits,
                d.date.as_deref().unwrap_or("—"),
                d.title
            );
        }
    }
    println!("  collocates  (T4)");
    println!("  n-grams     (T4)");
}

pub fn print_verify(v: &VerifyResult) {
    if !v.found {
        println!("not found in {}", v.corpora.join(", "));
        return;
    }
    println!("found {} match(es)", v.matches.len());
    for m in &v.matches {
        println!(
            "  {}:{}  {}  {}",
            m.rel_path,
            m.line,
            m.date.as_deref().unwrap_or("—"),
            m.title
        );
        println!("    > {}", keep_head(&flat(&m.original), 200));
    }
}

//! Human output for the analytics views: compact and aligned (J9 frame).
//! Formatting only; every number comes from `scout-corpus`.

use anyhow::Result;
use scout_corpus::colloc::{CollocResults, CompareResults};
use scout_corpus::concord::{Distribution, KwicResults, Profile};
use scout_corpus::keyness::{KeyWord, KeynessResults};
use scout_corpus::ngrams::NgramResults;
use scout_corpus::rowdist::RowDist;
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
    let near = r
        .near
        .as_ref()
        .map(|n| format!(" near {:?} (±{})", n.word, n.window))
        .unwrap_or_default();
    println!(
        "{} lines in {} pieces for {:?}{near} in {} · sort {} · showing {}",
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
        let date = l
            .date
            .as_deref()
            .map(|d| d.get(..10).unwrap_or(d))
            .unwrap_or("—");
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
        if let Some(d) = &g.dist {
            println!("{:>16}{}", "", dist_line(d));
        }
    }
}

/// One line per row distribution: `2016:3 2017:5 …`, docs as titles.
fn dist_line(d: &RowDist) -> String {
    let mut parts: Vec<String> = d
        .buckets
        .iter()
        .map(|b| match &b.title {
            Some(t) => format!("{}:{}", keep_head(t, 40), b.count),
            None => format!("{}:{}", b.key, b.count),
        })
        .collect();
    if let Some(o) = &d.other {
        parts.push(format!("+{} more docs:{}", o.documents, o.count));
    }
    parts.join(if d.buckets.iter().any(|b| b.title.is_some()) {
        " · "
    } else {
        " "
    })
}

fn opt3(x: Option<f64>) -> String {
    x.map(|v| format!("{v:.2}")).unwrap_or_else(|| "—".into())
}

pub fn print_collocates(r: &CollocResults) {
    println!(
        "{} collocates of {:?} in {} (f={} of {} tokens · ±{} · {} · min {}{}) · showing {}",
        r.candidates,
        r.node,
        r.corpora.join(", "),
        r.node_freq,
        r.total_tokens,
        r.window,
        r.score,
        r.min_freq,
        if r.keep_stopwords {
            " · stopwords kept"
        } else {
            ""
        },
        r.rows.len()
    );
    let w = r
        .rows
        .iter()
        .map(|x| x.collocate.chars().count())
        .max()
        .unwrap_or(9)
        .max(9);
    println!(
        "{}  {:>6}  {:>7}  {:>7}  {:>6}",
        pad_right("collocate", w),
        "count",
        "f(c)",
        "logDice",
        "MI"
    );
    for x in &r.rows {
        println!(
            "{}  {:>6}  {:>7}  {:>7.2}  {:>6.2}",
            pad_right(&x.collocate, w),
            x.count,
            x.collocate_freq,
            x.logdice,
            x.mi
        );
        if let Some(d) = &x.dist {
            println!("{}  {}", " ".repeat(w), dist_line(d));
        }
    }
}

pub fn print_compare(r: &CompareResults) {
    println!(
        "collocates of {:?} · ±{} · {} · min {} · showing {} of {}",
        r.node,
        r.window,
        r.score,
        r.min_freq,
        r.rows.len(),
        r.candidates
    );
    println!(
        "  A = {} in {} (f={}, {} tokens)",
        r.a.spec,
        r.a.corpora.join(", "),
        r.a.node_freq,
        r.a.total_tokens
    );
    println!(
        "  B = {} in {} (f={}, {} tokens)",
        r.b.spec,
        r.b.corpora.join(", "),
        r.b.node_freq,
        r.b.total_tokens
    );
    let w = r
        .rows
        .iter()
        .map(|x| x.collocate.chars().count())
        .max()
        .unwrap_or(9)
        .max(9);
    println!(
        "{}  {:>6} {:>7}  {:>6} {:>7}  {:>7}  {:>6}",
        pad_right("collocate", w),
        "A n",
        "A score",
        "B n",
        "B score",
        "ratio",
        "diff"
    );
    for x in &r.rows {
        let (sa, sb) = match r.score {
            scout_corpus::Score::LogDice => (x.a.logdice, x.b.logdice),
            scout_corpus::Score::Mi => (x.a.mi, x.b.mi),
        };
        println!(
            "{}  {:>6} {:>7}  {:>6} {:>7}  {:>7}  {:>6}",
            pad_right(&x.collocate, w),
            x.a.count,
            opt3(sa),
            x.b.count,
            opt3(sb),
            opt3(x.ratio),
            opt3(x.score_diff)
        );
    }
}

fn print_keys(title: &str, rows: &[KeyWord]) {
    println!("{title}");
    if rows.is_empty() {
        println!("  (none)");
        return;
    }
    let w = rows
        .iter()
        .map(|x| x.word.chars().count())
        .max()
        .unwrap_or(4)
        .max(4);
    println!(
        "  {}  {:>6} {:>6}  {:>9} {:>9}  {:>8}  {:>8}",
        pad_right("word", w),
        "A",
        "B",
        "A /mill",
        "B /mill",
        "G2",
        "%DIFF"
    );
    for x in rows {
        println!(
            "  {}  {:>6} {:>6}  {:>9.1} {:>9.1}  {:>8.1}  {:>8}",
            pad_right(&x.word, w),
            x.a,
            x.b,
            x.a_per_million,
            x.b_per_million,
            x.g2,
            x.pct_diff
                .map(|v| format!("{v:.0}"))
                .unwrap_or_else(|| "∞".into())
        );
    }
}

pub fn print_keyness(r: &KeynessResults) {
    println!(
        "keyness · A = {} ({} docs, {} tokens in {}) · B = {} ({} docs, {} tokens in {})",
        r.a.spec,
        r.a.documents,
        r.a.tokens,
        r.a.corpora.join(", "),
        r.b.spec,
        r.b.documents,
        r.b.tokens,
        r.b.corpora.join(", ")
    );
    println!(
        "  log-likelihood G2 ≥ {} (p < 0.05) · min {}{}",
        r.g2_critical,
        r.min_freq,
        if r.overlap_documents > 0 {
            format!(
                " · WARNING: {} documents in both slices",
                r.overlap_documents
            )
        } else {
            String::new()
        }
    );
    print_keys(&format!("key in A ({})", r.a.spec), &r.a_keys);
    print_keys(&format!("key in B ({})", r.b.spec), &r.b_keys);
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
        println!(
            "  top documents (per million, at least {} hits)",
            p.top_documents_min_hits
        );
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
    let coll: Vec<String> = p
        .collocates
        .items
        .iter()
        .map(|c| format!("{} {:.1} ({})", c.collocate, c.logdice, c.count))
        .collect();
    println!(
        "  collocates  (logDice, ±{}, min {}){}",
        p.collocates.window,
        p.collocates.min_freq,
        if coll.is_empty() { " —" } else { "" }
    );
    for chunk in coll.chunks(5) {
        println!("    {}", chunk.join(" · "));
    }
    println!(
        "  n-grams     ({}–{} containing the word){}",
        p.ngrams.n_min,
        p.ngrams.n_max,
        if p.ngrams.items.is_empty() {
            " —"
        } else {
            ""
        }
    );
    for g in &p.ngrams.items {
        println!("    {:>5}  {}", g.count, g.gram);
    }
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

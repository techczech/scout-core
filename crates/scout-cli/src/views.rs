//! Human output for every command: compact and aligned (J9 frame).
//! Formatting only; every number comes from `scout-corpus`.
//!
//! Every string printed here that is not a number or a word of this file
//! goes through [`crate::term::safe`] (a text of several lines through
//! `safe_lines`), so nothing from an archive file, the registry or an
//! argument reaches the terminal as a control sequence. A `{:?}` value is
//! left to Rust's own escaping of control characters.

use crate::term::{safe, safe_lines};
use scout_corpus::api::{
    CorporaList, CorpusShow, IndexBuildReport, IndexStatusReport, InitDefaults, Removed,
};
use scout_corpus::clean::CleanReport;
use scout_corpus::colloc::{CollocResults, CompareResults};
use scout_corpus::concord::{Distribution, KwicResults, Profile};
use scout_corpus::keyness::{KeyWord, KeynessResults};
use scout_corpus::ngrams::NgramResults;
use scout_corpus::rowdist::RowDist;
use scout_corpus::similar::SimilarResults;
use scout_corpus::verify::VerifyResult;
use scout_corpus::CorpusConfig;
use std::process::ExitCode;

/// Exit 0 with results, 1 without.
pub fn code(any: bool) -> ExitCode {
    if any {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

/// Text of a file on one line, safe for a terminal: runs of white space
/// (line breaks among them) become one space, and what is left goes
/// through [`safe`].
fn flat(s: &str) -> String {
    safe(&s.split_whitespace().collect::<Vec<_>>().join(" ")).into_owned()
}

/// A list of names on one line, safe for a terminal.
fn list(items: &[String], sep: &str) -> String {
    safe(&items.join(sep)).into_owned()
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
        list(&r.corpora, ", "),
        safe(&r.sort),
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
            safe(date),
            safe(&l.rel_path),
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
        list(&d.corpora, ", ")
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
            pad_right(&safe(&b.key), w),
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
        list(&r.corpora, ", "),
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
            g.count,
            g.pieces,
            safe(&g.gram),
            years
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
            Some(t) => format!("{}:{}", safe(&keep_head(t, 40)), b.count),
            None => format!("{}:{}", safe(&b.key), b.count),
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
        list(&r.corpora, ", "),
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
            pad_right(&safe(&x.collocate), w),
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
        safe(&r.a.spec),
        list(&r.a.corpora, ", "),
        r.a.node_freq,
        r.a.total_tokens
    );
    println!(
        "  B = {} in {} (f={}, {} tokens)",
        safe(&r.b.spec),
        list(&r.b.corpora, ", "),
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
            pad_right(&safe(&x.collocate), w),
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
    println!("{}", safe(title));
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
            pad_right(&safe(&x.word), w),
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
        safe(&r.a.spec),
        r.a.documents,
        r.a.tokens,
        list(&r.a.corpora, ", "),
        safe(&r.b.spec),
        r.b.documents,
        r.b.tokens,
        list(&r.b.corpora, ", ")
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
    println!("{:?} in {}", p.word, list(&p.corpora, ", "));
    println!(
        "  frequency   {}  ({:.1} per million of {} tokens)",
        p.frequency, p.per_million, p.total_tokens
    );
    println!("  pieces      {}", p.pieces);
    match &p.first_used {
        Some(f) => println!(
            "  first used  {}  {}  ({}:{})",
            safe(
                f.date_display
                    .as_deref()
                    .or(f.date.as_deref())
                    .unwrap_or("—")
            ),
            safe(&f.title),
            safe(&f.rel_path),
            f.line
        ),
        None => println!("  first used  —"),
    }
    let years: Vec<String> = p
        .by_year
        .iter()
        .filter(|b| b.hits > 0)
        .map(|b| format!("{}:{}", safe(&b.key), b.hits))
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
                safe(d.date.as_deref().unwrap_or("—")),
                safe(&d.title)
            );
        }
    }
    let coll: Vec<String> = p
        .collocates
        .items
        .iter()
        .map(|c| format!("{} {:.1} ({})", safe(&c.collocate), c.logdice, c.count))
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
        println!("    {:>5}  {}", g.count, safe(&g.gram));
    }
}

pub fn print_verify(v: &VerifyResult) {
    if !v.found {
        println!("not found in {}", list(&v.corpora, ", "));
        return;
    }
    println!("found {} match(es)", v.matches.len());
    for m in &v.matches {
        println!(
            "  {}:{}  {}  {}",
            safe(&m.rel_path),
            m.line,
            safe(m.date.as_deref().unwrap_or("—")),
            safe(&m.title)
        );
        println!("    > {}", keep_head(&flat(&m.original), 200));
    }
}

pub fn print_hits(res: &scout_corpus::SearchResults) {
    println!(
        "{} documents, {} passages for {:?} in {}{}",
        res.total_documents,
        res.total_passages,
        res.query,
        list(&res.corpora, ", "),
        res.mode
            .as_deref()
            .map(|m| format!(" ({})", safe(m)))
            .unwrap_or_default()
    );
    if !res.plan.ignored.is_empty() {
        println!("(not applied here: {})", list(&res.plan.ignored, " "));
    }
    for (i, d) in res.results.iter().enumerate() {
        let hit = &d.hits[0];
        println!();
        println!("{:>3}. [{}] {}", i + 1, safe(&d.corpus), safe(&d.title));
        let mut date = d
            .date
            .as_deref()
            .map(|s| s.get(..10).unwrap_or(s).to_string())
            .unwrap_or_else(|| "—".into());
        if d.date_source.as_deref() == Some("saved") {
            date.push_str(" (saved)");
        }
        let genre = d.genre.clone().unwrap_or_else(|| "—".into());
        let sim = hit
            .semantic_score
            .map(|s| format!("  cos {s:.3}"))
            .unwrap_or_default();
        println!(
            "     {:<10}  {:<10}  {}:{}  ({} passages){sim}",
            safe(&date),
            safe(&genre),
            safe(&d.rel_path),
            hit.line,
            d.passage_count
        );
        let q = flat(&hit.quote);
        let q = if q.chars().count() > 220 {
            format!("{}…", q.chars().take(220).collect::<String>())
        } else {
            q
        };
        println!("     > {q}");
    }
}

pub fn print_similar(res: &SimilarResults) {
    println!(
        "{} of {} passages like {} seed{} in {}",
        res.results.len(),
        res.total_passages,
        res.seeds.len(),
        if res.seeds.len() == 1 { "" } else { "s" },
        list(&res.corpora, ", ")
    );
    for (i, s) in res.seeds.iter().enumerate() {
        println!(
            "  seed {}: [{}] {}  {}",
            i + 1,
            safe(&s.corpus),
            safe(&s.title),
            safe(&s.passage_id)
        );
    }
    for (i, r) in res.results.iter().enumerate() {
        let seed = res
            .seeds
            .iter()
            .position(|s| s.passage_id == r.closest_seed)
            .map(|n| n + 1)
            .unwrap_or(0);
        println!();
        println!(
            "{:>3}. [{}] {}  ({:.3})",
            i + 1,
            safe(&r.corpus),
            safe(&r.title),
            r.score
        );
        let date = r
            .date
            .as_deref()
            .map(|s| s.get(..10).unwrap_or(s).to_string())
            .unwrap_or_else(|| "—".into());
        println!(
            "     {:<10}  {}:{}  like seed {seed} · shared: {}",
            safe(&date),
            safe(&r.rel_path),
            r.line_start,
            list(&r.shared_terms, ", ")
        );
        let q = flat(&r.quote);
        let q = if q.chars().count() > 220 {
            format!("{}…", q.chars().take(220).collect::<String>())
        } else {
            q
        };
        println!("     > {q}");
    }
}

pub fn print_init_defaults(r: &InitDefaults) {
    if r.wrote {
        println!("wrote {}", safe(&r.path));
    } else {
        println!(
            "{} exists; left unchanged (use --force to overwrite)",
            safe(&r.path)
        );
    }
    for c in &r.corpora {
        println!(
            "  {:<11} {:<24} {}",
            safe(&c.id),
            safe(&c.kind),
            safe(&c.path)
        );
    }
}

pub fn print_added(c: &CorpusConfig) {
    println!(
        "added {} ({}) {}",
        safe(&c.id),
        c.kind.as_str(),
        safe(&c.path)
    );
    if !c.include.is_empty() {
        println!("  include {}", list(&c.include, ", "));
    }
    if !c.exclude.is_empty() {
        println!("  exclude {}", list(&c.exclude, ", "));
    }
    println!("next: scout index build {}", safe(&c.id));
}

pub fn print_removed(r: &Removed) {
    println!("removed {} ({} left untouched)", safe(&r.id), safe(&r.path));
    if r.index_kept {
        println!("  index kept");
    } else if r.deleted.is_empty() {
        println!("  no index to delete");
    } else {
        for p in &r.deleted {
            println!("  deleted {}", safe(p));
        }
    }
}

pub fn print_corpora(r: &CorporaList) {
    println!("{:<11} {:<24} {:<8} path", "id", "kind", "index");
    for c in &r.corpora {
        println!(
            "{:<11} {:<24} {:<8} {}{}",
            safe(&c.id),
            safe(&c.kind),
            if c.index_built { "built" } else { "missing" },
            safe(&c.path),
            if c.folder_exists {
                ""
            } else {
                "  (folder missing)"
            }
        );
    }
}

pub fn print_corpus_show(r: &CorpusShow) {
    print!("{}", safe_lines(&r.toml));
    println!("\n# root:  {}", safe(&r.entry.root));
    println!(
        "# index: {} ({})",
        safe(&r.entry.index_path),
        if r.entry.index_built {
            "built"
        } else {
            "missing"
        }
    );
}

pub fn print_build(r: &IndexBuildReport) {
    for r in &r.reports {
        println!(
            "{:<10} {} docs · {} passages · {} tokens  ({} {}: {} indexed, {} unchanged, {} removed, {} not documents) {:.1}s",
            safe(&r.corpus),
            r.docs,
            r.passages,
            r.tokens,
            if r.full { "full" } else { "incremental" },
            r.files_seen,
            r.indexed,
            r.unchanged,
            r.removed,
            r.not_documents,
            r.elapsed_ms as f64 / 1000.0
        );
        if let Some(v) = &r.vectors {
            println!(
                "{:<10} vectors {} ({}-d): {} passages ({} {}: {} embedded, {} reused, {} removed) {:.1} MB {:.1}s",
                "",
                safe(&v.model),
                v.dim,
                v.passages,
                if v.full { "full" } else { "incremental" },
                safe(&v.vectors_path),
                v.embedded,
                v.reused,
                v.removed,
                v.bytes as f64 / 1_048_576.0,
                v.elapsed_ms as f64 / 1000.0
            );
        }
        if let Some(t) = &r.topics {
            crate::topic_views::print_topic_report(t);
        }
        if let Some(e) = &r.events {
            crate::topic_views::print_events_report(e);
        }
    }
}

pub fn print_status(r: &IndexStatusReport) {
    println!(
        "{:<11} {:>6} {:>9} {:>10}  {:<20}  stale",
        "corpus", "docs", "passages", "tokens", "built-at"
    );
    for s in &r.corpora {
        if !s.exists {
            let note = format!("no index; run `scout index build {}`", s.corpus);
            println!(
                "{:<11} {:>6} {:>9} {:>10}  {:<20}  {}",
                safe(&s.corpus),
                "—",
                "—",
                "—",
                "—",
                safe(&note)
            );
            continue;
        }
        let stale = match &s.stale {
            Some(st) if st.added + st.changed + st.removed == 0 && s.config_current => {
                "none".to_string()
            }
            Some(st) => format!(
                "{} new · {} changed · {} removed{}",
                st.added,
                st.changed,
                st.removed,
                if s.config_current {
                    ""
                } else {
                    " · config changed"
                }
            ),
            None => "—".into(),
        };
        println!(
            "{:<11} {:>6} {:>9} {:>10}  {:<20}  {}",
            safe(&s.corpus),
            s.docs,
            s.passages,
            s.tokens,
            safe(s.built_at.as_deref().unwrap_or_default()),
            stale
        );
    }
}

fn toks(t: &[String]) -> String {
    if t.is_empty() {
        "(nothing counted)".into()
    } else {
        t.join(" · ")
    }
}

pub fn print_clean_report(r: &CleanReport) {
    println!(
        "Cleaning report · {} · {} documents · {} passages · {} lines",
        safe(&r.corpus),
        r.documents,
        r.passages,
        r.lines
    );
    println!(
        "{:<28} {:>10} {:>16} {:>14}",
        "rule", "passages", "tokens changed", "lines"
    );
    for rule in &r.rules {
        println!(
            "{:<28} {:>10} {:>16} {:>14}",
            safe(&rule.label),
            rule.passages,
            rule.token_passages,
            rule.token_lines
        );
        if let Some(ps) = &rule.patterns {
            if ps.is_empty() {
                println!(
                    "    (no boilerplate rules in the registry; preview one with --rule <regex>)"
                );
            }
            for p in ps {
                println!(
                    "    rule {} ({}) {:?}: {} lines dropped in {} passages",
                    p.n,
                    safe(&p.origin),
                    p.regex,
                    p.lines,
                    p.passages
                );
            }
        }
    }
    for rule in &r.rules {
        if rule.samples.is_empty() {
            continue;
        }
        println!();
        println!("{} · samples", safe(&rule.label));
        for s in &rule.samples {
            let by = s
                .pattern
                .map(|n| format!(" · rule {n}"))
                .unwrap_or_default();
            println!("  {}:{}  ×{}{by}", safe(&s.rel_path), s.line, s.occurrences);
            println!("    original: {}", safe(&keep_head(&s.original, 160)));
            println!("    before:   {}", safe(&keep_head(&toks(&s.before), 160)));
            println!("    after:    {}", safe(&keep_head(&toks(&s.after), 160)));
            let mut change: Vec<String> = s.removed.iter().map(|t| format!("-{t}")).collect();
            change.extend(s.added.iter().map(|t| format!("+{t}")));
            println!("    change:   {}", safe(&keep_head(&change.join(" "), 160)));
        }
    }
}

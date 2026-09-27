//! Tweet files (`document_unit = "tweet"`): one document per tweet.
//!
//! The writing repo's Twitter-archive import holds many tweets per file
//! (`stream/2025-07.md` has a month; `threads/…md` one thread), each as:
//!
//! ```text
//! ## 2025-07-01T04:17:24Z — tweet 1939901126326764018 · kind: reply
//!
//! <!-- tweet id="1939901126326764018" -->
//! ~~~
//! The tweet text, verbatim.
//! ~~~
//!
//! Reply to: @someone
//! ```
//!
//! A tweet is the unit that has a date and a public URL, so it is the
//! document: citations, dates, filters and per-year counts are per tweet.
//! Its passages are the paragraphs inside the `~~~` fence; the heading, the
//! id comment and the trailer lines (`Reply to:`, `Media:`) are metadata and
//! are not counted as text.

use crate::adapter::{DocRecord, PassageRecord};
use crate::markdown::{self, ParsedFile};
use crate::registry::CorpusConfig;
use regex::Regex;
use std::sync::OnceLock;

fn heading_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^## (\S+) — tweet (\d+)(?: · kind: (\S+))?\s*$").expect("tweet heading")
    })
}

fn handle_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"@([A-Za-z0-9_]{1,30})").expect("handle"))
}

/// The public URL of a tweet.
pub fn tweet_url(handle: Option<&str>, id: &str) -> String {
    format!("https://x.com/{}/status/{id}", handle.unwrap_or("i"))
}

struct Pending {
    id: String,
    date: String,
    kind: Option<String>,
    /// (line number, text) inside the fence.
    lines: Vec<(usize, String)>,
    fence: Fence,
}

#[derive(PartialEq, Eq)]
enum Fence {
    Before,
    Open,
    Closed,
}

fn title_from(text: &[(usize, String)]) -> String {
    let joined = text
        .iter()
        .map(|(_, l)| l.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let words = joined.split_whitespace().collect::<Vec<_>>().join(" ");
    if words.chars().count() > 80 {
        format!("{}…", words.chars().take(80).collect::<String>().trim_end())
    } else {
        words
    }
}

/// Split a parsed tweet file into one document per tweet.
pub fn split(cfg: &CorpusConfig, rel_path: &str, parsed: &ParsedFile) -> Vec<DocRecord> {
    let file_meta = markdown::doc_meta(cfg, parsed);
    let fm = parsed.frontmatter.clone().unwrap_or_default();
    let get = |k: &str| fm.get(k).and_then(|v| v.as_scalar());
    let handle = get("venue")
        .or_else(|| get("handle"))
        .and_then(|v| handle_re().captures(&v).map(|c| c[1].to_string()));
    let thread = get("tweet_collection").as_deref() == Some("thread");

    let mut tweets: Vec<Pending> = Vec::new();
    for (i, line) in parsed.body.iter().enumerate() {
        let n = parsed.body_start_line + i;
        if let Some(cur) = tweets.last_mut() {
            if cur.fence == Fence::Open {
                if line.trim_end() == "~~~" {
                    cur.fence = Fence::Closed;
                } else {
                    cur.lines.push((n, line.clone()));
                }
                continue;
            }
        }
        if let Some(c) = heading_re().captures(line) {
            tweets.push(Pending {
                id: c[2].to_string(),
                date: c[1].to_string(),
                kind: c.get(3).map(|m| m.as_str().to_string()),
                lines: Vec::new(),
                fence: Fence::Before,
            });
            continue;
        }
        if let Some(cur) = tweets.last_mut() {
            if cur.fence == Fence::Before && line.trim_end() == "~~~" {
                cur.fence = Fence::Open;
            }
        }
    }

    tweets
        .into_iter()
        .map(|t| {
            // The fence content, split into paragraphs like any body.
            let passages = match t.lines.first() {
                None => vec![],
                Some((first, _)) => {
                    let body = ParsedFile {
                        frontmatter: None,
                        body_start_line: *first,
                        body: t.lines.iter().map(|(_, l)| l.clone()).collect(),
                    };
                    markdown::split_passages(&body)
                        .into_iter()
                        .map(|p| PassageRecord {
                            line_start: p.line_start,
                            line_end: p.line_end,
                            original: p.original,
                            ..PassageRecord::default()
                        })
                        .collect()
                }
            };
            let mut meta = file_meta.clone();
            meta.title = Some(if thread {
                file_meta
                    .title
                    .clone()
                    .unwrap_or_else(|| title_from(&t.lines))
            } else {
                title_from(&t.lines)
            });
            meta.date = Some(t.date.clone());
            meta.date_source = Some("published".into());
            meta.genre = Some(if thread {
                "thread".to_string()
            } else {
                t.kind.clone().unwrap_or_else(|| "tweet".into())
            });
            meta.summary = None;
            meta.kind = Some("tweet".into());
            meta.source = Some("x".into());
            meta.public_url = Some(tweet_url(handle.as_deref(), &t.id));
            meta.handle = handle.clone();
            DocRecord {
                key: format!("{rel_path}#{}", t.id),
                meta,
                passages,
                index_title: false,
            }
        })
        .collect()
}

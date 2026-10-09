//! Highlight Scout archive (`readings/works/*.md`, highlights ADR-0003 v2
//! Markdown): one work file = one document, one highlight = one passage.
//! Read-only; the format is the one `scout-archive` writes:
//!
//! ```text
//! ---
//! title: …            (may be a double-quoted scalar spanning lines)
//! author: …
//! type: article | book | tweet | …
//! source_system: readwise | x | zotero
//! source_id: "…"
//! url: https://…
//! source_data: {json}
//! ---
//!
//! > highlight text, one `> ` line per source line
//!
//! highlighted_at: 2021-04-03 | tags: a, b | color: yellow
//!
//! an optional note (the reader's, not the author's: not indexed)
//!
//! ---
//! ```
//!
//! The document date is the work's PUBLICATION date: Zotero's `date`, or a
//! tweet's timestamp (decoded from its id). When neither exists the date is
//! the earliest `highlighted_at` of the work, and `date_source` is `saved`.

use crate::adapter::{DocRecord, PassageRecord};
use crate::markdown::DocMeta;
use regex::Regex;
use std::collections::BTreeMap;
use std::sync::OnceLock;

/// Frontmatter scalars; a double-quoted value may continue over lines.
fn frontmatter(lines: &[&str]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut i = 0;
    while i < lines.len() {
        let l = lines[i];
        i += 1;
        let Some((k, v)) = l.split_once(':') else {
            continue;
        };
        let key = k.trim();
        if key.is_empty() || key.contains(' ') || l.starts_with(' ') {
            continue;
        }
        let mut v = v.trim().to_string();
        if v.starts_with('"') {
            // Continue until a line that closes the quote.
            while !closes_quote(&v) && i < lines.len() {
                v.push(' ');
                v.push_str(lines[i].trim());
                i += 1;
            }
            v = unquote(&v);
        }
        out.insert(key.to_string(), v);
    }
    out
}

/// Does `v` (starting with `"`) end with an unescaped closing quote?
fn closes_quote(v: &str) -> bool {
    if v.len() < 2 || !v.ends_with('"') {
        return false;
    }
    let inner = &v[1..v.len() - 1];
    let backslashes = inner.len() - inner.trim_end_matches('\\').len();
    backslashes.is_multiple_of(2)
}

fn unquote(v: &str) -> String {
    let t = v.trim();
    let t = t.strip_prefix('"').unwrap_or(t);
    let t = t.strip_suffix('"').unwrap_or(t);
    t.replace("\\\"", "\"")
}

fn status_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?:twitter|x)\.com/[^/]+/status(?:es)?/(\d+)").expect("status regex")
    })
}

/// A tweet's creation time from its snowflake id (ids from November 2010
/// on); older sequential ids give `None`.
pub fn snowflake_time(id: &str) -> Option<String> {
    let id: u64 = id.trim().parse().ok()?;
    let since_epoch = id >> 22;
    if since_epoch < 86_400_000 {
        return None;
    }
    let ms = since_epoch as i64 + 1_288_834_974_657;
    let t = chrono::DateTime::from_timestamp_millis(ms)?;
    Some(t.format("%Y-%m-%dT%H:%M:%SZ").to_string())
}

/// Zotero's `date` field (`2003-03-00 3/2003`): the first token, cut at the
/// first `00` part (`2003-03`), or `None` when the year is unknown.
pub fn zotero_date(raw: &str) -> Option<String> {
    let first = raw.split_whitespace().next()?;
    let parts: Vec<&str> = first.split('-').collect();
    let y = parts.first()?;
    if y.len() != 4 || !y.chars().all(|c| c.is_ascii_digit()) || *y == "0000" {
        return None;
    }
    let mut out = y.to_string();
    for p in parts.iter().skip(1).take(2) {
        if p.chars().all(|c| c == '0') || !p.chars().all(|c| c.is_ascii_digit()) {
            break;
        }
        out.push('-');
        out.push_str(p);
    }
    Some(out)
}

fn is_meta_line(l: &str) -> bool {
    [
        "highlighted_at: ",
        "tags: ",
        "color: ",
        "type: ",
        "format: ",
    ]
    .iter()
    .any(|p| l.starts_with(p))
}

/// The highlight of one record: its leading blockquote, and the meta line
/// after it. Image and LaTeX records carry no text and give `None`.
fn record(lines: &[(usize, &str)]) -> Option<PassageRecord> {
    let mut it = lines
        .iter()
        .skip_while(|(_, l)| l.trim().is_empty())
        .peekable();
    let mut quote: Vec<(usize, String)> = Vec::new();
    while let Some((n, l)) = it.peek() {
        let Some(rest) = l.strip_prefix('>') else {
            break;
        };
        quote.push((*n, rest.strip_prefix(' ').unwrap_or(rest).to_string()));
        it.next();
    }
    // Trailing empty quote lines are not text.
    while quote.last().is_some_and(|(_, t)| t.trim().is_empty()) {
        quote.pop();
    }
    if quote.is_empty() {
        return None;
    }
    let mut p = PassageRecord {
        line_start: quote[0].0,
        line_end: quote[quote.len() - 1].0,
        original: quote
            .iter()
            .map(|(_, t)| t.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        ..PassageRecord::default()
    };
    if let Some((_, meta)) = it.find(|(_, l)| !l.trim().is_empty()) {
        if is_meta_line(meta) {
            for part in meta.split(" | ") {
                let Some((k, v)) = part.split_once(": ") else {
                    continue;
                };
                let v = v.trim();
                match k.trim() {
                    "highlighted_at" => {
                        p.saved_at = Some(v.split('T').next().unwrap_or(v).to_string())
                    }
                    "tags" => {
                        p.tags = v
                            .split(',')
                            .map(|t| t.trim().to_string())
                            .filter(|t| !t.is_empty())
                            .collect()
                    }
                    "color" => p.color = Some(v.to_string()),
                    _ => {}
                }
            }
        }
    }
    Some(p)
}

fn is_http(u: &str) -> bool {
    u.starts_with("http://") || u.starts_with("https://")
}

/// A DOI as a resolvable link: `10.1007/x`, `doi:10.1007/x` and
/// `https://doi.org/10.1007/x` all give `https://doi.org/10.1007/x`.
fn doi_url(raw: &str) -> Option<String> {
    let t = raw.trim();
    let lower = t.to_ascii_lowercase();
    let bare = [
        "https://doi.org/",
        "http://doi.org/",
        "https://dx.doi.org/",
        "http://dx.doi.org/",
        "doi:",
    ]
    .iter()
    .find(|p| lower.starts_with(*p))
    .map(|p| t[p.len()..].trim())
    .unwrap_or(t);
    bare.starts_with("10.")
        .then(|| format!("https://doi.org/{bare}"))
}

/// The work's public link: `url:`, then `source_url:`, then a DOI (Zotero
/// keeps it in `source_data.fields.DOI`). Non-web values (`mailto:`, a bare
/// word) are not links. The X post URL is the `url:` of an X work.
fn public_link(fm: &BTreeMap<String, String>, source_data: &serde_json::Value) -> Option<String> {
    let get = |k: &str| fm.get(k).map(|v| v.trim()).filter(|v| !v.is_empty());
    let web = ["url", "source_url"]
        .iter()
        .filter_map(|k| get(k))
        .find(|u| is_http(u))
        .map(String::from);
    web.or_else(|| {
        let s = |v: Option<&serde_json::Value>| v.and_then(|v| v.as_str()).map(String::from);
        let fields = source_data.get("fields");
        s(fields.and_then(|f| f.get("DOI")))
            .or_else(|| s(source_data.get("DOI")))
            .or_else(|| s(source_data.get("doi")))
            .or_else(|| get("doi").map(String::from))
            .and_then(|d| doi_url(&d))
    })
}

/// The public link of a work file's frontmatter (see [`public_link`]), read
/// at cite time for an index built before the link rules covered it.
pub fn work_public_url(text: &str) -> Option<String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let lines: Vec<&str> = text
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect();
    if lines.first().map(|l| l.trim_end()) != Some("---") {
        return None;
    }
    let close = 1 + lines.iter().skip(1).position(|l| l.trim_end() == "---")?;
    let fm = frontmatter(&lines[1..close]);
    let source_data: serde_json::Value = fm
        .get("source_data")
        .and_then(|s| serde_json::from_str(s.trim()).ok())
        .unwrap_or(serde_json::Value::Null);
    public_link(&fm, &source_data)
}

/// Parse one work file. `None` when it has no frontmatter.
pub fn work(rel_path: &str, text: &str) -> Option<DocRecord> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let lines: Vec<&str> = text
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect();
    if lines.first().map(|l| l.trim_end()) != Some("---") {
        return None;
    }
    let close = 1 + lines.iter().skip(1).position(|l| l.trim_end() == "---")?;
    let fm = frontmatter(&lines[1..close]);
    let get = |k: &str| {
        fm.get(k)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let source_data: serde_json::Value = get("source_data")
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(serde_json::Value::Null);

    // Records: blocks between bare `---` lines, with 1-based line numbers.
    let mut passages = Vec::new();
    let mut cur: Vec<(usize, &str)> = Vec::new();
    for (i, l) in lines.iter().enumerate().skip(close + 1) {
        if l.trim_end() == "---" {
            passages.extend(record(&cur));
            cur.clear();
        } else {
            cur.push((i + 1, l));
        }
    }
    passages.extend(record(&cur));

    let url = public_link(&fm, &source_data);
    let source = get("source_system");
    let published = source_data
        .get("date")
        .and_then(|d| d.as_str())
        .and_then(zotero_date)
        .or_else(|| {
            let id = if source.as_deref() == Some("x") {
                get("source_id")
            } else {
                None
            };
            id.and_then(|id| snowflake_time(&id))
        })
        .or_else(|| {
            url.as_deref()
                .and_then(|u| status_re().captures(u))
                .and_then(|c| snowflake_time(&c[1]))
        });
    let (date, date_source) = match published {
        Some(d) => (Some(d), Some("published".to_string())),
        None => match passages.iter().filter_map(|p| p.saved_at.clone()).min() {
            Some(d) => (Some(d), Some("saved".to_string())),
            None => (None, None),
        },
    };
    let kind = get("type");
    let meta = DocMeta {
        title: get("title"),
        date,
        genre: kind.clone(),
        topics: vec![],
        lang: None,
        summary: None,
        author: get("author"),
        public_url: url,
        kind,
        source,
        date_source,
        handle: None,
    };
    Some(DocRecord {
        key: rel_path.to_string(),
        meta,
        passages,
        index_title: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_from_zotero_and_snowflakes() {
        assert_eq!(zotero_date("2003-03-00 3/2003").as_deref(), Some("2003-03"));
        assert_eq!(zotero_date("2003-00-00 2003").as_deref(), Some("2003"));
        assert_eq!(
            zotero_date("2016-05-31 May 31, 2016").as_deref(),
            Some("2016-05-31")
        );
        assert_eq!(zotero_date("0000-00-00"), None);
        assert_eq!(zotero_date(""), None);
        // 1939901126326764018 is 2025-07-01T04:17:24Z (the tweets corpus).
        assert_eq!(
            snowflake_time("1939901126326764018").as_deref(),
            Some("2025-07-01T04:17:24Z")
        );
        assert_eq!(snowflake_time("20"), None, "pre-snowflake ids have no time");
        assert_eq!(snowflake_time("rw_book_1"), None);
    }

    #[test]
    fn public_link_prefers_web_urls_then_doi() {
        let doc = |fm: &str| format!("---\n{fm}\n---\n\n> q\n");
        assert_eq!(
            work_public_url(&doc("url: https://x.com/a/status/1")).as_deref(),
            Some("https://x.com/a/status/1")
        );
        assert_eq!(
            work_public_url(&doc("url: mailto:a@b.c\nsource_url: https://e.org/p")).as_deref(),
            Some("https://e.org/p")
        );
        assert_eq!(
            work_public_url(&doc(
                "url: mailto:a@b.c\nsource_data: {\"fields\":{\"DOI\":\"10.1007/BF02478291\"}}"
            ))
            .as_deref(),
            Some("https://doi.org/10.1007/BF02478291")
        );
        assert_eq!(
            doi_url("doi:10.1/x").as_deref(),
            Some("https://doi.org/10.1/x")
        );
        assert_eq!(
            doi_url("https://doi.org/10.1/x").as_deref(),
            Some("https://doi.org/10.1/x")
        );
        assert_eq!(doi_url("not a doi"), None);
        assert_eq!(work_public_url(&doc("title: t")), None);
    }

    #[test]
    fn multi_line_quoted_title() {
        let fm = frontmatter(&[
            "title: \"Why do humans ever",
            "  develop? \\\"Really\\\"\"",
            "author: A",
        ]);
        assert_eq!(fm["title"], "Why do humans ever develop? \"Really\"");
        assert_eq!(fm["author"], "A");
    }
}

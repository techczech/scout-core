//! Citations (J1): British dates, per-corpus links, sentence selection and the
//! markdown / plain citation forms. Quotes are always original text.

use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use serde::Serialize;
use std::path::Path;

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Format a frontmatter date as "D Month YYYY" (or "Month YYYY", "YYYY" when
/// the date is only that precise). Unparseable dates give `None`: an unknown
/// date is omitted, never guessed.
pub fn format_date(raw: &str) -> Option<String> {
    let t = raw.trim();
    let t = t.split(['T', ' ']).next().unwrap_or(t);
    let parts: Vec<&str> = t.split('-').collect();
    let year = parts.first()?;
    if year.len() != 4 || !year.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let month = match parts.get(1) {
        None => return Some(year.to_string()),
        Some(m) => m.parse::<usize>().ok().filter(|m| (1..=12).contains(m))?,
    };
    let day = match parts.get(2) {
        None => return Some(format!("{} {}", MONTHS[month - 1], year)),
        Some(d) => d.parse::<u32>().ok().filter(|d| (1..=31).contains(d))?,
    };
    if parts.len() > 3 {
        return None;
    }
    Some(format!("{} {} {}", day, MONTHS[month - 1], year))
}

/// Everything except RFC 3986 unreserved characters is percent-encoded.
const QUERY_VALUE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// Fill a link template: `{path}` = URL-encoded absolute path, `{line}` = line.
pub fn render_link(template: &str, abs_path: &Path, line: usize) -> String {
    let path = abs_path.to_string_lossy();
    let enc = utf8_percent_encode(&path, QUERY_VALUE).to_string();
    template
        .replace("{path}", &enc)
        .replace("{line}", &line.to_string())
}

/// Byte spans `[start, end)` of sentences in `text`. A sentence ends after
/// `.`, `!`, `?` or `…` (plus any closing quotes, brackets or emphasis marks)
/// followed by whitespace. Spans are trimmed of surrounding whitespace.
pub fn sentence_spans(text: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    let mut start = 0;
    let mut i = 0;
    while i < chars.len() {
        let (_, c) = chars[i];
        if matches!(c, '.' | '!' | '?' | '…') {
            let mut j = i + 1;
            while j < chars.len()
                && matches!(
                    chars[j].1,
                    '.' | '!' | '?' | '…' | '"' | '\'' | '’' | '”' | ')' | ']' | '*' | '_'
                )
            {
                j += 1;
            }
            if j >= chars.len() || chars[j].1.is_whitespace() {
                let end = if j < chars.len() {
                    chars[j].0
                } else {
                    text.len()
                };
                spans.push((start, end));
                start = end;
                i = j;
                continue;
            }
        }
        i += 1;
    }
    if start < text.len() {
        spans.push((start, text.len()));
    }
    spans
        .into_iter()
        .filter_map(|(s, e)| {
            let seg = &text[s..e];
            let lead = seg.len() - seg.trim_start().len();
            let trimmed = seg.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some((s + lead, s + lead + trimmed.len()))
            }
        })
        .collect()
}

/// Skip a leading Markdown block marker (`# `, `> `, `- `, `1. `) so a quote
/// starts at the prose. The result is still a substring of the original.
pub fn skip_block_marker(text: &str, start: usize, end: usize) -> usize {
    let seg = &text[start..end];
    let mut off = 0;
    loop {
        let rest = &seg[off..];
        let t = rest.trim_start();
        let ws = rest.len() - t.len();
        let marker = if t.starts_with('#') {
            let h = t.len() - t.trim_start_matches('#').len();
            if t[h..].starts_with(' ') {
                h + 1
            } else {
                0
            }
        } else if t.starts_with("> ")
            || t.starts_with("- ")
            || t.starts_with("* ")
            || t.starts_with("+ ")
        {
            2
        } else {
            let d = t.len() - t.trim_start_matches(|c: char| c.is_ascii_digit()).len();
            if d > 0 && (t[d..].starts_with(". ") || t[d..].starts_with(") ")) {
                d + 2
            } else {
                0
            }
        };
        if marker == 0 {
            break;
        }
        off += ws + marker;
    }
    let rest = &seg[off..];
    start + off + (rest.len() - rest.trim_start().len())
}

/// The metadata a citation needs.
#[derive(Debug, Clone, Serialize)]
pub struct CiteSource<'a> {
    pub author: Option<&'a str>,
    pub title: &'a str,
    pub date: Option<&'a str>,
    pub link: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CiteFormat {
    Markdown,
    Plain,
}

fn attribution(src: &CiteSource, md: bool) -> String {
    let mut parts = Vec::new();
    if let Some(a) = src.author.filter(|a| !a.trim().is_empty()) {
        parts.push(a.to_string());
    }
    parts.push(if md {
        format!("*{}*", src.title)
    } else {
        src.title.to_string()
    });
    if let Some(d) = src.date.and_then(format_date) {
        parts.push(d);
    }
    format!("— {}", parts.join(", "))
}

/// Render a citation for an original-text `quote`.
pub fn render_citation(quote: &str, src: &CiteSource, format: CiteFormat) -> String {
    let mut out = String::new();
    match format {
        CiteFormat::Markdown => {
            for l in quote.lines() {
                if l.trim().is_empty() {
                    out.push_str(">\n");
                } else {
                    out.push_str("> ");
                    out.push_str(l);
                    out.push('\n');
                }
            }
            out.push('\n');
            out.push_str(&attribution(src, true));
            out.push('\n');
            if let Some(link) = src.link {
                out.push('<');
                out.push_str(link);
                out.push_str(">\n");
            }
        }
        CiteFormat::Plain => {
            out.push('“');
            out.push_str(quote);
            out.push_str("”\n");
            out.push_str(&attribution(src, false));
            out.push('\n');
            if let Some(link) = src.link {
                out.push_str(link);
                out.push('\n');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn british_dates() {
        assert_eq!(format_date("2016-06-23").as_deref(), Some("23 June 2016"));
        assert_eq!(format_date("2016-06").as_deref(), Some("June 2016"));
        assert_eq!(format_date("2016").as_deref(), Some("2016"));
        assert_eq!(format_date("unknown"), None);
        assert_eq!(format_date("2016-13-01"), None);
    }

    #[test]
    fn link_is_url_encoded() {
        let l = render_link(crate::registry::WRITEFLEX_LINK, Path::new("/a b/č.md"), 7);
        assert_eq!(l, "writeflex://open?path=%2Fa%20b%2F%C4%8D.md&line=7");
    }

    #[test]
    fn sentences_split_and_trim() {
        let t = "One here. Two (really)! Three";
        let s: Vec<&str> = sentence_spans(t)
            .into_iter()
            .map(|(a, b)| &t[a..b])
            .collect();
        assert_eq!(s, vec!["One here.", "Two (really)!", "Three"]);
    }
}

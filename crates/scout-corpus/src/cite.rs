//! Citations (J1): British dates, per-corpus links, sentence selection and the
//! markdown / plain citation forms. Quotes are always original text.

use crate::registry::CiteStyle;
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

/// Path segments keep `/`; everything else outside the unreserved set is
/// percent-encoded.
const PATH_SEGMENTS: &AsciiSet = &QUERY_VALUE.remove(b'/');

/// A `file://` URL for an absolute path.
pub fn file_url(abs_path: &Path) -> String {
    let path = abs_path.to_string_lossy();
    format!("file://{}", utf8_percent_encode(&path, PATH_SEGMENTS))
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
#[derive(Debug, Clone, Default, Serialize)]
pub struct CiteSource<'a> {
    pub style: CiteStyle,
    pub author: Option<&'a str>,
    pub title: &'a str,
    pub date: Option<&'a str>,
    /// The archive link (for example `writeflex://…`).
    pub link: Option<&'a str>,
    /// The document's public URL, when its frontmatter has one.
    pub public_url: Option<&'a str>,
    /// Tweets: the account handle, without `@`.
    pub handle: Option<&'a str>,
    /// Highlights: `published` or `saved` (the date is a save date).
    pub date_source: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CiteFormat {
    Markdown,
    Plain,
}

fn push_links(out: &mut String, links: &[(&str, Option<&str>)], md: bool) {
    for (label, url) in links {
        if let Some(u) = url.filter(|u| !u.trim().is_empty()) {
            if md {
                out.push_str(&format!(" · [{label}]({u})"));
            } else {
                out.push_str(&format!(" · {label}: {u}"));
            }
        }
    }
}

/// `— Dominik Lukeš (@techczech), tweet, 26 July 2025 · [public](…)`; the
/// archive link stands in when there is no public URL.
fn tweet_attribution(src: &CiteSource, md: bool) -> String {
    let mut who = src
        .author
        .filter(|a| !a.trim().is_empty())
        .unwrap_or("")
        .to_string();
    if let Some(h) = src.handle.filter(|h| !h.is_empty()) {
        who = if who.is_empty() {
            format!("@{h}")
        } else {
            format!("{who} (@{h})")
        };
    }
    let mut parts = Vec::new();
    if !who.is_empty() {
        parts.push(who);
    }
    parts.push("tweet".to_string());
    if let Some(d) = src.date.and_then(format_date) {
        parts.push(d);
    }
    let mut out = format!("— {}", parts.join(", "));
    let link = if src.public_url.is_some_and(|u| !u.trim().is_empty()) {
        ("public", src.public_url)
    } else {
        ("archive", src.link)
    };
    push_links(&mut out, &[link], md);
    out
}

/// `— Author, *Title*, 2019 · [highlight](…)`: the year of the work's
/// publication; a save date (no publication date known) reads `saved 2021`.
fn highlight_attribution(src: &CiteSource, md: bool) -> String {
    let mut parts = Vec::new();
    if let Some(a) = src.author.filter(|a| !a.trim().is_empty()) {
        parts.push(a.to_string());
    }
    parts.push(if md {
        format!("*{}*", src.title)
    } else {
        src.title.to_string()
    });
    if let Some(y) = src.date.and_then(crate::filter::year_of) {
        if src.date_source == Some("saved") {
            parts.push(format!("saved {y}"));
        } else {
            parts.push(y.to_string());
        }
    }
    let mut out = format!("— {}", parts.join(", "));
    push_links(&mut out, &[("highlight", src.link)], md);
    out
}

/// `— Author, *Title*, 23 June 2016 · [archive](…) · [public](…)`; the
/// plain form drops the Markdown (`· archive: … · public: …`). The title is
/// always the full frontmatter title. Tweets and highlights have their own
/// forms ([`CiteStyle`]).
fn attribution(src: &CiteSource, md: bool) -> String {
    match src.style {
        CiteStyle::Tweet => return tweet_attribution(src, md),
        CiteStyle::Highlight => return highlight_attribution(src, md),
        CiteStyle::Writing => {}
    }
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
    let mut out = format!("— {}", parts.join(", "));
    push_links(
        &mut out,
        &[("archive", src.link), ("public", src.public_url)],
        md,
    );
    out
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
        }
        CiteFormat::Plain => {
            out.push('“');
            out.push_str(quote);
            out.push_str("”\n");
            out.push_str(&attribution(src, false));
            out.push('\n');
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
    fn citation_carries_archive_and_public_links() {
        let mut src = CiteSource {
            author: Some("Dominik Lukeš"),
            title:
                "Repaved paths and generative metaphors: Expressing human purposes with technology",
            date: Some("2016-06-23"),
            link: Some("writeflex://open?path=%2Fa.md&line=3"),
            public_url: Some("https://medium.com/x/repaved"),
            ..CiteSource::default()
        };
        assert_eq!(
            render_citation("Q.", &src, CiteFormat::Markdown),
            "> Q.\n\n— Dominik Lukeš, *Repaved paths and generative metaphors: Expressing human purposes with technology*, 23 June 2016 · [archive](writeflex://open?path=%2Fa.md&line=3) · [public](https://medium.com/x/repaved)\n"
        );
        assert_eq!(
            render_citation("Q.", &src, CiteFormat::Plain),
            "“Q.”\n— Dominik Lukeš, Repaved paths and generative metaphors: Expressing human purposes with technology, 23 June 2016 · archive: writeflex://open?path=%2Fa.md&line=3 · public: https://medium.com/x/repaved\n"
        );
        src.public_url = None;
        assert_eq!(
            render_citation("Q.", &src, CiteFormat::Markdown),
            "> Q.\n\n— Dominik Lukeš, *Repaved paths and generative metaphors: Expressing human purposes with technology*, 23 June 2016 · [archive](writeflex://open?path=%2Fa.md&line=3)\n"
        );
        src.link = None;
        src.date = Some("undated");
        assert_eq!(
            render_citation("Q.", &src, CiteFormat::Plain),
            "“Q.”\n— Dominik Lukeš, Repaved paths and generative metaphors: Expressing human purposes with technology\n"
        );
    }

    #[test]
    fn tweet_and_highlight_citations() {
        let t = CiteSource {
            style: CiteStyle::Tweet,
            author: Some("Dominik Lukeš"),
            title: "ignored",
            date: Some("2025-07-26T10:12:00Z"),
            link: Some("writeflex://open?path=%2Ft.md&line=9"),
            public_url: Some("https://x.com/techczech/status/1"),
            handle: Some("techczech"),
            date_source: Some("published"),
        };
        assert_eq!(
            render_citation("Q", &t, CiteFormat::Markdown),
            "> Q\n\n— Dominik Lukeš (@techczech), tweet, 26 July 2025 · [public](https://x.com/techczech/status/1)\n"
        );
        assert_eq!(
            render_citation("Q", &t, CiteFormat::Plain),
            "“Q”\n— Dominik Lukeš (@techczech), tweet, 26 July 2025 · public: https://x.com/techczech/status/1\n"
        );
        let mut h = CiteSource {
            style: CiteStyle::Highlight,
            author: Some("George Lakoff"),
            title: "Metaphors We Live By",
            date: Some("1980-00"),
            link: Some("file:///a/b%20c.md"),
            date_source: Some("published"),
            ..CiteSource::default()
        };
        h.date = Some("1980");
        assert_eq!(
            render_citation("Q", &h, CiteFormat::Markdown),
            "> Q\n\n— George Lakoff, *Metaphors We Live By*, 1980 · [highlight](file:///a/b%20c.md)\n"
        );
        h.date = Some("2021-04-03");
        h.date_source = Some("saved");
        assert_eq!(
            render_citation("Q", &h, CiteFormat::Plain),
            "“Q”\n— George Lakoff, Metaphors We Live By, saved 2021 · highlight: file:///a/b%20c.md\n"
        );
        assert_eq!(file_url(Path::new("/a b/č.md")), "file:///a%20b/%C4%8D.md");
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

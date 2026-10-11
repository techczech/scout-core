//! Names and dates of a seeded record: periods, monthly ids, slugs, record
//! ids and the themes most of an event's items carry.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use unicode_normalization::UnicodeNormalization;

/// Longest slug (the part of a record id after `YYYY-MM-`).
pub const SLUG_MAX: usize = 60;
/// Most themes a seeded record carries.
pub const THEMES_MAX: usize = 5;

/// A briefing period: `YYYY-MM` or `YYYY-Qn`, exactly so (ASCII digits).
pub(crate) fn is_period(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 7
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && match &b[5..] {
            [b'Q', q] => (b'1'..=b'4').contains(q),
            [b'0', d] => (b'1'..=b'9').contains(d),
            [b'1', d] => (b'0'..=b'2').contains(d),
            _ => false,
        }
}

/// A monthly event id `<period>/E<digits>` as its period and number
/// (`E21`); `None` for anything else.
pub(crate) fn split_monthly_id(id: &str) -> Option<(&str, &str)> {
    let (period, number) = id.split_once('/')?;
    let digits = number.strip_prefix('E')?;
    (is_period(period) && !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
        .then_some((period, number))
}

/// Monthly ids in order: by period, then by event number.
pub(crate) fn by_monthly_id(a: &str, b: &str) -> Ordering {
    let (Some((pa, na)), Some((pb, nb))) = (split_monthly_id(a), split_monthly_id(b)) else {
        return a.cmp(b);
    };
    // Numbers of any length compare by their digits without leading zeros.
    let digits = |n: &str| n[1..].trim_start_matches('0').to_string();
    let (da, db) = (digits(na), digits(nb));
    pa.cmp(pb)
        .then(da.len().cmp(&db.len()))
        .then(da.cmp(&db))
        .then(na.cmp(nb))
}

/// The month an id starts with: the event's month, or the first month of
/// its quarter (`2022-Q2` gives `2022-04`).
pub(crate) fn period_month(period: &str) -> String {
    match period.as_bytes() {
        [.., b'Q', q] => format!("{}-{:02}", &period[..4], (q - b'1') * 3 + 1),
        _ => period.to_string(),
    }
}

/// First and last day of a month or quarter period.
pub(crate) fn period_span(period: &str) -> (String, String) {
    let first = period_month(period);
    let year: u32 = period[..4].parse().unwrap_or(0);
    let mut month: u32 = first[5..].parse().unwrap_or(1);
    if period.contains('Q') {
        month += 2;
    }
    let leap = (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400);
    let last = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    (
        format!("{first}-01"),
        format!("{:04}-{month:02}-{last:02}", year),
    )
}

/// Lower-case ASCII words joined by hyphens; empty when the name has no
/// ASCII letter or digit once accents are taken off. Never a path: no
/// `/`, `.` or `\` survives.
pub(crate) fn slugify(name: &str) -> String {
    let mut out = String::new();
    let plain = name
        .nfkd()
        .filter(|c| !('\u{300}'..='\u{36f}').contains(c))
        .flat_map(char::to_lowercase);
    for c in plain {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

/// The first `max` chars of a slug, without a hyphen at the end.
fn cut(slug: &str, max: usize) -> &str {
    // A slug is ASCII, so bytes are chars.
    slug[..slug.len().min(max)].trim_end_matches('-')
}

/// The id chosen for a monthly event, and how it was reached.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PickedId {
    pub id: String,
    /// The id before any `-2`, `-3` suffix.
    pub base: String,
    /// The name gave no slug; the event number stands in.
    pub fallback: bool,
    pub truncated: bool,
    /// 0 for none.
    pub suffix: usize,
}

/// The record id for a name: `<month>-<slug>`, the slug cut so that a
/// `-2`, `-3` … suffix still fits [`SLUG_MAX`]. `taken` holds the ids in
/// use (lower case); the chosen id is added to it.
pub(crate) fn assign_id(
    period: &str,
    number: &str,
    name: &str,
    taken: &mut BTreeSet<String>,
) -> PickedId {
    let full = slugify(name);
    let fallback = full.is_empty();
    let whole = if fallback {
        let n: String = number
            .to_lowercase()
            .chars()
            .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
            .collect();
        format!("event-{}", if n.is_empty() { "x" } else { &n })
    } else {
        full
    };
    let month = period_month(period);
    let mut n = 1usize;
    loop {
        let suffix = if n == 1 {
            String::new()
        } else {
            format!("-{n}")
        };
        let id = format!(
            "{month}-{}{suffix}",
            cut(&whole, SLUG_MAX.saturating_sub(suffix.len()))
        );
        if taken.insert(id.clone()) {
            return PickedId {
                id,
                base: format!("{month}-{}", cut(&whole, SLUG_MAX)),
                fallback,
                truncated: whole.len() > SLUG_MAX,
                suffix: if n == 1 { 0 } else { n },
            };
        }
        n += 1;
    }
}

/// A record id is lower-case ASCII words joined by single hyphens, so
/// `<id>.md` is always a plain file name.
pub(crate) fn is_slug(id: &str) -> bool {
    !id.is_empty()
        && id.split('-').all(|w| {
            !w.is_empty()
                && w.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

/// Themes carried by at least a third of the items: most frequent first,
/// then alphabetical, at most [`THEMES_MAX`]. Each item's themes count
/// once.
pub(crate) fn majority_themes<'a>(items: impl IntoIterator<Item = &'a [String]>) -> Vec<String> {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut total = 0usize;
    for themes in items {
        total += 1;
        let once: BTreeSet<&str> = themes
            .iter()
            .map(|t| t.trim())
            .filter(|t| !t.is_empty())
            .collect();
        for t in once {
            *counts.entry(t).or_default() += 1;
        }
    }
    let mut kept: Vec<(&str, usize)> = counts.into_iter().filter(|(_, c)| c * 3 >= total).collect();
    kept.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    kept.into_iter()
        .take(THEMES_MAX)
        .map(|(t, _)| t.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const AWKWARD: &str = "He said: \"it's #1\" — naïve café, 100% [sic] & more";

    #[test]
    fn slugs_are_lower_case_ascii_with_hyphens_and_never_a_path() {
        assert_eq!(slugify(AWKWARD), "he-said-it-s-1-naive-cafe-100-sic-more");
        assert_eq!(slugify("  ../../etc/passwd  "), "etc-passwd");
        assert_eq!(slugify("日本語"), "");
        assert_eq!(slugify("Ærø & Ünïcode"), "r-unicode");
        assert_eq!(slugify("İstanbul K"), "istanbul-k");
        assert_eq!(slugify("a\\..\\b\u{0}c"), "a-b-c");
        let long = slugify(&"ab ".repeat(40));
        assert!(is_slug(&long));
        assert!(is_slug(cut(&long, SLUG_MAX)) && cut(&long, SLUG_MAX).len() <= SLUG_MAX);
        for bad in ["", "-a", "a-", "a--b", "A", "a.md", "a/b", "é"] {
            assert!(!is_slug(bad), "{bad}");
        }
    }

    #[test]
    fn periods_give_their_first_month_and_their_first_and_last_day() {
        assert_eq!(period_month("2022-Q2"), "2022-04");
        assert_eq!(period_month("2026-09"), "2026-09");
        let span = |p| period_span(p);
        assert_eq!(span("2022-Q2"), ("2022-04-01".into(), "2022-06-30".into()));
        assert_eq!(span("2023-Q4"), ("2023-10-01".into(), "2023-12-31".into()));
        assert_eq!(span("2024-02"), ("2024-02-01".into(), "2024-02-29".into()));
        assert_eq!(span("2100-02"), ("2100-02-01".into(), "2100-02-28".into()));
        assert_eq!(span("2026-09"), ("2026-09-01".into(), "2026-09-30".into()));
        for ok in ["2026-01", "2026-12", "2022-Q1", "2022-Q4"] {
            assert!(is_period(ok), "{ok}");
        }
        for bad in [
            "2026-00",
            "2026-13",
            "2022-Q5",
            "2022-Q0",
            "202-01",
            "2026-1",
            "２０２６-01",
            "2026/01",
        ] {
            assert!(!is_period(bad), "{bad}");
        }
        assert_eq!(split_monthly_id("2026-09/E21"), Some(("2026-09", "E21")));
        assert_eq!(split_monthly_id("2022-Q2/E01"), Some(("2022-Q2", "E01")));
        for bad in [
            "2026-09/E",
            "2026-09/21",
            "2026-09/e21",
            "2026-09/E2x",
            "2026-09",
        ] {
            assert_eq!(split_monthly_id(bad), None, "{bad}");
        }
        let mut ids = vec![
            "2026-09/E10",
            "2026-10/E01",
            "2026-09/E2",
            "2022-Q2/E01",
            "2026-09/E02",
        ];
        ids.sort_by(|a, b| by_monthly_id(a, b));
        assert_eq!(
            ids,
            [
                "2022-Q2/E01",
                "2026-09/E02",
                "2026-09/E2",
                "2026-09/E10",
                "2026-10/E01"
            ]
        );
    }

    #[test]
    fn ids_take_a_suffix_on_collision_fall_back_to_the_number_and_fit_the_cap() {
        let mut taken = BTreeSet::new();
        let mut id = |p: &str, n: &str, name: &str| assign_id(p, n, name, &mut taken);
        assert_eq!(
            id("2026-09", "E02", "Gemini launch").id,
            "2026-09-gemini-launch"
        );
        assert_eq!(
            id("2026-09", "E03", "Gemini Launch!"),
            PickedId {
                id: "2026-09-gemini-launch-2".into(),
                base: "2026-09-gemini-launch".into(),
                fallback: false,
                truncated: false,
                suffix: 2
            }
        );
        assert_eq!(
            id("2026-09", "E07", "gemini  launch").id,
            "2026-09-gemini-launch-3"
        );
        assert_eq!(
            id("2026-10", "E01", "Gemini launch").id,
            "2026-10-gemini-launch"
        );
        assert_eq!(
            id("2022-Q2", "E01", "Quarter story").id,
            "2022-04-quarter-story"
        );
        let jp = id("2026-09", "E10", "日本語");
        assert_eq!((jp.id.as_str(), jp.fallback), ("2026-09-event-e10", true));
        assert_eq!(id("2026-09", "E12", "!!!").id, "2026-09-event-e12");
        let a = id("2026-09", "E20", &"x".repeat(80));
        let b = id("2026-09", "E21", &"x".repeat(80));
        assert_eq!(a.id, format!("2026-09-{}", "x".repeat(60)));
        assert_eq!(b.id, format!("2026-09-{}-2", "x".repeat(58)));
        assert!(a.truncated && b.suffix == 2);
        assert!(taken.iter().all(|i| is_slug(i)));
    }

    #[test]
    fn themes_are_those_a_third_of_the_items_carry_most_frequent_first_at_most_five() {
        let v = |l: &[&str]| l.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let pick = |items: Vec<Vec<String>>| majority_themes(items.iter().map(Vec::as_slice));
        assert_eq!(
            pick(vec![v(&["A", "B", "A"]), v(&["A"]), v(&["A", "C"]), v(&[])]),
            ["A"]
        );
        assert_eq!(
            pick(vec![v(&["b", "a"]), v(&["b", "a", "c"]), v(&["c"])]),
            ["a", "b", "c"]
        );
        assert_eq!(
            pick(vec![v(&["g", "f", "e", "d", "c", "b", "a"])]),
            ["a", "b", "c", "d", "e"]
        );
        // Blank themes are not themes; a theme is counted as trimmed.
        assert_eq!(pick(vec![v(&[" A ", "A", "  "])]), ["A"]);
        assert!(pick(vec![]).is_empty());
    }
}

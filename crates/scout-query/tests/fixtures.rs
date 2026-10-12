//! The shared grammar fixtures: every case in
//! `packages/scout-query/fixtures/grammar-cases.json` (generated from the TS
//! package) must parse to the same value here.

use scout_query::{date_range, is_empty_or_corpus_only, parse_query, parse_search};
use serde_json::Value;

fn fixtures() -> Value {
    let text = include_str!("../../../packages/scout-query/fixtures/grammar-cases.json");
    serde_json::from_str(text).unwrap()
}

#[test]
fn parse_cases_match_the_ts_package() {
    let f = fixtures();
    let cases = f["parse"].as_array().unwrap();
    assert!(cases.len() > 40);
    let mut failures = Vec::new();
    for c in cases {
        let input = c["input"].as_str().unwrap();
        let partial = c["partial"].as_bool().unwrap();
        let got = serde_json::to_value(parse_search(input, partial)).unwrap();
        if got != c["expected"] {
            failures.push(format!(
                "{input:?} partial={partial}\n  want {}\n  got  {}",
                c["expected"], got
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn date_range_cases_match_the_ts_package() {
    let f = fixtures();
    let mut failures = Vec::new();
    for c in f["date_range"].as_array().unwrap() {
        let input = c["input"].as_str().unwrap();
        let got = serde_json::to_value(date_range(input)).unwrap();
        if got != c["expected"] {
            failures.push(format!("{input:?}: want {} got {}", c["expected"], got));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Empty, or nothing but `in:` terms: the one answer an engine uses to
/// decide that a query stands for a whole corpus.
#[test]
fn corpus_only_cases_match_the_ts_package() {
    let f = fixtures();
    let cases = f["corpus_only"].as_array().unwrap();
    assert!(cases.len() > 20);
    let mut failures = Vec::new();
    for c in cases {
        let input = c["input"].as_str().unwrap();
        let want = c["expected"].as_bool().unwrap();
        let got = is_empty_or_corpus_only(input);
        if got != want || parse_query(input).is_empty_or_corpus_only() != want {
            failures.push(format!("{input:?}: want {want} got {got}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    // The ones the engine's behaviour turns on, by hand.
    for yes in ["", "  ", "in:notes", "in:notes in:hl"] {
        assert!(is_empty_or_corpus_only(yes), "{yes:?}");
    }
    for no in ["OR", "!", "i:", "tag:\"\"", "in:notes OR", "in:notes x"] {
        assert!(!is_empty_or_corpus_only(no), "{no:?}");
    }
}

/// A parse result without its default-valued keys, as the generated
/// fixture stores it.
fn sparse(v: Value) -> Value {
    let Value::Object(m) = v else { return v };
    Value::Object(
        m.into_iter()
            .filter(|(_, v)| match v {
                Value::Null | Value::Bool(false) => false,
                Value::String(s) => !s.is_empty(),
                Value::Array(a) => !a.is_empty(),
                _ => true,
            })
            .collect(),
    )
}

/// Parity by generation: 600 seeded random queries and 400 labels made by
/// `packages/scout-query/generated.test.ts`; this port must give exactly
/// the TS results.
#[test]
fn generated_cases_match_the_ts_package() {
    let text = include_str!("../../../packages/scout-query/fixtures/generated-cases.json");
    let f: Value = serde_json::from_str(text).unwrap();
    let cases = f["parse"].as_array().unwrap();
    assert!(cases.len() >= 500);
    let mut failures = Vec::new();
    for c in cases {
        let input = c["input"].as_str().unwrap();
        let partial = c["partial"].as_bool().unwrap();
        let got = sparse(serde_json::to_value(parse_search(input, partial)).unwrap());
        if got != c["expected"] {
            failures.push(format!(
                "{input:?} partial={partial}\n  want {}\n  got  {}",
                c["expected"], got
            ));
        }
    }
    let labels = f["quote"].as_array().unwrap();
    assert!(labels.len() >= 300);
    for c in labels {
        let label = c["label"].as_str().unwrap();
        let quoted = scout_query::quote_value(label);
        if quoted != c["quoted"].as_str().unwrap() {
            failures.push(format!(
                "quote_value({label:?}): want {} got {quoted:?}",
                c["quoted"]
            ));
        }
        for q in [
            format!("theme:{quoted}"),
            format!("x theme:{quoted} y"),
            format!("theme:{quoted}\u{2003}/re/"),
        ] {
            let got = parse_search(&q, false).theme;
            if got.as_deref() != Some(label) {
                failures.push(format!("{q:?}: theme {got:?}, want {label:?}"));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} failures\n{}",
        failures.len(),
        failures[..failures.len().min(8)].join("\n")
    );
}

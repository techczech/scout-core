//! The shared grammar fixtures: every case in
//! `packages/scout-query/fixtures/grammar-cases.json` (generated from the TS
//! package) must parse to the same value here.

use scout_query::{date_range, parse_search};
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

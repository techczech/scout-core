//! The shared event-record cases: every file of
//! `tests/fixtures/event-records/` must get from the reader
//! ([`scout_corpus::events::read`]) the verdict `verdicts.json` names for
//! it. The seed stage's own check of existing records
//! (`tools/topic-tagger/lib/event-record.js`) runs over the same folder in
//! `tools/topic-tagger/test/event-record-fixtures.test.js`, so the two
//! cannot drift apart unnoticed: a rule added to either needs a case here.

use scout_corpus::events;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/event-records")
}

fn cases() -> Vec<Value> {
    let text = fs::read_to_string(fixtures().join("verdicts.json")).unwrap();
    let v: Value = serde_json::from_str(&text).unwrap();
    v["cases"].as_array().unwrap().clone()
}

/// Put the case's file into `dir`: a copy of `records/<file>`, or what its
/// `make` describes.
fn materialise(case: &Value, dir: &Path) {
    let file = case["file"].as_str().unwrap();
    let to = dir.join(file);
    let make = &case["make"];
    if make.is_null() {
        fs::copy(fixtures().join("records").join(file), &to).unwrap();
    } else if let Some(p) = make.get("pad") {
        let mut bytes = p["text"].as_str().unwrap().as_bytes().to_vec();
        let size = p["bytes"].as_u64().unwrap() as usize;
        assert!(bytes.len() <= size);
        bytes.resize(size, p["byte"].as_str().unwrap().as_bytes()[0]);
        fs::write(&to, bytes).unwrap();
    } else if let Some(r) = make.get("repeat") {
        let text = format!(
            "{}{}{}",
            r["prefix"].as_str().unwrap(),
            r["unit"]
                .as_str()
                .unwrap()
                .repeat(r["count"].as_u64().unwrap() as usize),
            r["suffix"].as_str().unwrap()
        );
        fs::write(&to, text).unwrap();
    } else if let Some(h) = make.get("hex") {
        let h = h.as_str().unwrap();
        let bytes: Vec<u8> = (0..h.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&h[i..i + 2], 16).unwrap())
            .collect();
        fs::write(&to, bytes).unwrap();
    } else if let Some(target) = make.get("symlink") {
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.join(target.as_str().unwrap()), &to).unwrap();
        #[cfg(not(unix))]
        let _ = target;
    } else if make.get("dir").is_some() {
        fs::create_dir(&to).unwrap();
    } else {
        panic!("{file}: unknown make {make}");
    }
}

/// The file name without `.md`, as the reader cuts it.
fn stem(file: &str) -> &str {
    file.trim_end_matches(".md")
}

#[test]
fn the_reader_gives_every_shared_case_its_verdict() {
    let cases = cases();
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join(events::EVENTS_DIR);
    fs::create_dir_all(&dir).unwrap();
    for c in &cases {
        materialise(c, &dir);
    }
    let set = events::read(root.path()).unwrap().unwrap();
    let mut wrong = Vec::new();
    let (mut accepted, mut rejected) = (0, 0);
    for c in &cases {
        let file = c["file"].as_str().unwrap();
        let prefix = format!("{}/{file}: ", events::EVENTS_DIR);
        let read = set.records.values().any(|r| r.id == stem(file));
        // A refusal: the file's own "skipped" line, or its repeated id.
        let refusal = set.warnings.iter().find(|w| {
            w.starts_with(&prefix)
                && (w.ends_with("; skipped") || w.contains("repeated or empty id"))
        });
        let got = match (read, refusal) {
            (true, None) => "accept",
            (false, Some(_)) => "reject",
            (false, None) => "ignored",
            (true, Some(_)) => "accept and reject",
        };
        let want = c["verdict"].as_str().unwrap();
        if got != want {
            wrong.push(format!(
                "{file}: want {want}, the reader says {got} ({refusal:?})"
            ));
            continue;
        }
        match want {
            "accept" => accepted += 1,
            "reject" => {
                rejected += 1;
                let why = c["why"].as_str().unwrap();
                if !refusal.unwrap().contains(why) {
                    wrong.push(format!("{file}: want {why:?} in {refusal:?}"));
                }
            }
            _ => {}
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    // Nothing read or skipped beyond the cases, and every kind is there.
    assert_eq!(set.records.len(), accepted);
    assert_eq!(set.skipped, rejected);
    assert!(accepted >= 15 && rejected >= 35, "{accepted} {rejected}");
}

#[test]
fn every_file_of_the_folder_is_a_case_and_every_case_has_its_file() {
    let cases = cases();
    let mut named: Vec<String> = cases
        .iter()
        .filter(|c| c["make"].is_null())
        .map(|c| c["file"].as_str().unwrap().to_string())
        .collect();
    named.sort();
    let mut have: Vec<String> = fs::read_dir(fixtures().join("records"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    have.sort();
    assert_eq!(have, named);
    let mut all: Vec<&str> = cases.iter().map(|c| c["file"].as_str().unwrap()).collect();
    all.sort();
    all.dedup();
    assert_eq!(all.len(), cases.len(), "a file is named twice");
}

//! CLI seam over the app-facing facade: for every command, `scout … --json`
//! prints exactly `api::to_json` of the facade's response to the same
//! request, with the same stderr notes and the same exit code.

use scout_corpus::api::{self, *};
use scout_corpus::{Engine, Outcome, Registry};
use serde::Serialize;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn scout(cfg: &Path, data: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_scout"))
        .args(args)
        .env("SCOUT_CONFIG", cfg)
        .env("SCOUT_DATA_DIR", data)
        .output()
        .unwrap()
}

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

fn snapshot(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut v: Vec<(String, Vec<u8>)> = walk(root)
        .into_iter()
        .map(|p| (p.display().to_string(), fs::read(&p).unwrap()))
        .collect();
    v.sort();
    v
}

fn walk(root: &Path) -> Vec<std::path::PathBuf> {
    let mut out = vec![];
    for e in fs::read_dir(root).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}

/// What the CLI printed and how it exited, against the facade's reply.
struct Case {
    args: Vec<&'static str>,
    json: String,
    notes: Vec<String>,
    code: i32,
}

fn case<T: Serialize + Outcome>(args: Vec<&'static str>, reply: Reply<T>) -> Case {
    Case {
        args,
        json: api::to_json(&reply.body).unwrap() + "\n",
        notes: reply.notes.clone(),
        code: if reply.body.has_results() { 0 } else { 1 },
    }
}

fn fixture(tmp: &Path) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    let (w, n, data) = (tmp.join("writing"), tmp.join("notes"), tmp.join("data"));
    write(
        &w,
        "2016-essay.md",
        "---\ntitle: Generative metaphor\ndate: 2016-06-23\ngenre: essay\n---\n\
         A generative metaphor frames a problem. A conceptual metaphor too.\n\n\
         Rock &amp;amp;amp; roll is a metaphor I don’t mind.\n\n\
         *Dominik Lukeš*\n",
    );
    write(
        &w,
        "2021-note.md",
        "---\ntitle: Later note\ndate: 2021-03-01\ngenre: note\n---\n\
         Every conceptual metaphor is a model; the metaphor is a model of thought.\n\n\
         See [the essay](https://example.org/essay) on metaphor.\n",
    );
    write(&n, "x.md", "---\ntitle: X\n---\nmetaphor\n");
    let cfg = tmp.join("corpora.toml");
    fs::write(
        &cfg,
        format!(
            "[[corpus]]\nid = \"writing\"\nname = \"W\"\nkind = \"markdown-folder\"\npath = \"{}\"\nrequire_frontmatter = [\"genre\"]\ndefault_author = \"Dominik Lukeš\"\nboilerplate = [\"^\\\\*Dominik Lukeš\\\\*$\"]\nlink = \"writeflex://open?path={{path}}&line={{line}}\"\n\n\
             [[corpus]]\nid = \"notes\"\nname = \"N\"\nkind = \"markdown-folder\"\npath = \"{}\"\n",
            w.display(),
            n.display()
        ),
    )
    .unwrap();
    (cfg, data, w)
}

#[test]
fn cli_json_equals_facade_response_for_every_command() {
    let tmp = tempfile::tempdir().unwrap();
    let (cfg, data, w) = fixture(tmp.path());
    let before = snapshot(&w);
    assert!(scout(&cfg, &data, &["index", "build", "writing"])
        .status
        .success());

    let engine = Engine::new(Registry::load_from(&cfg).unwrap(), data.join("indexes"));
    let scope = |in_: &[&str]| api::Scope {
        in_: in_.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    };
    let e = &engine;
    let cases = vec![
        case(
            vec!["search", "metaphor", "--json"],
            e.search(&SearchQuery {
                query: "metaphor".into(),
                ..Default::default()
            })
            .unwrap(),
        ),
        case(
            vec![
                "search",
                "conceptual metaphor",
                "--in",
                "writing",
                "--passage",
                "--limit",
                "1",
                "--json",
            ],
            e.search(&SearchQuery {
                query: "conceptual metaphor".into(),
                in_: vec!["writing".into()],
                limit: 1,
                passage: true,
                ..Default::default()
            })
            .unwrap(),
        ),
        case(
            vec!["kwic", "metaphor", "--sort", "L1", "--json"],
            e.kwic(&KwicQuery {
                term: "metaphor".into(),
                sort: "L1".into(),
                ..Default::default()
            })
            .unwrap(),
        ),
        case(
            vec![
                "kwic",
                "metaphor",
                "--near",
                "conceptual",
                "--in",
                "writing",
                "--after",
                "2020",
                "--json",
            ],
            e.kwic(&KwicQuery {
                term: "metaphor".into(),
                near: Some("conceptual".into()),
                scope: api::Scope {
                    in_: vec!["writing".into()],
                    after: Some("2020".into()),
                    ..Default::default()
                },
                ..Default::default()
            })
            .unwrap(),
        ),
        case(
            vec![
                "collocates",
                "metaphor",
                "--min",
                "1",
                "--dist",
                "doc",
                "--json",
            ],
            e.collocates(&CollocatesQuery {
                node: "metaphor".into(),
                min: 1,
                dist: Some("doc".into()),
                ..Default::default()
            })
            .unwrap(),
        ),
        case(
            vec![
                "collocates",
                "metaphor",
                "--min",
                "1",
                "--after",
                "2020",
                "--compare",
                "before:2020",
                "--json",
            ],
            e.collocates(&CollocatesQuery {
                node: "metaphor".into(),
                min: 1,
                scope: api::Scope {
                    after: Some("2020".into()),
                    ..Default::default()
                },
                compare: Some("before:2020".into()),
                ..Default::default()
            })
            .unwrap(),
        ),
        case(
            vec![
                "keyness",
                "--a",
                "before:2020",
                "--b",
                "after:2020",
                "--min",
                "1",
                "--json",
            ],
            e.keyness(&KeynessQuery {
                a: "before:2020".into(),
                b: "after:2020".into(),
                min: 1,
                ..Default::default()
            })
            .unwrap(),
        ),
        case(
            vec!["dist", "metaphor", "--by", "year", "--json"],
            e.dist(&DistQuery {
                term: "metaphor".into(),
                by: "year".into(),
                ..Default::default()
            })
            .unwrap(),
        ),
        case(
            vec!["ngrams", "--n", "1-2", "--in", "writing", "--json"],
            e.ngrams(&NgramsQuery {
                n: "1-2".into(),
                scope: scope(&["writing"]),
                ..Default::default()
            })
            .unwrap(),
        ),
        case(
            vec!["ngrams", "--n", "1-3", "--containing", "metaphor", "--json"],
            e.ngrams(&NgramsQuery {
                n: "1-3".into(),
                containing: Some("metaphor".into()),
                ..Default::default()
            })
            .unwrap(),
        ),
        case(
            vec!["profile", "metaphor", "--min-hits", "1", "--json"],
            e.profile(&ProfileQuery {
                word: "metaphor".into(),
                min_hits: 1,
                ..Default::default()
            })
            .unwrap(),
        ),
        case(
            vec!["verify-quote", "I don't mind", "--json"],
            e.verify_quote(&VerifyQuoteQuery {
                text: "I don't mind".into(),
                ..Default::default()
            })
            .unwrap(),
        ),
        case(
            vec![
                "verify-quote",
                "not in the corpus",
                "--in",
                "writing",
                "--json",
            ],
            e.verify_quote(&VerifyQuoteQuery {
                text: "not in the corpus".into(),
                in_: vec!["writing".into()],
            })
            .unwrap(),
        ),
        case(
            vec!["cite", "writing:2016-essay.md:6", "--json"],
            e.cite(&CiteQuery {
                passage_id: "writing:2016-essay.md:6".into(),
            })
            .unwrap(),
        ),
        case(
            vec!["similar", "writing:2016-essay.md:6", "--json"],
            e.similar(&SimilarQuery {
                seeds: vec!["writing:2016-essay.md:6".into()],
                ..Default::default()
            })
            .unwrap(),
        ),
        case(
            vec![
                "similar",
                "writing:2016-essay.md:6",
                "--in",
                "writing",
                "--exclude-seeds",
                "--top",
                "1",
                "--json",
            ],
            e.similar(&SimilarQuery {
                seeds: vec!["writing:2016-essay.md:6".into()],
                in_: vec!["writing".into()],
                top: 1,
                exclude_seeds: true,
            })
            .unwrap(),
        ),
        case(vec!["index", "status", "--json"], e.index_status().unwrap()),
        case(vec!["corpora", "list", "--json"], e.corpora_list().unwrap()),
        case(
            vec![
                "clean-report",
                "writing",
                "--samples",
                "2",
                "--rule",
                "^See ",
                "--json",
            ],
            e.clean_report(&CleanReportQuery {
                corpus: "writing".into(),
                samples: 2,
                rules: vec!["^See ".into()],
            })
            .unwrap(),
        ),
    ];
    for c in cases {
        let o = scout(&cfg, &data, &c.args);
        assert_eq!(o.status.code(), Some(c.code), "{:?}", c.args);
        assert_eq!(String::from_utf8_lossy(&o.stdout), c.json, "{:?}", c.args);
        let err = String::from_utf8_lossy(&o.stderr);
        let notes: Vec<&str> = err.lines().collect();
        assert_eq!(notes, c.notes, "{:?}", c.args);
    }

    // index build: the same reports, bar the timing.
    let strip = |s: &str| {
        let mut v: serde_json::Value = serde_json::from_str(s).unwrap();
        for r in v["reports"].as_array_mut().unwrap() {
            r.as_object_mut().unwrap().remove("elapsed_ms");
        }
        v
    };
    let o = scout(
        &cfg,
        &data,
        &["index", "build", "writing", "--force", "--json"],
    );
    assert!(o.status.success());
    let facade = e
        .index_build(&IndexBuildQuery {
            ids: vec!["writing".into()],
            force: true,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        strip(&String::from_utf8_lossy(&o.stdout)),
        strip(&api::to_json(&facade.body).unwrap())
    );

    // Errors map the same way: the facade's error is the CLI's exit.
    assert!(e
        .kwic(&KwicQuery {
            term: "x".into(),
            scope: scope(&["notes"]),
            ..Default::default()
        })
        .is_err());
    assert_eq!(
        scout(&cfg, &data, &["kwic", "x", "--in", "notes", "--json"])
            .status
            .code(),
        Some(3)
    );
    assert_eq!(before, snapshot(&w), "sources are read-only");
}

/// Semantic and hybrid search: the CLI (with the hash fake embedder,
/// `SCOUT_EMBEDDER=hash`) prints exactly the facade's reply.
#[test]
fn cli_semantic_json_equals_facade_response() {
    let tmp = tempfile::tempdir().unwrap();
    let (cfg, data, w) = fixture(tmp.path());
    let before = snapshot(&w);
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_scout"))
            .args(args)
            .env("SCOUT_CONFIG", &cfg)
            .env("SCOUT_DATA_DIR", &data)
            .env("SCOUT_EMBEDDER", "hash")
            .output()
            .unwrap()
    };
    let o = run(&["index", "build", "writing", "--semantic", "--json"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["reports"][0]["vectors"]["model"], "hash-64");

    let e = Engine::new(Registry::load_from(&cfg).unwrap(), data.join("indexes"))
        .with_embedder(std::sync::Arc::new(scout_corpus::HashEmbedder::default()));
    let q = |query: &str, mode| SearchQuery {
        query: query.into(),
        in_: vec!["writing".into()],
        mode,
        ..Default::default()
    };
    use scout_corpus::SearchMode::*;
    let cases = vec![
        case(
            vec![
                "search",
                "metaphor model",
                "--in",
                "writing",
                "--semantic",
                "--json",
            ],
            e.search(&q("metaphor model", Semantic)).unwrap(),
        ),
        case(
            vec!["search", "metaphor", "--in", "writing", "--json"],
            e.search(&q("metaphor", Auto)).unwrap(),
        ),
        case(
            vec!["search", "metaphor", "--in", "writing", "--fts", "--json"],
            e.search(&q("metaphor", Fts)).unwrap(),
        ),
    ];
    assert!(cases[1].json.contains("\"mode\": \"hybrid\""));
    assert!(!cases[2].json.contains("\"mode\""));
    for c in cases {
        let o = run(&c.args);
        assert_eq!(o.status.code(), Some(c.code), "{:?}", c.args);
        assert_eq!(String::from_utf8_lossy(&o.stdout), c.json, "{:?}", c.args);
        let err = String::from_utf8_lossy(&o.stderr);
        assert_eq!(err.lines().collect::<Vec<_>>(), c.notes, "{:?}", c.args);
    }
    assert_eq!(before, snapshot(&w), "sources are read-only");
}

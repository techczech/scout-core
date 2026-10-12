//! CLI seam for the topic layer: `scout facets` and `scout topics` print
//! exactly the facade's JSON with its exit code, `index build` names the
//! join rate, and the archive folder is never written.

use scout_corpus::api::{self, FacetsQuery, ThemeMonthsQuery, TopicsQuery};
use scout_corpus::{Engine, Outcome, Registry};
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

fn files(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push((p.display().to_string(), fs::read(&p).unwrap()));
            }
        }
    }
    out.sort();
    out
}

fn fixture(tmp: &Path) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    let (h, data) = (tmp.join("archive"), tmp.join("data"));
    for (file, id, quote) in [
        ("a-1.md", "1", "Tao on proofs and models."),
        ("b-2.md", "2", "Astra writes a proof."),
        ("c-3.md", "3", "An untagged proof."),
    ] {
        write(
            &h,
            &format!("readings/works/{file}"),
            &format!(
                "---\ntitle: W{id}\nauthor: A\ntype: tweet\nsource_system: x\nsource_id: \"{id}\"\n---\n\n> {quote}\n\nhighlighted_at: 2026-09-0{id}\n\n---\n"
            ),
        );
    }
    write(
        &h,
        "analysis/topics/current.jsonl",
        concat!(
            r#"{"key":"x:1","path":"readings/works/a-1.md","date":"2026-09-01","ai_related":true,"themes":["AI and mathematics"],"entities":["Terence Tao"],"event":"2026-09/E21","event_name":"Fields declaration"}"#,
            "\n",
            r#"{"key":"x:2","path":"readings/works/b-2.md","date":"2026-09-02","ai_related":true,"themes":["AI and mathematics","Vibe coding"],"entities":["GPT-6 Astra"],"event":"2026-09/E21","event_name":"Fields declaration"}"#,
            "\n",
            r#"{"key":"x:9","path":"readings/works/gone.md","date":"2026-09-09","ai_related":false,"themes":[],"entities":[],"event":null}"#,
            "\n"
        ),
    );
    let cfg = tmp.join("corpora.toml");
    fs::write(
        &cfg,
        format!(
            "[[corpus]]\nid = \"highlights\"\nname = \"H\"\nkind = \"highlight-scout-archive\"\npath = \"{}\"\n",
            h.display()
        ),
    )
    .unwrap();
    (cfg, data, h)
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn facets_and_topics_match_the_facade_and_build_reports_the_join() {
    let tmp = tempfile::tempdir().unwrap();
    let (cfg, data, h) = fixture(tmp.path());
    let before = files(&h);

    let b = scout(&cfg, &data, &["index", "build"]);
    assert!(b.status.success());
    let text = stdout(&b);
    assert!(
        text.contains(
            "topics: 3 items, 2 matched (2 by key, 0 by path), 1 unmatched → 2 tagged documents"
        ),
        "{text}"
    );
    assert!(text.contains("unmatched e.g. x:9"), "{text}");
    let j: serde_json::Value =
        serde_json::from_slice(&scout(&cfg, &data, &["index", "build", "--json"]).stdout).unwrap();
    let t = &j["reports"][0]["topics"];
    assert_eq!(t["matched"], 2);
    assert_eq!(t["unmatched"], 1);
    assert_eq!(t["reloaded"], false);

    let engine = Engine::new(Registry::load_from(&cfg).unwrap(), data.join("indexes"));
    let check = |args: &[&str], json: String, has: bool| {
        let o = scout(&cfg, &data, args);
        assert_eq!(stdout(&o), json + "\n", "{args:?}");
        assert_eq!(o.status.code(), Some(if has { 0 } else { 1 }), "{args:?}");
    };
    let f = engine
        .facets(&FacetsQuery {
            query: "event:2026-09/E21".into(),
            ..Default::default()
        })
        .unwrap()
        .body;
    assert_eq!(f.total_documents, 2);
    check(
        &["facets", "event:2026-09/E21", "--json"],
        api::to_json(&f).unwrap(),
        f.has_results(),
    );
    let f = engine
        .facets(&FacetsQuery {
            query: "proof theme:\"Vibe coding\"".into(),
            top: 1,
            ..Default::default()
        })
        .unwrap()
        .body;
    check(
        &[
            "facets",
            "proof",
            "theme:\"Vibe coding\"",
            "--top",
            "1",
            "--json",
        ],
        api::to_json(&f).unwrap(),
        true,
    );
    let f = engine
        .facets(&FacetsQuery {
            query: "theme:Nothing".into(),
            ..Default::default()
        })
        .unwrap()
        .body;
    check(
        &["facets", "theme:Nothing", "--json"],
        api::to_json(&f).unwrap(),
        false,
    );
    let t = engine
        .topics(&TopicsQuery {
            field: "entities".into(),
            month: Some("2026-09".into()),
            ..Default::default()
        })
        .unwrap()
        .body;
    assert_eq!(t.items.len(), 2);
    check(
        &["topics", "entities", "--month", "2026-09", "--json"],
        api::to_json(&t).unwrap(),
        true,
    );

    // The theme × month matrix: the facade's JSON, with and without a
    // query and bounds.
    let m = engine
        .theme_months(&ThemeMonthsQuery::default())
        .unwrap()
        .body;
    assert_eq!(m.months, ["2026-09"]);
    assert_eq!(m.themes, ["AI and mathematics", "Vibe coding"]);
    assert_eq!(
        (m.counts.clone(), m.month_totals.clone()),
        (vec![vec![2], vec![1]], vec![2])
    );
    check(
        &["topics", "theme-months", "--json"],
        api::to_json(&m).unwrap(),
        true,
    );
    let m = engine
        .theme_months(&ThemeMonthsQuery {
            in_: vec!["highlights".into()],
            from: Some("2026-08".into()),
            to: Some("2026-10".into()),
            query: "Astra theme:\"Vibe coding\"".into(),
        })
        .unwrap()
        .body;
    assert_eq!((m.months.len(), m.documents), (3, 1));
    check(
        &[
            "topics",
            "theme-months",
            "Astra",
            "theme:\"Vibe coding\"",
            "--from",
            "2026-08",
            "--to",
            "2026-10",
            "--in",
            "highlights",
            "--json",
        ],
        api::to_json(&m).unwrap(),
        true,
    );
    let none = engine
        .theme_months(&ThemeMonthsQuery {
            query: "theme:Nothing".into(),
            ..Default::default()
        })
        .unwrap()
        .body;
    check(
        &["topics", "theme-months", "theme:Nothing", "--json"],
        api::to_json(&none).unwrap(),
        false,
    );
    let o = stdout(&scout(&cfg, &data, &["topics", "theme-months"]));
    assert!(
        o.starts_with("2 themes over 1 months from 2026-09 to 2026-09, 2 tagged documents"),
        "{o}"
    );
    assert!(o.contains("  2  AI and mathematics  2026-09:2"), "{o}");
    for bad in [
        vec!["topics", "theme-months", "--from", "2026"],
        vec!["topics", "theme-months", "--month", "2026-09"],
        vec!["topics", "themes", "--from", "2026-09"],
        vec!["topics", "themes", "a-query"],
    ] {
        let o = scout(&cfg, &data, &bad);
        assert_eq!(o.status.code(), Some(2), "{bad:?}");
        assert!(o.stdout.is_empty(), "{bad:?}");
    }

    // Human output.
    let o = stdout(&scout(&cfg, &data, &["facets", "event:2026-09/E21"]));
    assert!(o.starts_with("2 documents (2 tagged)"), "{o}");
    assert!(o.contains("  2  2026-09/E21  Fields declaration"), "{o}");
    let o = stdout(&scout(&cfg, &data, &["topics", "themes"]));
    assert!(o.contains("2  AI and mathematics"), "{o}");
    // Usage errors exit 2.
    assert_eq!(
        scout(&cfg, &data, &["topics", "colours"]).status.code(),
        Some(2)
    );
    assert_eq!(
        scout(&cfg, &data, &["search", "ai:maybe"]).status.code(),
        Some(2)
    );
    assert_eq!(files(&h), before, "archive untouched");
}

#[test]
fn theme_months_refuses_a_month_field_in_its_query_with_exit_2() {
    let tmp = tempfile::tempdir().unwrap();
    let (cfg, data, _) = fixture(tmp.path());
    assert!(scout(&cfg, &data, &["index", "build"]).status.success());
    for args in [
        vec!["topics", "theme-months", "month:2026-09"],
        vec!["topics", "theme-months", "month:2026-09", "--json"],
        vec![
            "topics",
            "theme-months",
            "proof",
            "month:2026",
            "--from",
            "2026-09",
        ],
    ] {
        let o = scout(&cfg, &data, &args);
        assert_eq!(o.status.code(), Some(2), "{args:?}");
        assert!(o.stdout.is_empty(), "{args:?}");
        let err = String::from_utf8_lossy(&o.stderr);
        assert!(
            err.contains("in the query") && err.contains("use --from and --to"),
            "{args:?}: {err}"
        );
    }
    // The same months by the bounds, and any other field, are taken.
    let o = scout(
        &cfg,
        &data,
        &[
            "topics",
            "theme-months",
            "ai:true",
            "--from",
            "2026-09",
            "--to",
            "2026-09",
        ],
    );
    assert_eq!(o.status.code(), Some(0), "{o:?}");
}

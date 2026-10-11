//! CLI seam for lasting events: `scout events` and `scout facets
//! --min-event-items` print exactly the facade's JSON with its exit code,
//! `index build` names the records and every file it left out, and the
//! archive folder is never written.

use scout_corpus::api::{self, EventsQuery, FacetsQuery};
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

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn events_and_the_facet_floor_match_the_facade_and_build_reports_the_records() {
    let tmp = tempfile::tempdir().unwrap();
    let (h, data) = (tmp.path().join("archive"), tmp.path().join("data"));
    for (id, quote) in [
        ("1", "Tao on proofs."),
        ("2", "A proof by Astra."),
        ("3", "A launch, no proof."),
    ] {
        write(
            &h,
            &format!("readings/works/w-{id}.md"),
            &format!(
                "---\ntitle: W{id}\nauthor: A\ntype: tweet\nsource_system: x\nsource_id: \"{id}\"\n---\n\n> {quote}\n\nhighlighted_at: 2026-09-0{id}\n\n---\n"
            ),
        );
    }
    write(
        &h,
        "analysis/topics/current.jsonl",
        concat!(
            r#"{"key":"x:1","date":"2026-09-01","themes":[],"entities":[],"event":"2026-09/E21","event_name":"Fields declaration"}"#,
            "\n",
            r#"{"key":"x:2","date":"2026-10-02","themes":[],"entities":[],"event":"2026-10/E04","event_name":"Follow-up"}"#,
            "\n",
            r#"{"key":"x:3","date":"2026-09-03","themes":[],"entities":[],"event":"2026-09/E02","event_name":"A launch"}"#,
            "\n"
        ),
    );
    write(
        &h,
        "analysis/events/2026-09-fields-declaration.md",
        "---\nid: 2026-09-fields-declaration\nname: \"Fields: the declaration\"\nstart: 2026-09-01\nend: 2026-10-02\nmembers:\n  - 2026-09/E21\n  - 2026-10/E04\norigin: model\nedited:\nmerged_into:\n---\n\nTwo months, one story.\n",
    );
    write(
        &h,
        "analysis/events/draft.md",
        "---\nname: No id yet\n---\n",
    );
    let cfg = tmp.path().join("corpora.toml");
    fs::write(
        &cfg,
        format!(
            "[[corpus]]\nid = \"highlights\"\nname = \"H\"\nkind = \"highlight-scout-archive\"\npath = \"{}\"\n",
            h.display()
        ),
    )
    .unwrap();
    let before = files(&h);

    let b = scout(&cfg, &data, &["index", "build"]);
    assert!(b.status.success());
    let text = stdout(&b);
    assert!(
        text.contains("events: 1 records (0 merged), 2 monthly events held, 1 files skipped"),
        "{text}"
    );
    assert!(
        text.contains("warning: analysis/events/draft.md: no id; skipped"),
        "{text}"
    );
    let j: serde_json::Value =
        serde_json::from_slice(&scout(&cfg, &data, &["index", "build", "--json"]).stdout).unwrap();
    let e = &j["reports"][0]["topics"]["events"];
    assert_eq!(e["records"], 1);
    assert_eq!(e["skipped"], 1);
    assert_eq!(e["warnings"][0], "analysis/events/draft.md: no id; skipped");

    let engine = Engine::new(Registry::load_from(&cfg).unwrap(), data.join("indexes"));
    let check = |args: &[&str], json: String, has: bool| {
        let o = scout(&cfg, &data, args);
        assert_eq!(stdout(&o), json + "\n", "{args:?}");
        assert_eq!(o.status.code(), Some(if has { 0 } else { 1 }), "{args:?}");
    };
    let l = engine.events(&EventsQuery::default()).unwrap().body;
    assert_eq!(l.events.len(), 2);
    assert_eq!(l.events[0].items, 2);
    check(&["events", "--json"], api::to_json(&l).unwrap(), true);
    let l = engine
        .events(&EventsQuery {
            month: Some("2026-10".into()),
            min_items: 2,
            merged: true,
            top: 5,
            ..Default::default()
        })
        .unwrap()
        .body;
    assert_eq!(l.events[0].month_items, Some(1));
    check(
        &[
            "events",
            "--month",
            "2026-10",
            "--min-items",
            "2",
            "--merged",
            "--top",
            "5",
            "--json",
        ],
        api::to_json(&l).unwrap(),
        true,
    );
    let none = engine
        .events(&EventsQuery {
            min_items: 9,
            ..Default::default()
        })
        .unwrap()
        .body;
    check(
        &["events", "--min-items", "9", "--json"],
        api::to_json(&none).unwrap(),
        none.has_results(),
    );
    assert!(!none.has_results());
    assert_eq!(
        scout(&cfg, &data, &["events", "--month", "soon"])
            .status
            .code(),
        Some(2)
    );

    // The old monthly id finds the whole lasting event.
    let f = engine
        .facets(&FacetsQuery {
            query: "event:2026-10/E04".into(),
            min_event_items: 2,
            ..Default::default()
        })
        .unwrap()
        .body;
    assert_eq!(f.total_documents, 2);
    assert_eq!(f.facets.event.items[0].value, "2026-09-fields-declaration");
    check(
        &[
            "facets",
            "event:2026-10/E04",
            "--min-event-items",
            "2",
            "--json",
        ],
        api::to_json(&f).unwrap(),
        true,
    );
    let f = engine
        .facets(&FacetsQuery {
            query: "launch".into(),
            min_event_items: 2,
            ..Default::default()
        })
        .unwrap()
        .body;
    assert!(f.facets.event.items.is_empty());
    check(
        &["facets", "launch", "--min-event-items", "2", "--json"],
        api::to_json(&f).unwrap(),
        true,
    );

    // Human output.
    let o = stdout(&scout(&cfg, &data, &["events"]));
    assert!(o.starts_with("2 events (highlights)\n"), "{o}");
    assert!(
        o.contains(
            "  2  2026-09-fields-declaration  Fields: the declaration  2026-09-01 to 2026-10-02"
        ),
        "{o}"
    );
    assert!(o.contains("  1  2026-09/E02  A launch"), "{o}");
    let o = stdout(&scout(&cfg, &data, &["facets", "proof"]));
    assert!(
        o.contains("2  2026-09-fields-declaration  Fields: the declaration"),
        "{o}"
    );
    assert_eq!(files(&h), before, "the archive folder is never written");
}

#[test]
fn records_without_any_tags_are_built_reported_and_listed() {
    let tmp = tempfile::tempdir().unwrap();
    let (h, data) = (tmp.path().join("archive"), tmp.path().join("data"));
    write(
        &h,
        "readings/works/w-1.md",
        "---\ntitle: W1\nauthor: A\ntype: tweet\nsource_system: x\nsource_id: \"1\"\n---\n\n> A proof.\n\nhighlighted_at: 2026-09-01\n\n---\n",
    );
    write(
        &h,
        "analysis/events/2026-09-fields-declaration.md",
        "---\nid: 2026-09-fields-declaration\nname: The declaration\nstart: 2026-09-01\nend: 2026-09-30\nmembers: [2026-09/E21]\nthemes: []\n---\n\nNo items yet.\n",
    );
    write(
        &h,
        "analysis/events/draft.md",
        "---\nname: No id yet\n---\n",
    );
    let cfg = tmp.path().join("corpora.toml");
    fs::write(
        &cfg,
        format!(
            "[[corpus]]\nid = \"highlights\"\nname = \"H\"\nkind = \"highlight-scout-archive\"\npath = \"{}\"\n",
            h.display()
        ),
    )
    .unwrap();
    let before = files(&h);

    let text = stdout(&scout(&cfg, &data, &["index", "build"]));
    assert!(
        text.contains("events: 1 records (0 merged), 1 monthly events held, 1 files skipped"),
        "{text}"
    );
    assert!(
        text.contains("warning: analysis/events/draft.md: no id; skipped"),
        "{text}"
    );
    assert!(!text.contains("topics:"), "{text}");
    let j: serde_json::Value =
        serde_json::from_slice(&scout(&cfg, &data, &["index", "build", "--json"]).stdout).unwrap();
    let r = &j["reports"][0];
    assert!(r.get("topics").is_none(), "{r}");
    assert_eq!(r["events"]["records"], 1);
    assert_eq!(r["events"]["skipped"], 1);

    let o = scout(&cfg, &data, &["events", "--min-items", "0", "--json"]);
    assert_eq!(o.status.code(), Some(0));
    let l: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(l["distinct"], 1);
    let e = &l["events"][0];
    assert_eq!(e["id"], "2026-09-fields-declaration");
    assert_eq!(e["record"], true);
    assert_eq!(e["items"], 0);
    assert_eq!(e["name"], "The declaration");
    assert_eq!(e["description"], "No items yet.");
    // The default floor is one item, so nothing is listed without it.
    assert_eq!(
        scout(&cfg, &data, &["events", "--json"]).status.code(),
        Some(1)
    );
    assert_eq!(files(&h), before, "the archive folder is never written");
}

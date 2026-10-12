//! CLI seam for the filter panel's engine work: `scout search --sort`,
//! `scout facets` without a query, repeated topic fields, the month bounds
//! of `theme-months`, and a quoted query on the terminal. Each `--json`
//! reply is the facade's, with its exit code.

use scout_corpus::api::{self, FacetsQuery, SearchQuery, Sort};
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

/// Three notes (two dated, one not), all tagged; `a` and `b` share a theme.
fn fixture(tmp: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let (notes, data) = (tmp.join("notes"), tmp.join("data"));
    for (file, date, text) in [
        ("a.md", "date: 2026-02-10\n", "A proof about agents."),
        ("b.md", "date: 2025-06-01\n", "Agents write code."),
        ("c.md", "", "An undated proof."),
    ] {
        write(
            &notes,
            file,
            &format!("---\ntitle: {file}\nauthor: A\n{date}---\n\n{text}\n"),
        );
    }
    write(
        &notes,
        "analysis/topics/current.jsonl",
        concat!(
            r#"{"key":"n:a","path":"a.md","date":"2026-02-10","ai_related":true,"themes":["Maths","Coding"],"entities":["Tao"],"entities_typed":[{"name":"Tao","type":"person"}],"event":null}"#,
            "\n",
            r#"{"key":"n:b","path":"b.md","date":"2026-03-02","ai_related":true,"themes":["Coding"],"entities":[],"event":null}"#,
            "\n",
            r#"{"key":"n:c","path":"c.md","date":"2026-03-09","ai_related":false,"themes":["Maths"],"entities":["Tao"],"event":null}"#,
            "\n"
        ),
    );
    let cfg = tmp.join("corpora.toml");
    fs::write(
        &cfg,
        format!(
            "[[corpus]]\nid = \"notes\"\nname = \"N\"\nkind = \"markdown-folder\"\npath = \"{}\"\nexclude = [\"analysis/**\"]\n",
            notes.display()
        ),
    )
    .unwrap();
    assert!(scout(&cfg, &data, &["index", "build"]).status.success());
    (cfg, data)
}

fn stdout(o: &Output) -> String {
    String::from_utf8(o.stdout.clone()).unwrap()
}

fn files(o: &Output) -> Vec<String> {
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    v["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["rel_path"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn search_sort_orders_by_date_lists_without_a_term_and_refuses_what_it_cannot_do() {
    let tmp = tempfile::tempdir().unwrap();
    let (cfg, data) = fixture(tmp.path());
    let e = Engine::new(Registry::load_from(&cfg).unwrap(), data.join("indexes"));
    let run = |args: &[&str]| scout(&cfg, &data, args);

    // Newest and oldest, with no term and no filter: every document.
    for (sort, name, want) in [
        (Sort::Newest, "newest", ["a.md", "b.md", "c.md"]),
        (Sort::Oldest, "oldest", ["b.md", "a.md", "c.md"]),
    ] {
        for query in ["", "in:notes"] {
            let o = run(&["search", query, "--sort", name, "--json"]);
            assert_eq!(o.status.code(), Some(0), "{name} {query:?}: {o:?}");
            assert_eq!(files(&o), want, "{name} {query:?}");
            let reply = e
                .search(&SearchQuery {
                    query: query.into(),
                    sort,
                    ..Default::default()
                })
                .unwrap();
            assert_eq!(
                stdout(&o),
                api::to_json(&reply.body).unwrap() + "\n",
                "{name}"
            );
        }
    }
    // No query argument at all is the empty query.
    assert_eq!(
        files(&run(&["search", "--sort", "newest", "--json"])).len(),
        3
    );
    // Filters alone, repeated fields, and paging by --limit.
    let o = run(&["search", "theme:Maths", "--sort", "oldest", "--json"]);
    assert_eq!(files(&o), ["a.md", "c.md"]);
    let o = run(&[
        "search",
        "theme:Maths theme:Coding",
        "--sort",
        "newest",
        "--json",
    ]);
    assert_eq!(files(&o), ["a.md"]);
    let o = run(&[
        "search",
        "month:2026-02 month:2026-03",
        "--sort",
        "oldest",
        "--limit",
        "2",
        "--json",
    ]);
    assert_eq!(files(&o), ["b.md", "a.md"]);
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(
        (v["total_documents"].as_u64(), &v["sort"]),
        (Some(3), &"oldest".into())
    );
    // Nothing found is exit 1, also in a date order.
    let o = run(&["search", "theme:Nothing", "--sort", "newest", "--json"]);
    assert_eq!(o.status.code(), Some(1));
    // The human listing is the same documents in the same order.
    let o = run(&["search", "--sort", "oldest"]);
    let text = stdout(&o);
    let at = |f: &str| {
        text.find(&format!("[notes] {f}"))
            .unwrap_or_else(|| panic!("{text}"))
    };
    assert!(at("b.md") < at("a.md") && at("a.md") < at("c.md"), "{text}");

    // Relevance: the default, named or not, byte for byte; no `sort` key.
    let named = run(&["search", "proof", "--sort", "relevance", "--json"]);
    let plain = run(&["search", "proof", "--json"]);
    assert_eq!(named.status.code(), Some(0));
    assert_eq!(named.stdout, plain.stdout);
    assert!(!stdout(&plain).contains("\"sort\""));
    let reply = e
        .search(&SearchQuery {
            query: "proof".into(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(stdout(&plain), api::to_json(&reply.body).unwrap() + "\n");

    // Exit 2: an empty query by relevance (as before), an unknown order,
    // a date order with a vector ranking.
    for (args, why) in [
        (vec!["search", "", "--json"], "empty query"),
        (vec!["search", "--sort", "relevance"], "empty query"),
        (
            vec!["search", "proof", "--sort", "latest"],
            "unknown sort \"latest\": use relevance, newest or oldest",
        ),
        (
            vec!["search", "proof", "--sort", "Newest"],
            "unknown sort \"Newest\"",
        ),
        (
            vec!["search", "proof", "--sort", "newest", "--semantic"],
            "does not combine with semantic or hybrid",
        ),
        (
            vec!["search", "proof", "--sort", "oldest", "--hybrid"],
            "does not combine with semantic or hybrid",
        ),
        (
            vec!["search", "theme:A theme:", "--sort", "newest"],
            "theme: needs a value",
        ),
    ] {
        let o = run(&args);
        assert_eq!(o.status.code(), Some(2), "{args:?}: {o:?}");
        assert!(o.stdout.is_empty(), "{args:?}");
        let err = String::from_utf8_lossy(&o.stderr);
        assert!(err.contains(why), "{args:?}: {err}");
    }
    // A corpus alone by relevance still lists nothing: exit 1.
    assert_eq!(
        run(&["search", "in:notes", "--json"]).status.code(),
        Some(1)
    );
    // --fts with a date order is the same search.
    let o = run(&["search", "proof", "--sort", "newest", "--fts", "--json"]);
    assert_eq!(files(&o), ["a.md", "c.md"]);
}

#[test]
fn facets_take_no_query_and_count_a_picked_month_beside_the_others() {
    let tmp = tempfile::tempdir().unwrap();
    let (cfg, data) = fixture(tmp.path());
    let e = Engine::new(Registry::load_from(&cfg).unwrap(), data.join("indexes"));
    for args in [
        vec!["facets", "--json"],
        vec!["facets", "", "--json"],
        vec!["facets", "in:notes", "--json"],
    ] {
        let o = scout(&cfg, &data, &args);
        assert_eq!(o.status.code(), Some(0), "{args:?}: {o:?}");
        let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
        assert_eq!(v["total_documents"], 3, "{args:?}");
        assert_eq!(v["tagged_documents"], 3, "{args:?}");
        assert_eq!(
            v["facets"]["entity"]["items"],
            serde_json::json!([{"value": "Tao", "count": 2, "type": "person"}]),
            "{args:?}"
        );
    }
    let query = "month:2026-02 theme:Coding theme:Maths";
    let o = scout(&cfg, &data, &["facets", query, "--json"]);
    let reply = e
        .facets(&FacetsQuery {
            query: query.into(),
            ..Default::default()
        })
        .unwrap();
    assert!(reply.body.has_results());
    assert_eq!(stdout(&o), api::to_json(&reply.body).unwrap() + "\n");
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["total_documents"], 1);
    // The plan: the string keys hold the last value, the lists them all.
    assert_eq!(
        v["plan"]["filters"],
        serde_json::json!({"theme": "Maths", "themes": ["Coding", "Maths"], "month": "2026-02"})
    );
    // Only one document carries both themes, in February; March is listed
    // with what it holds under the two themes: nothing, so not listed.
    assert_eq!(
        v["facets"]["month"]["items"],
        serde_json::json!([{"value": "2026-02", "count": 1}])
    );
    let o = scout(
        &cfg,
        &data,
        &["facets", "month:2026-02", "theme:Coding", "--json"],
    );
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(
        v["facets"]["month"]["items"],
        serde_json::json!([{"value": "2026-02", "count": 1}, {"value": "2026-03", "count": 1}])
    );
    assert_eq!(v["total_documents"], 1);
    // The human view of an empty query names it as one.
    let o = scout(&cfg, &data, &["facets"]);
    assert!(stdout(&o).starts_with("3 documents (3 tagged) for \"\" in notes\n"));
}

#[test]
fn theme_months_bounds_are_exactly_a_month_and_a_month_field_is_still_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let (cfg, data) = fixture(tmp.path());
    for bad in [
        "2026", "2026   ", "   2026", " 2026  ", "2026-13", "2026-2", " 2026-02", "",
    ] {
        for flag in ["--from", "--to"] {
            let o = scout(
                &cfg,
                &data,
                &["topics", "theme-months", flag, bad, "--json"],
            );
            assert_eq!(o.status.code(), Some(2), "{flag} {bad:?}: {o:?}");
            assert!(o.stdout.is_empty());
            let err = String::from_utf8_lossy(&o.stderr);
            assert!(
                err.contains("is not a month YYYY-MM"),
                "{flag} {bad:?}: {err}"
            );
        }
    }
    for query in [
        "month:2026-02",
        "month:2026-02 month:2026-03",
        "theme:Maths month:2026",
    ] {
        let o = scout(&cfg, &data, &["topics", "theme-months", query]);
        assert_eq!(o.status.code(), Some(2), "{query}");
        assert!(String::from_utf8_lossy(&o.stderr).contains("use --from and --to"));
    }
    let o = scout(
        &cfg,
        &data,
        &[
            "topics",
            "theme-months",
            "theme:Maths theme:Coding",
            "--from",
            "2026-02",
            "--to",
            "2026-03",
            "--json",
        ],
    );
    assert_eq!(o.status.code(), Some(0), "{o:?}");
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["month_totals"], serde_json::json!([1, 0]));
}

#[test]
fn a_quoted_query_keeps_joiners_tabs_and_no_break_spaces_as_they_are() {
    let tmp = tempfile::tempdir().unwrap();
    let (cfg, data) = fixture(tmp.path());
    // A family emoji (zero width joiners), a tab, a no-break space, the
    // zero width non-joiner, and a quote and a backslash of its own.
    let query = "theme:\"\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}\ta\u{a0}b \u{645}\u{6cc}\u{200c}\u{62e} \\\"x\\\\\"";
    let o = scout(&cfg, &data, &["facets", query]);
    assert_eq!(o.status.code(), Some(1));
    let text = stdout(&o);
    let shown = query.replace('\\', "\\\\").replace('"', "\\\"");
    assert_eq!(
        text.lines().next().unwrap(),
        format!("0 documents (0 tagged) for \"{shown}\" in notes")
    );
    assert!(!text.contains("\\u{") && !text.contains("\\t"), "{text}");
    // What acts on a terminal is still replaced, one for one.
    let o = scout(&cfg, &data, &["facets", "theme:a\u{1b}[31m\u{202e}b"]);
    assert!(stdout(&o)
        .starts_with("0 documents (0 tagged) for \"theme:a\u{fffd}[31m\u{fffd}b\" in notes"));
}

/// A query that is not empty and not only `in:` terms, yet selects nothing
/// by itself (`OR`, `i:`, `!`, `tag:""`), is not the empty query: `facets`
/// counts no document and a search in date order lists none, as before an
/// empty query counted or listed anything. `theme-months` gives the matrix
/// of no query, as it always did (each cell is the query with a month
/// added). The empty query and `in:` alone still stand for the corpus.
#[test]
fn a_query_that_selects_nothing_is_not_the_empty_query() {
    let tmp = tempfile::tempdir().unwrap();
    let (cfg, data) = fixture(tmp.path());
    let run = |args: &[&str]| scout(&cfg, &data, args);
    let json = |o: &Output| -> serde_json::Value { serde_json::from_slice(&o.stdout).unwrap() };

    let whole_matrix = json(&run(&["topics", "theme-months", "--json"]));
    assert_eq!(whole_matrix["theme_totals"], serde_json::json!([2, 2]));
    for query in [
        "OR",
        "i:",
        "!",
        "tag:\"\"",
        "OR i: !",
        "in:notes OR",
        "i: in:notes",
    ] {
        let o = run(&["facets", query, "--json"]);
        assert_eq!(o.status.code(), Some(1), "facets {query:?}: {o:?}");
        let f = json(&o);
        assert_eq!(f["total_documents"], 0, "facets {query:?}");
        assert_eq!(f["tagged_documents"], 0, "facets {query:?}");
        assert_eq!(f["facets"]["theme"]["distinct"], 0, "facets {query:?}");
        assert_eq!(
            f["facets"]["ai"],
            serde_json::json!({"true": 0, "false": 0})
        );
        let o = run(&["facets", query]);
        assert_eq!(o.status.code(), Some(1), "facets {query:?} (text)");

        let plain = run(&["search", query, "--json"]);
        assert_eq!(plain.status.code(), Some(1), "search {query:?}: {plain:?}");
        for sort in ["newest", "oldest"] {
            let o = run(&["search", query, "--sort", sort, "--json"]);
            assert_eq!(o.status.code(), Some(1), "search {query:?} {sort}: {o:?}");
            let r = json(&o);
            assert_eq!(r["total_documents"], 0, "search {query:?} {sort}");
            assert!(files(&o).is_empty(), "search {query:?} {sort}");
        }

        let o = run(&["topics", "theme-months", query, "--json"]);
        assert_eq!(o.status.code(), Some(0), "theme-months {query:?}: {o:?}");
        let m = json(&o);
        for key in ["months", "themes", "counts", "theme_totals", "month_totals"] {
            assert_eq!(m[key], whole_matrix[key], "theme-months {query:?} {key}");
        }
    }
    // With one more term such a query is that term's query.
    let o = run(&["facets", "OR theme:Maths", "--json"]);
    assert_eq!(json(&o)["total_documents"], 2);
    // A month picked beside it: the month facet still lists the others.
    let f = json(&run(&["facets", "OR month:2026-02", "--json"]));
    assert_eq!(f["total_documents"], 1);
    assert_eq!(f["facets"]["month"]["distinct"], 2);

    // The empty query and corpora alone: the whole corpus, as documented.
    for query in ["", "  ", "in:notes"] {
        let o = run(&["facets", query, "--json"]);
        assert_eq!(o.status.code(), Some(0), "facets {query:?}: {o:?}");
        assert_eq!(json(&o)["total_documents"], 3, "facets {query:?}");
        let o = run(&["search", query, "--sort", "newest", "--json"]);
        assert_eq!(o.status.code(), Some(0), "search {query:?}: {o:?}");
        assert_eq!(files(&o).len(), 3, "search {query:?}");
    }
}

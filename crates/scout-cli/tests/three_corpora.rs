//! CLI seam over the three corpus kinds: `index build` / `index status` for
//! writing, tweets and a highlights archive; `search` with `in:` and
//! `--in` across corpora; `scout cite <passage-id>`.

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

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn three_corpora_build_search_and_cite() {
    let tmp = tempfile::tempdir().unwrap();
    let (w, t, h, data) = (
        tmp.path().join("writing"),
        tmp.path().join("tweets"),
        tmp.path().join("archive"),
        tmp.path().join("data"),
    );
    write(
        &w,
        "essay.md",
        "---\ntitle: Generative metaphor\ndate: 2016-06-23\ngenre: essay\n---\nA generative metaphor frames a problem.\n",
    );
    write(
        &t,
        "stream/2025-07.md",
        "---\ntitle: Stream\nvenue: \"Twitter/X (@techczech)\"\n---\n\n## 2025-07-26T10:12:00Z — tweet 1949000000000000001 · kind: original\n\n<!-- tweet id=\"1949000000000000001\" -->\n~~~\nEvery metaphor is a model.\n~~~\n",
    );
    write(
        &h,
        "readings/works/lakoff-abc.md",
        "---\ntitle: Metaphors We Live By\nauthor: George Lakoff\ntype: book\nsource_system: zotero\nsource_id: \"ABC\"\nsource_data: {\"date\":\"1980-00-00 1980\"}\n---\n\n> Metaphor is pervasive in everyday life.\n\nhighlighted_at: 2024-01-15 | tags: linguistics\n\n---\n",
    );
    let cfg = tmp.path().join("corpora.toml");
    fs::write(
        &cfg,
        format!(
            "[[corpus]]\nid = \"writing\"\nname = \"W\"\nkind = \"markdown-folder\"\npath = \"{}\"\nrequire_frontmatter = [\"genre\"]\ndefault_author = \"Dominik Lukeš\"\nlink = \"writeflex://open?path={{path}}&line={{line}}\"\n\n\
             [[corpus]]\nid = \"tweets\"\nname = \"T\"\nkind = \"markdown-folder\"\npath = \"{}\"\ndefault_author = \"Dominik Lukeš\"\ndocument_unit = \"tweet\"\nlink = \"writeflex://open?path={{path}}&line={{line}}\"\n\n\
             [[corpus]]\nid = \"highlights\"\nname = \"H\"\nkind = \"highlight-scout-archive\"\npath = \"{}\"\n",
            w.display(),
            t.display(),
            h.display()
        ),
    )
    .unwrap();

    // Status before any build names the build command for every kind.
    let o = scout(&cfg, &data, &["index", "status"]);
    assert_eq!(stdout(&o).matches("run `scout index build").count(), 3);

    let o = scout(&cfg, &data, &["index", "build"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let status = stdout(&scout(&cfg, &data, &["index", "status"]));
    for row in ["writing", "tweets", "highlights"] {
        let line = status.lines().find(|l| l.starts_with(row)).unwrap();
        assert!(line.trim_end().ends_with("none"), "{line}");
        assert!(line.split_whitespace().nth(1).unwrap() == "1", "{line}");
    }

    // --in over all three: one document from each, merged.
    let o = scout(
        &cfg,
        &data,
        &[
            "search",
            "metaphor",
            "--in",
            "writing,tweets,highlights",
            "--json",
        ],
    );
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    let corpora: Vec<&str> = v["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["corpus"].as_str().unwrap())
        .collect();
    assert_eq!(corpora.len(), 3);
    for c in ["writing", "tweets", "highlights"] {
        assert!(corpora.contains(&c), "{corpora:?}");
    }
    // `in:` in the query selects without --in (and prints no note).
    let o = scout(&cfg, &data, &["search", "metaphor in:highlights", "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["corpora"], serde_json::json!(["highlights"]));
    assert_eq!(v["results"][0]["date_source"], "published");
    let hl_id = v["results"][0]["hits"][0]["passage_id"]
        .as_str()
        .unwrap()
        .to_string();

    // cite: markdown (default) and plain, from a passage id.
    let o = scout(&cfg, &data, &["cite", &hl_id]);
    assert!(o.status.success());
    let md = stdout(&o);
    assert!(md.starts_with("> Metaphor is pervasive in everyday life.\n\n— George Lakoff, *Metaphors We Live By*, 1980 · [highlight](file:///"), "{md}");
    let o = scout(
        &cfg,
        &data,
        &[
            "cite",
            "tweets:stream/2025-07.md#1949000000000000001:10",
            "--format",
            "plain",
        ],
    );
    assert_eq!(
        stdout(&o),
        "“Every metaphor is a model.”\n— Dominik Lukeš (@techczech), tweet, 26 July 2025 · public: https://x.com/techczech/status/1949000000000000001\n"
    );
    let o = scout(&cfg, &data, &["cite", &hl_id, "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(v["schema_version"], 1);
    assert_eq!(v["quote"], "Metaphor is pervasive in everyday life.");
    // No such passage: exit 1; a malformed id: exit 2.
    assert_eq!(
        scout(&cfg, &data, &["cite", "tweets:stream/2025-07.md:99"])
            .status
            .code(),
        Some(1)
    );
    assert_eq!(
        scout(&cfg, &data, &["cite", "nonsense"]).status.code(),
        Some(2)
    );
}

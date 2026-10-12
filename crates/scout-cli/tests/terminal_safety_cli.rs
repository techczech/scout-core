//! Terminal safety of the human (non-JSON) output: a string from an
//! archive file never reaches the terminal as a control sequence. The
//! theme, entity and event names, a key and a quote of the fixture hold
//! ESC, a C1 control (CSI), a line separator, a carriage return and a
//! line feed; the text views print U+FFFD in their place and the JSON
//! output is as it was (`serde_json` escapes the C0 controls itself).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const ESC: char = '\u{1b}';
const CSI: char = '\u{9b}';
const LSEP: char = '\u{2028}';
const FFFD: char = '\u{fffd}';

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

/// The names as the archive holds them.
fn theme() -> String {
    format!("Maths{ESC}[31m red{CSI}2J{LSEP}end")
}

fn entity() -> String {
    format!("Tao{ESC}]0;owned\u{7}")
}

fn event_name() -> String {
    format!("Fields{ESC}[1A\rdeclaration\nline two")
}

fn fixture(tmp: &Path) -> (PathBuf, PathBuf) {
    let (h, data) = (tmp.join("archive"), tmp.join("data"));
    for (n, quote) in [
        (1, format!("A proof{ESC}[2J about models.")),
        (2, "Another proof.".to_string()),
    ] {
        write(
            &h,
            &format!("readings/works/w-{n}.md"),
            &format!(
                "---\ntitle: W{n}\nauthor: A\ntype: tweet\nsource_system: x\nsource_id: \"{n}\"\n---\n\n> {quote}\n\nhighlighted_at: 2026-09-0{n}\n\n---\n"
            ),
        );
    }
    let row = |n: u32| {
        serde_json::json!({
            "key": format!("x:{n}"), "path": format!("readings/works/w-{n}.md"),
            "date": format!("2026-09-0{n}"), "ai_related": true,
            "themes": [theme()], "entities": [entity()],
            "event": "2026-09/E21", "event_name": event_name(),
        })
        .to_string()
    };
    // A key no document has: `index build` names it as unmatched.
    let gone = serde_json::json!({
        "key": format!("x:9{ESC}[5m"), "path": "readings/works/gone.md",
        "date": "2026-09-09", "ai_related": false, "themes": [], "entities": [], "event": null,
    })
    .to_string();
    write(
        &h,
        "analysis/topics/current.jsonl",
        &format!("{}\n{}\n{gone}\n", row(1), row(2)),
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
    (cfg, data)
}

/// Nothing a terminal acts on is in `text`, but the line feeds and tabs
/// the view itself writes.
fn assert_safe(text: &str, what: &str) {
    for c in text.chars() {
        let acts = (c.is_control() && c != '\n' && c != '\t') || c == LSEP || c == '\u{2029}';
        assert!(!acts, "{what}: U+{:04X} in {text:?}", c as u32);
    }
}

#[test]
fn a_name_with_an_escape_is_shown_safe_in_text_and_escaped_in_json() {
    let tmp = tempfile::tempdir().unwrap();
    let (cfg, data) = fixture(tmp.path());
    let safe_theme = format!("Maths{FFFD}[31m red{FFFD}2J{FFFD}end");
    let safe_entity = format!("Tao{FFFD}]0;owned{FFFD}");
    let safe_event = format!("Fields{FFFD}[1A{FFFD}declaration{FFFD}line two");

    let build = scout(&cfg, &data, &["index", "build"]);
    assert!(build.status.success(), "{build:?}");
    let text = String::from_utf8(build.stdout).unwrap();
    assert_safe(&text, "index build");
    assert!(
        text.contains(&format!("unmatched e.g. x:9{FFFD}[5m")),
        "{text}"
    );

    // Every human view that prints a name, an id or a quote of the files.
    let views: [(&[&str], &[&str]); 9] = [
        (&["topics", "themes"], &[&safe_theme]),
        (&["topics", "entities"], &[&safe_entity]),
        (&["topics", "events"], &[&safe_event]),
        (&["topics", "theme-months"], &[&safe_theme]),
        (
            &["facets", "proof"],
            &[&safe_theme, &safe_entity, &safe_event],
        ),
        (&["events"], &[&safe_event]),
        (&["search", "models"], &["A proof\u{fffd}[2J about models."]),
        (&["kwic", "proof"], &["\u{fffd}[2J about models."]),
        (&["verify-quote", "about models"], &["> about models"]),
    ];
    for (args, wants) in views {
        let o = scout(&cfg, &data, args);
        assert_eq!(o.status.code(), Some(0), "{args:?}: {o:?}");
        let text = String::from_utf8(o.stdout).unwrap();
        assert_safe(&text, &args.join(" "));
        for want in wants {
            assert!(text.contains(want), "{args:?}: want {want:?} in {text}");
        }
        // One line per row still: the line feed of a name starts none.
        assert!(!text.contains("\nline two"), "{args:?}: {text}");
        assert_safe(&String::from_utf8(o.stderr).unwrap(), "stderr");
    }

    // An error that names what it was given: stderr is a terminal too.
    let bad = format!("no{ESC}[31msuch");
    let o = scout(&cfg, &data, &["topics", "themes", "--in", &bad]);
    assert_ne!(o.status.code(), Some(0));
    let err = String::from_utf8(o.stderr).unwrap();
    assert!(!err.trim().is_empty());
    assert_safe(&err, "an error");

    // JSON: as it was. ESC is the six characters `\u001b`, no raw
    // control byte is in the output, and the value read back is the
    // archive's own string.
    for (args, at, want) in [
        (
            &["topics", "themes", "--json"][..],
            "/items/0/value",
            theme(),
        ),
        (
            &["topics", "entities", "--json"][..],
            "/items/0/value",
            entity(),
        ),
        (
            &["topics", "events", "--json"][..],
            "/items/0/name",
            event_name(),
        ),
        (
            &["topics", "theme-months", "--json"][..],
            "/themes/0",
            theme(),
        ),
        (
            &["facets", "proof", "--json"][..],
            "/facets/theme/items/0/value",
            theme(),
        ),
    ] {
        let o = scout(&cfg, &data, args);
        assert_eq!(o.status.code(), Some(0), "{args:?}");
        let raw = String::from_utf8(o.stdout).unwrap();
        assert!(raw.contains("\\u001b"), "{args:?}: {raw}");
        assert!(
            !raw.contains(ESC) && !raw.contains(FFFD),
            "{args:?}: {raw:?}"
        );
        assert!(
            // The line feeds are the JSON's own layout.
            !raw.chars().any(|c| (c as u32) < 0x20 && c != '\n'),
            "{args:?}: a raw C0 control in {raw:?}"
        );
        let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            v.pointer(at).and_then(|x| x.as_str()),
            Some(want.as_str()),
            "{args:?}"
        );
    }
}

/// An argument is one line of an error at most: a line feed, a carriage
/// return or a direction override in a path or an id starts no line of
/// its own and reorders nothing on stderr.
#[test]
fn an_argument_with_a_line_feed_makes_one_line_of_error() {
    let tmp = tempfile::tempdir().unwrap();
    let (cfg, data) = fixture(tmp.path());
    let path = format!(
        "{}/missing\nFAKE-LINE: all is well\rover\u{202e}txt",
        tmp.path().display()
    );
    let shown = format!("missing{FFFD}FAKE-LINE: all is well{FFFD}over{FFFD}txt");
    let id = "no\nFAKE-LINE\u{2066}such";
    let cases: [(&[&str], &str); 6] = [
        (
            &["correct", "list", "--key", "x:1", "--archive", &path],
            &shown,
        ),
        (
            &[
                "correct",
                "theme-add",
                "T",
                "--key",
                "x:1",
                "--archive",
                &path,
            ],
            &shown,
        ),
        (&["events", "check", "--archive", &path], &shown),
        (&["events", "seed", "--archive", &path], &shown),
        (&["topics", "themes", "--in", id], "FAKE-LINE"),
        (&["search", "proof", "--in", id], "FAKE-LINE"),
    ];
    for (args, want) in cases {
        let o = scout(&cfg, &data, args);
        assert_ne!(o.status.code(), Some(0), "{args:?}");
        assert_eq!(String::from_utf8(o.stdout).unwrap(), "", "{args:?}");
        let err = String::from_utf8(o.stderr).unwrap();
        assert!(err.starts_with("scout: "), "{args:?}: {err:?}");
        assert!(err.contains(want), "{args:?}: want {want:?} in {err:?}");
        assert_eq!(err.lines().count(), 1, "{args:?}: {err:?}");
        assert!(err.ends_with('\n') && !err.contains('\r'), "{args:?}");
        assert!(
            !err.contains('\u{202e}') && !err.contains('\u{2066}'),
            "{args:?}: {err:?}"
        );
        assert_safe(&err, "an error");
    }
}

/// A corrections file over the size limit is not read; the build goes
/// through, and its text output says in a line of its own that no
/// correction was applied.
#[test]
fn a_build_that_did_not_read_the_corrections_says_so_in_its_text_output() {
    let tmp = tempfile::tempdir().unwrap();
    let (cfg, data) = fixture(tmp.path());
    let file = tmp.path().join("archive/analysis/topics/corrections.jsonl");
    fs::write(&file, "{\"key\":\"x:1\",\"themes\":[\"Fixed\"]}\n").unwrap();
    let limit = scout_corpus::corrections::MAX_FILE_BYTES;
    // One byte over, as a sparse tail.
    fs::OpenOptions::new()
        .write(true)
        .open(&file)
        .unwrap()
        .set_len(limit + 1)
        .unwrap();
    let o = scout(&cfg, &data, &["index", "build"]);
    assert!(o.status.success(), "{o:?}");
    let text = String::from_utf8(o.stdout).unwrap();
    let line = text
        .lines()
        .find(|l| l.contains("WARNING"))
        .unwrap_or_else(|| panic!("no warning line in {text}"));
    assert_eq!(
        line.trim(),
        format!(
            "WARNING: corrections NOT applied: the corrections file is {} bytes, larger than the limit of {limit} bytes; it was not read and NO correction was applied",
            limit + 1
        )
    );
    // The JSON report of an unchanged build still carries it.
    let o = scout(&cfg, &data, &["index", "build", "--json"]);
    assert!(o.status.success(), "{o:?}");
    let v: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    let topics = &v["reports"][0]["topics"];
    assert_eq!(topics["corrections_skipped"]["bytes"], limit + 1, "{v}");
    assert_eq!(topics["corrections_skipped"]["limit"], limit);
    assert_eq!(topics["warnings"], 1);
    assert_eq!(topics["corrections"], 0);
    // And the theme the file would have set is not there.
    let o = scout(&cfg, &data, &["topics", "themes"]);
    assert!(!String::from_utf8(o.stdout).unwrap().contains("Fixed"));
}

/// A query, a term or a node the views show in quotes comes from an
/// argument: it goes through the same replacement as every other string,
/// not through an escape of its own. Each view prints U+FFFD where the
/// argument has ESC, a bidirectional override or a zero width space, and
/// nothing a terminal acts on.
#[test]
fn a_query_shown_in_quotes_is_shown_safe_in_every_text_view() {
    let tmp = tempfile::tempdir().unwrap();
    let (cfg, data) = fixture(tmp.path());
    assert!(scout(&cfg, &data, &["index", "build"]).status.success());
    let odd = format!("zq{ESC}[2Jzq\u{202e}x\u{200b}y");
    let shown = format!("zq{FFFD}[2Jzq{FFFD}x{FFFD}y");
    let query = format!("proof -{odd}");
    let views: [(&[&str], String); 8] = [
        (&["facets", &query], format!("for \"proof -{shown}\" in ")),
        (&["search", &query], format!("for \"proof -{shown}\" in ")),
        (&["kwic", &odd], format!("for \"{shown}\" in ")),
        (
            // `--near` takes one word, so it holds no such character;
            // it is shown through the same function.
            &["kwic", "proof", "--near", "models"],
            "for \"proof\" near \"models\" (±".to_string(),
        ),
        (
            &["dist", &odd, "--by", "year"],
            format!("for \"{shown}\" by "),
        ),
        (
            &["collocates", &odd],
            format!("collocates of \"{shown}\" in "),
        ),
        (
            &["collocates", &odd, "--compare", "after:2020"],
            format!("collocates of \"{shown}\" · ±"),
        ),
        (&["profile", &odd], format!("\"{shown}\" in ")),
    ];
    for (args, want) in views {
        let o = scout(&cfg, &data, args);
        let what = args[..args.len().min(2)].join(" ");
        let text = String::from_utf8(o.stdout).unwrap();
        let err = String::from_utf8(o.stderr).unwrap();
        assert!(
            text.contains(&want),
            "{what}: want {want:?} in {text:?} ({err:?})"
        );
        assert_safe(&text, &what);
        assert_safe(&err, &what);
        // Replaced, not written as an escape of the character.
        assert!(
            !text.contains("\\u{") && !text.contains("\\x"),
            "{what}: {text}"
        );
    }
    // A quote in the query is still one quoted string.
    let o = scout(&cfg, &data, &["facets", "\"A proof\""]);
    let text = String::from_utf8(o.stdout).unwrap();
    assert!(text.contains("for \"\\\"A proof\\\"\" in "), "{text}");
}

//! Reading the theme list (`scout_corpus::themes`): the list in its order
//! and spelling, that it is the list `CorrectionEditor::add_theme`
//! accepts, every refusal, and that nothing in the archive changes. Each
//! test builds its own archive in a temp folder.

mod event_seed_support;
use event_seed_support::{json, put, tree, Path, Value};
use scout_corpus::corrections::CorrectionEditor;
use scout_corpus::themes::{
    same_theme, themes_list, NoThemeList, ThemeEntry, MAX_THEMES_BYTES, MAX_THEME_DEFINITION_CHARS,
    MAX_THEME_NAME_CHARS, THEMES_FILE,
};

const MATHS: &str = "AI and mathematics";
const VIBE: &str = "Vibe coding and AI-assisted programming";
const READER: &str = "Reader’s “own” tools";

/// An archive with one tagged item without themes (`x:1`) and `list` as
/// its theme list.
fn archive(root: &Path, list: &Value) {
    put(
        root,
        "analysis/topics/current.jsonl",
        "{\"key\":\"x:1\",\"date\":\"2026-09-01\",\"themes\":[],\"entities\":[],\"event\":null}\n",
    );
    put(root, THEMES_FILE, &list.to_string());
}

fn three() -> Value {
    json!({"meta": {"status": "FINAL"}, "themes": [
        {"id": "T01", "name": VIBE, "definition": "Writing code\n with a model.", "include": ["x"], "added_in": "v1"},
        {"id": "T02", "name": MATHS},
        {"name": READER, "id": null, "definition": ""},
    ]})
}

fn entry(name: &str, id: Option<&str>, definition: Option<&str>) -> ThemeEntry {
    ThemeEntry {
        name: name.to_string(),
        id: id.map(String::from),
        definition: definition.map(String::from),
    }
}

/// The error of the read, causes joined as the CLI joins them.
fn refusal(root: &Path) -> String {
    format!("{:#}", themes_list(root).unwrap_err())
}

fn names(root: &Path) -> Vec<String> {
    themes_list(root)
        .unwrap()
        .into_iter()
        .map(|t| t.name)
        .collect()
}

#[test]
fn the_list_is_in_the_files_order_and_spelling_and_nothing_changes() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    archive(root, &three());
    let before = tree(root);
    assert_eq!(
        themes_list(root).unwrap(),
        [
            entry(VIBE, Some("T01"), Some("Writing code with a model.")),
            entry(MATHS, Some("T02"), None),
            entry(READER, None, None),
        ]
    );
    assert_eq!(
        serde_json::to_value(&themes_list(root).unwrap()[1]).unwrap(),
        json!({"name": MATHS, "id": "T02", "definition": null})
    );
    assert_eq!(tree(root), before);
}

/// A name is trimmed as the writer trims it; `id` and `definition` lose
/// their control characters; a later spelling of a theme already listed
/// is left out, since `add_theme` writes the first.
#[test]
fn names_are_trimmed_other_text_is_cleaned_and_a_repeated_theme_is_listed_once() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    archive(
        root,
        &json!({"themes": [
            {"name": format!("  {MATHS} "), "id": " T\u{0}01\u{1b}[31m ", "definition": "a\u{7}b\r\n  c\u{2028}d"},
            {"name": "ai AND mathematics", "id": "T02"},
            {"name": VIBE, "id": "\u{0}", "definition": " \n "},
        ]}),
    );
    assert_eq!(
        themes_list(root).unwrap(),
        [
            entry(MATHS, Some("T01[31m"), Some("ab c d")),
            entry(VIBE, None, None),
        ]
    );
    let e = CorrectionEditor::open(root).unwrap();
    assert_eq!(
        e.add_theme("x:1", "ai AND mathematics").unwrap().value,
        MATHS
    );
}

/// The invariant of the read: its names are the ones `add_theme` takes,
/// and it writes each as the read spells it.
#[test]
fn every_name_of_the_read_is_accepted_by_add_theme_and_written_in_that_spelling() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    archive(root, &three());
    let listed = names(root);
    assert_eq!(listed, [VIBE, MATHS, READER]);
    let e = CorrectionEditor::open(root).unwrap();
    for name in &listed {
        let done = e.add_theme("x:1", name).unwrap();
        assert_eq!((done.op.as_str(), &done.value), ("theme_add", name));
        assert!(done.tags.themes.contains(name), "{name}");
        // Given in another spelling of the same theme, the list's is
        // written.
        e.remove_theme("x:1", name).unwrap();
        let other = name
            .to_uppercase()
            .replace('’', "'")
            .replace(['“', '”'], "\"");
        assert_ne!(&other, name);
        assert!(same_theme(&other, name));
        assert_eq!(&e.add_theme("x:1", &other).unwrap().value, name);
    }
    // A name that is none of the read's is refused by the writer.
    for off in ["Off the list", "AI and mathematic", "AI & mathematics"] {
        assert!(!listed.iter().any(|n| same_theme(n, off)));
        let why = e.add_theme("x:1", off).unwrap_err().to_string();
        assert!(
            why.contains("not a theme of analysis/context/themes.json"),
            "{why}"
        );
    }
    // The list is as it was; only the corrections file is new.
    assert_eq!(names(root), listed);
}

#[test]
fn no_list_is_its_own_error_whichever_part_of_the_way_is_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    archive(root, &three());
    for gone in [THEMES_FILE, "analysis/context", "analysis"] {
        let at = root.join(gone);
        if at.is_dir() {
            std::fs::remove_dir_all(&at).unwrap();
        } else {
            std::fs::remove_file(&at).unwrap();
        }
        let before = tree(root);
        let e = themes_list(root).unwrap_err();
        assert_eq!(
            e.downcast_ref::<NoThemeList>(),
            Some(&NoThemeList),
            "{gone}"
        );
        assert_eq!(e.to_string(), "No theme list in this archive.");
        assert_eq!(tree(root), before);
        assert!(!root.join(gone).exists());
    }
}

#[test]
fn a_file_that_is_not_a_theme_list_is_refused_with_the_file_and_the_field() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    archive(root, &three());
    let at = root.join(THEMES_FILE);
    let e = CorrectionEditor::open(root).unwrap();
    // The loader's refusals: the read's and the writer's, word for word.
    for (bad, why) in [
        (
            "{",
            "not JSON (EOF while parsing an object at line 1 column 1) (nothing was changed)",
        ),
        ("[]", "no `themes` list (nothing was changed)"),
        ("{\"themes\": {}}", "no `themes` list (nothing was changed)"),
        (
            "{\"themes\": [{\"name\": \"A\"}, {\"name\": 3}]}",
            "theme 2 has no text `name` (nothing was changed)",
        ),
        (
            "{\"themes\": [{\"id\": \"T01\"}]}",
            "theme 1 has no text `name` (nothing was changed)",
        ),
        (
            "{\"themes\": [{\"name\": \"  \"}]}",
            "theme 1 has no text `name` (nothing was changed)",
        ),
        (
            "{\"themes\": [\"A\"]}",
            "theme 1 has no text `name` (nothing was changed)",
        ),
    ] {
        put(root, THEMES_FILE, bad);
        let before = tree(root);
        let want = format!("{}: {why}", at.display());
        assert_eq!(refusal(root), want, "{bad}");
        assert_eq!(
            e.add_theme("x:1", MATHS).unwrap_err().to_string(),
            want,
            "{bad}"
        );
        assert_eq!(tree(root), before);
    }
    // The read's own: a field only it returns, and a name `add_theme`
    // cannot be given. The writer reads these lists as it always did.
    for (bad, why) in [
        (
            json!({"themes": [{"name": "A"}, {"name": MATHS, "id": 2}]}),
            "theme 2: `id` is not text (nothing was changed)",
        ),
        (
            json!({"themes": [{"name": MATHS, "definition": ["d"]}]}),
            "theme 1: `definition` is not text (nothing was changed)",
        ),
        (
            json!({"themes": [{"name": MATHS}, {"name": "A\u{0}B"}]}),
            "theme 2: `name` holds a control character (U+0000); give it as one line of plain text (nothing was changed)",
        ),
        (
            json!({"themes": [{"name": MATHS}, {"name": "A\tB"}]}),
            "theme 2: `name` holds a control character (U+0009); give it as one line of plain text (nothing was changed)",
        ),
    ] {
        put(root, THEMES_FILE, &bad.to_string());
        assert_eq!(refusal(root), format!("{}: {why}", at.display()));
        assert_eq!(e.add_theme("x:1", MATHS).unwrap().value, MATHS);
        e.remove_theme("x:1", MATHS).unwrap();
    }
}

#[test]
fn a_list_over_the_cap_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    archive(root, &three());
    let mut body = three().to_string().into_bytes();
    body.resize(MAX_THEMES_BYTES as usize, b' ');
    std::fs::write(root.join(THEMES_FILE), &body).unwrap();
    assert_eq!(names(root).len(), 3);
    body.push(b' ');
    std::fs::write(root.join(THEMES_FILE), &body).unwrap();
    assert_eq!(
        refusal(root),
        format!(
            "{}: larger than {MAX_THEMES_BYTES} bytes; nothing was changed",
            root.join(THEMES_FILE).display()
        )
    );
}

#[test]
fn an_archive_path_with_dots_or_that_is_no_folder_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("archive");
    archive(&root, &three());
    let dotted = root.join("analysis/..");
    assert!(refusal(&dotted).contains("the archive path has a `..` component"));
    let file = root.join(THEMES_FILE);
    assert_eq!(
        refusal(&file),
        format!("{}: the archive is not a folder", file.display())
    );
    assert!(refusal(&tmp.path().join("none")).ends_with("the archive is not a folder"));
}

/// `analysis` or `context` as a link to a folder outside the archive that
/// holds a valid list: nothing there is read. The same for a file in a
/// folder's place.
#[cfg(unix)]
#[test]
fn a_link_in_place_of_a_folder_on_the_way_to_the_list_is_refused() {
    use std::os::unix::fs::symlink;
    let parts = ["analysis", "context"];
    for n in 0..parts.len() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("archive");
        let outside = tmp.path().join("outside");
        let mut rest = parts[n + 1..].to_vec();
        rest.push("themes.json");
        put(&outside, &rest.join("/"), &three().to_string());
        let at = root.join(parts[..=n].join("/"));
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        let want = format!(
            "{}: not a plain folder; the theme list was not read",
            at.display()
        );

        symlink(&outside, &at).unwrap();
        // The link does lead to a list.
        assert!(root.join(THEMES_FILE).is_file());
        let before = (tree(&root), tree(&outside));
        assert_eq!(refusal(&root), want);
        assert_eq!((tree(&root), tree(&outside)), before);

        std::fs::remove_file(&at).unwrap();
        std::fs::write(&at, "a file").unwrap();
        assert_eq!(refusal(&root), want);

        std::fs::remove_file(&at).unwrap();
        assert_eq!(refusal(&root), "No theme list in this archive.");
    }
}

#[cfg(unix)]
#[test]
fn a_symlink_and_a_folder_under_the_lists_name_are_refused() {
    use std::os::unix::fs::symlink;
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("archive");
    archive(&root, &three());
    let at = root.join(THEMES_FILE);
    let real = put(tmp.path(), "elsewhere/themes.json", &three().to_string());
    let not_plain = format!(
        "{}: not a plain file (a symlink, a folder or a special file); nothing was changed",
        at.display()
    );

    std::fs::remove_file(&at).unwrap();
    symlink(&real, &at).unwrap();
    assert!(at.is_file());
    let before = tree(tmp.path());
    assert_eq!(refusal(&root), not_plain);
    // To nothing: still a link, not "no list".
    std::fs::remove_file(&real).unwrap();
    assert_eq!(refusal(&root), not_plain);

    std::fs::remove_file(&at).unwrap();
    std::fs::create_dir(&at).unwrap();
    assert_eq!(refusal(&root), not_plain);
    std::fs::remove_dir(&at).unwrap();
    put(tmp.path(), "elsewhere/themes.json", &three().to_string());
    symlink(&real, &at).unwrap();
    assert_eq!(tree(tmp.path()), before);
}

/// The archive path is the caller's choice, a link or not.
#[cfg(unix)]
#[test]
fn an_archive_reached_through_a_link_is_read() {
    let tmp = tempfile::tempdir().unwrap();
    let real = tmp.path().join("real");
    archive(&real, &three());
    let link = tmp.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    assert_eq!(names(&link), [VIBE, MATHS, READER]);
}

/// The read needs neither tag rows nor an index: the list alone.
#[test]
fn the_list_is_read_in_an_archive_that_holds_nothing_else() {
    let tmp = tempfile::tempdir().unwrap();
    put(tmp.path(), THEMES_FILE, "{\"themes\": []}");
    let before = tree(tmp.path());
    assert!(themes_list(tmp.path()).unwrap().is_empty());
    assert_eq!(tree(tmp.path()), before);
}

/// A name the read returns is one the writer can write: the longest one
/// allowed is added in its spelling, and a longer one refuses the list
/// (a name of a mebibyte would pass no correction line).
#[test]
fn a_name_or_an_id_over_the_limit_refuses_the_list_and_one_at_the_limit_is_written() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    // Four bytes a character, the most a name can take of a line.
    let fits = "𝒜".repeat(MAX_THEME_NAME_CHARS);
    archive(
        root,
        &json!({"themes": [{"name": fits, "id": "i".repeat(MAX_THEME_NAME_CHARS)}]}),
    );
    assert_eq!(names(root), [fits.as_str()]);
    let e = CorrectionEditor::open(root).unwrap();
    assert_eq!(e.add_theme("x:1", &fits).unwrap().value, fits);
    e.remove_theme("x:1", &fits).unwrap();

    let at = root.join(THEMES_FILE);
    for (list, key) in [
        (
            json!([{"name": MATHS}, {"name": format!("{fits}𝒜")}]),
            "name",
        ),
        (
            json!([{"name": MATHS}, {"name": "n".repeat(1 << 20)}]),
            "name",
        ),
        (
            json!([{"name": MATHS}, {"name": VIBE, "id": "i".repeat(MAX_THEME_NAME_CHARS + 1)}]),
            "id",
        ),
    ] {
        put(root, THEMES_FILE, &json!({"themes": list}).to_string());
        let before = tree(root);
        assert_eq!(
            refusal(root),
            format!(
                "{}: theme 2: `{key}` is longer than 300 characters (nothing was changed)",
                at.display()
            )
        );
        assert_eq!(tree(root), before);
    }
}

#[test]
fn a_long_definition_is_cut_with_an_ellipsis() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let fits = "é".repeat(MAX_THEME_DEFINITION_CHARS);
    archive(
        root,
        &json!({"themes": [
            {"name": MATHS, "definition": fits},
            {"name": VIBE, "definition": format!("{fits}é")},
            {"name": READER, "definition": format!("{} {}", "d".repeat(MAX_THEME_DEFINITION_CHARS - 2), "word")},
        ]}),
    );
    let got = themes_list(root).unwrap();
    assert_eq!(got[0].definition.as_deref(), Some(fits.as_str()));
    let cut = got[1].definition.clone().unwrap();
    assert_eq!(cut.chars().count(), MAX_THEME_DEFINITION_CHARS);
    assert_eq!(
        cut,
        format!("{}…", "é".repeat(MAX_THEME_DEFINITION_CHARS - 1))
    );
    // No space before the ellipsis.
    assert_eq!(
        got[2].definition.clone().unwrap(),
        format!("{}…", "d".repeat(MAX_THEME_DEFINITION_CHARS - 2))
    );
}

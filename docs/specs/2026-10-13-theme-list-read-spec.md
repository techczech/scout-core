---
title: Reading the theme list (E7) — spec
date: 2026-10-13
status: to build
source: ArchiveScout release ticket E7 (a public read of the theme list)
---

# Reading the theme list (E7)

## Why

`CorrectionEditor::add_theme` accepts a theme only when it is a name of the archive's fixed theme list, compares the name it is given under the fold, and writes the list's own spelling. The engine does not expose that list. ArchiveScout therefore offers themes from the index's theme counts, which leaves out a theme no item has yet and makes the app guess the list's spelling. The app needs exactly the list `add_theme` accepts, read by the engine.

## Files (read only, never written)

`<archive>/analysis/context/themes.json`, the tagger's list (`corrections::THEMES_FILE`). As the writer's loader reads it today, and as the tagger writes it (`docs/specs/2026-10-10-topic-tagging-layer.md`):

```json
{"meta": {…}, "themes": [{"id": "T01", "name": "AI and mathematics", "definition": "…", "include": …, "exclude": …, "added_in": …}, …]}
```

| Field | Use |
|---|---|
| `themes` | a list; required |
| `themes[].name` | text, not empty once trimmed; required. The writer's only field |
| `themes[].id` | the tagger's id of the theme (`T01`); returned when present |
| `themes[].definition` | the tagger's description of the theme; returned when present, on one line |
| `meta`, `themes[].include`, `exclude`, `added_in`, anything else | not read |

No other file is read: not `themes-draft.json`, not `themes-v1.json`, not the index.

## Interface

Library, in the crate that holds the correction writer (`scout-corpus`), and CLI, JSON as in `docs/cli-json.md`:

- `themes_list(archive) -> Result<Vec<ThemeEntry>>`: the themes in the list's order. `ThemeEntry`: `name` (the list's spelling, trimmed as the writer trims it), `id` (text or null), `definition` (text or null).
- `same_theme(a, b) -> bool`: whether two theme names are the same theme, by the comparison `add_theme` makes (the fold of "Query grammar", `normalize::fold`, which is public already).
- CLI: `scout themes list --archive <path> [--json]`, one JSON line `{"themes":[…]}`. It runs before the engine loads (no registry, no index); a refusal is exit 2 with one line on stderr and nothing on stdout.

**One reader.** The file is opened and parsed by one loader, which `add_theme` and `themes_list` both call. What the writer accepts and refuses, and the words it refuses in, do not change.

**No counts.** The read is a pure file read and works without an index. An app that wants the number of items per theme joins the existing `topics` read (`scout topics themes`, `Engine::topics`) to this list by folded name (`same_theme(entry.name, item.value)`); a theme with no row there has no items.

## States and refusals

| State | Result |
|---|---|
| `--archive` path with a `..` component; not a folder | refused, as the briefings read refuses it |
| `analysis` or `analysis/context` is a symlink (to any folder) or a file | refused, path named: `<path>: not a plain folder; the theme list was not read` |
| `analysis`, `analysis/context` or `themes.json` not there | error `No theme list in this archive.` (typed: `NoThemeList`) |
| `themes.json` is a symlink, a folder or a special file | refused by the writer's open (entry looked at without following a link, opened with `O_NOFOLLOW`, the handle checked) |
| over 16 MiB (the writer's cap) | refused |
| not JSON; no `themes` list; a theme without a text `name` | the writer's loader's errors, word for word: `<file>: not JSON (…)`, `<file>: no \`themes\` list`, `<file>: theme 2 has no text \`name\`` |
| `id` or `definition` present, not null and not text | the read refuses, naming file, theme and field; the writer does not look at them and still accepts the list |
| a `name` holding a control character | the read refuses, naming file and theme (see Invariants); the writer still reads the list |
| `id` / `definition` absent, null or empty | null |
| a later name that is the same theme as an earlier one under the fold | left out of the read: `add_theme` would write the earlier spelling for it |

## Invariants

- No write, no lock, no index or registry read.
- Every `name` returned is accepted by `add_theme` as a theme of the list and written exactly as returned; a name that is no theme of the read (under `same_theme`) is refused by `add_theme`.
- `add_theme` takes its theme through a check that refuses every control character, and the read may not return a name it has altered. So a name holding a control character is not cleaned: the read refuses the list. `id` and `definition` are cleaned as the briefings read cleans one-line values (control characters dropped, whitespace runs as one space).
- Below the archive path (which may itself be a symlink) nothing behind a link is read.

## Tests

Fixture archive in a temp dir: the list in order and spelling with `id` and `definition`; the read and `add_theme` agree (every returned name is accepted and written in that spelling; a name not on the list refused); `same_theme` cases; each refusal above; a symlinked folder component and a symlinked list file refused; an archive reached through a link read; no file changed (snapshot before/after); the CLI line equals the library value and each CLI refusal is exit 2 with one stderr line.

## Version

`docs/cli-json.md` heading to 0.4.4 and the command documented there. Manifest versions and the tag are the driver's.

## As built (2026-10-13)

Where it lives: `crates/scout-corpus/src/themes.rs` (`themes_list`, `same_theme`, `ThemeEntry`, `NoThemeList`, the shared loader `load`), `crates/scout-cli/src/theme_cmds.rs`; contract in `docs/cli-json.md`, "`scout themes list`". Tests: `crates/scout-corpus/tests/theme_list_read.rs`, `crates/scout-cli/tests/themes_cli.rs`, one unit test in `themes.rs`. Differences from, and choices within, the text above:

- **A module of its own, not a function inside `corrections.rs`.** The loader moved out of `CorrectionEditor::theme_list` into `themes::load`; `theme_list` now calls it and keeps its own "no theme list" sentence. `corrections::THEMES_FILE` is a re-export of `themes::THEMES_FILE`. `add_theme` compares with `same_theme`.
- **The open is the writer's, unchanged** (`event_edit::store::read_plain`): it already looked at the entry without following a link, opened with `O_NOFOLLOW` and checked the handle, with a 16 MiB cap. It also carries `O_NONBLOCK`, which the briefings read's `open_plain` does not; one reader was kept, so the theme list is opened with it. The briefings read's `open_plain` and `read_plain` are not used here.
- **The folder check is the read's only.** `briefings::briefings_dir` became `briefings::plain_folder_below(archive, rel, unread)` (`pub(crate)`), which the briefings read calls with the words it had and the theme read calls for `analysis/context`. The writer makes no such check, as before.
- **Errors keep the writer's ending.** The loader's errors, and the read's own three, end in `(nothing was changed)`, also when the read gives them.
- **JSON shape.** `id` and `definition` are always present, null when absent, as `items` is in a briefing row.
- **`definition` is one line**: whitespace runs, line breaks included, as one space.
- **No `fold` re-export.** `normalize::fold` and `topics::fold` were public already; `same_theme` is the one addition.

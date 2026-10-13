---
title: Reading the monthly briefings (E8) — spec
date: 2026-10-13
status: to build
source: ArchiveScout release ticket 8 (monthly briefings to browse); archive-scout docs/plans/2026-10-12-topic-layer-release-tickets.md
---

# Reading the monthly briefings (E8)

## Why

ArchiveScout shows a month's briefing with each event linked to its event page. The app does not read archive files itself (an app-side reader of the tag rows was deleted after three review rounds, 2026-10-13); the engine reads them and the app asks the engine.

## Files (read only, never written)

`<archive>/analysis/context/briefings/`:

| File | Use |
|---|---|
| `<period>-briefing.json` | the briefing: `meta` (`month`, `model`, `generated`, `items`, …), `overview` (text), `events` (list), `names` (`{name, aliases, note}`), `debates` (`{title, summary, when}`), `engagement` (text), `problems` (list) |
| `<period>-events.json` | already read by `scout events seed`; not needed here |
| `<period>.md` | rendering of the same; not read |

`period` is `YYYY-MM` or `YYYY-Qn`. A name holding `.superseded-` is ignored, as the seed does. `meta.month` holds the period for quarters too.

## Interface

Library (crate that holds the event reads) and CLI, JSON as in `docs/cli-json.md`:

- `briefings(archive) -> Vec<BriefingListing>`: one row per `<period>-briefing.json`, newest period first (quarters sort by their first month). Row: `period`, `kind` (`month` | `quarter`), `model`, `generated`, `items` (meta.items), `events` (count). A file that cannot be read as a briefing gives a row with `error: <one line>` and no other values, so one bad file does not hide the rest.
- `briefing(archive, period) -> Briefing`: `period`, `kind`, `model`, `generated`, `items`, `overview`, `events`, `names`, `debates`, `engagement`.
  - each event: `monthly_id` (`<period>/E<nn>` in the spelling the index uses), `name`, `date`, `aliases`, `description` as the briefing wrote them, and `record`: the lasting event record that holds this monthly id now (`id`, `name`, `removed: bool`), resolved by the reader the event reads already use (a merged-away record resolves to the one that holds it), or `null` when no record holds it.
  - the briefing's own `items` lists (`m0018`, …) are ids local to the model run and are not returned.
- CLI: `scout briefings list --archive <path> [--json]`, `scout briefings show <period> --archive <path> [--json]`.

## States and refusals

| State | Result |
|---|---|
| briefings folder missing | `briefings` returns an empty list; `briefing` errors `No briefing for <period>.` |
| period not `YYYY-MM` / `YYYY-Qn` | error `"<period>" is not a month (YYYY-MM) or a quarter (YYYY-Qn).` before any file is opened (the period is never joined into a path unvalidated) |
| file is not a plain file (symlink, folder, special file) | refused, via `open_regular` |
| file over 4 MiB | refused with the size named |
| not JSON, not an object, `meta.month` ≠ file period, `events` not a list, wrongly typed field | error naming the file and the field, as the seed's errors do |
| absent or null `overview`, `names`, `debates`, `engagement`, `aliases`, `description`, `date` | empty value; not an error |
| event id not `E<number>` or repeated | error naming it |
| events folder missing or a record unreadable | `record: null` for the events affected is wrong: the read fails with the events reader's own error (a link must not silently vanish) |

## Invariants

- No write, no lock, no index change. Works without an index.
- Reads only plain files directly inside the briefings folder; the period comes from a validated string.
- Text is returned as stored (no Markdown rendering); control characters are dropped as the seed drops them from descriptions.

## Tests

Fixture archive in a temp dir: list order with months and a quarter; superseded file ignored; one bad file gives an error row and the others still list; each refusal above; record resolution for a plain, a merged-away and a removed record and for a monthly id no record holds; a symlinked briefing file refused; no file in the archive changed (snapshot before/after).

## Version

Tag v0.4.3; `docs/cli-json.md` heading and the two commands documented there.

## As built (2026-10-13)

Where it lives: `crates/scout-corpus/src/briefings.rs` (`briefings`, `briefing`), `crates/scout-cli/src/briefing_cmds.rs`; contract in `docs/cli-json.md`, "`scout briefings`". Differences from, and choices within, the text above:

- **Signatures.** Both calls return `anyhow::Result`: `briefings(&Path) -> Result<Vec<BriefingListing>>`, `briefing(&Path, &str) -> Result<Briefing>`. `BriefingListing` is an untagged enum: a `BriefingSummary` row or `Unreadable { error }`. `No briefing for <period>.` is the typed error `briefings::NoBriefing`.
- **CLI reply.** One JSON line, as the other archive commands print (`scout events merges`): `{"briefings":[…]}` for `list`, the briefing object for `show`. `--json` is accepted and changes nothing; there is no `schema_version` and no human view.
- **Symlinks.** `open_regular` follows a link to a regular file, and the spec refuses a symlinked briefing. So the entry itself is checked first, without following a link (`symlink_metadata`, the seed's rule and wording: `<file>: not a plain file`), and only a plain file is then opened by `open_regular`.
- **`meta.month` must be present.** A file with no `meta` object, or a `meta` without `month`, is refused (the seed lets an events file have no `month`). `meta.model`, `meta.generated` and `meta.items` may be absent (empty, empty, null).
- **An error row sorts by its file name's period** although it does not carry it (the row is `{"error": …}` only, as specified; the error text names the file).
- **The records folder is read only for a briefing that has events.** A briefing with an empty `events` list is shown in an archive without `analysis/events`.
- **Any record file the reader skips fails `show`** (for a briefing with events), not only one known to hold an event of this briefing: a skipped file's members are unknown. A record whose `merged_into` names no record or closes a cycle is read as not merged, as the reader and the events listing read it; that does not fail the read.
- **The archive path** gets the seed's two checks (`..` component refused; must be a folder). An archive that is not there is an error, not an empty list; a missing briefings folder inside an archive is the empty list.
- **One-line values.** Besides dropping control characters (`render::body_text`, on every text), `model`, `generated`, an event's `name`, `date` and aliases, a name's `name` and aliases, and a debate's `title` and `when` have whitespace runs as one space (the seed's reading of a name), and empty aliases are left out.
- **Shared with the seed.** `event_seed/sources.rs` now exposes `parse_event` (one briefing event: id, name, aliases, description), `file_period` and `one_line`; the seed's own reading goes through them and is unchanged.

### Changed after review (2026-10-13)

Three defects found in review of the first build, each fixed in `crates/scout-corpus/src/briefings.rs` with a test that fails without the fix:

- **Folders on the way to the briefings are not followed as links.** `briefings_dir` now looks at each of `analysis`, `analysis/context` and `analysis/context/briefings` with `symlink_metadata`. A symlink (to any folder) or a file there is refused by `list` and `show`: `<path>: not a plain folder; the briefings were not read`. The first one missing is "no briefings", as before. The archive path itself may still be a symlink. The records folder is not checked this way: the module does not open `analysis/events` itself, it calls the shared reader `events::read`, which is unchanged and follows a symlink at `events` (`analysis` above it is covered by the check here, since it runs first).
- **The open does not follow a link.** `read_plain` no longer opens with `open_regular` (which looks at and opens the path following links). The new `open_plain` opens with `O_NOFOLLOW` and no other flag (no `O_NONBLOCK`), then requires the opened handle to be a regular file; the size cap is applied on the handle as before. The look at the entry before the open stays, so a folder or a FIFO already there is refused without being opened. The gap that remains is `open_regular`'s: a FIFO put at the path between the look and the open can make the open wait. Off Unix, `open_plain` is `open_regular`. `open_regular` and its other callers are unchanged.
- **A linked record's `id` and `name` are cleaned.** `EventLink.id` and `EventLink.name` pass the same cleaning as a briefing's one-line values (`body_text`, then whitespace runs as one space). The records reader decodes YAML escapes after its check for raw control characters, so `name: "a\0b"` read as a name with a NUL. An id is the record's file name without `.md`, which the reader does not limit to an alphabet, so it is cleaned too; an id without such characters is returned unchanged.

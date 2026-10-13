---
title: Unmerge preview and engine follow-ups (E5, E6)
date: 2026-10-13
status: E6 specified and in build; E5 parts listed, each to be ticketed
source: archive-scout docs/plans/2026-10-12-topic-layer-release-tickets.md (E6); _COORDINATION highlights task log 4p (E5); Sol reviews of archive-scout fdafb3a..ad61e38
---

# Unmerge preview and engine follow-ups

## Why

ArchiveScout's Split off confirmation must say what a split moves. The app tried to count this itself by reading `analysis/topics/current.jsonl`; three review findings (wrong counts after a correction, a read outside the corpus root, an unbounded read) ended in that reader being deleted. Only the engine can say what its own unmerge will leave, so the engine says it.

## E6 — `EventEditor::unmerge_preview(log_id)`

Read-only. Returns what `unmerge(log_id)` would leave, computed from the files as they are now (records, merge log, tag rows, corrections) by the same resolution code the index build and the listing use. It never reads the index, so it cannot disagree with a stale one.

Reply (`UnmergePreview`, serialisable, also on the CLI as JSON):

| Field | Meaning |
|---|---|
| `log_id` | the merge asked about |
| `returning` | id and name of the event that becomes current again |
| `kept` | id and name of the event that stays (the current holder, through any chain) |
| `start`, `end` | the returning event's span after the split (as the listing would give it) |
| `returning_items` | items that are in the returning event after the split |
| `kept_items` | items that are in the kept event after the split |
| `in_both` | items in both after the split |
| `kept_items_now` | items in the kept event before the split |
| `undoable`, `why_not` | as the listing gives them; when false the counts are absent |

Membership is the engine's own: tag-row `event` and `events`, resolved through the records as the unmerge leaves them, then corrections (`event_add`, `event_remove`, his entries win), exactly as the index build applies them.

States that must be right (test matrix; each row is a test that runs the real `unmerge` afterwards, rebuilds, and compares the listing with the preview):

| # | State before | Expected |
|---|---|---|
| 1 | A merged into B; items only by tag row, one event each | A's items return; B keeps its own; `in_both` 0 |
| 2 | an item with `event: A-monthly`, `events: [B-monthly]` | counted in both |
| 3 | A's item added to B by correction after the merge | in both |
| 4 | A's item taken out of B by correction after the merge | per the engine's actual rule; pinned by the test, stated in the doc comment |
| 5 | an item added by hand to B (never A's) | stays in B |
| 6 | chain: C merged into A, A merged into B; unmerge A | C's items go with A |
| 7 | the returning record carries `removed` | `undoable: false` with a reason; and `unmerge` itself refuses (new refusal, nothing written) |
| 8 | merge not in effect / unknown id / merged by hand | error as `unmerge` gives, nothing computed |
| 9 | a tag-row line the reader passes over | same treatment as the index build; preview and outcome still equal |
| 10 | large tag-row file | streamed with the existing line bound; no whole-file read |

Invariants: writes nothing (folder byte-identical before and after); all reads stay inside the corpus root by the engine's existing path rules; `unmerge`'s checks are not weakened; the only behaviour change to `unmerge` is the new refusal in row 7, which the listing mirrors as `undoable: false`.

Impact: scout-corpus `event_edit`; CLI JSON doc (`docs/cli-json.md`); version v0.4.2. ArchiveScout then calls it from its Rust page read or a dedicated command and prints the numbers.

## E5 — follow-ups, not in this build

1. **Typed refusal from engine writes.** A validation refusal returns "refused, nothing changed" as a distinct result, so a client keeps its editor open and does not run the "archive may have changed" re-index path. Interface change to every write result: Opus build, review.
2. **Item day on search hits.** Read path.
3. **Multi-month sums in the engine** (Themes over time, event month strip) instead of in the app. Read path.

## As built (2026-10-13)

- Counts are tag rows resolved through the records, not listed documents: a tag row that joins no indexed document is counted here and not in `scout events`. A client that shows an event's item count from the index prints the preview's numbers only when `kept_items_now` equals that count, and no numbers otherwise.
- Corrections are streamed twice (the existing reader's design: the first pass finds undo lines).
- Row 4: a correction's id is resolved through the records as they are after the split. `event_remove: [B]` → the item is in A again; `event_remove: [A]` (or A's monthly id) → the item stays out of both.
- Version: the engine's version is the tag and the `docs/cli-json.md` heading (manifests stay 0.1.0), as for v0.4.1.
- The layer files (`current.jsonl`, `corrections.jsonl`) are read only when they are regular files: a link to a regular file is followed; a directory, a FIFO, a socket, a device, or a link to one of these is refused without being read (an error naming the path), by the preview and by the index build, through one function (`correction_lines::open_regular`: opened without blocking, checked on the opened handle).
- Confinement of the layer files to the corpus root (a link that leaves the root) is not enforced, as in the index build; follow-up.

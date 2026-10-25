# scout-core

`scout-core` is the shared engine for Highlight Scout, ArchiveScout, and later Scout-family tools such as SlideWell.

It contains three parts:

- `scout-index`: a Rust FTS5 index and search layer for container/record archives.
- `scout-archive`: Rust Markdown archive rendering and content-hash idempotency helpers.
- `@scout/query`: the TypeScript search-query grammar exposed from the repository root package.
- `scout-query`: the Rust port of `@scout/query`. Both implementations must pass one shared fixture file, `packages/scout-query/fixtures/grammar-cases.json` (regenerate it from the TS package with `SCOUT_WRITE_FIXTURES=1 npx vitest run packages/scout-query/grammar.test.ts`).

## Schema Freeze

The physical SQLite schema keeps the Highlight Scout v0.5.5 names: `works`, `highlights`, and `search_index`, including columns such as `work_id`, `work_type`, and `highlighted_at`.

Generic names such as `Container` and `Record` are API-level only. Do not rename SQL tables or columns.

## Root Package Quirk

The npm package lives at the repository root because Bun and npm git dependencies cannot target a subdirectory. Source code lives under `packages/scout-query/src`, and the root `prepare` script builds `packages/scout-query/dist`.

## Research folders

`scout capture --project <folder> --into literature/notes --kind evidence --title "Title" -`
writes a capture below `research/literature/notes/`. Omit `--into` for a flat capture.
Copied external artefacts stay beside the capture. One `research/_manifest.jsonl`
records every capture using paths relative to `research/`; existing flat receipts
remain valid. `scout capture --list --project <folder>` lists both layouts.
Destination paths must be relative and contain no `..`; symlinks are refused.

`scout capture --where-used <file>` accepts an existing file path or a
research-relative path such as `literature/notes/example.md`. References match
paths relative to the referencing Markdown file or to `research/`. Same-named
files in different subfolders remain distinct. Usages within `research/` do not
count.

After moving a capture yourself within `research/`, run
`scout capture --project <folder> --moved <old> <new>` with both paths relative
to `research/`. This explicitly updates its receipt under the root manifest lock;
the old file must be absent and the new file must exist. It does not move files,
rewrite links or provenance, or rebuild the index. Reads never repair receipts.
The full CLI and JSON contract is in [docs/cli-json.md](docs/cli-json.md).

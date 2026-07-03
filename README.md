# scout-core

`scout-core` is the shared engine for Highlight Scout, ArchiveScout, and later Scout-family tools such as SlideWell.

It contains three parts:

- `scout-index`: a Rust FTS5 index and search layer for container/record archives.
- `scout-archive`: Rust Markdown archive rendering and content-hash idempotency helpers.
- `@scout/query`: the TypeScript search-query grammar exposed from the repository root package.

## Schema Freeze

The physical SQLite schema keeps the Highlight Scout v0.5.5 names: `works`, `highlights`, and `search_index`, including columns such as `work_id`, `work_type`, and `highlighted_at`.

Generic names such as `Container` and `Record` are API-level only. Do not rename SQL tables or columns.

## Root Package Quirk

The npm package lives at the repository root because Bun and npm git dependencies cannot target a subdirectory. Source code lives under `packages/scout-query/src`, and the root `prepare` script builds `packages/scout-query/dist`.

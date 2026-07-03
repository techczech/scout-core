# Agent Instructions: scout-core

## Register

FOR ME. Telegraph. Prefer exact commands and concrete file paths.

## Consumers

- Highlight Scout consumes `scout-index`, `scout-archive`, and `@scout/query`.
- ArchiveScout and later Scout-family tools may consume the same core packages.

## Schema Freeze

- Physical SQLite schema is frozen.
- Never rename SQL identifiers: `works`, `highlights`, `search_index`, `work_id`, `work_type`, `highlighted_at`, `ocr_text`.
- Generic naming is Rust API only: `Container`, `Record`, `Hit`.

## Dependencies

- Keep versions pinned as written in the workspace manifest.
- Do not add dependencies unless the consuming plan explicitly says to.

## Commands

- `cargo test --workspace`
- `bun install`
- `bun run build`
- `bun run test`

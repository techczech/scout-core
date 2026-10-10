# Topic tagging layer (2026-10-10)

FOR AGENTS. Implemented by `tools/topic-tagger/`.

## Purpose

Model-made topic tags for archive works (AI items from 2025 on), so Dominik can research "reactions to the maths releases", "how views on vibe coding evolved". Tags live BESIDE the archive as provenance-stamped layers keyed by stable IDs. Never inside work files.

## States of a run

1. `planned`: selection + packing + cost estimate (`--dry-run` stops here, writes nothing).
2. `started`: `analysis/topics/<run-id>/` created, `run.json` written with `finished: null`.
3. `tagged`: `tags.jsonl` written.
4. `finished`: discovery adds `vocabulary.json`; `run.json` completed; `review.html` written.
- Run folder already existing = hard error (layers are additive). Exception: `--batch --resume` when `batch.json` is present.
- Failed items are listed in `run.json.failed_keys` and absent from `tags.jsonl`. Nothing is guessed.

## Layer format

`highlights-archive/analysis/topics/<run-id>/` (run-id like `2026-10-10-sample-2026-09`):

- `run.json`: run_id, mode, model, effort, thinking, api, started, finished, wall_seconds, prompt_version, prompt_text (full), selection_rule, item_count, capped_items, undated_works_skipped, key_fallbacks + key_note, requests, usage, cost_usd_tagging / _vocabulary / _total (from usage at list prices; batch = half), ai_count, ai_share, failed_keys, problems, readings_unchanged, vocabulary_model (when a consolidation model differs or is recorded).
- `tags.jsonl`: one line per item: `{key, path, date, type, ai_related, topics[], note?}`.
  - `key` = `<source_system>:<source_id>`; fallback `sha256:<16 hex of file content>` only if either is missing (flagged in run.json). The fallback follows the archive's content-hash ID rule; the archive ADR text was not found in the repo, so the exact hash recipe is an assumption (sha256 of the whole file, first 16 hex).
  - `path` is relative to the archive root (`readings/works/<file>.md`); it is a locator, `key` is the identity.
  - `date` = latest `highlighted_at` in the work.
  - Discovery topics are free-form labels (max 4). Vocabulary topics must come from the vocabulary or be `new: <name>` (max one per item).
- `vocabulary.json` (discovery only): `topics[{topic, definition, count, examples[3 keys], merged_labels}]`, `mapping{label: topic}`, `unmapped_labels`. `tags.jsonl` keeps the raw labels; the mapping is how scout resolves them.
- `review.html`: single file, no external requests, light/dark, summary then each topic with 3 examples.
- `batch.json` (only with `--batch`): batch id and request ids.

## Invariants

- No file under `readings/` is modified. `run.json.readings_unchanged` compares `git status --porcelain -- readings` before and after. Only `analysis/` may change.
- The key never from the model; items are sent with short per-request ids (`i1..iN`) mapped back by position.
- Cost guard: estimate = chars/3 tokens in (conservative for the Haiku 5.5 tokenizer) + 120 output tokens per item; run aborts before any request if above `--max-usd` (default 3); direct runs also stop sending when actual spend passes it.
- Requests stay under 25K estimated input tokens (Haiku 5.5 price doubles above 100K).
- API key arrives only via `bws_manager.py run --as ANTHROPIC_API_KEY`; never stored or printed.

## Modes

- discovery: model labels freely; one consolidation call proposes ~40-80 topics with definitions and an old-to-new mapping.
- vocabulary (`--mode vocabulary --vocabulary vocabulary.json`): pick from the list; `new:` proposals feed the next vocabulary revision. `--batch` uses Message Batches (50% off) for the full run.

## How scout reads it later

Scout joins `tags.jsonl` to works by `key` (works table `source_system` + `source_id`), not by `path`, so file renames do not break the layer. Where several runs exist, scout shows the newest run per key and displays run-id, model and prompt_version as provenance. Free-form labels resolve through the same run's `vocabulary.json.mapping`. A topic view = all keys with that topic, ordered by `date`.

## Tests

`cd tools/topic-tagger && npm test`: selection/date parsing, key derivation, batching under cap, result validation and merging, consolidation mapping, review page, and a fake-client end-to-end run. No network.

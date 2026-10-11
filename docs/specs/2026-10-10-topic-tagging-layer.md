# Topic tagging layer (2026-10-10; v2 2026-10-11)

FOR AGENTS. Implemented by `tools/topic-tagger/`. v2 (context-aware themes + entities) is the current design: see "v2" below. The v1 sections that follow describe the free-form layer, still runnable (`cli.js --from --to`).

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

## v2: context-aware themes + entities (decided by Dominik 2026-10-11)

Why: v1's Haiku saw each item alone plus a generic paragraph. It did not know the month's events (its training predates them), saw replies/quotes without parents and link-only tweets without the article, and knew nothing of the reader. Result: misplaced tags and a topic list mixing lasting themes with one-month product names.

### Fields per item (`tags.jsonl`)

`{key, path, date, type, ai_related, themes[], entities[], event, relevance, confidence}`

- `themes`: 0-3 names from the fixed curated list (`analysis/context/themes-draft.json`). Matched case-insensitively to the canonical name; anything else dropped and logged. AI items should have >=1; an AI item without one is kept with `confidence: low` so QA sees it.
- `entities`: 0-6 free names (models, products, companies, people as subject, papers, benchmarks). Normalised through the month briefing's alias list ("Astra" -> "GPT-6 Astra"), case-deduped, max 60 chars.
- `event`: id of a month-briefing event (`E01`...) or `null`. Unknown ids -> null + problem.
- `relevance`: <=15 words on why it matters to this reader.
- `confidence`: low | medium | high.

### Stages (CLI subcommands; each writes files and can be re-run alone)

`node cli.js <stage> --month YYYY-MM --run-id <id>`; `all` runs briefing -> review.

1. `profile-sources`: counts the reader's Readwise tags (`| tags:` lines, case-folded, system tags such as like/favorite/shortlist excluded) -> `analysis/context/readwise-tags.json`. The profile itself, `analysis/context/reader-profile.md`, is hand-drafted (status DRAFT, <=1,200 words) from the dominik-context skill, the tag list and his recent writing; it is not an API call.
2. `briefing` (claude-sonnet-5-5, effort medium): condensed view of every item (short id, date, type, author, title for non-tweets, first 300 chars) -> JSON (overview, events with ids/dates/aliases/2-5 item ids, names and aliases, debates, engagement). Events renumbered E01.. by date; unknown item ids dropped. Writes `briefings/<month>.md` (rendered from the JSON so ids match), `<month>-events.json` (id, name, date, aliases, description, items) and `<month>-briefing.json` (full + meta, system prompt, usage, cost).
3. `themes` (claude-opus-5-5, effort high, one call): profile + briefing + v1 `vocabulary.json` + Readwise tags -> ~40 lasting themes (name, definition, include, exclude), 25-55 enforced. Products/models/companies are entities, never themes. `themes-draft.json`, `meta.status: DRAFT`; the full version will use all monthly briefings.
4. `enrich` (no model): per item, written to `<run>/enrichment.jsonl`, counts in `run.json.stages.enrichment`.
   - Reply parent / quoted tweet: ids from the X import snapshot `imports/x-api-*/likes-saved.jsonl` (`reply_to_id`, `quoted_tweet_id`; the work file's `source_data` carries no ids); status links in the body also count as quotes. Text from archive works (tweet id = `source_id`, any date) or the reader's own tweets (`writing/tweets/stream/*.md`). Snapshot `parent_text`/`quoted_text` win when present. Missing parents are marked so the tagger can lower confidence.
   - Linked article: `🔗` and bare URLs in the body plus snapshot `article_urls` (t.co and image links skipped), joined to archive works by normalised URL (host without www/m., twitter=x, tracking params dropped, no trailing slash); text from `readings/fulltext/<same file>` else highlights; 400 chars; max 2 per item.
   - Thread hint: other items by the same author on the same day (max 6).
5. `tag` (claude-haiku-5-5, effort medium, structured output): system = cached blocks in this order: instructions, reader profile, themes (with include/exclude), month briefing (events, names, debates), the reader's own posts that month (condensed, <=30K chars), the MonDAI roundup covering the month (`ai-news-tracking/preview-browser/public/data/roundups/mondai-<mon>-<yyyy>.json`, condensed, <=20K chars), month index (one line per item: short id, MM-DD, type letter, author, gist <=100 chars). One `cache_control` breakpoint on the last block. User message = ~20 items with their context blocks; item ids are the month-index ids, so the model can relate items to the index; the key never comes from the model.
   - Hard cap: system + largest user batch < 100K tokens (Haiku price step). `fitPrompt` counts the system prompt with `messages.countTokens` and shrinks the index gists in steps (100 -> 30 chars) until system + 14K user reserve + 2K margin fits; batches are packed under the reserve with the chars/3 estimate.
   - First request sent alone (writes the cache; parallel requests cannot read an entry still being written), the rest in a pool of 4. `run.json.cache_stats`: token hit rate = cache reads / all prompt tokens; requests with a read.
   - Failure handling: missing items retried as a group (twice at most), failed requests split in halves to single items; failures listed in `failed_keys`.
   - The exact system prompt is saved as `prompt-tagging-system.txt` and `.json` (sha in `run.json.prompt_files`).
6. `qa` (claude-sonnet-5-5, effort medium): same system blocks (`prompt-tagging-system.json`) and item rendering; sample = seeded random 100 + all low-confidence, cap 150. Agreement per group (all / random / low): themes any-overlap, exact set, mean Jaccard; entities the same (case-insensitive); event equal (incl. both null) and equal-when-both-set; ai_related equal. `qa.jsonl` keeps both records per key.
7. `review`: `<run>/review.html`: profile and briefing rendered (collapsible, with relative links), themes with count + definition + 3 examples, top 50 entities, events with item counts + 3 examples, QA table, cost per stage, v1 vs v2 table for 15 random items. Same style rules as v1.

### run.json (v2 additions)

`version: 2, month, prompt_version: topics-v2, stages{enrichment, tagging, qa}` each with usage/cost/problems; `model, effort, prompt_cap, prompt_tokens_system, prompt_max_user_tokens_estimated, prompt_sizes[] (per block chars/est tokens), month_index{lines, gist_chars}, prompt_files, prompt_sources, cache_stats, confidence_counts, qa, qa_model, cost_usd_by_stage, cost_usd_total, context_files, readings_status_baseline, readings_unchanged`.

### Cost guard (v2)

`--max-usd` (default 5) is the cap for the whole month's v2 run: briefing + themes + tagging + QA, read back from the stage files. Each paid stage estimates first and aborts before any request if spent + estimate exceeds the cap (a re-run subtracts the cost of the output it replaces); tagging and QA stop sending once the remaining budget is spent. Prices in `lib/cost.js`: cache reads 0.1x input (Opus 5.5 0.05x), 5-minute writes 1.25x; Haiku's long-prompt card applies when uncached + cached prompt tokens exceed 100K.

### Prompt versions

`topics-v2` (tagging), `briefing-v1`, `themes-v1`. v1 runs keep recording `topics-v1` (`PROMPT_VERSION_V1`).

### Tests (v2)

Fixture archive in `test/fixtures-v2.js` (reply, quote of the reader's own tweet, link-only tweet with fulltext, same-day thread, missing parent, undated work, snapshot, stream, roundup) and a fake client. `enrich.test.js` (URL normalisation, joins, counts), `month-index.test.js` (ids, gists, cap and shrink), `tag-prompt.test.js` (block order, single breakpoint, fit under cap incl. a denser real counter, item rendering, params), `tag-results.test.js` (theme/entity/event/relevance/confidence validation, briefing and themes validation, QA sample and agreement, cache pricing), `pipeline.test.js` (all stages end to end, readings untouched, cost cap before any request, review escaping).

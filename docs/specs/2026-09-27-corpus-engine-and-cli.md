---
title: "Scout corpus engine and `scout` CLI: spec"
date: 2026-09-27
status: locked (Fable, per grill-only-on-functionality; functional picks by Dominik 2026-09-27)
journeys: ~/gitrepos/06_apps-utilities/01_desktop-apps/highlight-scout/docs/design/2026-09-27-archive-search-and-corpus-tools/journeys.md
record: ~/gitrepos/_COORDINATION/highlights/_TASK-LOG/2026-09-27-archive-search-and-analysis-survey.md
register: FOR AGENTS
---

# Scout corpus engine + `scout` CLI

## Why

- One index over Dominik's writing, tweets and highlights.
- Serves: Highlight Scout (the quick finder), ArchiveScout (the corpus lab, released publicly, works on ANY archive), the WriteFlex panel, and agents through the CLI and skill.
- Journeys served: J1 (quote and cite), J2 (sets: data only here), J4 KWIC, J5 collocates and word profile, J6 n-grams, J8-B (agent reports), J9 CLI.
- UI is out of scope; the mockups are a separate round.

## Invariants (must hold after every ticket)

1. **Sources are never written by query/index commands.** Capture is the one engine write to corpus folders, confined to `<project>/research/` for new research units and receipts. `git -C <source> status --porcelain` is unchanged after any query/index command.
2. **Quotes are original text.** Normalisation feeds counting and matching only. Every hit, KWIC line, citation and verify result carries the ORIGINAL source text and a source line number, never the normalised form. Offsets map from the normalised copy back to the original (the estate lesson "offsets from a normalised copy").
3. **Counts agree across views.** For the same word and filters, the KWIC line count = the n-gram count (n=1) = the word-profile frequency; a collocate row's count = the lines in its concordance link.
4. **Indexes are derived and disposable.** They are never inside a source folder and never committed. Deleting an index and rebuilding it gives identical query results.
5. **CLI parity.** App and CLI call the same library functions. The CLI has no logic of its own beyond argument parsing and output formatting.
6. **Deterministic output.** Stable sort with explicit tie-breaks (source path, then line), so the same query gives byte-identical `--json`.

## Where new state lives

| State | Location | Format |
|---|---|---|
| Corpus registry | `~/.config/scout/corpora.toml` | TOML, one `[[corpus]]` per corpus |
| Index per corpus | `~/Library/Application Support/scout/indexes/<corpus-id>.sqlite` (the platform data dir through `dirs`, so Windows and Linux work for the public release) | SQLite plus FTS5 |
| Boilerplate rules | inside the corpus entry in the registry | a list of regexes, line-level |
| Sets (J2) | out of scope for this spec; they belong to the app-side spec | — |

Registry entry fields:
- `id`, `name`
- `kind`: `markdown-folder` | `highlight-scout-archive`
- `path`
- `include` / `exclude` globs
- `require_frontmatter`: a list of keys a file must have to count as a document (for example `["genre"]`)
- `field_map`: title, date, genre, topics, lang, summary, author mapped to frontmatter keys
- `default_author`
- `boilerplate`: regexes
- `link`: a template, see below

The public `scout corpora init-defaults` command writes an empty registry. The maintainer preset (`scout corpora init-defaults --preset dominik`) writes these entries:
- `writing` = `~/gitrepos/02_writing-creation/writing`, requiring `genre`, excluding `cache/**`, `_sources/**`, `indexes/**`, `docs/**` and `tweets/**`; default author "Dominik Lukeš".
- `tweets` = the same repo's `tweets/`.
- `highlights` = the archive path read from `~/.config/highlight-scout/config.toml`.
- `learninglog` = `~/gitrepos/01_reading-research/_LEARNINGLOG`.
- `talks` = `~/gitrepos/03_presenting-teaching/dominiks-presentations-library`, excluding `_SLIDE-VERSIONS/**`, `_PRESENTATIONS/**`, `z-old-powerpoint-imports/**` and `**/bundle/**`.
- `slide-versions` = the presentations library's `_SLIDE-VERSIONS/`, as a separate corpus.
- `abstracts-archive` = `~/gitrepos/03_presenting-teaching/dominiks-session-abstracts-archive`, excluding `cache/**`.
- `york-edit-pass` = `~/gitrepos/03_presenting-teaching/york-2026-edit-pass`.
- `ai-foundations-edit-pass` = `~/gitrepos/03_presenting-teaching/ai-foundations-2026-edit-pass`.

The six discovery corpora use `markdown-folder`, include `**/*.md` and require no frontmatter. Existing registries are left untouched unless `--force` is supplied; register additions with `scout corpora add` on an already configured machine. Exclusions use the existing glob list, matched against corpus-relative paths. `**/bundle/**` excludes both root and nested bundle folders. Hidden path components are always skipped.

## Crate layout (scout-core workspace)

- **`crates/scout-corpus`** (new). This is a deep module; its public surface is roughly:
  - `Corpus::open(id)`, `build_index()`, `status()`
  - `search(query)`, `kwic(term, opts)`, `collocates(node, opts)`, `ngrams(opts)`, `profile(word, opts)`, `distribution(term, by)`, `verify_quote(text)`
- **`crates/scout-query`** (new): a Rust port of the `@scout/query` grammar.
  - The TS package stays; both must pass ONE shared fixture file, `packages/scout-query/fixtures/grammar-cases.json`. Generate it from the TS tests first, then make Rust pass it.
  - Grammar: `OR`, `AND`, `-x`, `"phrase"`, `prefix*`, `/regex/`, and the fields `au: ti: ty: tag: co: after: before: y:`, plus the new `in:<corpus>`, `lang:`, `genre:`, and (2026-10-11) the topic-layer fields `theme: entity: event: ai: month:` (see "Topic layer").
- **`crates/scout-cli`** (new), binary `scout`.
- **`scout-index`** and **`scout-archive`**: reuse them for FTS and highlight-archive reading. Do not break Highlight Scout (it depends on the tag v0.1.0; bump to a new tag, and do not move the old one).

## Documents, passages, tokens

- **Document** = one source file (markdown-folder) or one work (highlight archive). It carries the metadata above.
- **Passage** = one paragraph (a blank-line-separated block; a list item or heading is its own block). It stores the original text, the source line start and end, and the normalised text. FTS runs over passages, and search returns passage hits grouped by document.
- **Normalisation** (index time, never on disk in sources):
  - HTML entity decode (`&amp;` and so on, including double-encoded);
  - strip HTML tags;
  - unify curly and straight apostrophes and quotes;
  - Unicode NFC;
  - strip Markdown link targets and bare URLs;
  - drop lines matching the corpus boilerplate regexes.
- **Tokeniser:** Unicode word segmentation (`unicode-segmentation`), lowercase, apostrophe-internal words kept (`don't`). Czech diacritics are letters (`metafora`, `příklad` are single tokens). There is no lemmatisation in v1. Leave a `Lemmatizer` trait with an identity implementation, as a seam for Czech lemmas later.
- **Probe baseline (2026-09-27, naive Python, English pieces):** ~1,545 docs, ~1.7M tokens; "metaphor|metaphors" 2,628; "scaffolding" 64 in 30 docs. After normalisation, "amp amp amp" must be absent from the n-grams and "don't" must be one token. Use these as smoke expectations; expect the exact numbers to shift.

## Analytics definitions (fixed, so the views agree)

- **KWIC:** every token match of the term (a word, or a phrase = consecutive tokens). Context is N tokens left and right (default 8), rendered from ORIGINAL text by offset mapping. Sort keys: `L3 L2 L1 R1 R2 R3`, `date`, `source`. Filters: `in: lang: genre: after: before: y:`.
- **Collocates:** node = the term; window ±W (default 5) within a passage; stopwords optional (built-in English list; Czech list later).
  - Scores: logDice = 14 + log2(2·f(n,c) / (f(n)+f(c))); MI = log2(f(n,c)·N / (f(n)·f(c)·W₂)), where W₂ = window span.
  - `min_freq` defaults to 5.
  - Each row carries a count equal to its concordance lines (invariant 3).
- **Compare** (J5-A, writing vs highlights): the same node, window and score computed over two corpora or filters, returned side by side, plus a per-collocate ratio.
- **N-grams:** n in a range (default 3–5), within a passage. By default, only grams made entirely of stopwords are removed. The stricter rule (drop grams with ≥ n−1 stopwords) is the opt-in `--strict-stopwords`, because the default must keep real phrases such as "at the same time" and "the difference between" (amended 2026-09-27 after the mockup round). `--since/--until` gives counts per period.
- **Profile** (J5-B): collocates are a plain logDice list; collocates by grammatical relation are handed to Sketch Engine (amended 2026-09-27). The profile shows frequency plus per-year distribution, the top collocates (logDice), the top n-grams containing the word, and the top documents by relative frequency.
- **Distribution:** hit counts by year, corpus, genre or lang.
- **verify_quote:** is the exact string (after quote and apostrophe unification only) found in the original text of any passage? Returns the document, the line and the matched original text. This is what the report recipe (J8-B) uses to machine-verify every quote.

## Citation and link (J1)

- `cite` output for a passage hit, in two forms:
  - `markdown`: `> original passage or sentence` + `— Author, *Title*, 23 June 2016` + the link;
  - `plain`: the same without Markdown.
- The date is formatted as "D Month YYYY" (British style). An unknown date is omitted, never guessed.
- The link template per corpus kind:
  - writing and tweets: `writeflex://open?path=<abs-path-urlencoded>&line=<n>`. WriteFlex opens `path` today and ignores `line`; line jumping is a WriteFlex follow-up, not this spec.
  - highlights: the Highlight Scout deep link if one exists (check `highlight-scout/src-tauri`); otherwise the work file's `file://` path. Report which.
- Sentence selection: the hit sentence within the passage (a simple sentence split); `--passage` gives the whole paragraph.

## CLI (J9): exact surface

```
scout corpora list|init-defaults|show <id>
scout index build [<id>…] [--force]      # incremental by mtime+hash; prints docs/passages/tokens
scout index status                        # per corpus: docs, passages, tokens, built-at, stale files
scout capture --project <folder> --kind <kind> --title "<title>" [--prompt "<text>"] [--source <scout:id|url|file:path>]… [--artefact <path>] [--slug <slug>] [--by <who>] [--json] (<file.md> | -)
scout capture --list --project <folder> [--json]
scout capture --where-used <file> [--json]
scout search <query> [--in a,b] [--limit 20] [--cite markdown|plain] [--json]
scout kwic <term> [--in …] [--width 8] [--sort R1] [--limit 200] [filters] [--json]
scout collocates <node> [--in …] [--window 5] [--score logdice|mi] [--min 5] [--top 30] [--compare <corpus>] [--json]
scout ngrams [--in …] [--n 3-5] [--since Y] [--until Y] [--top 50] [--json]
scout profile <word> [--in …] [--json]
scout dist <term> --by year|corpus|genre|lang [--json]
scout verify-quote <text> [--in …] [--json]
scout cite <passage-id> [--format markdown|plain]
scout sync --status                       # reads Highlight Scout config: per-source last sync; read-only
scout facets <query> [--in …] [--top 20] [--json]                       # topic-layer counts over a query's result set
scout topics themes|entities|events|months [--month YYYY-MM] [--in …] [--top 0] [--json]
```

- Human output is compact and aligned (J9 frame).
- `--json` output is a stable schema, documented in `docs/cli-json.md`, with a `schema_version`.
- Exit codes: 0 ok, 1 no results, 2 usage, 3 index missing (the message names the `scout index build` command).
- The `sync --status` line matches J9, for example: `readwise   732 new · last 2026-07-22 · off`. "N new" needs the Readwise API; if no token is readable without prompting, print `last <date>` only. **Never print secrets.**

### Research capture (2026-10-20)

`scout capture` is the explicit source-writing verb. The deep module behind
`Engine::capture`, `capture_list` and `capture_where_used` owns input reading,
project containment, frontmatter, collision-safe creation, artefact copying,
manifest append and the containing corpus's in-process incremental build. The
CLI only parses and formats. Indexes stay derived, disposable and outside
source folders. Capture may succeed without a corpus or after an index failure;
its JSON reports the saved file and the manifest/index outcomes. The full
research folder, six-field JSON, listing and where-used contract is in
`docs/cli-json.md`, “`scout capture` — research-folder contract”.

## Performance targets (Macek, warm index)

- search < 150 ms
- kwic over writing < 500 ms
- collocates < 1.5 s
- ngrams 3–5 over writing < 3 s
- full `index build` of writing < 60 s; incremental rebuild with no changes < 3 s

Measure and report the actuals; a missed target is a finding, not a blocker.

## Agent skill (J8-B, J9)

- `~/gitrepos/_SKILLS/skills/<pick the fitting category>/scout-archive/SKILL.md`, published only through `_SKILLS/tools/publish.py`.
- Contents:
  - when to use it (any question about what Dominik wrote, tweeted or highlighted);
  - the command surface;
  - "always verify-quote before quoting";
  - the **archive report recipe**: question → search, kwic and dist to find evidence → read the passages → draft → `verify-quote` every quotation → HTML report where each quote carries a link → a report file saved under the writing repo's `docs/reports/` or wherever the user says.
- Private paths appear only in the skill's private layer. The public release of ArchiveScout later needs a generic version.

## Tickets (tracer bullets, one Opus session each)

| # | Ticket | Blocked by | Done when |
|---|---|---|---|
| T1 | Registry, markdown-folder adapter, normalisation, tokeniser, passages, FTS build; `scout corpora`, `scout index build/status`, and plain `scout search` + `--cite` over **writing** | — | `scout search "generative metaphor" --in writing --cite markdown` returns the 2016 "Repaved paths…" essay with an original-text quote and a writeflex link; the sources' git status is unchanged; a rebuild is identical |
| T2 | `scout-query` Rust port + shared fixtures; full grammar in `scout search`; `tweets` and `highlights` corpora; `--in` across corpora | T1 | TS and Rust pass the same fixture file; the three corpora are searchable together |
| T3 | KWIC, dist, ngrams, profile(freq + dist) + `verify-quote` | T1 | invariant 3 holds in tests (KWIC count = unigram count = profile freq); "amp amp amp" is absent; `verify-quote` finds the ZPD/scaffolding sentence |
| T4 | Collocates (logDice, MI, compare), profile completes (collocates + ngrams) | T3 | each collocate count = its concordance line count (test); `--compare highlights` works |
| T5 | `scout sync --status`, `docs/cli-json.md`, skill + report recipe, published | T2, T4 | the skill is installed; one real report run end to end (Fable drives it with Dominik's question) with every quote verified |

**Review:** new public interface and a cross-repo contract, so one review pass per ticket merge. Fable reviews directly (Opus build; no write path, since sources are read-only and indexes live in the app-data dir). Invariant 1 is checked by Fable's sweep on every ticket.

## Topic layer (2026-10-11)

Model-made topic tags beside a corpus (`docs/specs/2026-10-10-topic-tagging-layer.md`). Plumbing for the ArchiveScout topic features (proposal `archive-scout/docs/plans/2026-10-11-topic-layer-in-archivescout-proposal.md`); no UI here.

- **Files (read-only, in the corpus folder):** `analysis/topics/current.jsonl` and the optional, append-only `analysis/topics/corrections.jsonl`. Invariant 1 holds: neither is ever written.
- **Module:** `crates/scout-corpus/src/topics.rs`. `read_layer(root)` parses both files (corrections applied); `index::build` calls `topics::refresh`, which copies the layer into the index table `topic_docs (rel_path, key, matched_by, info JSON)` inside the build transaction; `filter::load_docs` attaches each document's tags (`DocInfo.topic`); `FacetCounter` counts.
- **Join:** by key = the frontmatter `source_system:source_id` (stored as `documents.source_id`, ENGINE_VERSION `scout-corpus/8`; highlights works, and markdown files that carry both keys; a tweet's key is `x:<id>`), else the layer's `path` = document key. A key match is never displaced by a path match. The build report carries the join rate (`topics` in `index build --json`).
- **Refresh:** re-join when the layer files' bytes (sha1 in index meta) or the documents changed; the layer is not in the fingerprint, so a new layer never forces a full rebuild. A missing layer file empties the table and reports nothing (not an error). Invariant 4 holds: a rebuild from scratch gives the same tags.
- **Reading:** streamed line by line; a line over 1 MiB or not valid UTF-8 is skipped and counted, never applied; the change check hashes the files incrementally.
- **Corrections:** per key, each field present among `themes`, `entities`, `event`, `event_name`, `ai_related` overrides; later lines win; unknown keys counted, not applied; a wrongly typed line skipped whole.
- **Grammar:** normative text in `docs/cli-json.md` ("Query grammar"): one explicit whitespace set, constructs (phrase, regex, field) recognised only at a token start, straight and curly double quotes as phrase delimiters with `\\` escapes, everything else a term. Rust `crates/scout-query/src/scan.rs` and TS `packages/scout-query/src/scan.ts` are the same algorithm; parity is proved by hand-written and by 600 generated cases.
- **Query fields** (in `DocFilter`, so search, slices and every analytics view share one definition): `theme:` / `entity:` / `event:` equality under the one fold (`normalize::fold`: NFC, quotes and apostrophes unified, whitespace collapsed, lowercase) that every text field comparison uses, `tag: co: au: ti: ty: lang: genre: source:` included, on both the stored and the query value; `ai:true|false`; `month:YYYY-MM|YYYY` on the layer month (the item's latest-highlight month). `y:` / `after:` / `before:` keep the document's own date: for a Zotero book the two differ (published 1980, read 2026-09). An untagged document fails every topic filter.
- **`tag:` is unchanged.** It matches the reader's highlight tags and the writing's frontmatter `topics`, never topic-layer themes or entities: those are model labels with their own provenance, and mixing them would silently change existing `tag:` results.
- **Facets:** over the full full-text/filter match of a query (not `--limit`, not the semantic-only part of hybrid). Values fold case like the filters, so a facet count = the document total of that query plus the field (tested). Months chronological; others count desc, value asc.
- **Search results** carry the document's tags as `topics` (absent when untagged).
- **Not here:** the briefings' `YYYY-MM-events.json` and `themes.json` are not read (the layer's `event_name` names events); entity aliases (`entity-aliases.json`) are not applied to query values.

- **Index target guard:** every build refuses an index or vector file that is a symlink or has more than one hard link (side files included), or whose real path is inside any registered corpus folder, before opening it.

## Keyness (T4)

Added 2026-09-27 in T4. The spec named "compare two slices" without defining it; this is the definition.

- **Command:** `scout keyness --a <slice> --b <slice> [--in …] [--top 40] [--min 5] [--json]`.
- **Slice:** one expression of whitespace-separated terms: `in:<ids>` or a bare corpus id, `after:`, `before:`, `y:2016` or `y:2010-2015` (inclusive, = `after:2010 before:2016`), `lang:`, `genre:`. A slice that names no corpus uses `--in` (default: every indexed corpus). The same syntax serves `collocates --compare`.
- **Counts:** two frequency lists of single tokens over the stored token streams (the tokens every other view counts). For a word: `a`, `b` = its counts in A and B; `c`, `d` = the slices' token totals.
- **Score:** log-likelihood G² (Rayson and Garside 2000): E₁ = c·(a+b)/(c+d), E₂ = d·(a+b)/(c+d), G² = 2·(a·ln(a/E₁) + b·ln(b/E₂)); a zero count contributes 0.
- **Effect size:** %DIFF (Gabrielatos and Marchi 2012) = ((a/c − b/d) / (b/d)) · 100; `null` when b = 0. Positive for A keys, negative for B keys.
- **Key for A:** a/c > b/d, a ≥ min (default 5) and G² ≥ 3.84 (p < 0.05, 1 d.f.). Key for B is symmetric (b ≥ min).
- **Output:** two lists, `a_keys` and `b_keys`, each ordered by G² desc, then word asc, cut at `--top`.
- **Stopwords stay in.** A shift in function words is a finding about style.
- **Overlap:** keyness assumes disjoint slices. The result reports `overlap_documents`, and the human output warns when it is non-zero.
- Worked check (in the tests): a = 10 of c = 1,000 and b = 5 of d = 2,000 give E₁ = 5, E₂ = 10, G² = 10·ln 2 = 6.931 and %DIFF = 300.

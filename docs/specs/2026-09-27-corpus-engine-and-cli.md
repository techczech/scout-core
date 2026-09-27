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

1. **Sources are never written.** Corpus folders are read-only to the engine. `git -C <source> status --porcelain` is unchanged after any command.
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

The initial registry is written by `scout corpora init-defaults` with three entries:
- `writing` = `~/gitrepos/02_writing-creation/writing`, requiring `genre`, excluding `cache/**`, `_sources/**`, `indexes/**`, `docs/**` and `tweets/**`; default author "Dominik Lukeš".
- `tweets` = the same repo's `tweets/`.
- `highlights` = the archive path read from `~/.config/highlight-scout/config.toml`.

## Crate layout (scout-core workspace)

- **`crates/scout-corpus`** (new). This is a deep module; its public surface is roughly:
  - `Corpus::open(id)`, `build_index()`, `status()`
  - `search(query)`, `kwic(term, opts)`, `collocates(node, opts)`, `ngrams(opts)`, `profile(word, opts)`, `distribution(term, by)`, `verify_quote(text)`
- **`crates/scout-query`** (new): a Rust port of the `@scout/query` grammar.
  - The TS package stays; both must pass ONE shared fixture file, `packages/scout-query/fixtures/grammar-cases.json`. Generate it from the TS tests first, then make Rust pass it.
  - Grammar: `OR`, `AND`, `-x`, `"phrase"`, `prefix*`, `/regex/`, and the fields `au: ti: ty: tag: co: after: before: y:`, plus the new `in:<corpus>`, `lang:`, `genre:`.
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
- **N-grams:** n in a range (default 3–5), within a passage. By default, grams made entirely of stopwords are removed, and grams with ≥ n−1 stopwords are dropped. `--since/--until` gives counts per period.
- **Profile** (J5-B): frequency plus per-year distribution, the top collocates (logDice), the top n-grams containing the word, and the top documents by relative frequency.
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
scout search <query> [--in a,b] [--limit 20] [--cite markdown|plain] [--json]
scout kwic <term> [--in …] [--width 8] [--sort R1] [--limit 200] [filters] [--json]
scout collocates <node> [--in …] [--window 5] [--score logdice|mi] [--min 5] [--top 30] [--compare <corpus>] [--json]
scout ngrams [--in …] [--n 3-5] [--since Y] [--until Y] [--top 50] [--json]
scout profile <word> [--in …] [--json]
scout dist <term> --by year|corpus|genre|lang [--json]
scout verify-quote <text> [--in …] [--json]
scout cite <passage-id> [--format markdown|plain]
scout sync --status                       # reads Highlight Scout config: per-source last sync; read-only
```

- Human output is compact and aligned (J9 frame).
- `--json` output is a stable schema, documented in `docs/cli-json.md`, with a `schema_version`.
- Exit codes: 0 ok, 1 no results, 2 usage, 3 index missing (the message names the `scout index build` command).
- The `sync --status` line matches J9, for example: `readwise   732 new · last 2026-07-22 · off`. "N new" needs the Readwise API; if no token is readable without prompting, print `last <date>` only. **Never print secrets.**

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

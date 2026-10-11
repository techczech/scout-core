---
title: "`scout --json` output schemas"
date: 2026-09-27
register: FOR AGENTS
spec: docs/specs/2026-09-27-corpus-engine-and-cli.md
---

# `scout --json` output schemas

Values in the examples are illustrative. Every `--json` document is one pretty-printed JSON object with a top-level `schema_version` (integer). A field is added without a bump; a rename, removal or meaning change bumps that command's `schema_version`.

## Library facade (apps)

Every command is one call on `scout_corpus::api::Engine` (`Engine::from_env()` = the CLI's registry and index dir; `Engine::new(registry, index_dir)` for apps and tests). Each call takes a request struct (`SearchQuery`, `KwicQuery`, `CollocatesQuery`, `KeynessQuery`, `DistQuery`, `NgramsQuery`, `ProfileQuery`, `VerifyQuoteQuery`, `CiteQuery`, `SimilarQuery`, `IndexBuildQuery`, `CleanReportQuery`, `FacetsQuery`, `TopicsQuery`, `EventsQuery`; `index_status()` and `corpora_list()` take none) and returns `Reply { body, notes }`.

- `api::to_json(&reply.body)` is byte-for-byte the command's `--json` stdout (minus the final newline); `notes` are the stderr lines; `Outcome::has_results(&body)` is exit 0 vs 1. Errors are the same typed errors the CLI maps to exit 2/3 (`IndexMissing`, `RegistryMissing`, `NoIndexedCorpus`) or 1 (`PassageNotFound`). `crates/scout-cli/tests/facade_parity.rs` holds this for every command.
- Requests are serde structs with every field defaulted to the CLI default, so partial JSON works: `{"term": "metaphor", "scope": {"in": ["writing"], "after": "2020"}}`. Rust `in_` fields serialise as `"in"`. `Scope` = `in`, `lang`, `genre`, `after`, `before`, `year`, `source`. Enumerated options stay strings as on the command line (`sort: "R1"`, `score: "mi"`, `by: "year"`, `n: "3-5"`, `dist: "doc"`).
- `collocates` returns `Collocates::Single` or, with `compare`, `Collocates::Compare` (untagged: the two JSON shapes below).
- `Engine` holds only the registry and index dir; each call opens and closes its index files. It is `Send + Sync + Clone`: call it from any thread, with no global mutable state.

## Conventions

- **Original text.** Every text field that quotes a source (`quote`, `left`, `node`, `right`, `original`) is ORIGINAL source text: Markdown, entities and curly quotes as written. Normalised text never appears.
- **Lines.** `line` is 1-based in the source file.
- **Documents and keys.** `rel_path` is the document key: the source path relative to the corpus root, plus `#<tweet id>` for a tweet (`stream/2025-07.md#1939901126326764018`; a tweets corpus has `document_unit = "tweet"`, one document per tweet). A highlights archive (`highlight-scout-archive`) has one document per work file (`readings/works/<slug>.md`) and one passage per highlight. `path` is always the absolute source FILE.
- **Passage id.** `passage_id` = `<corpus>:<rel_path>:<passage line_start>`. It is stable across rebuilds and is what `scout cite` takes (the line is split off at the last `:`).
- **Dates.** `date` is the raw value: the frontmatter date (`2016`, `2016-06` or `2016-06-23`), a tweet's UTC timestamp (`2025-07-26T10:12:00Z`), or a highlight work's publication date. `date_display` is British style ("23 June 2016"), or `null` when unparseable. An unknown date is `null`, never guessed.
- **`date_source`** (tweets and highlights; absent for writing): `published`, or `saved` when a highlight's work has no publication date and `date` is the earliest `highlighted_at` of its highlights. Publication dates: Zotero's `date` field (cut to its known precision), or a tweet's time decoded from its id; Readwise works carry none, so they are `saved`.
- **Highlight text.** A highlight's `quote` / `original` is its blockquote text with the `> ` markers removed; line numbers are the file's. Notes after a highlight (Dominik's own) and image/LaTeX records are not indexed.
- **Determinism.** Every list has an explicit order whose last tie-breaks are corpus, then path, then line (invariant 6). The same query over the same index gives byte-identical output.
- **Counts agree** (invariant 3). For the same term and filters: `kwic.total` = the n=1 (or n=phrase length) `ngrams` count = `profile.frequency` = the sum of `dist.buckets[].hits`; a collocate row's `count` = `kwic.total` of its `kwic.command` (`scout kwic <node> --near <collocate> --window W` over the same corpora and filters).
- **Default corpora.** Without `--in`, a command runs over every corpus whose index opens and prints one line on stderr naming the rest: `scout: note: using writing; not indexed: tweets, highlights`. `corpora` in the JSON lists the ones used. An `--in` naming an unindexed corpus is exit 3.
- **Exit codes.** 0 = results; 1 = none (kwic/dist/profile with zero hits, empty ngrams or collocates, no keywords, verify-quote not found); 2 = usage; 3 = index or registry missing, or no corpus indexed.

### `filters` object (kwic, dist, profile, ngrams)

Only the set keys appear; `{}` means unfiltered.

| key | type | meaning |
|---|---|---|
| `lang` | string[] | any-of (`--lang en,cs`) |
| `genre` | string[] | any-of (`--genre essay,note`) |
| `after` | string | inclusive lower bound; `YYYY`/`YYYY-MM` mean the first day of that period |
| `before` | string | exclusive upper bound; `--before 2016` = up to 31 Dec 2015 |
| `year` | int | the document's year (`--y 2016`) |
| `source` | string[] | any-of the source system: `readwise`, `x`, `zotero` (`--source x`); Dominik's own tweets are `x` |
| `kind` | string[] | any-of the document type (`tweet`, `article`, `book` …); a plural `s` is ignored (`ty:articles` matches `article`) |
| `author` | string | substring of the author |
| `title` | string | substring of the title |
| `theme` | string | topic layer: the document carries this theme (exact name) |
| `entity` | string | topic layer: the document carries this entity (exact normalised name) |
| `event` | string | topic layer: an event the document is in, by lasting id, merged-away id or monthly id (`2026-09/E21`); kept as typed (see "Lasting events") |
| `ai` | bool | topic layer: the document's `ai_related` |
| `month` | string | topic layer month: `YYYY-MM`, or `YYYY` for any month of that year |

Every text comparison (equality or substring) is made under the fold of "Query grammar" below, on both the stored value and the filter value. A document without a parseable date fails every date filter. A document without topic tags fails every topic filter, `ai:false` included. `source`, `kind`, `author`, `title` and the topic fields come from query fields (`source: ty: au: ti: theme: entity: event: ai: month:`) and slices; the analytics commands take `--source`.

### Slice expressions (`collocates --compare`, `keyness --a/--b`)

One quoted string, read by the query grammar ("Query grammar" below); bare words are corpus ids:

| term | meaning |
|---|---|
| `writing`, `in:writing,tweets` | the corpora; a slice without one uses the command's `--in` (default: every indexed corpus) |
| `after:2020`, `before:2015` (`since:`/`from:`, `until:`/`to:`) | as `--after` / `--before` |
| `y:2016` | one year |
| `y:2010-2015` | inclusive range = `after:2010 before:2016`; cannot combine with `after:`/`before:` |
| `y:2016-05`, `y:2016-`, `y:-2016` | the grammar's other date forms, as a range |
| `lang:en,cs`, `genre:essay,note` | as `--lang` / `--genre` |
| `source:zotero`, `zo:`, `ty:books`, `au:lakoff`, `ti:"metaphors we"` | the document fields above |
| `theme:"AI and mathematics"`, `entity:`, `event:`, `ai:true`, `month:2026-09` | the topic-layer fields above |

A slice refuses search terms (`-x`, `"phrase"`, `x*`, `/re/`) and the highlight fields `tag:`, `co:`, `i:`.

A slice carries only its own terms; it does not inherit the command's filter flags. So `scout collocates metaphor --after 2020 --compare "before:2015"` compares after-2020 writing (A) with before-2015 writing (B), and `--compare highlights` compares the same filters' writing with the highlights corpus. The echoed `spec` is the slice as given; for the main side of `--compare` it is the filter flags written as a slice (`after:2020`, or `all`), every value written by the grammar's value-writing rule (`ti:"GPT \\"Astra\\""`), so it parses back to the same filter. `lang: genre: source: ty:` are comma lists, so an item cannot itself hold a comma.

### Row distribution object (`--dist year|doc|corpus` on `ngrams` and `collocates`)

```json
"dist": {
  "by": "doc",
  "buckets": [{"key": "writing:blogs/x.md", "title": "…", "count": 27}],
  "other": {"documents": 41, "count": 88}
}
```

- `sum(buckets[].count) + other.count == ` the row's `count`, always.
- `year`: the years with a count, chronological, `unknown` last; no `other`.
- `doc`: the top 10 documents by count desc, then key asc; the rest are folded into `other` (absent when nothing is left over). `key` = `<corpus>:<rel_path>`.
- `corpus`: every corpus in scope, zero counts included, by count desc, then key.
- Without `--dist` the field is absent.

## Query grammar (normative)

One grammar, two implementations that must agree exactly: Rust `crates/scout-query` (`src/scan.rs`) and TS `packages/scout-query` (`src/scan.ts`). Neither uses a regular expression or a built-in whitespace class for tokenizing. Parity is held by `packages/scout-query/fixtures/grammar-cases.json` (hand-written cases, one or more per rule below) and `fixtures/generated-cases.json` (600 seeded random queries and 400 labels made by `packages/scout-query/generated.test.ts`; the Rust test `crates/scout-query/tests/fixtures.rs` must reproduce every TS result).

**WS** (whitespace) is exactly this set of code points, a constant (`WS`) in both implementations: U+0009–U+000D (tab, LF, VT, FF, CR), U+0020, U+0085, U+00A0, U+1680, U+2000–U+200A, U+2028, U+2029, U+202F, U+205F, U+3000. Nothing else separates tokens (U+FEFF does not).

**Double-quote variants**: `"` U+0022, `“` U+201C, `”` U+201D, `„` U+201E, `‟` U+201F. **Opening quotes**: `"`, `“`, `„`.

**Tokens.** The query is read left to right; WS separates tokens. A construct is recognised ONLY at a token start (the start of the query or right after WS), tried in this order:

1. **Phrase**: an opening quote, then text up to the next double-quote variant (any of them closes). Inside, `\` before a double-quote variant or before another `\` makes that character literal; any other `\` is itself. One `-` directly before the opening quote negates the phrase. The text is phrase text: nothing inside is a field, a regex or an operator. An opening quote that is never closed is not a phrase (rule 4 applies).
2. **Regex**: `/body/flags`, where `body` is one or more characters up to the next unescaped `/` (`\` escapes the next character; WS is allowed in the body), `flags` is zero or more of `a`–`z`, and the next character is WS or the end. Anything else starting with `/` is a term.
3. **Field**: `name:value`, where `name` (everything before the first `:` of the token, ASCII case ignored) is a known field name. The value is either quoted (rule 1's quote rules, no negation) or bare: everything up to the next WS, taken literally, possibly empty. An unknown name makes the whole token a term.
4. **Term**: everything up to the next WS. So `https://example.org/theme:Foo`, `km/h`, `(au:x)`, `foo-au:bar` and `-au:x` are single terms; a leading `-` negates a term; `OR`, `|` and `AND` (ASCII case ignored) are operators; `x*` is a prefix.

Whatever directly follows a closed phrase or quoted value, up to the next WS, is a plain term (rule 4) and never a construct: `theme:"v"tail` is the field plus the term `tail`.

**Writing a value** (`scout_corpus::topics::field_query(field, value)`, `scout_query::quote_value`, TS `quoteValue`): bare when the value is non-empty and holds no WS code point, no double-quote variant and no `\`; otherwise `"…"` with a `\` before every double-quote variant and every `\`. A value written this way reads back unchanged (`entity:"GPT \"Astra\""`).

**Fold (matching).** Every field value is compared under one function (`scout_corpus::normalize::fold`), applied to the stored value and to the query value alike, for every text field (`tag: co: au: ti: ty: lang: genre: source: theme: entity: event:`) and for facet grouping: Unicode NFC → apostrophe variants (`‘ ’ ‚ ‛ ′ ʼ`) to `'` and double-quote variants (plus `″`) to `"` → each run of WS to one space, none at the ends → lowercase. `tag: co: lang: genre: source: theme: entity: event:` are equality under the fold; `ty:` equality after dropping a final `s`; `au: ti:` substring. Not folded, because they are parsed values or identifiers: `after: before: y: month:` (dates), `ai:` (boolean), `in:` (corpus ids, exact). Field values are kept as typed in `plan` and in the JSON `filters`.

## `scout search <query>` — schema_version 1

The query is read by the grammar above ("Query grammar"); this is what its constructs mean in `scout search`:

| syntax | meaning in `scout search` |
|---|---|
| `generative metaphor` | every bare word prefix-matched, ANDed (T1 behaviour; also for 3+ words, unlike Highlight Scout's OR ranking); bare stopwords dropped unless every term is one |
| `"conceptual metaphor"` | consecutive tokens, exact |
| `metaph*` | explicit prefix |
| `a b OR c`, `a \| c` | OR of AND-groups (AND binds tighter); `AND` is the default |
| `-lakoff` | exclude passages with a token starting `lakoff` |
| `/metaphor(s\|ic)/i` | the passage's ORIGINAL text must match (Rust regex syntax; flags `i m s`) |
| `in:highlights` | corpora (narrows `--in`; alone it selects them) |
| `after: before: y: lang: genre: source: zo: ty: tw: bo: … au: ti:` | document filters (the `filters` object) |
| `tag:linguistics` | the highlight's tags, or the document's `topics` |
| `co:yellow` | the highlight's colour |
| `theme:"AI and mathematics"` | topic layer: documents carrying the theme (exact name; quote names with spaces) |
| `entity:"GPT-6 Astra"` | topic layer: documents carrying the entity (exact normalised name; `entity:Astra` does not match it) |
| `event:2026-09-fields-declaration`, `event:2026-09/E21` | topic layer: documents in the event. A lasting id selects that event; a monthly id (month part required) or a merged-away id selects the lasting event that holds it, so ALL that event's documents match, in every month (see "Lasting events"; a corpus without event records matches the monthly id alone, as before). Ids only: names and aliases are not matched |
| `ai:true`, `ai:false` | topic layer: `ai_related` (`yes`/`no`/`1`/`0` too; anything else is a usage error) |
| `month:2026-09`, `month:2026` | topic layer month (see "Topic layer" below); `y:`/`after:`/`before:` stay on the document's own date |

Each field takes one value; a repeated field keeps the last. `tag:` is unchanged: it never matches topic-layer themes or entities, because those are model-made and `tag:` holds the reader's own labels (Readwise tags) and the writing's frontmatter `topics`.

`i:` (has image) has no meaning here and is listed in `plan.ignored`. A query with filters or a regex but no positive term lists every passage that passes them (score 0).

```json
{
  "schema_version": 1,
  "query": "metaphor in:highlights -dead",
  "terms": [{"kind": "prefix", "tokens": "metaphor"}, {"kind": "phrase", "tokens": ["a", "b"]}],
  "plan": {
    "any_of": [[{"kind": "prefix", "tokens": "metaphor"}]],
    "not": [{"kind": "prefix", "tokens": "dead"}],
    "filters": {},
    "corpora": ["highlights"],
    "tag": "linguistics", "color": "yellow", "regexes": ["/x/i"], "ignored": ["i:"]
  },
  "corpora": ["highlights"],
  "total_documents": 12,
  "total_passages": 30,
  "results": [{
    "corpus": "writing", "rel_path": "…", "path": "/abs/…", "title": "…",
    "author": "Dominik Lukeš", "date": "2016-06-23", "date_display": "23 June 2016",
    "date_source": "published",
    "genre": "essay", "lang": "en", "kind": "book", "source": "zotero",
    "public_url": "https://medium.com/…",
    "score": 12.3, "rank": 1.0, "title_match": true, "passage_count": 3,
    "hits": [{
      "passage_id": "writing:…:12", "line_start": 12, "line_end": 13, "line": 12,
      "quote": "original sentence or passage", "score": 11.1, "link": "writeflex://open?path=…&line=12",
      "tags": ["linguistics"], "color": "yellow", "saved_at": "2024-01-15"
    }],
    "citation": {"markdown": "> …\n\n— Author, *Full Title*, 23 June 2016 · [archive](writeflex://…) · [public](https://…)\n", "plain": "“…”\n— Author, Full Title, 23 June 2016 · archive: writeflex://… · public: https://…\n"}
  }]
}
```

`plan` keys other than `any_of`, `not` and `filters` appear only when set; so do `date_source`, `kind`, `source`, `topics` and the hit's `tags`, `color`, `saved_at` (highlights).

A result whose document carries topic tags has a `topics` object (the same object in semantic and hybrid results):

```json
"topics": {
  "key": "x:1965432109876543210", "themes": ["AI and mathematics"], "entities": ["GPT-6 Astra", "Terence Tao"],
  "event": "2026-09/E21", "event_name": "Fields Medallists' declaration", "events": ["2026-09-fields-declaration"],
  "ai_related": true, "month": "2026-09",
  "confidence": "medium", "run_id": "2026-10-11-final-2026-09", "prompt_version": "topics-v2",
  "corrected": ["themes"]
}
```

`corrected` (only when non-empty) names the fields a correction overrode, in the order `themes`, `entities`, `event`, `event_name`, `ai_related`, `events` (`events` = the line carried `event_add` or `event_remove`). `events` is every event the document is in (see "Lasting events"); it is present exactly when the corpus has a records folder, and absent otherwise, when the document's one event is `event`. `event` and `event_name` stay the tag row's own monthly event; both are null when a correction took the document out of that event.

Order: `results` by `rank` desc, then `score` desc, corpus, path; `hits` by score desc, then line. `score` is BM25 (plus the title's score when the title matches); `rank` = `score` / the best `score` in the same corpus, so each corpus's best document has rank 1 and `--in a,b,c` interleaves the corpora instead of letting one corpus's BM25 scale win. With one corpus, rank order = score order.

- `public_url`: writing, the first `http(s)://` value among the frontmatter keys in `field_map.public_url` (`published_url`, then `source_url`, then `canonical_url`); a tweet, `https://x.com/<handle>/status/<id>` (the handle from the file's `venue`); a highlight work, its `url`. `null` when none.
- `link`: writing and tweets, the corpus `link` template (`writeflex://open?path=<file>&line=<n>`); highlights, the work file as `file:///…` (Highlight Scout registers no URL scheme, so there is no deep link).
- Citation lines by corpus:
  - writing: `— Author, *Full Title*, 23 June 2016 · [archive](<link>) · [public](<url>)` (each link when it exists);
  - tweets: `— Dominik Lukeš (@techczech), tweet, 26 July 2025 · [public](https://x.com/…)` (the archive link only when there is no public URL);
  - highlights: `— Author, *Title*, 1980 · [highlight](file:///…)`: the publication year, or `saved 2021` when `date_source` is `saved`.
  - `plain` writes `· archive: <link>`, `· public: <url>`, `· highlight: <link>`. An unknown date is left out.

### Semantic and hybrid search (`--semantic`, `--hybrid`, `--fts`)

Same shape, with two additive keys that appear only when vectors ranked the results, so full-text JSON is unchanged:

- top-level `mode`: `"semantic"` or `"hybrid"`;
- a hit's `semantic_score`: the passage's cosine to the query (0–1).

Modes (`SearchQuery.mode` in the facade: `auto`, `fts`, `semantic`, `hybrid`):

- `auto` (the default): hybrid for each corpus with current vectors (`scout index build --semantic`), full text for the rest. A facade `Engine` without an embedder is always full text, so apps are unaffected until they attach one (`Engine::with_embedder`).
- `semantic`: passages by cosine to the query's free words (fields such as `in:`, `lang:`, `y:`, `tag:`, `-x`, `/re/` still filter). A document scores its best passage; `score` is that cosine; hits quote the whole passage's original text.
- `hybrid`: the top 100 documents of the full-text and of the semantic ranking fused by reciprocal rank, `score` = Σ 1 / (60 + rank); a document's hits are its full-text hits, then its semantic ones. `total_documents` = full-text matches plus semantic-only documents in the pool.
- A corpus whose vectors are stale (the index changed since, or another model) is searched as full text, with the note ``scout: note: full text only for <id> (<why>); run `scout index build --semantic` ``.

Vectors live in `<corpus>.vectors.sqlite` beside the index. `index build --semantic --json` adds a `vectors` object to each report: `vectors_path`, `model`, `dim`, `full`, `passages`, `embedded`, `reused`, `removed`, `bytes`, `elapsed_ms`. Once a corpus has vectors, every `index build` updates them (only new or changed passage text is embedded). The model (default `minilm-l12`, `Xenova/paraphrase-multilingual-MiniLM-L12-v2`, about 490 MB; chosen by the ticket 07 recall test) is downloaded once on first use to `$HF_HOME` when set, else `<data dir>/scout/models`; `SCOUT_EMBED_MODEL` picks another (`e5-small`, `e5-base`). Passages under 5 tokens (headings, link lines) get no vector.

## Topic layer

A corpus folder may hold a topic layer: `analysis/topics/current.jsonl` (one model-tagged item per line, keyed `<source_system>:<source_id>`; format in `docs/specs/2026-10-10-topic-tagging-layer.md`) and the optional, append-only `analysis/topics/corrections.jsonl`. Both are read, never written.

- **Build.** `scout index build` copies the layer into the derived index, joined to documents by key (the work's frontmatter `source_system` + `source_id`), else by the layer's `path` = the document key. It re-joins when either file's bytes or the corpus's documents change; a new layer never forces a full rebuild. No layer file = no tags are joined and the report has no `topics`; event records are still read (see "Lasting events").
- **Corrections.** One JSON object per line with `key` and any of `themes` (string list), `entities` (string list), `event` (string or null), `event_name`, `ai_related` (bool or null). Each field present replaces the layer's value for that key; later lines win; other keys (`by`, `date`, `note`) are provenance and are not read. A correction for a key the layer lacks is counted (`corrections_unmatched`), not applied; a line with a wrongly typed field is skipped whole (`corrections_bad_lines`). An event changed without an `event_name` takes the name the layer gives that event id. In a corpus with a records folder (and only there, see "Lasting events") two further fields put an item in more events or take it out of one: `event_add` and `event_remove`, each a list of event ids (lasting, merged-away or monthly; a single string is a wrongly typed field). Exact shape: `{"key": "x:1965432109876543210", "event_add": ["2026-09-gemini-launch"], "event_remove": ["2026-09/E21"], "by": "Dominik", "date": "2026-10-11"}`. They apply in line order to the item's events (see "Lasting events"), the later line winning; within one line removals follow additions. A removal also takes the item out of an event the model gave it.
- **Month.** The item's `date` (its latest highlight) cut to `YYYY-MM`, else the layer's own `month` when that is `YYYY-MM`. `month:` filters on it and the `month` facet counts it.
- **Unparseable layer lines** (the file may be mid-write) are skipped and counted (`bad_lines`); a key repeated in the layer keeps its last line.
- **Matching** for themes, entities and event ids is equality under the fold of "Query grammar" (`entity:O'Reilly` matches `O’Reilly`). So every facet value, written with `field_query`, gives back exactly its `count` as `total_documents` (an event value that carries `corpus`: with `--in` that corpus) (tested over 400 generated labels with every whitespace code point, curly and straight quotes, apostrophes, backslashes, slashes and colons).
- **Reading limits.** Both files are streamed line by line; a line over 1 MiB (`topics::MAX_LINE_BYTES`) or not valid UTF-8 is skipped and counted (`bad_lines`, `corrections_bad_lines`), never applied.
- **Keys.** A highlights work's key comes from its frontmatter `source_system` + `source_id`; a markdown-folder document's from the same frontmatter keys when it has them (which also sets its `source:`); a tweet's is `x:<tweet id>`.

## Lasting events

A corpus folder may hold event records beside the topic layer: `analysis/events/<id>.md`, one Markdown file per event (spec: `archive-scout/docs/specs/2026-10-11-lasting-events-spec.md`). Searching and `scout index build` read them and never write them. Two things write them: `scout events seed` (below), which only ever creates new record files, and the edit commands ("Editing event records": `scout events set`, `merge`, `unmerge`, `apply-proposals`, `decide`, `remove`, `restore`, `create`), which replace or create one record file per command and append to two log files in the same folder. The records do not depend on the topic layer: a corpus with records and no `analysis/topics/current.jsonl` has them indexed all the same, each with no items (`scout events --min-items 0` lists them).

- **Without the folder** (no `analysis/events/` directory, or a file of that name) nothing in this section applies, and search, facets, topics and the build report are byte for byte what they were before lasting events. An item has one event, its tag row's `event`, matched by its monthly id. A tag row's `events` list and a correction's `event_add` / `event_remove` are not read: they are unknown keys, ignored whatever their type (a wrongly typed `event_add` does not make the line a bad line), and `corrected` never names `events`. No result carries `topics.events`. The folder existing is the whole test: an empty one turns all of this section on. This is the one statement of the rule; every other mention of these fields in this document assumes the folder. (`scout events` and `--min-event-items` are new and work on monthly events too.)
- **Record.** YAML frontmatter between `---` lines, then the description. Fields: `id` (must equal the file name without `.md`), `name` (absent: the id), `aliases` (list), `start`, `end`, `members` (list of the monthly event ids folded into the event, `2026-09/E21`), `themes` (list), `origin`, `edited`, `merged_into` (the id of the record this one was merged into; that one field is the whole of a merge, see "Merged events"), and the optional `removed` (the date he removed the event, written by `scout events remove`; absent on every other record, and the writer writes no `removed:` line for a record without it, so a record written before the field existed is unchanged). An empty value, `~` or `null` is no value. Other keys are ignored. Only files named `*.md` that do not start with `.` are records; any other file in the folder is left alone.
- **Resolution.** A monthly id resolves to the record that lists it in `members`; a record with `merged_into` resolves to that record, followed to the end of the chain, and so do its members. A monthly id no record lists stays an event of its own under its monthly id.
- **Merged events.** A merge is one field of one record: `merged_into` on the record merged away. The record merged into holds no copy of the other's members, aliases or dates, and is not written by a merge or its undo. What the combined event shows is worked out from the records each time it is asked for:
  - which event an id or an item is in: by the resolution above (the merged-away record keeps its `members`; they resolve through it);
  - its span: the surviving record's own `start` and `end`, covering every record merged into it, directly or through a chain. `start` is the earliest `start` of them all (a survivor without one takes the earliest). `end` is the latest of the survivor's `end` and each merged record's last known date (its `end`, else its `start`); a survivor with no `end` keeps none (the event is open). A date of a merged record that is not `YYYY-MM-DD` is passed over, and a date of the survivor that is not one is left as it is. One function does this (`events::cover`);
  - its aliases: the survivor's own, as they are, then the name and the aliases of each merged record, the records in id order, leaving out any that folds to the survivor's name or to an alias already listed (`events::combined`);
  - its name, description, themes, `origin`, `edited` and `removed`: the surviving record's own.

  `scout events` lists these (`start`, `end`, `aliases`, `merged_from`). No query matches on names or aliases, and no query reads an event's dates. A corpus where no record has `merged_into` lists exactly as it did before spans and aliases were derived, with no new key.
- **Removed events.** A record with `removed` set is kept and still holds its ids: its members resolve to it, so `scout events seed` makes no second record for them. But it is no event of any item: after the item's events are worked out (below), every id that resolves to a removed record is dropped, whoever put it there (the tag row, its `events` list or a correction), and an item whose tag row's `event` is a removed event has `topics.event` and `topics.event_name` null (`corrected` does not name `event` for that). The item keeps its themes, entities and other events. So `event:<id>` of a removed event, or of anything that resolves to it (its members, records merged into it), matches nothing; it is no value of the event facet or of `scout topics events`; and `scout events` leaves it out unless `--removed` is given. `removed` counts on the record an id resolves to: on a record that is merged into another it has no effect (the edit commands refuse to remove such a record).
- **An item's events.** The tag row's `event`, plus the ids in the tag row's optional `events` list (strings; monthly or lasting ids), plus `event_add` and minus `event_remove` from `corrections.jsonl`, each id resolved. An item counts once in each of its events. A search result shows them as `topics.events`.
- **Whose event an id is.** Records and the id table are per corpus. `event:<id>` over several corpora resolves the id in each corpus and matches the documents of each corpus's own event, and `scout events` lists events per corpus (the same id in two corpora is two rows), with or without records. For the event values of `facets` and `topics events` there is one rule, stated here and nowhere else. When at least one of the searched corpora (the reply's `corpora`) has a records folder, an event belongs to its corpus: the same id in two corpora (a lasting id, or a monthly id such as `2026-09/E21`) is two values, each counted, named and sized within its own corpus, and when more than one corpus is searched every event value carries `corpus`. When none of the searched corpora has a records folder, the event values are exactly what they were before lasting events: one value per id over the searched corpora together (the same monthly id in two corpora is one value, its count the sum, its name the most frequent `event_name` over all of them), and no value carries `corpus`. A corpus has a records folder when it had one at its last `index build`.
- **Engine `scout-corpus/11`.** (Unchanged by derived spans and aliases: the index holds the records as their files have them, and `scout events` derives from that copy.) The index copy of a record carries `removed` when the record has it, and a removed event is taken out of its items' events as above. An index built by an earlier engine is rebuilt in full at the next `index build`. For a corpus where no record has `removed`, the rebuilt index and every reply are what they were.
- **Behaviour change (engine `scout-corpus/10`).** In a corpus with event records, `event:2026-09/E21` no longer selects only the documents tagged with that monthly event: it selects every document of the lasting event that holds `2026-09/E21`, across all its months. Add `month:2026-09` to narrow to one month. Facet values and `scout topics events` values are lasting ids where a record holds the event. `plan` and `filters` keep the id as typed.
- **Bad records.** A file is skipped, with a warning naming it, when it is not a plain file (a symlink or folder), is over 1 MiB (`events::MAX_RECORD_BYTES`), is not UTF-8, holds a control character anywhere in the file, frontmatter or body (the warning reads `control character in record`; the characters are listed below), has no frontmatter, has frontmatter that is not exactly one YAML document read without error to its end (a document end marker `...` on a line of the frontmatter is refused, with or without anything after it, as is a second document `--- x`, so no part of the block goes unchecked), has frontmatter that is not a YAML mapping of plain keys (aliases `*x` and repeated keys are refused), has YAML nested deeper than 3 levels (`events::MAX_YAML_DEPTH`: the mapping, a list value, and one level inside a value of a key that is not a record field; the warning reads `bad YAML (nested deeper than 3 levels)`, or the YAML scanner's own `recursion limit exceeded` for more than 255 open `[`/`{`), has no `id`, has an `id` that is not the file name, repeats an id already read (ids compare under the fold), or has a wrongly typed field (`members` not a list, `name` a list). The other records are read. A `merged_into` that names no record or closes a cycle is warned about and that record is treated as not merged. A member already held by another event stays with the first record in id order, with a warning. The control characters refused are: every C0 control (U+0000 to U+001F) other than TAB, LF and a CR that is directly followed by LF; DEL (U+007F); the C1 controls (U+0080 to U+009F); U+2028 and U+2029; and U+FEFF, except one byte-order mark at the very start of the file, which is dropped as it always was (a second one, or one anywhere else, refuses the record). The whole file is checked before the frontmatter is looked for or any YAML is read, so a line of a record ends at LF and nowhere else, for this reader's own line checks and for the YAML scanner alike (YAML also ends a line at a bare CR and stops reading at a raw NUL). The rule is about the file's characters: one of them written as an escape inside a double-quoted value (`name: "a\0b\u2028c"`) is read as that character. An explicit YAML tag is not applied: every scalar is the text as written (`id: !!binary YQ==` is the id `YQ==`). This reader is the only implementation of the record format: `scout events seed` reads the records already in its out folder with it and passes every record it is about to write through it, so there is no second set of rules to keep in step. The cases are in `crates/scout-corpus/tests/fixtures/event-records/` (`verdicts.json` and `records/`), run by `crates/scout-corpus/tests/event_record_fixtures.rs`; a new rule needs a case there.
- **Build.** The records are copied into the derived index, with or without a topic layer, and re-read when any record file's name or bytes change. `index build` reports them (see `index build`).
- **Reading is not recursive.** The frontmatter is read by pulling YAML parser events one at a time and the read stops at the first value nested too deep, so a record nested 100,000 levels deep costs a warning, never the stack. Merge chains are followed in loops bounded by the number of records.

## `scout facets <query> [--in …] [--top 20] [--min-event-items 1] [--json]` — schema_version 1

Topic-layer counts over the documents `scout search` matches for the same query: the full-text and filter match, all of it (no `--limit`; semantic-only documents of hybrid search are not included). Facade: `Engine::facets(&FacetsQuery { query, in, top, min_event_items })`. Exit 1 when no document matches; 2 for an empty query or a bad field value.

```json
{
  "schema_version": 1,
  "query": "event:2026-09/E21",
  "plan": {"any_of": [], "not": [], "filters": {"event": "2026-09/E21"}},
  "corpora": ["highlights"],
  "total_documents": 41,
  "tagged_documents": 41,
  "top": 20,
  "facets": {
    "theme": {"distinct": 9, "items": [{"value": "AI and mathematics", "count": 38}]},
    "entity": {"distinct": 57, "items": [{"value": "Terence Tao", "count": 12}]},
    "event": {"distinct": 1, "items": [{"value": "2026-09/E21", "count": 41, "name": "Fields Medallists' declaration"}]},
    "month": {"distinct": 1, "items": [{"value": "2026-09", "count": 41}]},
    "ai": {"true": 41, "false": 0}
  }
}
```

- Each document counts once per value. `tagged_documents` = documents with topic tags; the rest count only in `total_documents`.
- `theme`, `entity`, `event`: count desc, then value; at most `--top` items (0 = all); `distinct` = values before the cut. `month`: every month, chronological. An event item's `value` is the lasting id where a record holds the event, else the monthly id; its `name` is the record's `name`, else the most frequent `event_name` of the counted documents tagged with that monthly id. A document counts once in each of its events.
- Themes, entities and months are counted across the corpora. Whether events are counted per corpus or across them follows the rule in "Lasting events" ("Whose event an id is"). Per corpus (some searched corpus has a records folder) and more than one corpus in `corpora`: every `event` item carries `"corpus": "<id>"` after `name`, and the same id in two corpora is two items, each with its own count and name: `{"value": "2026-09/E21", "count": 3, "name": "…", "corpus": "highlights"}`; ties in count and value are ordered by corpus; the count of an item with `corpus` is the `total_documents` of `event:<value>` run with `--in <corpus>` (without `--in` the query matches that id in every searched corpus). Otherwise (one corpus in `corpora`, or no searched corpus with a records folder) no item has the key and an item's count is the `total_documents` of `event:<value>` over the searched corpora, so that output is unchanged from before lasting events.
- `--min-event-items N` (default 1) leaves out of the `event` facet every event with fewer than N items. An event's size is its tagged documents with an indexed passage over the whole of its own corpus (the `items` of `scout events` for that corpus and id; sizes are never added up across corpora), not its count in this result. (Where no searched corpus has a records folder an event value spans the searched corpora, and so does its size: the sum of those `items`.) So `--min-event-items 2` hides one-item events, and an event of five items is kept even when one of them matches. `distinct` counts the events kept. Above 1 the reply carries `"min_event_items": N` after `top`; at 1 the key is absent.
- A value's displayed spelling is its most frequent one among the counted documents (ties: alphabetically first).

## `scout topics themes|entities|events|months [--month YYYY-MM] [--in …] [--top 0] [--json]` — schema_version 1

Every value of one field with its document count, over the tagged documents that have at least one indexed passage (the documents search can return), optionally in one layer month (`--month 2026-09` or `--month 2026`). Facade: `Engine::topics(&TopicsQuery { field, month, in, top })`. Exit 1 when no value is found; 2 for an unknown field or a bad month.

```json
{
  "schema_version": 1,
  "field": "event",
  "month": "2026-09",
  "corpora": ["highlights"],
  "tagged_documents": 1180,
  "distinct": 34,
  "items": [{"value": "2026-09/E21", "count": 41, "name": "Fields Medallists' declaration"}]
}
```

`field` is `theme`, `entity`, `event` or `month`; `items` are ordered as in `facets` (months chronological); `--top` 0 keeps all. Event values and names are as in `facets` (lasting ids where a record holds the event), and so is `corpus`, by the same rule ("Whose event an id is"): where a searched corpus has a records folder and more than one corpus is searched, every event item carries it and the same id in two corpora is two items; where none has, the same id is one item without it.

## `scout events [--month YYYY-MM] [--in …] [--min-items 1] [--merged] [--removed] [--top 0] [--json]` — schema_version 1

Every event of the searched corpora with its record fields and counts. Facade: `Engine::events(&EventsQuery { month, in, min_items, merged, removed, top })`. Exit 1 when no event is listed; 2 for a bad month.

```json
{
  "schema_version": 1,
  "month": null,
  "corpora": ["highlights"],
  "min_items": 1,
  "merged": false,
  "distinct": 1694,
  "events": [{
    "corpus": "highlights",
    "id": "2026-09-fields-declaration",
    "name": "Fields Medallists' declaration",
    "record": true,
    "start": "2026-09-01",
    "end": null,
    "items": 44,
    "months": [{"month": "2026-09", "count": 41}, {"month": "2026-10", "count": 3}],
    "own_start": "2026-09-04",
    "members": ["2026-09/E21"],
    "aliases": ["Fields declaration", "Mathematicians' backlash"],
    "merged_from": [{"id": "2026-10-mathematicians-backlash", "name": "Mathematicians' backlash"}],
    "themes": ["AI and mathematics"],
    "origin": "model",
    "edited": null,
    "merged_into": null,
    "description": "…"
  }]
}
```

- `items` = tagged documents with an indexed passage that are in the event (all months), each document once per event; `months` = those items by layer month, chronological (an item without a month is in `items` only). `event:<id>` gives `items` as `total_documents`, and `event:<id> month:<m>` a month's count.
- `record` is true when `analysis/events/<id>.md` holds the event; `members`, `themes`, `origin`, `edited`, `merged_into` and `description` (the file body) are that record's own.
- `start`, `end` and `aliases` are the event's ("Merged events"): the record's own, covering and taking in the records merged into it. `merged_from` lists those records, direct and chained, in id order, each `{id, name}`; the key is absent when there are none. `own_start` / `own_end` give the record's own date and are present only where `start` / `end` differs from it (`"own_start": null`: the record has no start and the event's comes from a merged record). In the example the event starts on 1 September because a record merged into it does; its own record starts on the 4th, and its one member is its own (`2026-10/E04` is in the merged record and resolves to this event). A record merged away, listed with `--merged`, is listed with its own values and no `merged_from`.
- An event nothing is merged into has none of the three keys, and its `start`, `end` and `aliases` are its record's. A monthly event no record holds is listed with its monthly id, `record: false`, the layer's name, null dates and empty lists.
- `--month` keeps the events with at least one item in that month (`YYYY-MM`, or `YYYY` for any month of the year) and adds `month_items` (that month's count) to each; `items` and `months` stay whole.
- `--min-items N` leaves out events with fewer items (`2` hides one-item events; `0` also lists records without items, which is every record of a corpus that has no topic layer). `--merged` also lists records merged into another (`merged_into` set; their items belong to the event they resolve to, so `items` is 0); without it they are not listed. `merged_into` is the value in effect: null for a record whose file names a missing record or closes a cycle. `--removed` also lists removed records (their items no longer have the event, so `items` is 0); without it they are not listed. Only then does the output differ from before the field: the reply carries `"removed": true` after `merged`, and a removed record's row carries `"removed": "<date>"` after `merged_into`. Without the flag neither key is present.
- Order: `items` desc, then `id`, then `corpus`. `distinct` = events before `--top`. Events are per corpus; two corpora never share one.

## `scout events seed --archive <path> [--out <dir>] [--apply]`

Creates one record file for every monthly event that no record holds yet (spec "Where lasting events come from", step 1). No model call, no network, no registry and no index: it reads an archive folder and writes record files. Facade: `scout_corpus::event_seed::seed(&SeedOptions { archive, out, apply })`. Exit 0 with one JSON line on stdout; 2 for any refusal below, with the reason on stderr and nothing on stdout.

- **Reads** (never writes): `<archive>/analysis/context/briefings/<period>-events.json`, period `YYYY-MM` or `YYYY-Qn` (plain files directly in that folder; a name holding `.superseded-` is ignored), `<archive>/analysis/topics/current.jsonl` (tag rows), and the records already in the out folder. The tag rows are read by the function `scout index build` reads them with (`topics::read_tag_rows`), so the seed and the index keep the same rows: a line that is not valid UTF-8, is over 1 MiB, is not a JSON object, has a wrongly typed field or has no `key` is passed over without changing anything, and the last valid row of a key counts. Corrections (`corrections.jsonl`) and a row's `events` list are not read: an event that only a correction names gets no record from the seed. The briefing files have no other reader than the seed.
- **Monthly events** are every event of every briefing file, tagged or not, plus every id `<period>/E<n>` that only a tag row names. Ids are compared as the index compares them (the one fold of `event:` matching: letter case and surrounding whitespace do not count), so a tag row's ` 2026-09/e01 ` and a briefing's `e01` are `2026-09/E01`, the spelling written to `members`; `E1` and `E01` are two events, as in the index. A row's date counts when it starts `YYYY-MM-DD` with a month the index accepts. One the out folder's records already hold (the reader's own resolution: a member of any record) is left alone; every other gets one record.
- **A record.** `id` = `YYYY-MM-<slug>`: the event's month, or the first month of its quarter; the slug is the name in lower-case ASCII words joined by hyphens (accents taken off), at most 60 characters, `event-e<n>` when the name gives none, with `-2`, `-3` … when the id is in use (by a record, or by any entry of the out folder, names compared in lower case). `name` = the briefing's name, else the `event_name` of the first of its tag rows, in key order, that has one, else the monthly id; whitespace runs are one space. `aliases` = the briefing's. `start`, `end` = the first and last date of its tagged items, else the first and last day of its month or quarter. `members` = the one monthly id. `themes` = the themes at least a third of its items carry, most frequent first, then alphabetical, at most 5. `origin: model`; `edited` and `merged_into` empty (nothing after the key; an empty list is `[]`). Every other value is written as a double-quoted YAML scalar on one line, in which a control character is an escape (`\x00`, `\r`, `\u2028`, `\uFEFF`): a name or alias keeps such a character and the file holds none. Body = the briefing's description as plain prose, without the characters the reader refuses in a record file ("Bad records"), since a body has no escapes: a line break that is not LF or CR LF (a bare CR, U+0085, U+2028, U+2029) becomes LF, every other refused character (the other C0 controls, DEL, the other C1 controls, U+FEFF) is dropped, and the ends are trimmed; TAB, LF and CR LF stay. The report does not count the descriptions changed this way.
- **Writes** (only with `--apply`): the out folder when it is missing (with any missing folder above it), and new files `<out>/<id>.md`, each opened create-exclusive and no-follow. An existing name is never opened for writing, replaced, renamed or removed, whatever it is (a file, a symlink, a hard link to a file elsewhere, a folder); a write that fails removes its own partial file. A second run over an unchanged archive and out folder creates nothing.
- **What is already in the out folder** is read by the records reader and by nothing else. A symlink or folder under a record name (`*.md`) is not read as a record: the reader refuses it and the seed stops. A hard-linked regular file is read like any other file: counted as a record when it is a valid one, refused when it is not; there is no link-count check (reading is harmless, and nothing reachable through a link is ever written). Entries that are not record names (no lower-case `.md` ending, or a leading `.`) are not read; their names only keep a new record from taking the same name.
- **Paths.** A `--archive` or `--out` path with a `..` component is refused before anything is looked at, with or without `--apply` (`a/../b`, `../b`, `b/..`; a name that merely holds dots, `a..b`, is not one). After a symlink, `..` is the parent of the link's target, which the path's text does not show, so the seed does not work out where such a path leads: give the path without `..`.
- **The out folder** defaults to `<archive>/analysis/events`. It must be the folder itself: a path that is a symlink (dangling or not; `link/.` and `link/` are the link too) or not a folder is refused, with or without `--apply`. Symlinks in the folders above it are accepted. `out` in the reply is the absolute path with those symlinks resolved, except in a dry run over a folder that does not exist yet, where it is the absolute path as given (a relative path is taken from the current folder). The folder is checked before the plan (and, with `--apply`, once more after it is made) and each record is then created by path: another process replacing the out folder while the seed runs is outside what it defends against (a single-user desktop tool).

```json
{"date":"2026-10-11T20:45:25.209Z","tool":"scout events seed","version":"0.1.0","archive":"/abs/archive","out":"/abs/archive/analysis/events","apply":true,"monthly_events":1712,"briefing_events":1712,"monthly_events_with_items":1694,"briefing_events_without_items":18,"already_in_a_record":0,"to_create":1712,"no_items":18,"one_item":690,"created":1712,"skipped":0,"notes":{"collisions":0,"fallback_slugs":0,"truncated_slugs":11,"non_ascii_names":2,"no_briefing_entry":0,"month_dates":18,"briefing_events_without_items":18},"collisions":[],"fallback_slugs":[],"no_briefing_entry":[]}
```

- `monthly_events` = briefing events plus ids only tag rows name; `already_in_a_record` = those a record holds; `to_create` = the rest (`no_items`, `one_item`: of those, events with no and with one tagged item); `created` = files written (0 without `--apply`, when stderr also says `dry run: nothing written; N records would be created in <out> (add --apply)`); `skipped` = `already_in_a_record`.
- `notes` counts monthly events per remark: `collisions` (the id took a suffix), `fallback_slugs` (the name gave no slug), `truncated_slugs`, `non_ascii_names`, `no_briefing_entry` (named by tag rows only), `month_dates` (dated by month or quarter), `briefing_events_without_items`. `collisions` (`{id, base, member}`), `fallback_slugs` (`{id, name, member}`) and `no_briefing_entry` (monthly ids) list the first three in full.
- **Refusals.** All but the last come before any write (the out folder is not created either), with or without `--apply`; the last, and a write that fails, stop a run that has begun writing, leaving the records made so far, each whole:
  - the `--archive` or `--out` path has a `..` component (the first check, before the archive is looked at): ``a/../b: the out path has a `..` component; give the path without it (nothing was written)``, or `the archive path` for `--archive`;
  - the archive is not a folder; the out folder is a symlink or not a folder;
  - a briefing file (any entry of the briefings folder named `<period>-events.json`) is not a plain file (a symlink or folder), is not JSON, is not a JSON object, has a `month` that is not its file name's period, has no `events` list, or holds an event that is not an object, an event id that is not `E<number>` or appears twice, a `name` or `description` that is not text, or `aliases` that is not a list of text (`name`, `description` and `aliases` may be absent or null). The error names the file: `<archive>/analysis/context/briefings/2026-09-events.json: event 2026-09/E01: "name" is not text`. No briefing file is skipped;
  - the reader skips any record file already in the out folder (every case of "Bad records": a symlink or folder under a record name, a file that is not a record whether hard-linked or not, a wrongly typed field, an id that is not the file name, a repeated id, frontmatter that is not one whole YAML document, a control character anywhere in the file …). Each such file is named with the reader's reason: `<out>/a.md: cannot read the record (id "YQ==" does not match the file name)`. Counting such a record's members as held would leave those events without a usable record;
  - the reader would skip, or read differently, a record about to be written. Every record is rendered to its bytes and given to the reader's own check (`events::parse_bytes`: the 1 MiB cap, UTF-8, control characters, the frontmatter and field rules), what comes back must equal the record meant, and the whole set, old and new, must hold no id twice and resolve every new member to its new record. Each fault names the monthly event: `2026-10/E01 ("Too long"): scout would not read its record 2026-10-too-long.md (larger than 1 MiB)`;
  - a record already in the out folder has a `merged_into` that names no record there or closes a cycle (the reader reads such a record as not merged, but its file keeps the value, and a new record of that id would take the old record and its members over at the next read). Each is named: `<out>/2026-09-alpha.md: merged_into "2026-09-beta" names no record; fix or clear merged_into first`. A `merged_into` chain that ends at a record is fine;
  - an id held before the run (a record id or a monthly id) would be held by another event once the new records are there. The out folder as it would be is put together from the old records as their files have them and the new ones, and resolved by the reader's own rules: `2026-09/e01: held by "alpha" now, and by "beta" once the new records are there`;
  - a planned file name already leads to an entry of the out folder. Which names are one name is the file system's to say (letter case; on APFS an existing `2026-09-ſtory.MD` is where `2026-09-story.md` leads), so when the out folder exists each planned name is looked up in it before the first write, and the seed works out no equivalence itself: `<out>/2026-09-story.md: this record would be created, but the name already leads to an entry of the out folder (one the file system treats as the same name, under another spelling); rename or remove that entry`;
  - (during the writes) a file of a record's name appeared after the names were looked up: it is not touched, the run stops, and running again continues.

## Editing event records

`scout events set | merge | unmerge | apply-proposals | proposals | decide | remove | restore | create | check`. Each works on the records folder of one archive, `<archive>/analysis/events`, given with `--archive <path>` (required). No registry and no index are read or written: run `scout index build` afterwards for search to see a change. Facade: `scout_corpus::event_edit::EventEditor::open(&archive)?` and one method per command, each taking typed arguments and returning the struct the CLI prints. Each command prints one JSON line on stdout and exits 0; a refusal exits 2 with the reason on stderr and nothing on stdout. Item-level changes (an item into or out of an event) are not here: they are `event_add` / `event_remove` in `corrections.jsonl`.

**What every edit does before it writes.** There is one implementation of the record format, the reader of "Lasting events", and the edits stand on it as the seed does.

- The folder is read once, by the reader. If it skips any file ("Bad records"), or a record's `merged_into` names no record or closes a cycle, the command stops and names the file.
- Each record the command changes is written by the seed's writer, read back by the reader's own check (`events::parse_bytes`) and compared with the record meant.
- The whole folder as it would be (every record as its file has it, the changed ones in place) is resolved by the reader's own rules. It must read every record, follow every `merged_into`, give no warning it does not give now, and its id table (which event each record id and monthly id resolves to) must be exactly the one the command means: unchanged for `set`, `remove`, `restore`; the same plus the new id for `create`; every id held by `a` held by `b` and nothing else moved for `merge a b`; `a`, every record merged into `a` and the monthly ids those records hold held by `a`, and nothing else moved, for `unmerge`.
- Only then is anything written. A refusal leaves every file as it was: no record, no log line, no temporary file.

**What is kept.** A command changes the fields it names and no other. Unless it sets the description (`set --description`, `create`), it rewrites only the frontmatter block: the bytes after the closing `---` line are carried over exactly as they are in the file. A record is rewritten only when its file starts with exactly the frontmatter the writer writes for it. A file with anything else there (a key scout does not read, a YAML comment, values in another YAML style, CR LF line ends, a byte-order mark) is refused: `<file>: the frontmatter is not as scout writes it …`, since rewriting it would lose what the reader does not read. The same for a record whose file is not named `<id>.md` (`x.md.md` holds the id `x`). No command creates the records folder.

**Replacing a record file.** (1) The target's permission bits are read (it must be a plain file; a link is not followed). (2) A new file named `.scout-edit-<pid>-<time>-<n>.tmp` (a name no record can have) is created in the records folder, create-new and no-follow, so it is a file of its own and not a link of either kind to another; it is created with mode 0600, given the target's permission bits on its descriptor, written and synced. (3) The target is read again, without following a link, and must be a plain file holding exactly the bytes the command read at its start; if not, the new file is removed, the target is not touched and the command stops (`changed since it was read`, or `no longer a plain file`). (4) The new file is renamed over the target, and the folder is synced. The old file is never opened for writing: the rename gives the record's name the new file. A record that has a second name (a hard link, inside the folder or outside it) is replaced under its own name only; the other name keeps the old bytes, and nothing outside the folder is written. Of the old file only the permission bits are carried over (not its owner, group, extended attributes or times). Not defended: a write to the target in the instant between (3) and (4) is lost (the rename replaces the file).

**Modes of new files.** A record made by `create`, and a log made by its first line, has mode 0644, set on the open descriptor whatever the umask.

**Paths.** As for the seed: an `--archive` path with a `..` component is refused; `<archive>/analysis/events` must be a real folder, not a symlink (symlinks above it are accepted). Another process replacing the folder while a command runs is outside what it defends against.

**Inputs.** A name, an alias or a reason is trimmed and refused when it holds a control character (`char::is_control`, U+2028, U+2029 or U+FEFF; so also a line break or a TAB): `the name holds a control character (U+000A) …`. A description is cleaned by the seed's body rule (see `scout events seed`, "A record"). A date is `YYYY-MM-DD`, a real calendar day. Ids are compared under the fold of "Query grammar".

### `scout events set <id> --archive <path> [--name s] [--description s] [--start d] [--end d] [--add-alias s]… [--remove-alias s]…`

Changes the named fields of one record and sets its `edited` to today's date (UTC). `origin`, `members`, `themes`, `merged_into` and `removed` are left as they are. `--description` replaces the body. `--end ""` clears `end`. Aliases compare under the fold; removals are made first, then additions (an alias the record already has is not added twice). Facade: `EventEditor::set(id, &FieldEdit { name, description, start, end, add_aliases, remove_aliases })`.

```json
{"tool":"scout events set","at":"2026-10-12T08:00:00.000Z","id":"2026-09-alpha","file":"/abs/archive/analysis/events/2026-09-alpha.md","changed":true,"record":{"id":"2026-09-alpha","name":"Alpha","aliases":[],"start":"2026-09-05","end":null,"members":["2026-09/E01"],"themes":["T"],"origin":"model","edited":"2026-10-12","merged_into":null,"description":"…"}}
```

- `record` is the record as it is now (`removed` is present only on a removed record). `changed` is false, and nothing is written, when the file already is as asked (the same values on a record already `edited` today).
- Refused: no such record; no field given; an empty name or alias; a control character in a name or alias; a `--start` or `--end` that is not a date (`--start ""` included); `end` before `start`, or a `start`/`end` set while the other is not a date; an alias to remove that the record does not have; a record the reader would not read back (over 1 MiB).

`remove`, `restore` and `create` print the same object with their own `tool`.

### `scout events remove <id> --archive <path>` / `scout events restore <id> --archive <path>`

`remove` sets `removed: <today>` on the record; `restore` takes it off. Nothing else of the record changes (`edited` neither), so `restore` after `remove` leaves the file byte for byte as it was. What a removed event means for search is in "Lasting events" ("Removed events"). Removing a removed record, or restoring one that is not removed, writes nothing (`changed: false`). Refused: no such record; `remove` of a record merged into another (`<id>: merged into <b>; remove <b>, or undo the merge first`). Facade: `EventEditor::remove(id)`, `EventEditor::restore(id)`.

### `scout events create --archive <path> --name s [--description s] [--start d] [--end d]`

Creates a record with `origin: dominik`, `edited` today, no members, no aliases, no themes. Its id is `YYYY-MM-<slug>` by the seed's naming (the slug of the name, `event-x` when the name gives none, `-2`, `-3` … when the id is in use by a record or by any entry of the folder), the month taken from `--start`, else from today. The file is created as the seed creates one: the name is looked up in the folder before the write and the open is create-exclusive and no-follow, so an existing name is never written. The new file's mode is 0644. Refused: an empty name or one with a control character; a date that is not `YYYY-MM-DD`; `end` before `start`; a name that appeared in the folder after the look-up. Facade: `EventEditor::create(&NewEvent { name, description, start, end })`.

### `scout events merge <a> <b> --archive <path> [--by dominik|model] [--reason s]`

Merges `a` INTO `b`; `--by` defaults to `dominik`. Facade: `EventEditor::merge(&MergeRequest { a, b, by, reason })`.

**The rule: a merge changes one field of one record, `a.merged_into`, which becomes `b`.** Nothing is written to `b`, by a merge or by its undo.

- `a` gains `merged_into: <b>` and is otherwise unchanged (the command checks this of its own result and stops otherwise): it keeps its `members`, `aliases` and dates. Records already merged into `a` keep `merged_into: <a>`; the reader follows the chain.
- `b` is read: it must exist and not be merged away. Its file is not opened for writing, not replaced and not compared again before the write.
- What the combined event then shows (the ids it holds, its span, its aliases, the records merged into it) is derived by the reader: see "Merged events" and `scout events`.
- `edited` is not set on either record; the merge is in the log.
- **The rule for model merges**: `--by model` is refused when `a` has `edited` set (his record would be hidden behind another). `edited` on `b` refuses nothing, since `b` is not written.

```json
{"tool":"scout events merge","at":"2026-10-12T08:00:00.000Z","log_id":"m000001","a":"2026-10-beta","b":"2026-09-alpha","by":"dominik","reason":"one story","a_record":{"id":"2026-10-beta","…":"the record merged away as it is now","merged_into":"2026-09-alpha"}}
```

Refused, with nothing changed: `a` and `b` are one record; either is missing; either is already merged into another record (which also rules out a cycle: `b` merged into `a` is a `b` that is merged away); either is removed; either has a `start` or `end` that is not a date; `--by model` with `a` edited; a reason with a control character; `a`'s file not as the writer writes it ("What is kept"); a merge after which the reader would give a warning it does not give now (a monthly id that `a` and another event both list would then be reported against a different event); a merge of `a` that is in the log and was never applied (`unmerge` that line first; see "Order of writes"); a log that cannot take the line (see "Appending": a second hard link, a line over 4 MiB, a file that would pass 256 MiB, no id left).

### `scout events unmerge <log-id> --archive <path>`

Undoes the merge of one line of `merge-log.jsonl`. Facade: `EventEditor::unmerge(log_id)`.

**The rule: an undo clears `a.merged_into` and writes nothing else.** `b` is not written and none of its values is read. No other merge line is looked at.

- Any merge in effect can be undone at any time: whatever was merged into `b` or into `a` before or after it, whatever he changed on either record, whether `b` has itself since been merged away or removed, and in any order with other undos.
- A merge and its undo, with `a` not otherwise changed between, leave `a`'s file byte for byte as it was. `b`'s file is never written by either, so whatever he does to `b` in between is untouched.
- Records merged into `a` stay merged into `a` and leave `b`'s event with it.
- A line that was never applied (`a` is merged into nothing: a merge cut short after its log line, or an undo cut short before its own) is withdrawn: only the undo line is written, and the reply has `"withdrawn": true`.

```json
{"tool":"scout events unmerge","at":"2026-10-12T08:00:00.000Z","log_id":"m000002","undoes":"m000001","a":"2026-10-beta","b":"2026-09-alpha","withdrawn":false,"a_record":{"id":"2026-10-beta","…":"the record as it is now","merged_into":null}}
```

Refused, with nothing changed: no merge line of that id; already undone; record `a` is gone; `a` is merged into a record other than `b` (`merged_into` was changed by other means: `<id>: <a> is no longer merged into <b> (<a> is now merged into <c>); the merge cannot be undone`); `a`'s file not as the writer writes it; an undo after which the reader would give a warning it does not give now (a monthly id that `a` and another record of the event both list, which only a hand edit produces: apart, it could belong to one only; take it off one of them first); a log that cannot take the undo line.

### `merge-log.jsonl` and `merge-decisions.jsonl`

Both live in the records folder and are append-only. Only the edit commands read or write them (one parser, in `event_edit/log.rs`).

- **A merge line**: `{"id": "m000001", "at": "<UTC time>", "by": "dominik"|"model", "reason": "…"|null, "a": "<id>", "b": "<id>", "a_merged_into_before": null}`. `a` is the record merged away, `b` the surviving record. `a_merged_into_before` is `a`'s `merged_into` before the merge, always null (a record already merged into another is not merged); a line with any other value there is malformed. The line holds no copy of either record and no list of what moved: the merge wrote `merged_into` on `a` and nothing else, and `unmerge` and `check` need only `a` and `b` from it.
- **An undo line**: `{"id": "m000002", "at": "<UTC time>", "undoes": "m000001"}`.
- **Ids** are `m` and six to nine digits (`m000001` … `m999999999`; six digits are written up to 999999), one sequence for merge and undo lines, each greater than the line before. An id of any other form, `m1000000000` or longer included, is a malformed line. When the last line has `m999999999`, `merge` and `unmerge` refuse (`the last line has the largest id there is`); the file is his to move away.
- **A decision line** (`merge-decisions.jsonl`): `{"at": "<UTC time>", "file": "<proposals file name>", "a": "<id>", "b": "<id>", "decision": "approve"|"dismiss"}`, `a` and `b` as the proposal line has them.
- **Limits.** A log is at most 256 MiB (268,435,456 bytes) and a line, with its newline, at most 4 MiB (4,194,304 bytes), on reading and on writing. A line over the limit is refused before anything is written (`the line to append is … bytes, over the limit of 4194304 for one line`).
- **Reading.** A log is read whole before the command writes anything. It must be a plain file (a symlink, dangling or not, a folder or a special file is refused: `not a plain file`) within the limits (`larger than 268435456 bytes`, `line 3: longer than 4194304 bytes`), UTF-8, every line a JSON object of exactly these keys with nothing else, the last line ended by a newline. A log with a second hard link is read like any other (reading writes nowhere); a merge-log id must be well formed and greater than the one before, a merge line's `a_merged_into_before` must be null, and an undo line must name an earlier merge line not yet undone. Anything else stops the command before any write, naming the file and the line: `<file>: line 3: not JSON (…); fix or remove the line by hand; nothing was changed`. Every command that reads a log is stopped this way: `merge`, `unmerge`, `apply-proposals`, `check`, `proposals` and `decide` (these three read both logs).
- **Appending.** A log is written in place, so more is asked of it than of a record. Before the write, with nothing changed on a refusal:
  - the line with its newline must be within the line limit, and the file with the line within the file limit, so no append leaves a log that is then refused on reading (`<file>: is N bytes, and a line of M more would take it over the limit of 268435456 bytes …`);
  - the file is opened for append without following a link, and the file opened (the descriptor, not the path) must be a regular file with exactly one hard link (`<file>: has 2 hard links; an append would also change the file under its other name …`: the other name may be outside the records folder), the file that was read, at the length it was read at (else `changed since it was read`);
  - a log that was not there is created, create-new (so the name is no link of either kind), with mode 0644.

  Then the line and its newline go out in one `write` call (a short write is taken back by truncating to the old length, and is an error) and the file is synced. No line is ever rewritten or removed. `merge` appends its line as its first write. `unmerge` and `decide` append theirs last, so they make all of these checks (opening the file for reading) before their first write, and the append makes them again. Not defended: a link added to a log, or a write to it, in the instant between the check on the descriptor and the write.

### Order of writes: what a crash leaves

`merge` and `unmerge` each write one record and one log line.

- **`merge a b`**: (1) the log line; (2) record `a` (`merged_into: b`). Stopped after (1): no record has changed, and the line is in the log.
- **`unmerge`**: (1) record `a`, `merged_into` cleared; (2) the undo line. Before (1) the log is checked for whether it can take the line. Stopped after (1): the records are as before the merge, and the merge line has no undo line.
- **A merge line that is not undone is in one of two states**, told from record `a`: *in effect* (`a` is merged into `b`) or *not applied* (`a` is merged into nothing: either stop above). `scout events check` reports every line that is not applied, and `scout events unmerge <id>` withdraws it (it appends the undo line and writes no record), after which the merge can be made again. `merge` refuses an `a` that has a not-applied line and names the line. Both states are ones the reader reads without a warning.
- **A stop after the first write that is not a crash** (a write failed; record `a` was changed by hand or by another tool between the check before the log line and its own rename) leaves the same states. The error says so: `… the merge m000004 is in <log> and may not be applied; \`scout events check\` says whether it is, \`scout events unmerge m000004\` takes it back`, or for an undo `the undo of m000004 is not finished: <a> is unmerged and the undo line is not in the log; run \`scout events unmerge m000004\` again`. Record `a` is compared with the bytes read before the first write, so a change made before that point stops the command with nothing changed. A change to `b` at any point changes nothing for the merge: `b` is not written.
- `set`, `remove`, `restore` and `create` write one file each. `decide approve` checks that `merge-decisions.jsonl` can take its line, makes its merge (in the order above) and appends its decision line last; stopped between, the pair is merged and no longer waiting.
- A crash between creating a temporary file and renaming it leaves one `.scout-edit-*.tmp` file. The reader does not read it; `check` lists it; no command removes it.

### `scout events check --archive <path>`

Compares `merge-log.jsonl` with the records. Reads only. Exit 0 when nothing is wrong, 1 (with the same JSON line) when `problems` is not empty, 2 when the folder or a log cannot be read. Facade: `EventEditor::check()`.

```json
{"tool":"scout events check","at":"2026-10-12T08:00:00.000Z","dir":"/abs/archive/analysis/events","ok":false,"records":1712,"merges":3,"undone":1,"in_effect":1,"problems":[{"kind":"not-applied","log_id":"m000003","a":"2026-10-beta","b":"2026-09-alpha","detail":"the merge of 2026-10-beta into 2026-09-alpha is in the log and 2026-10-beta is merged into nothing (a merge cut short after its log line, or an undo cut short before its own)","fix":"`scout events unmerge m000003` withdraws it; then merge again if it is wanted"}],"information":[]}
```

- `merges` = merge lines in the log, `undone` = those an undo line undoes, `in_effect` = the rest that are in effect.
- `problems[].kind` (`ok` is false with any): `not-applied` (a merge line that is not undone whose `a` is merged into nothing; see above); `not-as-logged` (such a line whose `a` is gone or is merged into a record other than its `b`; put right by hand); `temporary-file` (a `.scout-edit-*.tmp` file left by an edit that was cut short).
- `information[]` (same shape; `ok` stays true and the exit code 0): `merged-without-log`, a record with `merged_into` and no merge line in effect for that pair. Setting `merged_into` by hand is allowed and is read like any merge; `unmerge` has no line to undo it by, so it is undone by clearing the field by hand.
- Only `merged_into` is compared with the log. Names, aliases, dates and members are his to change and no merge depends on them.

### `scout events apply-proposals <file.jsonl> --archive <path> [--apply]`

Makes the merges a proposals file of the merge pass judges `same` and `confident`. Without `--apply` nothing is written (stderr says `dry run: nothing written; N merges would be made (add --apply)`). Facade: `EventEditor::apply_proposals(file, apply)`.

- **The file** must be a plain file named `*.jsonl` directly inside `<archive>/analysis/events/merge-proposals/`, itself a real folder. The path given is used only to find that folder (its parent, with symlinks resolved, must be that folder) and the file's name; the file opened is the one of that name in that folder, without following a link. A path with `..` is refused. The file is never written.
- **Lines**: one JSON object each; `a`, `b` (record ids), `verdict`, `confidence`, `reason`, `a_items`, `b_items` are read, other keys ignored. A line with an `error` (not null), or with no `verdict`, is skipped and counted (`error_lines`, `no_verdict_lines`). `a_items` and `b_items` are taken only as whole numbers from 0 to 10,000,000 (missing or null is 0); a judged line where either is anything else (negative, a fraction, text, larger) is skipped and counted (`malformed_lines`), and its pair is neither merged nor listed as waiting. A line that is not a JSON object stops the command (the file may still be being written), as does a judged line whose `a`, `b` or `verdict` is not text. Blank lines are passed over.
- **Which merges.** Every pair with `verdict: "same"` and `confidence: "confident"`, in file order, as a `--by model` merge with the line's `reason`. A pair is taken as the two events its ids are in by then, so with A~B and B~C all three end in one event. Of the two events, the one that starts later is merged into the one that starts earlier, by the `start` of each event's own surviving record, not the span derived for it (a record without `start` counts as latest); on the same start the one with more items survives (the line's `a_items` / `b_items`, the largest a file gives for an id, a survivor's count growing by what is merged into it; the sum stops at the largest number it can hold rather than wrapping); then the id that sorts first. Since the survivor of every merge is the earlier one, the surviving record of an event is the one with the earliest own `start` of all its records, and a chain ends in its earliest event whatever the order of its lines.
- **Skipped**, each listed in `skipped` with `why` and counted in `skipped_counts`: `already-one-event` (the two are in one event already, which is what makes a second run a no-op), `no-record` (an id names no record), `model-rule` (the record that would be merged away has `edited` set; `edited` on the survivor skips nothing; the pair then waits for him, see `proposals`), `refused` (any other refusal of `merge`: a removed record, a date that is not a date, a record not as the writer writes it, a reason with a control character, a not-applied merge line for that record).
- **With `--apply`** each merge is made by `merge` itself, one at a time, each whole with its own log line and every check. The plan is worked out on a copy of the records in memory with the function `merge` uses, the reader's own resolution deciding after each pair which event an id is in. If a merge stops with an error, the run stops there: `… <file>: line 2: the merge of <a> into <b> stopped the run; 1 of 3 merges were made before it and stay (each is whole and in the log); run again for the rest`.

```json
{"tool":"scout events apply-proposals","at":"…","file":"/abs/archive/analysis/events/merge-proposals/2026-10-11-merge-1.jsonl","apply":true,"lines":659,"error_lines":0,"no_verdict_lines":0,"malformed_lines":0,"same_confident":57,"merges":[{"a":"2026-10-beta","b":"2026-09-alpha","reason":"…","line":12,"log_id":"m000001"}],"merged":1,"skipped":[{"a":"…","b":"…","line":40,"why":"model-rule","detail":"…"}],"skipped_counts":{"model-rule":1}}
```

`merges` are the merges in the order made (`a` merged into `b`; `log_id` null without `--apply`); `merged` = how many were made (0 without `--apply`).

### `scout events proposals <file.jsonl> --archive <path> [--json]`

Lists the pairs of a proposals file still waiting for him. Reads only; the output is the one JSON line with or without `--json`. Facade: `EventEditor::proposals(file)`.

- Waiting: a pair judged `same` + `doubtful`, or `same` + `confident` that the rule for model merges refuses now (`why`: `doubtful` or `model-rule`); each pair once (the first line that names it); without the pairs `merge-decisions.jsonl` holds a decision for under this file's name (`decided`), the pairs whose two records are already in one event (`already_one_event`), and the pairs naming a record that is not there (`no_record`).
- `from` and `into` say what approving does (the direction rule of `apply-proposals`, on the records as they are now).

```json
{"tool":"scout events proposals","at":"…","file":"/abs/…/merge-proposals/2026-10-11-merge-1.jsonl","lines":659,"malformed_lines":0,"waiting":[{"a":"2026-08-x","b":"2026-09-y","a_name":"X","b_name":"Y","a_items":8,"b_items":111,"confidence":"doubtful","reason":"…","why":"doubtful","detail":null,"line":601,"from":"2026-09-y","into":"2026-08-x"}],"decided":0,"already_one_event":0,"no_record":0}
```

### `scout events decide <file.jsonl> <a> <b> approve|dismiss --archive <path>`

Decides one waiting pair, given in either order. `approve` makes the merge as his (`--by dominik`) with the proposal's reason, in the direction `proposals` shows; `dismiss` merges nothing. Either way one line is appended to `merge-decisions.jsonl`, after the merge; that the file can take the line ("Appending") is checked before the merge, so a refused decisions log leaves the pair unmerged. Facade: `EventEditor::decide(file, a, b, Decision::Approve | Decision::Dismiss)`.

```json
{"tool":"scout events decide","at":"…","file":"/abs/…/merge-proposals/2026-10-11-merge-1.jsonl","a":"2026-08-x","b":"2026-09-y","decision":"approve","merge":{"tool":"scout events merge","log_id":"m000058","…":"…"}}
```

Refused, with nothing changed: the pair is not waiting (no such proposal, already decided, already in one event, a record missing); a `merge-decisions.jsonl` that cannot take the line (checked before the merge, so nothing is merged); an approval whose merge is refused (no decision is recorded then).

## `scout cite <passage-id> [--format markdown|plain] [--json]` — schema_version 1

Prints the citation of one passage (the `passage_id` of search, kwic or verify-quote), quoting the whole passage. Default `--format markdown`. Exit 1 when the index has no such passage; 2 for a malformed id.

```json
{
  "schema_version": 1,
  "passage_id": "tweets:stream/2025-07.md#1940162082952905136:78",
  "corpus": "tweets", "rel_path": "stream/2025-07.md#1940162082952905136", "path": "/abs/…/stream/2025-07.md",
  "line_start": 78, "line_end": 78,
  "quote": "original passage text",
  "title": "…", "author": "Dominik Lukeš", "date": "2025-07-01T21:34:21Z", "date_display": "1 July 2025",
  "date_source": "published",
  "link": "writeflex://open?path=…&line=78", "public_url": "https://x.com/techczech/status/1940162082952905136",
  "citation": {"markdown": "> …\n\n— Dominik Lukeš (@techczech), tweet, 1 July 2025 · [public](https://x.com/…)\n", "plain": "“…”\n— …\n"}
}
```

## `scout similar <passage-id>… [--in …] [--top 20] [--exclude-seeds] [--json]` — schema_version 1

"More like these": passages ranked by similarity to 1–10 seed passages (`passage_id`s). Facade: `Engine::similar(&SimilarQuery { seeds, in, top, exclude_seeds })`. Exit 1 when nothing shares two terms with the seeds or a seed id names no passage; 2 for no seed, more than 10, or a malformed id.

- **Terms**: the passage's index tokens minus stopwords and tokens with no letter. Weight = (1 + ln tf) × idf, idf = ln((1 + N) / (1 + df)) + 1, with N and df counted in passages over the searched corpora.
- **Score**: cosine of the passage vector with the centroid (mean) of the L2-normalised seed vectors, rounded to 6 decimals. A candidate must share at least 2 distinct terms with the seeds.
- **Scope**: `--in` (default every indexed corpus) chooses the corpora searched; a seed may lie outside them. Without `--exclude-seeds` each seed is itself a result (score 1 for a single seed).
- `shared_terms`: up to 3 shared terms by their contribution to the score (centroid weight × passage weight), then alphabetically. `closest_seed`: the seed with the highest cosine to the passage; the first given on a tie.
- `total_passages`: every candidate before `--top`.
- Order: `score` desc, then `passage_id`.

```json
{
  "schema_version": 1,
  "seeds": [{"passage_id": "writing:blogs/…/2016-06-23-repaved-paths-….md:24", "corpus": "writing", "rel_path": "…", "title": "Repaved paths and generative metaphors: …", "date": "2016-06-23"}],
  "corpora": ["writing", "tweets", "highlights"],
  "total_passages": 6983,
  "results": [{
    "passage_id": "highlights:readings/works/….md:199", "corpus": "highlights", "rel_path": "…", "path": "/abs/…",
    "line_start": 199, "line_end": 199,
    "quote": "original passage text",
    "score": 0.225284,
    "shared_terms": ["analogies", "metaphors", "new"],
    "closest_seed": "writing:blogs/…:24",
    "title": "The First 20 Hours", "author": "…", "date": "2019-05-14", "date_display": "14 May 2019",
    "date_source": "saved", "genre": null, "kind": "book", "source": "readwise",
    "link": "file:///…", "public_url": null,
    "citation": {"markdown": "…", "plain": "…"}
  }]
}
```

`date_source`, `kind` and `source` appear only when set.

## `scout kwic <term>` — schema_version 1

```json
{
  "schema_version": 1,
  "term": "scaffolding",
  "node_tokens": ["scaffolding"],
  "corpora": ["writing"],
  "filters": {},
  "near": {"word": "conceptual", "window": 5},
  "width": 8,
  "sort": "R1",
  "total": 43,
  "pieces": 21,
  "returned": 43,
  "lines": [{
    "corpus": "writing",
    "passage_id": "writing:blogs/x.md:25",
    "rel_path": "blogs/x.md",
    "path": "/abs/path/blogs/x.md",
    "title": "Pronominal paragraphs",
    "date": "2023-12-01",
    "date_display": "1 December 2023",
    "genre": "essay",
    "lang": "en",
    "line": 25,
    "left": "does not cave in on itself. Like a ",
    "node": "scaffolding",
    "right": " or a bulkhead. If language was just a",
    "node_offset": 412,
    "link": "writeflex://open?path=…&line=25"
  }]
}
```

- Matching: every position where the term's tokens occur consecutively in a passage (exact tokens after normalisation, no prefix; overlapping phrase matches each count).
- `total` counts all matching lines before `--limit`; `pieces` counts distinct documents among them; `returned` = `lines.length`.
- `near` (only with `--near <word> [--window 5]`): keep only lines with that single token within `window` tokens left or right of the node, inside the passage (the node's own tokens do not count). This is a collocate row's concordance.
- `left + node + right` is one contiguous substring of the original passage text. `left` starts at the first of up to `width` tokens before the node, `right` ends at the last of up to `width` tokens after it; either can include Markdown, entities or newlines as written. Punctuation glued to the edge token is included: `right` extends to the next whitespace (or the passage end) when everything up to it is non-alphanumeric, and `left` mirrors this, so "(building scaffolding)" gives `right` = `")"`. A run that reaches into letters (a stripped link target, say) is left out. `node_offset` is the node's byte offset inside the passage's original text.
- `sort`: `Lk` / `Rk` (k = 1–9) sort by the k-th token left / right of the node, then k+1 outward (a missing token sorts first); `date` sorts ascending with undated last; `source` = corpus, path, line. Every order ends with corpus, path, line, `node_offset`.

## `scout dist <term> --by year|corpus|genre|lang` — schema_version 1

```json
{
  "schema_version": 1,
  "term": "metaphor",
  "node_tokens": ["metaphor"],
  "corpora": ["writing"],
  "filters": {},
  "by": "year",
  "total": 2250,
  "pieces": 279,
  "buckets": [
    {"key": "2005", "hits": 732, "pieces": 31, "tokens": 190000, "per_million": 3852.632}
  ]
}
```

- `buckets` holds every bucket that has filtered documents, including zero-hit ones, so `tokens` is the full denominator; `per_million` = hits / tokens × 10⁶, three decimals.
- A document without the dimension falls in the bucket `"unknown"`.
- Order: `year` is chronological with `unknown` last; the others are hits desc, then key asc.
- `sum(buckets[].hits) == total`.

## `scout ngrams` — schema_version 1

```json
{
  "schema_version": 1,
  "corpora": ["writing"],
  "filters": {},
  "n_min": 3,
  "n_max": 5,
  "since": null,
  "until": null,
  "strict_stopwords": false,
  "total_tokens": 2429122,
  "documents": 1742,
  "grams": [
    {"gram": "a lot of", "n": 3, "count": 598, "pieces": 283, "per_million": 246.179}
  ]
}
```

- Grams are counted within passages over normalised tokens (lowercase; `don't` is one token).
- Default: only grams made entirely of stopwords are dropped. `--strict-stopwords`: grams with at least n−1 stopwords are dropped (for n=1, any stopword).
- `--since Y` / `--until Y` are inclusive years; undated documents are then left out, and each gram carries `"by_year": {"2016": 3, …}` (years with a count only).
- `--containing <word or phrase>` (facade: `NgramsQuery.containing`) keeps only grams that contain its tokens, normalised and tokenised as passages are; the top-level `"containing"` then lists those tokens (absent when unset). A word found nowhere gives no grams; a value with no word tokens is a usage error. The profile uses the same filter for its n-grams.
- `--dist year|doc|corpus` adds `"dist"` (top level: the chosen dimension) and a row distribution object on every gram (see above); its counts sum to the gram's `count`.
- Order: count desc, then gram asc; at most `--top`.

## `scout profile <word> [--min-hits 3]` — schema_version 2

```json
{
  "schema_version": 2,
  "word": "metaphor",
  "node_tokens": ["metaphor"],
  "corpora": ["writing"],
  "filters": {},
  "frequency": 2250,
  "total_tokens": 2429458,
  "per_million": 926.132,
  "pieces": 279,
  "first_used": {
    "corpus": "writing", "passage_id": "writing:…:32", "rel_path": "…", "title": "…",
    "date": "2000-01-04", "date_display": "4 January 2000", "line": 32
  },
  "by_year": [{"key": "2000", "hits": 2, "pieces": 1, "tokens": 30000, "per_million": 66.667}],
  "top_documents_min_hits": 3,
  "top_documents": [
    {"corpus": "writing", "rel_path": "…", "title": "…", "date": "2014-03-04", "hits": 44, "tokens": 1878, "per_million": 23429.2}
  ],
  "collocates": {
    "status": "ok", "window": 5, "score": "logdice", "min_freq": 5, "keep_stopwords": false,
    "items": [{"collocate": "conceptual", "count": 171, "collocate_freq": 977, "logdice": 10.762, "mi": 4.24, "kwic": {"near": {"word": "conceptual", "window": 5}, "command": "scout kwic metaphor --near conceptual --window 5 --in writing"}}]
  },
  "ngrams": {
    "status": "ok", "n_min": 3, "n_max": 5,
    "items": [{"gram": "of metaphor in", "n": 3, "count": 81, "pieces": 29, "per_million": 33.341}]
  }
}
```

- Version 2 (T4): `collocates` and `ngrams` are filled (`arrives_in` is gone), and `top_documents` needs `min_hits`.
- `frequency` equals `kwic.total` for the same word and filters; `by_year` is the same bucket list as `dist --by year`.
- `first_used`: the earliest dated occurrence (date, then corpus, path, line); `null` when no hit is dated.
- `top_documents`: up to 10 documents with at least `--min-hits` hits (default 3, so 1-hit notes no longer dominate), by per-million desc, then hits desc, then corpus, path.
- `collocates.items`: the top 15 rows of `scout collocates <word>` with the same filters (logDice, window 5, min 5, stopwords dropped); rows as in `scout collocates`.
- `ngrams.items`: the top 10 3–5-grams containing the word, rows as in `scout ngrams` (default stopword rule).

## `scout collocates <node>` — schema_version 1

`scout collocates <node> [--in …] [filters] [--window 5] [--score logdice|mi] [--min 5] [--top 30] [--keep-stopwords] [--dist year|doc|corpus] [--json]`

```json
{
  "schema_version": 1,
  "node": "metaphor",
  "node_tokens": ["metaphor"],
  "corpora": ["writing"],
  "filters": {},
  "window": 5,
  "window_span": 10,
  "score": "logdice",
  "min_freq": 5,
  "keep_stopwords": false,
  "node_freq": 2250,
  "total_tokens": 2429458,
  "candidates": 577,
  "rows": [{
    "collocate": "conceptual",
    "count": 171,
    "collocate_freq": 977,
    "logdice": 10.762,
    "mi": 4.24,
    "kwic": {
      "near": {"word": "conceptual", "window": 5},
      "command": "scout kwic metaphor --near conceptual --window 5 --in writing"
    }
  }]
}
```

- Node occurrences are the KWIC hits for the node (word or phrase). The window is `window` tokens left and right of the node inside one passage; the node's own tokens are not in it.
- `count` = f(n,c) = the node occurrences with the collocate at least once in the window. It equals `total` of `kwic.command` (invariant 3).
- `node_freq` = f(n); `collocate_freq` = f(c) over the filtered documents; `total_tokens` = N; `window_span` = W₂ = 2 · `window`.
- `logdice` = 14 + log2(2·f(n,c) / (f(n) + f(c))); `mi` = log2(f(n,c)·N / (f(n)·f(c)·W₂)). Both are always present, to three decimals; `score` names the one that orders the rows.
- `--min` filters on `count`. `candidates` = rows reaching it, before `--top`.
- Stopwords: by default a built-in English stopword is never a collocate (the node may be one). Function words fill every window and swamp the list; the phrases they form are what `ngrams` is for. `--keep-stopwords` lists them.
- `--dist` adds `"dist"` at the top level and a row distribution object per row (see above).
- Order: the chosen score desc, then `count` desc, then collocate asc.

## `scout collocates <node> --compare <slice>` — schema_version 1

The same node, window, score and `--min` over two slices side by side. A = the command's own corpora and filter flags; B = the slice (see "Slice expressions"). `--dist` does not combine with `--compare` (exit 2).

```json
{
  "schema_version": 1,
  "node": "metaphor",
  "node_tokens": ["metaphor"],
  "window": 5, "window_span": 10, "score": "logdice", "min_freq": 5, "keep_stopwords": false,
  "a": {"spec": "after:2020", "corpora": ["writing"], "filters": {"after": "2020"}, "node_freq": 278, "total_tokens": 466749},
  "b": {"spec": "before:2015", "corpora": ["writing"], "filters": {"before": "2015"}, "node_freq": 1735, "total_tokens": 1716466},
  "candidates": 490,
  "rows": [{
    "collocate": "conceptual",
    "a": {"count": 5, "collocate_freq": 91, "logdice": 8.794, "mi": 3.206, "kwic": {"near": {…}, "command": "scout kwic metaphor --near conceptual --window 5 --in writing --after 2020"}},
    "b": {"count": 158, "collocate_freq": 842, "logdice": 10.972, "mi": 4.214, "kwic": {"near": {…}, "command": "… --before 2015"}},
    "ratio": 0.198,
    "score_diff": -2.178
  }]
}
```

- A row appears when its `count` reaches `--min` on at least one side; the other side shows its actual count (possibly below `--min`, or 0 with `null` scores).
- `ratio` = (a.count / a.node_freq) / (b.count / b.node_freq): how much more often the node has this collocate in A than in B. `0` when a.count is 0; `null` when b.count is 0.
- `score_diff` = A's score − B's score in the chosen score; `null` when either side is 0.
- Order: the higher of the two scores desc, then a.count + b.count desc, then collocate asc. Swapping A and B gives the same rows in the same order with `a`/`b` swapped, `score_diff` negated and `ratio` inverted.

## `scout keyness --a <slice> --b <slice>` — schema_version 1

`scout keyness --a <slice> --b <slice> [--in …] [--top 40] [--min 5] [--json]`. Definition: spec, "Keyness (T4)".

```json
{
  "schema_version": 1,
  "a": {"spec": "before:2015", "corpora": ["writing"], "filters": {"before": "2015"}, "documents": 991, "tokens": 1716466},
  "b": {"spec": "after:2020", "corpora": ["writing"], "filters": {"after": "2020"}, "documents": 600, "tokens": 466749},
  "overlap_documents": 0,
  "min_freq": 5,
  "g2_critical": 3.84,
  "top": 40,
  "a_keys": [{"word": "se", "a": 15843, "b": 1031, "a_per_million": 9230.011, "b_per_million": 2208.896, "g2": 3041.165, "pct_diff": 317.856}],
  "b_keys": [{"word": "ai", "a": 29, "b": 2293, "a_per_million": 16.895, "b_per_million": 4912.705, "g2": 6777.214, "pct_diff": -99.656}]
}
```

- Single tokens over the stored token streams; `a`, `b` = counts; `tokens` = the slice totals c, d.
- `g2` = log-likelihood G² = 2·(a·ln(a/E₁) + b·ln(b/E₂)), E₁ = c·(a+b)/(c+d), E₂ = d·(a+b)/(c+d).
- `pct_diff` = %DIFF = ((a/c − b/d) / (b/d)) · 100: positive for A keys, negative for B keys, `null` when b = 0.
- `a_keys`: a/c > b/d, a ≥ `--min`, G² ≥ 3.84. `b_keys`: the mirror (b ≥ `--min`). Each ordered by G² desc, then word asc, at most `--top`.
- Stopwords are kept. `overlap_documents` counts documents in both slices; keyness assumes it is 0.
- Exit 1 when both lists are empty.

## `scout verify-quote <text>` — schema_version 1

```json
{
  "schema_version": 1,
  "quote": "the exact text as given",
  "corpora": ["writing"],
  "found": true,
  "matches": [{
    "corpus": "writing",
    "passage_id": "writing:…:204",
    "rel_path": "…",
    "path": "/abs/…",
    "title": "How Humans Hallucinate and How We Deal with It",
    "date": "2025-12-07",
    "line": 204,
    "original": "Vygotsky's zone of proximal development is often inflated",
    "link": "writeflex://open?path=…&line=204"
  }]
}
```

- The quote (trimmed of surrounding whitespace) must occur exactly in the original text of one passage. The only leniency is quote and apostrophe unification on both sides (‘ ’ ‛ ′ ʼ = `'`; “ ” „ ‟ ″ = `"`). No case folding, whitespace collapsing or entity decoding; a quote spanning two paragraphs is not found.
- `original` is the matched source text, which may differ from `quote` only in quote and apostrophe forms.
- Order: corpus, path, line. Exit 1 when `found` is false.


## `scout clean-report <corpus> [--samples 5] [--rule <regex>]… [--json]` — schema_version 1

What each normalisation stage changed, read from the source files (read-only; no index needed, so it also sees passages the index skips because boilerplate emptied them). `--rule` previews a candidate boilerplate regex as if it were in the registry; nothing is saved.

```json
{
  "schema_version": 1,
  "corpus": "writing",
  "documents": 1742, "passages": 60383, "lines": 73729,
  "samples_per_rule": 5,
  "rules": [{
    "rule": "entities", "label": "HTML entities",
    "passages": 57, "token_passages": 4, "token_lines": 4,
    "samples": [{
      "passage_id": "writing:blogs/…:117", "rel_path": "blogs/…", "path": "/abs/…", "line": 117,
      "original": "Rock &amp;amp;amp; roll",
      "before": ["rock", "amp", "amp", "amp", "roll"], "after": ["rock", "roll"],
      "removed": ["amp", "amp", "amp"], "added": [],
      "occurrences": 2
    }]
  }, {
    "rule": "boilerplate", "label": "Boilerplate lines", "passages": 80, "token_passages": 80, "token_lines": 80,
    "patterns": [{"n": 1, "regex": "^\\*Dominik Lukeš\\*$", "origin": "registry", "lines": 80, "passages": 80}],
    "samples": [{"…": "…", "pattern": 1}]
  }]
}
```

- `rules` always lists the six stages in pipeline order: `boilerplate`, `entities`, `tags` (HTML tags and comments), `links_urls` (Markdown link targets and bare URLs), `apostrophes` (curly/straight quote unification), `nfc`.
- `passages`: passages whose text the stage changed. `token_passages`: of those, passages whose counted tokens differ when that one stage is left out. `token_lines`: source lines whose tokens differ.
- A sample is one source line: `original` as written, `after` = the tokens the index counts, `before` = the tokens with that one stage left out (every other stage still applied). So an apostrophe sample reads `before ["don’t"]`, `after ["don't"]`. `removed` / `added` are the multiset differences.
- Samples are grouped by their change (`removed` + `added`); `occurrences` = lines with that change. Groups are ordered by occurrences desc, then change; each group's sample is its first line in corpus order. At most `--samples` per stage.
- A stage sees only what earlier stages left: a line dropped as boilerplate is not counted again for links.
- `patterns` (boilerplate only): registry rules then `--rule` previews, `n` 1-based; a dropped line is credited to the first rule that matches it. `pattern` on a boilerplate sample names that rule.

## `scout index build [ids…] [--force] --json` — schema_version 1

```json
{"schema_version": 1, "reports": [{"corpus": "writing", "index_path": "/abs/…/writing.sqlite", "full": false, "docs": 1742, "passages": 60383, "tokens": 2429458, "files_seen": 1800, "indexed": 3, "unchanged": 1797, "removed": 0, "not_documents": 58, "elapsed_ms": 812}]}
```

`full` is true on a first build, `--force`, or an engine or registry-entry change. `elapsed_ms` is the one non-deterministic field.

A build refuses (exit 2, nothing opened or deleted) when the index file or one of its `-wal`/`-shm`/`-journal` side files is a symlink or has more than one hard link, or when the index's real location is inside any registered corpus folder; the vector store is checked the same way. The side-file paths are built byte for byte from the index path (never through a text conversion), so a folder name that is not valid UTF-8 is checked as the same files SQLite will open.

A corpus whose folder has a topic layer adds a `topics` object (see "Topic layer"):

```json
"topics": {"layer_path": "/abs/…/analysis/topics/current.jsonl", "items": 10118, "matched": 10102, "matched_by_key": 10102, "matched_by_path": 0, "unmatched": 16, "unmatched_sample": ["x:1874…"], "tagged_documents": 10102, "bad_lines": 0, "corrections": 0, "corrections_unmatched": 0, "corrections_bad_lines": 0, "reloaded": true}
```

`items` = distinct keys in the layer; `matched` = items joined to a document (`matched_by_key` + `matched_by_path`); `unmatched_sample` = up to 5 unmatched keys, sorted; `reloaded` is false when neither the layer files nor the documents changed and the stored copy was kept. The human output adds a line `topics: N items, M matched (K by key, P by path), U unmatched → T tagged documents`.

A corpus with a records folder (`analysis/events/`, see "Lasting events") adds `events` inside `topics`; when the corpus has records and no topic layer there is no `topics` object and the same `events` object sits beside it, directly in the corpus's report (`reports[i].events`):

```json
"events": {"dir": "/abs/…/analysis/events", "records": 1694, "merged": 0, "members": 1694, "skipped": 1, "warnings_total": 1, "warnings": ["analysis/events/draft.md: no id; skipped"]}
```

`records` = records read; `merged` = those with a `merged_into` in effect; `members` = monthly ids that resolve to a record; `skipped` = files left out as malformed; `warnings` = the first 50 (`events::MAX_WARNINGS`) of `warnings_total` lines, one per skipped file, unusable `merged_into` or contested member, skipped files first in file-name order. The human output adds `events: R records (M merged), N monthly events held, S files skipped` and one `warning:` line each.

## `scout index status --json` — schema_version 1

```json
{"schema_version": 1, "corpora": [{"corpus": "writing", "kind": "markdown-folder", "index_path": "/abs/…", "exists": true, "docs": 1742, "passages": 60383, "tokens": 2429458, "built_at": "2026-09-27T14:52:00Z", "config_current": true, "stale": {"added": 0, "changed": 1, "removed": 0}}]}
```

Every registered corpus, in registry order. Without an index: `exists` false, zero counts, `built_at` and `stale` null.

## `scout corpora list --json` — schema_version 1

```json
{"schema_version": 1, "corpora": [{"id": "notes", "name": "Notes", "kind": "markdown-folder", "path": "~/Notes", "root": "/Users/…/Notes", "index_path": "/abs/…/notes.sqlite", "index_built": true, "folder_exists": true}]}
```

`path` is as written in the registry; `root` is it expanded. `folder_exists` is false when the source folder is gone: the entry and its index are kept and stay searchable (the human list appends `(folder missing)`); nothing is removed automatically.

## `scout corpora add <folder> [--id] [--name] [--kind] [--author] [--include <glob>]… [--exclude <glob>]… --json`

The added registry entry, exactly as written to the registry (the `[[corpus]]` table as JSON; no `schema_version`, because it is the registry's own shape):

```json
{"id": "notes", "name": "Notes", "kind": "markdown-folder", "path": "~/Notes", "include": ["**/*.md", "**/*.txt"], "exclude": [], "require_frontmatter": [], "field_map": {"title": "title", "date": "date", "…": "…"}, "boilerplate": []}
```

- `path`: absolute, with `~` when under the home folder; `~` in the argument is expanded. The folder must exist.
- `id`: default a slug of the folder name (ASCII, lower case, accents dropped), de-duplicated as `notes`, `notes-2`. An explicit `--id` already in use is refused, naming the clash.
- `kind`: default `markdown-folder`; never guessed from the folder's contents. `highlight-scout-archive` only when asked for (`--kind highlight-scout-archive`), and the folder must have `readings/works/`.
- markdown-folder defaults: include `**/*.md` and `**/*.txt`, no required frontmatter, no link template, no default author, one document per file. A `.txt` file with no frontmatter title is titled by its file stem and stays undated.
- Refused (exit 2, registry unchanged): a path that is not valid UTF-8 (the registry is text and would record a different folder), a missing path or a file, a path already registered (`path … already registered as \`<id>\``), a clashing id, a bad glob, a registry that does not parse.
- Creates the registry when missing. Does not build the index; the human output ends `next: scout index build <id>`.

Library: `scout_corpus::api::add_corpus(AddCorpus) -> Result<CorpusConfig>` (the user registry); `scout_corpus::register::RegistryStore::new(registry_path, index_dir).add(&AddCorpus)` for an explicit registry. `AddCorpus` is a serde struct with every field but `path` optional: `{"path": "~/Notes", "id": null, "name": null, "kind": null, "author": null, "include": [], "exclude": []}`.

## `scout corpora remove <id> [--keep-index] --json` — schema_version 1

```json
{"schema_version": 1, "id": "notes", "path": "~/Notes", "index_kept": false, "deleted": ["/abs/…/notes.sqlite", "/abs/…/notes.vectors.sqlite"]}
```

Removes the registry entry and, unless `--keep-index`, the corpus's word index and meaning vectors (with SQLite side files); `deleted` lists the files removed. The source folder is never touched. An unknown id is the usual `unknown corpus` error (exit 2); no registry is `RegistryMissing` (exit 3).

Library: `scout_corpus::api::remove_corpus(id, keep_index) -> Result<Removed>`, or `RegistryStore::remove`.

## Registry writes

`add`, `remove` and `init-defaults` write the registry atomically (a temp file in the same directory, then a rename); a failed write leaves the old file as it was. `add` appends the new `[[corpus]]` table and `remove` cuts the entry's lines, so the rest of the file, comments included, stays byte-for-byte; when a hand-edited layout cannot be edited that way, the file is regenerated with its leading comment block kept. `init-defaults` writes an empty registry whose comment explains `scout corpora add`. With no registry, every other command fails with `no corpus registry at <path>; add a folder with \`scout corpora add <folder>\`` (exit 3).

---
title: "`scout --json` output schemas"
date: 2026-09-27
register: FOR AGENTS
spec: docs/specs/2026-09-27-corpus-engine-and-cli.md
---

# `scout --json` output schemas

Values in the examples are illustrative. Every `--json` document is one pretty-printed JSON object with a top-level `schema_version` (integer). A field is added without a bump; a rename, removal or meaning change bumps that command's `schema_version`.

## Library facade (apps)

Every command is one call on `scout_corpus::api::Engine` (`Engine::from_env()` = the CLI's registry and index dir; `Engine::new(registry, index_dir)` for apps and tests). Each call takes a request struct (`SearchQuery`, `KwicQuery`, `CollocatesQuery`, `KeynessQuery`, `DistQuery`, `NgramsQuery`, `ProfileQuery`, `VerifyQuoteQuery`, `CiteQuery`, `SimilarQuery`, `IndexBuildQuery`, `CleanReportQuery`; `index_status()` and `corpora_list()` take none) and returns `Reply { body, notes }`.

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
| `lang` | string[] | any-of, case-insensitive (`--lang en,cs`) |
| `genre` | string[] | any-of, case-insensitive (`--genre essay,note`) |
| `after` | string | inclusive lower bound; `YYYY`/`YYYY-MM` mean the first day of that period |
| `before` | string | exclusive upper bound; `--before 2016` = up to 31 Dec 2015 |
| `year` | int | the document's year (`--y 2016`) |
| `source` | string[] | any-of the source system: `readwise`, `x`, `zotero` (`--source x`); Dominik's own tweets are `x` |
| `kind` | string[] | any-of the document type (`tweet`, `article`, `book` …); a plural `s` is ignored (`ty:articles` matches `article`) |
| `author` | string | case-insensitive substring of the author |
| `title` | string | case-insensitive substring of the title |

A document without a parseable date fails every date filter. `source`, `kind`, `author` and `title` come from query fields (`source: ty: au: ti:`) and slices; the analytics commands take `--source`.

### Slice expressions (`collocates --compare`, `keyness --a/--b`)

One quoted string, parsed by the `@scout/query` grammar (`crates/scout-query`); bare words are corpus ids:

| term | meaning |
|---|---|
| `writing`, `in:writing,tweets` | the corpora; a slice without one uses the command's `--in` (default: every indexed corpus) |
| `after:2020`, `before:2015` (`since:`/`from:`, `until:`/`to:`) | as `--after` / `--before` |
| `y:2016` | one year |
| `y:2010-2015` | inclusive range = `after:2010 before:2016`; cannot combine with `after:`/`before:` |
| `y:2016-05`, `y:2016-`, `y:-2016` | the grammar's other date forms, as a range |
| `lang:en,cs`, `genre:essay,note` | as `--lang` / `--genre` |
| `source:zotero`, `zo:`, `ty:books`, `au:lakoff`, `ti:"metaphors we"` | the document fields above |

A slice refuses search terms (`-x`, `"phrase"`, `x*`, `/re/`) and the highlight fields `tag:`, `co:`, `i:`.

A slice carries only its own terms; it does not inherit the command's filter flags. So `scout collocates metaphor --after 2020 --compare "before:2015"` compares after-2020 writing (A) with before-2015 writing (B), and `--compare highlights` compares the same filters' writing with the highlights corpus. The echoed `spec` is the slice as given; for the main side of `--compare` it is the filter flags written as a slice (`after:2020`, or `all`).

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

## `scout search <query>` — schema_version 1

The query is the `@scout/query` grammar (TS `packages/scout-query`, Rust `crates/scout-query`, one shared fixture file `packages/scout-query/fixtures/grammar-cases.json`):

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

`plan` keys other than `any_of`, `not` and `filters` appear only when set; so do `date_source`, `kind`, `source` and the hit's `tags`, `color`, `saved_at` (highlights).

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

## `scout index status --json` — schema_version 1

```json
{"schema_version": 1, "corpora": [{"corpus": "writing", "kind": "markdown-folder", "index_path": "/abs/…", "exists": true, "docs": 1742, "passages": 60383, "tokens": 2429458, "built_at": "2026-09-27T14:52:00Z", "config_current": true, "stale": {"added": 0, "changed": 1, "removed": 0}}]}
```

Every registered corpus, in registry order. Without an index: `exists` false, zero counts, `built_at` and `stale` null.

## `scout corpora list --json` — schema_version 1

```json
{"schema_version": 1, "corpora": [{"id": "writing", "name": "Writing", "kind": "markdown-folder", "path": "~/gitrepos/…/writing", "root": "/Users/…/writing", "index_path": "/abs/…/writing.sqlite", "index_built": true}]}
```

`path` is as written in the registry; `root` is it expanded.

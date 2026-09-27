---
title: "`scout --json` output schemas"
date: 2026-09-27
register: FOR AGENTS
spec: docs/specs/2026-09-27-corpus-engine-and-cli.md
---

# `scout --json` output schemas

Values in the examples are illustrative. Every `--json` document is one pretty-printed JSON object with a top-level `schema_version` (integer). A field is added without a bump; a rename, removal or meaning change bumps that command's `schema_version`.

## Conventions

- **Original text.** Every text field that quotes a source (`quote`, `left`, `node`, `right`, `original`) is ORIGINAL source text: Markdown, entities and curly quotes as written. Normalised text never appears.
- **Lines.** `line` is 1-based in the source file.
- **Passage id.** `passage_id` = `<corpus>:<rel_path>:<passage line_start>`. It is stable across rebuilds and is what `scout cite` takes.
- **Dates.** `date` is the raw frontmatter value (`2016`, `2016-06` or `2016-06-23`). `date_display` is British style ("23 June 2016"), or `null` when unparseable. An unknown date is `null`, never guessed.
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

A document without a parseable date fails every date filter.

### Slice expressions (`collocates --compare`, `keyness --a/--b`)

One quoted string of whitespace-separated terms, in the `@scout/query` field syntax:

| term | meaning |
|---|---|
| `writing`, `in:writing,tweets` | the corpora; a slice without one uses the command's `--in` (default: every indexed corpus) |
| `after:2020`, `before:2015` | as `--after` / `--before` |
| `y:2016` | one year |
| `y:2010-2015` | inclusive range = `after:2010 before:2016`; cannot combine with `after:`/`before:` |
| `lang:en,cs`, `genre:essay,note` | as `--lang` / `--genre` |

A slice carries only its own terms; it does not inherit the command's filter flags. So `scout collocates metaphor --after 2020 --compare "before:2015"` compares after-2020 writing (A) with before-2015 writing (B), and `--compare highlights` will compare the same filters' writing with the highlights corpus once T2 indexes it. The echoed `spec` is the slice as given; for the main side of `--compare` it is the filter flags written as a slice (`after:2020`, or `all`).

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

## `scout search` — schema_version 1

```json
{
  "schema_version": 1,
  "query": "generative metaphor",
  "terms": [{"kind": "prefix", "tokens": "generative"}, {"kind": "phrase", "tokens": ["a", "b"]}],
  "corpora": ["writing"],
  "total_documents": 12,
  "total_passages": 30,
  "results": [{
    "corpus": "writing", "rel_path": "…", "path": "/abs/…", "title": "…",
    "author": "Dominik Lukeš", "date": "2016-06-23", "date_display": "23 June 2016",
    "genre": "essay", "lang": "en",
    "public_url": "https://medium.com/…",
    "score": 12.3, "title_match": true, "passage_count": 3,
    "hits": [{
      "passage_id": "writing:…:12", "line_start": 12, "line_end": 13, "line": 12,
      "quote": "original sentence or passage", "score": 11.1, "link": "writeflex://open?path=…&line=12"
    }],
    "citation": {"markdown": "> …\n\n— Author, *Full Title*, 23 June 2016 · [archive](writeflex://…) · [public](https://…)\n", "plain": "“…”\n— Author, Full Title, 23 June 2016 · archive: writeflex://… · public: https://…\n"}
  }]
}
```

Order: `results` by score desc, then corpus, then path; `hits` by score desc, then line.

- `public_url` is the first `http(s)://` value among the frontmatter keys in the corpus's `field_map.public_url` (writing: `published_url`, then `source_url`, then `canonical_url`); `null` when none.
- Citation line: author, the full frontmatter title, the British date, then `· [archive](<link>)` and `· [public](<url>)` when each exists. `plain` writes `· archive: <link> · public: <url>`. An unknown date is left out.

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
- `containing` (library only, used by the profile) appears when set.
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

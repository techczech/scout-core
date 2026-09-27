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
- **Counts agree** (invariant 3). For the same term and filters: `kwic.total` = the n=1 (or n=phrase length) `ngrams` count = `profile.frequency` = the sum of `dist.buckets[].hits`.
- **Exit codes.** 0 = results; 1 = none (kwic/dist/profile with zero hits, empty ngrams, verify-quote not found); 2 = usage; 3 = index or registry missing.

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
    "score": 12.3, "title_match": true, "passage_count": 3,
    "hits": [{
      "passage_id": "writing:…:12", "line_start": 12, "line_end": 13, "line": 12,
      "quote": "original sentence or passage", "score": 11.1, "link": "writeflex://open?path=…&line=12"
    }],
    "citation": {"markdown": "> …\n— Author, *Title*, 23 June 2016\n…", "plain": "…"}
  }]
}
```

Order: `results` by score desc, then corpus, then path; `hits` by score desc, then line.

## `scout kwic <term>` — schema_version 1

```json
{
  "schema_version": 1,
  "term": "scaffolding",
  "node_tokens": ["scaffolding"],
  "corpora": ["writing"],
  "filters": {},
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
- `left + node + right` is one contiguous substring of the original passage text. `left` starts at the first of up to `width` tokens before the node, `right` ends at the last of up to `width` tokens after it; either can include Markdown, entities or newlines as written. `node_offset` is the node's byte offset inside the passage's original text.
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
- `containing` (library only, used by the profile in T4) appears when set.
- Order: count desc, then gram asc; at most `--top`.

## `scout profile <word>` — schema_version 1

```json
{
  "schema_version": 1,
  "word": "metaphor",
  "node_tokens": ["metaphor"],
  "corpora": ["writing"],
  "filters": {},
  "frequency": 2250,
  "total_tokens": 2429122,
  "per_million": 926.257,
  "pieces": 279,
  "first_used": {
    "corpus": "writing", "passage_id": "writing:…:32", "rel_path": "…", "title": "…",
    "date": "2000-01-04", "date_display": "4 January 2000", "line": 32
  },
  "by_year": [{"key": "2000", "hits": 2, "pieces": 1, "tokens": 30000, "per_million": 66.667}],
  "top_documents": [
    {"corpus": "writing", "rel_path": "…", "title": "…", "date": "2014-03-04", "hits": 44, "tokens": 1878, "per_million": 23429.2}
  ],
  "collocates": {"status": "pending", "arrives_in": "T4", "items": []},
  "ngrams": {"status": "pending", "arrives_in": "T4", "items": []}
}
```

- `frequency` equals `kwic.total` for the same word and filters; `by_year` is the same bucket list as `dist --by year`.
- `first_used`: the earliest dated occurrence (date, then corpus, path, line); `null` when no hit is dated.
- `top_documents`: up to 10, by per-million desc, then hits desc, then corpus, path.
- `collocates` and `ngrams` are placeholders until T4: `status` becomes `"ok"` and `items` fills (collocates: a plain logDice list; ngrams: the top n-grams containing the word).

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

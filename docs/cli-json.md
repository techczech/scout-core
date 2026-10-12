---
title: "`scout --json` output schemas"
date: 2026-09-27
register: FOR AGENTS
spec: docs/specs/2026-09-27-corpus-engine-and-cli.md
---

# `scout --json` output schemas

Values in the examples are illustrative. Every `--json` document is one pretty-printed JSON object with a top-level `schema_version` (integer). A field is added without a bump; a rename, removal or meaning change bumps that command's `schema_version`.

## Changed in this version (0.4.1)

Engine `scout-corpus/14`. Every change is a read: no command writes anything it did not write before, no key changes type, and each new key is absent where it has nothing to say, so a corpus without merges, without `mentioned_works`, without days on its tag rows and without event items that lack a month (those now show as `undated_items`) gets the replies it got from 0.4.0, byte for byte (tested: `crates/scout-corpus/tests/event_page_reads.rs`).

1. **`scout events merges --archive <path> [--all] [--json]`**, facade `EventEditor::merges()` / `merges_all()` / `merge_list(all)`: the merges in effect, newest first, each with the log id `unmerge` takes, and with `undoable: false` and `why_not` on one `unmerge` would refuse now ("`scout events merges`").
2. **`merge_id`** on each entry of a listed event's `merged_from`: the merge in effect for that record in the events folder as it is now (the answer of `scout events merges`), absent for a record merged by hand; and a top-level **`stale: true`** on the listing when the events folder changed since the index was built, in which case the entries the folder no longer agrees with have no `merge_id`. An entry with a `merge_id` that `unmerge` would refuse now also has `undoable: false` and `why_not` ("`scout events`").
3. **`own_aliases`** on a listed event, only where `aliases` (own and merged-in) differs from the record's own ("`scout events`").
4. **`month_spans`** on a listed event: per month, the first and the last day of its items. A new key beside `months`, which is unchanged ("`scout events`").
5. **`undated_items`** on a listed event: how many of its items have no layer month ("`scout events`", which also says how they are reached).
6. **`topics.mentioned_works`** on a search result and a **`works`** facet in `scout facets`: the works a document's tag row says it mentions ("Topic layer", "Mentioned works"; "`scout facets`"). No query field selects by a work.
7. Documented, not changed: `--min-items 0` lists records without items, each row has `origin` and `edited` (they always did), and `min_items` counts an event's items in the whole corpus ("`scout events`"); the event facet of `event:<id>` lists the event itself and each event that shares items with it, in the facet's usual order, so the event is told by its id and not by its place ("`scout facets`", "Related events").
8. An index built by an earlier engine is rebuilt in full at the next `index build`. Until then it is read as it is: no `mentioned_works`, no `works` facet and no `month_spans`; `merge_id`, `own_aliases` and `undated_items` need no rebuild.

Rust callers that build these structs by hand: `events::MergedFrom` has the new fields `merge_id`, `undoable` and `why_not`; `event_edit::MergeInfo` has `undoable` and `why_not`; `EventListing` has `month_spans`, `undated_items`, `own_aliases`; `EventsResults` has `stale`; `topics::TopicInfo` has `mentioned_works` and `day`; `topics::Facets` has `works`; `FacetItem` has `key`, `title`, `rel_path`, `path`.

## Library facade (apps)

Every command is one call on `scout_corpus::api::Engine` (`Engine::from_env()` = the CLI's registry and index dir; `Engine::new(registry, index_dir)` for apps and tests). Each call takes a request struct (`SearchQuery`, `KwicQuery`, `CollocatesQuery`, `KeynessQuery`, `DistQuery`, `NgramsQuery`, `ProfileQuery`, `VerifyQuoteQuery`, `CiteQuery`, `SimilarQuery`, `IndexBuildQuery`, `CleanReportQuery`, `FacetsQuery`, `TopicsQuery`, `ThemeMonthsQuery`, `EventsQuery`; `index_status()` and `corpora_list()` take none) and returns `Reply { body, notes }`.

- `api::to_json(&reply.body)` is byte-for-byte the command's `--json` stdout (minus the final newline); `notes` are the stderr lines; `Outcome::has_results(&body)` is exit 0 vs 1. Errors are the same typed errors the CLI maps to exit 2/3 (`IndexMissing`, `RegistryMissing`, `NoIndexedCorpus`) or 1 (`PassageNotFound`). `crates/scout-cli/tests/facade_parity.rs` holds this for every command.
- Requests are serde structs with every field defaulted to the CLI default, so partial JSON works: `{"term": "metaphor", "scope": {"in": ["writing"], "after": "2020"}}`. Rust `in_` fields serialise as `"in"`. `Scope` = `in`, `lang`, `genre`, `after`, `before`, `year`, `source`. Enumerated options stay strings as on the command line (`sort: "R1"`, `score: "mi"`, `by: "year"`, `n: "3-5"`, `dist: "doc"`; `SearchQuery.sort` is `"relevance"`, `"newest"` or `"oldest"`).
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
- **Human output and the terminal.** Without `--json`, every string a command prints that comes from a file or an argument (a theme, entity or event name, a title, a quote, a path, a key, a corpus id, a registry entry, the text of an error or a note on stderr, the citation `scout cite` prints) goes through one function of the CLI (`term::safe`) before it is written. It replaces each of these by U+FFFD, one character for one, and keeps everything else: the C0 control characters but the tab (U+0000 to U+001F: ESC, CR, LF, BEL … ), DEL (U+007F), the C1 controls (U+0080 to U+009F), the line and paragraph separators (U+2028, U+2029); the bidirectional controls, which reorder what is shown (U+061C, U+200E, U+200F, U+202A to U+202E, the isolates U+2066 to U+2069); and the invisible format characters (the zero width space U+200B, U+2060 to U+2069, the byte order mark U+FEFF, the tag block U+E0000 to U+E007F). The zero width joiner and non-joiner (U+200D, U+200C) are kept on purpose: emoji sequences (a family, a profession) are built with the joiner and Persian and Indic text is spelt with both; the cost of replacing the tag block is that the three flags written with tag characters (England, Scotland, Wales) show as a black flag and replacement characters. So a name in an archive file cannot colour, move, reorder or rewrite the terminal, or start a line of its own. A string a view shows in double quotes (the query of `search` and `facets`, the term of `kwic`, `dist` and `profile`, the `--near` word, the node of `collocates`, a rule's pattern in `clean-report`) goes through the same function first and is then quoted, a `"` or a `\` of its own escaped: no view leaves such a string to an escape of the language. **A string from a file or an argument is one line of output at most**: a line break in the output is always one the program wrote between such strings. An error on stderr is `scout: ` and its causes joined by `: `, each cause one line (`--archive $'missing\nFAKE'` prints one line, the line feed as U+FFFD); the refusals of `scout events seed` and of the event edits that list several faults print one line per fault. Two texts are documents of several lines and keep their own line feeds, each line treated so: a citation and the registry entry of `corpora show`. Not covered: a usage error of the argument parser (an unknown option, a missing value), written before any command runs, repeats the argument as it was typed, line feeds included. **JSON output is not changed**: every string is the file's own, with the control characters below U+0020 written as JSON escapes (ESC is `\u001b`); characters from U+0080 up, the C1 controls and U+2028 among them, are written as themselves, as JSON allows. The one-line JSON of `scout events …` and `scout correct …` is JSON too.
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
| `theme` | string | topic layer: the document carries this theme (exact name); with several `theme:` terms, the last one |
| `themes` | string[] | only with two or more `theme:` terms: all of them, in query order; the document carries EVERY one |
| `entity` | string | topic layer: the document carries this entity (exact normalised name); with several terms, the last one |
| `entities` | string[] | only with two or more `entity:` terms: all of them; the document carries EVERY one |
| `event` | string | topic layer: an event the document is in, by lasting id, merged-away id or monthly id (`2026-09/E21`); kept as typed (see "Lasting events"); with several terms, the last one |
| `events` | string[] | only with two or more `event:` terms: all of them; the document is in EVERY one |
| `ai` | bool | topic layer: the document's `ai_related` |
| `month` | string | topic layer month: `YYYY-MM`, or `YYYY` for any month of that year; with several terms, the last one |
| `months` | string[] | only with two or more `month:` terms: all of them; the document is in ANY one |

Every text comparison (equality or substring) is made under the fold of "Query grammar" below, on both the stored value and the filter value. A document without a parseable date fails every date filter. A document without topic tags fails every topic filter, `ai:false` included. `source`, `kind`, `author`, `title` and the topic fields come from query fields (`source: ty: au: ti: theme: entity: event: ai: month:`) and slices; the analytics commands take `--source`. A query or slice with ONE term of a topic field gives the string key alone, exactly as before engine `scout-corpus/13`; the list key is added only when a field has two or more terms, and the string key then still holds the last term, so a reader of the string keys parses every reply ("Repeated fields" in "Query grammar").

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
| `theme:"AI and mathematics"`, `entity:`, `event:`, `ai:true`, `month:2026-09` | the topic-layer fields above; `theme:`, `entity:`, `event:` and `month:` may be repeated, as in a query ("Repeated fields") |

A slice refuses search terms (`-x`, `"phrase"`, `x*`, `/re/`) and the highlight fields `tag:`, `co:`, `i:`.

A slice carries only its own terms; it does not inherit the command's filter flags. So `scout collocates metaphor --after 2020 --compare "before:2015"` compares after-2020 writing (A) with before-2015 writing (B), and `--compare highlights` compares the same filters' writing with the highlights corpus. The echoed `spec` is the slice as given; for the main side of `--compare` it is the filter flags written as a slice (`after:2020`, or `all`), every value written by the grammar's value-writing rule (`ti:"GPT \\"Astra\\""`), one term per value of a repeated field, so it parses back to the same filter. `lang: genre: source: ty:` are comma lists, so an item cannot itself hold a comma.

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

**Changed in this version** (engine `scout-corpus/13`, `@scout/query` parser of the same date):

1. **Repeated `theme:`, `entity:`, `event:` terms are ANDed and repeated `month:` terms ORed** ("Repeated fields" below). BEHAVIOUR CHANGE: before, a repeated field kept the last value, so `theme:A theme:B` meant `theme:B`; it now means "carries A and B". Every other field keeps the last value, as before.
2. The parser's result gains four lists, `themes`, `entities`, `events`, `months` (every value of the field, in query order). The scalar keys `theme`, `entity`, `event`, `month` are unchanged (the last value).
3. Each term of a repeated field needs a value: `theme: theme:A` is now a usage error (it was `theme:A`).
4. `scout facets` takes an empty query ("`scout facets`"); its month facet is counted without the query's `month:` terms, and its `ai` facet without the query's `ai:` term (before, `facets "ai:true"` gave `"false": 0`).
5. `scout search --sort newest|oldest`, with which a query may be empty or filters only ("Order").
6. Entity types: `topics.entity_types` on a result, `type` on an entity facet value ("Topic layer", "Entity types").
7. `scout topics theme-months`: `--from` / `--to` are exactly `YYYY-MM` (a padded bare year, `"2026   "`, was taken as a year); the matrix is made in one pass.
8. **Human-readable views only (no `--json` reply changes): a query, term or pattern shown in quotes keeps its tab, no-break space and emoji joiners as themselves.** BEHAVIOUR CHANGE: before, the quoted string was written with the programming language's escapes, so `scout search 'a<TAB>b'` printed `… for "a\tb" in …` and a no-break space printed `\u{a0}`; the header now holds the tab and the no-break space themselves, as "Human output and the terminal" says those characters are kept. A `"` or a `\` in the string still gets a `\` before it, and every character that section replaces is still replaced. The quoted string appears only in the one-line header of a view (`search`, `facets`, `kwic`, `dist`, `collocates`, `compare`, `profile`) and in the `rule …` line of `clean-report`, never in a column of an aligned table, so a tab in it moves nothing but the rest of its own line.
9. The parser answers one more question: is the query empty or only `in:` terms (`scout_query::is_empty_or_corpus_only`, TS `isEmptyOrCorpusOnly`; "An empty query and a query that only names corpora"). Only such a query stands for the whole corpus in `facets` and in a search in date order.

One grammar, two implementations that must agree exactly: Rust `crates/scout-query` (`src/scan.rs`) and TS `packages/scout-query` (`src/scan.ts`). Neither uses a regular expression or a built-in whitespace class for tokenizing. Parity is held by `packages/scout-query/fixtures/grammar-cases.json` (hand-written cases, one or more per rule below) and `fixtures/generated-cases.json` (600 seeded random queries and 400 labels made by `packages/scout-query/generated.test.ts`; the Rust test `crates/scout-query/tests/fixtures.rs` must reproduce every TS result).

**WS** (whitespace) is exactly this set of code points, a constant (`WS`) in both implementations: U+0009–U+000D (tab, LF, VT, FF, CR), U+0020, U+0085, U+00A0, U+1680, U+2000–U+200A, U+2028, U+2029, U+202F, U+205F, U+3000. Nothing else separates tokens (U+FEFF does not).

**Double-quote variants**: `"` U+0022, `“` U+201C, `”` U+201D, `„` U+201E, `‟` U+201F. **Opening quotes**: `"`, `“`, `„`.

**Tokens.** The query is read left to right; WS separates tokens. A construct is recognised ONLY at a token start (the start of the query or right after WS), tried in this order:

1. **Phrase**: an opening quote, then text up to the next double-quote variant (any of them closes). Inside, `\` before a double-quote variant or before another `\` makes that character literal; any other `\` is itself. One `-` directly before the opening quote negates the phrase. The text is phrase text: nothing inside is a field, a regex or an operator. An opening quote that is never closed is not a phrase (rule 4 applies).
2. **Regex**: `/body/flags`, where `body` is one or more characters up to the next unescaped `/` (`\` escapes the next character; WS is allowed in the body), `flags` is zero or more of `a`–`z`, and the next character is WS or the end. Anything else starting with `/` is a term.
3. **Field**: `name:value`, where `name` (everything before the first `:` of the token, ASCII case ignored) is a known field name. The value is either quoted (rule 1's quote rules, no negation) or bare: everything up to the next WS, taken literally, possibly empty. An unknown name makes the whole token a term.
4. **Term**: everything up to the next WS. So `https://example.org/theme:Foo`, `km/h`, `(au:x)`, `foo-au:bar` and `-au:x` are single terms; a leading `-` negates a term; `OR`, `|` and `AND` (ASCII case ignored) are operators; `x*` is a prefix.

Whatever directly follows a closed phrase or quoted value, up to the next WS, is a plain term (rule 4) and never a construct: `theme:"v"tail` is the field plus the term `tail`.

**Repeated fields.** A field may occur more than once in a query. What that means depends on the field, and on nothing else (not on the order of the terms, nor on what stands between them):

| fields | a repeated term |
|---|---|
| `theme:` `entity:` `event:` | **AND**: every term counts; a document must carry every theme and every entity named and be in every event named. The same value twice is that value once |
| `month:` | **OR**: every term counts; a document must be in any one of the months named (a document has one month, so AND would match nothing). `YYYY` and `YYYY-MM` may be mixed: `month:2026 month:2025-12` is all of 2026 and December 2025 |
| every other field: `au:`/`author:`, `ti:`/`title:`, `ty:`/`type:` and the type shortcuts (`tw:`, `bo:`, `art:` …), `tag:`, `co:`/`color:`/`colour:`, `y:`/`d:`/`date:`/`year:`, `after:`/`since:`/`from:`, `before:`/`until:`/`to:`, `in:`, `lang:`, `genre:`, `source:`, `ai:` | **the last wins**: the field has one value, the last one given, exactly as before. (`lang:`, `genre:`, `source:`, `ty:`, `in:` take a comma list in that one value; `i:` and `zo:` are flags.) |

- The parser (both implementations) returns, beside the scalar `theme`, `entity`, `event`, `month` (the last value, unchanged), the lists `themes`, `entities`, `events`, `months`: every value in query order, empty values included, `[]` when the field is not given. Field names fold ASCII case, so `THEME:a theme:b` is two `theme:` terms. A type shortcut's value is free text and fills no list.
- The engine reads the lists. Each term must have a value: an empty one (`theme:`, `theme:""`, a blank one) is a usage error, `theme: needs a value`, wherever it stands; each `month:` value must be `YYYY-MM` or `YYYY`.
- The grammar has no negated field: `-theme:x` is one negated term (rule 4), the text `theme:x` excluded from passages. That is unchanged.
- In `plan.filters` and `filters`: "`filters` object" above (the string key is the last term; the list key appears from two terms up).
- `scout topics theme-months` refuses any `month:` term, one or several ("`scout topics theme-months`").
- `topics::field_query(field, value)` writes ONE term; to select several values, join the terms with a space.

**Writing a value** (`scout_corpus::topics::field_query(field, value)`, `scout_query::quote_value`, TS `quoteValue`): bare when the value is non-empty and holds no WS code point, no double-quote variant and no `\`; otherwise `"…"` with a `\` before every double-quote variant and every `\`. A value written this way reads back unchanged (`entity:"GPT \"Astra\""`).

**Fold (matching).** Every field value is compared under one function (`scout_corpus::normalize::fold`), applied to the stored value and to the query value alike, for every text field (`tag: co: au: ti: ty: lang: genre: source: theme: entity: event:`) and for facet grouping: Unicode NFC → apostrophe variants (`‘ ’ ‚ ‛ ′ ʼ`) to `'` and double-quote variants (plus `″`) to `"` → each run of WS to one space, none at the ends → lowercase. `tag: co: lang: genre: source: theme: entity: event:` are equality under the fold; `ty:` equality after dropping a final `s`; `au: ti:` substring. Not folded, because they are parsed values or identifiers: `after: before: y: month:` (dates), `ai:` (boolean), `in:` (corpus ids, exact). Field values are kept as typed in `plan` and in the JSON `filters`.

## `scout search <query> [--in …] [--limit 20] [--sort relevance|newest|oldest] [--json]` — schema_version 1

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
| `theme:A theme:B`, `entity:X entity:Y`, `event:a event:b` | documents carrying BOTH (AND; "Repeated fields") |
| `month:2026-02 month:2026-03` | documents of EITHER month (OR) |

`theme:`, `entity:`, `event:` and `month:` take several values, one per term; every other field takes one value and a repeated one keeps the last ("Repeated fields" in "Query grammar"). `tag:` is unchanged: it never matches topic-layer themes or entities, because those are model-made and `tag:` holds the reader's own labels (Readwise tags) and the writing's frontmatter `topics`.

`i:` (has image) has no meaning here and is listed in `plan.ignored`. A query with filters or a regex but no positive term lists every passage that passes them (score 0), its documents in path order.

**An empty query and a query that only names corpora.** In relevance order (the default) these are what they were: an empty or blank query is a usage error (`empty query`, exit 2), and a query with no term and no filter (`in:highlights` alone, `i:` alone) returns no result (exit 1, `total_documents` 0). Neither lists the corpus. With `--sort newest` or `--sort oldest` an empty query and one that only names corpora list every document that has an indexed passage ("Order").

Which query that is, is the parser's answer about the text, not about what the text compiles to (`scout_query::is_empty_or_corpus_only(raw)` and `Query::is_empty_or_corpus_only()`, TS `isEmptyOrCorpusOnly(raw)`; shared cases under `corpus_only` in `fixtures/grammar-cases.json`): true when the query has no token at all (empty, or WS only) or when every token is an `in:` field term. Any other token makes it false, also one that selects nothing by itself: `OR`, `!`, `i:`, `tag:""`, `in:notes OR`. Such a query is not the empty query anywhere: `scout facets OR` counts no document (exit 1) and `scout search OR --sort newest` lists none (exit 1), exactly as before engine `scout-corpus/13`. `scout topics theme-months OR` gives the matrix of no query, also as before: each of its cells is the query with a `month:` term added, which selects by that month. For the same reason the base a facet is counted over when its own terms are taken out (`month`, `ai`; "`scout facets`") is every tagged document whenever nothing else in the query selects: `facets "OR month:2026-02"` lists every month, each with the total of `OR month:<that month>`.

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

`plan` keys other than `any_of`, `not` and `filters` appear only when set; so do `date_source`, `kind`, `source`, `topics`, the hit's `tags`, `color`, `saved_at` (highlights) and the top-level `sort` (only `"newest"` or `"oldest"`; absent in relevance order). Every result carries what a list of documents shows, each key always present, null when unknown: `title`, `author`, `date` with `date_display`, `public_url`, `genre`, `lang` (and `kind`, `source`, `date_source` when set).

A result whose document carries topic tags has a `topics` object (the same object in semantic and hybrid results):

```json
"topics": {
  "key": "x:1965432109876543210", "themes": ["AI and mathematics"], "entities": ["GPT-6 Astra", "Terence Tao"],
  "entity_types": {"gpt-6 astra": "model", "terence tao": "person"},
  "event": "2026-09/E21", "event_name": "Fields Medallists' declaration", "events": ["2026-09-fields-declaration"],
  "ai_related": true, "month": "2026-09",
  "confidence": "medium", "run_id": "2026-10-11-final-2026-09", "prompt_version": "topics-v2",
  "mentioned_works": ["readwise:rw_book_49885089"],
  "corrected": ["themes"]
}
```

`corrected` (only when non-empty) names the fields a correction overrode, in the order `themes`, `entities`, `event`, `event_name`, `ai_related`, `events` (`events` = the line carried `event_add` or `event_remove`). `events` is every event the document is in (see "Lasting events"); it is present exactly when the corpus has a records folder, and absent otherwise, when the document's one event is `event`. `event` and `event_name` stay the tag row's own monthly event; both are null when a correction took the document out of that event. `entities` is a list of names (strings), as it always was; `entity_types` beside it gives the type of each entity that has one, keyed by the name under the fold of "Query grammar" (so look a name up by its fold: NFC, quotes unified, whitespace runs to one space, lowercase). It is absent when no entity of the document has a type ("Topic layer", "Entity types"). `mentioned_works` (only when non-empty; engine `scout-corpus/14`) is the tag row's list of the works the document mentions, each a layer key (`<source_system>:<source_id>`) as the row writes it, in the row's order ("Topic layer", "Mentioned works"). The same `topics` object is the `tags` of a `scout correct` reply, so that carries the key too, under the same rule.

### Order (`--sort relevance|newest|oldest`)

Facade: `SearchQuery.sort` (`"relevance"`, the default, `"newest"`, `"oldest"`). An unknown value is a usage error (exit 2).

- **`relevance`** is the order described below and nothing else changes: the reply is byte for byte what it was before `--sort` existed (no `sort` key), whether the option is left out or given as `relevance`.
- **`newest` / `oldest`** order the documents by their own date: the result's `date` (writing: the frontmatter date; a tweet: its timestamp; a highlights work: its publication date, else the day of its first highlight, `date_source: "saved"`), never the topic layer's month. The key is the day of that date (a year or a month counts as its first day, a timestamp as its day: the reading `after:` / `before:` use), then the `date` text itself, so two timestamps of one day keep their order. `newest` is that key descending, `oldest` ascending. **A document without a date, or with one that does not parse, comes last in both orders.** Ties (the same `date`, or both undated): corpus id, then path, ascending in both orders.
- The whole result is ordered before `--limit` cuts it, over every searched corpus together; `total_documents` and `total_passages` are the whole match, as always. Paging is `--limit` (the first N of that order).
- **A query needs no term and no filter.** With a date order an empty query, and one that only names corpora (`in:highlights`), lists every document with an indexed passage (a query of other tokens that select nothing, `OR` or `i:`, lists none, as by relevance: "An empty query and a query that only names corpora"); a query of filters lists the documents that pass them, as it does by relevance. Each result shows its first passages in line order (score 0) when the query has no term.
- With a term, the documents are the ones relevance order returns, each with the same `score`, `rank` and hits; only their order differs.
- A date order is a full-text search: `auto` does not use vectors, and `--semantic` or `--hybrid` with it is a usage error (`sort newest or oldest is a full-text search; it does not combine with semantic or hybrid`, exit 2).
- The reply carries `"sort": "newest"` (or `"oldest"`) after `mode`.

Order by relevance: `results` by `rank` desc, then `score` desc, corpus, path; `hits` by score desc, then line. `score` is BM25 (plus the title's score when the title matches); `rank` = `score` / the best `score` in the same corpus, so each corpus's best document has rank 1 and `--in a,b,c` interleaves the corpora instead of letting one corpus's BM25 scale win. With one corpus, rank order = score order.

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

A corpus folder may hold a topic layer: `analysis/topics/current.jsonl` (one model-tagged item per line, keyed `<source_system>:<source_id>`; format in `docs/specs/2026-10-10-topic-tagging-layer.md`) and the optional, append-only `analysis/topics/corrections.jsonl`. `scout index build` reads both and writes neither; searching reads the index's copy. `current.jsonl` is never written by scout; `corrections.jsonl` has one writer, `scout correct` (see "Correcting an item's tags").

- **Build.** `scout index build` copies the layer into the derived index, joined to documents by key (the work's frontmatter `source_system` + `source_id`), else by the layer's `path` = the document key. It re-joins when either file's bytes or the corpus's documents change; a new layer never forces a full rebuild. No layer file = no tags are joined and the report has no `topics`; event records are still read (see "Lasting events").
- **Corrections.** One JSON object per line with `key` and any of `themes` (string list), `entities` (string list), `event` (string or null), `event_name`, `ai_related` (bool or null). Each field present replaces the layer's value for that key; later lines win; other keys (`by`, `date`, `note`) are provenance and are not read. A correction for a key the layer lacks is counted (`corrections_unmatched`), not applied; a line with a wrongly typed field is skipped whole (`corrections_bad_lines`). An event changed without an `event_name` takes the name the layer gives that event id. In a corpus with a records folder (and only there, see "Lasting events") two further fields put an item in more events or take it out of one: `event_add` and `event_remove`, each a list of event ids (lasting, merged-away or monthly; a single string is a wrongly typed field). Exact shape: `{"key": "x:1965432109876543210", "event_add": ["2026-09-gemini-launch"], "event_remove": ["2026-09/E21"], "by": "Dominik", "date": "2026-10-11"}`. They apply in line order to the item's events (see "Lasting events"), the later line winning; within one line removals follow additions. A removal also takes the item out of an event the model gave it.
- **Numbered corrections** (engine `scout-corpus/12`). A line whose `id` is a canonical numbered id is a numbered line: the lines `scout correct` writes. The canonical id of a number from 1 to 999,999,999 is `c` and the number written with at least six digits and no more than it needs (`c000001`, `c999999`, `c1000000`): one spelling per number, told by one function for the reader, the writer and `scout correct list`. It is read only when it is valid, and what is valid is decided by one function for the reader and for the writer (`correction_lines::Sequence::judge`): a text `at`, `date`, `by` and `key`; besides those and `id`, exactly one change and no other key; an id greater than the id of every earlier numbered line of the file, valid or not. The change is one of `theme_add`, `theme_remove`, `entity_add`, `entity_remove`, `event_add`, `event_remove` holding a list of one non-blank name (`entity_add` with an `entity_type` of the layer's types, which the reader keeps as that name's type, see "Entity types"; the others without one), or `undoes` holding a canonical numbered id (an `undoes` of another spelling, `c0000001`, makes the line a bad one). Any other numbered line is a bad line (`corrections_bad_lines`): it is not applied, and no later line undoes it. So a numbered line with a `note`, with `themes` or `entities`, with two changes, or with an id at or below an earlier one is skipped and counted, and `scout correct` refuses to append to a file that holds one, naming that line ("Correcting an item's tags"). A valid `theme_add` / `theme_remove` / `entity_add` / `entity_remove` patches the item's list as it stands at that line: an addition is not made when the list already has the name under the fold, and is added trimmed; a removal takes out every name equal under the fold. `corrected` then names `themes` / `entities`. They are read with or without a records folder. A valid numbered `event_add` / `event_remove` is read as on any line: in a corpus with a records folder; without one the line is counted in `corrections` and changes nothing. `undoes` makes the line an undo line: it is valid when it names an earlier valid correction (not an undo line) of the same `key` that no earlier line has undone. That correction is then not applied at all, as if it were not in the file (`corrections_undone` counts such lines; neither they nor the undo line count in `corrections`), also when its key has no tag row. An `id` that is absent or of another form (`c1`, `C000001`, `c000000`, and `c0000001` with a zero too many) makes no numbered line: the line is read as a hand-written one, on which `theme_add`, `theme_remove`, `entity_add`, `entity_remove` and `undoes` are unknown keys, not read and not type-checked. So a file without a numbered line reads exactly as it did before.
- **Month.** The item's `date` (its latest highlight) cut to `YYYY-MM`, else the layer's own `month` when that is `YYYY-MM`. `month:` filters on it, the `month` facet counts it and `theme-months` puts the item in its column: one function gives an item its month (`TopicInfo::layer_month`) and all three call it.
- **Entity types** (engine `scout-corpus/13`). A tag row may carry `entities_typed`, a list of `{"name": …, "type": …}` beside its `entities` (the list of names, which stays what the row's entities are). Each object with a non-blank text `name` and `type` whose name is one of the row's `entities` under the fold gives that entity its type (trimmed, otherwise as written; the layer's types are `person`, `org`, `model`, `product`, `work`, `event`, `place`, `other`); a later object of the same name wins. `entities_typed` never makes a row a bad line: whether a line is a row is decided without the key, which is skipped unread exactly as before engine `scout-corpus/13`, and the key is then read from the line a second time, apart. Each item of the list is judged alone: an item of another shape (not an object, a `name` or `type` that is not text or is blank, a name the row does not list) gives no type, and the well-formed items beside it still count. The whole value gives no type when it is not a list, when that second read cannot read it (a number out of range such as `1e400`, or nesting past the JSON reader's limit of 128 levels counted from the line's own object, anywhere inside the value), or when the key is on the line twice. A numbered correction's `entity_type` is different: it is part of what makes the line valid ("Numbered corrections"), so one that is not text naming a type of the layer (`7`, `null`, `["person"]`, `"thing"`), on any numbered line, makes that line a bad line for the reader and a refusal for `scout correct`, as it did before engine `scout-corpus/13`. The index stores the types with the tags, and a result shows them as `topics.entity_types` (name under the fold → type). Corrections: a numbered `entity_add` gives the name it adds the line's `entity_type` (also when the row has that name by now); a numbered `entity_remove` takes the name's type with the name; a hand-written `entities` list keeps the type of each name it keeps and gives a new name none; an undone line gives nothing. Types are not matched by any query field.
- **Mentioned works** (engine `scout-corpus/14`). A tag row may carry `mentioned_works`: a list of strings, each the layer key (`<source_system>:<source_id>`) of another work of the archive that the item mentions. It is a list of keys, never of titles or objects: the tagger computes it without a model by matching the item's person and work entities against the archive's authors and titles, and writes at most five keys per row (`docs/specs/2026-10-10-topic-tagging-layer.md`, "Prompt v3"). Scout reads what the row says and matches no title against anything. The rule is the one for `entities_typed`: the key never makes a row a bad line. Whether a line is a row is decided without it (it is skipped unread there, exactly as before engine `scout-corpus/14`), and it is then read from the line again, apart. Each item of the list is judged alone: a non-blank text is a key (trimmed, otherwise as written; a key the list already has under the fold is not taken twice), and an item of any other shape (a number, `null`, an object such as `{"key": …}`, a list, a blank text) is passed over while the texts beside it still count. The whole value gives no work when it is not a list, when that second read cannot read the line (a number out of range such as `1e400` anywhere in the value), or when the key is on the line twice. (A value nested past the JSON reader's limit is like any other value the second read cannot read: the first read skips the key unread and accepts the row, the second read fails, and the row gives no work.) The index stores the keys with the tags; a result shows them as `topics.mentioned_works`, and `scout facets` counts them (`works`). No correction changes them and no query field matches them. The row's `related` list (`[{"key", "relation"}]`) is not read.
- **Unparseable layer lines** (the file may be mid-write) are skipped and counted (`bad_lines`); a key repeated in the layer keeps its last line.
- **Matching** for themes, entities and event ids is equality under the fold of "Query grammar" (`entity:O'Reilly` matches `O’Reilly`). So every facet value, written with `field_query`, gives back exactly its `count` as `total_documents` (an event value that carries `corpus`: with `--in` that corpus) (tested over 400 generated labels with every whitespace code point, curly and straight quotes, apostrophes, backslashes, slashes and colons).
- **Reading limits.** Both files are streamed line by line; a line over 1 MiB (`topics::MAX_LINE_BYTES`) or not valid UTF-8 is skipped and counted (`bad_lines`, `corrections_bad_lines`), never applied. A blank line is passed over and not counted. The corrections file is streamed twice (first to find which corrections an undo line takes back, then to apply the rest in order; a file whose bytes differ between the two is an error, exit 2: run the build again), and no line is kept in memory once it is read: what is kept is the id and key of each valid numbered correction. **A corrections file over 64 MiB** (67,108,864 bytes, `correction_lines::MAX_FILE_BYTES`, the one limit `scout correct` keeps the file under) **is not read at all**: the build completes, applies NO correction, and says so: `corrections_skipped` and `warnings` in the report and a `WARNING` line in the human output ("`scout index build`"). The index is rebuilt from the file once it is back under the limit.
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
- **Engine `scout-corpus/14`.** The tags stored for a document also hold its tag row's `mentioned_works` and the day of the row's `date` ("Topic layer", "Mentioned works"; `month_spans` in "`scout events`"). An index built by an earlier engine is rebuilt in full at the next `index build`; until then it is read as it is, and its replies carry no `mentioned_works`, no `works` facet and no `month_spans`. For a corpus whose tag rows have no `mentioned_works` and no `date` that is a day, the rebuilt index gives the replies it gave before.
- **Engine `scout-corpus/13`.** Entity types are stored with the tags ("Topic layer", "Entity types"), and repeated `theme:` / `entity:` / `event:` / `month:` terms all count ("Repeated fields"). An index built by an earlier engine is rebuilt in full at the next `index build`; until then it is read as it is and its results carry no `entity_types`. For a corpus whose tag rows have no `entities_typed` and whose corrections add no entity, the rebuilt index and every reply to a query without a repeated topic field are what they were.
- **Engine `scout-corpus/12`.** Numbered corrections are read ("Topic layer"), and a record with an `origin` other than `model` or `dominik`, a `start`, `end`, `edited` or `removed` that is not a date, or an `end` before its `start` is skipped ("Bad records"). An index built by an earlier engine is rebuilt in full at the next `index build`; for a corpus with no numbered correction and no such record the rebuilt index and every reply are what they were.
- **Engine `scout-corpus/11`.** (Unchanged by derived spans and aliases: the index holds the records as their files have them, and `scout events` derives from that copy.) The index copy of a record carries `removed` when the record has it, and a removed event is taken out of its items' events as above. An index built by an earlier engine is rebuilt in full at the next `index build`. For a corpus where no record has `removed`, the rebuilt index and every reply are what they were.
- **Behaviour change (engine `scout-corpus/10`).** In a corpus with event records, `event:2026-09/E21` no longer selects only the documents tagged with that monthly event: it selects every document of the lasting event that holds `2026-09/E21`, across all its months. Add `month:2026-09` to narrow to one month. Facet values and `scout topics events` values are lasting ids where a record holds the event. `plan` and `filters` keep the id as typed.
- **Bad records.** A file is skipped, with a warning naming it, when it is not a plain file (a symlink or folder), is over 1 MiB (`events::MAX_RECORD_BYTES`), is not UTF-8, holds a control character anywhere in the file, frontmatter or body (the warning reads `control character in record`; the characters are listed below), has no frontmatter, has frontmatter that is not exactly one YAML document read without error to its end (a document end marker `...` on a line of the frontmatter is refused, with or without anything after it, as is a second document `--- x`, so no part of the block goes unchecked), has frontmatter that is not a YAML mapping of plain keys (aliases `*x` and repeated keys are refused), has YAML nested deeper than 3 levels (`events::MAX_YAML_DEPTH`: the mapping, a list value, and one level inside a value of a key that is not a record field; the warning reads `bad YAML (nested deeper than 3 levels)`, or the YAML scanner's own `recursion limit exceeded` for more than 255 open `[`/`{`), has no `id`, has an `id` that is not the file name, repeats an id already read (ids compare under the fold), has a wrongly typed field (`members` not a list, `name` a list), or has a value that a field cannot have: an `origin` that is not empty, `model` or `dominik` (`origin "Dominik" is not model or dominik`), a `start`, `end`, `edited` or `removed` that is not empty or a real calendar date `YYYY-MM-DD` (`start "early 2026" is not a date YYYY-MM-DD`; `2026-09`, `2026-9-1`, `2026-02-30` and a timestamp are not dates), or an `end` before its `start` (`end 2026-09-01 is before start 2026-09-02`). The other records are read. A `merged_into` that names no record or closes a cycle is warned about and that record is treated as not merged. A member already held by another event stays with the first record in id order, with a warning. The control characters refused are: every C0 control (U+0000 to U+001F) other than TAB, LF and a CR that is directly followed by LF; DEL (U+007F); the C1 controls (U+0080 to U+009F); U+2028 and U+2029; and U+FEFF, except one byte-order mark at the very start of the file, which is dropped as it always was (a second one, or one anywhere else, refuses the record). The whole file is checked before the frontmatter is looked for or any YAML is read, so a line of a record ends at LF and nowhere else, for this reader's own line checks and for the YAML scanner alike (YAML also ends a line at a bare CR and stops reading at a raw NUL). The rule is about the file's characters: one of them written as an escape inside a double-quoted value (`name: "a\0b\u2028c"`) is read as that character. An explicit YAML tag is not applied: every scalar is the text as written (`id: !!binary YQ==` is the id `YQ==`). This reader is the only implementation of the record format: `scout events seed` reads the records already in its out folder with it and passes every record it is about to write through it, so there is no second set of rules to keep in step. The cases are in `crates/scout-corpus/tests/fixtures/event-records/` (`verdicts.json` and `records/`), run by `crates/scout-corpus/tests/event_record_fixtures.rs`; a new rule needs a case there.
- **Build.** The records are copied into the derived index, with or without a topic layer, and re-read when any record file's name or bytes change. `index build` reports them (see `index build`).
- **Reading is not recursive.** The frontmatter is read by pulling YAML parser events one at a time and the read stops at the first value nested too deep, so a record nested 100,000 levels deep costs a warning, never the stack. Merge chains are followed in loops bounded by the number of records.

## `scout facets [query] [--in …] [--top 20] [--min-event-items 1] [--json]` — schema_version 1

Topic-layer counts over the documents `scout search` matches for the same query: the full-text and filter match, all of it (no `--limit`; semantic-only documents of hybrid search are not included). Facade: `Engine::facets(&FacetsQuery { query, in, top, min_event_items })`. Exit 1 when no document matches; 2 for a bad field value. The query may be left out.

- **An empty query, or one that only names corpora** (`scout facets`, `scout facets ''`, `scout facets in:highlights`; before engine `scout-corpus/13` an empty query was exit 2 and `in:` alone counted nothing; a query of other tokens that select nothing, `OR`, `!`, `i:`, `tag:""`, still counts nothing and is exit 1, see "An empty query and a query that only names corpora" in "Query grammar"): the documents counted are every indexed document of the searched corpora that has topic tags and at least one indexed passage, the documents `scout topics` counts. Untagged documents are left out, since they have no facet value, so here `total_documents` = `tagged_documents`. Those are exactly the documents a search with one more topic term selects from, so each count is still the `total_documents` of that search (`theme:<value>`, …).
- **A number is the search's number.** The count beside a value is the `total_documents` of `scout search` for this query with that value's term added, and it is counted by the selection search runs, not by a second one (tested for every value of every facet of a fixture, with and without terms of the same field already in the query):
  - **The one rule for which documents a facet is counted over.** A field whose further term narrows is counted over the documents of the WHOLE query; a field whose further term replaces or widens is counted over the documents of the query WITHOUT that field's own terms. In both cases the count is the total of the search the query becomes when the value is picked.
  - `theme`, `entity`, `event` narrow (terms are ANDed): with `theme:A` in the query the count beside theme B is the documents carrying A and B, which is `theme:A theme:B`; the count beside A itself is the query's own total.
  - `month` widens (terms are ORed): a month's count is what that month holds under the rest of the query, whether it is picked or not. It is the `total_documents` of the rest of the query with that month alone, and for a month not yet picked it is what adding it adds (the search with it added returns that many more documents, unless a picked year already holds the month). So picking a month does not empty the list of the others.
  - `ai` replaces (the last `ai:` term wins): each of the two counts is the `total_documents` of the query with that value as its `ai:` term, the rest unchanged, so `facets "ai:true"` and `facets "ai:false"` give the same two counts, and `"false"` there is the total of `ai:true ai:false`. (Before this, `ai` was counted under the whole query and the value not picked read 0.)
  - A query without a `month:` or `ai:` term is counted as before. No other facet is in the second group: these five are all the facets a query field selects by, and the other fields whose last term wins (`au:`, `ti:`, `y:`, `lang:`, …) have no facet.
  - `works` (engine `scout-corpus/14`) is counted over the documents of the whole query, by that same selection, and is informational: there is no `work:` field, so no search has a work's count as its total, and picking a value narrows nothing.
  - `total_documents`, `tagged_documents` and `plan` are the whole query's, `month:` and `ai:` terms included.

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
- An `entity` item carries `"type"` when a counted document gives that entity a type: `{"value": "Terence Tao", "count": 12, "type": "person"}`. Where the counted documents disagree it is the most frequent type among them (each document once), and of equally frequent ones the first in name order (`model` before `product`). An entity no counted document types has no `type` key. No other facet's items have the key.
- `theme`, `entity`, `event`: count desc, then value; at most `--top` items (0 = all); `distinct` = values before the cut. `month`: every month, chronological. An event item's `value` is the lasting id where a record holds the event, else the monthly id; its `name` is the record's `name`, else the most frequent `event_name` of the counted documents tagged with that monthly id. A document counts once in each of its events.
- Themes, entities and months are counted across the corpora. Whether events are counted per corpus or across them follows the rule in "Lasting events" ("Whose event an id is"). Per corpus (some searched corpus has a records folder) and more than one corpus in `corpora`: every `event` item carries `"corpus": "<id>"` after `name`, and the same id in two corpora is two items, each with its own count and name: `{"value": "2026-09/E21", "count": 3, "name": "…", "corpus": "highlights"}`; ties in count and value are ordered by corpus; the count of an item with `corpus` is the `total_documents` of `event:<value>` run with `--in <corpus>` (without `--in` the query matches that id in every searched corpus). Otherwise (one corpus in `corpora`, or no searched corpus with a records folder) no item has the key and an item's count is the `total_documents` of `event:<value>` over the searched corpora, so that output is unchanged from before lasting events.
- `--min-event-items N` (default 1) leaves out of the `event` facet every event with fewer than N items. An event's size is its tagged documents with an indexed passage over the whole of its own corpus (the `items` of `scout events` for that corpus and id; sizes are never added up across corpora), not its count in this result. (Where no searched corpus has a records folder an event value spans the searched corpora, and so does its size: the sum of those `items`.) So `--min-event-items 2` hides one-item events, and an event of five items is kept even when one of them matches. `distinct` counts the events kept. Above 1 the reply carries `"min_event_items": N` after `top`; at 1 the key is absent.
- A value's displayed spelling is its most frequent one among the counted documents (ties: alphabetically first).
- **`works`** (engine `scout-corpus/14`): the works the counted documents mention, from their tag rows' `mentioned_works` ("Topic layer", "Mentioned works"). The key is in `facets` after `ai` and is absent when no counted document mentions a work, so a corpus without the field has the five facets it had.

  ```json
  "works": {"distinct": 1160, "items": [
    {"value": "readwise:rw_book_49885089", "count": 90, "corpus": "highlights", "key": "readwise:rw_book_49885089",
     "title": "Software 2.0", "rel_path": "readings/works/….md", "path": "/abs/archive/readings/works/….md"},
    {"value": "readwise:rw_book_1", "count": 2}
  ]}
  ```

  - `value` is the work as the layer writes it, which is a layer key (a string; the layer gives no title). Values are one under the fold, shown in the most frequent spelling; `count` = the counted documents that mention it, each once. Count desc, then value; at most `--top` items; `distinct` = values before the cut.
  - A value that is a document of the index is named: `key` (the key in the spelling that document has), `title`, `corpus`, `rel_path` and `path`, the keys a search result has. The document is found by the layer's own data and nothing else: the key is looked up exactly (trimmed, not folded) as `index build` looks up a tag row's key, first among the documents' `source_system` + `source_id`, then among the tag rows the build joined by `path`, in the corpus whose documents mention it (with several corpora searched: the first of them, in `corpora` order, that has a counted document mentioning it). A key that names no indexed document (the work is not in the corpus, or has no source key) has `value` and `count` only. Titles are never compared.
  - Not in `scout topics`: there is no `works` field there.
- **Related events.** For the events that share items with an event, ask `scout facets "event:<id>"` and read its `event` facet: it lists every event the selected documents are in, the selected event included. The selected event has all its items (`count` = `total_documents`; `event` narrows, so the value in the query counts the whole result), and each other item is another event with the number of items the two share. An event that shares none has itself alone. The items are ordered as in any facet, by count and then by value, so the selected event has no place of its own: another event that shares all its items has the same count and comes before it when its id sorts first. Find the selected event by its id (`value`, and `corpus` when present), not by position. `--top` cuts this list like any other and can cut the selected event out of it; ask with `--top 0` (or a top of at least `distinct`) when the others are needed reliably. `--min-event-items` applies as elsewhere. (Tested: `event_page_reads.rs`, the tie included.)

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

`field` is `theme`, `entity`, `event` or `month`; `items` are ordered as in `facets` (months chronological); `--top` 0 keeps all. An entity item carries `type` as in `facets`. Event values and names are as in `facets` (lasting ids where a record holds the event), and so is `corpus`, by the same rule ("Whose event an id is"): where a searched corpus has a records folder and more than one corpus is searched, every event item carries it and the same id in two corpora is two items; where none has, the same id is one item without it.

## `scout topics theme-months [--from YYYY-MM] [--to YYYY-MM] [--in …] [QUERY…] [--json]` — schema_version 1

The theme × month matrix in one call: documents per theme and layer month. Read-only. Facade: `Engine::theme_months(&ThemeMonthsQuery { in, from, to, query })`. Exit 1 when no document is counted; 2 for a bound that is not exactly `YYYY-MM`, seven bytes naming a real month (a blank one, `--from ''` or `--to '  '`, one with a space around it, a bare year `2026`, a year padded to seven bytes `'2026   '` and `2026-13` included: only a bound that is not given is taken from the documents), `--to` before `--from`, a range of more than 1,800 months, `--month`, a query with a `month:` term (one or several), or a bad query. (`--from`, `--to` and a query are refused with any other `scout topics` field.)

```json
{
  "schema_version": 1,
  "query": "",
  "corpora": ["highlights"],
  "from": "2026-02",
  "to": "2026-03",
  "months": ["2026-02", "2026-03"],
  "themes": ["Vibe coding and AI-assisted programming", "AI and mathematics"],
  "counts": [[219, 240], [31, 52]],
  "theme_totals": [459, 83],
  "month_totals": [1034, 1101],
  "documents": 2135
}
```

- **Documents counted.** A month's documents are the ones `scout facets` counts for the same query with `month:YYYY-MM` added, and they are chosen by the same function of the engine, whatever the query is. The matrix is made in one pass (engine `scout-corpus/13`; before, the selection ran once per month): that function selects the documents of the query once per corpus, and each tagged one goes in the column of its month, the month the `month:` filter compares (one function gives an item its month, "Topic layer", "Month"; `month:YYYY-MM` is equality with it, so the query with `month:M` added selects exactly the selected documents whose month is `M`). The queries: none (every tagged document with at least one indexed passage, as `scout topics`), one that only selects corpora (`in:one`: every such document of those corpora), fields, terms (the grammar of `scout search`). A query with a `month:` field is refused (`month:2026-02 in the query: theme-months counts every month of its range, so a query does not pick one; use --from and --to`): the months are the columns, chosen with `--from` and `--to`. The field is told by the parser of `scout search`, so `month:` with or without other terms, as `YYYY` or `YYYY-MM`, is refused, and the text `month:` inside a quoted phrase is a phrase. A document counts in its layer month ("Topic layer", "Month"); one whose tags give no month is counted nowhere.
- **`months`**: every month from `from` to `to`, ascending, months without a document included. A bound not given is the earliest (latest) month of a counted document; `from` and `to` in the reply are the first and last of `months`, null when there is none (no bound given and nothing counted). A bound beyond the documents is kept: the months asked for are listed, with zeros.
- **`themes`**: every theme at least one counted document of the range carries, by `theme_totals` descending, then by name. A theme is its name under the fold of "Query grammar"; the spelling shown is the most frequent one among the counted documents (ties: alphabetically first), as in a facet.
- **`counts[t][m]`**: the documents carrying `themes[t]` whose month is `months[m]`. A document with two themes counts in both rows; a theme named twice on one document counts once. `theme_totals[t]` is row `t` added up (a document has one month).
- **`month_totals[m]`**: the tagged documents of `months[m]`, each once, with or without a theme. `documents` is their sum.
- **The same numbers as `facets`.** `counts[t][m]` is the count `scout facets "month:<months[m]> <QUERY>" --top 0` gives `themes[t]` in its `theme` facet (0 when the facet has no such value), and `month_totals[m]` is that reply's `tagged_documents`, with the same `--in`. This holds for every query `theme-months` accepts, which is why one with a `month:` term is not accepted: in `facets` a second `month:` term is ORed with the first, so the documents counted would not have been the cell's month alone. (Tested for every cell of a fixture: without a query, with a query that only selects corpora (`in:`), with corpora and a field or a term, and with queries of terms and of each other field, over one corpus and two.)
- **Several corpora.** As for the theme facet: a theme is its name, and its counts are added up over the searched corpora (`corpora`). There is no per-corpus split; ask per corpus with `--in`.

Human output: one line per theme, its total, its name and the months that have a count (`2026-02:219`).

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
    "months": [{"month": "2026-09", "count": 41}, {"month": "2026-10", "count": 2}],
    "month_spans": {"2026-09": {"first": "2026-09-02", "last": "2026-09-30"}, "2026-10": {"first": "2026-10-01", "last": "2026-10-04"}},
    "undated_items": 1,
    "own_start": "2026-09-04",
    "members": ["2026-09/E21"],
    "aliases": ["Fields declaration", "Mathematicians' backlash"],
    "own_aliases": ["Fields declaration"],
    "merged_from": [{"id": "2026-10-mathematicians-backlash", "name": "Mathematicians' backlash", "merge_id": "m000012"}],
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
- **`merge_id`** (on a `merged_from` entry): the id of the merge-log line in effect that merged that record in, the id `scout events unmerge` takes; for a record merged in through a chain it is the line of its own merge (undoing it takes that record, and what is merged into it, out of the event). It is absent for a record merged by hand (`merged_into` set with no line in effect, the `merged-without-log` of `scout events check`), which is undone by clearing the field by hand. The ids come from the events folder as it is at the time of the listing, never from the index: the corpus's `analysis/events/` records and its `merge-log.jsonl` are read as `unmerge` reads them (the one record reader, the one log reader), and the merges in effect are told by the function `scout events merges` lists them with. This is done only for a corpus whose index has a merged record. An entry gets the id of the merge in effect for its record only when the folder agrees with the index about it: every record on the index's chain from the entry's record to the event it is listed under is in the folder with the same `merged_into`. So an id the listing gives is one `unmerge` finds in effect, and the merge's `a` and `b` are the entry's record and the record the index has it merged into. `merge_id` names the merge in effect; whether `unmerge` accepts it now is said by `undoable` (below): an entry with a `merge_id` and no `undoable: false` is one `unmerge` accepts now. Where the folder does not agree (the merge was undone, the record's `merged_into` was changed by hand, the listed event has itself been merged since), the entry is still listed (until `index build`), has no `merge_id`, and the reply carries `stale`. The folder is opened as `unmerge` opens it (`<corpus root>/analysis/events` under the edit commands' path rules). An events folder or a log that cannot be read (see "`merge-log.jsonl`", "Reading"; for the folder: one the edit commands refuse to open, a file in it that is no record, or a `merged_into` that names no record or closes a cycle, the cases in which `unmerge` is refused too) costs the ids and nothing else: the listing is whole, without `merge_id` and without `stale`, and a note says so on stderr (`scout: note: no merge_id for <corpus>: the merge log was not read (…)`, or `… the events folder was not read (…)`).
- **`undoable`, `why_not`** (on a `merged_from` entry that has a `merge_id`): `"undoable": false` and `"why_not": "<the refusal>"` when `scout events unmerge <merge_id>` would be refused now; both keys are absent when it would be accepted, and on an entry without a `merge_id`. The answer is `unmerge`'s own: every check `unmerge` makes before its first write is one function, which `unmerge` runs and then writes, and which the listing runs without writing, so `why_not` is the refusal `unmerge` gives, word for word (it names files under the corpus root as resolved, the path `unmerge --archive <corpus root>` names them by). The refusals a merge in effect can meet: record `a`'s file is not in the form scout writes (a comment, a key scout does not read), so it is not rewritten; with `a` unmerged the reader would not read the folder as meant (a monthly id that `a` and another record both list); the log has no id left or cannot take the undo line. The records folder and the log are read once per corpus for the listing and each merge is asked against that one read. The whole record set is gone over once, and each merge is then answered from what its undo can change (record `a`, the records merged into it and the monthly ids they list), by the function `unmerge` itself uses; so the cost of the listing grows with the size of the folder, not with the folder times its merges (`event_edit/unmerge_check.rs`, tested against a whole re-read of the folder per merge, word for word).
- **`stale`** (top level, `true` or absent): the events folder changed since the index was built; rebuild the index (`scout index build`). It is given once for the whole listing, when for any searched corpus a record the index has as merged in is not merged in that way in the folder now (see `merge_id`, whose entries then have none). It is found by the same read as the ids, so it is only looked for in a corpus whose index has a merged record: a folder change that touches no record the index has as merged in (a first merge made since the build, an edited name) is not reported here.
- **`own_aliases`**: the record's own aliases, the ones `scout events set --remove-alias` can remove. Present only when `aliases` differs from them, which is when a merged record added a name (`[]`: the record has none of its own); the key is absent otherwise, as for `own_start` / `own_end`.
- **`month_spans`**: by month, `{first, last}`: the earliest and the latest day (`YYYY-MM-DD`) of the event's items in that month. An item's day is its tag row's `date` when that is a calendar day (the first ten characters of the value) and is the date that gave the item its month ("Topic layer", "Month"); it is the layer's date (the item's latest highlight), not the document's own `date`. An item whose month comes from the row's `month` alone has no day and is counted in `months` only, so a month none of whose items has a day has no entry, and the key is absent when no month has one. `months` is unchanged: a list of `{month, count}`.
- **`undated_items`**: the items of the event that have no layer month: they are in `items` and in no month of `months` (`items` = the sum of the counts in `months` + `undated_items`). The key is absent when there are none. They are reached with the rest: `scout search "event:<id>"` lists every item of the event (`total_documents` = `items`), and the undated ones are those whose `topics.month` is null. No `month:` value selects them alone. In `--sort oldest` or `newest` they are placed by their document's own date like any other result ("Order"): last only when the document has no date either.
- An event nothing is merged into has none of `own_start`, `own_end`, `own_aliases` and `merged_from`, and its `start`, `end` and `aliases` are its record's. A monthly event no record holds is listed with its monthly id, `record: false`, the layer's name, null dates and empty lists.
- `--month` keeps the events with at least one item in that month (`YYYY-MM`, or `YYYY` for any month of the year) and adds `month_items` (that month's count) to each; `items` and `months` stay whole.
- `--min-items N` leaves out events with fewer items (`2` hides one-item events; `0` also lists records without items, which is every record of a corpus that has no topic layer). `min_items` counts an event's `items`: its items in the whole of its corpus, all months, whatever `--month` is. A record without items is listed like any other row, with `items: 0` and `months: []`; its `origin` (`model`, `dominik` or null) and `edited` say whose it is, so a caller can keep his own empty events in view and leave the rest out. With `--month` an event without an item in that month is not listed, so a record without items never is. `--merged` also lists records merged into another (`merged_into` set; their items belong to the event they resolve to, so `items` is 0); without it they are not listed. `merged_into` is the value in effect: null for a record whose file names a missing record or closes a cycle. `--removed` also lists removed records (their items no longer have the event, so `items` is 0); without it they are not listed. Only then does the output differ from before the field: the reply carries `"removed": true` after `merged`, and a removed record's row carries `"removed": "<date>"` after `merged_into`. Without the flag neither key is present.
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

`scout events set | merge | unmerge | apply-proposals | proposals | decide | remove | restore | create | check | merges`. Each works on the records folder of one archive, `<archive>/analysis/events`, given with `--archive <path>` (required). No registry and no index are read or written: run `scout index build` afterwards for search to see a change. Facade: `scout_corpus::event_edit::EventEditor::open(&archive)?` and one method per command, each taking typed arguments and returning the struct the CLI prints. Each command prints one JSON line on stdout and exits 0; a refusal exits 2 with the reason on stderr and nothing on stdout. Item-level changes (an item into or out of an event) are not here: they are `scout correct event-add` / `event-remove` ("Correcting an item's tags"), which append `event_add` / `event_remove` lines to `corrections.jsonl`.

**What every edit does before it writes.** There is one implementation of the record format, the reader of "Lasting events", and the edits stand on it as the seed does.

- The folder is read once, by the reader. If it skips any file ("Bad records"), or a record's `merged_into` names no record or closes a cycle, the command stops and names the file.
- Each record the command changes is written by the seed's writer, read back by the reader's own check (`events::parse_bytes`) and compared with the record meant.
- The whole folder as it would be (every record as its file has it, the changed ones in place) is resolved by the reader's own rules. It must read every record, follow every `merged_into`, give no warning it does not give now, and its id table (which event each record id and monthly id resolves to) must be exactly the one the command means: unchanged for `set`, `remove`, `restore`; the same plus the new id for `create`; every id held by `a` held by `b` and nothing else moved for `merge a b`; `a`, every record merged into `a` and the monthly ids those records hold held by `a`, and nothing else moved, for `unmerge`.
- Only then is anything written. A refusal leaves every file as it was: no record, no log line, no temporary file.

**What is kept.** A command changes the fields it names and no other. Unless it sets the description (`set --description`, `create`), it rewrites only the frontmatter block: the bytes after the closing `---` line are carried over exactly as they are in the file. A record is rewritten only when its file starts with exactly the frontmatter the writer writes for it. A file with anything else there (a key scout does not read, a YAML comment, values in another YAML style, CR LF line ends, a byte-order mark) is refused: `<file>: the frontmatter is not as scout writes it …`, since rewriting it would lose what the reader does not read. The same for a record whose file is not named `<id>.md` (`x.md.md` holds the id `x`). No command creates the records folder.

**Replacing a record file.** (1) The target's permission bits are read (it must be a plain file; a link is not followed). (2) A new file named `.scout-edit-<pid>-<time>-<n>.tmp` (a name no record can have) is created in the records folder, create-new and no-follow, so it is a file of its own and not a link of either kind to another; it is created with mode 0600, given the target's permission bits on its descriptor, written and synced. (3) The target is read again, without following a link, and must be a plain file holding exactly the bytes the command read at its start; if not, the new file is removed, the target is not touched and the command stops (`changed since it was read`, or `no longer a plain file`). (4) The new file is renamed over the target, and the folder is synced. The old file is never opened for writing: the rename gives the record's name the new file. A record that has a second name (a hard link, inside the folder or outside it) is replaced under its own name only; the other name keeps the old bytes, and nothing outside the folder is written. Of the old file only the permission bits are carried over (not its owner, group, extended attributes or times). Not defended: a write to the target in the instant between (3) and (4) is lost (the rename replaces the file).

**Modes of new files.** A new file (a record made by `create` or by `scout events seed`, a log made by its first line, `corrections.jsonl` made by its first line) is created with the default mode of a new file: its permission bits are what the process's umask leaves of 0666 (0644 under umask 022, 0600 under 077, 0664 under 002). No mode is set on it afterwards, and a later append leaves its bits as they are. Only a replaced record has bits set by scout: those of the file it replaces (above).

**Paths.** As for the seed: an `--archive` path with a `..` component is refused; `<archive>/analysis/events` must be a real folder, not a symlink (symlinks above it are accepted). Another process replacing the folder while a command runs is outside what it defends against.

**Inputs.** A name, an alias or a reason is trimmed and refused when it holds a control character (`char::is_control`, U+2028, U+2029 or U+FEFF; so also a line break or a TAB): `the name holds a control character (U+000A) …`. A description is cleaned by the seed's body rule (see `scout events seed`, "A record"). A date is `YYYY-MM-DD`, a real calendar day. Ids are compared under the fold of "Query grammar".

### `scout events set <id> --archive <path> [--name s] [--description s] [--start d] [--end d] [--add-alias s]… [--remove-alias s]…`

Changes the named fields of one record and sets its `edited` to today's date (UTC). `origin`, `members`, `themes`, `merged_into` and `removed` are left as they are. `--description` replaces the body. `--end ""` clears `end`. Aliases compare under the fold; removals are made first, then additions (an alias the record already has is not added twice). Facade: `EventEditor::set(id, &FieldEdit { name, description, start, end, add_aliases, remove_aliases })`.

```json
{"tool":"scout events set","at":"2026-10-12T08:00:00.000Z","id":"2026-09-alpha","file":"/abs/archive/analysis/events/2026-09-alpha.md","changed":true,"record":{"id":"2026-09-alpha","name":"Alpha","aliases":[],"start":"2026-09-05","end":null,"members":["2026-09/E01"],"themes":["T"],"origin":"model","edited":"2026-10-12","merged_into":null,"description":"…"}}
```

- `record` is the record as it is now (`removed` is present only on a removed record). `changed` is false, and nothing is written, when the file already is as asked (the same values on a record already `edited` today).
- Refused: no such record; no field given; an empty name or alias; a control character in a name or alias; a `--start` or `--end` that is not a date (`--start ""` included); `end` before `start`; with `--start` or `--end` given (`--end ""` too), a bound the record keeps that is not a date (`<id>: start "…" is not a date YYYY-MM-DD; set it to one in the same command`; the reader no longer reads such a record, so this is reached only if the reader's rule is loosened); an alias to remove that the record does not have; a record the reader would not read back (over 1 MiB).

`remove`, `restore` and `create` print the same object with their own `tool`.

### `scout events remove <id> --archive <path>` / `scout events restore <id> --archive <path>`

`remove` sets `removed: <today>` on the record; `restore` takes it off. Nothing else of the record changes (`edited` neither), so `restore` after `remove` leaves the file byte for byte as it was. What a removed event means for search is in "Lasting events" ("Removed events"). Removing a removed record, or restoring one that is not removed, writes nothing (`changed: false`). Refused: no such record; `remove` of a record merged into another (`<id>: merged into <b>; remove <b>, or undo the merge first`). Facade: `EventEditor::remove(id)`, `EventEditor::restore(id)`.

### `scout events create --archive <path> --name s [--description s] [--start d] [--end d]`

Creates a record with `origin: dominik`, `edited` today, no members, no aliases, no themes. Its id is `YYYY-MM-<slug>` by the seed's naming (the slug of the name, `event-x` when the name gives none, `-2`, `-3` … when the id is in use by a record or by any entry of the folder), the month taken from `--start`, else from today. The file is created as the seed creates one: the name is looked up in the folder before the write and the open is create-exclusive and no-follow, so an existing name is never written. The new file's mode is what the umask leaves of 0666 ("Modes of new files"). Refused: an empty name or one with a control character; a date that is not `YYYY-MM-DD`; `end` before `start`; a name that appeared in the folder after the look-up. Facade: `EventEditor::create(&NewEvent { name, description, start, end })`.

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

Refused, with nothing changed: `a` and `b` are one record; either is missing; either is already merged into another record (which also rules out a cycle: `b` merged into `a` is a `b` that is merged away); either is removed; either has a `start` or `end` that is not a date; `--by model` with `a` edited; a reason with a control character; `a`'s file not as the writer writes it ("What is kept"); a merge after which the reader would give a warning it does not give now (a monthly id that `a` and another event both list would then be reported against a different event); a merge of `a` that is in the log and was never applied (`unmerge` that line first; see "Order of writes"); a log that cannot take the line (see "Appending": a second hard link, a file not open to writing, a line over 4 MiB, a file that would pass 256 MiB, no id left), asked before the first write and again by the append itself.

### `scout events unmerge <log-id> --archive <path>`

Undoes the merge of one line of `merge-log.jsonl`. Facade: `EventEditor::unmerge(log_id)`.

**The rule: an undo clears `a.merged_into` and writes nothing else.** `b` is not written and none of its values is read. No other merge line is looked at.

- Any merge in effect can be undone at any time, where record `a`'s file is as scout writes it, the folder reads as meant without the merge and the log can take the line (`scout events merges` and `scout events` say beforehand, with `undoable: false` and `why_not`, when one of these would refuse it): whatever was merged into `b` or into `a` before or after it, whatever he changed on either record, whether `b` has itself since been merged away or removed, and in any order with other undos.
- A merge and its undo, with `a` not otherwise changed between, leave `a`'s file byte for byte as it was. `b`'s file is never written by either, so whatever he does to `b` in between is untouched.
- Records merged into `a` stay merged into `a` and leave `b`'s event with it.
- A line that was never applied (`a` is merged into nothing: a merge cut short after its log line, or an undo cut short before its own) is withdrawn: only the undo line is written, and the reply has `"withdrawn": true`.

```json
{"tool":"scout events unmerge","at":"2026-10-12T08:00:00.000Z","log_id":"m000002","undoes":"m000001","a":"2026-10-beta","b":"2026-09-alpha","withdrawn":false,"a_record":{"id":"2026-10-beta","…":"the record as it is now","merged_into":null}}
```

Refused, with nothing changed: no merge line of that id; already undone; record `a` is gone; `a` is merged into a record other than `b` (`merged_into` was changed by other means: `<id>: <a> is no longer merged into <b> (<a> is now merged into <c>); the merge cannot be undone`); `a`'s file not as the writer writes it; an undo after which the reader would give a warning it does not give now (a monthly id that `a` and another record of the event both list, which only a hand edit produces: apart, it could belong to one only; take it off one of them first); a log that cannot take the undo line.

### `merge-log.jsonl` and `merge-decisions.jsonl`

Both live in the records folder and are append-only. Only the edit commands write them, and they have one parser (`event_edit/log.rs`), which every reader goes through: the edit commands, `scout events merges`, and `scout events` for the `merge_id` of a merged-in record. The reader keeps each merge line's place by its id as it reads, so an undo line finds its merge line in one step and reading a log takes time in proportion to its length.

- **A merge line**: `{"id": "m000001", "at": "<UTC time>", "by": "dominik"|"model", "reason": "…"|null, "a": "<id>", "b": "<id>", "a_merged_into_before": null}`. Every one of the seven keys must be on the line: `reason` and `a_merged_into_before` may be null, not missing (`line 3: not a merge line (missing field `reason`)`). `a` is the record merged away, `b` the surviving record. `a_merged_into_before` is `a`'s `merged_into` before the merge, always null (a record already merged into another is not merged); a line with any other value there is malformed. The line holds no copy of either record and no list of what moved: the merge wrote `merged_into` on `a` and nothing else, and `unmerge` and `check` need only `a` and `b` from it.
- **An undo line**: `{"id": "m000002", "at": "<UTC time>", "undoes": "m000001"}`.
- **Ids** are canonical numbered ids, as in `corrections.jsonl`: `m` and the number (1 to 999,999,999) written with at least six digits and no more than it needs (`m000001` … `m999999`, `m1000000` … `m999999999`), so one spelling per number; one sequence for merge and undo lines, each greater than the line before. An id of any other form (`m1`, `m0000001` with a zero too many, `m000000`, `m1000000000` or longer) is a malformed line. When the last line has `m999999999`, `merge` and `unmerge` refuse (`the last line has the largest id there is`); the file is his to move away.
- **A decision line** (`merge-decisions.jsonl`): `{"at": "<UTC time>", "file": "<proposals file name>", "a": "<id>", "b": "<id>", "decision": "approve"|"dismiss"}`, `a` and `b` as the proposal line has them.
- **Limits.** A log is at most 256 MiB (268,435,456 bytes) and a line, with its newline, at most 4 MiB (4,194,304 bytes), on reading and on writing. A line over the limit is refused before anything is written (`the line to append is … bytes, over the limit of 4194304 for one line`).
- **Reading.** A log is read whole before the command writes anything. It must be a plain file (a symlink, dangling or not, a folder or a special file is refused: `not a plain file`) within the limits (`larger than 268435456 bytes`, `line 3: longer than 4194304 bytes`), UTF-8, every line a JSON object of exactly these keys, each of them present and nothing else, the last line ended by a newline. A log with a second hard link is read like any other (reading writes nowhere); a merge-log id must be well formed and greater than the one before, a merge line's `a_merged_into_before` must be null, and an undo line must name an earlier merge line not yet undone. Anything else stops the command before any write, naming the file and the line: `<file>: line 3: not JSON (…); fix or remove the line by hand; nothing was changed`. Every command that reads a log is stopped this way: `merge`, `unmerge`, `apply-proposals`, `merges`, and `check`, `proposals` and `decide` (these three read both logs). The one reader that is not stopped is the listing `scout events`, which reads the records folder and the merge log only for `merge_id` and `stale`, and lists without them when either cannot be read.
- **Appending.** A log is written in place, so more is asked of it than of a record. Before the write, with nothing changed on a refusal:
  - the line with its newline must be within the line limit, and the file with the line within the file limit, so no append leaves a log that is then refused on reading (`<file>: is N bytes, and a line of M more would take it over the limit of 268435456 bytes …`);
  - the file is opened for append without following a link, and the file opened (the descriptor, not the path) must be a regular file with exactly one hard link (`<file>: has 2 hard links; an append would also change the file under its other name …`: the other name may be outside the records folder), the file that was read, at the length it was read at (else `changed since it was read`);
  - a log that was not there is created, create-new (so the name is no link of either kind), with the default mode of a new file ("Modes of new files").

  Then the line and its newline go out in one `write` call (a short write is taken back by truncating to the old length, and is an error) and the file is synced. No line is ever rewritten or removed. `merge` appends its line as its first write. `unmerge` and `decide` append theirs last, so they make all of these checks before their first write, and the append makes them again. **One function makes these checks, for the append and for every answer to "would the append succeed?"** (`merge` before its first write, `unmerge`, `decide`, each merge `apply-proposals` plans with or without `--apply`, and the `undoable` of `scout correct list`): the append is that function followed by the write, and the question is the same function with no write, opening the file for append just the same (so a file that may not be written is refused by both: `open <file> to append (it must be a plain file, not a link, that may be written; nothing was changed): Permission denied`). They differ in one place: where no file is there, the append creates it, and the question only sees that the name is still free. Not defended: a link added to a log, or a write to it, in the instant between the check on the descriptor and the write.

### Order of writes: what a crash leaves

`merge` and `unmerge` each write one record and one log line.

- **`merge a b`**: (1) the log line; (2) record `a` (`merged_into: b`). Stopped after (1): no record has changed, and the line is in the log.
- **`unmerge`**: (1) record `a`, `merged_into` cleared; (2) the undo line. Before (1) the log is checked for whether it can take the line. Stopped after (1): the records are as before the merge, and the merge line has no undo line.
- **A merge line that is not undone is in one of two states**, told from record `a`: *in effect* (`a` is merged into `b`) or *not applied* (`a` is merged into nothing: either stop above). `scout events check` reports every line that is not applied, and `scout events unmerge <id>` withdraws it (it appends the undo line and writes no record), after which the merge can be made again. `merge` refuses an `a` that has a not-applied line and names the line. Both states are ones the reader reads without a warning.
- **A stop after the first write that is not a crash** (a write failed; record `a` was changed by hand or by another tool between the check before the log line and its own rename) leaves the same states. The error says so: `… the merge m000004 is in <log> and may not be applied; \`scout events check\` says whether it is, \`scout events unmerge m000004\` takes it back`, or for an undo `the undo of m000004 is not finished: <a> is unmerged and the undo line is not in the log; run \`scout events unmerge m000004\` again`. Record `a` is compared with the bytes read before the first write, so a change made before that point stops the command with nothing changed. A change to `b` at any point changes nothing for the merge: `b` is not written.
- `set`, `remove`, `restore` and `create` write one file each. `decide approve` checks that `merge-decisions.jsonl` can take its line, makes its merge (in the order above) and appends its decision line last; stopped between, the pair is merged and no longer waiting.
- A crash between creating a temporary file and renaming it leaves one `.scout-edit-*.tmp` file. The reader does not read it; `check` lists it; no command removes it.

### `scout events merges --archive <path> [--all] [--json]`

Lists the merges of `merge-log.jsonl`. Nothing is written or created. To say whether each merge can be undone it asks the same readiness question `unmerge` asks, which opens the existing log in append mode and writes no byte to it; the same holds for the `undoable` answer on an events listing. Exit 0 with one JSON line (`--json` is accepted and changes nothing); 2 when the folder or the log cannot be read, as for `check`. Facade: `EventEditor::merges()` (the `merges` list below), `EventEditor::merges_all()` (the same with `--all`), `EventEditor::merge_list(all)` (the whole line).

```json
{"tool":"scout events merges","at":"2026-10-12T08:00:00.000Z","dir":"/abs/archive/analysis/events","merges":[{"id":"m000003","at":"2026-10-12T07:59:00.000Z","by":"dominik","reason":"one story","a":{"id":"2026-10-beta","name":"Beta"},"b":{"id":"2026-09-alpha","name":"Alpha story"}}]}
```

- `merges`: every merge line in effect, newest first (by line id, descending). A line is in effect when no undo line undoes it and record `a` is merged into its `b`: the `in_effect` of `scout events check`, by the same function, so the two always agree in number. `id` is the line's id, which `scout events unmerge` takes; `at`, `by` and `reason` are the line's; `a` is the record merged away and `b` the surviving record, each `{id, name}` with the id as the line has it and the record's name as it is now.
- `undoable`, `why_not`: a merge in effect that `scout events unmerge <id>` would refuse now has `"undoable": false` and `"why_not"`, the refusal word for word; a merge it would accept has neither key, and nor has an undone line (which has `undone_by`). It is asked of the checks `unmerge` itself makes before its first write (one function; see `undoable` in "`scout events`"), against the one read of the folder and the log this command makes. So a row without `undoable: false` is one `unmerge` accepts now.
- A line that is neither undone nor in effect (`not-applied` or `not-as-logged` in `check`) is no merge and is not listed, with or without `--all`. A record merged by hand has no line and is not listed (`check` names it under `information`).
- `--all` also lists the undone lines, in the same order, each with `undone_by` (the id of its undo line) and `undone_at` (that line's time); the reply then carries `"all": true` after `dir`. A side of an undone line whose record is gone has `"name": null`. Without `--all` none of the three keys is present.
- It uses the log's one reader and the records' one reader, like every edit command: a malformed log line, or a record the reader skips, stops it (exit 2).

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
- **Which merges.** Every pair with `verdict: "same"` and `confidence: "confident"`, in file order, as a `--by model` merge with the line's `reason`. A pair is taken as the two events its ids are in by then, so with A~B and B~C all three end in one event. Of the two events, the one that starts later is merged into the one that starts earlier, by the `start` of each event's own surviving record, not the span derived for it (a record without `start` counts as latest); on the same start the one with more items survives (an event's count is that of every record of it the file names, added up: for each id the largest `a_items` / `b_items` the file gives it, kept under the record that holds that id when the command starts, so a record already merged into another, by an earlier run over the file or by any other merge, counts towards its holder; a survivor's count then grows by what is merged into it, and the sum stops at the largest number it can hold rather than wrapping. A run cut short and taken up again therefore picks the survivors one run would); then the id that sorts first. Since the survivor of every merge is the earlier one, the surviving record of an event is the one with the earliest own `start` of all its records, and a chain ends in its earliest event whatever the order of its lines.
- **Skipped**, each listed in `skipped` with `why` and counted in `skipped_counts`: `already-one-event` (the two are in one event already, which is what makes a second run a no-op), `no-record` (an id names no record), `model-rule` (the record that would be merged away has `edited` set; `edited` on the survivor skips nothing; the pair then waits for him, see `proposals`), `refused` (any other refusal of `merge` that can be told from what was read: a removed record, a record not as the writer writes it, a reason with a control character, a not-applied merge line for that record, a merge after which the reader would give a warning it does not give now or resolve an id otherwise, no log id left, a log line over 4 MiB, a log that the merges before it and this line would take past 256 MiB). The run without `--apply` and the run with it work out the same merges and the same skipped pairs with the same code: what the dry run plans is what `--apply` makes, and a pair whose line the log could not take (its id, its length) is skipped by both, not a stop of the run.
- **Refused as a whole**, with and without `--apply`, in the same words, exit 2, nothing on stdout and nothing changed: at least one merge is planned and `merge-log.jsonl` cannot be appended to at all, whatever the line: not a plain file, a second hard link, changed since it was read, not open to writing ("Appending"). This is not a skipped pair. It is said once, for the run: `<log>: has 2 hard links; an append would also change the file under its other name, which may be outside its folder, so it is not written; nothing was changed`, then `<file>: 3 merges are planned and none was made: the merge log cannot be appended to`. The dry run asks the log the question `merge` asks before its first write ("Appending"), so it does not print a plan that `--apply` would then stop at. A run that plans no merge appends nothing and is not refused for its log. `proposals` reads only and is not refused for it either.
- **With `--apply`** each merge is made by `merge` itself, one at a time, each whole with its own log line and every check. The plan is worked out on a copy of the records in memory with the function `merge` uses, the reader's own resolution deciding after each pair which event an id is in. If a merge still stops with an error (what only the moment of the write can tell: a record or the log changed, or the log given a second name, after the plan was made; a write that fails), the run stops there: `… <file>: line 2: the merge of <a> into <b> stopped the run; 1 of 3 merges were made before it and stay (each is whole and in the log); run again for the rest`.

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

## Correcting an item's tags

`scout correct theme-add | theme-remove | entity-add | entity-remove | event-add | event-remove | undo | list`: his fixes to one item's tags. Each works on one archive, given with `--archive <path>` (required). Each of the first seven appends ONE line to `<archive>/analysis/topics/corrections.jsonl`; `list` writes nothing. This is the file's only writer. The model's rows (`current.jsonl`) are never written. No registry and no index are read or written: the index has the old tags until `scout index build` runs, and every result says so (`"reindex_needed": true`; nothing here starts a build). Facade: `scout_corpus::corrections::CorrectionEditor::open(&archive)?` and `add_theme(key, theme)`, `remove_theme(key, theme)`, `add_entity(key, name, type)`, `remove_entity(key, name)`, `add_to_event(key, event_id)`, `remove_from_event(key, event_id)`, `undo(id)`, `list(key)`, each returning the struct the CLI prints. Each command prints one JSON line on stdout and exits 0; a refusal exits 2 with the reason on stderr, nothing on stdout and nothing written.

```
scout correct theme-add    <theme>  --key <key> --archive <path>
scout correct theme-remove <theme>  --key <key> --archive <path>
scout correct entity-add   <name>   --type person|org|model|product|work|event|place|other --key <key> --archive <path>
scout correct entity-remove <name>  --key <key> --archive <path>
scout correct event-add    <event>  --key <key> --archive <path>
scout correct event-remove <event>  --key <key> --archive <path>
scout correct undo <id> --archive <path>
scout correct list --key <key> --archive <path> [--json]
```

**The lines written.** One of these per command, keys in this order, one name per line:

```json
{"id":"c000001","at":"2026-10-12T08:00:00.000Z","date":"2026-10-12","by":"dominik","key":"readwise:rw_book_1","theme_add":["AI and mathematics"]}
{"id":"c000002","at":"…","date":"…","by":"dominik","key":"readwise:rw_book_1","theme_remove":["Vibe coding and AI-assisted programming"]}
{"id":"c000003","at":"…","date":"…","by":"dominik","key":"x:19654","entity_add":["GPT-6 Astra"],"entity_type":"model"}
{"id":"c000004","at":"…","date":"…","by":"dominik","key":"x:19654","entity_remove":["Terence Tao"]}
{"id":"c000005","at":"…","date":"…","by":"dominik","key":"x:19654","event_add":["2026-09-gemini-launch"]}
{"id":"c000006","at":"…","date":"…","by":"dominik","key":"x:19654","event_remove":["2026-09-fields-declaration"]}
{"id":"c000007","at":"…","date":"…","by":"dominik","key":"x:19654","undoes":"c000005"}
```

`id` is a canonical numbered id (`c` and the number, 1 to 999,999,999, written with at least six digits and no more than it needs: `c000001`, `c1000000`; an id of any other spelling, `c0000001` included, is not a numbered id and `undo` does not take it), one sequence for corrections and undo lines, each greater than the numbered line before (`c999999999` is the last; then every command refuses and the file is his to move away). `at` is the UTC time, `date` its first ten characters. How the reader reads these keys is in "Topic layer" ("Corrections", "Numbered corrections"). `entity_type` is kept on the line and the reader gives it to the name as its type (engine `scout-corpus/13`; "Topic layer", "Entity types"): the item's `entities` stay plain names, and `entity_types` beside them maps the name under the fold to the type, in `tags` here and in a search result alike.

**Why patch lines.** The older keys `themes` and `entities` replace the model's list whole. The lines written here patch it instead, as `event_add` / `event_remove` always did, so a correction says one thing (this theme on, this entity off) and nothing about the rest of the item's tags: a later re-tag of the item keeps his correction and is not overridden by a list computed on the day he made it.

**Undo.** `undo <id>` appends one line, `{…, "key": <the correction's key>, "undoes": "<id>"}`. The reader then does not apply the correction named, as if its line were not in the file. No value is restored from anywhere and none is kept on the line, so the undo is exact whatever the model's row is by then; once every correction of an item is undone its tags are the model's and its `corrected` is empty.

- Undo is last change first, per item and per value. A correction is not undone while a later line in effect for the same item touches the same theme, the same entity or the same event (ids compared as the reader resolves them, so a monthly id and the record that holds it are one event): `c000001: c000002 (theme_remove "AI and mathematics") later changed the same theme of x:4; undo is last change first, so undo that one before this`. A line in effect is one the reader applies: not an undo line, and not a correction already undone. A later hand-written line with a whole `themes`, `entities` or `event` touches every theme, entity or event of its item (`line 17 (a hand-written line) later changed …`).
- Corrections of other values, other fields and other items do not stand in the way.
- Refused as well: no correction of that id; one already undone (`c000001: already undone, by c000007`); an undo line (`an undo is not undone, make the correction again`); a line without a numbered id.
- Undo, like every command here, is refused when the correction's key has no tag row any more (the item left `current.jsonl` after the correction was made): `x:4: no tag row in analysis/topics/current.jsonl; only an item the layer holds is corrected (nothing was changed)`, before any write. The correction's line stays in the file, and the reader goes on doing with it what it does with every correction for a key without a tag row ("Topic layer"): it is counted in `corrections_unmatched` and applied to nothing. If the item's row comes back, the correction applies again and can be undone.
- Refused too, as for every command: no id left (`the last correction has the largest id there is (c999999999)`), and a file that cannot take the line ("The file").

**What each command checks before it writes.**

- **The key** must be an item with a tag row in `current.jsonl` (`x:404: no tag row in analysis/topics/current.jsonl`). It is trimmed; a control character refuses it.
- **A theme to add** must be a name of the archive's theme list, `<archive>/analysis/context/themes.json` (`themes[].name`, the tagger's list; this command is its only reader in scout, and the index never reads it). Names compare under the fold of "Query grammar"; the line carries the list's spelling. Refused: no list, a list that is not `{"themes": [{"name": "…"}, …]}`, a name not on it, a theme the item has.
- **A theme to remove** must be one the item has now (model's or corrected), on the list or not; the line carries the item's spelling.
- **An entity to add** is free text: trimmed, not empty, no control character (as for an event's name), at most 200 characters, with `--type` one of `person`, `org`, `model`, `product`, `work`, `event`, `place`, `other` (the types of the layer's `entities_typed`). Refused: one the item has under the fold. **An entity to remove** must be one the item has; the line carries the item's spelling.
- **An event** may be given by lasting id, merged-away id or monthly id. It must resolve to a record that is not removed (`2031-01/E09: no event record holds this id`; `<id>: a removed event; restore it first`), and the line carries that record's id, the lasting event the id resolves to at that moment. `event-add` refuses an event the item is in; `event-remove` one it is not in. `event-remove` takes the item out whoever put it there, its tag row included (`tags.event` is then null).
- **Without a records folder** (`<archive>/analysis/events`) `event-add` and `event-remove` refuse: `…/analysis/events: no records folder; without one the reader reads no event_add or event_remove, so the item's events are not corrected`. The theme and entity commands work there: the reader reads their keys with or without the folder.
- **The file as it is.** It is read whole first (below). The command stops, naming the file and the line, when the reader would not read a line of it (`line 3: the reader does not read it (no text `key`, or a field of the wrong type)`), when a line is blank, not JSON or not ended by a newline, or when a numbered line is not one this command writes: a key other than those above (`line 2: a numbered line with the key "note", which `scout correct` does not write`), more than one change or more than one name on a line, `at`, `date`, `by` or `key` missing, an `entity_type` that is no type or stands without `entity_add`, an id not greater than the numbered line before, an `undoes` that names no earlier correction of the same key or one already undone. Whether a numbered line is one this command writes is decided by the function the reader asks ("Topic layer", "Numbered corrections"), so the two agree line for line: a build skips exactly the numbered lines named here, applies none of them and counts each in `corrections_bad_lines`; this command does not append after one, and gives the line and the reason. (A blank line and a last line without its newline are rules of this command for the file alone: a build passes over the first without counting it and reads the second.) Lines without a numbered id (written by hand, in the older shape) are left as they are and may stand anywhere in the file.
- **The reader's answer.** The tag rows and the event records are read by the reader's own code, which is asked what it makes of the file as it is and of the file with the new line. The second must be the first with exactly the change meant on that item, `corrected` naming the field, every other item unchanged; for an undo it must equal what the reader makes of the file without the line taken back. Otherwise: `the reader would not read the correction as meant`.

**The file.** The handling is the merge log's (`### merge-log.jsonl and merge-decisions.jsonl`, "Reading" and "Appending"), with its own limits: the file at most 64 MiB (67,108,864 bytes; the same constant is the size over which a build does not read the file, "Reading limits") and a line with its newline at most 1 MiB (1,048,576 bytes, the reader's line limit, so no line is written that the reader would skip). It must be a plain file (a symlink, dangling or not, a folder or a special file is refused: `not a plain file`), UTF-8, every line a JSON object, the last line ended by a newline. It is appended to only when the file opened (without following a link) is a regular file with exactly one hard link (`has 2 hard links; an append would also change the file under its other name …`), is the file that was read, at the length read (`changed since it was read`), and stays within both limits with the line. These are the checks of the merge log's one function ("Appending"), which the append runs and `list` runs without the write for each line's `undoable`. The line and its newline go out in one `write` call and the file is synced. A file that is not there is created, create-new, with the mode the umask leaves of 0666 ("Modes of new files"). No line is ever rewritten or removed. `list` reads the file the same way and writes nothing.

**Paths.** An `--archive` path with a `..` component is refused; `<archive>/analysis/topics` must exist and be a real folder, not a symlink (symlinks above it are accepted). Not defended: `current.jsonl` or a record changing between the command's read of it and its append; a link added to the corrections file in the instant between the check on its descriptor and the write.

**The result** of the six corrections and of `undo`:

```json
{"tool":"scout correct theme-add","at":"2026-10-12T08:00:00.000Z","id":"c000001","key":"x:4","op":"theme_add","value":"AI and mathematics","file":"/abs/archive/analysis/topics/corrections.jsonl","reindex_needed":true,"tags":{"key":"x:4","themes":["AI and mathematics"],"entities":[],"event":null,"event_name":null,"events":[],"ai_related":true,"month":"2026-09","confidence":null,"run_id":null,"prompt_version":null,"corrected":["themes"]}}
```

`op` is the line's key (`theme_add` …) or `undo`; `value` the name or event id written (for an undo: of the correction taken back); `entity_type` is present for `entity-add` (and for the undo of one); `undoes` is present for `undo`. `tags` is the item as the reader has it with the new line: the `topics` object a search result will show after the next `index build` (tested: an index build of the archive after each command gives a search hit exactly this object).

**`scout correct list --key <key>`**:

```json
{"tool":"scout correct list","key":"x:2","file":"/abs/archive/analysis/topics/corrections.jsonl","model":{…},"effective":{…},"corrections":[
  {"id":"c000004","line":4,"at":"…","date":"2026-10-12","by":"dominik","op":"undo","value":null,"undoes":"c000001","undone_by":null,"undoable":false,"why_not":"c000004 is the undo of c000001; an undo is not undone, make the correction again (nothing was changed)"},
  {"id":"c000002","line":2,"at":"…","date":"2026-10-12","by":"dominik","op":"entity_add","value":"Astra","entity_type":"model","undoes":null,"undone_by":null,"undoable":true,"why_not":null},
  {"id":"c000001","line":1,"at":"…","date":"2026-10-12","by":"dominik","op":"theme_add","value":"Metaphor and cognition","undoes":null,"undone_by":"c000004","undoable":false,"why_not":"c000001: already undone, by c000004 (nothing was changed)"}]}
```

`model` is the item's `topics` object with no correction applied (events resolved through the records as always); `effective` the object with the file as it is. `corrections` are the file's lines for this key, newest first: `undoable` says whether `undo <id>` would be accepted now, `why_not` the refusal it would give, word for word. Both come from running the undo itself up to its append, by the one function `undo` runs: the order of changes, the item's tag row, an id for the undo line, the reader's answer, and whether the file can take the line ("The file": its type, its link count, its limits, whether it may be written). So a correction is `undoable` exactly when its undo would reach the append, and when the file cannot be appended to (say, a second hard link) every line is `"undoable": false` with that reason. What `list` cannot see is what only the moment of the write can tell (the file changed after `list` read it; a write that fails). A line without a numbered id has `"id": null`, `"op": "hand-written"`, `"fields"` (the fields of it the reader reads) and is never undoable. The file is checked as for a write, so a file the writer would not append to is an error here too. Refused: a key without a tag row.

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

`corrections_undone` (after `corrections_bad_lines`) is present only when not 0: the numbered correction lines an undo line took back. `corrections_bad_lines` counts every non-blank line of the corrections file that was not read: not a JSON object, no text `key`, a wrongly typed field, over the line limit, not UTF-8, or a numbered line that is not valid ("Numbered corrections").

When the corrections file was not read at all (over 64 MiB, "Reading limits"), two more keys follow `corrections_undone`, and the four `corrections…` counts are 0 because no correction was applied:

```json
"corrections_skipped": {"reason": "the corrections file is 67108865 bytes, larger than the limit of 67108864 bytes; it was not read and NO correction was applied", "bytes": 67108865, "limit": 67108864}, "warnings": 1
```

Both are absent otherwise. `warnings` counts what about the layer files needs attention; a corrections file not read is the one thing it counts. A build that keeps the stored copy (`reloaded: false`) repeats them. The human output then adds a line of its own under the `topics:` line: `WARNING: corrections NOT applied: the corrections file is 67108865 bytes, larger than …`.

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

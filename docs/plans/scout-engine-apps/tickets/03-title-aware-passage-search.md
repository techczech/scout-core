# 03: Title- and topic-aware passage search

**What to build:** A search where some query terms match only the document's title, topics or summary (not the passage) still returns that document's best passage for the remaining terms, ranked by combined evidence.
- Example: `paths metaphor` must return the 2016 essay "Repaved paths and generative metaphors…". Its best passage contains "paths" but not "metaphor", and "metaphor" is in the title. Today its top hit is a tweet announcing the essay.
- Rule: a document matches when every term appears in (the passage ∪ the title ∪ the topics ∪ the summary). The shown passage is the one covering the most terms, with ties broken by passage score.
- Ranking: document fields weigh less than passage text, so a passage with every term still beats one relying on the title.
- Invariants 2 (quotes are original text) and 6 (deterministic output) hold.

**Feature / journey:** `quick-finder-search`, `scout-search-panel` (J1).
**Design:** engine spec `docs/specs/2026-09-27-corpus-engine-and-cli.md` (search section).
**Blocked by:** None: dispatchable now.
**Seams under test:** the facade `search()`; the CLI parity test.
**Status:** landed

- [ ] `scout search "paths metaphor" --in writing` returns the 2016 essay first, quoting a passage with "paths".
- [ ] A query whose terms all occur in one passage ranks that passage above title-only matches (test).
- [ ] The T1 done-when query ("generative metaphor") still returns the essay first; the timings stay within the spec.

**Also in this ticket (found by the cleaning bench, 2026-09-28):** inline Markdown or HTML emphasis inside a word splits it into tokens. `He <u>sw**a**m</u>` counts as "he sw a m" (585 lines in the writing corpus). Stripping emphasis markers and inline tags must not create word boundaries where there was none; `swam` is one token. Test it, and report the clean-report before/after.

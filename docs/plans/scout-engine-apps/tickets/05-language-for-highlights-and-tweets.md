# 05: A language for every document (highlights and tweets included)

**What to build:** Highlights documents (15,010) and tweets have no `lang`. A slice with `lang:en` therefore silently drops every highlight, and the ArchiveScout Czech/English nudge ignores them. At index time, give every document without a `lang` field an inferred language (`en`, `cs` or `und`), using a small deterministic detector: Czech-specific diacritics plus a Czech/English stopword ratio over the document's normalised tokens, with `und` below a confidence threshold. Store `lang` with `lang_source: inferred | frontmatter`. The frontmatter always wins.

**Feature / journey:** `lab-compare-slices`, J5/J6 (writing against highlights comparisons).
**Blocked by:** None.
**Seams under test:** the detector (golden cases: English, Czech, mixed, short); the facade `index status` or dist `--by lang` showing highlights split into en/cs/und.
**Status:** ready

- [ ] `scout dist metaphor --in highlights --by lang` shows en and cs buckets (not "unknown").
- [ ] A frontmatter `lang` is never overridden (test).
- [ ] The index version is bumped if the schema changes; say so, because the apps must re-pin.

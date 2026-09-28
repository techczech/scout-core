# 04: Multi-corpus passage identity (quotes must never come from the wrong corpus)

**What to build:** Fix a violation of invariant 2 (quotes are original text), found by the ArchiveScout builder on 2026-09-28.
- In `concord.rs`, the KWIC line renderer caches "the last passage" by row id alone, not by (corpus, row id).
- When consecutive hits come from different corpora with the same row id, it renders the wrong passage's text. It either panics (index out of bounds at `concord.rs:324` in a multi-corpus n-gram→concordance test) or silently shows a wrong quote.

Required:
- The fix: key every per-passage cache and lookup by (corpus, row id), or by the stable passage id.
- **Audit every multi-corpus path for the same bug class:** search/search_all, kwic, collocates `--near`, dist, ngrams `--dist doc`, profile top documents, similar, cite and verify-quote. Report each one checked.
- Add an app-facing `containing` option to the n-gram request (the library already has it internally), so the apps can list only the phrases with a word.

**Feature / journey:** J4/J5/J6, every multi-corpus view in both apps.
**Blocked by:** None: dispatchable now (urgent).
**Seams under test:** the facade functions, with a two-corpus fixture DESIGNED so that row ids collide across corpora.
**Status:** ready

- [ ] A two-corpus fixture with colliding row ids: every KWIC, search, cite and similar result's quote is a substring of its OWN document's original text (test over all rows).
- [ ] No panic in the n-gram→concordance multi-corpus path.
- [ ] The CLI parity test stays green, and the index version is unchanged unless the schema must change (say so).

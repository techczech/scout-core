# 02: Similar passages ("more like these")

**What to build:** `scout similar <passage-id>… [--in …] [--top 20] [--exclude-seeds] [--json]` and the facade call.
- Given 1–10 seed passages, rank other passages by weighted term overlap: tf-idf over passage tokens with stopwords removed, cosine similarity against the centroid of the seeds.
- Each suggestion carries the top 3 shared terms (the "why") and which seed it is closest to.
- Deterministic, with ties broken by passage id.

**Feature / journey:** J2-C, the agent/engine suggests more like the set's seeds; he approves each.
**Design:** `~/gitrepos/06_apps-utilities/01_desktop-apps/highlight-scout/docs/design/2026-09-27-archive-search-and-corpus-tools/round-1/hs-2-sets.html` (HS-2A, the suggestions state).
**Blocked by:** 01 (the facade).
**Seams under test:** the facade `similar()`; the CLI.
**Status:** landed

**Context (cold read):**
- The Scout corpus engine lives in ~/gitrepos/06_apps-utilities/03_misc-utilities/scout-core (spec: docs/specs/2026-09-27-corpus-engine-and-cli.md; JSON shapes: docs/cli-json.md). It indexes three corpora: `writing` (Dominik's writing, 1,742 pieces), `tweets` (14,892, one doc per tweet) and `highlights` (the Highlight Scout archive, 14,914 works).
- The registry is at `~/.config/scout/corpora.toml`; the indexes are in `~/Library/Application Support/scout/indexes/`. The CLI is `scout`.
- **Invariants:**
  - sources are read-only;
  - quotes are original text;
  - counts agree across views;
  - the app and CLI call the same library functions (no logic duplicated in apps).
- Journeys and picks: `~/gitrepos/06_apps-utilities/01_desktop-apps/highlight-scout/docs/design/2026-09-27-archive-search-and-corpus-tools/journeys.md` (see "Dominik's picks").

- [ ] Seeding with the 2016 "Repaved paths and generative metaphors" passage returns other metaphor passages first, each with its shared terms.
- [ ] Seeds are excluded with `--exclude-seeds`; the output is byte-identical across runs.
- [ ] Across writing, tweets and highlights: < 1 s on a warm index, measured and reported.

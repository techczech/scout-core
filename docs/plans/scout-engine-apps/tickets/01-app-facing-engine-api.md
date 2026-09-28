# 01: App-facing engine API and cleaning report

**What to build:** A stable library facade in `scout-corpus`, so the Tauri apps (Highlight Scout, ArchiveScout) call the engine in-process with exactly the CLI's behaviour and JSON:
- one request/response API per command (search, kwic, collocates, ngrams, profile, dist, keyness, verify-quote, cite, index status/build);
- request and response types that serialise to the documented cli-json shapes;
- the CLI refactored to call ONLY this facade.

Add a **cleaning report** (`scout clean-report <corpus> [--samples 5] [--json]`): per normalisation rule (entities, apostrophes, tags/links/URLs, boilerplate regexes), how many passages it changed, plus N before/after samples as original line → counted tokens. ArchiveScout's corpus home (AS-3B cleaning bench) renders it.

Then tag scout-core `v0.2.0`, so the apps can pin it as a git dependency.

**Feature / journey:** groundwork for the J4/J5/J6 lab views and the J1 search in both apps; J7-C corpus home.
**Design:** `~/gitrepos/06_apps-utilities/01_desktop-apps/archive-scout/docs/design/round-3-corpus-lab/as-3b-corpus-home-cleaning-bench.html`; engine spec above.
**Blocked by:** None: dispatchable now.
**Seams under test:** the facade functions (request → response JSON), and the CLI binary's output equal to the facade's for the same request.
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

- [ ] Every CLI command goes through the facade; no command logic remains in `scout-cli`.
- [ ] A test asserts that the CLI `--json` output equals the facade's serialised response for each command (table-driven).
- [ ] `clean-report writing` shows real counts for the entity, apostrophe and boilerplate rules, with samples (the "amp amp amp" case, a curly apostrophe case).
- [ ] The facade is safe to call from a background thread; the index is opened per call or pooled, with no global mutable state.
- [ ] `cargo test --workspace` is green; cli-json.md is updated; the tag `v0.2.0` is created by the driver after landing.

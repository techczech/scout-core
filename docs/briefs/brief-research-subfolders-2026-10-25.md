# Brief — scout: research/ folders with subfolders

You are the builder on this parcel. Build exactly what the spec or mockup fixes; do not re-decide architecture, scope or model assignment. The parcel is complete when every listed behaviour works end to end and the repository's existing checks pass; continue until then rather than returning after a first implementation. Shape new code as deep modules — substantial behaviour behind a small interface at a clean seam — and write tests at the seams the parcel names, sized like the neighbouring test files. If a named seam is missing, first look for the nearest existing seam that serves it; if none does, build the smallest seam the behaviour needs and flag it in your report; stop only if creating it would change a public interface the spec fixes. If you meet a pre-existing bug or behaviour the task does not mention, leave it and report it as a follow-up. Do not commit and do not touch `.git`; produce files, the driver commits. Do not spawn or delegate to other agents. While iterating, run only the tests for the code you changed (the named test files, or the runner's changed/related mode); run the repository's full unit suite once, at the end, and do not re-run a suite that passed unless code changed since. Run only the end-to-end specs for the feature you touched, plus any gate-set spec guarding a path your diff touches. Run long suites in the background and wait for them rather than polling in short loops. Report what you built, what you did not, any assumption you made under ambiguity, any spec conflict verbatim, and one line per real decision with its reason.

## Where
- Worktree: `/Users/dominiklukes/gitrepos/06_apps-utilities/03_misc-utilities/.wt-scout-research-subfolders` (branch `feat/research-subfolders`, from main). Read `AGENTS.md` and `README.md`. Rust workspace (`crates/`) plus `packages/`. Files only inside this worktree. Use a target dir inside the worktree.
- Evidence: the UnityBC clean-up log, finding N12: `/Users/dominiklukes/gitrepos/02_writing-creation/writing/academic-writing-research/papers/unitybc-paper/research/cleanup-agent-notes-2026-10-25.md` (read only). Example of the wanted layout: `/Users/dominiklukes/gitrepos/02_writing-creation/writing/academic-writing-research/papers/unitybc-paper/research/` (read only: do not write there), with subfolders such as `literature/` and `llm-judge-pilot-report/`.

## Owner's request (2026-10-25)
Research captures and where-used must work inside `research/` subfolders.

## Facts (verify)
- `scout capture` writes flat `research/<name>.md` with `research/_manifest.jsonl` beside it (`crates/scout-corpus/src/capture.rs:350,364`).
- `receipts` reads `research/_manifest.jsonl` (`:615`).
- `where_used` (`:628-658`) walks MarkdownFolder corpora, skips dot-directories and every directory named `research`, and matches on the basename substring only.
- CLI: `crates/scout-cli/src/capture_cmd.rs:39,61`.

## Build
1. **Capture into a subfolder:** `scout capture --into <subpath>` (or the CLI's existing naming style) writes `research/<subpath>/<name>.md`.
   - One `_manifest.jsonl` stays at the `research/` root, with each entry recording its path relative to `research/`.
   - Old flat entries stay valid; no migration is required.
   - Reject `..`, absolute paths and symlink escapes.
2. Receipts and listing read captures at any depth under `research/`.
3. **where-used:**
   - Finds usages of a research file wherever it lives (any depth).
   - Matches on the path relative to the referencing file, or the research-relative path, not the bare basename, so two files with the same name in different subfolders are told apart.
   - Usages inside `research/` itself still do not count, as now.
4. If a research file is moved within `research/`, `_manifest.jsonl` can be updated:
   - provide `scout capture --moved <old> <new>`, or a re-scan that repairs the paths;
   - choose one and justify it;
   - it is explicit, never on read.
5. Update the docs or README section about research folders.

## Seams under test
- Rust unit and integration tests in the crate's existing style:
  - capture into a subfolder;
  - receipts at depth;
  - where-used with two same-named files in different subfolders;
  - path-escape refusals;
  - old flat manifests;
  - the move route.
- `cargo test` for the workspace once at the end, plus any `packages/` tests that touch capture.

## Report
Files changed, the CLI surface, tests with counts, follow-ups. Do not commit.

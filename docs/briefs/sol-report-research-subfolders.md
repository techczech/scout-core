No action needed. Research subfolders are implemented and feature tests pass; repository-wide checks have unrelated failures.

Implemented:

- `scout capture --project <folder> --into <subpath> ...` writes nested captures and copied external artefacts together. One root manifest records research-relative paths, and flat receipts remain valid.
- Listing reads both flat and nested receipts without modifying them.
- Where-used matches complete research-relative paths or paths relative to the referencing Markdown file. Tests distinguish same-named files in different subfolders, exclude research self-usages, and cover Markdown destinations containing spaces and percent encoding.
- Capture destinations reject parent traversal, absolute paths and symlink components.
- `scout capture --project <folder> --moved <old> <new> [--json]` explicitly updates the receipt after a manual move. Both paths are research-relative. It requires the old file to be absent, the new file to exist, exactly one old receipt and no conflicting new receipt. The manifest rewrite uses the existing root lock and atomic writer; symlinked and hard-linked manifests are refused.

Files changed:

- `crates/scout-corpus/src/capture.rs`: destination handling, relative receipts, path matching and explicit receipt repair.
- `crates/scout-corpus/src/api.rs`: added `Engine::capture_moved(project, old, new)` beside the existing capture methods.
- `crates/scout-cli/src/capture_cmd.rs`: added `--into` and `--moved`, with argument conflicts and existing output conventions.
- `crates/scout-corpus/tests/capture.rs`: six new regression tests and updated one legacy basename-only fixture to use a path.
- `crates/scout-cli/tests/capture.rs`: one new end-to-end nested-capture and move test.
- `README.md` and `docs/cli-json.md`: documented nested folders, lookup and move semantics.
- `docs/briefs/sol-report-research-subfolders.md`: this report.

Validation:

- Confirmed the new nested-capture, escape-refusal, same-name lookup, CLI and space-containing-link tests failed before their corresponding implementation.
- Focused corpus capture suite: 23 passed, exit 0.
- Focused CLI capture suite: 5 passed; facade parity suite: 4 passed, exit 0.
- Rust formatting check for all five changed Rust files: passed.
- Ran `cargo test --workspace` once, with `CARGO_TARGET_DIR` and `TMPDIR` inside this worktree. It stopped with 376 passed, 2 failed and 3 ignored; later suites did not run. Both failures are in unchanged `layer_files_regular_only.rs:75`: socket-fixture creation fails with `path must be shorter than SUN_LEN`. Its supposedly short temporary directory inherits the required long worktree-local TMPDIR.
- Pinned `bun install --frozen-lockfile`, including the TypeScript build: passed. Ran `bun run test` once: all 236 package tests passed across three files, but the command exited 1 because Vitest also collected 15 unchanged `tools/topic-tagger/test/*.test.js` files that use `node:test`, reporting `No test suite found`. No package code touches capture.

Decisions and assumptions:

- Chose explicit `--moved` receipt repair after a manual move because it preserves read-only listing and avoids guessing file identity during a rescan.
- Added the smallest missing facade seam, `Engine::capture_moved`, to keep mutation logic out of CLI parsing.
- Refused every symlink component, including internal aliases, to make destination containment unambiguous.
- Interpreted listing as reading the single root manifest at arbitrary recorded depths; it does not invent receipts for unmanifested files.
- Kept move repair limited to receipt paths. It does not move files, rewrite links or provenance, or rebuild indexes.

Spec conflicts: none.

Follow-ups: the driver should handle the existing Unix-socket fixture path limit and the root Vitest/Node-runner collection mismatch separately. Neither was changed under this parcel.

All work and build files stayed inside the specified worktree. The writing repository was read only. Scratch logs and temporary test directories were deleted. No commits or `.git` operations were performed. The implementation is ready for the driver's review; repository-wide validation is not green.

## Fix round r1

No action needed. The research-subfolder review fixes are implemented, and all Rust workspace tests pass.

- F-1: Where-used recognises sentence punctuation, bold markup and parenthesised research-relative paths. Restored the legacy `Reuse {file}` fixture alongside its path form.
- F-2: Bare prose names require exactly one matching research basename in the project. Markdown destinations resolve relative to their referencing file; links to other files cannot fall back to basename or suffix matching. Missing targets cannot match another project's research suffix. Absolute requests derive their research-relative identity from the enclosing research root rather than the first ancestor named research.
- F-3: Existing destination components use their on-disk spelling. A completed case-only rename can update its receipt even when OLD still resolves on a case-insensitive filesystem. The local regression exercised that filesystem behaviour and case-insensitive link resolution.
- F-4: Move destinations refuse the manifest, hidden components, absolute paths and parent traversal; existing symlink refusals remain covered.
- F-5: Destination validation rejects hidden, dot and empty components before creating subfolders. Creation follows collision checks and Markdown rendering; failed writes remove newly created empty subfolders.
- S-1/S-2: Replaced the overlapping splitters with one tokenizer and one delimiter constant, with identity matching in `ResearchIdentity`. CLI argument extraction and receipt serialisation now return errors instead of using panicking unwraps or positional indexing.

Changed in this round: `crates/scout-corpus/src/capture.rs`, `crates/scout-corpus/tests/capture.rs`, `crates/scout-cli/src/capture_cmd.rs`, and this report. Added seven regression tests covering F-1 through F-5; retained and restored legacy coverage. The lookup/glob fixture now includes the research file whose basename it mentions.

Validation: the seven new regressions failed before the fixes. Focused corpus capture suite: 30 passed. CLI capture suite: 5 passed. Facade parity suite: 4 passed. All exited 0. Formatting checks passed for the three changed Rust files. Ran `cargo test --workspace` exactly once after the fixes: exit 0, 499 passed, 0 failed, 3 ignored across 59 test-result summaries, including doc tests. Every test command used `TMPDIR=/tmp` and the worktree-local `target/` directory. No package code changed; TypeScript tests were not rerun.

Decision: distinguish prose shorthand from Markdown destinations so legacy prose remains discoverable without treating a link to another file as a usage. Decision: normalise existing paths through the filesystem so receipts retain disk spelling and case-only moves remain explicit.

No commits or `.git` operations. Removed the scratch test log; temporary test fixtures use automatic cleanup. No changes outside the specified worktree, apart from the authorised short-path temporary test fixtures. Independent re-review remains with the parent session.

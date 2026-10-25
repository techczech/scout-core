# Review r1: scout-core PR 5, research subfolders (Sonnet 5.5, reviewer)

Verdict: NOT CLEAN on spec/behaviour (2 MAJOR, several MINOR). No path-escape or data-loss hole found; write path is safe.

## Spec axis

F-1 MAJOR. where-used tokenizer misses ordinary prose mentions. `mentions_research` (crates/scout-corpus/src/capture.rs:754-790) splits only on whitespace and `"'\`()[]<>{},;=|`. Tokens such as `flat.md.`, `**flat.md**`, `flat.md:` and `literature/a.md.` never equal the identity. Run against a scratch vault: with `see flat.md.`, `**flat.md**`, `flat.md: yes` in a note, `--where-used flat.md` returned only the `(see flat.md)` and `[x](flat.md)` lines. The old code (`line.contains(file)`) found all of them. Failure: a research file cited at the end of a sentence or in bold is reported as unused, so the user may delete or move a file still referenced. The builder changed the legacy test fixture from `Reuse {file}` to a path (diff of crates/scout-corpus/tests/capture.rs) rather than keeping it passing for prose. Spec 4/3 asks for exact path matching, not for dropping prose citations.

F-2 MAJOR. where-used false positives on bare names and across projects. (a) `candidate == identity` (capture.rs:~783) matches a bare token `flat.md` anywhere, even when that link resolves to `notes/deep/flat.md`, a different file. Observed: `[x](flat.md)` in `notes/deep/n.md` was reported as a usage of `research/flat.md`. Spec: "Matches on the path relative to the referencing file, or the research-relative path, not the bare basename". For a top-level research file the research-relative path is the bare basename, so same-named files in other folders collide. (b) When the requested path does not exist on disk (relative form, e.g. `research/sub2/a.md`), identity is only the research suffix, and `research_suffix(resolved) == identity` accepts `../other/research/sub2/a.md` from any other project. (When the file exists, canonicalisation gives exact matching; the other-project link correctly did not match in that run.) (c) `research_suffix` (capture.rs:~741) starts at the first component named `research`, so a project living under an ancestor directory called `research` yields a wrong suffix and false negatives.

F-3 MINOR. Case-insensitive filesystems. `--into Literature` when `literature/` exists writes into the existing directory on APFS but records `Literature/...` in the manifest (observed). where-used and `--moved` compare case-sensitively, so the receipt and the on-disk path disagree. `--moved literature/a.md literature/A.md` (case-only rename) is refused with "old capture still exists" because `symlink_metadata(old)` succeeds on APFS.

F-4 MINOR. `--moved` accepts any existing regular file as NEW, including `_manifest.jsonl`, dotfiles and non-capture files, provided OLD has a receipt (capture.rs:~694). Not exploitable beyond self-harm, but a receipt can be pointed at the manifest itself.

F-5 MINOR. `--into` creates directories before the collision/copy checks (capture.rs:~396 then loop), so a later failure leaves empty subdirectories. Hidden `--into .hid` is accepted but the file is not indexed (indexed: false) and where-used skips dot-directories; the spec did not forbid it, but it creates unindexed captures. The error text says "." components are rejected, yet `a/./b` is silently accepted (Path::components drops mid-path CurDir).

## Standards axis (judgement calls)

S-1 MINOR. `mentions_research` is dense: two splitters chained, two delimiter lists duplicated as string literals, three overlapping match rules. Smell: duplicated code and a long function doing tokenising, decoding, resolving and identity comparison. Extracting a `links_in(line)` iterator and a small `ResearchIdentity` type would make F-1/F-2 fixable in one place.

S-2 MINOR. `capture_cmd.rs` uses `args.project.as_deref().unwrap()` and `args.moved[0]/[1]` indexing; clap guarantees these, but a named struct field pair would remove the unwraps. `--moved` prints `CaptureList` rows, reusing the list type: acceptable.

S-3 MINOR. Report says "six new regression tests" in corpus plus one CLI test; the escape-refusal and move tests cover refusals but I found no test for trailing-punctuation mentions, cross-project same suffix, or case-only rename (F-1..F-3 are untested).

## Checked and holds

1. Path safety of `--into`: `research_relative` rejects empty, absolute, `..`, prefix components; `research_path` refuses any symlink component (observed: `--into lnk/q` with `lnk -> /tmp` refused, nothing written to /tmp). `../x` and `/tmp/zz` refused. Unicode directory (`é/x`) stays inside research. No escape found. Residual TOCTOU between lstat and create is negligible under the held lock.
2. `--moved`: takes the same lock as capture (`research/manifest`), refuses when old still exists, new missing, new already receipted, old receipted twice, old unreceipted, manifest symlinked or hard-linked; writes via `write_atomic` (temp file, fsync, rename) so a crash leaves either the old or the new manifest. Untouched lines are kept verbatim, only the target row is re-serialised, and `CaptureReceipt` has no extra fields, so nothing is lost. A malformed manifest line aborts before writing. No duplicate or lost receipts found. Observed: basename-only legacy entry `flat.md` moved to `literature/flat.md` correctly rewritten.
3. Backward compatibility: flat and basename-only manifest entries list unchanged (`receipts()` just parses lines); where-used finds flat basename mentions (subject to F-1/F-2).
4. where-used link forms: `../../research/literature/a.md#sec`, `<../../research/literature/a.md>` and `..%2F..%2Fresearch%2F...` from a nested corpus folder all matched; a different-subfolder same-name file (`sub2/a.md`) and `other/research/a.md` were not matched when the target file exists. Links under `research/` are still skipped.

## Test results

`CARGO_TARGET_DIR=/tmp/scout-rv-target cargo test --workspace` (default TMPDIR): `layer_files_regular_only.rs` PASSES, confirming the builder's failure came from its long worktree TMPDIR, not from the code. The first run stopped at `event_edit::log::tests::a_long_log_is_read_with_one_line_looked_at_per_undo`, a 3-second timing assertion (log.rs:926) that flaked under load; a `--no-fail-fast` re-run showed no FAILED lines. I did not run origin/main because nothing fails now. I did not run the `packages/` bun tests. Target dir and scratch vault deleted.

## Not verified

The final `--no-fail-fast` run was filtered through grep for FAILED/panicked only; I did not read per-suite counts. Windows behaviour untested. Concurrency between two simultaneous `--moved`/capture calls was reasoned from the shared lock, not run.

## Verdict

NOT CLEAN. Write-path safety is sound; fix F-1 and F-2 (where-used matching) and add tests, then re-review those two functions only.

# Review r2: scout-core PR 5, research subfolders fix round r1 (Sonnet 5.5)

Verdict: NOT CLEAN. All r1 findings are fixed except one residual in F-2, and the fix leaves a cross-project false positive for bare names (N-1, MAJOR). Scope: the last commit only (`git diff HEAD~1`).

## r1 findings

- F-1 FIXED. Scratch vault: `see flat.md.`, `**flat.md**`, `flat.md: yes`, `_flat.md_`, `~~flat.md~~`, `"flat.md".`, `flat.md,`, `flat.md;`, `flat.md)`, `*flat.md*`, `**research/flat.md**.`, `[a](<../research/flat.md>)`, `(../research/flat.md "title")`, `#h` anchors all matched. `flat.md.bak`, `flat.md.md`, `xflat.md`, `flat.mdx` did not match (correct). Evidence: capture.rs `links_in`, `MENTION_DELIMITERS`.
- F-2 PARTLY FIXED.
  - (a) FIXED. `[x](flat.md)` in `notes/deep/n.md` (resolves to `notes/deep/flat.md`) is not a usage of `research/flat.md`; `[y](../../research/flat.md)` is. A bare prose name matches only when it is the sole basename in that project's research tree: two `dup.md` files gave no hit for either.
  - (b) FIXED for links. A missing target no longer matches `../other/research/...` from another project (identity is canonical `root.join(relative)` per referencing file); `[o](../../p2/research/literature/a.md)` did not match p1's file. Residual: see N-1.
  - (c) FIXED when the corpus root is the `research` ancestor (absolute request matched `see z.md.` and `../research/z.md`). Residual: see N-2.
- F-3 FIXED. `--into LITERATURE/new` with existing `literature/` wrote under `literature/` and the receipt recorded `literature/new/...` (on-disk spelling). I did not exercise the case-only `--moved` rename by hand; the unit test `capture_records_disk_case_and_moved_allows_completed_case_only_rename` passes.
- F-4 FIXED by reading: `moved` refuses a `new` whose file name is `_manifest.jsonl` (case-insensitive), hidden components, absolute, `..`. Test `moved_refuses_manifest_hidden_and_outside_destinations` passes. Note the check is on the file name only; `_manifest.jsonl` in a subfolder is also refused, which is fine.
- F-5 FIXED. Component validation (`research_relative`) now rejects empty, `.`-prefixed and `.`/`..` parts on the raw string, so `a/./b` is refused (the CLI run with `a/./b` failed on a missing positional in my invocation, so I rely on the unit test `into_rejects_hidden_empty_and_dot_components_without_creating_directories`, which passes). Directories are created after collision checks and rendering, with an `EmptyDirectories` drop guard.
- S-1 FIXED (one tokenizer, one delimiter constant, `ResearchIdentity`). S-2 FIXED (`context(...)`, slice pattern `[old, new]`, no unwraps). S-3 FIXED: seven new tests.

## New findings

N-1 MAJOR. Bare-name prose match ignores project boundaries (capture.rs, `ResearchIdentity::mentioned`, single-component branch: `self.unique_basename && candidate.file_name() == self.target.file_name()`). The multi-component branch is guarded by `parent.starts_with(root.parent())`; the single-component branch is not. Scratch run: `--where-used <abs>/p1/research/flat.md` returned `p2/notes/o.md:1: see flat.md and also research/flat.md` and `p3/notes/n.md:1: see zz.md and flat.md`; neither project has that file (p2 has no `flat.md`; p3 has no research folder). Failure: a note in another project that happens to say `flat.md` is reported as a usage, so the report overstates use (a safe-side error, but the same class as r1 F-2). Same result for the relative request `research/flat.md` that exists in the cwd project. Fix: apply the same `parent.starts_with(project root)` guard to the bare-name branch.

N-2 MINOR. `where_used` still prunes any directory named `research` under the corpus root (capture.rs where_used `filter_entry`, `e.file_name() != "research"`). A project under a folder called `research` inside a corpus (`vault/research/proj/notes/n.md`) is never scanned; the same file with the corpus root set to that folder works. The new test only covers the second layout (`lookup_uses_project_research_root_below_research_ancestor` makes the corpus root the ancestor). Failure: false negatives for such vaults. Pre-existing pruning, but F-2(c) claims the ancestor case is handled.

N-3 MINOR. Relative (non-existent in cwd) requests are applied per referencing file against that file's nearest `research/` ancestor, so `literature/a.md` run from p1 reports p2's `research/literature/a.md` mentions too. This follows from not knowing the project and is arguably intended, but the nearest-ancestor search (`referencing.parent()?.ancestors().map(join("research")).find(is_dir)`) can bind a project with no research folder to an unrelated `research/` further up.

## Tests

`cargo test -p scout-corpus --test capture`: 30 passed, 0 failed. `cargo test -p scout-cli --test capture`: 5 passed, 0 failed. Run with `TMPDIR=/tmp/scr2`, target `/tmp/scout-rv2-target`; both deleted. Not run: the workspace suite, bun tests, Windows.

## Not verified

Manual `--moved` case-only rename, concurrency, and the cleanup guard under a mid-write failure (read only).

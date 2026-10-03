# 09: Corpus registration for strangers (add, remove, generic starter)

**Why:** ArchiveScout and the public `archive-scout` skill are being prepared for public release (ArchiveScout round-4 first-run design, 2026-10-02). A stranger cannot register a corpus today: the CLI has only `corpora list | show | init-defaults`, and `init-defaults` writes Dominik's own paths (`Registry::defaults()` hard-codes `~/gitrepos/02_writing-creation/writing`, his author name and the WriteFlex link). The missing-registry error tells strangers to run it. This is needed whichever first-run direction Dominik picks; importers (tweets.js, Readwise CSV) wait for his source-type answer and are NOT in this ticket.

## Invariants (must still hold)

- The engine never writes to a corpus's source folder. The only file this ticket writes is the registry (`~/.config/scout/corpora.toml`, or the path the existing code resolves) and the index store it already writes.
- Dominik's existing registry keeps working byte-for-byte unless he runs a write command; his default corpora stay available to him.
- Registry writes are atomic (write temp + rename in the same dir) and never leave a half-written file; comments at the top of the file are preserved or regenerated, never corrupt the TOML.
- No private path, name or URL in any code path a stranger reaches.

## Behaviour

1. `scout corpora add <path> [--id <id>] [--name <name>] [--kind markdown-folder|highlight-scout-archive] [--author <name>] [--include <glob>…] [--exclude <glob>…] [--json]`
   - Path must exist and be a directory; `~` expanded; stored as given (with `~` when under home).
   - `--kind` defaults by detection: a Highlight Scout archive is recognised by its marker files; otherwise `markdown-folder`.
   - `--id` defaults to a slug of the folder name, de-duplicated (`notes`, `notes-2`); explicit id that clashes → error naming the clash.
   - Defaults for markdown-folder: include `**/*.md` and `**/*.txt`, no required frontmatter, no link template, no default author, document unit File.
   - Creates the registry if missing. Prints what was added and the next command (`scout index build <id>`). Does NOT build the index.
   - `--json` returns the added corpus config.
2. `scout corpora remove <id> [--keep-index] [--json]`: removes the entry; deletes that corpus's index (word and meaning stores) unless `--keep-index`. Never touches the source folder. Unknown id → the existing "unknown corpus" error.
3. `scout corpora init-defaults` becomes generic: writes an empty registry with a comment explaining `scout corpora add`. Dominik's defaults move behind an explicit `--preset dominik` that is NOT mentioned in the missing-registry error or `--help` examples (keep the flag documented in the code comment only). The missing-registry error says: `no corpus registry at <path>; add a folder with \`scout corpora add <folder>\``.
4. Library API in `scout-corpus::api`: `add_corpus(AddCorpus) -> Result<CorpusConfig>` and `remove_corpus(id, keep_index) -> Result<Removed>` so ArchiveScout can call them without the CLI. Same validation as the CLI.
5. A folder that is missing at load time: `corpora list` shows its state as `folder missing` (index kept); search over it still works from the index. (Decided 2026-10-02: keep and mark, never auto-remove.)
6. Plain `.txt` without frontmatter is indexed: title = file stem, date = none (date policy for undated docs waits for Dominik's answer; leave them undated as today).

## States to cover (test matrix)

| Registry | Command | Expect |
|---|---|---|
| missing | add | registry created with one corpus |
| missing | list / search | new error text, exit non-zero |
| exists, comments + Dominik's 3 corpora | add | 4 corpora, original 3 unchanged field-for-field |
| exists | add same path twice | second refused (path already registered as `<id>`) |
| exists | add with clashing `--id` | refused |
| exists | add nonexistent path / a file | refused, registry untouched |
| exists | remove | entry gone, index dir gone, source mtimes unchanged |
| exists | remove --keep-index | entry gone, index dir kept |
| exists, corrupt TOML | add | refused with parse error, file untouched |
| write interrupted (simulate failing rename) | add | original file intact |
| folder deleted after add | list | `folder missing`; index still searchable |
| `.txt` without frontmatter | index build | indexed, titled by stem |
| init-defaults | no flag | empty generic registry; grep finds no `gitrepos`, `Lukeš`, `writeflex` |

**Seams under test:** registry read-modify-write (atomic), `api::add_corpus/remove_corpus`, kind detection.
**Sweep after:** `grep -rn "gitrepos\|Lukeš\|dominik" crates` returns only the `--preset dominik` block; Dominik's live `scout corpora list` output identical before/after the build (no write command run against his registry during tests: use a temp HOME/XDG dir).
**Review:** owed (write path + public interface): Fable 5.1 for an Opus build.
**Status:** ready

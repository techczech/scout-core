# 07: Semantic search over any corpus

**What to build:** Dominik (DTC one-search-preview-4, 2026-09-28): "we should build semantic options for any archive". Today semantic search exists only for highlights, through Highlight Scout's qmd index. Make it an engine capability for every corpus (writing, tweets, highlights, and any pile ArchiveScout indexes), used by both apps and the CLI.

**Decisions (Fable's technical call; recorded here, ADR to follow if this lands):**
- **Local only.** No network at query time, no cloud embeddings; his archive never leaves the machine. This fits the public ArchiveScout release.
- **Model:** a small multilingual sentence-embedding model (e.g. `paraphrase-multilingual-MiniLM-L12-v2` or `multilingual-e5-small`), because his corpus is Czech and English.
  - Run it in-process via ONNX (fastembed-rs or ort), cross-platform, with the model downloaded once on first use to the platform data dir (not bundled).
  - State the download size to the user before downloading.
  - Pick the model by a small recall test on his corpus: 10 hand-written queries of his kind, e.g. "the metaphor of the mind as a computer", with judged hits from `scout search`. Report the chosen model and its scores.
- **Unit:** one vector per passage, stored beside the FTS index (`<corpus>.vectors` or a sqlite-vec table). Built incrementally with the index; an optional step (`scout index build --semantic`), since it costs minutes.
- **Query:** `scout search '<q>' --semantic` (cosine, top k) and a HYBRID mode (the default when vectors exist): FTS rank fused with semantic rank (reciprocal rank fusion), so exact words still win when present.
  - Invariants 2 (quotes are original text, passage-level) and 6 (determinism) hold.
  - `similar` may use the vectors when they exist, keeping tf-idf as the fallback.

**Blocked by:** None. **Seams under test:** the embedder trait (with a fake embedder in tests, so there is no model download in CI); the vector store round trip; hybrid fusion ordering; facade parity.
**Status:** landed

- [ ] `scout search "the mind as a machine" --semantic --in writing` returns the mind-as-computer passages (2005 Czech essay, 2016 draft) without the literal words.
- [ ] A full vector build of writing + tweets + highlights: time and disk size reported; incremental rebuild with no changes < 3 s.
- [ ] Apps unaffected until they opt in; the index version bumps only if the FTS schema changes.

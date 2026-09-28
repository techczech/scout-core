# 08: Semantic housekeeping — one model-location rule, typed errors, vector cache

**What to build:** Three follow-ups the app builders reported after v0.3.0:
1. **One model-location rule in the engine.** Highlight Scout and ArchiveScout each re-implemented where to find the model: `SCOUT_MODEL_DIR`, then `HF_HOME`, then `~/local-models` if it holds a model cache, then the platform data dir. Apps opened from Finder do not see `HF_HOME`. Move the rule into scout-corpus as `semantic::model_dir()`, used by the CLI and exposed to the apps; the apps then delete their copies (separate small app tickets).
2. **Typed errors.** Semantic refusals ("no current vectors", "model not on disk", "model downloading") become typed variants in the facade error, so apps stop matching message text.
3. **Vector cache.** Each semantic query re-reads every vector from SQLite (~265 MB over three corpora), which is most of the ~1 s per query. Keep vectors in memory in the `Engine` (lazy, invalidated by `content_generation`), so a warm query takes < 200 ms. Optionally quantise to int8 if recall stays within 1 hit on the ticket-07 recall set.

**Blocked by:** None. **Seams under test:** `model_dir()` resolution (temp HOME fixtures); facade error variants; warm-query timing on a fixture, plus a real measurement.
**Status:** ready

- [ ] The CLI, Highlight Scout and ArchiveScout all resolve the same model folder from a Finder-like environment (HF_HOME unset).
- [ ] A warm semantic query over three corpora takes < 200 ms (measured and reported).

// Orchestrates one run: select -> pack -> guard cost -> tag -> consolidate -> write layer.
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { selectWorks } from './works.js';
import { packItems } from './batching.js';
import { estimateRunUsd, addUsage } from './cost.js';
import { DISCOVERY_SYSTEM, PROMPT_VERSION, vocabularySystem } from './prompts.js';
import { collectBatches, runPool, submitBatches, tagBatch, waitForBatches } from './model.js';
import { applyVocabulary, summarise } from './results.js';
import { consolidate } from './consolidate.js';
import { buildReviewHtml } from './review.js';
import { fullStatus, readingsStatus } from './guard.js';

const writeJson = (p, o) => writeFileSync(p, JSON.stringify(o, null, 2) + '\n');
const RANGE_RE = /^\d{4}-\d{2}-\d{2}$/;

export function planRun(opts) {
  if (!RANGE_RE.test(opts.from) || !RANGE_RE.test(opts.to)) throw new Error('--from/--to must be YYYY-MM-DD');
  const { items, stats } = selectWorks(join(opts.archiveRoot, 'readings/works'), opts.from, opts.to);
  const batches = packItems(items, { maxItems: opts.maxItems, maxTokens: opts.maxTokens });
  const est = estimateRunUsd(opts.model, batches, { batch: opts.batchApi });
  return { items, stats, batches, est };
}

export async function executeRun(opts, { client, log = () => {} } = {}) {
  opts = { ...opts, log };
  const plan = planRun(opts);
  const { items, stats, batches, est } = plan;
  log(`selected ${items.length} items in ${batches.length} requests; estimated $${est.usd.toFixed(3)} (cap $${opts.maxUsd})`);
  if (est.usd > opts.maxUsd) throw new Error(`estimated cost $${est.usd.toFixed(3)} exceeds --max-usd ${opts.maxUsd}; aborting before any request`);
  if (opts.dryRun) return { dryRun: true, ...plan };
  if (!items.length) throw new Error('no items selected');

  let vocabulary = null, vocabularyHash = null;
  if (opts.mode === 'vocabulary') {
    const raw = readFileSync(opts.vocabularyPath, 'utf8');
    vocabulary = JSON.parse(raw);
    vocabularyHash = createHash('sha256').update(raw).digest('hex').slice(0, 16);
  }
  const system = vocabulary ? vocabularySystem(vocabulary) : DISCOVERY_SYSTEM;
  const cfg = { model: opts.model, effort: opts.effort, system, vocabulary };

  const runDir = join(opts.archiveRoot, 'analysis/topics', opts.runId);
  const resuming = opts.resume && existsSync(join(runDir, 'batch.json'));
  if (existsSync(runDir) && !resuming) throw new Error(`${runDir} already exists; layers are additive, pick a new --run-id`);
  mkdirSync(runDir, { recursive: true });

  const statusBefore = readingsStatus(opts.archiveRoot);
  const otherBefore = fullStatus(opts.archiveRoot);
  const started = new Date();
  const run = {
    run_id: opts.runId,
    mode: opts.mode,
    model: opts.model,
    effort: opts.effort,
    thinking: 'adaptive',
    api: opts.batchApi ? 'message-batches (50% off)' : 'messages',
    started: started.toISOString(),
    finished: null,
    prompt_version: PROMPT_VERSION,
    prompt_text: system,
    selection_rule: `works whose latest highlighted_at is in ${opts.from}..${opts.to}; text = title, author, type, date, highlights, capped at 12000 chars`,
    item_count: items.length,
    capped_items: stats.capped,
    undated_works_skipped: stats.undated,
    key_fallbacks: stats.keyFallbacks,
    key_note: stats.keyFallbacks
      ? 'some keys fell back to sha256:<16 hex of file content> because source_system/source_id were missing'
      : 'all keys are <source_system>:<source_id> from frontmatter; no content-hash fallback was needed',
    requests: batches.length,
    estimated_usd: +est.usd.toFixed(4),
    vocabulary_source: vocabulary ? { path: opts.vocabularyPath, sha256_16: vocabularyHash, topics: vocabulary.topics.length } : null,
  };
  writeJson(join(runDir, 'run.json'), run);

  // ---- tagging ----
  const records = [];
  const failed = [];
  const problems = [];
  let usage = { input_tokens: 0, output_tokens: 0 };
  let spent = 0;
  let tagCost = 0;
  const take = (r) => {
    records.push(...r.records);
    failed.push(...r.failed);
    problems.push(...r.problems);
    usage = addUsage(usage, r.usage);
    tagCost += r.cost || 0;
  };
  if (opts.batchApi) {
    let id;
    if (resuming) id = JSON.parse(readFileSync(join(runDir, 'batch.json'), 'utf8')).batch_id;
    else {
      id = await submitBatches(client, cfg, batches);
      writeJson(join(runDir, 'batch.json'), { batch_id: id, requests: batches.map((b) => b.id), submitted: new Date().toISOString() });
    }
    log(`batch ${id} submitted; polling`);
    await waitForBatches(client, id, { log });
    take(await collectBatches(client, id, batches, cfg));
    spent = tagCost;
  } else {
    let done = 0, stopped = false;
    await runPool(batches, opts.concurrency, async (b) => {
      if (stopped) return { records: [], failed: b.items.map((i) => i.key), usage: {}, cost: 0, problems: [`${b.id}: not sent, spend cap reached`] };
      return tagBatch(client, cfg, b);
    }, (b, r) => {
      take(r);
      spent = tagCost;
      if (spent > opts.maxUsd) stopped = true;
      if (++done % 10 === 0 || done === batches.length) log(`${done}/${batches.length} requests, ${records.length} items, $${spent.toFixed(3)}`);
    });
  }
  records.sort((a, b) => a.key.localeCompare(b.key));
  writeFileSync(join(runDir, 'tags.jsonl'), records.map((r) => JSON.stringify(r)).join('\n') + (records.length ? '\n' : ''));
  run.tagged_items = records.length;
  run.failed_keys = failed;
  run.problems = problems.slice(0, 200);
  run.usage = usage;
  run.cost_usd_tagging = +tagCost.toFixed(4);
  run.tagging_finished = new Date().toISOString();
  run.wall_seconds_tagging = Math.round((Date.now() - started.getTime()) / 1000);
  run.readings_unchanged = readingsUnchanged(opts.archiveRoot, statusBefore);
  run.archive_status_outside_analysis_unchanged = outsideUnchanged(opts.archiveRoot, otherBefore);
  writeJson(join(runDir, 'run.json'), run); // tags are safe on disk before any vocabulary call
  return finishRun(opts, { client, runDir, run, records, vocabulary, started });
}

const readingsUnchanged = (root, before) => (before === null ? 'unchecked (not a git repo)' : before === readingsStatus(root));
const strip = (s) => (s || '').split('\n').filter((l) => !l.includes('analysis/')).join('\n');
const outsideUnchanged = (root, before) => (before === null ? 'unchecked' : strip(before) === strip(fullStatus(root)));

// Vocabulary + review + final run.json. Re-runnable on an existing run folder (consolidateRun).
async function finishRun(opts, { client, runDir, run, records, vocabulary, started }) {
  const { ai, topicCounts } = summarise(records);
  const recordsByKey = Object.fromEntries(records.map((r) => [r.key, r]));
  let rows, unmappedCount = 0, vocabCost = 0;
  if (run.mode === 'discovery') {
    const c = await consolidate(client, { model: opts.consolidateModel, records: records.filter((r) => r.ai_related), log: opts.log });
    vocabCost = c.cost;
    rows = c.rows;
    unmappedCount = c.unmapped.length;
    writeJson(join(runDir, 'vocabulary.json'), {
      run_id: run.run_id,
      model: opts.consolidateModel,
      method: 'propose topics from frequent labels, then file every label under them in chunks of 200',
      free_form_labels: c.labelCount,
      unmapped_labels: c.unmapped,
      topics: c.rows.map((t) => ({ ...t, merged_labels: c.mergedLabels[t.topic] || [] })),
      mapping: c.mapping,
    });
    run.vocabulary_model = opts.consolidateModel;
    run.vocabulary_topics = c.rows.length;
    run.vocabulary_usage = c.usage;
    run.vocabulary_bad_assignments = c.badAssignments;
  } else {
    const defs = new Map(vocabulary.topics.map((t) => [t.topic, t.definition]));
    const ident = Object.fromEntries([...topicCounts.keys()].map((t) => [t, t]));
    const topics = [...topicCounts.keys()].map((t) => ({ topic: t, definition: defs.get(t) || '(new topic proposed by the model)' }));
    rows = applyVocabulary(records, topics, ident).topics;
  }
  const finished = new Date();
  run.finished = finished.toISOString();
  run.wall_seconds = Math.round((finished - started) / 1000);
  run.ai_count = ai;
  run.ai_share = +(ai / Math.max(records.length, 1)).toFixed(4);
  run.cost_usd_vocabulary = +vocabCost.toFixed(4);
  run.cost_usd_total = +((run.cost_usd_tagging || 0) + vocabCost).toFixed(4);
  run.cost_note = 'computed per request from response usage at list prices (haiku-5-5 $0.10/$0.50 per MTok, batch half; sonnet-5-5 $2/$10)';
  writeJson(join(runDir, 'run.json'), run);
  writeFileSync(join(runDir, 'review.html'), buildReviewHtml({ archiveRoot: opts.archiveRoot, run, rows, recordsByKey, unmappedCount }));
  return { run, runDir };
}

// Re-run only the vocabulary step on a run whose tagging finished (e.g. after a failed consolidation).
export async function consolidateRun(opts, { client, log = () => {} }) {
  opts = { ...opts, log };
  const runDir = join(opts.archiveRoot, 'analysis/topics', opts.runId);
  const run = JSON.parse(readFileSync(join(runDir, 'run.json'), 'utf8'));
  if (!run.tagging_finished) throw new Error('tagging did not finish for this run');
  const records = readFileSync(join(runDir, 'tags.jsonl'), 'utf8').split('\n').filter(Boolean).map((l) => JSON.parse(l));
  const before = readingsStatus(opts.archiveRoot);
  const wallBefore = run.wall_seconds_tagging;
  const started = new Date(Date.now() - wallBefore * 1000);
  const r = await finishRun(opts, { client, runDir, run, records, vocabulary: null, started });
  r.run.readings_unchanged = r.run.readings_unchanged === true && readingsUnchanged(opts.archiveRoot, before);
  writeJson(join(runDir, 'run.json'), r.run);
  return r;
}

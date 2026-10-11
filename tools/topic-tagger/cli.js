#!/usr/bin/env node
// topic-tagger CLI. Run with ANTHROPIC_API_KEY injected by the estate bws helper (never stored).
//   v2 (context-aware, one month):  cli.js <stage> --month YYYY-MM [--run-id id]
//     stages: profile-sources | briefing | themes | enrich | tag | qa | review | all
//   v1 (legacy free-form topics):   cli.js --from YYYY-MM-DD --to YYYY-MM-DD [...]
import { parseArgs } from 'node:util';
import { homedir } from 'node:os';
import { join } from 'node:path';
import Anthropic from '@anthropic-ai/sdk';
import { consolidateRun, executeRun } from './lib/run.js';
import { DEFAULT_MAX_ITEMS, DEFAULT_MAX_TOKENS } from './lib/batching.js';
import * as v2 from './lib/pipeline.js';

const STAGES = ['profile-sources', 'briefing', 'themes', 'enrich', 'tag', 'qa', 'review', 'all'];
const PAID = new Set(['briefing', 'themes', 'tag', 'qa', 'all']);
const log = (m) => console.error(`[${new Date().toISOString().slice(11, 19)}] ${m}`);
const home = (p) => join(homedir(), p);

const { values: v, positionals } = parseArgs({
  allowPositionals: true,
  options: {
    archive: { type: 'string', default: home('gitrepos/01_reading-research/highlights-archive') },
    // v2
    month: { type: 'string' },
    'tweets-dir': { type: 'string', default: home('gitrepos/02_writing-creation/writing/tweets/stream') },
    'roundup-dir': { type: 'string', default: home('gitrepos/01_reading-research/ai-news-tracking/preview-browser/public/data/roundups') },
    roundup: { type: 'string' },
    'v1-run': { type: 'string', default: '2026-10-10-sample-2026-09' },
    'briefing-model': { type: 'string', default: 'claude-sonnet-5-5' },
    'themes-model': { type: 'string', default: 'claude-opus-5-5' },
    'qa-model': { type: 'string', default: 'claude-sonnet-5-5' },
    'qa-effort': { type: 'string', default: 'medium' },
    'tweets-chars': { type: 'string', default: '25000' },
    'roundup-chars': { type: 'string', default: '12000' },
    'user-reserve': { type: 'string', default: '12000' },
    seed: { type: 'string', default: '20261011' },
    force: { type: 'boolean', default: false },
    // shared
    'run-id': { type: 'string' },
    model: { type: 'string', default: 'claude-haiku-5-5' },
    effort: { type: 'string' },
    'max-usd': { type: 'string' },
    concurrency: { type: 'string' },
    // v1
    from: { type: 'string' },
    to: { type: 'string' },
    mode: { type: 'string', default: 'discovery' },
    vocabulary: { type: 'string' },
    'consolidate-model': { type: 'string', default: 'claude-haiku-5-5' },
    'max-items': { type: 'string', default: String(DEFAULT_MAX_ITEMS) },
    'max-tokens': { type: 'string', default: String(DEFAULT_MAX_TOKENS) },
    batch: { type: 'boolean', default: false },
    resume: { type: 'boolean', default: false },
    'consolidate-only': { type: 'boolean', default: false },
    'dry-run': { type: 'boolean', default: false },
  },
});

const stage = positionals[0];
try {
  if (stage) await runV2(stage);
  else await runV1();
} catch (e) {
  console.error(`error: ${e.message}`);
  process.exit(1);
}

async function runV2(stage) {
  if (!STAGES.includes(stage)) { console.error(`unknown stage ${stage}; one of ${STAGES.join(', ')}`); process.exit(2); }
  if (!v.month) { console.error('v2 stages need --month YYYY-MM'); process.exit(2); }
  const env = {
    archiveRoot: v.archive, month: v.month,
    runId: v['run-id'] || `${new Date().toISOString().slice(0, 10)}-v2-${v.month}`,
    v1RunId: v['v1-run'], tweetsDir: v['tweets-dir'], roundupDir: v['roundup-dir'], roundupPath: v.roundup,
    model: v.model, effort: v.effort || 'medium', briefingModel: v['briefing-model'], themesModel: v['themes-model'],
    qaModel: v['qa-model'], qaEffort: v['qa-effort'],
    tweetsChars: Number(v['tweets-chars']), roundupChars: Number(v['roundup-chars']), userReserve: Number(v['user-reserve']),
    maxUsd: Number(v['max-usd'] || 5), concurrency: Number(v.concurrency || 4), seed: Number(v.seed), force: v.force, dryRun: v['dry-run'],
    client: PAID.has(stage) ? new Anthropic({ maxRetries: 4 }) : null, log,
  };
  const steps = stage === 'all' ? ['briefing', 'themes', 'enrich', 'tag', 'qa', 'review'] : [stage];
  const fns = { 'profile-sources': v2.stageProfileSources, briefing: v2.stageBriefing, themes: v2.stageThemes, enrich: v2.stageEnrich, tag: v2.stageTag, qa: v2.stageQa, review: v2.stageReview };
  const out = {};
  for (const s of steps) { log(`== ${s}`); out[s] = await fns[s](env); }
  console.log(JSON.stringify({ runId: env.runId, ...out }, null, 2));
}

async function runV1() {
  if (!v.from || !v.to) { console.error('usage: cli.js <stage> --month YYYY-MM | cli.js --from YYYY-MM-DD --to YYYY-MM-DD [--mode discovery|vocabulary --vocabulary file] [--batch] [--dry-run] [--consolidate-only --run-id id] [--max-usd 3] [--run-id id]'); process.exit(2); }
  if (!['discovery', 'vocabulary'].includes(v.mode)) { console.error('--mode must be discovery or vocabulary'); process.exit(2); }
  if (v.mode === 'vocabulary' && !v.vocabulary) { console.error('vocabulary mode needs --vocabulary <vocabulary.json>'); process.exit(2); }
  const opts = {
    archiveRoot: v.archive, from: v.from, to: v.to,
    runId: v['run-id'] || `${new Date().toISOString().slice(0, 10)}-${v.mode}-${v.from}_${v.to}`,
    mode: v.mode, vocabularyPath: v.vocabulary, model: v.model, effort: v.effort || 'low',
    consolidateModel: v['consolidate-model'], maxUsd: Number(v['max-usd'] || 3),
    maxItems: Number(v['max-items']), maxTokens: Number(v['max-tokens']),
    concurrency: Number(v.concurrency || 6), batchApi: v.batch, resume: v.resume, dryRun: v['dry-run'],
  };
  const client = opts.dryRun ? null : new Anthropic({ maxRetries: 4 });
  const r = v['consolidate-only'] ? await consolidateRun(opts, { client, log }) : await executeRun(opts, { client, log });
  if (r.dryRun) {
    console.log(JSON.stringify({ items: r.items.length, requests: r.batches.length, stats: r.stats, estimate: r.est }, null, 2));
  } else {
    const { run, runDir } = r;
    console.log(JSON.stringify({ runDir, items: run.item_count, tagged: run.tagged_items, ai: run.ai_count, aiShare: run.ai_share, failed: run.failed_keys.length, topics: run.vocabulary_topics, costUsd: run.cost_usd_total, wallSeconds: run.wall_seconds, readingsUnchanged: run.readings_unchanged }, null, 2));
  }
}

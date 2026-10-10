#!/usr/bin/env node
// topic-tagger CLI. Run with ANTHROPIC_API_KEY injected by the estate bws helper (never stored).
import { parseArgs } from 'node:util';
import { homedir } from 'node:os';
import { join } from 'node:path';
import Anthropic from '@anthropic-ai/sdk';
import { consolidateRun, executeRun } from './lib/run.js';
import { DEFAULT_MAX_ITEMS, DEFAULT_MAX_TOKENS } from './lib/batching.js';

const { values: v } = parseArgs({
  options: {
    archive: { type: 'string', default: join(homedir(), 'gitrepos/01_reading-research/highlights-archive') },
    from: { type: 'string' },
    to: { type: 'string' },
    'run-id': { type: 'string' },
    mode: { type: 'string', default: 'discovery' },
    vocabulary: { type: 'string' },
    model: { type: 'string', default: 'claude-haiku-5-5' },
    effort: { type: 'string', default: 'low' },
    'consolidate-model': { type: 'string', default: 'claude-haiku-5-5' },
    'max-usd': { type: 'string', default: '3' },
    'max-items': { type: 'string', default: String(DEFAULT_MAX_ITEMS) },
    'max-tokens': { type: 'string', default: String(DEFAULT_MAX_TOKENS) },
    concurrency: { type: 'string', default: '6' },
    batch: { type: 'boolean', default: false },
    resume: { type: 'boolean', default: false },
    'consolidate-only': { type: 'boolean', default: false },
    'dry-run': { type: 'boolean', default: false },
  },
});
if (!v.from || !v.to) { console.error('usage: cli.js --from YYYY-MM-DD --to YYYY-MM-DD [--mode discovery|vocabulary --vocabulary file] [--batch] [--dry-run] [--consolidate-only --run-id id] [--max-usd 3] [--run-id id]'); process.exit(2); }
if (!['discovery', 'vocabulary'].includes(v.mode)) { console.error('--mode must be discovery or vocabulary'); process.exit(2); }
if (v.mode === 'vocabulary' && !v.vocabulary) { console.error('vocabulary mode needs --vocabulary <vocabulary.json>'); process.exit(2); }

const opts = {
  archiveRoot: v.archive, from: v.from, to: v.to,
  runId: v['run-id'] || `${new Date().toISOString().slice(0, 10)}-${v.mode}-${v.from}_${v.to}`,
  mode: v.mode, vocabularyPath: v.vocabulary, model: v.model, effort: v.effort,
  consolidateModel: v['consolidate-model'], maxUsd: Number(v['max-usd']),
  maxItems: Number(v['max-items']), maxTokens: Number(v['max-tokens']),
  concurrency: Number(v.concurrency), batchApi: v.batch, resume: v.resume, dryRun: v['dry-run'],
};
const log = (m) => console.error(`[${new Date().toISOString().slice(11, 19)}] ${m}`);
const client = opts.dryRun ? null : new Anthropic({ maxRetries: 4 });
try {
  const r = v['consolidate-only'] ? await consolidateRun(opts, { client, log }) : await executeRun(opts, { client, log });
  if (r.dryRun) {
    console.log(JSON.stringify({ items: r.items.length, requests: r.batches.length, stats: r.stats, estimate: r.est }, null, 2));
  } else {
    const { run, runDir } = r;
    console.log(JSON.stringify({ runDir, items: run.item_count, tagged: run.tagged_items, ai: run.ai_count, aiShare: run.ai_share, failed: run.failed_keys.length, topics: run.vocabulary_topics, costUsd: run.cost_usd_total, wallSeconds: run.wall_seconds, readingsUnchanged: run.readings_unchanged }, null, 2));
  }
} catch (e) {
  console.error(`error: ${e.message}`);
  process.exit(1);
}

// events-propose-merges: propose which lasting-event records are one story. Proposals only.
// Writes exactly two files, <archive>/analysis/events/merge-proposals/<run-id>.jsonl and <run-id>-ledger.json.
// It never opens a record file for writing (records are read through the injected reader, `scout events --json`).
// Chains are not resolved: A~B and B~C stay two lines. Only verdict "same" lines are merge proposals; the rest are audit.
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { join } from 'node:path';
import { buildCandidates, SIGNALS } from './events-merge-candidates.js';
import { costFromUsage, estimateTokens, addUsage } from './cost.js';
import { textOf, runPool } from './model.js';
import { parseWork } from './works.js';
import { clip, wellFormed } from './text.js';

export const MERGE_PROMPT_VERSION = 'events-merge-v1';
export const PAIRS_PER_REQUEST = 8;
export const REQUEST_CHAR_CAP = 70000;
export const ITEMS_PER_SIDE = 6;
export const ITEM_CHARS = 280;
const OUT_PER_PAIR = 90;
const THINKING_PER_REQUEST = 2500;
const squash = (s) => String(s ?? '').replace(/\s+/g, ' ').trim();

export const SYSTEM = `You decide whether two records in one person's reading archive describe the same story. Each record is an "event": a group of tweets and articles he saved about one thing. They were written as separate monthly events, and many stories run over more than one month.

For each pair give:
- verdict "same": one continuing story that a reader would file together (the same announcement and its aftermath, the same controversy continuing).
- verdict "related": two stories that only share a company, a person or a theme.
- verdict "different": unrelated stories. A product launch and a later unrelated launch by the same company are "different".
- confidence "confident" or "doubtful". When unsure, say "doubtful".
- reason: one plain sentence, at most 160 characters, naming what the two share or what separates them.

Shared words in the names are not evidence by themselves. Judge from the descriptions, the dates and the sample items. Use only what is given. Answer for every pair, using its id.`;

export const SCHEMA = {
  type: 'object',
  properties: {
    results: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          id: { type: 'string' },
          verdict: { type: 'string', enum: ['same', 'related', 'different'] },
          confidence: { type: 'string', enum: ['confident', 'doubtful'] },
          reason: { type: 'string' },
        },
        required: ['id', 'verdict', 'confidence', 'reason'],
        additionalProperties: false,
      },
    },
  },
  required: ['results'],
  additionalProperties: false,
};

/** Default reader: the Rust implementation of the record format, via `scout events --in <corpus> --json`. */
export function scoutReader({ corpus = 'highlights', bin = 'scout' } = {}) {
  return () => {
    const r = spawnSync(bin, ['events', '--in', corpus, '--json'], { encoding: 'utf8', maxBuffer: 256 * 1024 * 1024 });
    if (r.error) throw new Error(`cannot run ${bin} events: ${r.error.message}`);
    if (r.status !== 0) throw new Error(`${bin} events failed: ${squash(r.stderr).slice(0, 300)}`);
    return JSON.parse(r.stdout).events;
  };
}

const readCurrent = (A) => readFileSync(join(A, 'analysis/topics/current.jsonl'), 'utf8').split('\n').filter(Boolean).map((l) => JSON.parse(l));
const dist = (date, e) => (date < e.start ? Date.parse(e.start) - Date.parse(date) : date > e.end ? Date.parse(date) - Date.parse(e.end) : 0);

function itemLoader(A) {
  const cache = new Map();
  return (path) => {
    if (cache.has(path)) return cache.get(path);
    let t = '';
    try {
      const w = parseWork(readFileSync(join(A, path), 'utf8'), path);
      const body = squash(w.highlights.map((h) => h.text).join(' ').replace(/!\[[^\]]*\]\([^)]*\)/g, '').replace(/https?:\/\/t\.co\/\S+/g, ''));
      t = clip(w.type === 'tweet' || !w.title ? body : `${squash(w.title)}: ${body}`, ITEM_CHARS);
    } catch { t = ''; }
    cache.set(path, t);
    return t;
  };
}

function sideText(label, e, prof, rows, other, loadText) {
  const items = [...rows].sort((x, y) => dist(x.date, other) - dist(y.date, other) || x.date.localeCompare(y.date) || x.key.localeCompare(y.key)).slice(0, ITEMS_PER_SIDE)
    .sort((x, y) => x.date.localeCompare(y.date));
  return [
    `${label}: ${squash(e.name)}`,
    `aliases: ${(e.aliases || []).map(squash).join('; ') || '-'}`,
    `span: ${e.start} to ${e.end}; ${e.items} items`,
    `description: ${squash(e.description) || '-'}`,
    `top themes: ${prof.topThemes.join('; ') || '-'}`,
    `top named entities: ${prof.topEntities.join('; ') || '-'}`,
    `sample items (nearest the other event in time):`,
    ...items.map((r) => `- ${r.date}: ${loadText(r.path) || '(text unavailable)'}`),
  ].join('\n');
}

export function pairMessage(p, n, ctx) {
  return `<pair id="p${n}">\n${sideText('A', p.a, ctx.profiles.get(p.a.id), ctx.rows.get(p.a.id), p.b, ctx.loadText)}\n\n${sideText('B', p.b, ctx.profiles.get(p.b.id), ctx.rows.get(p.b.id), p.a, ctx.loadText)}\n</pair>`;
}

/** Pack pairs (in the order given) into requests of at most PAIRS_PER_REQUEST pairs and REQUEST_CHAR_CAP characters. */
export function packPairs(pairs, ctx, { perRequest = PAIRS_PER_REQUEST, charCap = REQUEST_CHAR_CAP } = {}) {
  const reqs = [];
  let cur = null;
  for (const p of pairs) {
    const len = pairMessage(p, 0, ctx).length;
    if (!cur || cur.pairs.length >= perRequest || cur.chars + len > charCap) { cur = { id: `r${reqs.length + 1}`, pairs: [], chars: 0 }; reqs.push(cur); }
    cur.pairs.push(p); cur.chars += len;
  }
  for (const r of reqs) { r.user = userMessage(r.pairs, ctx); r.estTokens = estimateTokens(SYSTEM) + estimateTokens(r.user); }
  return reqs;
}

const userMessage = (pairs, ctx) => `Judge each pair.\n\n${pairs.map((p, i) => pairMessage(p, i + 1, ctx)).join('\n\n')}`;
const requestUsd = (model, r) => costFromUsage(model, { input_tokens: r.estTokens, output_tokens: r.pairs.length * OUT_PER_PAIR + THINKING_PER_REQUEST });

async function ask(client, { model, effort, user }) {
  const params = {
    model, max_tokens: 16000, thinking: { type: 'adaptive' },
    output_config: { effort, format: { type: 'json_schema', schema: SCHEMA } },
    system: SYSTEM, messages: [{ role: 'user', content: wellFormed(user) }],
  };
  return client.messages.stream ? client.messages.stream(params).finalMessage() : client.messages.create(params);
}

function validEntry(o) {
  return o && ['same', 'related', 'different'].includes(o.verdict) && ['confident', 'doubtful'].includes(o.confidence) && typeof o.reason === 'string' && squash(o.reason);
}

/** Judge one request's pairs. A failed or unparseable request is split in halves down to one pair; a bad entry becomes an error line. */
async function judge(client, cfg, pairs, ctx, tally) {
  const out = [];
  const user = userMessage(pairs, ctx);
  let parsed = null, err = null;
  try {
    const msg = await ask(client, { ...cfg, user });
    tally.requests++;
    const usage = msg.usage || {};
    tally.usage = addUsage(tally.usage, usage);
    tally.cost += costFromUsage(cfg.model, usage);
    if (msg.stop_reason === 'refusal' || msg.stop_reason === 'max_tokens') throw new Error(msg.stop_reason);
    parsed = JSON.parse(textOf(msg));
    if (!parsed || !Array.isArray(parsed.results)) throw new Error('reply has no results array');
  } catch (e) { err = e.message; }
  if (err) {
    if (pairs.length === 1) return [{ pair: pairs[0], error: `request failed: ${err}` }];
    const mid = Math.ceil(pairs.length / 2);
    return [...await judge(client, cfg, pairs.slice(0, mid), ctx, tally), ...await judge(client, cfg, pairs.slice(mid), ctx, tally)];
  }
  const byId = new Map();
  for (const r of parsed.results) if (r && typeof r.id === 'string') byId.set(r.id, r);
  pairs.forEach((p, i) => {
    const r = byId.get(`p${i + 1}`);
    if (!r) out.push({ pair: p, error: 'no verdict returned for this pair' });
    else if (!validEntry(r)) out.push({ pair: p, error: `malformed verdict: ${JSON.stringify(r).slice(0, 160)}` });
    else out.push({ pair: p, verdict: r.verdict, confidence: r.confidence, reason: clip(squash(r.reason), 160) });
  });
  return out;
}

export async function proposeMerges(o) {
  const A = o.archiveRoot;
  const log = o.log || (() => {});
  if (!o.runId || !/^[\w.-]+$/.test(o.runId)) throw new Error('events-propose-merges needs --run-id (letters, digits, . _ -)');
  const minItems = o.minItems ?? 2;
  const model = o.model;
  const dir = join(A, 'analysis/events/merge-proposals');
  const outPath = join(dir, `${o.runId}.jsonl`);
  const ledgerPath = join(dir, `${o.runId}-ledger.json`);
  if (!o.dryRun && (existsSync(outPath) || existsSync(ledgerPath))) throw new Error(`${outPath} already exists; pick a new --run-id`);
  const events = (o.readEvents || scoutReader())();
  const rows = readCurrent(A);
  const cand = buildCandidates({ events, rows, minItems, archiveTop: o.archiveTop });
  const ctx = { profiles: cand.profiles, rows: cand.rows, loadText: (o.loadText || itemLoader(A)) };
  const reqs = packPairs(cand.pairs, ctx);
  const estimate = reqs.reduce((n, r) => n + requestUsd(model, r), 0) * 1.25;
  const summary = {
    kind: 'events-propose-merges', run_id: o.runId, model, prompt_version: MERGE_PROMPT_VERSION, min_items: minItems,
    events_considered: cand.stats.events_considered, archive_frequent_entities: cand.stats.archive_frequent_entities,
    candidates_by_signal: cand.stats.by_signal, candidate_pairs: cand.stats.candidate_pairs,
    same_month_dropped: cand.stats.same_month_dropped, dropped_by_partner_cap: cand.stats.dropped_by_partner_cap,
    kept_pairs: cand.stats.kept_pairs, kept_by_signal: cand.stats.kept_by_signal,
    requests: reqs.length, estimate_usd: +estimate.toFixed(2),
  };
  if (o.dryRun) return { ...summary, dry_run: true, examples: cand.pairs, estimate_basis: 'chars/3 input tokens, 90 output + 2500 thinking tokens per request (Sonnet price), x1.25 margin' };

  const tally = { requests: 0, usage: { input_tokens: 0, output_tokens: 0 }, cost: 0 };
  const results = [];
  let stopped = false;
  const cfg = { model, effort: o.effort || 'medium' };
  await runPool(reqs, o.concurrency || 3, async (r) => {
    if (stopped || tally.cost + requestUsd(model, r) > o.maxUsd) { stopped = true; return null; }
    return judge(o.client, cfg, r.pairs, ctx, tally);
  }, (r, res) => {
    if (!res) return;
    results.push(...res);
    if (tally.cost >= o.maxUsd) stopped = true;
    log(`${results.length}/${cand.pairs.length} pairs judged, $${tally.cost.toFixed(3)}`);
  });

  const now = (o.now || new Date()).toISOString();
  const line = (x) => {
    const p = x.pair;
    const base = {
      a: p.a.id, b: p.b.id, a_name: p.a.name, b_name: p.b.name, a_items: p.a.items, b_items: p.b.items,
      a_span: [p.a.start, p.a.end], b_span: [p.b.start, p.b.end], signals: p.signals,
    };
    const tail = { model, prompt_version: MERGE_PROMPT_VERSION, judged_at: now };
    return x.error ? { ...base, error: x.error, ...tail } : { ...base, verdict: x.verdict, confidence: x.confidence, reason: x.reason, ...tail };
  };
  const lines = results.map(line).sort((x, y) => x.a.localeCompare(y.a) || x.b.localeCompare(y.b));
  const count = (f) => lines.filter(f).length;
  const ledger = {
    ...summary, started_at: now, max_usd: o.maxUsd, stopped_budget: stopped && results.length < cand.pairs.length,
    pairs_judged: lines.length, pairs_not_judged: cand.pairs.length - lines.length, errors: count((l) => l.error),
    verdicts: Object.fromEntries(['same', 'related', 'different'].map((v) => [v, Object.fromEntries(['confident', 'doubtful'].map((c) => [c, count((l) => l.verdict === v && l.confidence === c)]))])),
    merge_proposals: { confident: count((l) => l.verdict === 'same' && l.confidence === 'confident'), doubtful: count((l) => l.verdict === 'same' && l.confidence === 'doubtful') },
    model_requests: tally.requests, usage: tally.usage, cost_usd_total: +tally.cost.toFixed(4),
  };
  mkdirSync(dir, { recursive: true });
  writeFileSync(outPath, lines.map((l) => JSON.stringify(l)).join('\n') + (lines.length ? '\n' : ''));
  writeFileSync(ledgerPath, JSON.stringify(ledger, null, 2) + '\n');
  return { ...ledger, output: outPath, ledger: ledgerPath };
}

export { SIGNALS };

// Model calls for v2: one-off JSON calls (briefing, themes) and batched tagging over a cached system prompt.
import { addUsage, costFromUsage } from './cost.js';
import { buildTagParams } from './tag-prompt.js';
import { validateTagBatch } from './tag-results.js';
import { textOf, runPool } from './model.js';
import { wellFormed } from './text.js';

// Streaming JSON call (large outputs). Returns {parsed, usage, cost, stop_reason}.
export async function callJson(client, { model, effort = 'medium', system, user, schema, maxTokens = 32000 }) {
  const params = {
    model, max_tokens: maxTokens, thinking: { type: 'adaptive' },
    output_config: { effort, format: { type: 'json_schema', schema } },
    system: typeof system === 'string' ? wellFormed(system) : system, messages: [{ role: 'user', content: wellFormed(user) }],
  };
  const msg = client.messages.stream ? await client.messages.stream(params).finalMessage() : await client.messages.create(params);
  if (msg.stop_reason === 'max_tokens' || msg.stop_reason === 'refusal') throw new Error(`${model} stopped: ${msg.stop_reason}`);
  const usage = msg.usage || {};
  return { parsed: JSON.parse(textOf(msg)), usage, cost: costFromUsage(model, usage), stop_reason: msg.stop_reason };
}

const empty = () => ({ records: [], failed: [], usage: { input_tokens: 0, output_tokens: 0 }, cost: 0, problems: [], requests: [] });

function merge(a, b) {
  return {
    records: [...a.records, ...b.records], failed: [...a.failed, ...b.failed],
    usage: addUsage(a.usage, b.usage), cost: a.cost + b.cost,
    problems: [...a.problems, ...b.problems], requests: [...a.requests, ...b.requests],
  };
}

/**
 * Tag one batch; missing items are retried once as a group, a failed request is split in halves
 * down to single items. cfg: {model, effort, system, vctx}. Every request's usage is kept in .requests.
 */
export async function tagBatchV2(client, cfg, batch, depth = 0) {
  const out = empty();
  let msg;
  try {
    msg = await client.messages.create(buildTagParams({ ...cfg, batch }));
  } catch (e) {
    if (batch.items.length === 1) return { ...out, failed: [batch.items[0].key], problems: [`${batch.id}: ${e.message}`] };
    return splitRetry(client, cfg, batch, depth, `${batch.id}: split after ${e.message}`);
  }
  const usage = msg.usage || {};
  out.usage = addUsage(out.usage, usage);
  out.cost = costFromUsage(cfg.model, usage);
  out.requests.push({ id: batch.id, items: batch.items.length, usage });
  let parsed;
  try {
    if (msg.stop_reason === 'refusal' || msg.stop_reason === 'max_tokens') throw new Error(msg.stop_reason);
    parsed = JSON.parse(textOf(msg));
  } catch (e) {
    if (batch.items.length === 1) return { ...out, failed: [batch.items[0].key], problems: [`${batch.id}: ${e.message}`] };
    return merge(out, await splitRetry(client, cfg, batch, depth, `${batch.id}: split after ${e.message}`));
  }
  const v = validateTagBatch(parsed, batch, cfg.vctx);
  out.records = v.records;
  out.problems = v.problems;
  if (!v.missing.length) return out;
  if (depth >= 2 || batch.items.length === 1) return { ...out, failed: v.missing };
  const retry = await tagBatchV2(client, cfg, { id: `${batch.id}r`, items: batch.items.filter((i) => v.missing.includes(i.key)) }, depth + 1);
  return merge(out, retry);
}

async function splitRetry(client, cfg, batch, depth, why) {
  const mid = Math.ceil(batch.items.length / 2);
  const a = await tagBatchV2(client, cfg, { id: `${batch.id}a`, items: batch.items.slice(0, mid) }, depth + 1);
  const b = await tagBatchV2(client, cfg, { id: `${batch.id}b`, items: batch.items.slice(mid) }, depth + 1);
  const m = merge(a, b);
  m.problems.unshift(why);
  return m;
}

/**
 * Run all batches: the first alone (it writes the cache; parallel requests cannot read an entry that is
 * still being written), then the rest in a pool. Stops sending once spend passes maxUsd.
 */
export async function tagAll(client, cfg, batches, { concurrency = 4, maxUsd = Infinity, log = () => {} } = {}) {
  let total = empty();
  let stopped = false;
  const take = (r) => { total = merge(total, r); if (total.cost > maxUsd) stopped = true; };
  if (!batches.length) return { ...total, stopped };
  take(await tagBatchV2(client, cfg, batches[0]));
  const first = total.requests[0] && total.requests[0].usage;
  log(`first request: cache write ${first?.cache_creation_input_tokens || 0}, cache read ${first?.cache_read_input_tokens || 0}, uncached ${first?.input_tokens || 0}`);
  let done = 1;
  await runPool(batches.slice(1), concurrency, async (b) => {
    if (stopped) return { ...empty(), failed: b.items.map((i) => i.key), problems: [`${b.id}: not sent, spend cap reached`] };
    return tagBatchV2(client, cfg, b);
  }, (b, r) => {
    take(r);
    if (++done % 10 === 0 || done === batches.length) log(`${done}/${batches.length} requests, ${total.records.length} items, $${total.cost.toFixed(3)}`);
  });
  return { ...total, stopped };
}

// Cache statistics over request usages. hit rate = cache reads / all prompt tokens.
export function cacheStats(requests) {
  let read = 0, write = 0, uncached = 0, hits = 0;
  for (const r of requests) {
    const u = r.usage || {};
    read += u.cache_read_input_tokens || 0;
    write += u.cache_creation_input_tokens || 0;
    uncached += u.input_tokens || 0;
    if ((u.cache_read_input_tokens || 0) > 0) hits++;
  }
  const all = read + write + uncached;
  return {
    requests: requests.length, requests_with_cache_read: hits,
    cache_read_input_tokens: read, cache_creation_input_tokens: write, uncached_input_tokens: uncached,
    token_hit_rate: all ? +(read / all).toFixed(4) : 0,
    request_hit_rate: requests.length ? +(hits / requests.length).toFixed(4) : 0,
    max_prompt_tokens: Math.max(0, ...requests.map((r) => (r.usage.input_tokens || 0) + (r.usage.cache_read_input_tokens || 0) + (r.usage.cache_creation_input_tokens || 0))),
  };
}

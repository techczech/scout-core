// One batch -> one validated result, via the Messages API (direct or Batches).
import { buildParams } from './prompts.js';
import { validateBatch } from './results.js';
import { addUsage, costFromUsage } from './cost.js';

export function textOf(message) {
  const b = (message.content || []).find((c) => c.type === 'text');
  return b ? b.text : '';
}

export function parseMessage(message) {
  if (message.stop_reason === 'refusal') throw new Error('refusal');
  if (message.stop_reason === 'max_tokens') throw new Error('max_tokens');
  return JSON.parse(textOf(message));
}

// Tag one batch; on failure split it in halves (down to single items) so one bad item
// cannot sink twenty. Returns {records, failed:[keys], usage, problems}.
export async function tagBatch(client, cfg, batch) {
  let usage = { input_tokens: 0, output_tokens: 0 };
  let cost = 0; // priced per request (the >100K price step applies per prompt, not per run)
  try {
    const msg = await client.messages.create(buildParams({ ...cfg, batch }));
    usage = addUsage(usage, msg.usage || {});
    cost += costFromUsage(cfg.model, msg.usage || {});
    const out = validateBatch(parseMessage(msg), batch, { vocabulary: cfg.vocabulary });
    if (out.missing.length === 0) return { records: out.records, failed: [], usage, cost, problems: out.problems };
    if (batch.items.length === 1) return { records: out.records, failed: out.missing, usage, cost, problems: out.problems };
    // retry only the missing items
    const retry = await tagBatch(client, cfg, { id: batch.id + 'r', items: batch.items.filter((i) => out.missing.includes(i.key)) });
    return { records: [...out.records, ...retry.records], failed: retry.failed, usage: addUsage(usage, retry.usage), cost: cost + retry.cost, problems: [...out.problems, ...retry.problems] };
  } catch (e) {
    if (batch.items.length === 1) return { records: [], failed: [batch.items[0].key], usage, cost, problems: [`${batch.id}: ${e.message}`] };
    const mid = Math.ceil(batch.items.length / 2);
    const a = await tagBatch(client, cfg, { id: batch.id + 'a', items: batch.items.slice(0, mid) });
    const b = await tagBatch(client, cfg, { id: batch.id + 'b', items: batch.items.slice(mid) });
    return {
      records: [...a.records, ...b.records],
      failed: [...a.failed, ...b.failed],
      usage: addUsage(addUsage(usage, a.usage), b.usage),
      cost: cost + a.cost + b.cost,
      problems: [`${batch.id}: split after ${e.message}`, ...a.problems, ...b.problems],
    };
  }
}

// Small worker pool; calls onDone(batch, result) as each finishes.
export async function runPool(batches, concurrency, work, onDone) {
  let next = 0;
  const workers = Array.from({ length: Math.min(concurrency, batches.length) }, async () => {
    while (next < batches.length) {
      const b = batches[next++];
      onDone(b, await work(b));
    }
  });
  await Promise.all(workers);
}

// ---- Message Batches API (50% off) ----
export async function submitBatches(client, cfg, batches) {
  const requests = batches.map((b) => ({ custom_id: b.id, params: buildParams({ ...cfg, batch: b }) }));
  const mb = await client.messages.batches.create({ requests });
  return mb.id;
}

export async function waitForBatches(client, id, { pollMs = 60000, log = () => {} } = {}) {
  for (;;) {
    const b = await client.messages.batches.retrieve(id);
    if (b.processing_status === 'ended') return b;
    log(`batch ${id}: ${b.processing_status}, processing ${b.request_counts.processing}`);
    await new Promise((r) => setTimeout(r, pollMs));
  }
}

// Collect results into the same shape as tagBatch; failed requests list their items as failed.
export async function collectBatches(client, id, batches, cfg) {
  const byId = new Map(batches.map((b) => [b.id, b]));
  const out = { records: [], failed: [], usage: { input_tokens: 0, output_tokens: 0 }, cost: 0, problems: [] };
  const seen = new Set();
  for await (const r of await client.messages.batches.results(id)) {
    const batch = byId.get(r.custom_id);
    if (!batch) continue;
    seen.add(r.custom_id);
    if (r.result.type !== 'succeeded') {
      out.failed.push(...batch.items.map((i) => i.key));
      out.problems.push(`${r.custom_id}: ${r.result.type}`);
      continue;
    }
    const msg = r.result.message;
    out.usage = addUsage(out.usage, msg.usage || {});
    out.cost += costFromUsage(cfg.model, msg.usage || {}, { batch: true });
    try {
      const v = validateBatch(parseMessage(msg), batch, { vocabulary: cfg.vocabulary });
      out.records.push(...v.records);
      out.failed.push(...v.missing);
      out.problems.push(...v.problems);
    } catch (e) {
      out.failed.push(...batch.items.map((i) => i.key));
      out.problems.push(`${r.custom_id}: ${e.message}`);
    }
  }
  for (const b of batches) if (!seen.has(b.id)) out.failed.push(...b.items.map((i) => i.key));
  return out;
}

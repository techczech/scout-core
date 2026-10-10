// Turn free-form discovery labels into a proposed vocabulary, in two small calls:
// (1) propose topics + definitions from the frequent labels; (2) file every label under them in chunks.
import { ASSIGN_SCHEMA, ASSIGN_SYSTEM, VOCAB_SCHEMA, VOCAB_SYSTEM } from './prompts.js';
import { applyVocabulary } from './results.js';
import { textOf, runPool } from './model.js';
import { addUsage, costFromUsage } from './cost.js';

export const ASSIGN_CHUNK = 200;
export const PROPOSE_MIN_COUNT = 2;
export const PROPOSE_MAX_LABELS = 1200;

export function labelCounts(records) {
  const m = new Map();
  for (const r of records) for (const t of r.topics) m.set(t, (m.get(t) || 0) + 1);
  return [...m.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
}

// Labels shown when proposing: all with count >= 2, topped up with singletons to at least 300.
export function proposalLabels(counts) {
  const multi = counts.filter(([, n]) => n >= PROPOSE_MIN_COUNT);
  const pick = multi.length >= 300 ? multi : counts.slice(0, 300);
  return pick.slice(0, PROPOSE_MAX_LABELS);
}

export function validateProposal(parsed) {
  const names = new Set();
  const topics = [];
  for (const t of (parsed && parsed.topics) || []) {
    const name = String(t.topic || '').replace(/\s+/g, ' ').trim();
    if (!name || names.has(name)) continue;
    names.add(name);
    topics.push({ topic: name, definition: String(t.definition || '').trim() });
  }
  return topics;
}

// chunk: array of labels (numbered from 1). Returns {mapping:{label:topic}, bad:count}.
export function validateAssignment(parsed, chunk, topics) {
  const names = new Set(topics.map((t) => t.topic));
  const mapping = {};
  let bad = 0;
  for (const a of (parsed && parsed.assignments) || []) {
    const label = chunk[a.n - 1];
    if (label === undefined || mapping[label]) { bad++; continue; }
    if (!names.has(a.topic)) { bad++; continue; }
    mapping[label] = a.topic;
  }
  return { mapping, bad };
}

async function call(client, model, system, schema, content, usage, effort = 'low', maxTokens = 16000) {
  const msg = await client.messages.create({
    model, max_tokens: maxTokens, thinking: { type: 'adaptive' },
    output_config: { effort, format: { type: 'json_schema', schema } },
    system, messages: [{ role: 'user', content }],
  });
  if (msg.stop_reason === 'max_tokens' || msg.stop_reason === 'refusal') throw new Error(`consolidation ${msg.stop_reason}`);
  usage.total = addUsage(usage.total, msg.usage || {});
  usage.cost += costFromUsage(model, msg.usage || {});
  return JSON.parse(textOf(msg));
}

export async function consolidate(client, { model, records, concurrency = 4, log = () => {} }) {
  const counts = labelCounts(records);
  const labels = counts.map(([l]) => l);
  const usage = { total: { input_tokens: 0, output_tokens: 0 }, cost: 0 };
  const shown = proposalLabels(counts);
  const proposal = validateProposal(await call(client, model, VOCAB_SYSTEM, VOCAB_SCHEMA,
    `Labels (count<TAB>label):\n${shown.map(([l, n]) => `${n}\t${l}`).join('\n')}`, usage));
  if (proposal.length < 5) throw new Error(`proposal too small (${proposal.length} topics)`);
  log(`proposed ${proposal.length} topics from ${shown.length} labels; filing ${labels.length} labels`);
  const list = proposal.map((t) => `- ${t.topic}: ${t.definition}`).join('\n');
  const mapping = {};
  let bad = 0;
  // Pass 1 files all labels; pass 2 retries whatever the first pass missed or got wrong.
  for (let pass = 1; pass <= 2; pass++) {
    const todo = labels.filter((l) => !mapping[l]);
    const chunks = [];
    for (let i = 0; i < todo.length; i += ASSIGN_CHUNK) chunks.push(todo.slice(i, i + ASSIGN_CHUNK));
    bad = 0;
    await runPool(chunks.map((c, i) => ({ id: i, c })), concurrency, async ({ c }) => {
      const content = `Vocabulary:\n${list}\n\nLabels:\n${c.map((l, i) => `${i + 1}. ${l}`).join('\n')}`;
      return validateAssignment(await call(client, model, ASSIGN_SYSTEM, ASSIGN_SCHEMA, content, usage), c, proposal);
    }, (_, r) => { Object.assign(mapping, r.mapping); bad += r.bad; });
    log(`pass ${pass}: ${labels.filter((l) => !mapping[l]).length} labels still unfiled`);
  }
  const unmapped = labels.filter((l) => !mapping[l]);
  const applied = applyVocabulary(records, proposal, mapping);
  const mergedLabels = {};
  for (const [l, t] of Object.entries(mapping)) (mergedLabels[t] ||= []).push(l);
  return { usage: usage.total, cost: usage.cost, labelCount: labels.length, topics: proposal, mapping, unmapped, badAssignments: bad, mergedLabels, rows: applied.topics };
}

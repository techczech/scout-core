import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { executeRun } from '../lib/run.js';

function fixture(n) {
  const root = mkdtempSync(join(tmpdir(), 'tt-run-'));
  const dir = join(root, 'readings/works');
  mkdirSync(dir, { recursive: true });
  for (let i = 0; i < n; i++) {
    writeFileSync(join(dir, `w${i}.md`), `---\ntitle: T${i}\nauthor: A${i}\ntype: tweet\nsource_system: x\nsource_id: "${i}"\nurl: https://x.com/${i}\n---\n\n> item ${i} ${i % 2 ? 'about Claude Code' : 'about gardening'}\n\nhighlighted_at: 2026-09-${String(10 + (i % 5))}\n`);
  }
  return root;
}

// Fake client: answers per item from its text; consolidation merges everything into one topic.
const fakeClient = (calls) => ({
  messages: {
    create: async (p) => {
      calls.push(p);
      if (p.system.includes('controlled topic vocabulary')) {
        const topics = ['Claude Code', 'T2', 'T3', 'T4', 'T5'].map((t) => ({ topic: t, definition: `def ${t}` }));
        return { stop_reason: 'end_turn', usage: { input_tokens: 500, output_tokens: 100 }, content: [{ type: 'text', text: JSON.stringify({ topics }) }] };
      }
      if (p.system.includes('file free-form topic labels')) {
        const labels = [...p.messages[0].content.matchAll(/^(\d+)\. (.*)$/gm)];
        const assignments = labels.map(([, n, l]) => ({ n: Number(n), topic: l === 'Claude Code' ? 'Claude Code' : '' }));
        return { stop_reason: 'end_turn', usage: { input_tokens: 500, output_tokens: 100 }, content: [{ type: 'text', text: JSON.stringify({ assignments }) }] };
      }
      const ids = [...p.messages[0].content.matchAll(/<item id="(i\d+)">\n([\s\S]*?)\n<\/item>/g)];
      const results = ids.map(([, id, body]) => ({ id, ai_related: /Claude/.test(body), topics: /Claude/.test(body) ? ['Claude Code'] : [], note: '' }));
      return { stop_reason: 'end_turn', usage: { input_tokens: 1000, output_tokens: 200 }, content: [{ type: 'text', text: JSON.stringify({ results }) }] };
    },
  },
});

const base = (root, extra = {}) => ({
  archiveRoot: root, from: '2026-09-01', to: '2026-09-30', runId: 'r1', mode: 'discovery', model: 'claude-haiku-5-5',
  effort: 'low', consolidateModel: 'claude-haiku-5-5', maxUsd: 3, maxItems: 4, maxTokens: 25000, concurrency: 2, batchApi: false, ...extra,
});

test('dry run sends nothing and writes nothing', async () => {
  const root = fixture(6);
  const r = await executeRun(base(root, { dryRun: true }), { client: null });
  assert.equal(r.items.length, 6);
  assert.equal(existsSync(join(root, 'analysis')), false);
});

test('cost guard aborts before any request', async () => {
  const root = fixture(6);
  const calls = [];
  await assert.rejects(executeRun(base(root, { maxUsd: 0.0000001 }), { client: fakeClient(calls) }), /exceeds --max-usd/);
  assert.equal(calls.length, 0);
});

test('discovery run writes the layer and refuses to overwrite', async () => {
  const root = fixture(6);
  const calls = [];
  const { run, runDir } = await executeRun(base(root), { client: fakeClient(calls) });
  assert.equal(calls.length, 4); // 2 tagging + proposal + one assignment chunk
  assert.equal(calls[0].model, 'claude-haiku-5-5');
  assert.equal(calls[0].output_config.effort, 'low');
  assert.equal(calls[0].output_config.format.type, 'json_schema');
  const lines = readFileSync(join(runDir, 'tags.jsonl'), 'utf8').trim().split('\n').map((l) => JSON.parse(l));
  assert.equal(lines.length, 6);
  assert.equal(lines.filter((l) => l.ai_related).length, 3);
  assert.equal(run.item_count, 6);
  assert.equal(run.ai_count, 3);
  assert.ok(run.cost_usd_total > 0);
  assert.ok(run.prompt_text.includes('ai_related'));
  const vocab = JSON.parse(readFileSync(join(runDir, 'vocabulary.json'), 'utf8'));
  assert.equal(vocab.topics[0].count, 3);
  assert.equal(vocab.topics[0].examples.length, 3);
  assert.equal(vocab.topics[0].topic, 'Claude Code');
  assert.ok(readFileSync(join(runDir, 'review.html'), 'utf8').includes('Claude Code'));
  await assert.rejects(executeRun(base(root), { client: fakeClient([]) }), /already exists/);
});

test('vocabulary mode requires picking from the vocabulary in the prompt', async () => {
  const root = fixture(4);
  const vp = join(root, 'v.json');
  writeFileSync(vp, JSON.stringify({ topics: [{ topic: 'Claude Code', definition: 'The tool' }] }));
  const calls = [];
  const { run } = await executeRun(base(root, { mode: 'vocabulary', vocabularyPath: vp, runId: 'r2' }), { client: fakeClient(calls) });
  assert.ok(calls[0].system.includes('- Claude Code: The tool'));
  assert.equal(run.vocabulary_source.topics, 1);
  assert.equal(existsSync(join(root, 'analysis/topics/r2/vocabulary.json')), false);
});

test('consolidate-only re-runs the vocabulary step on a finished tagging pass', async () => {
  const root = fixture(6);
  const { runDir } = await executeRun(base(root, { runId: 'r3' }), { client: fakeClient([]) });
  const before = readFileSync(join(runDir, 'tags.jsonl'), 'utf8');
  const { consolidateRun } = await import('../lib/run.js');
  const r = await consolidateRun(base(root, { runId: 'r3' }), { client: fakeClient([]) });
  assert.equal(readFileSync(join(runDir, 'tags.jsonl'), 'utf8'), before);
  assert.equal(r.run.vocabulary_topics, 5);
});

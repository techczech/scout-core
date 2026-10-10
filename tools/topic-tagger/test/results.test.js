import { test } from 'node:test';
import assert from 'node:assert/strict';
import { validateBatch, applyVocabulary, summarise } from '../lib/results.js';
import { validateProposal, validateAssignment, labelCounts, proposalLabels } from '../lib/consolidate.js';
import { renderReview } from '../lib/review.js';

const item = (k) => ({ key: k, path: `readings/works/${k}.md`, date: '2026-09-01', type: 'tweet', text: 't' });
const batch = { id: 'b1', items: [item('x:1'), item('x:2'), item('x:3')] };

test('discovery: validates, dedupes, caps topics and note length', () => {
  const parsed = { results: [
    { id: 'i1', ai_related: true, topics: ['vibe coding', ' vibe coding ', 'a', 'b', 'c', 'd'], note: 'one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen' },
    { id: 'i2', ai_related: false, topics: ['ignored'], note: '' },
  ] };
  const r = validateBatch(parsed, batch);
  assert.equal(r.records.length, 2);
  assert.deepEqual(r.records[0].topics, ['vibe coding', 'a', 'b', 'c']);
  assert.equal(r.records[0].note.split(' ').length, 15);
  assert.deepEqual(r.records[1].topics, []);
  assert.equal('note' in r.records[1], false);
  assert.deepEqual(r.missing, ['x:3']);
  assert.deepEqual(Object.keys(r.records[0]), ['key', 'path', 'date', 'type', 'ai_related', 'topics', 'note']);
});

test('unknown ids, bad flags and AI items without topics are reported', () => {
  const r = validateBatch({ results: [
    { id: 'zz', ai_related: true, topics: ['a'] },
    { id: 'i1', ai_related: 'yes', topics: [] },
    { id: 'i2', ai_related: true, topics: [] },
    { id: 'i3', ai_related: true, topics: ['ok'] },
  ] }, batch);
  assert.deepEqual(r.records.map((x) => x.key), ['x:3']);
  assert.deepEqual(r.missing.sort(), ['x:1', 'x:2']);
  assert.ok(r.problems.length >= 3);
  assert.equal(validateBatch(null, batch).missing.length, 3);
});

test('vocabulary mode: only vocabulary topics plus at most one new:', () => {
  const vocabulary = { topics: [{ topic: 'vibe coding' }, { topic: 'Claude Code' }] };
  const r = validateBatch({ results: [
    { id: 'i1', ai_related: true, topics: ['vibe coding', 'made up', 'new: agent memory', 'new: second new'], note: '' },
  ] }, { id: 'b', items: [item('x:1')] }, { vocabulary });
  assert.deepEqual(r.records[0].topics, ['vibe coding', 'new: agent memory']);
  assert.equal(r.problems.length, 2);
});

test('consolidation: proposal dedupe, chunk assignment validation, counts and examples', () => {
  const recs = [
    { key: 'a', topics: ['vibe coding', 'Claude Code'], ai_related: true },
    { key: 'b', topics: ['vibe-coding'], ai_related: true },
    { key: 'c', topics: ['odd one'], ai_related: true },
  ];
  const counts = labelCounts(recs);
  assert.equal(counts.length, 4);
  assert.equal(proposalLabels(counts).length, 4); // fewer than 300 multi-use labels: top up with singletons
  const topics = validateProposal({ topics: [
    { topic: 'Vibe coding', definition: 'd1' }, { topic: ' Vibe coding ', definition: 'dup' }, { topic: 'Claude Code', definition: 'd2' }, { topic: '', definition: 'x' },
  ] });
  assert.equal(topics.length, 2);
  const chunk = ['vibe coding', 'vibe-coding', 'Claude Code', 'odd one'];
  const { mapping, bad } = validateAssignment({ assignments: [
    { n: 1, topic: 'Vibe coding' }, { n: 2, topic: 'Vibe coding' }, { n: 3, topic: 'Claude Code' },
    { n: 4, topic: 'Not in vocabulary' }, { n: 9, topic: 'Claude Code' }, { n: 1, topic: 'Claude Code' },
  ] }, chunk, topics);
  assert.equal(bad, 3);
  assert.equal(mapping['odd one'], undefined);
  const a = applyVocabulary(recs, topics, mapping);
  assert.equal(a.topics[0].topic, 'Vibe coding');
  assert.equal(a.topics[0].count, 2);
  assert.deepEqual(a.topics[0].examples, ['a', 'b']);
  assert.deepEqual(a.unmapped, { 'odd one': 1 });
  assert.equal(summarise(recs).ai, 3);
});

test('review page escapes content, has dark mode and no external requests', () => {
  const html = renderReview({
    run: { run_id: 'r', mode: 'discovery', model: 'm', effort: 'low', prompt_version: 'p', selection_rule: 's', item_count: 4, ai_count: 2, cost_usd_total: 0.1234 },
    rows: [{ topic: 'T <b>', definition: 'd', count: 2, examples: ['a'] }],
    examples: { a: { author: 'au', date: '2026-09-01', url: 'https://x.com/a', snippet: '<script>x</script>' } },
  });
  assert.ok(html.includes('prefers-color-scheme:dark'));
  assert.ok(html.includes('&lt;script&gt;'));
  assert.ok(!html.includes('<script>'));
  assert.ok(!/src=|<link /.test(html));
  assert.ok(html.includes('50%'));
});

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { assembleSystem, fitPrompt, renderItem, buildTagParams, systemText, PROMPT_VERSION, TAG_SCHEMA, BLOCK_ORDER } from '../lib/tag-prompt.js';
import { assignShortIds } from '../lib/month-index.js';

const fixed = (n = 1) => ({
  instructions: 'Instructions.',
  reader_profile: 'P'.repeat(3000 * n),
  themes: 'T'.repeat(6000),
  month_briefing: 'B'.repeat(15000),
  reader_tweets: 'W'.repeat(30000),
  mondai_roundup: 'R'.repeat(20000),
});
const monthItems = (n) => assignShortIds(Array.from({ length: n }, (_, i) => ({
  key: `x:${i}`, date: '2026-09-10', type: 'tweet', author: `a${i}`, title: '', highlights: [{ text: `item ${i} ${'word '.repeat(30)}` }],
})));

test('prompt version is bumped to v2', () => assert.equal(PROMPT_VERSION, 'topics-v2'));

test('system blocks: fixed order, XML-wrapped, one cache breakpoint on the last block', () => {
  const s = assembleSystem({ month_index: 'idx', themes: 'th', instructions: 'ins', reader_profile: 'prof' });
  assert.deepEqual(s.map((b) => b.text.split('\n')[0]), ['ins', '<reader_profile>', '<themes>', '<month_index>']);
  assert.equal(s.filter((b) => b.cache_control).length, 1);
  assert.deepEqual(s[s.length - 1].cache_control, { type: 'ephemeral' });
  assert.equal(BLOCK_ORDER[BLOCK_ORDER.length - 1], 'month_index');
});

test('fitPrompt keeps system + largest batch under the 100K cap, shrinking only the month index', async () => {
  const items = monthItems(1600);
  const r = await fitPrompt(fixed(), items, { userReserve: 14000 });
  assert.ok(r.systemTokens + 14000 <= 100000);
  assert.ok(r.index.gistChars < 100, 'index had to shrink');
  assert.equal(r.index.lines, 1600);
  assert.ok(r.sizes.find((s) => s.block === 'month_index'));
  assert.ok(systemText(r.system).includes('<reader_tweets>'));
});

test('fitPrompt re-shrinks when the real counter reports more tokens than the estimate', async () => {
  const items = monthItems(1600);
  let calls = 0;
  const count = async (sys) => { calls++; return Math.ceil(systemText(sys).length / 1.8); }; // denser than the chars/2.2 estimate
  const r = await fitPrompt(fixed(), items, { count, userReserve: 14000 });
  assert.ok(r.systemTokens + 14000 + 1500 <= 100000);
  assert.ok(calls >= 3);
});

test('fitPrompt refuses when the fixed context leaves no room', async () => {
  await assert.rejects(() => fitPrompt(fixed(90), monthItems(10)), /no room for the month index/);
});

test('item rendering carries reply, quote, article and thread context with month ids', () => {
  const [a, b] = monthItems(2);
  const ctx = {
    parent: { handle: 'openai', text: 'Introducing Astra' }, quoted_missing: '5',
    linked: [{ title: 'Essay', author: 'Frank', text: 'Body' }], thread: [b.key],
  };
  const t = renderItem({ ...a, highlights: [{ text: 'x'.repeat(3000) + ' ![image](u)' }] }, ctx, new Map([[b.key, b.sid]]));
  assert.match(t, /^<item id="m0001">/);
  assert.match(t, /\[replying to @openai\]: Introducing Astra/);
  assert.match(t, /\[quoting: quoted tweet not in the archive\]/);
  assert.match(t, /\[linked article: Essay — Frank\]: Body/);
  assert.match(t, /\[same author same day\]: m0002/);
  assert.match(t, / \[…\]/); // text capped
});

test('request params: structured output schema, adaptive thinking, cached system', () => {
  const [a] = monthItems(1);
  const p = buildTagParams({ model: 'claude-haiku-5-5', effort: 'medium', system: assembleSystem({ instructions: 'i' }), batch: { items: [{ ...a, text: '<item id="m0001">x</item>' }] } });
  assert.equal(p.output_config.effort, 'medium');
  assert.equal(p.output_config.format.schema, TAG_SCHEMA);
  assert.deepEqual(TAG_SCHEMA.properties.results.items.required, ['id', 'ai_related', 'themes', 'entities', 'event', 'relevance', 'confidence']);
  assert.equal(p.messages[0].content, '<item id="m0001">x</item>');
  assert.ok(p.system[0].cache_control);
});

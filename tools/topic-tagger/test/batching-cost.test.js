import { test } from 'node:test';
import assert from 'node:assert/strict';
import { packItems } from '../lib/batching.js';
import { costFromUsage, estimateTokens, estimateRunUsd } from '../lib/cost.js';

const mk = (n, chars) => Array.from({ length: n }, (_, i) => ({ key: `k${i}`, text: 'x'.repeat(chars) }));

test('short items group by count', () => {
  const b = packItems(mk(45, 300), { maxItems: 20, maxTokens: 25000 });
  assert.deepEqual(b.map((x) => x.items.length), [20, 20, 5]);
});

test('long items group by token cap and stay under it', () => {
  const b = packItems(mk(30, 12000), { maxItems: 20, maxTokens: 25000 });
  for (const x of b) assert.ok(x.estTokens <= 25000);
  assert.ok(b.length > 3);
  assert.equal(b.reduce((n, x) => n + x.items.length, 0), 30);
});

test('an oversized item still gets its own request', () => {
  const b = packItems(mk(2, 90000), { maxItems: 20, maxTokens: 25000 });
  assert.equal(b.length, 2);
});

test('cost: haiku prices, batch half, long-prompt step', () => {
  const u = { input_tokens: 1e6, output_tokens: 1e6 };
  assert.equal(costFromUsage('claude-haiku-5-5', { input_tokens: 50000, output_tokens: 10000 }), 0.005 + 0.005);
  assert.equal(costFromUsage('claude-haiku-5-5', { input_tokens: 50000, output_tokens: 10000 }, { batch: true }), 0.005);
  assert.equal(costFromUsage('claude-haiku-5-5', { input_tokens: 200000, output_tokens: 0 }), 0.1);
  assert.throws(() => costFromUsage('nope', u));
});

test('estimate is chars/3 and sums to a run estimate', () => {
  assert.equal(estimateTokens('x'.repeat(300)), 100);
  const e = estimateRunUsd('claude-haiku-5-5', packItems(mk(20, 300)));
  assert.ok(e.usd > 0 && e.usd < 0.01);
});

test('run estimate prices each request on its own (no price step from summing)', () => {
  const batches = packItems(mk(400, 3000), { maxItems: 20 }); // 20 requests
  const e = estimateRunUsd('claude-haiku-5-5', batches);
  assert.ok(e.inTok > 100000);
  const one = estimateRunUsd('claude-haiku-5-5', [batches[0]]).usd;
  assert.ok(Math.abs(e.usd - one * 20) < 1e-9);
});

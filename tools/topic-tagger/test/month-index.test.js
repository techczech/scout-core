import { test } from 'node:test';
import assert from 'node:assert/strict';
import { assignShortIds, buildMonthIndex, gistOf, MIN_GIST } from '../lib/month-index.js';
import { condensedView } from '../lib/briefing.js';

const mk = (n) => Array.from({ length: n }, (_, i) => ({
  key: `x:${n - i}`, date: `2026-09-${String(1 + (i % 28)).padStart(2, '0')}`, type: i % 7 ? 'tweet' : 'article',
  author: `author ${i}`, title: `Title ${i}`, highlights: [{ text: `Tweet number ${i} about something ${'long '.repeat(40)} ![image](https://pbs.twimg.com/a.jpg)` }],
}));

test('short ids follow date then key, zero-padded, stable', () => {
  const a = assignShortIds(mk(30));
  const b = assignShortIds([...mk(30)].reverse());
  assert.deepEqual(a.map((x) => [x.sid, x.key]), b.map((x) => [x.sid, x.key]));
  assert.equal(a[0].sid, 'm0001');
  assert.ok(a.every((x, i) => i === 0 || a[i - 1].date <= x.date));
});

test('gist: tweets use text without images/links; articles use title then text', () => {
  const t = gistOf({ type: 'tweet', title: 't', highlights: [{ text: 'Hello ![image](https://x/y.jpg) 🔗 https://a.b/c world https://t.co/x' }] });
  assert.equal(t, 'Hello world');
  assert.equal(gistOf({ type: 'article', title: 'Essay', highlights: [{ text: 'first' }] }), 'Essay — first');
});

test('index: a heading per day, then "id [type] author: gist"; tweets unmarked; gist ≤ 100 chars', () => {
  const items = assignShortIds(mk(10));
  const idx = buildMonthIndex(items, { maxTokens: 1e6 });
  const lines = idx.text.split('\n');
  const heads = lines.filter((l) => l.startsWith('## '));
  const rows = lines.filter((l) => !l.startsWith('## '));
  assert.equal(rows.length, 10);
  assert.equal(heads.length, new Set(items.map((i) => i.date)).size);
  assert.match(lines[0], /^## 2026-09-01$/);
  assert.match(lines[1], /^m0001 \[article\] author_0: Title 0 — /);
  assert.match(rows.find((l) => l.startsWith('m0002')), /^m0002 author_\d+: Tweet number/);
  assert.equal(idx.gistChars, 100);
  for (const l of rows) assert.ok(l.split(': ').slice(1).join(': ').length <= 100);
});

test('index shrinks gists to fit the token cap, and throws when even the minimum does not fit', () => {
  const items = assignShortIds(mk(1600));
  const full = buildMonthIndex(items, { maxTokens: 1e7 });
  const capped = buildMonthIndex(items, { maxTokens: Math.floor(full.estTokens * 0.7) });
  assert.ok(capped.gistChars < 100);
  assert.ok(capped.estTokens <= Math.floor(full.estTokens * 0.7));
  assert.equal(capped.lines, 1600);
  assert.throws(() => buildMonthIndex(items, { maxTokens: 1000 }), new RegExp(`${MIN_GIST}-char`));
});

test('briefing view: one line per item, title only for non-tweets, text cut at 300', () => {
  const items = assignShortIds(mk(8));
  const v = condensedView(items).split('\n');
  assert.equal(v.length, 8);
  const art = v.find((l) => l.includes('| article |'));
  assert.match(art, /\| Title \d+ \|/);
  for (const l of v) assert.ok(l.split(' | ').pop().length <= 300);
  assert.ok(!v.join('\n').includes('pbs.twimg.com'));
});

import { cut, clip } from '../lib/text.js';
test('cut and clip never leave half an emoji (lone surrogate breaks the request JSON)', () => {
  const s = 'ab😀cd';
  assert.equal(cut(s, 3), 'ab');
  assert.equal(cut(s, 4), 'ab😀');
  assert.equal(clip(s, 4), 'ab…');
  assert.ok(clip('x'.repeat(98) + '😀😀', 100).isWellFormed());
  const items = assignShortIds([{ key: 'x:1', date: '2026-09-01', type: 'tweet', author: 'a', highlights: [{ text: 'y'.repeat(99) + '😀 tail' }] }]);
  assert.ok(buildMonthIndex(items).text.isWellFormed());
  assert.ok(condensedView(items, { chars: 100 }).isWellFormed());
});

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { parseWork, deriveKey, itemText, selectWorks } from '../lib/works.js';

const tweet = `---
title: Hello model
author: someone
type: tweet
source_system: x
source_id: "123"
url: https://x.com/someone/status/123
---

> First line
> second line

highlighted_at: 2026-09-09 | tags: like

---

> Another highlight

highlighted_at: 2026-09-20

---
`;
const noId = `---
title: Orphan
type: article
---

> text

highlighted_at: 2026-08-31
`;
const undated = `---
title: No highlights
source_system: zotero
source_id: "9"
type: book
---
body only
`;

function archive() {
  const root = mkdtempSync(join(tmpdir(), 'tt-'));
  const dir = join(root, 'readings/works');
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, 'a-tweet.md'), tweet);
  writeFileSync(join(dir, 'b-orphan.md'), noId);
  writeFileSync(join(dir, 'c-undated.md'), undated);
  writeFileSync(join(dir, 'd-copy.md'), tweet); // same key as a-tweet
  return dir;
}

test('parseWork reads frontmatter, highlights and the latest date', () => {
  const w = parseWork(tweet, 'readings/works/a.md');
  assert.equal(w.key, 'x:123');
  assert.equal(w.type, 'tweet');
  assert.equal(w.date, '2026-09-20');
  assert.equal(w.highlights.length, 2);
  assert.equal(w.highlights[0].text, 'First line\nsecond line');
});

test('deriveKey falls back to a stable content hash and says so', () => {
  const a = deriveKey({ type: 'article' }, noId);
  const b = deriveKey({ type: 'article' }, noId);
  assert.equal(a.fallback, true);
  assert.match(a.key, /^sha256:[0-9a-f]{16}$/);
  assert.equal(a.key, b.key);
  assert.equal(deriveKey({ source_system: 'x', source_id: '1' }, '').fallback, false);
});

test('itemText caps long items and reports it', () => {
  const w = parseWork(tweet, 'p');
  w.highlights = [{ date: '2026-09-01', text: 'x'.repeat(50) }];
  const r = itemText(w, 80);
  assert.equal(r.capped, true);
  assert.equal(r.text.length, 80);
  assert.equal(itemText(w, 5000).capped, false);
});

test('selectWorks filters by latest highlight date, skips undated, dedupes keys', () => {
  const dir = archive();
  const sep = selectWorks(dir, '2026-09-01', '2026-09-30');
  assert.deepEqual(sep.items.map((i) => i.key), ['x:123']);
  assert.equal(sep.stats.undated, 1);
  assert.equal(sep.stats.duplicateKeys, 1);
  const aug = selectWorks(dir, '2026-08-01', '2026-08-31');
  assert.equal(aug.items.length, 1);
  assert.equal(aug.stats.keyFallbacks, 1);
  assert.equal(aug.items[0].path, 'readings/works/b-orphan.md');
});

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { buildFixture, fakeClient } from './fixtures-v2.js';
import { briefingGroups, quarterOf, extendRun } from '../lib/extend-run.js';
import { extendThemes, assertAppendOnly } from '../lib/themes.js';
import { briefingSystem } from '../lib/briefing.js';
import { renderOverview } from '../lib/overview.js';

test('quarter briefing merge: a thin month pulls its whole quarter into one briefing', () => {
  const months = ['2022-04', '2022-05', '2022-06', '2022-07', '2022-08', '2022-09'];
  const counts = { '2022-04': 19, '2022-05': 28, '2022-06': 13, '2022-07': 15, '2022-08': 21, '2022-09': 40 };
  const { keyOf, groups } = briefingGroups(months, counts);
  assert.equal(quarterOf('2022-06'), '2022-Q2');
  assert.deepEqual(['2022-04', '2022-05', '2022-06'].map((m) => keyOf[m]), ['2022-Q2', '2022-Q2', '2022-Q2']);
  assert.deepEqual(['2022-07', '2022-08', '2022-09'].map((m) => keyOf[m]), ['2022-07', '2022-08', '2022-09']); // 15 is not thin
  assert.deepEqual(groups[0], { key: '2022-Q2', months: ['2022-04', '2022-05', '2022-06'], items: 60, merged: true });
  assert.equal(groups.length, 4);
  // a lone month of a partial quarter is never "merged" with nothing
  assert.deepEqual(briefingGroups(['2022-10'], { '2022-10': 3 }).groups, [{ key: '2022-10', months: ['2022-10'], items: 3, merged: false }]);
  assert.match(briefingSystem('2022-Q2'), /one quarter \(2022-Q2/);
  assert.match(briefingSystem('2022-05'), /one month \(2022-05\)/);
});

test('theme extension is append-only: old entries identical, new ones added_in, duplicates dropped, edits refused', () => {
  const old = [{ id: 'T01', name: 'AI and mathematics', definition: 'd', include: 'i', exclude: 'e' }, { id: 'T02', name: 'Philosophy and ethics', definition: 'd2', include: 'i', exclude: 'e' }];
  const frozen = JSON.stringify(old);
  const parsed = { themes: [
    { name: 'philosophy and ethics', definition: 'dup', include: '', exclude: '' },
    ...Array.from({ length: 8 }, (_, i) => ({ name: `Anthropology ${i}`, definition: ' d ', include: 'i', exclude: 'e' })),
  ], notes: 'n' };
  const r = extendThemes(old, parsed, { addedIn: '2022-01..2024-12' });
  assert.equal(JSON.stringify(r.themes.slice(0, 2)), frozen);
  assert.equal(r.added.length, 8);
  assert.deepEqual(r.dropped, ['philosophy and ethics']);
  assert.equal(r.themes[2].id, 'T03');
  assert.equal(r.themes[2].added_in, '2022-01..2024-12');
  assert.equal(old[0].added_in, undefined);
  assert.throws(() => extendThemes(old, { themes: parsed.themes.slice(0, 3), notes: '' }, { addedIn: 'x' }), /expected 8-30/);
  assert.throws(() => assertAppendOnly(old, [{ ...old[0], definition: 'changed' }, old[1]]), /append-only/);
  assert.throws(() => assertAppendOnly(old, [old[0]]), /shrank/);
});

test('overview: theme added later shows as not applied to the months it was never run on, plus a by-year table', () => {
  const month = (m, items) => ({ month: m, items, ai_share: 0.5, cost: { briefing: 0, briefing_new: 0, tagging: 0, qa: 0 }, review: 'r', briefing: 'b' });
  const html = renderOverview({
    generated: 'g', months: [month('2024-12', 10), month('2025-01', 20)], themes: ['Old', 'New'],
    counts: { Old: { '2024-12': 2, '2025-01': 4 }, New: { '2024-12': 5 } }, totals: { Old: 6, New: 5 },
    themeInfo: { New: { label: 'new (2022-01..2024-12), not applied to 2025-26', notAppliedFrom: '2025-01' } },
    entities: [], extraCosts: [], total: 0, themesHref: 't', aliasesHref: 'a',
  });
  assert.ok(html.includes('not applied to 2025-26'));
  assert.equal((html.match(/class="na"/g) || []).length, 2); // monthly cell + 2025 year cell
  assert.ok(html.includes('Themes by year'));
});

test('extend run end to end with a fake client: quarter briefing, append-only themes v2, months, merged layer', async () => {
  const { root, stream, rdir } = buildFixture();
  execFileSync('git', ['init', '-q'], { cwd: root });
  execFileSync('git', ['add', 'readings'], { cwd: root });
  execFileSync('git', ['-c', 'user.email=t@t', '-c', 'user.name=t', 'commit', '-qm', 'fixture'], { cwd: root });
  const ctx = join(root, 'analysis/context');
  mkdirSync(ctx, { recursive: true });
  writeFileSync(join(ctx, 'reader-profile.md'), '---\nstatus: x\n---\n# Reader\nHe researches AI and mathematics.');
  writeFileSync(join(ctx, 'readwise-tags.json'), JSON.stringify({ tags: [{ tag: 'epistemology', count: 3 }] }));
  const v1 = { meta: { status: 'FINAL', cost_usd: 0.9 }, notes: 'n', themes: [
    { id: 'T01', name: 'AI and mathematics', definition: 'd', include: 'i', exclude: 'e' }, { id: 'T02', name: 'Education', definition: 'd', include: 'i', exclude: 'e' }] };
  writeFileSync(join(ctx, 'themes.json'), JSON.stringify(v1, null, 2) + '\n');
  const before = readFileSync(join(ctx, 'themes.json'), 'utf8');
  const calls = [];
  const base = {
    archiveRoot: root, tweetsDir: stream, roundupDir: rdir, v1RunId: 'none', model: 'claude-haiku-5-5', effort: 'medium',
    briefingModel: 'claude-sonnet-5-5', themesModel: 'claude-opus-5-5', qaModel: 'claude-sonnet-5-5', qaEffort: 'medium', entityModel: 'claude-sonnet-5-5',
    tweetsChars: 25000, roundupChars: 12000, userReserve: 12000, concurrency: 2, seed: 1, client: fakeClient({ calls, clean: true }), log: () => {},
    fromMonth: '2026-08', toMonth: '2026-09', runPrefix: 'ext', ledgerName: 'ext-ledger-name', maxUsd: 40,
  };
  const r = await extendRun(base);
  assert.equal(r.months_extended, 2);
  assert.equal(r.items, 7);
  assert.ok(existsSync(join(ctx, 'briefings/2026-Q3-briefing.json')), 'thin August pulled the quarter into one briefing');
  assert.ok(!existsSync(join(ctx, 'briefings/2026-09-briefing.json')));
  assert.equal(JSON.parse(readFileSync(join(ctx, 'briefings/2026-Q3-briefing.json'), 'utf8')).meta.items, 7);
  assert.equal(readFileSync(join(ctx, 'themes-v1.json'), 'utf8'), before);
  const v2t = JSON.parse(readFileSync(join(ctx, 'themes.json'), 'utf8'));
  assert.equal(JSON.stringify(v2t.themes.slice(0, 2)), JSON.stringify(v1.themes));
  assert.ok(v2t.themes.length > 10 && v2t.themes.slice(2).every((t) => t.added_in === '2026-08..2026-09'));
  assert.equal(v2t.meta.cost_usd, 0.9);
  assert.ok(v2t.meta.extension.cost_usd > 0);
  const cur = readFileSync(join(root, 'analysis/topics/current.jsonl'), 'utf8').trim().split('\n').map((l) => JSON.parse(l));
  assert.match(cur.find((x) => x.key === 'x:201').event, /^2026-Q3\/E\d\d$/);
  assert.ok(existsSync(join(root, 'analysis/topics/ext-2026-08/review.html')));
  const html = readFileSync(join(root, 'analysis/topics/overview.html'), 'utf8');
  assert.ok(html.includes('2026-Q3.md'));
  assert.equal(execFileSync('git', ['status', '--porcelain', '--', 'readings'], { cwd: root, encoding: 'utf8' }), '');
  const n = calls.length;
  await extendRun(base);
  assert.equal(calls.length, n, 'resumable: nothing paid twice');
  assert.equal(JSON.parse(readFileSync(join(ctx, 'themes.json'), 'utf8')).themes.length, v2t.themes.length);
});

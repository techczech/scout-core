import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { buildFixture, fakeClient, article, tweet } from './fixtures-v2.js';
import { buildWorkIndex, mentionedWorks } from '../lib/mentions.js';
import { fold, mentionedInText, dropOwnEntities } from '../lib/entity-guard.js';
import { exampleFor } from '../lib/review.js';
import { clip } from '../lib/text.js';
import { retagRun } from '../lib/retag-run.js';
import { structuralProblem } from '../lib/full-run.js';

function archive() {
  const { root, stream, rdir } = buildFixture();
  const w = (f, s) => writeFileSync(join(root, 'readings/works', f), s);
  w('tao-1.md', article('t1', 'Terence Tao', 'What is good mathematics', 'https://terrytao.wordpress.com/a', 'a'.repeat(5), '2022-03-01'));
  w('tao-2.md', article('t2', 'Terence Tao', 'Machine assisted proof', 'https://terrytao.wordpress.com/b', 'b', '2024-03-01').replace('> b', '> b\n\nhighlighted_at: 2024-03-02 | tags: x\n\n> c'));
  w('tweet-tao-3.md', tweet('t3', 'Terence Tao', 'A short post', '2025-02-02'));
  w('book-4.md', article('b4', 'Someone Else', 'Seeing Like a State', 'https://example.org/book', 'x', '2023-01-01'));
  w('single-5.md', article('s5', 'Zvi', 'Zvi on things', 'https://example.org/z', 'x', '2023-01-01'));
  return { root, stream, rdir };
}

test('mentioned_works: person -> author (2+ words, folded exact), work -> non-tweet title (2+ words), most highlights first, never self', () => {
  const { root } = archive();
  const idx = buildWorkIndex(root);
  const ents = [{ name: 'terence TAO', type: 'person' }, { name: 'Seeing Like a State', type: 'work' }, { name: 'Zvi', type: 'person' }, { name: 'Tao', type: 'person' }, { name: 'Machine assisted proof', type: 'org' }];
  const m = mentionedWorks(idx, ents, 'readwise:t1');
  assert.deepEqual(m.map((x) => x.key), ['readwise:t2', 'x:t3', 'readwise:b4'], 'two highlights first, then the tweet by Tao; self (t1) excluded; single-word names and wrong types never match');
  assert.equal(m[0].via, 'terence TAO');
  assert.deepEqual(mentionedWorks(idx, [{ name: 'Terence Tao', type: 'person' }], 'x:none', { max: 1 }).map((x) => x.key), ['readwise:t2']);
  assert.deepEqual(mentionedWorks(idx, [{ name: 'Zvi on things', type: 'work' }, { name: 'A short post', type: 'work' }], 'x:none').map((x) => x.key), ['readwise:s5'], 'tweet titles are never matched');
});

test('entity guard folds case, diacritics and punctuation; bylines are not mentions', () => {
  assert.equal(fold('  José  O’Brien-Smith '), 'jose o brien smith');
  assert.equal(mentionedInText('Terence Tao', 'By Terence Tao. Proofs.'), false);
  assert.equal(mentionedInText('Terence Tao', 'By Terence Tao. As terence tao notes, proofs.'), true);
  const r = dropOwnEntities([{ name: 'Vtrivedy10', type: 'person' }, { name: 'Claude', type: 'model' }], { author: 'Vtrivedy10', url: 'https://x.com/Vtrivedy10/status/1', highlights: [{ text: 'Claude is good' }] });
  assert.deepEqual(r, { kept: [{ name: 'Claude', type: 'model' }], dropped: ['Vtrivedy10'] });
});

test('surrogate-safe clipping: an emoji at the cut is dropped whole, never a U+FFFD on a review page', () => {
  const t = 'a'.repeat(199) + '😀 and more text';
  assert.ok(!clip(t, 201).includes('\ud83d') || clip(t, 201).includes('😀'));
  assert.equal(clip(t, 201).toWellFormed(), clip(t, 201));
  const { root } = archive();
  writeFileSync(join(root, 'readings/works/emoji-9.md'), tweet('9', 'e', t, '2026-09-07'));
  const ex = exampleFor(root, { key: 'x:9', path: 'readings/works/emoji-9.md', date: '2026-09-07' });
  assert.equal(ex.snippet.toWellFormed(), ex.snippet);
  assert.ok(!ex.snippet.includes('�'));
});

test('structural check ignores dropped own-author entities', () => {
  const run = { item_count: 100, failed_keys: [], stages: { tagging: { problem_count: 40, problems: Array(40).fill('entity is own author/site m0001: X') } } };
  assert.equal(structuralProblem(run), null);
});

test('re-tag run end to end: new run folders, v3 records, mentioned_works, themes untouched, no relevance anywhere, resumable', async () => {
  const { root, stream, rdir } = archive();
  execFileSync('git', ['init', '-q'], { cwd: root });
  execFileSync('git', ['add', 'readings'], { cwd: root });
  execFileSync('git', ['-c', 'user.email=t@t', '-c', 'user.name=t', 'commit', '-qm', 'fixture'], { cwd: root });
  const ctx = join(root, 'analysis/context');
  mkdirSync(ctx, { recursive: true });
  writeFileSync(join(ctx, 'reader-profile.md'), '---\nstatus: x\n---\n# Reader\nHe researches AI.');
  writeFileSync(join(ctx, 'readwise-tags.json'), JSON.stringify({ tags: [] }));
  const themes = { meta: { cost_usd: 1 }, notes: '', themes: [{ id: 'T01', name: 'AI and mathematics', definition: 'd', include: 'i', exclude: 'e' }] };
  writeFileSync(join(ctx, 'themes.json'), JSON.stringify(themes, null, 2) + '\n');
  const before = readFileSync(join(ctx, 'themes.json'), 'utf8');
  const calls = [];
  const client = fakeClient({ calls, clean: true });
  const base = {
    archiveRoot: root, tweetsDir: stream, roundupDir: rdir, v1RunId: 'none', model: 'claude-haiku-5-5', effort: 'medium',
    briefingModel: 'claude-sonnet-5-5', themesModel: 'claude-opus-5-5', qaModel: 'claude-sonnet-5-5', qaEffort: 'medium', entityModel: 'claude-sonnet-5-5',
    tweetsChars: 25000, roundupChars: 12000, userReserve: 12000, concurrency: 2, seed: 1, client, log: () => {},
    fromMonth: '2026-09', toMonth: '2026-09', runPrefix: 'v3', maxUsd: 25,
  };
  // the briefing for the month exists already (re-tag never writes briefings)
  const { stageBriefing } = await import('../lib/pipeline.js');
  await stageBriefing({ ...base, month: '2026-09', runId: 'seed', maxUsd: 5 });
  const nBriefing = calls.length;
  const r = await retagRun(base);
  assert.equal(r.items, 6);
  assert.equal(r.themes_unchanged, true);
  assert.equal(readFileSync(join(ctx, 'themes.json'), 'utf8'), before);
  assert.ok(!calls.slice(nBriefing).some((c) => JSON.stringify(c.system).includes('factual briefing')), 'no new briefing');
  const A = join(root, 'analysis/topics');
  for (const f of ['v3-2026-09/tags.jsonl', 'v3-2026-09/review.html', 'v3-2026-09/qa.jsonl', 'v3-ledger.json', 'current.jsonl', 'overview.html']) assert.ok(existsSync(join(A, f)), f);
  const cur = readFileSync(join(A, 'current.jsonl'), 'utf8').trim().split('\n').map((l) => JSON.parse(l));
  const astra = cur.find((x) => x.key === 'x:201');
  assert.deepEqual(astra.entities, ['GPT-6 Astra', 'OpenAI']);
  assert.ok(astra.entities.every((e) => typeof e === 'string') && astra.entities_typed.every((e) => e.name && e.type));
  assert.equal(astra.prompt_version, 'topics-v3');
  assert.ok(Array.isArray(astra.related) && Array.isArray(astra.mentioned_works));
  assert.ok(cur.every((x) => !('relevance' in x)));
  assert.ok(!readFileSync(join(A, 'v3-2026-09/tags.jsonl'), 'utf8').includes('relevance'));
  assert.ok(!/why:|relevance/.test(readFileSync(join(A, 'v3-2026-09/review.html'), 'utf8')));
  assert.equal(execFileSync('git', ['status', '--porcelain', '--', 'readings'], { cwd: root, encoding: 'utf8' }), '');
  const n = calls.length;
  await retagRun(base);
  assert.equal(calls.length, n, 'resumable: nothing paid twice');
});

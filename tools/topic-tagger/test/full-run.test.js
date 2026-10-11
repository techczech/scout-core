import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { buildFixture, fakeClient } from './fixtures-v2.js';
import { ruleMap, modelGroups, normaliseEntities, resolveEntity, spellingKey, revalidate } from '../lib/entities.js';
import { monthsBetween, structuralProblem, fullRun } from '../lib/full-run.js';
import { roundupForMonth, itemMonth } from '../lib/sources.js';
import { renderOverview } from '../lib/overview.js';
import * as v2 from '../lib/pipeline.js';

test('entity rules: spelling variants, safe briefing aliases, unique versioned longer name', () => {
  const counts = new Map([['Claude Fable 5.1', 50], ['claude fable 5.1', 2], ['Fable 5.1', 11], ['GPT-6', 4], ['GPT-6 Astra', 100], ['GPT-6 Sol', 40],
    ['Astra', 3], ['Terence Tao', 9], ['Terence Tao letter', 20], ['Open AI', 1], ['OpenAI', 30], ['Open-AI', 1]]);
  const { map, canonical } = ruleMap(counts, [new Map([['astra', 'GPT-6 Astra']])]);
  assert.deepEqual(map['claude fable 5.1'], { to: 'Claude Fable 5.1', rule: 'rule:spelling' });
  assert.deepEqual(map['Fable 5.1'], { to: 'Claude Fable 5.1', rule: 'rule:unique-longer-name' });
  assert.equal(map['Astra'].rule, 'rule:briefing-alias');
  assert.equal(map['GPT-6'], undefined, 'two longer names contain it');
  assert.equal(map['Terence Tao'], undefined, 'people are never folded into events');
  assert.equal(map['Open-AI'].to, 'Open AI'); // spelling key unifies hyphen and space only
  assert.ok(canonical.includes('OpenAI'));
  assert.equal(spellingKey(' Claude’s  Code. '), 'claudes code');
  const bad = ruleMap(new Map([['GPT-2', 3], ['GPT-Realtime-2', 9], ['GLM-4.7', 2], ['GLM-4.7-Flash', 5], ['s1', 2], ['Intern-S1', 4], ['Opus', 5], ['Opus 4.6', 9], ['Opus 4.5', 2], ['Claude Opus 4.5', 8]]),
    [new Map([['opus', 'Opus 4.6'], ['opus 4.5', 'Claude Opus 4.5']])]).map;
  assert.deepEqual(Object.keys(bad), ['Opus 4.5']);
  assert.equal(bad['Opus 4.5'].to, 'Claude Opus 4.5');
});

test('model groups validated against the list; chains resolved; normalised lists deduped', () => {
  const names = ['OpenAI', 'Open AI', 'Codex', 'OpenAI Codex', 'Anthropic'];
  const g = modelGroups({ groups: [
    { canonical: 'OpenAI', variants: ['Open AI', 'Not listed'] },
    { canonical: 'OpenAI Codex', variants: ['Codex'] },
    { canonical: 'Invented', variants: ['Anthropic'] },
  ] }, names, {}, 'claude-sonnet-5-5');
  const rivals = modelGroups({ groups: [{ canonical: 'Deep Research', variants: ['Google Deep Research', 'OpenAI Deep Research', 'deep-research'] }] },
    ['Deep Research', 'Google Deep Research', 'OpenAI Deep Research', 'deep-research'], {}, 'm');
  assert.deepEqual(Object.keys(rivals.map), ['deep-research']);
  const vet = modelGroups({ groups: [{ canonical: 'Terence Tao', variants: ['Tao declaration', 'Tao'] }, { canonical: 'Claude Mythos 5', variants: ['Claude Mythos', 'Mythos 5'] }] },
    ['Terence Tao', 'Tao declaration', 'Tao', 'Claude Mythos 5', 'Claude Mythos', 'Mythos 5'], {}, 'm');
  assert.deepEqual(Object.keys(vet.map).sort(), ['Mythos 5', 'Tao']);
  const rv = revalidate({ 'DeepSeek-V3.1': { to: 'DeepSeek v3', rule: 'model:m' }, 'Muse Spark 1.2': { to: 'Muse Spark', rule: 'model:m' }, 'Karpathy': { to: 'Andrej Karpathy', rule: 'model:m' }, 'Opus 4': { to: 'Claude Opus 4', rule: 'rule:x' } });
  assert.deepEqual(Object.keys(rv.map).sort(), ['Karpathy', 'Opus 4']);
  assert.equal(rv.vetoed.length, 2);
  assert.deepEqual(Object.keys(g.map).sort(), ['Codex', 'Open AI']);
  assert.equal(g.map['Codex'].rule, 'model:claude-sonnet-5-5');
  assert.equal(g.dropped, 2);
  const map = { a: { to: 'b' }, b: { to: 'c' }, x: { to: 'y' }, y: { to: 'x' } };
  assert.equal(resolveEntity(map, 'a'), 'c');
  assert.ok(['x', 'y'].includes(resolveEntity(map, 'x')));
  assert.deepEqual(normaliseEntities(map, ['a', 'b', 'c', 'z']), ['c', 'z']);
});

test('months between, structural stop rule', () => {
  assert.deepEqual(monthsBetween('2025-11', '2026-02'), ['2025-11', '2025-12', '2026-01', '2026-02']);
  assert.equal(monthsBetween('2025-01', '2026-10').length, 22);
  const run = (failed, probs, n = 100) => ({ item_count: n, failed_keys: Array(failed).fill('k'), stages: { tagging: { problem_count: probs } } });
  assert.equal(structuralProblem(run(2, 10)), null);
  assert.match(structuralProblem(run(3, 0)), /failed/);
  assert.match(structuralProblem(run(0, 16)), /validation problems/);
});

test('roundup chosen by how many of its items fall in the month, not by its name', () => {
  const { rdir } = buildFixture();
  writeFileSync(join(rdir, 'mondai-oct-2026.json'), JSON.stringify({ content_library: [
    { date: 'Sep 2, 2026' }, { date: '2026-09-03' }, { date: '2026-09-04' }, { date: '2026-09-05' }, { date: '2026-10-01' }] }));
  assert.equal(itemMonth('Jan 9, 2026'), '2026-01');
  assert.match(roundupForMonth(rdir, '2026-09').path, /mondai-oct-2026\.json$/); // 4 September items beat 0
  assert.equal(roundupForMonth(rdir, '2026-10'), null); // only one item
});

test('overview page: theme x month table with links, escaping, no external requests', () => {
  const html = renderOverview({
    generated: 'g', months: [{ month: '2026-08', items: 10, ai_share: 0.5, cost: { briefing: 0.1, briefing_new: 0.1, tagging: 0.02, qa: 0.05 }, qa: { themes_any_overlap: 0.8, event_match: 0.9 }, review: 'r8/review.html', briefing: 'b8.md' },
      { month: '2026-09', items: 20, ai_share: 0.9, cost: { briefing: 0.4, briefing_new: 0, tagging: 0.2, qa: 0.5 }, review: 'r9/review.html', briefing: 'b9.md' }],
    themes: ['Vibe <coding>'], counts: { 'Vibe <coding>': { '2026-08': 2, '2026-09': 9 } }, totals: { 'Vibe <coding>': 11 },
    entities: [{ name: 'GPT-6 Astra', count: 5 }], extraCosts: [{ stage: 'Final themes', usd: 0.9 }], total: 2.27, qaOverall: { themes_any_overlap: 0.8 }, themesHref: 't', aliasesHref: 'a',
  });
  assert.ok(html.includes('Vibe &lt;coding&gt;'));
  assert.ok(html.includes('href="r9/review.html"') && html.includes('href="b8.md"'));
  assert.ok(html.includes('>9</td>'));
  assert.ok(!/<script|src=|<link /.test(html));
  assert.ok(html.includes('$2.27'));
});

test('full run end to end with a fake client: briefings, final themes, months, entity map, current.jsonl, overview', async () => {
  const { root, stream, rdir } = buildFixture();
  execFileSync('git', ['init', '-q'], { cwd: root });
  execFileSync('git', ['add', 'readings'], { cwd: root });
  execFileSync('git', ['-c', 'user.email=t@t', '-c', 'user.name=t', 'commit', '-qm', 'fixture'], { cwd: root });
  mkdirSync(join(root, 'analysis/context'), { recursive: true });
  writeFileSync(join(root, 'analysis/context/reader-profile.md'), '---\nstatus: DRAFT v2\n---\n# Reader\nHe researches AI and mathematics.');
  const calls = [];
  const client = fakeClient({ calls, clean: true });
  const base = {
    archiveRoot: root, tweetsDir: stream, roundupDir: rdir, v1RunId: 'none', model: 'claude-haiku-5-5', effort: 'medium',
    briefingModel: 'claude-sonnet-5-5', themesModel: 'claude-opus-5-5', qaModel: 'claude-sonnet-5-5', qaEffort: 'medium', entityModel: 'claude-sonnet-5-5',
    tweetsChars: 25000, roundupChars: 12000, userReserve: 12000, concurrency: 2, seed: 1, client, log: () => {},
  };
  // the September v2 run whose stats feed the final themes
  const sept = { ...base, month: '2026-09', runId: 'v2-sept', maxUsd: 5 };
  v2.stageProfileSources(sept);
  await v2.stageBriefing(sept);
  await v2.stageThemes({ ...sept, v1RunId: 'v1' }).catch(() => {}); // no v1 vocabulary in this fixture
  mkdirSync(join(root, 'analysis/topics/v1'), { recursive: true });
  writeFileSync(join(root, 'analysis/topics/v1/vocabulary.json'), JSON.stringify({ topics: [] }));
  await v2.stageThemes({ ...sept, v1RunId: 'v1' });
  v2.stageEnrich(sept);
  await v2.stageTag(sept);

  const r = await fullRun({ ...base, fromMonth: '2026-08', toMonth: '2026-09', runPrefix: 'final', septV2RunId: 'v2-sept', maxUsd: 45 });
  assert.equal(r.months, 2);
  assert.equal(r.items, 7); // 1 August + 6 September items
  const A = join(root, 'analysis');
  for (const f of ['context/briefings/2026-08.md', 'context/themes.json', 'context/entity-aliases.json', 'topics/current.jsonl', 'topics/overview.html', 'topics/final-2026-08/review.html', 'topics/final-2026-09/qa.jsonl', 'topics/final-ledger.json']) {
    assert.ok(existsSync(join(A, f)), f);
  }
  const cur = readFileSync(join(A, 'topics/current.jsonl'), 'utf8').trim().split('\n').map((l) => JSON.parse(l));
  const astra = cur.find((x) => x.key === 'x:201');
  assert.equal(astra.month, '2026-09');
  assert.match(astra.event, /^2026-09\/E\d\d$/);
  assert.ok(astra.event_name);
  assert.equal(astra.run_id, 'final-2026-09');
  const ledger = JSON.parse(readFileSync(join(A, 'topics/final-ledger.json'), 'utf8'));
  assert.deepEqual(ledger.preexisting_briefings, ['2026-09']);
  assert.equal(ledger.costs.per['2026-09'].briefing_new, 0);
  assert.ok(readFileSync(join(A, 'topics/overview.html'), 'utf8').includes('final-2026-08/review.html'));
  assert.equal(execFileSync('git', ['status', '--porcelain', '--', 'readings'], { cwd: root, encoding: 'utf8' }), '');
  // resumable: a second call sends no new model requests
  const before = calls.length;
  await fullRun({ ...base, fromMonth: '2026-08', toMonth: '2026-09', runPrefix: 'final', septV2RunId: 'v2-sept', maxUsd: 45 });
  assert.equal(calls.length, before);
});

test('full run stops before any request when the ledger would pass the cap', async () => {
  const { root, stream, rdir } = buildFixture();
  mkdirSync(join(root, 'analysis/context'), { recursive: true });
  writeFileSync(join(root, 'analysis/context/reader-profile.md'), '# R');
  const calls = [];
  await assert.rejects(() => fullRun({ archiveRoot: root, tweetsDir: stream, roundupDir: rdir, briefingModel: 'claude-sonnet-5-5', client: fakeClient({ calls }), log: () => {},
    fromMonth: '2026-09', toMonth: '2026-09', runPrefix: 'f', maxUsd: 0.0001 }), /briefing failures/);
  assert.equal(calls.length, 0);
});

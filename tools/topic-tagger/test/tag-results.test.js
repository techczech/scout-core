import { test } from 'node:test';
import assert from 'node:assert/strict';
import { validateTagBatch, aliasMap } from '../lib/tag-results.js';
import { agreement, qaSample } from '../lib/qa.js';
import { validateBriefing } from '../lib/briefing.js';
import { validateThemes } from '../lib/themes.js';
import { costFromUsage } from '../lib/cost.js';

const it = (sid, key) => ({ sid, key, path: `readings/works/${key}.md`, date: '2026-09-02', type: 'tweet' });
const batch = { id: 'b1', items: [it('m0001', 'x:1'), it('m0002', 'x:2'), it('m0003', 'x:3')] };
const ctx = {
  themes: [{ name: 'AI and mathematics' }, { name: 'Vibe coding and AI-assisted programming' }, { name: 'Epistemology' }, { name: 'AI safety' }],
  events: [{ id: 'E01' }, { id: 'E02' }],
  aliases: aliasMap({ names: [{ name: 'GPT-6 Astra', aliases: ['Astra', 'GPT-6 Astra'] }, { name: 'Claude Opus 5.5', aliases: ['Opus 5.5'] }] }),
};

test('themes: only list names (case-insensitive), deduped, max 3', () => {
  const r = validateTagBatch({ results: [
    { id: 'm0001', ai_related: true, themes: ['ai and MATHEMATICS', 'AI and mathematics', 'Made up', 'Epistemology', 'AI safety', 'Vibe coding and AI-assisted programming'], entities: [], event: '', relevance: '', confidence: 'high' },
  ] }, batch, ctx);
  assert.deepEqual(r.records[0].themes, ['AI and mathematics', 'Epistemology', 'AI safety']);
  assert.ok(r.problems.some((p) => p.includes('Made up')));
});

test('entities: alias-normalised to full names, case-deduped, capped at 6, over-long dropped', () => {
  const r = validateTagBatch({ results: [
    { id: 'm0001', ai_related: true, themes: ['Epistemology'], entities: ['Astra', 'gpt-6 astra', 'opus 5.5', ' OpenAI ', 'openai', 'A', 'B', 'C', 'D', 'x'.repeat(80)], event: 'e01', relevance: '', confidence: 'medium' },
  ] }, batch, ctx);
  assert.deepEqual(r.records[0].entities, ['GPT-6 Astra', 'Claude Opus 5.5', 'OpenAI', 'A', 'B', 'C']);
  assert.equal(r.records[0].event, 'E01');
});

test('alias map uses only aliases whose words are in the full name; combined names skipped', () => {
  const m = aliasMap({ names: [
    { name: 'Claude Fable 5.1', aliases: ['Fable', 'Fable 5', 'Fable 5.1'] },
    { name: 'GPT-6 Sol / Luna', aliases: ['Sol', 'Luna'] },
    { name: 'Jev', aliases: ['System One models'] },
  ] });
  assert.equal(m.get('fable'), 'Claude Fable 5.1');
  assert.equal(m.get('fable 5.1'), 'Claude Fable 5.1');
  assert.equal(m.get('fable 5'), undefined);
  assert.equal(m.get('sol'), undefined);
  assert.equal(m.get('system one models'), undefined);
  assert.equal(m.get('jev'), 'Jev');
});

test('event must be a briefing id; relevance trimmed to 15 words; confidence enforced; record shape', () => {
  const r = validateTagBatch({ results: [
    { id: 'm0001', ai_related: true, themes: ['AI safety'], entities: [], event: 'E99', relevance: 'one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen sixteen', confidence: 'sure' },
    { id: 'm0002', ai_related: true, themes: [], entities: [], event: '', relevance: '', confidence: 'high' },
    { id: 'm0002', ai_related: false, themes: [], entities: [], event: '', relevance: '', confidence: 'high' },
    { id: 'zz', ai_related: false, themes: [], entities: [], event: '', relevance: '', confidence: 'high' },
  ] }, batch, ctx);
  const [a, b] = r.records;
  assert.equal(a.event, null);
  assert.equal(a.relevance.split(' ').length, 15);
  assert.equal(a.confidence, 'low');
  assert.equal(b.confidence, 'low', 'AI item without themes is forced to low so QA sees it');
  assert.deepEqual(Object.keys(a), ['key', 'path', 'date', 'type', 'ai_related', 'themes', 'entities', 'event', 'relevance', 'confidence']);
  assert.deepEqual(r.missing, ['x:3']);
  for (const want of ['unknown event E99', 'bad confidence', 'AI item without themes', 'duplicate id', 'unknown id zz']) assert.ok(r.problems.some((p) => p.includes(want)), want);
  assert.equal(validateTagBatch(null, batch, ctx).missing.length, 3);
});

test('briefing validation renumbers events by date and drops unknown item ids', () => {
  const b = validateBriefing({ overview: 'o', events: [
    { id: 'Q', name: 'Later', date: '2026-09-20', aliases: ['L', 'L'], description: 'd', items: ['m0001', 'm9'] },
    { id: 'P', name: 'Earlier', date: '2026-09-02', aliases: [], description: 'd', items: [] },
  ], names: [{ name: 'GPT-6 Astra', aliases: ['Astra', 'GPT-6 Astra'], note: 'n' }], debates: [], engagement: 'e' }, ['m0001']);
  assert.deepEqual(b.events.map((e) => [e.id, e.name]), [['E01', 'Earlier'], ['E02', 'Later']]);
  assert.deepEqual(b.events[1].items, ['m0001']);
  assert.deepEqual(b.events[1].aliases, ['L']);
  assert.deepEqual(b.names[0].aliases, ['Astra']);
  assert.equal(b.problems.length, 1);
});

test('themes validation dedupes and enforces 25-55', () => {
  const mk = (n) => ({ themes: Array.from({ length: n }, (_, i) => ({ name: `Theme ${i}`, definition: 'd', include: 'i', exclude: 'e' })), notes: '' });
  const t = validateThemes({ ...mk(30), themes: [...mk(30).themes, { name: 'theme 0', definition: '', include: '', exclude: '' }] });
  assert.equal(t.themes.length, 30);
  assert.equal(t.themes[29].id, 'T30');
  assert.throws(() => validateThemes(mk(10)), /10 themes/);
});

test('QA sample: 100 random plus low-confidence, capped at 150, reproducible', () => {
  const recs = Array.from({ length: 1000 }, (_, i) => ({ key: `k${i}`, confidence: i % 10 === 0 ? 'low' : 'high' }));
  const a = qaSample(recs, { seed: 1 });
  const b = qaSample(recs, { seed: 1 });
  assert.deepEqual(a.keys, b.keys);
  assert.equal(a.keys.length, 150);
  assert.equal(new Set(a.keys).size, 150);
  assert.equal(a.low_total, 100);
  const few = qaSample(recs.slice(0, 120), { seed: 1 });
  assert.equal(few.keys.length, 100 + few.low_total - few.keys.slice(0, 100).filter((k) => recs[Number(k.slice(1))].confidence === 'low').length);
});

test('agreement: overlap, exact, Jaccard, event and ai_related', () => {
  const r = (themes, entities, event, ai = true) => ({ themes, entities, event, ai_related: ai });
  const a = agreement([
    { a: r(['A', 'B'], ['GPT-6 Astra'], 'E01'), b: r(['B'], ['gpt-6 astra'], 'E01') },
    { a: r(['A'], [], null), b: r(['C'], [], null, false) },
  ]);
  assert.equal(a.themes_any_overlap, 0.5);
  assert.equal(a.themes_exact, 0);
  assert.equal(a.themes_mean_jaccard, 0.25);
  assert.equal(a.entities_any_overlap, 1);
  assert.equal(a.entities_exact, 1);
  assert.equal(a.event_match, 1);
  assert.equal(a.event_match_when_both_set, 1);
  assert.equal(a.ai_related_match, 0.5);
});

test('cost: cache reads and writes priced off the input rate; long-prompt step counts cached tokens', () => {
  const u = { input_tokens: 1000, cache_creation_input_tokens: 0, cache_read_input_tokens: 90000, output_tokens: 1000 };
  assert.ok(Math.abs(costFromUsage('claude-haiku-5-5', u) - (1000 * 0.1 + 90000 * 0.01 + 1000 * 0.5) / 1e6) < 1e-12);
  const w = { input_tokens: 0, cache_creation_input_tokens: 80000, output_tokens: 0 };
  assert.ok(Math.abs(costFromUsage('claude-haiku-5-5', w) - 80000 * 0.125 / 1e6) < 1e-12);
  const long = { input_tokens: 20000, cache_read_input_tokens: 90000, output_tokens: 0 };
  assert.ok(Math.abs(costFromUsage('claude-haiku-5-5', long) - (20000 * 0.5 + 90000 * 0.05) / 1e6) < 1e-12);
  assert.ok(Math.abs(costFromUsage('claude-opus-5-5', { cache_read_input_tokens: 1e6 }) - 0.2) < 1e-12);
});

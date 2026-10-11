import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, readFileSync, readdirSync, writeFileSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { buildCandidates } from '../lib/events-merge-candidates.js';
import { proposeMerges, packPairs } from '../lib/events-merge-run.js';
import { treeHash } from '../lib/tag-new-run.js';
import { tweet } from './fixtures-v2.js';

// spec: [id, name, start, end, {aliases, n, ents, themes}]. Every event is one monthly id "<month>/E01" with n tag rows.
let seq = 0;
function world(specs, { files = false } = {}) {
  const root = mkdtempSync(join(tmpdir(), 'tt-merge-'));
  mkdirSync(join(root, 'analysis/topics'), { recursive: true });
  mkdirSync(join(root, 'analysis/events'), { recursive: true });
  mkdirSync(join(root, 'readings/works'), { recursive: true });
  const events = [], rows = [];
  for (const [id, name, start, end, o = {}] of specs) {
    const n = o.n ?? 4;
    const member = `${start.slice(0, 7)}/${id}`;
    events.push({ corpus: 'highlights', id, name, record: true, start, end, items: n, months: [{ month: start.slice(0, 7), count: n }], members: [member], aliases: o.aliases || [], themes: ['T'], origin: 'model', edited: null, merged_into: o.merged_into || null, description: o.description || `About ${name}.` });
    writeFileSync(join(root, 'analysis/events', `${id}.md`), `---\nid: ${id}\nname: ${name}\n---\n${name}\n`);
    for (let i = 0; i < n; i++) {
      const key = `x:${++seq}`;
      const date = i === 0 ? start : i === n - 1 ? end : start;
      const path = `readings/works/w${seq}.md`;
      if (files) writeFileSync(join(root, path), tweet(String(seq), 'auth', `Text of ${name} item ${i}`, date));
      rows.push({ key, path, date, month: date.slice(0, 7), event: member, themes: ['T'], entities_typed: (o.ents || []).map((e) => ({ name: e, type: 'org' })) });
    }
  }
  writeFileSync(join(root, 'analysis/topics/current.jsonl'), rows.map((r) => JSON.stringify(r)).join('\n') + '\n');
  return { root, events, rows };
}
const ids = (c) => c.pairs.map((p) => `${p.a.id}~${p.b.id}`).sort();
const cands = (w, extra = {}) => buildCandidates({ events: w.events, rows: w.rows, archiveTop: 0, ...extra }); // fixtures hold a handful of entities, so "top 30" would be all of them

test('entity signal: a named entity frequent in both events makes a pair; an archive-wide frequent one does not', () => {
  const w = world([['a', 'Alpha story', '2026-08-01', '2026-08-20', { ents: ['Orion Labs'] }], ['b', 'Zeta tale', '2026-09-05', '2026-09-20', { ents: ['Orion Labs'] }]]);
  const c = cands(w);
  assert.deepEqual(ids(c), ['a~b']);
  assert.deepEqual(c.pairs[0].signals, ['entity']);
  assert.deepEqual(c.stats.by_signal, { entity: 1, 'alias-token': 0, 'name-similarity': 0 });
  const frequent = cands(w, { archiveTop: 1 }); // Orion Labs is then the archive's most frequent entity
  assert.deepEqual(frequent.stats.archive_frequent_entities, ['Orion Labs']);
  assert.deepEqual(ids(frequent), []);
});

test('entity signal needs 30% of each event and two items', () => {
  const w = world([['a', 'Alpha story', '2026-08-01', '2026-08-20', { n: 10, ents: [] }], ['b', 'Zeta tale', '2026-09-05', '2026-09-20', { n: 4, ents: ['Orion Labs'] }]]);
  w.rows.filter((r) => r.event === '2026-08/a').slice(0, 2).forEach((r) => r.entities_typed.push({ name: 'Orion Labs', type: 'org' })); // 2 of 10 = 20%
  assert.deepEqual(ids(cands(w)), []);
  w.rows.filter((r) => r.event === '2026-08/a').slice(2, 3).forEach((r) => r.entities_typed.push({ name: 'Orion Labs', type: 'org' })); // 3 of 10
  assert.deepEqual(ids(cands(w)), ['a~b']);
});

test('alias-token signal: shared word of 5+ letters; stopwords, short words and archive-entity words do not count', () => {
  const w = world([
    ['a', 'Quasar release', '2026-08-01', '2026-08-20', { aliases: ['Quasar Lite', 'launch wave'] }],
    ['b', 'Later chatter', '2026-09-05', '2026-09-20', { aliases: ['quasar', 'launch wave', 'cats'] }],
    ['c', 'Third thing', '2026-09-05', '2026-09-20', { aliases: ['launch wave', 'cats'] }],
  ]);
  const c = cands(w);
  assert.deepEqual(ids(c), ['a~b']);
  assert.deepEqual(c.pairs[0].signals, ['alias-token']);
  assert.deepEqual(c.pairs[0].shared_tokens, ['quasar']);
  // a word of an archive-wide frequent entity's name is not a signal
  const w2 = world([['a', 'Ranger wins', '2026-08-01', '2026-08-20', { ents: ['Ranger Corp'] }], ['b', 'Ranger loses badly', '2026-09-05', '2026-09-20', { n: 4, ents: ['Ranger Corp', 'Other'] }]]);
  w2.rows.filter((r) => r.event === '2026-09/b').forEach((r) => { r.entities_typed = [{ name: 'Ranger Corp', type: 'org' }]; });
  assert.ok(!ids(cands(w2, { archiveTop: 1 })).length);
});

test('name-similarity signal: two shared name words, or most of the name', () => {
  const w = world([['a', 'Moon base plan', '2026-08-01', '2026-08-20'], ['b', 'Moon base delay', '2026-09-05', '2026-09-20'], ['c', 'Moon garden', '2026-09-05', '2026-09-20']]);
  const c = cands(w);
  assert.deepEqual(ids(c), ['a~b']);
  assert.deepEqual(c.pairs[0].signals, ['name-similarity']);
});

test('span window: 45 days between end and start is in, 46 is out, overlap is in', () => {
  const ents = { ents: ['Orion Labs'] };
  const w = world([
    ['a', 'Alpha story', '2026-01-20', '2026-02-10', ents],
    ['in', 'Beta tale', '2026-03-27', '2026-04-05', ents], // 45 days after 02-10
    ['out', 'Gamma note', '2026-03-28', '2026-04-05', ents], // 46 days
    ['over', 'Delta item', '2026-02-05', '2026-03-01', ents], // overlaps a
  ]);
  const c = cands(w, { maxPartners: 99 });
  const got = ids(c);
  assert.ok(got.includes('a~in') && got.includes('a~over'));
  assert.ok(!got.includes('a~out'));
});

test('events below --min-items are not candidates on either side; merged records are skipped', () => {
  const ents = { ents: ['Orion Labs'] };
  const w = world([['a', 'Alpha story', '2026-08-01', '2026-08-20', { n: 1, ...ents }], ['b', 'Zeta tale', '2026-09-05', '2026-09-20', ents], ['c', 'Eta piece', '2026-10-06', '2026-10-20', ents], ['d', 'Theta bit', '2026-10-06', '2026-10-20', { ...ents, merged_into: 'c' }]]);
  assert.equal(cands(w, { minItems: 2 }).stats.events_considered, 2); // b and c
  assert.equal(cands(w, { minItems: 1 }).stats.events_considered, 3); // a joins; d stays out (merged)
  assert.deepEqual(ids(cands(w, { minItems: 2 })), ['b~c']);
});

test('same-month records are paired only on strong overlap', () => {
  const w = world([['a', 'Alpha story', '2026-09-01', '2026-09-10', { ents: ['Orion Labs'] }], ['b', 'Zeta tale', '2026-09-12', '2026-09-20', { ents: ['Orion Labs'] }],
    ['c', 'Eta piece', '2026-09-01', '2026-09-10', { ents: ['P1', 'P2', 'P3'] }], ['d', 'Theta bit', '2026-09-12', '2026-09-20', { ents: ['P1', 'P2', 'P3'] }]]);
  const c = cands(w);
  assert.deepEqual(ids(c), ['c~d']);
  assert.equal(c.stats.same_month_dropped, 1);
});

test('partner cap: no event is in more than 6 pairs', () => {
  const specs = [['hub', 'Hub story', '2026-08-01', '2026-08-20', { ents: ['Orion Labs'] }]];
  for (let i = 0; i < 9; i++) specs.push([`p${i}`, `Partner number${i}`, '2026-09-05', '2026-09-20', { ents: ['Orion Labs'] }]);
  const w = world(specs);
  const uncapped = cands(w, { maxPartners: 99 });
  const c = cands(w);
  const per = new Map();
  for (const p of c.pairs) for (const id of [p.a.id, p.b.id]) per.set(id, (per.get(id) || 0) + 1);
  assert.ok(uncapped.pairs.length > c.pairs.length);
  assert.ok(Math.max(...per.values()) <= 6);
  assert.ok(c.stats.dropped_by_partner_cap > 0);
});

// ---- the run ----
const fake = ({ usage = { input_tokens: 1000, output_tokens: 500 }, answer } = {}) => {
  const calls = [];
  const reply = (params) => {
    calls.push(params);
    const content = params.messages[0].content;
    const pairs = [...content.matchAll(/<pair id="(p\d+)">\nA: (.*)\n[\s\S]*?\nB: (.*)\n/g)];
    const results = pairs.map(([, id, a, b]) => answer ? answer(id, a, b, pairs.length) : { id, verdict: a.includes('Orion') && b.includes('Orion') ? 'same' : 'different', confidence: 'confident', reason: 'They share the Orion launch.' });
    return { stop_reason: 'end_turn', usage, content: [{ type: 'text', text: JSON.stringify({ results }) }] };
  };
  return { calls, messages: { create: async (p) => { const r = reply(p); return typeof r.then === 'function' ? r : r; }, stream: (p) => ({ finalMessage: async () => reply(p) }) } };
};
const orionWorld = () => world([
  ['a', 'Orion launch', '2026-08-01', '2026-08-20', { ents: ['Orion Labs'] }],
  ['b', 'Orion aftermath', '2026-09-05', '2026-09-20', { ents: ['Orion Labs'] }],
  ['c', 'Comet plans', '2026-08-02', '2026-08-25', { ents: ['Pluto Inc'] }],
  ['d', 'Pluto tale', '2026-09-06', '2026-09-22', { ents: ['Pluto Inc'] }],
], { files: true });
const base = (w, extra = {}) => ({ archiveRoot: w.root, runId: 'r1', model: 'claude-sonnet-5-5', maxUsd: 5, readEvents: () => w.events, concurrency: 1, archiveTop: 0, now: new Date('2026-10-11T10:00:00Z'), ...extra });
const outDir = (w) => join(w.root, 'analysis/events/merge-proposals');
const jl = (p) => readFileSync(p, 'utf8').split('\n').filter(Boolean).map((l) => JSON.parse(l));
// every file outside the output folder, as path -> bytes
const snapshot = (root) => { const out = {}; const walk = (d, rel) => { for (const e of readdirSync(d, { withFileTypes: true })) { const p = join(d, e.name); if (e.isDirectory()) { if (rel + e.name === 'analysis/events/merge-proposals') continue; walk(p, `${rel}${e.name}/`); } else out[`${rel}${e.name}`] = readFileSync(p, 'utf8'); } }; walk(root, ''); return out; };

test('run: one line per judged pair, a is the earlier record, only the two outputs appear, nothing else changes', async () => {
  const w = orionWorld();
  const before = snapshot(w.root);
  const client = fake();
  const r = await proposeMerges(base(w, { client }));
  assert.deepEqual(readdirSync(outDir(w)).sort(), ['r1-ledger.json', 'r1.jsonl']);
  assert.deepEqual(snapshot(w.root), before);
  const lines = jl(join(outDir(w), 'r1.jsonl'));
  assert.equal(lines.length, 2);
  const ab = lines.find((l) => l.a === 'a');
  assert.deepEqual(Object.keys(ab).sort(), ['a', 'a_items', 'a_name', 'a_span', 'b', 'b_items', 'b_name', 'b_span', 'confidence', 'judged_at', 'model', 'prompt_version', 'reason', 'signals', 'verdict'].sort());
  assert.equal(ab.b, 'b'); assert.equal(ab.verdict, 'same'); assert.equal(ab.model, 'claude-sonnet-5-5'); assert.deepEqual(ab.a_span, ['2026-08-01', '2026-08-20']);
  const ledger = JSON.parse(readFileSync(join(outDir(w), 'r1-ledger.json'), 'utf8'));
  assert.equal(ledger.verdicts.same.confident, 1); // Orion pair; the Comet/Pluto pair is "different"
  assert.equal(ledger.verdicts.different.confident, 1);
  assert.equal(ledger.merge_proposals.confident, 1);
  assert.equal(ledger.candidates_by_signal.entity, 2);
  assert.equal(ledger.stopped_budget, false);
  assert.ok(ledger.cost_usd_total > 0 && r.model_requests === 1);
  // the request carries names, aliases, descriptions, spans and sample item text
  const msg = client.calls[0].messages[0].content;
  assert.match(msg, /Orion launch/); assert.match(msg, /span: 2026-08-01 to 2026-08-20/); assert.match(msg, /Text of Orion launch item 0/);
});

test('run: a reply that is malformed for one pair becomes an error line and the rest are kept', async () => {
  const w = orionWorld();
  const client = fake({ answer: (id, a) => (a.includes('Comet') ? { id, verdict: 'maybe', confidence: 'confident', reason: 'x' } : { id, verdict: 'same', confidence: 'doubtful', reason: 'Same Orion story.' }) });
  await proposeMerges(base(w, { client }));
  const lines = jl(join(outDir(w), 'r1.jsonl'));
  assert.equal(lines.length, 2);
  const bad = lines.find((l) => l.a === 'c');
  assert.match(bad.error, /malformed verdict/); assert.equal(bad.verdict, undefined);
  assert.equal(lines.find((l) => l.a === 'a').verdict, 'same');
  const ledger = JSON.parse(readFileSync(join(outDir(w), 'r1-ledger.json'), 'utf8'));
  assert.equal(ledger.errors, 1); assert.equal(ledger.verdicts.same.doubtful, 1);
});

test('run: an unreadable reply splits the request down to single pairs; a pair that still fails is an error line', async () => {
  const w = orionWorld();
  let n = 0;
  const client = { messages: { create: async (params) => {
    n++;
    const c = params.messages[0].content;
    const two = (c.match(/<pair id=/g) || []).length > 1;
    if (two || c.includes('Comet')) return { stop_reason: 'end_turn', usage: { input_tokens: 10, output_tokens: 10 }, content: [{ type: 'text', text: 'not json' }] };
    return { stop_reason: 'end_turn', usage: { input_tokens: 10, output_tokens: 10 }, content: [{ type: 'text', text: JSON.stringify({ results: [{ id: 'p1', verdict: 'same', confidence: 'confident', reason: 'ok' }] }) }] };
  } } };
  await proposeMerges(base(w, { client }));
  const lines = jl(join(outDir(w), 'r1.jsonl'));
  assert.equal(n, 3);
  assert.equal(lines.find((l) => l.a === 'a').verdict, 'same');
  assert.match(lines.find((l) => l.a === 'c').error, /request failed/);
});

test('batching: 8 pairs per request', () => {
  const specs = [];
  for (let i = 0; i < 18; i++) specs.push([`e${i}`, `Story ${String.fromCharCode(97 + i)}${i}`, `2026-0${1 + (i % 2)}-01`, `2026-0${1 + (i % 2)}-20`, { ents: [`Co${Math.floor(i / 2)}`] }]);
  const w = world(specs, { files: true });
  const c = buildCandidates({ events: w.events, rows: w.rows, archiveTop: 0 });
  assert.equal(c.pairs.length, 9);
  const ctx = { profiles: c.profiles, rows: c.rows, loadText: () => 't' };
  assert.deepEqual(packPairs(c.pairs, ctx, { perRequest: 4 }).map((r) => r.pairs.length), [4, 4, 1]);
  assert.deepEqual(packPairs(c.pairs, ctx).map((r) => r.pairs.length), [8, 1]);
});

test('budget stop: pairs judged so far are written and the ledger says stopped_budget', async () => {
  const specs = [];
  for (let i = 0; i < 20; i++) specs.push([`e${i}`, `Story ${String.fromCharCode(97 + i)}${i}`, i % 2 ? '2026-09-01' : '2026-08-01', i % 2 ? '2026-09-20' : '2026-08-20', { ents: [`Co${Math.floor(i / 2)}`] }]);
  const w = world(specs, { files: true });
  const client = fake({ usage: { input_tokens: 1000, output_tokens: 6000 } }); // $0.062 a request
  const r = await proposeMerges(base(w, { client, maxUsd: 0.07, archiveTop: 0, concurrency: 1 }));
  assert.ok(r.stopped_budget);
  assert.ok(r.pairs_judged > 0 && r.pairs_judged < 10);
  assert.equal(r.pairs_judged + r.pairs_not_judged, 10);
  assert.equal(jl(join(outDir(w), 'r1.jsonl')).length, r.pairs_judged);
});

test('dry run: counts and an estimate, no model call, nothing written', async () => {
  const w = orionWorld();
  const before = treeHash(w.root);
  const r = await proposeMerges(base(w, { dryRun: true, client: null }));
  assert.equal(r.dry_run, true); assert.equal(r.kept_pairs, 2); assert.equal(r.requests, 1);
  assert.ok(r.estimate_usd > 0); assert.equal(r.candidates_by_signal.entity, 2);
  assert.equal(treeHash(w.root), before);
  assert.ok(!existsSync(outDir(w)));
});

test('an existing output for the run id is refused and left alone', async () => {
  const w = orionWorld();
  mkdirSync(outDir(w), { recursive: true });
  writeFileSync(join(outDir(w), 'r1.jsonl'), 'keep\n');
  const client = fake();
  await assert.rejects(proposeMerges(base(w, { client })), /already exists/);
  assert.equal(readFileSync(join(outDir(w), 'r1.jsonl'), 'utf8'), 'keep\n');
  assert.equal(client.calls.length, 0);
});

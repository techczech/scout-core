import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdirSync, readFileSync, readdirSync, writeFileSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { buildFixture, fakeClient, tweet } from './fixtures-v2.js';
import { fullRun } from '../lib/full-run.js';
import { tagNewRun, treeHash, appendToCurrent, splitMonthItems } from '../lib/tag-new-run.js';
import * as v2 from '../lib/pipeline.js';

const NOW = new Date('2026-10-12T08:05:00Z');
const STAMP = '20261012-0805';

// Two tagged months (2026-08, 2026-09, prefix "final"), then new works: two in 2026-09, one in 2026-08, one in 2026-11 (no context).
async function prepared() {
  const { root, stream, rdir } = buildFixture();
  mkdirSync(join(root, 'analysis/context'), { recursive: true });
  writeFileSync(join(root, 'analysis/context/reader-profile.md'), '---\nstatus: DRAFT v2\n---\n# Reader\nHe researches AI and mathematics.');
  const calls = [];
  const base = {
    archiveRoot: root, tweetsDir: stream, roundupDir: rdir, v1RunId: 'none', model: 'claude-haiku-5-5', effort: 'medium',
    briefingModel: 'claude-sonnet-5-5', themesModel: 'claude-opus-5-5', qaModel: 'claude-sonnet-5-5', qaEffort: 'medium', entityModel: 'claude-sonnet-5-5',
    tweetsChars: 25000, roundupChars: 12000, userReserve: 12000, concurrency: 2, seed: 1, client: fakeClient({ calls, clean: true }), log: () => {},
  };
  const sept = { ...base, month: '2026-09', runId: 'v2-sept', maxUsd: 5 };
  v2.stageProfileSources(sept);
  await v2.stageBriefing(sept);
  mkdirSync(join(root, 'analysis/topics/v1'), { recursive: true });
  writeFileSync(join(root, 'analysis/topics/v1/vocabulary.json'), JSON.stringify({ topics: [] }));
  await v2.stageThemes({ ...sept, v1RunId: 'v1' });
  v2.stageEnrich(sept);
  await v2.stageTag(sept);
  await fullRun({ ...base, fromMonth: '2026-08', toMonth: '2026-09', runPrefix: 'final', septV2RunId: 'v2-sept', maxUsd: 45 });
  const w = (f, s) => writeFileSync(join(root, 'readings/works', f), s);
  w('frank-new-207.md', tweet('207', 'frank', 'New proof of the lemma, mathematicians argue about credit', '2026-09-07'));
  w('gina-new-208.md', tweet('208', 'gina', 'Astra is great. INVENTED link', '2026-09-08'));
  w('hank-new-209.md', tweet('209', 'hank', 'Lunch was good', '2026-08-15'));
  w('ivy-new-210.md', tweet('210', 'ivy', 'A mathematician in November', '2026-11-02'));
  calls.length = 0;
  return { root, base, calls, topics: join(root, 'analysis/topics'), ctx: join(root, 'analysis/context') };
}

const opts = (base, extra = {}) => ({ ...base, runPrefix: 'final', maxUsd: 5, now: NOW, ...extra });
const lines = (p) => readFileSync(p, 'utf8').split('\n').filter(Boolean);
const dirs = (topics) => readdirSync(topics, { withFileTypes: true }).filter((e) => e.isDirectory()).map((e) => e.name).sort();
// a client that answers the invented-event item with an event id that is not in the month's list
const inventing = (client) => ({ ...client, messages: { ...client.messages, create: async (params) => {
  const msg = await client.messages.create(params);
  if (!params.system || !Array.isArray(params.system) || !params.messages[0].content.includes('INVENTED')) return msg;
  const o = JSON.parse(msg.content[0].text);
  for (const r of o.results) { const body = params.messages[0].content.split('<item id="' + r.id + '">')[1] || ''; if (body.split('</item>')[0].includes('INVENTED')) r.event = 'E77'; }
  return { ...msg, content: [{ type: 'text', text: JSON.stringify(o) }] };
} } });

test('new items get rows linked only to existing events; an invented event id is dropped and counted; old data is untouched', async () => {
  const { base, topics, ctx } = await prepared();
  const ctxHash = treeHash(ctx);
  const oldDirs = dirs(topics);
  const oldHashes = Object.fromEntries(oldDirs.map((d) => [d, treeHash(join(topics, d))]));
  const before = lines(join(topics, 'current.jsonl'));
  const events = (m) => new Set(JSON.parse(readFileSync(join(ctx, `briefings/${m}-briefing.json`), 'utf8')).events.map((e) => e.id));
  const r = await tagNewRun(opts({ ...base, client: inventing(base.client) }));
  assert.equal(r.untagged_items, 5); // the fixture's July essay is outside the tagged range, so it is skipped too
  assert.deepEqual(r.months.map((m) => [m.month, m.new_items, m.tagged]), [['2026-08', 1, 1], ['2026-09', 2, 2]]);
  assert.deepEqual(r.skipped_no_context, [{ month: '2026-07', items: 1 }, { month: '2026-11', items: 1 }]);
  assert.equal(r.events_dropped_total, 1);
  assert.equal(r.context_unchanged, true);
  assert.match(r.qa, /skipped/);
  // new run folders, one per month, named with prefix, stamp and month
  const added = dirs(topics).filter((d) => !oldDirs.includes(d));
  assert.deepEqual(added, [`final-new-${STAMP}-2026-08`, `final-new-${STAMP}-2026-09`]);
  for (const d of added) for (const f of ['tags.jsonl', 'run.json', 'enrichment.jsonl', 'prompt-tagging-system.json', 'prompt-tagging-system.txt']) assert.ok(existsSync(join(topics, d, f)), `${d}/${f}`);
  assert.ok(!existsSync(join(topics, added[1], 'qa.jsonl')));
  const run = JSON.parse(readFileSync(join(topics, added[1], 'run.json'), 'utf8'));
  assert.equal(run.events_dropped, 1);
  assert.equal(run.kind, 'tag-new');
  const ledger = JSON.parse(readFileSync(join(topics, `final-new-${STAMP}-ledger.json`), 'utf8'));
  assert.equal(ledger.model, 'claude-haiku-5-5');
  assert.equal(ledger.skipped_no_context[1].month, '2026-11');
  assert.ok(ledger.cost_usd_total > 0 && ledger.finished && ledger.context_unchanged);
  // rows: new keys only, events from the existing list, invented one has no event
  const after = lines(join(topics, 'current.jsonl'));
  assert.equal(after.length, before.length + 3);
  const rows = Object.fromEntries(after.map((l) => JSON.parse(l)).map((x) => [x.key, x]));
  assert.deepEqual(['x:207', 'x:208', 'x:209'].map((k) => !!rows[k]), [true, true, true]);
  assert.equal(rows['x:210'], undefined);
  assert.equal(rows['x:208'].event, null);
  assert.match(rows['x:207'].event, /^2026-09\/E\d\d$/);
  assert.ok(events('2026-09').has(rows['x:207'].event.split('/')[1]));
  assert.equal(rows['x:207'].run_id, `final-new-${STAMP}-2026-09`);
  assert.equal(rows['x:207'].month, '2026-09');
  // nothing else moved
  assert.equal(treeHash(ctx), ctxHash, 'analysis/context byte for byte');
  for (const d of oldDirs) assert.equal(treeHash(join(topics, d)), oldHashes[d], `${d} unchanged`);
  const afterSet = new Set(after);
  assert.ok(before.every((l) => afterSet.has(l)), 'every old line still present unchanged');
  assert.deepEqual(after.filter((l) => before.includes(l)), before, 'old lines keep their order');
});

test('--months limits the run to the named months', async () => {
  const { base, topics } = await prepared();
  const r = await tagNewRun(opts(base, { months: '2026-08' }));
  assert.deepEqual(r.months.map((m) => m.month), ['2026-08']);
  assert.equal(r.untagged_items, 1);
  assert.deepEqual(r.skipped_no_context, []);
  assert.equal(dirs(topics).filter((d) => d.includes('-new-')).length, 1);
  assert.equal(lines(join(topics, 'current.jsonl')).filter((l) => l.includes('"x:209"')).length, 1);
  assert.ok(!lines(join(topics, 'current.jsonl')).some((l) => l.includes('"x:207"')));
  await assert.rejects(() => tagNewRun(opts(base, { months: '2026-9' })), /not a month/);
});

test('--dry-run needs no client, writes nothing, and reports counts, context and an estimate', async () => {
  const { root, base, calls } = await prepared();
  const hash = treeHash(join(root, 'analysis'));
  const r = await tagNewRun(opts({ ...base, client: null }, { dryRun: true }));
  assert.equal(r.dry_run, true);
  assert.equal(r.untagged_items, 5);
  assert.deepEqual(r.months.map((m) => [m.month, m.untagged, m.has_context]), [['2026-08', 1, true], ['2026-09', 2, true]]);
  assert.deepEqual(r.skipped_no_context, [{ month: '2026-07', items: 1 }, { month: '2026-11', items: 1 }]);
  assert.ok(r.estimate_usd > 0 && r.months.every((m) => m.estimate_usd > 0));
  assert.equal(calls.length, 0);
  assert.equal(treeHash(join(root, 'analysis')), hash, 'nothing written');
});

test('the cost cap stops before the month that would exceed it and leaves no partial month', async () => {
  const { base, topics, calls } = await prepared();
  const dry = await tagNewRun(opts({ ...base, client: null }, { dryRun: true }));
  const first = dry.months[0].estimate_usd;
  const r = await tagNewRun(opts(base, { maxUsd: first + 0.00001 }));
  assert.deepEqual(r.months.map((m) => m.month), ['2026-08']);
  assert.deepEqual(r.not_run_budget.map((m) => m.month), ['2026-09']);
  assert.equal(dirs(topics).filter((d) => d.includes('-new-')).length, 1);
  assert.ok(!lines(join(topics, 'current.jsonl')).some((l) => l.includes('"x:207"')));
  // a cap below the first month's estimate sends nothing at all
  const before = lines(join(topics, 'current.jsonl')).length;
  calls.length = 0;
  const r2 = await tagNewRun(opts(base, { maxUsd: 0.0000001, now: new Date('2026-10-12T09:00:00Z'), months: '2026-09' }));
  assert.equal(r2.months.length, 0);
  assert.equal(calls.length, 0);
  assert.equal(lines(join(topics, 'current.jsonl')).length, before);
  assert.equal(dirs(topics).filter((d) => d.includes('-new-')).length, 1);
});

test('a second run right after finds nothing to tag and writes nothing', async () => {
  const { root, base, calls } = await prepared();
  await tagNewRun(opts(base, { months: '2026-08,2026-09' }));
  const hash = treeHash(join(root, 'analysis'));
  calls.length = 0;
  const r = await tagNewRun(opts(base, { months: '2026-08,2026-09', now: new Date('2026-10-12T09:30:00Z') }));
  assert.equal(r.untagged_items, 0);
  assert.equal(r.months.length, 0);
  assert.equal(calls.length, 0);
  assert.equal(treeHash(join(root, 'analysis')), hash);
});

test('appendToCurrent keeps old lines byte for byte and in place, inserting new rows by date and key', () => {
  const { root } = buildFixture();
  const p = join(root, 'current.jsonl');
  const old = [{ key: 'a', date: '2026-01-01' }, { key: 'c', date: '2026-01-03', extra: 'x  y' }, { key: 'd', date: '2026-02-01' }].map((o) => JSON.stringify(o));
  writeFileSync(p, old.join('\n') + '\n');
  appendToCurrent(p, [{ key: 'z', date: '2026-03-01' }, { key: 'b', date: '2026-01-02' }]);
  const out = lines(p);
  assert.deepEqual(out.map((l) => JSON.parse(l).key), ['a', 'b', 'c', 'd', 'z']);
  assert.ok(old.every((l) => out.includes(l)));
  assert.throws(() => appendToCurrent(p, [{ key: 'a', date: '2026-01-01' }]), /already has a row/);
});

test('already-tagged items keep the short ids numbered over the tagged set; new items continue after them', () => {
  const it = (key, date) => ({ key, date });
  const month = [it('b', '2026-09-02'), it('a', '2026-09-01'), it('n1', '2026-09-01'), it('c', '2026-09-03'), it('n0', '2026-09-09')];
  const { indexItems, newItems } = splitMonthItems(month, new Set(['a', 'b', 'c']));
  assert.deepEqual(indexItems.filter((i) => !newItems.includes(i)).map((i) => [i.key, i.sid]), [['a', 'm0001'], ['b', 'm0002'], ['c', 'm0003']]);
  assert.deepEqual(newItems.map((i) => [i.key, i.sid]), [['n1', 'm0004'], ['n0', 'm0005']]);
});

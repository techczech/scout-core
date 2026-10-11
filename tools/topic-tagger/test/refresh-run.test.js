import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { buildFixture, fakeClient } from './fixtures-v2.js';
import { fullRun } from '../lib/full-run.js';
import { refreshRun, listRunMonths, listBriefingMonths, stampOf } from '../lib/refresh-run.js';
import * as v2 from '../lib/pipeline.js';

const NOW = new Date('2026-10-11T12:34:56Z');
const STAMP = '20261011-1234';

// A finished two-month run (prefix "final": 2026-08 and 2026-09), the starting point of every refresh.
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
  calls.length = 0;
  return { root, base, calls, topics: join(root, 'analysis/topics'), bdir: join(root, 'analysis/context/briefings') };
}

const refreshOpts = (base, extra = {}) => ({ ...base, months: '2026-09', runPrefix: 'final', maxUsd: 5, now: NOW, ...extra });
// every file of a folder as name -> sha256, to prove a run folder is untouched byte for byte
const folderHash = (dir) => Object.fromEntries(readdirSync(dir).sort().map((f) => [f, createHash('sha256').update(readFileSync(join(dir, f))).digest('hex')]));
const readCurrent = (topics) => readFileSync(join(topics, 'current.jsonl'), 'utf8').trim().split('\n').map((l) => JSON.parse(l));

test('refresh moves the run folder and the three briefing files aside, regenerates them, and leaves other months byte for byte', async () => {
  const { base, calls, topics, bdir } = await prepared();
  const aug = folderHash(join(topics, 'final-2026-08'));
  const oldBriefing = readFileSync(join(bdir, '2026-09-briefing.json'), 'utf8');
  const oldTags = readFileSync(join(topics, 'final-2026-09/tags.jsonl'), 'utf8');
  const r = await refreshRun(refreshOpts(base));
  assert.equal(r.stamp, STAMP);
  assert.equal(stampOf(NOW), STAMP);
  const sup = `final-2026-09.superseded-${STAMP}`;
  assert.equal(readFileSync(join(topics, sup, 'tags.jsonl'), 'utf8'), oldTags, 'old run folder moved whole');
  assert.equal(readFileSync(join(bdir, `2026-09-briefing.superseded-${STAMP}.json`), 'utf8'), oldBriefing);
  assert.ok(existsSync(join(bdir, `2026-09-events.superseded-${STAMP}.json`)));
  assert.ok(existsSync(join(bdir, `2026-09.superseded-${STAMP}.md`)));
  for (const f of ['2026-09-briefing.json', '2026-09-events.json', '2026-09.md']) assert.ok(existsSync(join(bdir, f)), `${f} regenerated`);
  assert.ok(existsSync(join(topics, 'final-2026-09/tags.jsonl')) && existsSync(join(topics, 'final-2026-09/review.html')) && existsSync(join(topics, 'final-2026-09/qa.jsonl')));
  assert.ok(calls.length > 0, 'briefing, tagging and QA were paid for again');
  assert.deepEqual(folderHash(join(topics, 'final-2026-08')), aug, 'untouched month identical byte for byte');
  assert.ok(existsSync(join(topics, `final-refresh-${STAMP}-ledger.json`)));
  assert.equal(JSON.parse(readFileSync(join(topics, 'final-ledger.json'), 'utf8')).kind, undefined, 'the first run ledger is not rewritten');
  const cur = readCurrent(topics);
  assert.equal(cur.length, 7, 'all months of the prefix are merged');
  assert.ok(cur.every((x) => !x.run_id.includes('superseded')));
  assert.match(cur.find((x) => x.key === 'x:201').event, /^2026-09\/E\d\d$/);
  const html = readFileSync(join(topics, 'overview.html'), 'utf8');
  assert.ok(html.includes('final-2026-08/review.html') && html.includes('final-2026-09/review.html') && !html.includes('superseded'));
  assert.ok(r.this_run_usd > 0);
});

test('superseded folders and briefing files are ignored by every lister and by the merge', async () => {
  const { base, topics, bdir } = await prepared();
  // a stale superseded copy carrying a key that must never reach current.jsonl
  const stale = join(topics, 'final-2026-08.superseded-20200101-0000');
  mkdirSync(stale);
  writeFileSync(join(stale, 'tags.jsonl'), JSON.stringify({ key: 'x:STALE', path: 'p', date: '2026-08-01', type: 'tweet', ai_related: false, themes: [], entities: [], event: '', related: [], confidence: 'high' }) + '\n');
  writeFileSync(join(bdir, '2026-08-briefing.superseded-20200101-0000.json'), '{}');
  assert.deepEqual(listRunMonths(topics, 'final'), ['2026-08', '2026-09']);
  assert.deepEqual(listBriefingMonths(bdir), ['2026-08', '2026-09']);
  await refreshRun(refreshOpts(base));
  assert.deepEqual(listRunMonths(topics, 'final'), ['2026-08', '2026-09']);
  assert.ok(!readFileSync(join(topics, 'current.jsonl'), 'utf8').includes('x:STALE'));
  assert.equal(readCurrent(topics).length, 7);
});

test('a quarter-keyed briefing is refused with a clear error and nothing is moved', async () => {
  const { base, topics, bdir } = await prepared();
  writeFileSync(join(bdir, '2026-Q4-briefing.json'), '{}');
  await assert.rejects(() => refreshRun(refreshOpts(base, { months: '2026-09,2026-10' })), /2026-10 is covered by the quarter briefing 2026-Q4; refresh supports month-keyed briefings only/);
  assert.ok(existsSync(join(topics, 'final-2026-09/tags.jsonl')), 'valid month not moved either');
  assert.equal(readdirSync(topics).filter((n) => n.includes('superseded') || n.includes('refresh')).length, 0);
  await assert.rejects(() => refreshRun(refreshOpts(base, { months: '2026-9' })), /not a month/);
});

test('an interrupted refresh resumes without moving the fresh partial output aside again', async () => {
  const { base, calls, topics, bdir } = await prepared();
  // interrupted after the briefing: token counting (first thing the tagging stage asks for) fails
  const good = base.client;
  const dying = { messages: { ...good.messages, countTokens: async () => { throw new Error('network down'); } } };
  await assert.rejects(() => refreshRun(refreshOpts({ ...base, client: dying })), /stopped: 2026-09: network down/);
  const sup = `final-2026-09.superseded-${STAMP}`;
  assert.ok(existsSync(join(topics, sup)));
  assert.ok(existsSync(join(bdir, '2026-09-briefing.json')), 'the briefing was regenerated before the failure');
  const freshBriefing = readFileSync(join(bdir, '2026-09-briefing.json'), 'utf8');
  const ledgers = () => readdirSync(topics).filter((f) => /^final-refresh-.*-ledger\.json$/.test(f));
  assert.equal(ledgers().length, 1);
  calls.length = 0;
  const later = new Date('2026-10-11T13:00:00Z');
  const r = await refreshRun(refreshOpts(base, { now: later }));
  assert.equal(r.stamp, STAMP, 'the same refresh continued');
  assert.equal(readFileSync(join(bdir, '2026-09-briefing.json'), 'utf8'), freshBriefing, 'fresh briefing not redone');
  assert.deepEqual(readdirSync(topics).filter((f) => f.includes('superseded')), [sup], 'nothing else moved aside');
  assert.equal(readdirSync(bdir).filter((f) => f.includes('superseded')).length, 3);
  assert.equal(ledgers().length, 1);
  assert.ok(JSON.parse(readFileSync(join(topics, ledgers()[0]), 'utf8')).finished);
  assert.equal(readCurrent(topics).length, 7);
  // once finished, the same command is a new refresh with its own stamp
  await refreshRun(refreshOpts(base, { now: later }));
  assert.equal(readdirSync(topics).filter((f) => f.includes('superseded')).length, 2);
});

test('the cost cap stops a refresh cleanly before any request, and a higher cap resumes it', async () => {
  const { base, calls, topics } = await prepared();
  const before = readFileSync(join(topics, 'current.jsonl'), 'utf8');
  await assert.rejects(() => refreshRun(refreshOpts(base, { maxUsd: 0.0001 })), /exceeds cap/);
  assert.equal(calls.length, 0, 'no request made over the cap');
  assert.equal(readFileSync(join(topics, 'current.jsonl'), 'utf8'), before, 'current.jsonl untouched until the merge');
  const r = await refreshRun(refreshOpts(base, { maxUsd: 5 }));
  assert.equal(r.stamp, STAMP);
  assert.equal(readdirSync(topics).filter((f) => f.includes('superseded')).length, 1);
  assert.equal(readCurrent(topics).length, 7);
});

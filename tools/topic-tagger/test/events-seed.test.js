import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parse } from 'yaml';
import { assignId, LEDGER_FILE, majorityThemes, parseRecord, periodMonth, periodSpan, planSeed, seedEvents, slugify, SLUG_MAX } from '../lib/events-seed.js';

const CLI = join(dirname(fileURLToPath(import.meta.url)), '..', 'cli.js');
const put = (root, rel, text) => { const p = join(root, rel); mkdirSync(dirname(p), { recursive: true }); writeFileSync(p, text); return p; };
const row = (key, event, date, themes = [], extra = {}) => JSON.stringify({ key, path: `readings/works/${key}.md`, date, month: date?.slice(0, 7), themes, entities: [], event, event_name: null, ...extra });
const events = (month, list) => JSON.stringify({ month, model: 'test', events: list.map(([id, name, description = `About ${name}.`, aliases = []]) => ({ id, name, date: `${month.slice(0, 4)}-01-01`, aliases, description, items: [] })) });
const tree = (root) => {
  const out = {};
  const walk = (d) => { for (const e of readdirSync(d, { withFileTypes: true })) { const p = join(d, e.name); if (e.isDirectory()) walk(p); else out[p] = readFileSync(p, 'utf8'); } };
  if (existsSync(root)) walk(root);
  return out;
};
const records = (dir) => Object.fromEntries(readdirSync(dir).filter((f) => f.endsWith('.md')).map((f) => f.slice(0, -3)).sort().map((id) => [id, parseRecord(readFileSync(join(dir, `${id}.md`), 'utf8'))]));

const AWKWARD = 'He said: "it\'s #1" — naïve café, 100% [sic] & more';

function archive() {
  const root = mkdtempSync(join(tmpdir(), 'events-seed-'));
  put(root, 'analysis/context/briefings/2026-09-events.json', events('2026-09', [
    ['E01', AWKWARD, 'Line one: with a colon.\n\n--- not a fence\nLine "two".', ["O'Reilly: the 'book'", 'Ünïcode — alias']],
    ['E02', 'Gemini launch'],
    ['E03', 'Gemini Launch!'],
    ['E04', 'Nobody tagged this'],
    ['E10', '日本語'],
    ['E11', `Long ${'word '.repeat(30)}`],
  ]));
  put(root, 'analysis/context/briefings/2026-10-events.json', events('2026-10', [['E01', 'Gemini launch']]));
  put(root, 'analysis/context/briefings/2022-Q2-events.json', events('2022-Q2', [['E01', 'Quarter story']]));
  // Superseded files and folders are never read.
  put(root, 'analysis/context/briefings/2026-09-events.superseded-20261011-1603.json', events('2026-09', [['E01', 'OLD NAME'], ['E99', 'Old only']]));
  put(root, 'analysis/context/briefings/2026-08.superseded-20261011-1603/2026-08-events.json', events('2026-08', [['E01', 'In a superseded folder']]));
  put(root, 'analysis/topics/current.jsonl', [
    row('x:1', '2026-09/E01', '2026-09-03', ['A', 'B']),
    row('x:2', '2026-09/E01', '2026-09-20', ['A']),
    row('x:3', '2026-09/E01', '2026-10-02', ['A', 'C']),
    row('x:4', '2026-09/E02', '2026-09-05', ['A']),
    row('x:5', '2026-09/E03', '2026-09-06'),
    row('x:6', '2026-09/E10', '2026-09-07'),
    row('x:7', '2026-09/E11', '2026-09-08'),
    row('x:8', '2026-10/E01', '2026-10-09'),
    row('x:9', '2022-Q2/E01', null),
    row('x:10', '2026-08/E01', '2026-08-01'),
    row('x:11', '2026-09/E99', '2026-09-09', [], { event_name: 'Only in the tag rows' }),
    row('x:12', null, '2026-09-10'),
    'not json',
  ].join('\n') + '\n');
  return root;
}

test('slugs: lower-case ASCII, hyphens, at most 60 chars, accents folded, never a path', () => {
  assert.equal(slugify(AWKWARD), 'he-said-it-s-1-naive-cafe-100-sic-more');
  assert.equal(slugify('  ../../etc/passwd  '), 'etc-passwd');
  assert.equal(slugify('日本語'), '');
  assert.equal(slugify('Ærø & Ünïcode'), 'r-unicode');
  const long = slugify('ab '.repeat(40));
  assert.ok(long.length <= SLUG_MAX && !long.endsWith('-') && /^[a-z0-9]+(-[a-z0-9]+)*$/.test(long));
  assert.equal(periodMonth('2022-Q2'), '2022-04');
  assert.equal(periodMonth('2026-09'), '2026-09');
  assert.deepEqual(periodSpan('2022-Q2'), { start: '2022-04-01', end: '2022-06-30' });
  assert.deepEqual(periodSpan('2024-02'), { start: '2024-02-01', end: '2024-02-29' });
});

test('ids: collisions take -2, -3; an empty slug falls back to the event number; the suffix fits the cap', () => {
  const taken = new Set();
  assert.equal(assignId('2026-09', 'E02', 'Gemini launch', taken).id, '2026-09-gemini-launch');
  assert.deepEqual(assignId('2026-09', 'E03', 'Gemini Launch!', taken), { id: '2026-09-gemini-launch-2', base: '2026-09-gemini-launch', fallback: false, truncated: false, suffix: 2 });
  assert.equal(assignId('2026-09', 'E07', 'gemini  launch', taken).id, '2026-09-gemini-launch-3');
  assert.equal(assignId('2026-10', 'E01', 'Gemini launch', taken).id, '2026-10-gemini-launch');
  assert.equal(assignId('2026-09', 'E10', '日本語', taken).id, '2026-09-event-e10');
  assert.equal(assignId('2026-09', 'E12', '!!!', taken).id, '2026-09-event-e12');
  const a = assignId('2026-09', 'E20', 'x'.repeat(80), taken);
  const b = assignId('2026-09', 'E21', 'x'.repeat(80), taken);
  assert.equal(a.id, `2026-09-${'x'.repeat(60)}`);
  assert.equal(b.id, `2026-09-${'x'.repeat(58)}-2`);
  assert.ok(a.truncated && b.suffix === 2);
});

test('themes: carried by a third of the items, most frequent first, at most five', () => {
  const items = [{ themes: ['A', 'B', 'A'] }, { themes: ['A'] }, { themes: ['A', 'C'] }, { themes: [] }];
  assert.deepEqual(majorityThemes(items), ['A']);
  assert.deepEqual(majorityThemes([{ themes: ['b', 'a'] }, { themes: ['b', 'a', 'c'] }, { themes: ['c'] }]), ['a', 'b', 'c']);
  assert.deepEqual(majorityThemes([{ themes: ['g', 'f', 'e', 'd', 'c', 'b', 'a'] }]), ['a', 'b', 'c', 'd', 'e']);
});

test('a dry run writes nothing, not even the out folder', () => {
  const root = archive();
  const before = tree(root);
  const r = seedEvents({ archiveRoot: root });
  assert.equal(r.apply, false);
  assert.equal(r.to_create, 9);
  assert.equal(r.created, 0);
  assert.equal(r.briefing_events, 8);
  assert.equal(r.briefing_events_without_items, 1);
  assert.deepEqual(r.notes.briefing_events_without_items, ['2026-09/E04']);
  assert.ok(!existsSync(join(root, 'analysis', 'events')));
  assert.deepEqual(tree(root), before);
});

test('apply: one record per tagged monthly event, the spec frontmatter as valid YAML, superseded input ignored', () => {
  const root = archive();
  const out = join(root, 'analysis', 'events');
  const r = seedEvents({ archiveRoot: root, apply: true, now: new Date('2026-10-11T12:00:00Z'), version: '9.9.9' });
  assert.equal(r.created, 9);
  const recs = records(out);
  assert.deepEqual(Object.keys(recs), [
    '2022-04-quarter-story',
    '2026-08-2026-08-e01',
    '2026-09-event-e10',
    '2026-09-gemini-launch',
    '2026-09-gemini-launch-2',
    '2026-09-he-said-it-s-1-naive-cafe-100-sic-more',
    `2026-09-long-${'word-'.repeat(11)}`.slice(0, 8 + SLUG_MAX).replace(/-$/, ''),
    '2026-09-only-in-the-tag-rows',
    '2026-10-gemini-launch',
  ]);
  for (const [id, rec] of Object.entries(recs)) {
    assert.deepEqual(Object.keys(rec.frontmatter), ['id', 'name', 'aliases', 'start', 'end', 'members', 'themes', 'origin', 'edited', 'merged_into']);
    assert.equal(rec.frontmatter.id, id);
    assert.equal(rec.frontmatter.origin, 'model');
    assert.equal(rec.frontmatter.edited, null);
    assert.equal(rec.frontmatter.merged_into, null);
    assert.equal(rec.frontmatter.members.length, 1);
  }
  const awkward = recs['2026-09-he-said-it-s-1-naive-cafe-100-sic-more'];
  assert.deepEqual(awkward.frontmatter, {
    id: '2026-09-he-said-it-s-1-naive-cafe-100-sic-more', name: AWKWARD, aliases: ["O'Reilly: the 'book'", 'Ünïcode — alias'],
    start: '2026-09-03', end: '2026-10-02', members: ['2026-09/E01'], themes: ['A', 'B', 'C'], origin: 'model', edited: null, merged_into: null,
  });
  assert.equal(awkward.body, '\nLine one: with a colon.\n\n--- not a fence\nLine "two".\n');
  // The empty fields are written empty, as the spec shows them.
  const raw = readFileSync(join(out, '2026-09-gemini-launch.md'), 'utf8');
  assert.match(raw, /\nedited:\nmerged_into:\n---\n\nAbout Gemini launch\.\n$/);
  // A quarter event: id from the quarter's first month; no dated items, so the quarter's first and last day.
  assert.deepEqual([recs['2022-04-quarter-story'].frontmatter.start, recs['2022-04-quarter-story'].frontmatter.end, recs['2022-04-quarter-story'].frontmatter.members], ['2022-04-01', '2022-06-30', ['2022-Q2/E01']]);
  // Tag rows naming an event no briefing file holds: named from the row, else the monthly id; empty body.
  assert.equal(recs['2026-09-only-in-the-tag-rows'].frontmatter.name, 'Only in the tag rows');
  assert.equal(recs['2026-08-2026-08-e01'].frontmatter.name, '2026-08/E01');
  assert.equal(recs['2026-08-2026-08-e01'].body, '\n');
  assert.ok(!JSON.stringify(recs).includes('OLD NAME') && !JSON.stringify(recs).includes('superseded folder'));
  assert.deepEqual(r.notes.collisions, [{ id: '2026-09-gemini-launch-2', base: '2026-09-gemini-launch', member: '2026-09/E03' }]);
  assert.deepEqual(r.notes.fallback_slugs.map((x) => x.id), ['2026-09-event-e10']);
  assert.deepEqual(r.notes.no_briefing_entry, ['2026-08/E01', '2026-09/E99']);
  assert.deepEqual(r.notes.month_dates, ['2022-Q2/E01']);
  const ledger = readFileSync(join(out, LEDGER_FILE), 'utf8').trim().split('\n').map((l) => JSON.parse(l));
  assert.deepEqual(ledger, [{ date: '2026-10-11T12:00:00.000Z', tool: 'topic-tagger events-seed', version: '9.9.9', created: 9, skipped: 0, already_in_a_record: 0, skipped_existing_file: 0, monthly_events_with_items: 9, briefing_events_without_items: 1 }]);
  // Nothing but records and the ledger is left behind.
  assert.deepEqual(readdirSync(out).filter((f) => !f.endsWith('.md')), [LEDGER_FILE]);
});

test('idempotent: a second run writes no record and changes none; ids are stable', () => {
  const root = archive();
  const out = join(root, 'analysis', 'events');
  seedEvents({ archiveRoot: root, apply: true });
  const first = Object.fromEntries(Object.entries(tree(out)).filter(([p]) => p.endsWith('.md')));
  const r = seedEvents({ archiveRoot: root, apply: true });
  assert.equal(r.created, 0);
  assert.equal(r.to_create, 0);
  assert.equal(r.already_in_a_record, 9);
  assert.deepEqual(Object.fromEntries(Object.entries(tree(out)).filter(([p]) => p.endsWith('.md'))), first);
  assert.equal(readFileSync(join(out, LEDGER_FILE), 'utf8').trim().split('\n').length, 2);
});

test('a record he changed is never rewritten; an event folded into another record is not seeded again', () => {
  const root = archive();
  const out = join(root, 'analysis', 'events');
  // He already holds two monthly events in one record of his own, under an id the seed would have used.
  const mine = put(out, '2026-09-gemini-launch.md', '---\nid: 2026-09-gemini-launch\nname: "Gemini: my name"\nmembers: [2026-09/E02, 2026-10/E01]\norigin: dominik\nedited: 2026-10-11\n---\n\nMy words.\n');
  const before = readFileSync(mine, 'utf8');
  const r = seedEvents({ archiveRoot: root, apply: true });
  assert.equal(readFileSync(mine, 'utf8'), before);
  assert.equal(r.already_in_a_record, 2);
  assert.equal(r.created, 7);
  const ids = Object.keys(records(out));
  assert.ok(!ids.includes('2026-10-gemini-launch'));
  // 2026-09/E03 slugs to the same id, which is taken by his record: it gets -2, his file is untouched.
  assert.deepEqual(records(out)['2026-09-gemini-launch-2'].frontmatter.members, ['2026-09/E03']);
  // New monthly events arriving later are added without touching what is there.
  const all = tree(out);
  put(root, 'analysis/topics/current.jsonl', readFileSync(join(root, 'analysis/topics/current.jsonl'), 'utf8') + row('x:20', '2026-10/E02', '2026-10-10') + '\n');
  const again = seedEvents({ archiveRoot: root, apply: true });
  assert.equal(again.created, 1);
  for (const [p, text] of Object.entries(all)) if (p.endsWith('.md')) assert.equal(readFileSync(p, 'utf8'), text);
});

test('an unreadable record stops the run before anything is written', () => {
  for (const [name, text, why] of [
    ['broken.md', '---\nid: [unclosed\n---\n', /cannot read the record/],
    ['no-frontmatter.md', 'just prose\n', /cannot read the record/],
    ['other-name.md', '---\nid: something-else\nmembers: []\n---\n', /does not match the file name/],
    ['bad-members.md', '---\nid: bad-members\nmembers: 2026-09/E01\n---\n', /members is not a list/],
  ]) {
    const root = archive();
    const out = join(root, 'analysis', 'events');
    put(out, name, text);
    const before = tree(out);
    assert.throws(() => seedEvents({ archiveRoot: root, apply: true }), why);
    assert.deepEqual(tree(out), before);
  }
});

test('--out: records go to the resolved out folder and nowhere else; the archive is untouched', () => {
  const root = archive();
  const before = tree(root);
  const scratch = mkdtempSync(join(tmpdir(), 'events-out-'));
  const real = join(scratch, 'real');
  mkdirSync(real);
  symlinkSync(real, join(scratch, 'link'));
  const r = seedEvents({ archiveRoot: root, outDir: join(scratch, 'link', '..', 'link'), apply: true });
  assert.equal(r.created, 9);
  assert.ok(r.out.endsWith('/real'));
  assert.deepEqual(tree(root), before);
  assert.equal(readdirSync(real).filter((f) => f.endsWith('.md')).length, 9);
  assert.deepEqual(readdirSync(scratch).sort(), ['link', 'real']);
  // Every planned id is a bare slug, so a file name can never step out of the folder.
  for (const rec of planSeed({ archiveRoot: root, outDir: join(scratch, 'none') }).records) assert.match(rec.id, /^[a-z0-9]+(-[a-z0-9]+)*$/);
});

test('names that try to leave the folder still give plain file names inside it', () => {
  const root = mkdtempSync(join(tmpdir(), 'events-seed-'));
  put(root, 'analysis/context/briefings/2026-09-events.json', events('2026-09', [['E01', '../../../outside'], ['E02', '/etc/passwd'], ['E03', '..'], ['E04', 'a\\..\\b\u0000c']]));
  put(root, 'analysis/topics/current.jsonl', ['E01', 'E02', 'E03', 'E04'].map((e, i) => row(`x:${i}`, `2026-09/${e}`, '2026-09-01')).join('\n'));
  const out = join(root, 'deep', 'out');
  seedEvents({ archiveRoot: root, outDir: out, apply: true });
  assert.deepEqual(Object.keys(records(out)), ['2026-09-a-b-c', '2026-09-etc-passwd', '2026-09-event-e03', '2026-09-outside']);
  assert.deepEqual(Object.keys(tree(root)).filter((p) => !p.startsWith(join(root, 'analysis')) && !p.startsWith(out)), []);
  assert.equal(records(out)['2026-09-a-b-c'].frontmatter.name, 'a\\..\\b\u0000c');
});

test('the ledger is only ever a plain file of the out folder', () => {
  const root = archive();
  const out = join(root, 'analysis', 'events');
  mkdirSync(out, { recursive: true });
  const elsewhere = put(root, 'elsewhere.txt', 'keep\n');
  symlinkSync(elsewhere, join(out, LEDGER_FILE));
  assert.throws(() => seedEvents({ archiveRoot: root, apply: true }), /not a plain file/);
  assert.equal(readFileSync(elsewhere, 'utf8'), 'keep\n');
});

test('cli: events-seed prints counts, writes only with --apply, and the files parse as YAML', () => {
  const root = archive();
  const out = mkdtempSync(join(tmpdir(), 'events-cli-'));
  const run = (...args) => JSON.parse(execFileSync(process.execPath, [CLI, 'events-seed', '--archive', root, '--out', out, ...args], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }));
  const dry = run();
  assert.equal(dry.to_create, 9);
  assert.equal(dry.created, 0);
  assert.deepEqual(readdirSync(out), []);
  const wet = run('--apply');
  assert.equal(wet.created, 9);
  assert.equal(wet.one_item, 8);
  assert.equal(run('--apply').created, 0);
  for (const f of readdirSync(out).filter((x) => x.endsWith('.md'))) {
    const text = readFileSync(join(out, f), 'utf8');
    const fm = parse(text.slice(4, text.indexOf('\n---\n', 4)));
    assert.equal(`${fm.id}.md`, f);
  }
});

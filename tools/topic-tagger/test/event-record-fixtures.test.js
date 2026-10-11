// The shared event-record cases (crates/scout-corpus/tests/fixtures/event-records/): the seed stage's check of
// existing records (lib/event-record.js) must give every file the verdict verdicts.json names, the same file the
// Rust reader is held to in crates/scout-corpus/tests/event_record_fixtures.rs. Then the seed itself: one record
// the reader would skip stops the run before anything is written.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { copyFileSync, existsSync, lstatSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, readlinkSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { checkRecords, fold, parseRecordText, stemOf } from '../lib/event-record.js';
import { planSeed, seedEvents } from '../lib/events-seed.js';

const HERE = dirname(fileURLToPath(import.meta.url));
const CLI = join(HERE, '..', 'cli.js');
const FIXTURES = join(HERE, '..', '..', '..', 'crates', 'scout-corpus', 'tests', 'fixtures', 'event-records');
const CASES = JSON.parse(readFileSync(join(FIXTURES, 'verdicts.json'), 'utf8')).cases;
const byVerdict = (v) => CASES.filter((c) => c.verdict === v);

/** Put the case's file into `dir`: a copy of records/<file>, or what its `make` describes. */
function materialise(c, dir) {
  const to = join(dir, c.file);
  const make = c.make;
  if (!make) copyFileSync(join(FIXTURES, 'records', c.file), to);
  else if (make.pad) writeFileSync(to, Buffer.concat([Buffer.from(make.pad.text), Buffer.alloc(make.pad.bytes - Buffer.byteLength(make.pad.text), make.pad.byte)]));
  else if (make.repeat) writeFileSync(to, make.repeat.prefix + make.repeat.unit.repeat(make.repeat.count) + make.repeat.suffix);
  else if (make.hex) writeFileSync(to, Buffer.from(make.hex, 'hex'));
  else if (make.symlink) symlinkSync(join(dir, make.symlink), to);
  else if (make.dir) mkdirSync(to);
  else throw new Error(`${c.file}: unknown make`);
}

const folder = (cases) => {
  const dir = mkdtempSync(join(tmpdir(), 'event-records-'));
  for (const c of cases) materialise(c, dir);
  return dir;
};

/** Every entry of `dir` with what it is and holds, so "nothing was written" can be asserted. */
const listing = (dir) => Object.fromEntries(readdirSync(dir).sort().map((f) => {
  const st = lstatSync(join(dir, f));
  return [f, st.isSymbolicLink() ? `link:${readlinkSync(join(dir, f))}` : st.isDirectory() ? 'dir' : readFileSync(join(dir, f)).toString('hex').slice(0, 4096) + `:${st.size}`];
}));

/** An archive with two monthly events, each tagged once. */
function archive() {
  const root = mkdtempSync(join(tmpdir(), 'event-records-archive-'));
  const put = (rel, text) => { mkdirSync(dirname(join(root, rel)), { recursive: true }); writeFileSync(join(root, rel), text); };
  put('analysis/context/briefings/2026-09-events.json', JSON.stringify({ month: '2026-09', events: [{ id: 'E01', name: 'First story' }, { id: 'E50', name: 'Second story' }] }));
  put('analysis/topics/current.jsonl', ['E01', 'E50'].map((e, i) => JSON.stringify({ key: `x:${i}`, date: '2026-09-01', themes: [], event: `2026-09/${e}` })).join('\n') + '\n');
  return root;
}

test('the seed check gives every shared case the verdict the reader gives it', () => {
  const dir = folder(CASES);
  const got = new Map(checkRecords(dir).map((v) => [v.file, v]));
  const wrong = [];
  for (const c of CASES) {
    const v = got.get(c.file);
    const verdict = !v ? 'ignored' : v.ok ? 'accept' : 'reject';
    if (verdict !== c.verdict) wrong.push(`${c.file}: want ${c.verdict}, the seed says ${verdict}${v?.why ? ` (${v.why})` : ''}`);
    else if (verdict === 'reject' && !v.why.includes(c.why)) wrong.push(`${c.file}: want ${JSON.stringify(c.why)} in ${JSON.stringify(v.why)}`);
    else if (verdict === 'accept') assert.equal(v.record.id, stemOf(c.file));
  }
  assert.deepEqual(wrong, []);
  assert.equal(got.size, CASES.length - byVerdict('ignored').length);
  // Every kind is there.
  assert.ok(byVerdict('accept').length >= 15 && byVerdict('reject').length >= 35 && byVerdict('ignored').length >= 3);
});

test('every file of the shared folder is a case', () => {
  assert.deepEqual(readdirSync(join(FIXTURES, 'records')).sort(), CASES.filter((c) => !c.make).map((c) => c.file).sort());
});

test('an accepted record is read as the reader reads it', () => {
  const read = (file) => parseRecordText(readFileSync(join(FIXTURES, 'records', file), 'utf8'), stemOf(file));
  assert.deepEqual(read('valid-full.md'), { id: 'valid-full', name: 'He said: "it\'s" é', start: '2026-09-01', end: null, origin: 'model', edited: null, merged_into: null, aliases: ["O'Reilly: x", 'plain'], members: ['2026-09/E01', '2026-10/E04'], themes: [] });
  assert.equal(read('valid-minimal.md').name, 'valid-minimal');
  assert.deepEqual([read('2026.md').id, read('2026.md').name], ['2026', 'true']);
  assert.deepEqual(read('valid-quoted-null.md').members, ['2026-09/E03', '~']);
  assert.equal(read('valid-quoted-null.md').name, 'null');
  assert.equal(read('valid-padded-id.md').id, 'valid-padded-id');
  assert.equal(read('double.md.md').id, 'double');
  // Ids compare as the reader compares them.
  assert.equal(fold('  Twin   ID’s '), "twin id's");
  assert.equal(fold('twin  id'), fold('twin id'));
});

test('the review case: a record with a valid id and members and a wrongly typed name stops the seed', () => {
  const root = archive();
  const out = join(root, 'analysis', 'events');
  mkdirSync(out);
  writeFileSync(join(out, 'a.md'), '---\nid: a\nname: [Oops]\nmembers: [2026-09/E01]\n---\n');
  const before = listing(out);
  for (const apply of [false, true]) {
    assert.throws(() => seedEvents({ archiveRoot: root, apply }), (err) => {
      assert.match(err.message, /analysis\/events\/a\.md: cannot read the record \(name is not a single value\)/);
      assert.match(err.message, /nothing was written/);
      return true;
    });
    assert.deepEqual(listing(out), before);
  }
  assert.throws(() => planSeed({ archiveRoot: root, outDir: out }), /name is not a single value/);
  // The command line: exit 1, the file and the rule on stderr, nothing on stdout, nothing written.
  const run = spawnSync(process.execPath, [CLI, 'events-seed', '--archive', root, '--apply'], { encoding: 'utf8' });
  assert.equal(run.status, 1);
  assert.equal(run.stdout, '');
  assert.match(run.stderr, /a\.md: cannot read the record \(name is not a single value\)/);
  assert.deepEqual(listing(out), before);
  // Mended, the record holds its event again and only the other event is seeded.
  writeFileSync(join(out, 'a.md'), '---\nid: a\nname: Oops\nmembers: [2026-09/E01]\n---\n');
  const r = seedEvents({ archiveRoot: root, apply: true });
  assert.deepEqual([r.already_in_a_record, r.created], [1, 1]);
  assert.deepEqual(readdirSync(out).sort(), ['2026-09-second-story.md', 'a.md']);
});

test('each refused kind of record stops the seed before anything is written, naming the file and the rule', () => {
  const valid = CASES.find((c) => c.file === 'valid-minimal.md');
  for (const c of byVerdict('reject')) {
    const root = archive();
    const out = join(root, 'analysis', 'events');
    mkdirSync(out);
    // Beside a valid record; the repeated id needs the file it repeats.
    for (const x of [valid, c, ...(c.file === 'twin id.md' ? [CASES.find((k) => k.file === 'twin  id.md')] : [])]) materialise(x, out);
    const before = listing(out);
    assert.throws(() => seedEvents({ archiveRoot: root, apply: true }), (err) => {
      assert.ok(err.message.includes(`${join(out, c.file)}: cannot read the record (`), `${c.file}: ${err.message.slice(0, 300)}`);
      assert.ok(err.message.includes(c.why), `${c.file}: ${err.message.slice(0, 300)}`);
      return true;
    }, c.file);
    assert.deepEqual(listing(out), before, c.file);
  }
});

test('several refused records are all named in one stop', () => {
  const root = archive();
  const out = join(root, 'analysis', 'events');
  mkdirSync(out);
  for (const f of ['name-list.md', 'no-id.md', 'valid-full.md']) materialise(CASES.find((c) => c.file === f), out);
  assert.throws(() => seedEvents({ archiveRoot: root, apply: true }), /name-list\.md: cannot read the record \(name is not a single value\)\n.*no-id\.md: cannot read the record \(no id\)\nthese 2 records are not read by scout/);
  assert.equal(readdirSync(out).length, 3);
});

test('the accepted and ignored cases together let the seed run, and their members count as held', () => {
  const root = archive();
  const out = join(root, 'analysis', 'events');
  mkdirSync(out);
  for (const c of [...byVerdict('accept'), ...byVerdict('ignored')]) materialise(c, out);
  const names = readdirSync(out);
  // valid-full.md and valid-contested-member.md hold 2026-09/E01; nothing holds 2026-09/E50.
  const r = seedEvents({ archiveRoot: root, apply: true });
  assert.deepEqual([r.already_in_a_record, r.created, r.skipped_existing_file], [1, 1, 0]);
  assert.deepEqual(readdirSync(out).filter((f) => !names.includes(f)), ['2026-09-second-story.md']);
  assert.ok(existsSync(join(out, '.hidden.md')));
  // What the seed wrote is a record the same check reads.
  assert.ok(checkRecords(out).every((v) => v.ok));
});

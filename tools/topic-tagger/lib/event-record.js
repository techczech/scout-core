// The event-record contract, as the Rust reader applies it (crates/scout-corpus/src/events.rs: `scan`,
// `parse_record`, `EventSet::from_records`). The seed stage uses this to decide whether an existing record is one
// the reader will read; a record that fails here is one the reader skips.
//
// Kept in step with the reader by the shared cases in crates/scout-corpus/tests/fixtures/event-records/: the Rust
// test (tests/event_record_fixtures.rs) and the Node test (test/event-record-fixtures.test.js) both run over that
// folder and must give the verdict `verdicts.json` names for every file. A rule added to either side needs a case
// there.
//
// The rules, in the reader's order. A file of the records folder is a record when its name ends in `.md` and does
// not start with `.`; anything else is left alone. A record is refused when it:
//   - is not a plain file (a symlink or a folder), cannot be read, or is larger than MAX_RECORD_BYTES;
//   - is not valid UTF-8;
//   - has no frontmatter (an optional BOM, a first line `---`, a later line that is `---` with nothing but
//     whitespace after it);
//   - has frontmatter that is not YAML, uses a YAML alias anywhere, nests deeper than MAX_YAML_DEPTH levels, has a
//     top-level key that is not a plain name, or repeats a top-level key;
//   - has frontmatter that is not a mapping;
//   - has `id`, `name`, `start`, `end`, `origin`, `edited` or `merged_into` that is a list or a mapping;
//   - has no `id` (absent, empty, `~` or `null`), or an `id` that is not the file name without `.md`;
//   - has `aliases`, `members` or `themes` that is neither empty nor a list of single values;
//   - repeats the id of a record read before it (file-name order; ids compare under `fold`).
// Every scalar is text (`id: 123` is the id "123"); an unquoted empty value, `~`, `null`, `Null` or `NULL` is no
// value. Other keys are ignored. A `merged_into` naming no record and a member another record already holds are
// warnings in the reader, not refusals, and pass here.
//
// Where the two YAML libraries could disagree this side refuses: a refusal stops the seed with the file named,
// while a wrong acceptance would leave an event with no usable record. Known case: a second YAML document inside
// the frontmatter (`--- text` on a line) is refused here and ignored by the reader.
import { closeSync, constants, fstatSync, lstatSync, openSync, readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { isAlias, isMap, isScalar, isSeq, parseDocument } from 'yaml';

/** Largest record file read, in bytes (`events::MAX_RECORD_BYTES`). */
export const MAX_RECORD_BYTES = 1 << 20;
/** Deepest YAML nesting read (`events::MAX_YAML_DEPTH`). */
export const MAX_YAML_DEPTH = 3;

// Unicode White_Space, as Rust's `str::trim` and the query grammar's `WS` have it.
const WS = '\\t\\n\\v\\f\\r \\u0085\\u00a0\\u1680\\u2000-\\u200a\\u2028\\u2029\\u202f\\u205f\\u3000';
const TRIM_RE = new RegExp(`^[${WS}]+|[${WS}]+$`, 'g');
const TRIM_END_RE = new RegExp(`[${WS}]+$`);
const WS_RUN_RE = new RegExp(`[${WS}]+`, 'g');
const QUOTES = { '‘': "'", '’': "'", '‚': "'", '‛': "'", '′': "'", 'ʼ': "'", '“': '"', '”': '"', '„': '"', '‟': '"', '″': '"' };
const NULLS = new Set(['', '~', 'null', 'Null', 'NULL']);
const SCALAR_FIELDS = ['name', 'start', 'end', 'origin', 'edited', 'merged_into'];
const LIST_FIELDS = ['aliases', 'members', 'themes'];

const trim = (s) => s.replace(TRIM_RE, '');

/** The reader's matching rule for ids (`normalize::fold`): NFC, quotes unified, whitespace runs to one space, trimmed, lower case. */
export function fold(s) {
  const text = trim(String(s).normalize('NFC').replace(WS_RUN_RE, ' '));
  return [...text].map((c) => (QUOTES[c] ?? c).toLowerCase()).join('');
}

/** The file name without `.md` (every trailing `.md`, as the reader cuts it). */
export function stemOf(name) {
  let stem = name;
  while (stem.endsWith('.md')) stem = stem.slice(0, -3);
  return stem;
}

/** Is this entry of the records folder a record file name? */
export const isRecordName = (name) => name.endsWith('.md') && !name.startsWith('.');

/** The frontmatter block of `text`, or null when it has none. */
function frontmatterOf(text) {
  const t = text.startsWith('﻿') ? text.slice(1) : text;
  const rest = t.startsWith('---\r\n') ? t.slice(5) : t.startsWith('---\n') ? t.slice(4) : null;
  if (rest === null) return null;
  for (let at = 0; at < rest.length;) {
    const nl = rest.indexOf('\n', at);
    const end = nl === -1 ? rest.length : nl + 1;
    if (rest.slice(at, end).replace(TRIM_END_RE, '') === '---') return rest.slice(0, at);
    at = end;
  }
  return null;
}

const isNull = (node) => node == null || (isScalar(node) && (node.type === 'PLAIN' || node.type === undefined) && NULLS.has(String(node.value ?? '')));

/** A top-level value as the reader sees it: {null}, {str}, {list} or {other} (a mapping, or a list holding one). */
function valueOf(node) {
  if (isNull(node)) return { null: true };
  if (isScalar(node)) return { str: String(node.value) };
  if (isSeq(node) && node.items.every((x) => x == null || isScalar(x))) return { list: node.items.filter((x) => !isNull(x)).map((x) => String(x.value)) };
  return { other: true };
}

/** The top-level mapping of the frontmatter as Map(key -> value); throws Error(rule) when the reader refuses it. */
function readFlat(front) {
  let doc;
  try {
    // failsafe: every scalar is text, as in the reader. A stack overflow in the library is a refusal too.
    doc = parseDocument(front, { schema: 'failsafe', uniqueKeys: false, merge: false, strict: true, prettyErrors: false, logLevel: 'silent' });
  } catch (err) {
    throw new Error(`bad YAML (${err.message})`);
  }
  if (doc.errors.length) throw new Error(`bad YAML (${doc.errors[0].message.split('\n')[0]})`);
  // Every node, without recursion: an alias anywhere and a collection below the third level are refused.
  const stack = [[doc.contents, 1]];
  while (stack.length) {
    const [node, level] = stack.pop();
    if (node == null) continue;
    if (isAlias(node)) throw new Error('bad YAML (YAML aliases are not read)');
    const children = isMap(node) ? node.items.flatMap((p) => [p.key, p.value]) : isSeq(node) ? node.items : null;
    if (!children) continue;
    if (level > MAX_YAML_DEPTH) throw new Error(`bad YAML (nested deeper than ${MAX_YAML_DEPTH} levels)`);
    for (const child of children) stack.push([child, level + 1]);
  }
  const top = doc.contents;
  const map = new Map();
  if (isMap(top)) {
    for (const pair of top.items) {
      if (pair.key != null && !isScalar(pair.key)) throw new Error('bad YAML (a key is not a plain name)');
      const key = pair.key == null ? '' : String(pair.key.value ?? '');
      if (map.has(key)) throw new Error(`bad YAML (key ${JSON.stringify(key)} appears twice)`);
      map.set(key, valueOf(pair.value));
    }
  }
  if (!isMap(top)) throw new Error('frontmatter is not a mapping');
  return map;
}

/**
 * Check one record's text against the reader's contract; `stem` is its file name without `.md`.
 * Returns the fields the reader would read; throws Error(rule) when the reader would skip the file.
 */
export function parseRecordText(text, stem) {
  const front = frontmatterOf(text);
  if (front === null) throw new Error('no frontmatter');
  const map = readFlat(front);
  const scalar = (k) => {
    const v = map.get(k);
    if (!v || v.null) return null;
    if ('str' in v) return trim(v.str) || null;
    throw new Error(`${k} is not a single value`);
  };
  const id = scalar('id');
  if (id === null) throw new Error('no id');
  if (id !== stem) throw new Error(`id ${JSON.stringify(id)} does not match the file name`);
  const out = { id };
  for (const k of SCALAR_FIELDS) out[k] = scalar(k);
  out.name ??= id;
  for (const k of LIST_FIELDS) {
    const v = map.get(k);
    if (!v || v.null) out[k] = [];
    else if (v.list) out[k] = v.list.map(trim).filter(Boolean);
    else throw new Error(`${k} is not a list`);
  }
  return out;
}

/** The bytes of a plain file of at most MAX_RECORD_BYTES, never through a link; throws Error(rule) otherwise. */
function readPlain(path) {
  const st = lstatSync(path, { throwIfNoEntry: false });
  if (!st) throw new Error('cannot be read');
  if (!st.isFile()) throw new Error('not a plain file');
  if (st.size > MAX_RECORD_BYTES) throw new Error('larger than 1 MiB');
  let fd;
  try {
    fd = openSync(path, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK);
  } catch {
    throw new Error('cannot be read');
  }
  try {
    if (!fstatSync(fd).isFile()) throw new Error('not a plain file');
    const buf = readFileSync(fd);
    if (buf.length > MAX_RECORD_BYTES) throw new Error('larger than 1 MiB');
    return buf;
  } catch (err) {
    if (/^(not a plain file|larger than 1 MiB)$/.test(err.message)) throw err;
    throw new Error('cannot be read');
  } finally {
    closeSync(fd);
  }
}

/**
 * The verdict on every record file of `dir`, in the reader's order (file names by their UTF-8 bytes):
 * [{file, ok: true, record} | {file, ok: false, why}]. A missing folder gives []. Reads only.
 */
export function checkRecords(dir) {
  let names;
  try {
    names = readdirSync(dir);
  } catch (err) {
    if (err.code === 'ENOENT' || err.code === 'ENOTDIR') return [];
    throw err;
  }
  const seen = new Set();
  const out = [];
  for (const file of names.filter(isRecordName).sort((a, b) => Buffer.compare(Buffer.from(a), Buffer.from(b)))) {
    try {
      const bytes = readPlain(join(dir, file));
      let text;
      try { text = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).decode(bytes); } catch { throw new Error('not valid UTF-8'); }
      const record = parseRecordText(text, stemOf(file));
      const key = fold(record.id);
      if (!key || seen.has(key)) throw new Error(`repeated or empty id ${JSON.stringify(record.id)}`);
      seen.add(key);
      out.push({ file, ok: true, record });
    } catch (err) {
      out.push({ file, ok: false, why: err.message });
    }
  }
  return out;
}

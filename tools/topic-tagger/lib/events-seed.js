// Seed stage for lasting events (spec: archive-scout docs/specs/2026-10-11-lasting-events-spec.md).
// Deterministic: no model call, no network. Every monthly event (every event of every briefing file, with or
// without tagged items, plus any event id only the tag rows name) that is not yet in a record's `members`
// becomes one record file `<out>/<id>.md`.
//
// Reads (never writes): <archive>/analysis/context/briefings/<period>-events.json (names with `.superseded-`
// are ignored; the folder is not walked) and <archive>/analysis/topics/current.jsonl (model tag rows;
// corrections.jsonl is not applied here, the reader applies it at query time).
// Writes (only with apply): new record files, each created with O_CREAT|O_EXCL|O_NOFOLLOW directly inside the
// out folder, and the out folder itself (with any missing folder above it) when it is missing. Nothing else: no temporary file, no log file, and
// no existing file is ever opened for writing, renamed or removed. The run's counts go to the caller (the CLI
// prints them as one JSON line).
//
// The out folder: the path given (made absolute, `.` and `..` removed), which must be a real folder and not a
// symlink. Symlinks in the folders ABOVE it are accepted (macOS /tmp and /var are such links) and resolved once;
// every record is created under that resolved path, which the report gives as `out`.
import { closeSync, constants, existsSync, lstatSync, mkdirSync, openSync, readdirSync, readFileSync, realpathSync, unlinkSync, writeSync } from 'node:fs';
import { basename, dirname, join, resolve } from 'node:path';
import { isDeepStrictEqual } from 'node:util';
import { parse, stringify } from 'yaml';

export const SLUG_MAX = 60;
export const THEMES_MAX = 5;
export const TOOL = 'topic-tagger events-seed';
const ID_RE = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;
const EVENTS_FILE_RE = /^(\d{4}-(?:(?:0[1-9]|1[0-2])|Q[1-4]))-events\.json$/;
const MONTHLY_ID_RE = /^(\d{4}-(?:(?:0[1-9]|1[0-2])|Q[1-4]))\/(E\d+)$/;
const YAML_OPTS = { lineWidth: 0, nullStr: '' };

/** Lower-case ASCII words joined by hyphens, at most `max` chars; '' when the name has no ASCII letter or digit. */
export function slugify(name, max = SLUG_MAX) {
  const ascii = String(name ?? '').normalize('NFKD').replace(/[̀-ͯ]/g, '').toLowerCase();
  const slug = ascii.replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '');
  return slug.slice(0, max).replace(/-+$/, '');
}

/** The month an id starts with: the event's month, or the first month of its quarter (2022-Q2 -> 2022-04). */
export function periodMonth(period) {
  const q = /^(\d{4})-Q([1-4])$/.exec(period);
  return q ? `${q[1]}-${String((Number(q[2]) - 1) * 3 + 1).padStart(2, '0')}` : period;
}

/** First and last day of a month or quarter period. */
export function periodSpan(period) {
  const first = periodMonth(period);
  const [y, m] = first.split('-').map(Number);
  const lastMonth = /Q/.test(period) ? m + 2 : m;
  const lastDay = new Date(Date.UTC(y, lastMonth, 0)).getUTCDate();
  return { start: `${first}-01`, end: `${y}-${String(lastMonth).padStart(2, '0')}-${String(lastDay).padStart(2, '0')}` };
}

/**
 * The record id for a name: `<month>-<slug>`, the slug cut so that a `-2`, `-3` … suffix still fits SLUG_MAX.
 * `taken` holds ids in use (lower case); the chosen id is added to it.
 * Returns {id, base, fallback (the name gave no slug), truncated, suffix (0 = none)}.
 */
export function assignId(period, number, name, taken) {
  const full = slugify(name, Number.MAX_SAFE_INTEGER);
  const fallback = full === '';
  const whole = fallback ? `event-${String(number).toLowerCase().replace(/[^a-z0-9]/g, '') || 'x'}` : full;
  const month = periodMonth(period);
  for (let n = 1; ; n++) {
    const suffix = n === 1 ? '' : `-${n}`;
    const slug = whole.slice(0, SLUG_MAX - suffix.length).replace(/-+$/, '') + suffix;
    const id = `${month}-${slug}`;
    if (!taken.has(id)) {
      taken.add(id);
      return { id, base: `${month}-${whole.slice(0, SLUG_MAX).replace(/-+$/, '')}`, fallback, truncated: whole.length > SLUG_MAX, suffix: n === 1 ? 0 : n };
    }
  }
}

/** Themes carried by at least a third of the items: most frequent first, then alphabetical, at most THEMES_MAX. */
export function majorityThemes(items) {
  const counts = new Map();
  for (const it of items) for (const t of new Set((it.themes || []).filter((x) => typeof x === 'string' && x.trim()))) counts.set(t, (counts.get(t) || 0) + 1);
  return [...counts.entries()]
    .filter(([, c]) => c * 3 >= items.length)
    .sort((a, b) => b[1] - a[1] || (a[0] < b[0] ? -1 : a[0] > b[0] ? 1 : 0))
    .slice(0, THEMES_MAX)
    .map(([t]) => t);
}

const isDate = (s) => typeof s === 'string' && /^\d{4}-\d{2}-\d{2}/.test(s);
const byMonthlyId = (a, b) => {
  const [, pa, na] = MONTHLY_ID_RE.exec(a);
  const [, pb, nb] = MONTHLY_ID_RE.exec(b);
  return (pa < pb ? -1 : pa > pb ? 1 : 0) || Number(na.slice(1)) - Number(nb.slice(1)) || (na < nb ? -1 : na > nb ? 1 : 0);
};

/** Monthly events from the briefings folder, by monthly id (`2026-09/E21`). */
export function readMonthlyEvents(archiveRoot) {
  const dir = join(archiveRoot, 'analysis', 'context', 'briefings');
  const out = new Map();
  if (!existsSync(dir)) return out;
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    if (e.name.includes('.superseded-') || !e.isFile()) continue;
    const m = EVENTS_FILE_RE.exec(e.name);
    if (!m) continue;
    const file = join(dir, e.name);
    let doc;
    try { doc = JSON.parse(readFileSync(file, 'utf8')); } catch (err) { throw new Error(`${file}: not JSON (${err.message})`); }
    if (doc?.month !== undefined && doc.month !== m[1]) throw new Error(`${file}: "month" is ${JSON.stringify(doc.month)}, the file name says ${m[1]}`);
    if (!Array.isArray(doc?.events)) throw new Error(`${file}: no "events" list`);
    for (const ev of doc.events) {
      const id = `${m[1]}/${ev?.id}`;
      if (!MONTHLY_ID_RE.test(id)) throw new Error(`${file}: event id ${JSON.stringify(ev?.id)} is not E<number>`);
      if (out.has(id)) throw new Error(`${file}: event ${id} appears twice`);
      out.set(id, {
        name: typeof ev.name === 'string' ? ev.name.replace(/\s+/g, ' ').trim() : '',
        date: isDate(ev.date) ? ev.date.slice(0, 10) : null,
        aliases: Array.isArray(ev.aliases) ? ev.aliases.filter((a) => typeof a === 'string' && a.trim()).map((a) => a.replace(/\s+/g, ' ').trim()) : [],
        description: typeof ev.description === 'string' ? ev.description.trim() : '',
      });
    }
  }
  return out;
}

/** Tag rows grouped by their `event` (the last row per key counts, as the reader does). */
export function readTaggedItems(archiveRoot) {
  const file = join(archiveRoot, 'analysis', 'topics', 'current.jsonl');
  const rows = new Map();
  if (!existsSync(file)) return new Map();
  for (const line of readFileSync(file, 'utf8').split('\n')) {
    if (!line.trim()) continue;
    let r;
    try { r = JSON.parse(line); } catch { continue; }
    if (r && typeof r.key === 'string' && r.key.trim()) rows.set(r.key.trim(), r);
  }
  const byEvent = new Map();
  for (const r of rows.values()) {
    if (typeof r.event !== 'string' || !MONTHLY_ID_RE.test(r.event.trim())) continue;
    const id = r.event.trim();
    if (!byEvent.has(id)) byEvent.set(id, []);
    byEvent.get(id).push(r);
  }
  return byEvent;
}

/** Split a record file into frontmatter object and body; throws when it is not a record. */
export function parseRecord(text) {
  const t = text.replace(/^\ufeff/, '');
  const m = /^---\r?\n([\s\S]*?)\r?\n---[ \t]*(?:\r?\n|$)/.exec(t);
  if (!m) throw new Error('no frontmatter');
  const fm = parse(m[1]);
  if (!fm || typeof fm !== 'object' || Array.isArray(fm)) throw new Error('frontmatter is not a mapping');
  return { frontmatter: fm, body: t.slice(m[0].length) };
}

/** Records already in `outDir`: ids in use and the monthly ids they hold. Any unreadable record stops the run. */
export function readExistingRecords(outDir) {
  const ids = new Set();
  const members = new Map();
  if (!existsSync(outDir)) return { ids, members };
  for (const e of readdirSync(outDir, { withFileTypes: true })) {
    if (!e.name.endsWith('.md') || e.name.startsWith('.')) continue;
    const file = join(outDir, e.name);
    const stem = e.name.slice(0, -3);
    let fm;
    try { ({ frontmatter: fm } = parseRecord(readFileSync(file, 'utf8'))); } catch (err) { throw new Error(`${file}: cannot read the record (${err.message}); fix or remove it before seeding`); }
    if (fm.id !== stem) throw new Error(`${file}: id ${JSON.stringify(fm.id)} does not match the file name; fix it before seeding`);
    if (fm.members != null && !(Array.isArray(fm.members) && fm.members.every((x) => typeof x === 'string'))) throw new Error(`${file}: members is not a list of ids; fix it before seeding`);
    ids.add(stem.toLowerCase());
    for (const mem of fm.members || []) members.set(mem.trim().toLowerCase(), stem);
  }
  return { ids, members };
}

/** The file text of a record. The frontmatter is written by the YAML library and read back as a check. */
export function renderRecord(frontmatter, body) {
  const yaml = stringify(frontmatter, YAML_OPTS);
  if (!isDeepStrictEqual(parse(yaml), frontmatter)) throw new Error(`record ${frontmatter.id}: frontmatter does not read back as written`);
  return `---\n${yaml}---\n\n${body ? `${body}\n` : ''}`;
}

/**
 * Work out the records a seed run would create. Pure reading.
 * Returns {records: [{id, member, text, items}], counts and notes}.
 */
export function planSeed({ archiveRoot, outDir }) {
  const monthly = readMonthlyEvents(archiveRoot);
  const tagged = readTaggedItems(archiveRoot);
  const existing = readExistingRecords(outDir);
  const taken = new Set(existing.ids);
  const records = [];
  const notes = { collisions: [], fallback_slugs: [], truncated_slugs: [], non_ascii_names: [], no_briefing_entry: [], month_dates: [] };
  let alreadyMembers = 0;
  // Every briefing event, tagged or not, and every event id only the tag rows name.
  const all = [...new Set([...monthly.keys(), ...tagged.keys()])].sort(byMonthlyId);
  for (const member of all) {
    if (existing.members.has(member.toLowerCase())) { alreadyMembers++; continue; }
    const items = tagged.get(member) || [];
    const [, period, number] = MONTHLY_ID_RE.exec(member);
    const brief = monthly.get(member);
    if (!brief) notes.no_briefing_entry.push(member);
    const tagName = items.map((r) => r.event_name).find((n) => typeof n === 'string' && n.trim());
    const name = brief?.name || (tagName ? tagName.replace(/\s+/g, ' ').trim() : '') || member;
    const picked = assignId(period, number, name, taken);
    if (!ID_RE.test(picked.id)) throw new Error(`${member}: id ${JSON.stringify(picked.id)} is not a slug`);
    if (picked.suffix) notes.collisions.push({ id: picked.id, base: picked.base, member });
    if (picked.fallback) notes.fallback_slugs.push({ id: picked.id, name, member });
    if (picked.truncated) notes.truncated_slugs.push({ id: picked.id, name, member });
    // eslint-disable-next-line no-control-regex
    if (/[^\x00-\x7f]/.test(name)) notes.non_ascii_names.push({ id: picked.id, name, member });
    // No dated item (or no item at all): the first and last day of the event's month or quarter.
    const dates = items.map((r) => r.date).filter(isDate).map((d) => d.slice(0, 10)).sort();
    const span = dates.length ? { start: dates[0], end: dates[dates.length - 1] } : periodSpan(period);
    if (!dates.length) notes.month_dates.push(member);
    const frontmatter = {
      id: picked.id,
      name,
      aliases: brief?.aliases || [],
      start: span.start,
      end: span.end,
      members: [member],
      themes: majorityThemes(items),
      origin: 'model',
      edited: null,
      merged_into: null,
    };
    records.push({ id: picked.id, member, items: items.length, text: renderRecord(frontmatter, brief?.description || '') });
  }
  const withoutItems = [...monthly.keys()].filter((id) => !tagged.has(id)).sort(byMonthlyId);
  return {
    records,
    counts: {
      monthly_events: all.length,
      briefing_events: monthly.size,
      monthly_events_with_items: tagged.size,
      briefing_events_without_items: withoutItems.length,
      already_in_a_record: alreadyMembers,
      to_create: records.length,
      no_items: records.filter((r) => r.items === 0).length,
      one_item: records.filter((r) => r.items === 1).length,
    },
    notes: { ...notes, briefing_events_without_items: withoutItems },
  };
}

/**
 * The folder records are created in: `outDir` made absolute, required to be a real folder (not a symlink, not a
 * file), with the symlinks of the folders above it resolved. With `create` it is made when missing; without, a
 * missing folder is returned as given (nothing is written in a dry run).
 */
export function resolveOutDir(outDir, { create = false } = {}) {
  const wanted = resolve(outDir);
  // Looked at before anything is made, and without following: a link here (dangling or not) is refused.
  let st = lstatSync(wanted, { throwIfNoEntry: false });
  if (!st && create) {
    mkdirSync(wanted, { recursive: true });
    st = lstatSync(wanted);
  }
  if (!st) return wanted;
  if (st.isSymbolicLink()) throw new Error(`${wanted}: the out folder is a symlink; give the folder itself`);
  if (!st.isDirectory()) throw new Error(`${wanted}: the out folder is not a folder`);
  return realpathSync(wanted);
}

/**
 * Create `<dir>/<name>` holding `text`, only if no entry of that name is there (file, folder, or a link of any
 * kind); false when one is. O_EXCL makes the new file itself, so the write cannot land in a file that existed
 * (through a symlink or a hard link), and nothing else is created. `name` must be a bare `<slug>.md`.
 */
export function createExclusive(dir, name, text) {
  if (!/^[a-z0-9]+(?:-[a-z0-9]+)*\.md$/.test(name)) throw new Error(`${name}: not a record file name`);
  const target = join(dir, name);
  if (dirname(target) !== dir || basename(target) !== name) throw new Error(`${name}: outside the out folder`);
  let fd;
  try {
    fd = openSync(target, constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW, 0o644);
  } catch (err) {
    if (err.code === 'EEXIST') return false;
    throw err;
  }
  try {
    const buf = Buffer.from(text, 'utf8');
    for (let at = 0; at < buf.length;) at += writeSync(fd, buf, at);
  } catch (err) {
    // The file is this call's own (O_EXCL made it): take the partial record away.
    closeSync(fd);
    fd = undefined;
    unlinkSync(target);
    throw err;
  } finally {
    if (fd !== undefined) closeSync(fd);
  }
  return true;
}

/**
 * The seed stage. Without `apply` nothing is written (not even the out folder). With it, the only files written
 * are new records in the out folder. Returns the run's counts with `date`, `tool` and `version`.
 */
export function seedEvents({ archiveRoot, outDir, apply = false, now = new Date(), version = '0' }) {
  const dir = resolveOutDir(outDir || join(archiveRoot, 'analysis', 'events'), { create: apply });
  const plan = planSeed({ archiveRoot, outDir: dir });
  const report = { date: now.toISOString(), tool: TOOL, version, archive: archiveRoot, out: dir, apply, ...plan.counts, created: 0, skipped: plan.counts.already_in_a_record, skipped_existing_file: 0, notes: plan.notes };
  if (!apply) return report;
  for (const r of plan.records) {
    if (createExclusive(dir, `${r.id}.md`, r.text)) report.created++;
    else report.skipped_existing_file++;
  }
  report.skipped = plan.counts.already_in_a_record + report.skipped_existing_file;
  return report;
}

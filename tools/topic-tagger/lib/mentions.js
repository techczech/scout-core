// mentioned_works: deterministic join of an item's person/work entities to the archive's own works, so each item
// links to other readings across the whole archive. No model. Rules (strict on purpose; short names are ambiguous):
//  - person entity: folded exact match with a work's author or author_name, both sides at least 2 words
//    ("Terence Tao" yes; "Tao" or the handle "karpathy" no);
//  - work entity: folded exact match with a non-tweet work's title of at least 2 words;
//  - never the item itself; up to 5 keys, most-highlighted works first (then newest, then key).
import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { parseWork } from './works.js';
import { fold } from './entity-guard.js';

export const MAX_MENTIONED = 5;
const wordCount = (s) => fold(s).split(' ').filter(Boolean).length;

// One pass over every work (dated or not): lookup tables plus key -> display info.
export function buildWorkIndex(archiveRoot, { relBase = 'readings/works' } = {}) {
  const dir = join(archiveRoot, relBase);
  const byAuthor = new Map(), byTitle = new Map(), info = new Map();
  const add = (m, k, key) => { if (!m.has(k)) m.set(k, new Set()); m.get(k).add(key); };
  for (const f of readdirSync(dir).filter((n) => n.endsWith('.md')).sort()) {
    const w = parseWork(readFileSync(join(dir, f), 'utf8'), `${relBase}/${f}`);
    if (info.has(w.key)) continue;
    info.set(w.key, { key: w.key, path: w.path, title: w.title, author: w.author || w.authorName, date: w.date, type: w.type, url: w.url, highlights: w.highlights.length });
    for (const a of new Set([w.author, w.authorName])) if (a && wordCount(a) >= 2) add(byAuthor, fold(a), w.key);
    if (w.type !== 'tweet' && w.title && wordCount(w.title) >= 2) add(byTitle, fold(w.title), w.key);
  }
  return { byAuthor, byTitle, info };
}

/**
 * names: [{name, type}] (use raw and normalised names together). Returns [{key, via}] up to max, via = the entity name.
 */
export function mentionedWorks(index, entities, selfKey, { max = MAX_MENTIONED } = {}) {
  const via = new Map();
  for (const e of entities) {
    const f = fold(e.name);
    if (!f || wordCount(e.name) < 2) continue;
    const hits = [];
    if (e.type === 'person') hits.push(...(index.byAuthor.get(f) || []));
    if (e.type === 'work') hits.push(...(index.byTitle.get(f) || []));
    for (const k of hits) if (k !== selfKey && !via.has(k)) via.set(k, e.name);
  }
  return [...via.entries()]
    .map(([key, name]) => ({ key, via: name, w: index.info.get(key) }))
    .sort((a, b) => b.w.highlights - a.w.highlights || String(b.w.date).localeCompare(String(a.w.date)) || a.key.localeCompare(b.key))
    .slice(0, max).map(({ key, via: v }) => ({ key, via: v }));
}

// Month index: one line per item ("m0042 Vtrivedy10: gist", under a heading per day) so each item is read
// alongside the whole month. Short ids are assigned by date then key, so they are stable for a month.
import { clip } from './text.js';

export const MAX_GIST = 100;
export const MIN_GIST = 30;

const squash = (s) => String(s ?? '').replace(/\s+/g, ' ').trim();

export function assignShortIds(items) {
  const sorted = [...items].sort((a, b) => (a.date || '').localeCompare(b.date || '') || a.key.localeCompare(b.key));
  const width = Math.max(4, String(sorted.length).length);
  return sorted.map((it, i) => ({ ...it, sid: `m${String(i + 1).padStart(width, '0')}` }));
}

// Gist: tweet = its text (images/links removed); other works = title, then first highlight.
export function gistOf(item) {
  const first = squash((item.highlights || []).map((h) => h.text).join(' ')
    .replace(/!\[[^\]]*\]\([^)]*\)/g, '').replace(/🔗\s*\S+/g, '').replace(/https?:\/\/\S+/g, ''));
  if (item.type === 'tweet') return first || squash(item.title);
  return squash(item.title) + (first ? ` — ${first}` : '');
}

// Real Haiku 5.5 tokenisation of index lines is ~2.2-2.4 chars/token (ids, handles, numbers).
export const indexTokens = (s) => Math.ceil(s.length / 2.2);

// Tweets carry no type marker (most items); other types are marked [article] etc.
const lineFor = (it, gistChars) => {
  const g = gistOf(it);
  const gist = clip(g, gistChars);
  const type = it.type === 'tweet' ? '' : `[${it.type || '?'}] `;
  return `${it.sid} ${type}${squash(it.author).replace(/\s+/g, '_')}: ${gist}`;
};

// Lines grouped under one heading per day ("## 2026-09-04") so dates are not repeated per line.
function render(ordered, gistChars) {
  const out = [];
  let day = null;
  for (const it of ordered) {
    if (it.date !== day) { day = it.date; out.push(`## ${day || 'undated'}`); }
    out.push(lineFor(it, gistChars));
  }
  return out.join('\n');
}

/**
 * items must carry sid. Shrinks the gist length in steps of 10 until the index fits maxTokens
 * (estimate via `count`, default chars/2.2 = indexTokens). Returns {text, gistChars, estTokens, lines}.
 * Throws if even MIN_GIST does not fit: the caller must raise the budget or drop other context.
 */
export function buildMonthIndex(items, { maxTokens = 60000, maxGist = MAX_GIST, count = indexTokens } = {}) {
  const ordered = [...items].sort((a, b) => a.sid.localeCompare(b.sid));
  for (let g = maxGist; g >= MIN_GIST; g -= 10) {
    const text = render(ordered, g);
    const est = count(text);
    if (est <= maxTokens) return { text, gistChars: g, estTokens: est, lines: ordered.length };
  }
  throw new Error(`month index for ${ordered.length} items does not fit ${maxTokens} tokens even at ${MIN_GIST}-char gists`);
}

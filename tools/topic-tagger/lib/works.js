// Reads archive work files: frontmatter, highlights, stable key, month-range selection.
import { createHash } from 'node:crypto';
import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

export const ITEM_CHAR_CAP = 12000;

export function parseFrontmatter(text) {
  const m = text.match(/^---\n([\s\S]*?)\n---\n?/);
  if (!m) return { meta: {}, body: text };
  const meta = {};
  for (const line of m[1].split('\n')) {
    const kv = line.match(/^([A-Za-z_][\w-]*):\s*(.*)$/);
    if (!kv) continue;
    let v = kv[2].trim();
    if (/^".*"$/.test(v) || /^'.*'$/.test(v)) v = v.slice(1, -1);
    meta[kv[1]] = v;
  }
  return { meta, body: text.slice(m[0].length) };
}

// A highlight is a run of "> " lines closed by a "highlighted_at: YYYY-MM-DD" line.
export function parseHighlights(body) {
  const out = [];
  let buf = [];
  for (const line of body.split('\n')) {
    if (line.startsWith('>')) buf.push(line.replace(/^>\s?/, ''));
    else {
      const d = line.match(/^highlighted_at:\s*(\d{4}-\d{2}-\d{2})/);
      if (d) {
        out.push({ date: d[1], text: buf.join('\n').trim() });
        buf = [];
      }
    }
  }
  return out;
}

// Key = "<source_system>:<source_id>". If either is missing, fall back to a
// content hash of the whole file ("sha256:<16 hex>") and flag it.
export function deriveKey(meta, rawText) {
  if (meta.source_system && meta.source_id) {
    return { key: `${meta.source_system}:${meta.source_id}`, fallback: false };
  }
  const h = createHash('sha256').update(rawText).digest('hex').slice(0, 16);
  return { key: `sha256:${h}`, fallback: true };
}

export function parseWork(rawText, relPath) {
  const { meta, body } = parseFrontmatter(rawText);
  const highlights = parseHighlights(body);
  const dates = highlights.map((h) => h.date).sort();
  const { key, fallback } = deriveKey(meta, rawText);
  return {
    key,
    keyFallback: fallback,
    path: relPath,
    title: meta.title || '',
    author: meta.author || '',
    type: meta.type || '',
    url: meta.url || '',
    date: dates.length ? dates[dates.length - 1] : null,
    highlights,
  };
}

// Text sent to the model for one item, capped by characters.
export function itemText(work, cap = ITEM_CHAR_CAP) {
  const head = `Title: ${work.title}\nAuthor: ${work.author}\nType: ${work.type}\nDate: ${work.date}\nHighlights:\n`;
  const full = head + work.highlights.map((h) => h.text).filter(Boolean).join('\n---\n');
  if (full.length <= cap) return { text: full, capped: false };
  return { text: full.slice(0, cap), capped: true };
}

export function inRange(date, from, to) {
  return !!date && date >= from && date <= to;
}

// worksDir: .../readings/works. relBase: prefix stored in "path" (default "readings/works").
export function selectWorks(worksDir, from, to, { relBase = 'readings/works' } = {}) {
  const stats = { scanned: 0, undated: 0, selected: 0, keyFallbacks: 0, capped: 0, duplicateKeys: 0 };
  const items = [];
  const seen = new Set();
  for (const f of readdirSync(worksDir).filter((n) => n.endsWith('.md')).sort()) {
    stats.scanned++;
    const w = parseWork(readFileSync(join(worksDir, f), 'utf8'), `${relBase}/${f}`);
    if (!w.date) { stats.undated++; continue; }
    if (!inRange(w.date, from, to)) continue;
    if (seen.has(w.key)) { stats.duplicateKeys++; continue; }
    seen.add(w.key);
    const { text, capped } = itemText(w);
    w.text = text;
    w.capped = capped;
    if (capped) stats.capped++;
    if (w.keyFallback) stats.keyFallbacks++;
    items.push(w);
  }
  stats.selected = items.length;
  return { items, stats };
}

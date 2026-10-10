// Single-file review page: summary, then each topic with definition, count, 3 examples.
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { parseWork } from './works.js';

const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));

export function exampleFor(archiveRoot, rec) {
  let w;
  try { w = parseWork(readFileSync(join(archiveRoot, rec.path), 'utf8'), rec.path); } catch { w = null; }
  const text = w ? w.highlights.map((h) => h.text).join(' ').replace(/\s+/g, ' ').trim() : '';
  return {
    key: rec.key,
    author: w ? w.author : '',
    title: w ? w.title : '',
    date: rec.date,
    url: w ? w.url : '',
    snippet: text.slice(0, 200) + (text.length > 200 ? '…' : ''),
  };
}

const CSS = `:root{color-scheme:light dark;--bg:#fff;--fg:#1c1c1e;--mut:#6b6b70;--line:#ddd;--card:#f6f6f7;--a:#0b57d0}
@media (prefers-color-scheme:dark){:root{--bg:#161618;--fg:#e8e8ea;--mut:#9a9aa0;--line:#333;--card:#212124;--a:#8ab4f8}}
body{font:16px/1.5 system-ui,sans-serif;background:var(--bg);color:var(--fg);margin:0 auto;padding:1rem;max-width:52rem}
h1{font-size:1.4rem}h2{font-size:1.1rem;margin:0}a{color:var(--a)}
.sum{display:flex;flex-wrap:wrap;gap:.5rem 1.5rem;background:var(--card);padding:.75rem 1rem;border-radius:8px}
.sum b{display:block;font-size:1.3rem}.sum span{color:var(--mut);font-size:.85rem}
.topic{border-top:1px solid var(--line);padding:1rem 0}.def{margin:.2rem 0 .6rem;color:var(--mut)}
.n{font-weight:400;color:var(--mut)}.ex{background:var(--card);border-radius:6px;padding:.5rem .75rem;margin:.4rem 0;font-size:.92rem}
.meta{color:var(--mut);font-size:.8rem;overflow-wrap:anywhere}`;

// rows: [{topic,definition,count,examples:[key]}]; examples: key -> {author,date,url,snippet}
export function renderReview({ run, rows, examples, unmappedCount = 0 }) {
  const aiPct = run.item_count ? Math.round((100 * run.ai_count) / run.item_count) : 0;
  const stat = (v, l) => `<div><b>${esc(v)}</b><span>${esc(l)}</span></div>`;
  const topics = rows.map((t) => `<section class="topic"><h2>${esc(t.topic)} <span class="n">(${t.count})</span></h2>
<p class="def">${esc(t.definition)}</p>
${t.examples.map((k) => {
    const e = examples[k] || {};
    const link = e.url ? `<a href="${esc(e.url)}">${esc(e.url)}</a>` : '';
    return `<div class="ex"><div class="meta">${esc(e.author)} · ${esc(e.date)} · ${link}</div><div>${esc(e.snippet)}</div></div>`;
  }).join('\n')}
</section>`).join('\n');
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Topic review: ${esc(run.run_id)}</title><style>${CSS}</style></head><body>
<h1>Topic review: ${esc(run.run_id)}</h1>
<p class="meta">${esc(run.mode)} mode · model ${esc(run.model)} · effort ${esc(run.effort)} · prompt ${esc(run.prompt_version)}${run.vocabulary_model ? ` · vocabulary by ${esc(run.vocabulary_model)}` : ''}<br>${esc(run.selection_rule)}</p>
<div class="sum">${stat(run.item_count, 'items')}${stat(run.ai_count, 'AI-related')}${stat(aiPct + '%', 'AI share')}${stat(rows.length, 'proposed topics')}${stat('$' + run.cost_usd_total.toFixed(3), 'cost')}</div>
${unmappedCount ? `<p class="meta">${unmappedCount} free-form labels were not placed in any topic.</p>` : ''}
${topics}
</body></html>`;
}

// recordsByKey: key -> tags record (gives the path to read the example from).
export function buildReviewHtml({ archiveRoot, run, rows, recordsByKey, unmappedCount }) {
  const examples = {};
  for (const t of rows) for (const k of t.examples) if (recordsByKey[k]) examples[k] = exampleFor(archiveRoot, recordsByKey[k]);
  return renderReview({ run, rows, examples, unmappedCount });
}

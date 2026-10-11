// Overview page across months: per-month counts/AI share/cost/QA, theme x month table, top entities,
// links to each month's review page and briefing. Self-contained, light/dark, phone width (table scrolls).
import { esc } from './markdown.js';

const CSS = `:root{color-scheme:light dark;--bg:#fff;--fg:#1c1c1e;--mut:#6b6b70;--line:#ddd;--card:#f6f6f7;--a:#0b57d0;--heat:11,87,208}
@media (prefers-color-scheme:dark){:root{--bg:#161618;--fg:#e8e8ea;--mut:#9a9aa0;--line:#333;--card:#212124;--a:#8ab4f8;--heat:138,180,248}}
body{font:16px/1.5 system-ui,sans-serif;background:var(--bg);color:var(--fg);margin:0 auto;padding:1rem;max-width:80rem}
h1{font-size:1.4rem}h2{font-size:1.2rem;margin:2rem 0 .5rem;border-bottom:1px solid var(--line);padding-bottom:.3rem}a{color:var(--a)}
nav{display:flex;flex-wrap:wrap;gap:.2rem .8rem}.meta{color:var(--mut);font-size:.85rem}
.sum{display:flex;flex-wrap:wrap;gap:.5rem 1.5rem;background:var(--card);padding:.75rem 1rem;border-radius:8px}.sum b{display:block;font-size:1.3rem}.sum span{color:var(--mut);font-size:.85rem}
.tbl{overflow-x:auto}table{border-collapse:collapse;font-size:.82rem}th,td{border-bottom:1px solid var(--line);padding:.3rem .4rem;text-align:right;white-space:nowrap}
th{color:var(--mut);font-weight:600}td.l,th.l{text-align:left;position:sticky;left:0;background:var(--bg);white-space:normal;min-width:12rem;max-width:18rem}
.ents{columns:3 14rem;padding-left:1.4rem}.n{color:var(--mut)}`;

const stat = (v, l) => `<div><b>${esc(v)}</b><span>${esc(l)}</span></div>`;
const pct = (x) => (x === null || x === undefined ? '–' : `${Math.round(x * 100)}%`);
const usd = (x) => `$${Number(x || 0).toFixed(2)}`;

/**
 * d: {generated, months:[{month, items, ai_share, cost:{briefing,tagging,qa}, qa:{themes_any_overlap,...}, review, briefing, failed}],
 *     themes:[name], counts:{theme:{month:n}}, totals:{theme:n}, entities:[{name,count}], extraCosts:[{stage,usd}], total}
 */
export function renderOverview(d) {
  const ms = d.months.map((m) => m.month);
  const monthRows = d.months.map((m) => `<tr><td class="l"><a href="${esc(m.review)}">${esc(m.month)}</a> · <a href="${esc(m.briefing)}">briefing</a></td><td>${m.items}</td><td>${pct(m.ai_share)}</td><td>${usd(m.cost.briefing)}</td><td>${usd(m.cost.tagging)}</td><td>${usd(m.cost.qa)}</td><td>${pct(m.qa && m.qa.themes_any_overlap)}</td><td>${pct(m.qa && m.qa.event_match)}</td><td>${m.failed || 0}</td></tr>`).join('\n');
  const max = Math.max(1, ...d.themes.flatMap((t) => ms.map((m) => (d.counts[t] || {})[m] || 0)));
  const share = (t, m) => { const mm = d.months.find((x) => x.month === m); const n = (d.counts[t] || {})[m] || 0; return mm && mm.items ? n / mm.items : 0; };
  const maxShare = Math.max(0.0001, ...d.themes.flatMap((t) => ms.map((m) => share(t, m))));
  const themeRows = d.themes.map((t) => `<tr><td class="l">${esc(t)} <span class="n">(${d.totals[t] || 0})</span></td>${ms.map((m) => {
    const n = (d.counts[t] || {})[m] || 0;
    const a = (share(t, m) / maxShare * 0.75).toFixed(2);
    return `<td style="background:rgba(var(--heat),${n ? a : 0})" title="${esc(t)} ${m}: ${n} items (${Math.round(share(t, m) * 100)}% of the month)">${n || ''}</td>`;
  }).join('')}</tr>`).join('\n');
  const extra = (d.extraCosts || []).map((c) => `<tr><td class="l">${esc(c.stage)}</td><td>${usd(c.usd)}</td></tr>`).join('');
  const items = d.months.reduce((s, m) => s + m.items, 0);
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Topic layer overview ${esc(ms[0])} to ${esc(ms[ms.length - 1])}</title><style>${CSS}</style></head><body>
<h1>Topic layer overview: ${esc(ms[0])} to ${esc(ms[ms.length - 1])}</h1>
<p class="meta">Generated ${esc(d.generated)} · themes from <a href="${esc(d.themesHref)}">themes.json</a> · entity map <a href="${esc(d.aliasesHref)}">entity-aliases.json</a> · combined layer <a href="current.jsonl">current.jsonl</a> (max ${d.colorMax || max} items in one cell)</p>
<nav><a href="#months">Months</a><a href="#themes">Themes by month</a><a href="#entities">Top entities</a><a href="#cost">Cost</a></nav>
<div class="sum">${stat(items, 'items')}${stat(d.months.length, 'months')}${stat(d.themes.length, 'themes')}${stat(pct(d.qaOverall && d.qaOverall.themes_any_overlap), 'QA theme overlap')}${stat(usd(d.total), 'cost of this run')}</div>
<h2 id="months">Months</h2><div class="tbl"><table><tr><th class="l">Month</th><th>Items</th><th>AI</th><th>Briefing</th><th>Tagging</th><th>QA</th><th>QA themes overlap</th><th>QA event same</th><th>Failed</th></tr>${monthRows}</table></div>
<h2 id="themes">Themes by month (item counts; shading = share of that month's items)</h2>
<div class="tbl"><table><tr><th class="l">Theme</th>${ms.map((m) => `<th>${esc(m.slice(2))}</th>`).join('')}</tr>${themeRows}</table></div>
<h2 id="entities">Top ${d.entities.length} entities (normalised)</h2><ol class="ents">${d.entities.map((e) => `<li>${esc(e.name)} <span class="n">(${e.count})</span></li>`).join('')}</ol>
<h2 id="cost">Cost</h2><div class="tbl"><table><tr><th class="l">Stage</th><th>USD</th></tr>
<tr><td class="l">Briefings (new this run)</td><td>${usd(d.months.reduce((s, m) => s + (m.cost.briefing_new || 0), 0))}</td></tr>
<tr><td class="l">Tagging</td><td>${usd(d.months.reduce((s, m) => s + (m.cost.tagging || 0), 0))}</td></tr>
<tr><td class="l">QA</td><td>${usd(d.months.reduce((s, m) => s + (m.cost.qa || 0), 0))}</td></tr>${extra}
<tr><th class="l">Total</th><th>${usd(d.total)}</th></tr></table></div>
</body></html>`;
}

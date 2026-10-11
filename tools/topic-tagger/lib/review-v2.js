// v2 review page: one self-contained file. Context (profile, briefing), themes, entities, events,
// QA agreement, cost, and a v1 vs v2 comparison. Light/dark, phone width, no external requests.
import { esc, mdToHtml } from './markdown.js';

const CSS = `:root{color-scheme:light dark;--bg:#fff;--fg:#1c1c1e;--mut:#6b6b70;--line:#ddd;--card:#f6f6f7;--a:#0b57d0}
@media (prefers-color-scheme:dark){:root{--bg:#161618;--fg:#e8e8ea;--mut:#9a9aa0;--line:#333;--card:#212124;--a:#8ab4f8}}
body{font:16px/1.5 system-ui,sans-serif;background:var(--bg);color:var(--fg);margin:0 auto;padding:1rem;max-width:52rem}
h1{font-size:1.4rem}h2{font-size:1.2rem;margin:2rem 0 .5rem;border-bottom:1px solid var(--line);padding-bottom:.3rem}h3{font-size:1.05rem;margin:0}
a{color:var(--a)}nav{display:flex;flex-wrap:wrap;gap:.2rem .8rem;margin:.5rem 0}nav a{white-space:nowrap}
.sum{display:flex;flex-wrap:wrap;gap:.5rem 1.5rem;background:var(--card);padding:.75rem 1rem;border-radius:8px}
.sum b{display:block;font-size:1.3rem}.sum span{color:var(--mut);font-size:.85rem}
.topic{border-top:1px solid var(--line);padding:.9rem 0}.def{margin:.2rem 0 .5rem;color:var(--mut)}
.n{font-weight:400;color:var(--mut)}.ex{background:var(--card);border-radius:6px;padding:.5rem .75rem;margin:.4rem 0;font-size:.92rem}
.meta{color:var(--mut);font-size:.8rem;overflow-wrap:anywhere}
details{background:var(--card);border-radius:8px;padding:.5rem 1rem;margin:.5rem 0}summary{cursor:pointer;font-weight:600}
.tbl{overflow-x:auto}table{border-collapse:collapse;width:100%;font-size:.88rem}th,td{border-bottom:1px solid var(--line);padding:.35rem .4rem;text-align:left;vertical-align:top}
th{color:var(--mut);font-weight:600}.ents{columns:2 14rem;padding-left:1.2rem}.tag{display:inline-block;background:var(--card);border-radius:4px;padding:0 .35rem;margin:.1rem .15rem .1rem 0}`;

const stat = (v, l) => `<div><b>${esc(v)}</b><span>${esc(l)}</span></div>`;
// Snippets come from raw highlights: drop markdown images and unresolvable t.co links.
export const cleanSnippet = (s) => String(s ?? '').replace(/!\[[^\]]*\]\([^)]*\)?/g, '').replace(/https?:\/\/t\.co\/\S+/g, '').replace(/\s+/g, ' ').trim();
const pct = (x) => (x === null || x === undefined ? '–' : `${Math.round(x * 100)}%`);

function exampleHtml(e) {
  if (!e) return '';
  const link = e.url ? `<a href="${esc(e.url)}">${esc(e.url)}</a>` : esc(e.path || '');
  const tags = [...(e.themes || []).map((t) => `<span class="tag">${esc(t)}</span>`), ...(e.entities || []).map((t) => `<span class="tag">${esc(t)}</span>`)].join('');
  return `<div class="ex"><div class="meta">${esc(e.sid || '')} · ${esc(e.author)} · ${esc(e.date)} · ${link}</div><div>${esc(cleanSnippet(e.snippet))}</div>${e.relevance ? `<div class="meta">why: ${esc(e.relevance)}</div>` : ''}${tags ? `<div class="meta">${tags}</div>` : ''}</div>`;
}

function qaTable(qa) {
  if (!qa) return '<p class="meta">QA has not been run.</p>';
  const rows = [['Themes: any overlap', 'themes_any_overlap'], ['Themes: exact set', 'themes_exact'], ['Themes: mean Jaccard', 'themes_mean_jaccard'],
    ['Entities: any overlap', 'entities_any_overlap'], ['Entities: exact set', 'entities_exact'], ['Entities: mean Jaccard', 'entities_mean_jaccard'],
    ['Event: same (incl. both none)', 'event_match'], ['Event: same when both set', 'event_match_when_both_set'], ['AI-related: same', 'ai_related_match']];
  const groups = [['All', qa.all], ['Random sample', qa.random], ['Low confidence', qa.low_confidence]].filter(([, g]) => g && g.n);
  return `<div class="tbl"><table><tr><th>Measure</th>${groups.map(([n, g]) => `<th>${esc(n)} (n=${g.n})</th>`).join('')}</tr>
${rows.map(([label, k]) => `<tr><td>${esc(label)}</td>${groups.map(([, g]) => `<td>${pct(g[k])}</td>`).join('')}</tr>`).join('\n')}</table></div>`;
}

/**
 * d: {run, profileMd, briefingMd, profileHref, briefingHref, themes:[{name,definition,count,examples:[key]}],
 *     entities:[{name,count}], events:[{id,name,date,description,count,examples:[key]}], qa, costs:[{stage,model,usd}],
 *     compare:[{key, v1:[...], themes, entities, event}], examples: key -> example}
 */
export function renderReviewV2(d) {
  const { run } = d;
  const ex = (k) => exampleHtml(d.examples[k]);
  const themes = d.themes.map((t) => `<section class="topic"><h3>${esc(t.name)} <span class="n">(${t.count})</span></h3>
<p class="def">${esc(t.definition)}</p>${t.examples.map(ex).join('\n')}</section>`).join('\n');
  const events = d.events.map((e) => `<section class="topic"><h3>${esc(e.id)} · ${esc(e.name)} <span class="n">(${e.count} items · ${esc(e.date)})</span></h3>
<p class="def">${esc(e.description)}</p>${e.examples.map(ex).join('\n')}</section>`).join('\n');
  const ents = `<ol class="ents">${d.entities.map((e) => `<li>${esc(e.name)} <span class="n">(${e.count})</span></li>`).join('')}</ol>`;
  const costRows = d.costs.map((c) => `<tr><td>${esc(c.stage)}</td><td>${esc(c.model || '')}</td><td>$${Number(c.usd || 0).toFixed(3)}</td></tr>`).join('');
  const total = d.costs.reduce((s, c) => s + Number(c.usd || 0), 0);
  const cmp = d.compare.map((c) => {
    const e = d.examples[c.key] || {};
    return `<tr><td><div class="meta">${esc(e.sid || '')} · ${esc(e.author)} · ${esc(e.date)}</div>${esc(cleanSnippet(e.snippet).slice(0, 140))}</td><td>${esc((c.v1 || []).join('; ') || '–')}</td><td>${esc(c.themes.join('; ') || '–')}</td><td>${esc(c.entities.join('; ') || '–')}</td><td>${esc(c.event || '–')}</td></tr>`;
  }).join('\n');
  const cs = run.cache_stats || {};
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>Topic review v2: ${esc(run.run_id)}</title><style>${CSS}</style></head><body>
<h1>Topic review v2: ${esc(run.run_id)}</h1>
<p class="meta">themes + entities · tagging ${esc(run.model)} (effort ${esc(run.effort)}) · prompt ${esc(run.prompt_version)} · QA ${esc(run.qa_model || '–')}<br>${esc(run.selection_rule)}</p>
<nav><a href="#context">Context</a><a href="#themes">Themes</a><a href="#entities">Entities</a><a href="#events">Events</a><a href="#qa">QA</a><a href="#cost">Cost</a><a href="#compare">v1 vs v2</a></nav>
<div class="sum">${stat(run.item_count, 'items')}${stat(run.tagged_items, 'tagged')}${stat(pct(run.ai_share), 'AI share')}${stat(d.themes.length, 'themes')}${stat(d.events.length, 'events')}${stat(pct(cs.token_hit_rate), 'cache hit (tokens)')}${stat('$' + total.toFixed(2), 'cost')}</div>
<h2 id="context">Context the tagger saw</h2>
<p class="meta">Prompt: ${esc(run.prompt_tokens_system)} system tokens + up to ${esc(run.prompt_max_user_tokens_counted ?? run.prompt_max_user_tokens_estimated)} per batch (cap ${esc(run.prompt_cap)}); largest prompt sent: ${esc(cs.max_prompt_tokens)} tokens.</p>
<details><summary>Reader profile</summary><p class="meta"><a href="${esc(d.profileHref)}">${esc(d.profileHref)}</a></p>${mdToHtml(d.profileMd)}</details>
<details><summary>Month briefing</summary><p class="meta"><a href="${esc(d.briefingHref)}">${esc(d.briefingHref)}</a></p>${mdToHtml(d.briefingMd)}</details>
<h2 id="themes">Themes with counts</h2>${themes}
<h2 id="entities">Top ${d.entities.length} entities</h2>${ents}
<h2 id="events">Events and their items</h2>${events}
<h2 id="qa">QA agreement (${esc(run.qa_model || '')} re-tag vs ${esc(run.model)})</h2>${qaTable(d.qa)}
<h2 id="cost">Cost</h2><div class="tbl"><table><tr><th>Stage</th><th>Model</th><th>USD</th></tr>${costRows}<tr><th>Total</th><th></th><th>$${total.toFixed(3)}</th></tr></table></div>
<h2 id="compare">v1 vs v2 (${d.compare.length} random items)</h2>
<div class="tbl"><table><tr><th>Item</th><th>v1 topics</th><th>v2 themes</th><th>v2 entities</th><th>v2 event</th></tr>${cmp}</table></div>
</body></html>`;
}

// Extend the topic layer to older months (2022-2024): briefings (quarter briefings for thin months) ->
// append-only theme extension -> per month (enrich, tag, structural check, QA, review) -> entity merge and
// current.jsonl/overview.html over the whole layer (older + earlier run). Resumable like the full run.
// 2025-26 months are never re-tagged. Spend cap covers everything this run pays for.
import { copyFileSync, existsSync, readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import * as v2 from './pipeline.js';
import { runPool } from './model.js';
import { callJson } from './tag-run.js';
import { briefingForPrompt } from './briefing.js';
import { THEMES_EXTEND_PROMPT_VERSION, THEMES_SCHEMA, themesExtendSystem, themesExtendUserMessage, extendThemes } from './themes.js';
import { countTagsInText } from './sources.js';
import { costFromUsage } from './cost.js';
import { monthsBetween, structuralProblem, mergeAndRender } from './full-run.js';

const readJson = (p) => JSON.parse(readFileSync(p, 'utf8'));
const writeJson = (p, o) => { mkdirSync(dirname(p), { recursive: true }); writeFileSync(p, JSON.stringify(o, null, 2) + '\n'); };
const stripFm = (s) => s.replace(/^---\n[\s\S]*?\n---\n?/, '').trim();

export const THIN_MONTH_ITEMS = 15;
export const quarterOf = (month) => `${month.slice(0, 4)}-Q${Math.ceil(Number(month.slice(5, 7)) / 3)}`;

/**
 * Which briefing each month uses. A quarter in which any month has fewer than `min` items gets one quarter
 * briefing shared by all its months in the range ("2022-Q2"); every other month keeps its own.
 * counts: {month: items}. Returns {keyOf: {month: key}, groups: [{key, months, items, merged}]} in date order.
 */
export function briefingGroups(months, counts, { min = THIN_MONTH_ITEMS } = {}) {
  const byQuarter = new Map();
  for (const m of months) { const q = quarterOf(m); if (!byQuarter.has(q)) byQuarter.set(q, []); byQuarter.get(q).push(m); }
  const keyOf = {};
  const groups = [];
  for (const [q, ms] of byQuarter) {
    const merged = ms.length > 1 && ms.some((m) => (counts[m] || 0) < min);
    if (merged) groups.push({ key: q, months: ms, items: ms.reduce((n, m) => n + (counts[m] || 0), 0), merged: true });
    else for (const m of ms) groups.push({ key: m, months: [m], items: counts[m] || 0, merged: false });
    for (const m of ms) keyOf[m] = merged ? q : m;
  }
  return { keyOf, groups };
}

export async function extendRun(base) {
  const A = base.archiveRoot;
  const months = monthsBetween(base.fromMonth, base.toMonth);
  const ctx = join(A, 'analysis/context');
  const topics = join(A, 'analysis/topics');
  const themesPath = join(ctx, 'themes.json');
  const themesV1Path = join(ctx, 'themes-v1.json');
  const aliasesPath = join(ctx, 'entity-aliases.json');
  const ledgerPath = join(topics, `${base.ledgerName}-ledger.json`);
  const log = base.log;
  const addedIn = `${base.fromMonth}..${base.toMonth}`;

  const counts = {};
  for (const m of months) counts[m] = v2.loadMonthItems({ archiveRoot: A, month: m }).items.length;
  const { keyOf, groups } = briefingGroups(months, counts);
  const briefingFile = (k) => join(ctx, 'briefings', `${k}-briefing.json`);

  // Earlier run (2025-26): its ledger and months, read only.
  const firstLedgerPath = join(topics, `${base.runPrefix}-ledger.json`);
  const firstLedger = existsSync(firstLedgerPath) ? readJson(firstLedgerPath) : null;
  const laterMonths = firstLedger ? firstLedger.months : [];

  let ledger = existsSync(ledgerPath) ? readJson(ledgerPath) : null;
  if (!ledger) {
    ledger = { kind: 'extension', run_prefix: base.runPrefix, from: base.fromMonth, to: base.toMonth, months, item_counts: counts, briefing_groups: groups, started: new Date().toISOString(), max_usd: base.maxUsd, preexisting_briefings: groups.filter((g) => existsSync(briefingFile(g.key))).map((g) => g.key) };
    writeJson(ledgerPath, ledger);
  }
  const runOf = (m) => { const p = join(topics, `${base.runPrefix}-${m}`, 'run.json'); return existsSync(p) ? readJson(p) : null; };
  const stageCost = (r, k) => (r && r.stages && r.stages[k] && r.stages[k].cost_usd) || 0;
  const share = (k) => groups.find((g) => g.key === k).months.length;
  function extCosts() {
    const per = {};
    for (const m of months) {
      const k = keyOf[m];
      const b = existsSync(briefingFile(k)) ? (readJson(briefingFile(k)).meta.cost_usd || 0) / share(k) : 0;
      const r = runOf(m);
      per[m] = { briefing: b, briefing_new: ledger.preexisting_briefings.includes(k) ? 0 : b, tagging: stageCost(r, 'tagging'), qa: stageCost(r, 'qa') };
    }
    const themes = existsSync(themesPath) ? ((readJson(themesPath).meta.extension || {}).cost_usd || 0) : 0;
    const total = Object.values(per).reduce((s, c) => s + c.briefing_new + c.tagging + c.qa, 0) + themes;
    return { per, themes, ents: 0, total };
  }
  // Whole layer shown in the overview: this run's months + the earlier run's months, both themes calls, the entity pass.
  function allCosts() {
    const e = extCosts();
    const per = { ...e.per };
    for (const m of laterMonths) {
      const bk = join(ctx, 'briefings', `${m}-briefing.json`);
      const b = existsSync(bk) ? readJson(bk).meta.cost_usd || 0 : 0;
      const r = runOf(m);
      per[m] = { briefing: b, briefing_new: firstLedger.preexisting_briefings.includes(m) ? 0 : b, tagging: stageCost(r, 'tagging'), qa: stageCost(r, 'qa') };
    }
    const themesV1 = existsSync(themesV1Path) ? readJson(themesV1Path).meta.cost_usd || 0 : (readJson(themesPath).meta.cost_usd || 0);
    const ents = existsSync(aliasesPath) ? readJson(aliasesPath).meta.cost_usd || 0 : 0;
    const total = Object.values(per).reduce((s, c) => s + c.briefing_new + c.tagging + c.qa, 0) + themesV1 + e.themes + ents;
    return { per, themes: themesV1, themes_extension: e.themes, ents, total, this_run_total: e.total + ents };
  }
  const spent = () => extCosts().total;
  const envFor = (month) => ({ ...base, month, briefingKey: keyOf[month], runId: `${base.runPrefix}-${month}`, themesName: 'themes.json', qaRandom: 20, qaLowCap: 10, spentFn: spent, force: false });
  const report = { failures: [] };

  // 1. briefings (one per month, or per quarter for thin months)
  const todo = groups.filter((g) => !existsSync(briefingFile(g.key)));
  log(`briefings: ${todo.length} to write (${groups.length - todo.length} exist); ${groups.filter((g) => g.merged).map((g) => g.key).join(', ') || 'no quarter merges'}`);
  await runPool(todo.map((g) => ({ id: g.key, g })), 3, async ({ g }) => {
    try { return await v2.stageBriefing({ ...envFor(g.months[0]), briefingKey: g.key, briefingMonths: g.merged ? g.months : null }); } catch (e) { return { error: e.message }; }
  }, (b, r) => { if (r.error) { report.failures.push(`briefing ${b.id}: ${r.error}`); log(`briefing ${b.id} FAILED: ${r.error}`); } });
  if (report.failures.length) throw new Error(`stopping after briefing failures: ${report.failures.join('; ')}`);

  // 2. theme extension (append-only; v1 copy kept)
  const current = readJson(themesPath);
  if (!current.meta.extension) {
    if (!existsSync(themesV1Path)) copyFileSync(themesPath, themesV1Path);
    const existing = current.themes;
    const rangeTags = new Map();
    for (const m of months) for (const it of v2.loadMonthItems({ archiveRoot: A, month: m }).items) countTagsInText(readFileSync(join(A, it.path), 'utf8'), rangeTags);
    const user = themesExtendUserMessage({
      profile: stripFm(readFileSync(join(ctx, 'reader-profile.md'), 'utf8')),
      briefings: groups.map((g) => ({ month: g.merged ? `${g.key} (${g.months.join(', ')})` : g.key, text: briefingForPrompt(readJson(briefingFile(g.key))) })),
      existing, readwiseTags: readJson(join(ctx, 'readwise-tags.json')),
      rangeTags: [...rangeTags.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).slice(0, 120).map(([tag, count]) => ({ tag, count })),
    });
    const system = themesExtendSystem(existing.length);
    const est = costFromUsage(base.themesModel, { input_tokens: Math.ceil((system.length + user.length) / 2.5), output_tokens: 30000 });
    if (spent() + est > base.maxUsd) throw new Error(`theme extension: estimate $${est.toFixed(2)} would pass the cap`);
    log(`theme extension: one call, ~${Math.ceil(user.length / 2.5)} tokens in, est $${est.toFixed(2)}`);
    const r = await callJson(base.client, { model: base.themesModel, effort: 'high', system, user, schema: THEMES_SCHEMA, maxTokens: 64000 });
    const t = extendThemes(existing, r.parsed, { addedIn });
    writeJson(themesPath, {
      meta: { ...current.meta, extension: { status: `v2: ${existing.length} themes unchanged from themes-v1.json + ${t.added.length} added for ${addedIn}; added themes are not applied to the earlier-tagged months`, added_in: addedIn, added_count: t.added.length, dropped_duplicates: t.dropped, model: base.themesModel, effort: 'high', prompt_version: THEMES_EXTEND_PROMPT_VERSION, generated: new Date().toISOString(), usage: r.usage, cost_usd: +r.cost.toFixed(4), system_prompt: system, v1_copy: 'themes-v1.json' } },
      notes: current.notes, extension_notes: t.notes, themes: t.themes,
    });
    log(`theme extension: +${t.added.length} (${t.added.map((x) => x.name).join('; ')}); $${r.cost.toFixed(3)}`);
  }

  // 3. months
  let stop = null;
  await runPool(months.map((m) => ({ id: m })), base.monthConcurrency || 2, async ({ id: m }) => {
    if (stop) return { skipped: true };
    const env = envFor(m);
    const p = v2.paths(env);
    try {
      if (!existsSync(p.enrichment)) v2.stageEnrich(env);
      if (!existsSync(p.tags)) await v2.stageTag(env);
      const run = readJson(p.runJson);
      const sp = structuralProblem(run);
      if (sp) { stop = `${m}: ${sp}`; return { error: stop }; }
      if (!run.qa) await v2.stageQa(env);
      v2.stageReview(env);
      log(`${m} done; extension total so far $${spent().toFixed(2)}`);
      return { ok: true };
    } catch (e) { stop = stop || `${m}: ${e.message}`; return { error: e.message }; }
  }, (b, r) => { if (r.error) report.failures.push(`${b.id}: ${r.error}`); });
  if (stop) throw new Error(`stopped: ${stop}`);

  // 4. entity merge over the whole layer (earlier map kept as entity-aliases-v1.json), current.jsonl, overview.html
  const v1Aliases = join(ctx, 'entity-aliases-v1.json');
  if (existsSync(aliasesPath) && !existsSync(v1Aliases)) copyFileSync(aliasesPath, v1Aliases);
  if (spent() + 1 > base.maxUsd) throw new Error('cap reached before the entity merge');
  const added = readJson(themesPath).themes.filter((t) => t.added_in);
  const themeInfo = Object.fromEntries(added.map((t) => [t.name, { label: `new (${t.added_in}), not applied to 2025-26`, notAppliedFrom: laterMonths[0] || '2025-01' }]));
  const keyAll = (m) => keyOf[m] || m;
  const alls = [...new Set([...months, ...laterMonths])].sort();
  // re-merge unless the stored map already covers every month (a resumed run pays for it once)
  const covered = existsSync(aliasesPath) && alls.every((m) => ((readJson(aliasesPath).meta || {}).months || []).includes(m));
  const r = await mergeAndRender({ ...base, forceEntities: !covered }, { months: alls, briefingKeyOf: keyAll, costs: allCosts, ledger, ledgerPath, report, themeInfo });
  ledger = readJson(ledgerPath);
  ledger.this_run_costs = extCosts();
  ledger.theme_additions = added.map((t) => ({ id: t.id, name: t.name }));
  writeJson(ledgerPath, ledger);
  return { ...r, this_run_usd: +(extCosts().total + (allCosts().ents)).toFixed(4), months_extended: months.length };
}

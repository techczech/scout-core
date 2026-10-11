// Full run over a range of months: briefings -> final themes -> per month (enrich, tag, QA, review) ->
// entity merge -> current.jsonl + overview.html. Resumable: every step skips output that already exists.
// One spend ledger for the whole run (new briefings + themes.json + entity map + every month's tagging and QA).
import { existsSync, readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import * as v2 from './pipeline.js';
import { runPool } from './model.js';
import { callJson } from './tag-run.js';
import { briefingForPrompt } from './briefing.js';
import { THEMES_FINAL_SYSTEM, THEMES_FINAL_PROMPT_VERSION, THEMES_SCHEMA, themesFinalUserMessage, validateThemes } from './themes.js';
import { aliasMap } from './tag-results.js';
import { ENTITY_PROMPT_VERSION, ENTITY_SCHEMA, ENTITY_SYSTEM, entityCounts, ruleMap, modelGroups, normaliseEntities, revalidate } from './entities.js';
import { agreement } from './qa.js';
import { renderOverview } from './overview.js';
import { costFromUsage } from './cost.js';

const readJson = (p) => JSON.parse(readFileSync(p, 'utf8'));
const writeJson = (p, o) => { mkdirSync(dirname(p), { recursive: true }); writeFileSync(p, JSON.stringify(o, null, 2) + '\n'); };
const readJsonl = (p) => (existsSync(p) ? readFileSync(p, 'utf8').split('\n').filter(Boolean).map((l) => JSON.parse(l)) : []);
const stripFm = (s) => s.replace(/^---\n[\s\S]*?\n---\n?/, '').trim();

export function monthsBetween(from, to) {
  const out = [];
  let [y, m] = from.split('-').map(Number);
  const [ty, tm] = to.split('-').map(Number);
  while (y < ty || (y === ty && m <= tm)) { out.push(`${y}-${String(m).padStart(2, '0')}`); m++; if (m > 12) { m = 1; y++; } }
  return out;
}

// Structural stop rule: more than 2% failed items or validation problems on more than 15% of items.
export function structuralProblem(run) {
  const n = run.item_count || 1;
  const failed = (run.failed_keys || []).length;
  const probs = (run.stages.tagging && run.stages.tagging.problem_count) || 0;
  if (failed / n > 0.02) return `${failed} of ${n} items failed`;
  if (probs / n > 0.15) return `${probs} validation problems for ${n} items`;
  return null;
}

export async function fullRun(base) {
  const A = base.archiveRoot;
  const months = monthsBetween(base.fromMonth, base.toMonth);
  const ctx = join(A, 'analysis/context');
  const topics = join(A, 'analysis/topics');
  const ledgerPath = join(topics, `${base.runPrefix}-ledger.json`);
  const themesPath = join(ctx, 'themes.json');
  const aliasesPath = join(ctx, 'entity-aliases.json');
  const log = base.log;
  const envFor = (month) => ({ ...base, month, runId: `${base.runPrefix}-${month}`, themesName: 'themes.json', qaRandom: 40, qaLowCap: 20, spentFn: spent, force: false });
  const briefingJson = (m) => join(ctx, 'briefings', `${m}-briefing.json`);

  let ledger = existsSync(ledgerPath) ? readJson(ledgerPath) : null;
  if (!ledger) {
    ledger = { run_prefix: base.runPrefix, months, started: new Date().toISOString(), max_usd: base.maxUsd, preexisting_briefings: months.filter((m) => existsSync(briefingJson(m))) };
    writeJson(ledgerPath, ledger);
  }
  function costs() {
    const per = {};
    for (const m of months) {
      const b = existsSync(briefingJson(m)) ? readJson(briefingJson(m)).meta.cost_usd || 0 : 0;
      const rp = join(topics, `${base.runPrefix}-${m}`, 'run.json');
      const r = existsSync(rp) ? readJson(rp) : null;
      per[m] = {
        briefing: b, briefing_new: ledger.preexisting_briefings.includes(m) ? 0 : b,
        tagging: (r && r.stages.tagging && r.stages.tagging.cost_usd) || 0, qa: (r && r.stages.qa && r.stages.qa.cost_usd) || 0,
      };
    }
    const themes = existsSync(themesPath) ? readJson(themesPath).meta.cost_usd || 0 : 0;
    const ents = existsSync(aliasesPath) ? readJson(aliasesPath).meta.cost_usd || 0 : 0;
    const total = Object.values(per).reduce((s, c) => s + c.briefing_new + c.tagging + c.qa, 0) + themes + ents;
    return { per, themes, ents, total };
  }
  function spent() { return costs().total; }
  const report = { failures: [] };

  // 1. briefings
  const todo = months.filter((m) => !existsSync(briefingJson(m)));
  log(`briefings: ${todo.length} to write (${months.length - todo.length} exist)`);
  await runPool(todo.map((m) => ({ id: m })), 3, async ({ id }) => {
    try { return await v2.stageBriefing(envFor(id)); } catch (e) { return { error: e.message }; }
  }, (b, r) => { if (r.error) { report.failures.push(`briefing ${b.id}: ${r.error}`); log(`briefing ${b.id} FAILED: ${r.error}`); } });
  if (report.failures.length) throw new Error(`stopping after briefing failures: ${report.failures.join('; ')}`);

  // 2. final themes
  if (!existsSync(themesPath)) {
    const sept = join(topics, base.septV2RunId);
    const septTags = readJsonl(join(sept, 'tags.jsonl'));
    const themeCounts = {};
    for (const r of septTags) for (const t of r.themes) themeCounts[t] = (themeCounts[t] || 0) + 1;
    const offList = {};
    for (const p of (readJson(join(sept, 'run.json')).stages.tagging.problems || [])) {
      const m = p.match(/^theme outside list for m\d+: (.+)$/);
      if (m) offList[m[1]] = (offList[m[1]] || 0) + 1;
    }
    const user = themesFinalUserMessage({
      profile: stripFm(readFileSync(join(ctx, 'reader-profile.md'), 'utf8')),
      briefings: months.map((m) => ({ month: m, text: briefingForPrompt(readJson(briefingJson(m))) })),
      draft: readJson(join(ctx, 'themes-draft.json')), septStats: { themeCounts, offList, items: septTags.length },
      readwiseTags: readJson(join(ctx, 'readwise-tags.json')),
    });
    const est = costFromUsage(base.themesModel, { input_tokens: Math.ceil(user.length / 2.5), output_tokens: 30000 });
    if (spent() + est > base.maxUsd) throw new Error(`themes: estimate $${est.toFixed(2)} would pass the cap`);
    log(`themes: one call, ~${Math.ceil(user.length / 2.5)} tokens in, est $${est.toFixed(2)}`);
    const r = await callJson(base.client, { model: base.themesModel, effort: 'high', system: THEMES_FINAL_SYSTEM, user, schema: THEMES_SCHEMA, maxTokens: 64000 });
    const t = validateThemes(r.parsed);
    writeJson(themesPath, {
      meta: { status: 'FINAL for 2025-01..2026-10 (non-AI themes to be extended for older items)', model: base.themesModel, effort: 'high', prompt_version: THEMES_FINAL_PROMPT_VERSION, generated: new Date().toISOString(),
        inputs: { profile: 'analysis/context/reader-profile.md (v2)', briefings: months.map((m) => `analysis/context/briefings/${m}.md`), draft: 'analysis/context/themes-draft.json', september_stats: `analysis/topics/${base.septV2RunId}`, readwise_tags: 'analysis/context/readwise-tags.json' },
        usage: r.usage, cost_usd: +r.cost.toFixed(4), system_prompt: THEMES_FINAL_SYSTEM },
      notes: t.notes, themes: t.themes,
    });
    log(`themes: ${t.themes.length}; $${r.cost.toFixed(3)}`);
  }

  // 3. months: enrich -> tag -> structural check -> QA -> review (two months at a time)
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
      log(`${m} done; run total so far $${spent().toFixed(2)}`);
      return { ok: true };
    } catch (e) { stop = stop || `${m}: ${e.message}`; return { error: e.message }; }
  }, (b, r) => { if (r.error) report.failures.push(`${b.id}: ${r.error}`); });
  if (stop) throw new Error(`stopped: ${stop}`);

  // 4. entity merge
  const all = [];
  for (const m of months) for (const r of readJsonl(join(topics, `${base.runPrefix}-${m}`, 'tags.jsonl'))) all.push({ ...r, month: m });
  if (!existsSync(aliasesPath) || base.forceEntities) {
    const counts = entityCounts(all);
    const rules = ruleMap(counts, months.map((m) => aliasMap(readJson(briefingJson(m)))));
    const names = rules.canonical.filter((n) => counts.get(n) >= 2).sort((a, b) => a.localeCompare(b));
    log(`entities: ${counts.size} raw, ${Object.keys(rules.map).length} mapped by rules; ${names.length} sent to the model`);
    const r = await callJson(base.client, { model: base.entityModel, effort: 'medium', system: ENTITY_SYSTEM, user: `Entity names (count, name), sorted alphabetically:\n${names.map((n) => `${counts.get(n)}\t${n}`).join('\n')}`, schema: ENTITY_SCHEMA, maxTokens: 64000 });
    const mg = modelGroups(r.parsed, names, rules.map, base.entityModel);
    const map = { ...rules.map, ...mg.map };
    const byRule = {};
    for (const v of Object.values(map)) byRule[v.rule] = (byRule[v.rule] || 0) + 1;
    writeJson(aliasesPath, {
      meta: { generated: new Date().toISOString(), months, raw_names: counts.size, mapped: Object.keys(map).length, by_rule: byRule, model_dropped_entries: mg.dropped,
        model: base.entityModel, prompt_version: ENTITY_PROMPT_VERSION, usage: r.usage, cost_usd: +r.cost.toFixed(4), system_prompt: ENTITY_SYSTEM,
        rules: { 'rule:spelling': 'same name up to case, spaces, hyphens, quotes -> most frequent spelling', 'rule:briefing-alias': "a monthly briefing's alias whose words all occur in the full name", 'rule:unique-longer-name': 'versioned name contained in exactly one more frequent longer name' } },
      map,
    });
    log(`entities: ${Object.keys(map).length} mapped (${JSON.stringify(byRule)}); $${r.cost.toFixed(3)}`);
  }
  // Vetoes are re-applied to the stored map without a new model call, so rule fixes stay deterministic.
  const stored = readJson(aliasesPath);
  const rv = revalidate(stored.map);
  if (rv.vetoed.length) {
    stored.map = rv.map;
    stored.meta.vetoed = [...(stored.meta.vetoed || []), ...rv.vetoed];
    stored.meta.mapped = Object.keys(rv.map).length;
    stored.meta.by_rule = Object.values(rv.map).reduce((o, v) => ({ ...o, [v.rule]: (o[v.rule] || 0) + 1 }), {});
    writeJson(aliasesPath, stored);
    log(`entities: ${rv.vetoed.length} stored model merges vetoed`);
  }
  const aliases = stored.map;

  // 5. current.jsonl + overview.html
  const evNames = {};
  for (const m of months) for (const e of readJson(briefingJson(m)).events) evNames[`${m}/${e.id}`] = e.name;
  const current = all.map((r) => {
    const runId = `${base.runPrefix}-${r.month}`;
    const ev = r.event ? `${r.month}/${r.event}` : null;
    return { key: r.key, path: r.path, date: r.date, type: r.type, month: r.month, ai_related: r.ai_related, themes: r.themes,
      entities: normaliseEntities(aliases, r.entities), event: ev, event_name: ev ? evNames[ev] || null : null,
      relevance: r.relevance, confidence: r.confidence, run_id: runId, prompt_version: 'topics-v2' };
  }).sort((a, b) => a.date.localeCompare(b.date) || a.key.localeCompare(b.key));
  writeFileSync(join(topics, 'current.jsonl'), current.map((x) => JSON.stringify(x)).join('\n') + '\n');

  const themes = readJson(themesPath).themes.map((t) => t.name);
  const counts = {}, totals = {};
  for (const r of current) for (const t of r.themes) { (counts[t] ||= {})[r.month] = (counts[t][r.month] || 0) + 1; totals[t] = (totals[t] || 0) + 1; }
  const ents = new Map();
  for (const r of current) for (const e of r.entities) ents.set(e, (ents.get(e) || 0) + 1);
  const c = costs();
  const allPairs = [];
  const monthRows = months.map((m) => {
    const run = readJson(join(topics, `${base.runPrefix}-${m}`, 'run.json'));
    for (const q of readJsonl(join(topics, `${base.runPrefix}-${m}`, 'qa.jsonl'))) allPairs.push({ a: q.tagger, b: q.qa, group: q.group });
    return { month: m, items: run.tagged_items, ai_share: run.ai_share, cost: c.per[m], qa: run.qa && run.qa.all, failed: (run.failed_keys || []).length,
      review: `${base.runPrefix}-${m}/review.html`, briefing: `../context/briefings/${m}.md` };
  });
  const qaOverall = { all: agreement(allPairs), random: agreement(allPairs.filter((x) => x.group === 'random')), low_confidence: agreement(allPairs.filter((x) => x.group === 'low-confidence')) };
  const html = renderOverview({
    generated: new Date().toISOString(), months: monthRows, themes: [...themes].sort((a, b) => (totals[b] || 0) - (totals[a] || 0)), counts, totals,
    entities: [...ents.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).slice(0, 60).map(([name, count]) => ({ name, count })),
    extraCosts: [{ stage: 'Final themes (Opus)', usd: c.themes }, { stage: 'Entity merge (model pass)', usd: c.ents }],
    total: c.total, qaOverall: qaOverall.all, themesHref: '../context/themes.json', aliasesHref: '../context/entity-aliases.json',
  });
  writeFileSync(join(topics, 'overview.html'), html);
  ledger.finished = new Date().toISOString();
  ledger.costs = c;
  ledger.qa_overall = qaOverall;
  writeJson(ledgerPath, ledger);
  return { months: months.length, items: current.length, total: +c.total.toFixed(4), qaOverall, failures: report.failures };
}

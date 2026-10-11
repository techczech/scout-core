// Refresh named months: after items in a month gained dates, redo that month's briefing, enrichment, tags and QA,
// then run the retag-style merge over ALL months of the run prefix (current.jsonl, overview.html, refreshed review pages).
// Nothing is deleted: the month's run folder and its three month-keyed briefing files are renamed with a
// `.superseded-<YYYYMMDD-HHMM>` marker, and every lister (listRunMonths below) ignores marked names.
// Ledger: <prefix>-refresh-<stamp>-ledger.json (separate from the retag ledger <prefix>-ledger.json, which is left
// untouched). It is created, with the move plan, before the first move; an unfinished ledger for the same months is the
// resume point, so an interrupted refresh never moves fresh partial output aside again. Cap = this refresh's spend only.
import { existsSync, readFileSync, readdirSync, renameSync, writeFileSync, mkdirSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { join, dirname } from 'node:path';
import * as v2 from './pipeline.js';
import { runPool } from './model.js';
import { ENTITY_PROMPT_VERSION } from './entities.js';
import { structuralProblem, mergeAndRender } from './full-run.js';
import { PROMPT_VERSION } from './tag-prompt.js';
import { quarterOf } from './extend-run.js';

const readJson = (p) => JSON.parse(readFileSync(p, 'utf8'));
const writeJson = (p, o) => { mkdirSync(dirname(p), { recursive: true }); writeFileSync(p, JSON.stringify(o, null, 2) + '\n'); };
const sha = (p) => createHash('sha256').update(readFileSync(p)).digest('hex').slice(0, 16);
const escRe = (s) => s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

export const SUPERSEDED = '.superseded-';
export const isSuperseded = (name) => name.includes(SUPERSEDED);

/** Months that have a live run folder `<prefix>-<YYYY-MM>` (superseded folders never match), with tags written. */
export function listRunMonths(topicsDir, prefix, { include = [] } = {}) {
  const re = new RegExp(`^${escRe(prefix)}-(\\d{4}-\\d{2})$`);
  const out = new Set(include);
  if (existsSync(topicsDir)) {
    for (const n of readdirSync(topicsDir)) {
      const m = !isSuperseded(n) && n.match(re);
      if (m && existsSync(join(topicsDir, n, 'tags.jsonl'))) out.add(m[1]);
    }
  }
  return [...out].sort();
}

/** Month-keyed briefing months: names `<YYYY-MM>-briefing.json` only (quarter keys and superseded files excluded). */
export function listBriefingMonths(briefingsDir) {
  if (!existsSync(briefingsDir)) return [];
  return readdirSync(briefingsDir).filter((n) => !isSuperseded(n)).map((n) => (n.match(/^(\d{4}-\d{2})-briefing\.json$/) || [])[1]).filter(Boolean).sort();
}

export const stampOf = (d) => d.toISOString().slice(0, 16).replace(/[-:]/g, '').replace('T', '-');
const withMarker = (file, stamp) => file.replace(/(\.[^.]+)$/, `${SUPERSEDED}${stamp}$1`);

export async function refreshRun(base) {
  const A = base.archiveRoot;
  const log = base.log;
  const months = [...new Set(String(base.months || '').split(',').map((s) => s.trim()).filter(Boolean))].sort();
  if (!months.length) throw new Error('refresh needs --months YYYY-MM[,YYYY-MM...]');
  for (const m of months) if (!/^\d{4}-(0[1-9]|1[0-2])$/.test(m)) throw new Error(`refresh: "${m}" is not a month (YYYY-MM)`);
  const ctx = join(A, 'analysis/context');
  const bdir = join(ctx, 'briefings');
  const topics = join(A, 'analysis/topics');
  const themesPath = join(ctx, 'themes.json');
  const aliasesPath = join(ctx, 'entity-aliases.json');
  const bfiles = (m) => ['.md', '-events.json', '-briefing.json'].map((s) => `${m}${s}`);
  const runName = (m) => `${base.runPrefix}-${m}`;

  // ---- ledger: resume an unfinished one for the same months, else validate everything, then create it ----
  const ledgerFiles = existsSync(topics) ? readdirSync(topics).filter((n) => new RegExp(`^${escRe(base.runPrefix)}-refresh-\\d{8}-\\d{4}-ledger\\.json$`).test(n)).sort() : [];
  let ledgerPath = null, ledger = null;
  for (const n of ledgerFiles.reverse()) {
    const l = readJson(join(topics, n));
    if (!l.finished && JSON.stringify(l.months) === JSON.stringify(months)) { ledgerPath = join(topics, n); ledger = l; break; }
  }
  const resumed = !!ledger;
  if (!resumed) {
    for (const m of months) {
      if (existsSync(join(bdir, `${m}-briefing.json`))) continue;
      const q = quarterOf(m);
      throw new Error(existsSync(join(bdir, `${q}-briefing.json`))
        ? `refresh: ${m} is covered by the quarter briefing ${q}; refresh supports month-keyed briefings only`
        : `refresh: no month-keyed briefing for ${m}; refresh supports month-keyed briefings only`);
    }
    const stamp = stampOf(base.now || new Date());
    const plan = {};
    for (const m of months) {
      const run = existsSync(join(topics, runName(m))) ? { from: runName(m), to: `${runName(m)}${SUPERSEDED}${stamp}` } : null;
      const files = bfiles(m).filter((f) => existsSync(join(bdir, f))).map((f) => ({ from: f, to: withMarker(f, stamp) }));
      for (const t of [run && join(topics, run.to), ...files.map((f) => join(bdir, f.to))]) if (t && existsSync(t)) throw new Error(`refresh: ${t} already exists; wait a minute and retry`);
      plan[m] = { run, briefing_files: files };
    }
    ledgerPath = join(topics, `${base.runPrefix}-refresh-${stamp}-ledger.json`);
    ledger = { kind: 'refresh', prompt_version: PROMPT_VERSION, run_prefix: base.runPrefix, months, stamp, started: new Date().toISOString(), max_usd: base.maxUsd,
      themes_sha256_16: sha(themesPath), superseded: plan };
    writeJson(ledgerPath, ledger);
  }
  // ---- moves (idempotent: a target that exists means that item was already moved) ----
  for (const m of months) {
    const { run, briefing_files: files } = ledger.superseded[m];
    if (run && !existsSync(join(topics, run.to)) && existsSync(join(topics, run.from))) { renameSync(join(topics, run.from), join(topics, run.to)); log(`${m}: ${run.from} -> ${run.to}`); }
    for (const f of files) if (!existsSync(join(bdir, f.to)) && existsSync(join(bdir, f.from))) renameSync(join(bdir, f.from), join(bdir, f.to));
  }

  // ---- costs: the cap covers this refresh's own spend (new briefings, tagging, QA of the named months) ----
  const runPath = (m) => join(topics, runName(m), 'run.json');
  const stageCost = (r, k) => (r && r.stages && r.stages[k] && r.stages[k].cost_usd) || 0;
  const briefingCost = (m) => (existsSync(join(bdir, `${m}-briefing.json`)) ? readJson(join(bdir, `${m}-briefing.json`)).meta.cost_usd || 0 : 0);
  const entCost = () => {
    if (!existsSync(aliasesPath)) return 0;
    const meta = readJson(aliasesPath).meta || {};
    return meta.prompt_version === ENTITY_PROMPT_VERSION ? meta.cost_usd || 0 : 0;
  };
  const allMonths = () => listRunMonths(topics, base.runPrefix, { include: months });
  const costs = () => {
    const per = {};
    for (const m of allMonths()) {
      const r = existsSync(runPath(m)) ? readJson(runPath(m)) : null;
      const fresh = months.includes(m) ? briefingCost(m) : 0;
      per[m] = { briefing: fresh, briefing_new: fresh, tagging: stageCost(r, 'tagging'), qa: stageCost(r, 'qa') };
    }
    const ents = entCost();
    return { per, themes: 0, ents, total: Object.values(per).reduce((s, c) => s + c.briefing_new + c.tagging + c.qa, 0) + ents };
  };
  const thisRun = () => months.reduce((s, m) => { const c = costs().per[m]; return s + c.briefing_new + c.tagging + c.qa; }, 0);
  const spent = thisRun;
  const envFor = (month) => ({ ...base, month, briefingKey: month, runId: runName(month), themesName: 'themes.json', qaRandom: base.qaRandom ?? 40, qaLowCap: base.qaLowCap ?? 20, spentFn: spent, force: false });
  const report = { failures: [] };

  // ---- per month: briefing -> enrich -> tag -> structural check -> QA (review pages come after the merge) ----
  let stop = null;
  await runPool(months.map((m) => ({ id: m })), base.monthConcurrency || 2, async ({ id: m }) => {
    if (stop) return { skipped: true };
    const env = envFor(m);
    const p = v2.paths(env);
    try {
      if (!(existsSync(p.briefingJson) && existsSync(p.eventsJson) && existsSync(p.briefingMd))) await v2.stageBriefing(env);
      if (!existsSync(p.enrichment)) v2.stageEnrich(env);
      if (!existsSync(p.tags)) await v2.stageTag(env);
      const run = readJson(p.runJson);
      const sp = structuralProblem(run);
      if (sp) { stop = `${m}: ${sp}`; return { error: stop }; }
      if (!run.qa) await v2.stageQa(env);
      log(`${m} done; this refresh so far $${spent().toFixed(2)}`);
      return { ok: true };
    } catch (e) { stop = stop || `${m}: ${e.message}`; return { error: e.message }; }
  }, (b, r) => { if (r.error) report.failures.push(`${b.id}: ${r.error}`); });
  if (stop) throw new Error(`stopped: ${stop}`);

  // ---- merge over every live month of the prefix; entity map reused when it already covers them (as retag) ----
  const all = allMonths();
  const briefingKeyOf = (m) => (existsSync(join(bdir, `${m}-briefing.json`)) ? m : quarterOf(m));
  const stored = existsSync(aliasesPath) ? readJson(aliasesPath).meta || {} : null;
  const fresh = !!stored && stored.prompt_version === ENTITY_PROMPT_VERSION && all.every((m) => (stored.months || []).includes(m));
  if (!fresh && spent() + 0.5 > base.maxUsd) throw new Error('cap reached before the entity merge');
  const r = await mergeAndRender({ ...base, forceEntities: !fresh, afterMerge: async ({ aliases, workIndex }) => {
    for (const m of months) v2.stageReview({ ...envFor(m), entityMap: aliases, workIndex });
  } }, { months: all, briefingKeyOf, costs, ledger, ledgerPath, report });
  ledger = readJson(ledgerPath);
  if (sha(themesPath) !== ledger.themes_sha256_16) throw new Error('themes.json changed during the refresh');
  ledger.themes_unchanged = true;
  ledger.this_run_usd = +thisRun().toFixed(4);
  ledger.merged_months = all.length;
  writeJson(ledgerPath, ledger);
  return { ...r, refreshed: months, stamp: ledger.stamp, this_run_usd: ledger.this_run_usd, ledger: ledgerPath, themes_unchanged: true };
}

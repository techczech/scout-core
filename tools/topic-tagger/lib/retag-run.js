// Re-tag every month of a range with the current prompt (topics-v3) into new run folders, reusing the existing
// briefings and themes.json (neither is changed; a hash check proves it). Then: entity merge, mentioned_works join,
// current.jsonl, overview.html, and each month's review page (written after the merge so it shows normalised links).
// Earlier run folders stay as history. Resumable like the other runs; one ledger with a spend cap.
import { copyFileSync, existsSync, readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { join } from 'node:path';
import * as v2 from './pipeline.js';
import { runPool } from './model.js';
import { ENTITY_PROMPT_VERSION } from './entities.js';
import { monthsBetween, structuralProblem, mergeAndRender } from './full-run.js';
import { PROMPT_VERSION } from './tag-prompt.js';
import { quarterOf } from './extend-run.js';
import { writeFileSync, mkdirSync } from 'node:fs';
import { dirname } from 'node:path';

const readJson = (p) => JSON.parse(readFileSync(p, 'utf8'));
const writeJson = (p, o) => { mkdirSync(dirname(p), { recursive: true }); writeFileSync(p, JSON.stringify(o, null, 2) + '\n'); };
const sha = (p) => createHash('sha256').update(readFileSync(p)).digest('hex').slice(0, 16);

export async function retagRun(base) {
  const A = base.archiveRoot;
  const months = monthsBetween(base.fromMonth, base.toMonth);
  const ctx = join(A, 'analysis/context');
  const topics = join(A, 'analysis/topics');
  const themesPath = join(ctx, 'themes.json');
  const aliasesPath = join(ctx, 'entity-aliases.json');
  const ledgerPath = join(topics, `${base.runPrefix}-ledger.json`);
  const log = base.log;
  // a month uses its own briefing, else the quarter briefing written for thin months
  const briefingKeyOf = (m) => (existsSync(join(ctx, 'briefings', `${m}-briefing.json`)) ? m : quarterOf(m));
  for (const m of months) if (!existsSync(join(ctx, 'briefings', `${briefingKeyOf(m)}-briefing.json`))) throw new Error(`no briefing for ${m}`);

  let ledger = existsSync(ledgerPath) ? readJson(ledgerPath) : null;
  if (!ledger) {
    ledger = { kind: 'retag', prompt_version: PROMPT_VERSION, run_prefix: base.runPrefix, months, started: new Date().toISOString(), max_usd: base.maxUsd, themes_sha256_16: sha(themesPath), briefings: Object.fromEntries(months.map((m) => [m, briefingKeyOf(m)])) };
    writeJson(ledgerPath, ledger);
  }
  const runPath = (m) => join(topics, `${base.runPrefix}-${m}`, 'run.json');
  const stageCost = (r, k) => (r && r.stages && r.stages[k] && r.stages[k].cost_usd) || 0;
  const entCost = () => {
    if (!existsSync(aliasesPath)) return 0;
    const meta = readJson(aliasesPath).meta || {};
    return meta.prompt_version === ENTITY_PROMPT_VERSION ? meta.cost_usd || 0 : 0;
  };
  function costs() {
    const per = {};
    for (const m of months) {
      const r = existsSync(runPath(m)) ? readJson(runPath(m)) : null;
      per[m] = { briefing: 0, briefing_new: 0, tagging: stageCost(r, 'tagging'), qa: stageCost(r, 'qa') };
    }
    const ents = entCost();
    const total = Object.values(per).reduce((s, c) => s + c.tagging + c.qa, 0) + ents;
    return { per, themes: 0, ents, total };
  }
  const spent = () => costs().total;
  const envFor = (month) => ({ ...base, month, briefingKey: briefingKeyOf(month), runId: `${base.runPrefix}-${month}`, themesName: 'themes.json', qaRandom: base.qaRandom ?? 15, qaLowCap: base.qaLowCap ?? 10, spentFn: spent, force: false });
  const report = { failures: [] };

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
      log(`${m} done; run total so far $${spent().toFixed(2)}`);
      return { ok: true };
    } catch (e) { stop = stop || `${m}: ${e.message}`; return { error: e.message }; }
  }, (b, r) => { if (r.error) report.failures.push(`${b.id}: ${r.error}`); });
  if (stop) throw new Error(`stopped: ${stop}`);

  // entity merge over the re-tagged months (previous map kept once as entity-aliases-pre-v3.json)
  const pre = join(ctx, 'entity-aliases-pre-v3.json');
  if (existsSync(aliasesPath) && !existsSync(pre) && readJson(aliasesPath).meta.prompt_version !== ENTITY_PROMPT_VERSION) copyFileSync(aliasesPath, pre);
  const fresh = existsSync(aliasesPath) && readJson(aliasesPath).meta.prompt_version === ENTITY_PROMPT_VERSION
    && months.every((m) => (readJson(aliasesPath).meta.months || []).includes(m));
  if (spent() + 0.5 > base.maxUsd) throw new Error('cap reached before the entity merge');
  const r = await mergeAndRender({ ...base, forceEntities: !fresh, afterMerge: async ({ aliases, workIndex }) => {
    for (const m of months) v2.stageReview({ ...envFor(m), entityMap: aliases, workIndex });
  } }, { months, briefingKeyOf, costs, ledger, ledgerPath, report });
  ledger = readJson(ledgerPath);
  if (sha(themesPath) !== ledger.themes_sha256_16) throw new Error('themes.json changed during the re-tag');
  ledger.themes_unchanged = true;
  writeJson(ledgerPath, ledger);
  return { ...r, themes_unchanged: true };
}

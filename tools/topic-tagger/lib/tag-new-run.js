// tag-new: tag only the items that have no row in analysis/topics/current.jsonl, month by month, against each month's
// EXISTING briefing, events and themes. Nothing but new files is written:
//   - one new run folder per month, <run-prefix>-new-<stamp>-<YYYY-MM>/ (tags.jsonl, run.json, enrichment.jsonl, prompt files),
//   - one ledger <run-prefix>-new-<stamp>-ledger.json,
//   - current.jsonl, extended in place by appendToCurrent (old lines kept byte for byte, new rows inserted in date/key order).
// It cannot create, rename, renumber or delete an event and never writes under analysis/context/ (a content hash of that
// tree before and after is checked; a change aborts the run). Event ids the model returns that are not in the month's list
// are dropped by the same validator the tag stage uses and counted. Sonnet QA is not run.
// Month index: items already tagged keep the short ids they had (numbered over the month's tagged items, as the earlier run
// did); new items get the next ids after them, so ids in the briefing and in earlier prompts stay valid.
import { existsSync, mkdirSync, readFileSync, readdirSync, renameSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { join } from 'node:path';
import * as v2 from './pipeline.js';
import { selectWorks } from './works.js';
import { assignShortIds } from './month-index.js';
import { buildArchiveIndex, loadXSnapshot, enrichItems } from './enrich.js';
import { loadOwnTweets } from './sources.js';
import { PROMPT_VERSION, PROMPT_CAP, systemText, userMessageV2 } from './tag-prompt.js';
import { aliasMap, AUTHOR_DROP_PREFIX } from './tag-results.js';
import { tagAll, cacheStats } from './tag-run.js';
import { readingsStatus } from './guard.js';
import { revalidate } from './entities.js';
import { buildWorkIndex } from './mentions.js';
import { buildCurrentRow, structuralProblem } from './full-run.js';
import { quarterOf } from './extend-run.js';
import { stampOf } from './refresh-run.js';

const readJson = (p) => JSON.parse(readFileSync(p, 'utf8'));
const sha16 = (s) => createHash('sha256').update(s).digest('hex').slice(0, 16);
const cmpRow = (a, b) => a.date.localeCompare(b.date) || a.key.localeCompare(b.key);

/** Hash of every file under dir (relative path + bytes), for before/after comparison. */
export function treeHash(dir) {
  const h = createHash('sha256');
  const walk = (d, rel) => {
    if (!existsSync(d)) return;
    for (const e of readdirSync(d, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
      if (e.isDirectory()) walk(join(d, e.name), `${rel}${e.name}/`);
      else h.update(`${rel}${e.name}\0`).update(readFileSync(join(d, e.name))).update('\0');
    }
  };
  walk(dir, '');
  return h.digest('hex');
}

/**
 * Add rows to current.jsonl. Every existing line stays exactly as it is and in its place; each new row (sorted by
 * date, key) goes before the first existing line that sorts after it. Written to a temp file, checked, then renamed.
 */
export function appendToCurrent(currentPath, newRows) {
  if (!newRows.length) return 0;
  const old = readFileSync(currentPath, 'utf8').split('\n').filter(Boolean);
  const oldKeys = new Set(old.map((l) => JSON.parse(l).key));
  for (const r of newRows) if (oldKeys.has(r.key)) throw new Error(`current.jsonl already has a row for ${r.key}`);
  const pending = [...newRows].sort(cmpRow);
  const out = [];
  let i = 0;
  for (const line of old) {
    const o = JSON.parse(line);
    while (i < pending.length && cmpRow(pending[i], o) < 0) out.push(JSON.stringify(pending[i++]));
    out.push(line);
  }
  while (i < pending.length) out.push(JSON.stringify(pending[i++]));
  const text = out.join('\n') + '\n';
  const lines = new Set(text.split('\n'));
  for (const l of old) if (!lines.has(l)) throw new Error('current.jsonl update would drop an existing line');
  const tmp = `${currentPath}.tmp-${process.pid}`;
  writeFileSync(tmp, text);
  renameSync(tmp, currentPath);
  return newRows.length;
}

/** Items of `month` split into already-tagged (sids as before) and new (next sids). Returns {indexItems, newItems}. */
export function splitMonthItems(monthItems, taggedKeys) {
  const old = assignShortIds(monthItems.filter((i) => taggedKeys.has(i.key)));
  const width = Math.max(4, String(old.length).length);
  const fresh = monthItems.filter((i) => !taggedKeys.has(i.key)).sort((a, b) => (a.date || '').localeCompare(b.date || '') || a.key.localeCompare(b.key))
    .map((it, i) => ({ ...it, sid: `m${String(old.length + i + 1).padStart(width, '0')}` }));
  return { indexItems: [...old, ...fresh], newItems: fresh };
}

export async function tagNewRun(base) {
  const A = base.archiveRoot;
  const log = base.log || (() => {});
  if (!base.runPrefix) throw new Error('tag-new needs --run-prefix');
  const filter = base.months ? [...new Set(String(base.months).split(',').map((s) => s.trim()).filter(Boolean))].sort() : null;
  for (const m of filter || []) if (!/^\d{4}-(0[1-9]|1[0-2])$/.test(m)) throw new Error(`tag-new: "${m}" is not a month (YYYY-MM)`);
  const ctxDir = join(A, 'analysis/context');
  const bdir = join(ctxDir, 'briefings');
  const topics = join(A, 'analysis/topics');
  const currentPath = join(topics, 'current.jsonl');
  if (!existsSync(currentPath)) throw new Error(`tag-new: ${currentPath} not found`);
  const tagged = new Set(readFileSync(currentPath, 'utf8').split('\n').filter(Boolean).map((l) => JSON.parse(l).key));

  // ---- 1. untagged items by month (same item definition, dating and month assignment as the tag stage) ----
  const scan = selectWorks(join(A, 'readings/works'), '0000-01-01', '9999-12-31').items.filter((i) => !tagged.has(i.key));
  const candidateMonths = [...new Set(scan.map((i) => i.date.slice(0, 7)))].sort().filter((m) => !filter || filter.includes(m));
  const perMonth = [];
  for (const m of candidateMonths) {
    const monthItems = v2.loadMonthItems({ archiveRoot: A, month: m }).items;
    const split = splitMonthItems(monthItems, tagged);
    if (!split.newItems.length) continue;
    const briefingKey = existsSync(join(bdir, `${m}-briefing.json`)) ? m : existsSync(join(bdir, `${quarterOf(m)}-briefing.json`)) ? quarterOf(m) : null;
    perMonth.push({ month: m, briefingKey, ...split });
  }
  const skipped = perMonth.filter((x) => !x.briefingKey).map((x) => ({ month: x.month, items: x.newItems.length }));
  const todo = perMonth.filter((x) => x.briefingKey);
  const untaggedTotal = perMonth.reduce((n, x) => n + x.newItems.length, 0);
  const stamp = stampOf(base.now || new Date());
  const report = {
    kind: 'tag-new', dry_run: !!base.dryRun, run_prefix: base.runPrefix, stamp, model: base.model, prompt_version: PROMPT_VERSION,
    qa: 'skipped (tag-new runs no Sonnet QA)', untagged_items: untaggedTotal,
    months: [], skipped_no_context: skipped, not_run_budget: [], rejected: [], events_dropped_total: 0,
  };
  if (!todo.length) { report.note = untaggedTotal ? 'every month with untagged items lacks context' : 'nothing untagged'; return report; }
  for (const f of [join(ctxDir, 'reader-profile.md'), join(ctxDir, 'themes.json')]) if (!existsSync(f)) throw new Error(`tag-new needs ${f}`);

  // ---- 2. shared read-only inputs ----
  const archive = buildArchiveIndex(A);
  const snapshot = loadXSnapshot(join(A, 'imports'));
  const ownTweets = loadOwnTweets(base.tweetsDir);
  const runName = (m) => `${base.runPrefix}-new-${stamp}-${m}`;
  const envFor = (x) => ({ ...base, month: x.month, briefingKey: x.briefingKey, runId: runName(x.month), themesName: 'themes.json', force: false });
  const plan = async (x, count) => {
    const env = envFor(x);
    const contexts = enrichItems(x.indexItems, { archive, snapshot, ownTweets, archiveRoot: A }).contexts;
    const fit = await v2.tagPrompt(env, x.indexItems, { count, needEnrichment: false });
    const sidOf = new Map(x.indexItems.map((i) => [i.key, i.sid]));
    const batches = v2.packForTagging(x.newItems, contexts, Math.min(base.userReserve, fit.maxUserTokens), sidOf);
    return { env, contexts, fit, batches, estimate: v2.estimateTagging(base.model, fit.systemTokens, batches) };
  };

  if (base.dryRun) {
    // Prompt size is estimated by characters (no token-counting call, so no API key). That over-counts big months so much that
    // the month index may not "fit"; the estimate then uses the system-prompt token count the month's earlier run recorded.
    const prevRun = new Map();
    for (const l of readFileSync(currentPath, 'utf8').split('\n')) { if (!l) continue; const r = JSON.parse(l); if (!prevRun.has(r.month)) prevRun.set(r.month, r.run_id); }
    let total = 0;
    for (const x of todo) {
      let p, basis = 'characters';
      try { p = await plan(x, undefined); } catch (e) {
        const rj = join(topics, prevRun.get(x.month) || '-', 'run.json');
        if (!existsSync(rj) || !readJson(rj).prompt_tokens_system) { report.months.push({ month: x.month, briefing_key: x.briefingKey, untagged: x.newItems.length, has_context: true, estimate_error: e.message }); continue; }
        const contexts = enrichItems(x.indexItems, { archive, snapshot, ownTweets, archiveRoot: A }).contexts;
        const batches = v2.packForTagging(x.newItems, contexts, base.userReserve, new Map(x.indexItems.map((i) => [i.key, i.sid])));
        p = { batches, estimate: v2.estimateTagging(base.model, readJson(rj).prompt_tokens_system, batches) };
        basis = `earlier run ${prevRun.get(x.month)}`;
      }
      total += p.estimate;
      report.months.push({ month: x.month, briefing_key: x.briefingKey, untagged: x.newItems.length, has_context: true, batches: p.batches.length, estimate_usd: +p.estimate.toFixed(4), system_prompt_basis: basis });
    }
    report.estimate_usd = +total.toFixed(4);
    report.estimate_basis = 'same estimator as the tag stage (estimateTagging); system prompt size per system_prompt_basis';
    return report;
  }

  // ---- 3. tag each month; a month is written only when complete; current.jsonl follows each month ----
  const ctxBefore = treeHash(ctxDir);
  const readingsBefore = readingsStatus(A);
  const stored = existsSync(join(ctxDir, 'entity-aliases.json')) ? readJson(join(ctxDir, 'entity-aliases.json')) : { map: {} };
  const aliases = revalidate(stored.map).map; // in memory only, never written back
  const workIndex = buildWorkIndex(A);
  const ledgerPath = join(topics, `${base.runPrefix}-new-${stamp}-ledger.json`);
  const ledger = { ...report, started: new Date().toISOString(), max_usd: base.maxUsd };
  let spent = 0;
  let stopped = false;
  try {
    for (const x of todo) {
      if (stopped) { report.not_run_budget.push({ month: x.month, items: x.newItems.length }); continue; }
      const p = await plan(x, v2.realCount(base.client, base.model));
      const longest = [...p.batches].sort((a, b) => userMessageV2(b).length - userMessageV2(a).length).slice(0, 3);
      let maxUserReal = 0;
      for (const b of longest) maxUserReal = Math.max(maxUserReal, await v2.realCount(base.client, base.model)([{ type: 'text', text: '.' }], userMessageV2(b)) - 1);
      if (p.fit.systemTokens + maxUserReal >= PROMPT_CAP) throw new Error(`${x.month}: largest prompt would be ${p.fit.systemTokens + maxUserReal} tokens (cap ${PROMPT_CAP})`);
      log(`${x.month}: ${x.newItems.length} new items; estimated $${p.estimate.toFixed(3)}; spent so far $${spent.toFixed(3)} of $${base.maxUsd}`);
      if (spent + p.estimate > base.maxUsd) { stopped = true; report.not_run_budget.push({ month: x.month, items: x.newItems.length, estimate_usd: +p.estimate.toFixed(4) }); continue; }
      const briefing = readJson(v2.paths(p.env).briefingJson);
      const vctx = { themes: p.fit.themes, events: briefing.events, aliases: aliasMap(briefing), sidKey: new Map(x.indexItems.map((i) => [i.sid, i.key])) };
      const started = Date.now();
      const r = await tagAll(base.client, { model: base.model, effort: base.effort, system: p.fit.system, vctx }, p.batches, { concurrency: base.concurrency, maxUsd: base.maxUsd - spent, log });
      spent += r.cost;
      const eventsDropped = r.problems.filter((s) => /^unknown event /.test(s)).length;
      const run = {
        run_id: runName(x.month), version: 2, kind: 'tag-new', month: x.month, prompt_version: PROMPT_VERSION, briefing_key: x.briefingKey,
        selection_rule: `works dated in ${x.month} with no row in current.jsonl`, item_count: x.newItems.length,
        model: base.model, effort: base.effort, thinking: 'adaptive', max_usd: base.maxUsd,
        tagged_items: r.records.length, failed_keys: r.failed, events_dropped: eventsDropped,
        author_entities_dropped: r.problems.filter((s) => s.startsWith(AUTHOR_DROP_PREFIX)).length,
        ai_count: r.records.filter((s) => s.ai_related).length,
        prompt_tokens_system: p.fit.systemTokens, prompt_largest_total_counted: p.fit.systemTokens + maxUserReal, cache_stats: cacheStats(r.requests),
        prompt_files: { text: 'prompt-tagging-system.txt', blocks: 'prompt-tagging-system.json', sha256_16: sha16(systemText(p.fit.system)) },
        month_index: { lines: p.fit.index.lines, gist_chars: p.fit.index.gistChars, new_sids: [x.newItems[0].sid, x.newItems[x.newItems.length - 1].sid] },
        stages: { tagging: { finished: new Date().toISOString(), wall_seconds: Math.round((Date.now() - started) / 1000), requests: r.requests.length, batches: p.batches.length,
          usage: r.usage, cost_usd: +r.cost.toFixed(4), stopped_at_cap: r.stopped, problems: r.problems.slice(0, 300), problem_count: r.problems.length } },
        started: new Date(started).toISOString(),
      };
      if (r.stopped) { stopped = true; report.not_run_budget.push({ month: x.month, items: x.newItems.length, partial_discarded: true, cost_usd: +r.cost.toFixed(4) }); continue; }
      // dropped event ids are this stage's designed outcome (counted above), not a structural fault
      const tg = run.stages.tagging;
      const sp = structuralProblem({ ...run, stages: { tagging: { ...tg, problems: tg.problems.filter((s) => !/^unknown event /.test(s)), problem_count: tg.problem_count - eventsDropped } } });
      if (sp) { report.rejected.push({ month: x.month, reason: sp, cost_usd: +r.cost.toFixed(4) }); continue; }
      // complete: write the folder, then the rows
      const dir = v2.paths(p.env).run;
      if (existsSync(dir)) throw new Error(`${dir} already exists`);
      mkdirSync(dir, { recursive: true });
      r.records.sort((a, b) => a.key.localeCompare(b.key));
      writeFileSync(join(dir, 'enrichment.jsonl'), x.newItems.map((it) => JSON.stringify({ key: it.key, sid: it.sid, ...p.contexts.get(it.key) })).join('\n') + '\n');
      writeFileSync(join(dir, 'prompt-tagging-system.txt'), systemText(p.fit.system));
      writeFileSync(join(dir, 'prompt-tagging-system.json'), JSON.stringify(p.fit.system, null, 2) + '\n');
      writeFileSync(join(dir, 'tags.jsonl'), r.records.map((s) => JSON.stringify(s)).join('\n') + (r.records.length ? '\n' : ''));
      writeFileSync(join(dir, 'run.json'), JSON.stringify(run, null, 2) + '\n');
      const evNames = Object.fromEntries(briefing.events.map((e) => [`${x.briefingKey}/${e.id}`, e.name]));
      const rows = r.records.map((rec) => buildCurrentRow({ ...rec, month: x.month }, { runId: runName(x.month), briefingKey: x.briefingKey, aliases, workIndex, evNames }).row);
      appendToCurrent(currentPath, rows);
      report.events_dropped_total += eventsDropped;
      report.months.push({ month: x.month, briefing_key: x.briefingKey, new_items: x.newItems.length, tagged: r.records.length, failed: r.failed.length, events_dropped: eventsDropped, cost_usd: +r.cost.toFixed(4), run_dir: runName(x.month) });
      log(`${x.month}: ${r.records.length}/${x.newItems.length} tagged, $${r.cost.toFixed(3)}`);
    }
  } finally {
    Object.assign(ledger, report, { finished: new Date().toISOString(), cost_usd_total: +spent.toFixed(4) });
    writeFileSync(ledgerPath, JSON.stringify(ledger, null, 2) + '\n');
  }
  if (treeHash(ctxDir) !== ctxBefore) throw new Error('analysis/context changed during tag-new (this must never happen)');
  ledger.context_unchanged = true;
  const readingsAfter = readingsStatus(A);
  ledger.readings_unchanged = readingsAfter === null ? 'unchecked (not a git repo)' : readingsAfter === readingsBefore;
  writeFileSync(ledgerPath, JSON.stringify(ledger, null, 2) + '\n');
  return { ...report, cost_usd_total: +spent.toFixed(4), context_unchanged: true, ledger: ledgerPath };
}

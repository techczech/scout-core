// v2 stages for one month. Each stage reads the previous stages' files and writes its own; any stage
// can be re-run alone. Writes only under <archive>/analysis/. Spend across paid stages is capped (maxUsd).
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, join, relative } from 'node:path';
import { selectWorks } from './works.js';
import { assignShortIds } from './month-index.js';
import { estimateTokens, costFromUsage, PRICES } from './cost.js';
import { packItems } from './batching.js';
import { readingsStatus } from './guard.js';
import { readwiseTagCounts, parseTweetStream, condenseTweets, loadOwnTweets, findRoundup, condenseRoundup } from './sources.js';
import { buildArchiveIndex, loadXSnapshot, enrichItems } from './enrich.js';
import { BRIEFING_PROMPT_VERSION, BRIEFING_SCHEMA, briefingSystem, condensedView, validateBriefing, renderBriefingMarkdown, briefingForPrompt } from './briefing.js';
import { THEMES_PROMPT_VERSION, THEMES_SCHEMA, THEMES_SYSTEM, themesUserMessage, validateThemes, themesForPrompt } from './themes.js';
import { PROMPT_VERSION, PROMPT_CAP, TAG_INSTRUCTIONS, fitPrompt, renderItem, systemText, promptEstimate, userMessageV2 } from './tag-prompt.js';
import { aliasMap } from './tag-results.js';
import { callJson, tagAll, cacheStats } from './tag-run.js';
import { qaSample, agreement, shuffle } from './qa.js';
import { renderReviewV2 } from './review-v2.js';
import { exampleFor } from './review.js';

const readJson = (p) => JSON.parse(readFileSync(p, 'utf8'));
const writeJson = (p, o) => { mkdirSync(dirname(p), { recursive: true }); writeFileSync(p, JSON.stringify(o, null, 2) + '\n'); };
const writeText = (p, s) => { mkdirSync(dirname(p), { recursive: true }); writeFileSync(p, s); };
const readJsonl = (p) => readFileSync(p, 'utf8').split('\n').filter(Boolean).map((l) => JSON.parse(l));
const sha16 = (s) => createHash('sha256').update(s).digest('hex').slice(0, 16);
const stripFrontmatter = (s) => s.replace(/^---\n[\s\S]*?\n---\n?/, '').trim();

export function paths(env) {
  const a = env.archiveRoot;
  const ctx = join(a, 'analysis/context');
  const run = join(a, 'analysis/topics', env.runId);
  return {
    ctx,
    profile: join(ctx, 'reader-profile.md'),
    readwiseTags: join(ctx, 'readwise-tags.json'),
    briefingMd: join(ctx, 'briefings', `${env.month}.md`),
    briefingJson: join(ctx, 'briefings', `${env.month}-briefing.json`),
    eventsJson: join(ctx, 'briefings', `${env.month}-events.json`),
    themes: join(ctx, 'themes-draft.json'),
    run,
    runJson: join(run, 'run.json'),
    enrichment: join(run, 'enrichment.jsonl'),
    promptText: join(run, 'prompt-tagging-system.txt'),
    promptJson: join(run, 'prompt-tagging-system.json'),
    tags: join(run, 'tags.jsonl'),
    qaJsonl: join(run, 'qa.jsonl'),
    review: join(run, 'review.html'),
    v1Run: join(a, 'analysis/topics', env.v1RunId || ''),
    tweetStream: join(env.tweetsDir, `${env.month}.md`),
  };
}

export function monthRange(month) {
  if (!/^\d{4}-\d{2}$/.test(month)) throw new Error('--month must be YYYY-MM');
  return { from: `${month}-01`, to: `${month}-31` };
}

export function loadMonthItems(env) {
  const { from, to } = monthRange(env.month);
  const { items, stats } = selectWorks(join(env.archiveRoot, 'readings/works'), from, to);
  return { items: assignShortIds(items), stats };
}

function loadRun(env) {
  const p = paths(env);
  return existsSync(p.runJson) ? readJson(p.runJson) : null;
}

function saveRun(env, run) { writeJson(paths(env).runJson, run); }

function baseRun(env, items, stats) {
  return {
    run_id: env.runId, version: 2, month: env.month,
    prompt_version: PROMPT_VERSION,
    selection_rule: `works whose latest highlighted_at is in ${monthRange(env.month).from}..${monthRange(env.month).to}`,
    item_count: items.length, undated_works_skipped: stats.undated, key_fallbacks: stats.keyFallbacks,
    readings_status_baseline: readingsStatus(env.archiveRoot),
    max_usd: env.maxUsd, stages: {}, started: new Date().toISOString(),
  };
}

// Spend recorded so far for this month's v2 run: briefing + themes + tagging + QA.
export function spentSoFar(env) {
  const p = paths(env);
  let s = 0;
  if (existsSync(p.briefingJson)) s += readJson(p.briefingJson).meta.cost_usd || 0;
  if (existsSync(p.themes)) s += readJson(p.themes).meta.cost_usd || 0;
  const run = loadRun(env);
  if (run) for (const k of ['tagging', 'qa']) s += (run.stages[k] && run.stages[k].cost_usd) || 0;
  return s;
}

function guardSpend(env, stage, estimate, { replacing = 0 } = {}) {
  const spent = spentSoFar(env) - replacing;
  env.log(`${stage}: estimated $${estimate.toFixed(3)}; spent so far $${spent.toFixed(3)} of $${env.maxUsd}`);
  if (spent + estimate > env.maxUsd) throw new Error(`${stage}: estimate $${estimate.toFixed(3)} + spent $${spent.toFixed(3)} exceeds cap $${env.maxUsd}; aborting before any request`);
  return env.maxUsd - spent;
}

function readingsCheck(env, run) {
  const now = readingsStatus(env.archiveRoot);
  if (now === null) return 'unchecked (not a git repo)';
  return now === (run ? run.readings_status_baseline : now);
}

// ---------- stage 1: profile sources (the profile itself is written by hand) ----------
export function stageProfileSources(env) {
  const p = paths(env);
  const tags = readwiseTagCounts(join(env.archiveRoot, 'readings/works'));
  writeJson(p.readwiseTags, { generated: new Date().toISOString(), source: 'highlighted_at lines "| tags: ..." in readings/works, case-folded', ...tags });
  env.log(`readwise tags: ${tags.distinct_tags} distinct, top ${tags.tags.length} written`);
  return { readwiseTags: p.readwiseTags, profileExists: existsSync(p.profile) };
}

// ---------- stage 2: month briefing ----------
export async function stageBriefing(env) {
  const p = paths(env);
  const { items } = loadMonthItems(env);
  const view = condensedView(items);
  const system = briefingSystem(env.month);
  const user = `Items of ${env.month} (${items.length}; id | date | type | author | [title |] first characters):\n${view}`;
  const estIn = Math.ceil((system.length + user.length) / 3);
  const replacing = existsSync(p.briefingJson) ? readJson(p.briefingJson).meta.cost_usd || 0 : 0;
  guardSpend(env, 'briefing', costFromUsage(env.briefingModel, { input_tokens: estIn, output_tokens: 30000 }), { replacing });
  const r = await callJson(env.client, { model: env.briefingModel, effort: 'medium', system, user, schema: BRIEFING_SCHEMA, maxTokens: 64000 });
  const b = validateBriefing(r.parsed, items.map((i) => i.sid));
  const meta = {
    month: env.month, model: env.briefingModel, effort: 'medium', prompt_version: BRIEFING_PROMPT_VERSION,
    generated: new Date().toISOString(), items: items.length, view_chars: view.length,
    usage: r.usage, cost_usd: +r.cost.toFixed(4), problems: b.problems, system_prompt: system,
  };
  writeJson(p.briefingJson, { meta, ...b });
  writeJson(p.eventsJson, { month: env.month, model: env.briefingModel, generated: meta.generated, events: b.events.map(({ id, name, date, aliases, description, items: its }) => ({ id, name, date, aliases, description, items: its })) });
  writeText(p.briefingMd, renderBriefingMarkdown(env.month, b, meta));
  env.log(`briefing: ${b.events.length} events, ${b.names.length} names, ${b.debates.length} debates; $${r.cost.toFixed(3)}`);
  return { events: b.events.length, cost: r.cost, usage: r.usage };
}

// ---------- stage 3: themes draft ----------
export async function stageThemes(env) {
  const p = paths(env);
  for (const f of [p.profile, p.briefingMd, p.readwiseTags, join(p.v1Run, 'vocabulary.json')]) if (!existsSync(f)) throw new Error(`themes needs ${f}`);
  const user = themesUserMessage({
    profile: stripFrontmatter(readFileSync(p.profile, 'utf8')),
    briefingMd: stripFrontmatter(readFileSync(p.briefingMd, 'utf8')),
    v1Vocabulary: readJson(join(p.v1Run, 'vocabulary.json')),
    readwiseTags: readJson(p.readwiseTags),
  });
  const replacing = existsSync(p.themes) ? readJson(p.themes).meta.cost_usd || 0 : 0;
  guardSpend(env, 'themes', costFromUsage(env.themesModel, { input_tokens: Math.ceil((THEMES_SYSTEM.length + user.length) / 3), output_tokens: 20000 }), { replacing });
  const r = await callJson(env.client, { model: env.themesModel, effort: 'high', system: THEMES_SYSTEM, user, schema: THEMES_SCHEMA, maxTokens: 64000 });
  const t = validateThemes(r.parsed);
  const meta = {
    status: 'DRAFT', note: 'Draft from the September 2026 briefing only; the full version will use all monthly briefings. Awaiting Dominik\'s review.',
    model: env.themesModel, effort: 'high', prompt_version: THEMES_PROMPT_VERSION, generated: new Date().toISOString(),
    inputs: { profile: relative(env.archiveRoot, p.profile), briefing: relative(env.archiveRoot, p.briefingMd), v1_vocabulary: relative(env.archiveRoot, join(p.v1Run, 'vocabulary.json')), readwise_tags: relative(env.archiveRoot, p.readwiseTags) },
    usage: r.usage, cost_usd: +r.cost.toFixed(4), system_prompt: THEMES_SYSTEM,
  };
  writeJson(p.themes, { meta, notes: t.notes, themes: t.themes });
  env.log(`themes: ${t.themes.length}; $${r.cost.toFixed(3)}`);
  return { themes: t.themes.length, cost: r.cost };
}

// ---------- stage 4: context enrichment ----------
export function stageEnrich(env) {
  const p = paths(env);
  const { items, stats } = loadMonthItems(env);
  const run = loadRun(env) || baseRun(env, items, stats);
  const archive = buildArchiveIndex(env.archiveRoot);
  const snapshot = loadXSnapshot(join(env.archiveRoot, 'imports'));
  const ownTweets = loadOwnTweets(env.tweetsDir);
  const { contexts, counts } = enrichItems(items, { archive, snapshot, ownTweets, archiveRoot: env.archiveRoot });
  mkdirSync(p.run, { recursive: true });
  writeText(p.enrichment, items.map((it) => JSON.stringify({ key: it.key, sid: it.sid, ...contexts.get(it.key) })).join('\n') + '\n');
  run.stages.enrichment = {
    finished: new Date().toISOString(), counts,
    sources: { archive_tweet_ids: archive.byTweetId.size, archive_urls: archive.byUrl.size, x_snapshot_rows: snapshot.size, own_tweets: ownTweets.size },
    file: 'enrichment.jsonl',
  };
  saveRun(env, run);
  env.log(`enrichment: ${JSON.stringify(counts)}`);
  return counts;
}

// Build (or reload) the exact system blocks used for tagging and QA.
async function tagPrompt(env, items, { count }) {
  const p = paths(env);
  for (const f of [p.profile, p.themes, p.briefingJson, p.enrichment]) if (!existsSync(f)) throw new Error(`tagging needs ${f}`);
  const themes = readJson(p.themes).themes;
  const briefing = readJson(p.briefingJson);
  const tw = existsSync(p.tweetStream) ? condenseTweets(parseTweetStream(readFileSync(p.tweetStream, 'utf8')), { maxChars: env.tweetsChars }) : null;
  const rpath = env.roundupPath || findRoundup(env.roundupDir, env.month);
  const rd = rpath ? condenseRoundup(readJson(rpath), { maxChars: env.roundupChars }) : null;
  const fixed = {
    instructions: TAG_INSTRUCTIONS,
    reader_profile: stripFrontmatter(readFileSync(p.profile, 'utf8')),
    themes: themesForPrompt(themes),
    month_briefing: briefingForPrompt(briefing),
    reader_tweets: tw ? `The reader's own posts on X in ${env.month} (${tw.tweets} posts, each cut to ${tw.perTweetCap} chars):\n${tw.text}` : '',
    mondai_roundup: rd ? `The reader's own monthly AI roundup that covers this period (condensed):\n${rd.text}` : '',
  };
  const fit = await fitPrompt(fixed, items, { cap: PROMPT_CAP, userReserve: env.userReserve, count });
  return { ...fit, themes, briefing, sources: { tweet_stream: tw ? { path: p.tweetStream, tweets: tw.tweets, per_tweet_cap: tw.perTweetCap } : null, roundup: rd ? { path: rpath, items: rd.items, per_item_cap: rd.perItemCap } : null } };
}

const realCount = (client, model) => async (system, user) => (await client.messages.countTokens({ model, system, messages: [{ role: 'user', content: user || '.' }] })).input_tokens;

// sidOf must cover the whole month so thread hints resolve even when only a sample is packed.
export function packForTagging(items, contexts, maxTokens, sidOf = new Map(items.map((i) => [i.key, i.sid]))) {
  const rendered = items.map((it) => ({ ...it, text: renderItem(it, contexts.get(it.key), sidOf) }));
  return packItems(rendered, { maxItems: 20, maxTokens, estimate: promptEstimate });
}

function estimateTagging(model, systemTokens, batches, outPerItem = 250, thinkingPerReq = 2500) {
  const p = PRICES[model];
  let usd = systemTokens * p.in * p.cacheWrite / 1e6; // first write
  for (const b of batches) usd += (systemTokens * p.in * p.cacheRead + b.estTokens * p.in + (b.items.length * outPerItem + thinkingPerReq) * p.out) / 1e6;
  return usd * 1.25; // margin for misses and retries
}

// ---------- stage 5: tagging ----------
export async function stageTag(env) {
  const p = paths(env);
  const { items, stats } = loadMonthItems(env);
  const run = loadRun(env) || baseRun(env, items, stats);
  if (existsSync(p.tags) && !env.force && !env.dryRun) throw new Error(`${p.tags} exists; pass --force to re-tag`);
  const contexts = new Map(readJsonl(p.enrichment).map((r) => [r.key, r]));
  const fit = await tagPrompt(env, items, { count: realCount(env.client, env.model) });
  const batches = packForTagging(items, contexts, Math.min(env.userReserve, fit.maxUserTokens));
  // Verify the cap with real counts: system + the 3 longest user messages.
  const longest = [...batches].sort((a, b) => userMessageV2(b).length - userMessageV2(a).length).slice(0, 3);
  let maxUserReal = 0;
  for (const b of longest) maxUserReal = Math.max(maxUserReal, await realCount(env.client, env.model)([{ type: 'text', text: '.' }], userMessageV2(b)) - 1);
  if (fit.systemTokens + maxUserReal >= PROMPT_CAP) throw new Error(`largest prompt would be ${fit.systemTokens + maxUserReal} tokens (cap ${PROMPT_CAP}); lower --user-reserve or the source budgets`);
  const replacing = (run.stages.tagging && run.stages.tagging.cost_usd) || 0;
  const estimate = estimateTagging(env.model, fit.systemTokens, batches);
  if (env.dryRun) {
    return { dryRun: true, systemTokens: fit.systemTokens, maxUserTokensReal: maxUserReal, largestPromptReal: fit.systemTokens + maxUserReal, maxUserTokensEstimated: Math.max(...batches.map((b) => b.estTokens)), batches: batches.length, gistChars: fit.index.gistChars, sizes: fit.sizes, estimateUsd: +estimate.toFixed(4), spentSoFar: +spentSoFar(env).toFixed(4) };
  }
  const remaining = guardSpend(env, 'tagging', estimate, { replacing });
  writeText(p.promptText, systemText(fit.system));
  writeJson(p.promptJson, fit.system);
  const vctx = { themes: fit.themes, events: fit.briefing.events, aliases: aliasMap(fit.briefing) };
  const started = Date.now();
  const r = await tagAll(env.client, { model: env.model, effort: env.effort, system: fit.system, vctx }, batches, { concurrency: env.concurrency, maxUsd: remaining, log: env.log });
  r.records.sort((a, b) => a.key.localeCompare(b.key));
  writeText(p.tags, r.records.map((x) => JSON.stringify(x)).join('\n') + (r.records.length ? '\n' : ''));
  const cs = cacheStats(r.requests);
  const ai = r.records.filter((x) => x.ai_related).length;
  Object.assign(run, {
    model: env.model, effort: env.effort, thinking: 'adaptive', api: 'messages (prompt caching, 5-minute TTL)',
    tagged_items: r.records.length, failed_keys: r.failed, ai_count: ai, ai_share: +(ai / Math.max(r.records.length, 1)).toFixed(4),
    prompt_cap: PROMPT_CAP, prompt_tokens_system: fit.systemTokens, prompt_max_user_tokens_estimated: Math.max(...batches.map((b) => b.estTokens)),
    prompt_max_user_tokens_counted: maxUserReal, prompt_largest_total_counted: fit.systemTokens + maxUserReal,
    prompt_sizes: fit.sizes, month_index: { lines: fit.index.lines, gist_chars: fit.index.gistChars },
    prompt_files: { text: 'prompt-tagging-system.txt', blocks: 'prompt-tagging-system.json', sha256_16: sha16(systemText(fit.system)) },
    prompt_sources: fit.sources, cache_stats: cs,
    confidence_counts: ['low', 'medium', 'high'].reduce((o, c) => ({ ...o, [c]: r.records.filter((x) => x.confidence === c).length }), {}),
  });
  run.stages.tagging = {
    finished: new Date().toISOString(), wall_seconds: Math.round((Date.now() - started) / 1000), requests: r.requests.length, batches: batches.length,
    usage: r.usage, cost_usd: +r.cost.toFixed(4), stopped_at_cap: r.stopped, problems: r.problems.slice(0, 300), problem_count: r.problems.length,
  };
  run.readings_unchanged = readingsCheck(env, run);
  saveRun(env, run);
  env.log(`tagging: ${r.records.length}/${items.length} items, failed ${r.failed.length}, $${r.cost.toFixed(3)}, cache token hit rate ${cs.token_hit_rate}`);
  return { tagged: r.records.length, failed: r.failed.length, cost: r.cost, cache: cs };
}

// ---------- stage 6: QA ----------
export async function stageQa(env) {
  const p = paths(env);
  const run = loadRun(env);
  if (!run || !existsSync(p.tags) || !existsSync(p.promptJson)) throw new Error('QA needs a finished tagging stage');
  const { items } = loadMonthItems(env);
  const records = readJsonl(p.tags);
  const byKey = new Map(records.map((r) => [r.key, r]));
  const sample = qaSample(records, { seed: env.seed });
  const contexts = new Map(readJsonl(p.enrichment).map((r) => [r.key, r]));
  const chosen = items.filter((it) => sample.group[it.key]);
  const system = readJson(p.promptJson);
  const themes = readJson(p.themes).themes;
  const briefing = readJson(p.briefingJson);
  const batches = packForTagging(chosen, contexts, env.userReserve, new Map(items.map((i) => [i.key, i.sid])));
  const replacing = (run.stages.qa && run.stages.qa.cost_usd) || 0;
  const remaining = guardSpend(env, 'qa', estimateTagging(env.qaModel, run.prompt_tokens_system * 1.1, batches), { replacing });
  const r = await tagAll(env.client, { model: env.qaModel, effort: env.qaEffort, system, vctx: { themes, events: briefing.events, aliases: aliasMap(briefing) } }, batches, { concurrency: env.concurrency, maxUsd: remaining, log: env.log });
  const pairs = r.records.map((b) => ({ key: b.key, group: sample.group[b.key], a: byKey.get(b.key), b }));
  writeText(p.qaJsonl, pairs.map((x) => JSON.stringify({ key: x.key, group: x.group, tagger: x.a, qa: x.b })).join('\n') + '\n');
  const qa = {
    all: agreement(pairs),
    random: agreement(pairs.filter((x) => x.group === 'random')),
    low_confidence: agreement(pairs.filter((x) => x.group === 'low-confidence')),
    low_confidence_total: sample.low_total, low_confidence_in_sample: sample.low_included,
  };
  run.qa_model = env.qaModel;
  run.qa = qa;
  run.stages.qa = {
    finished: new Date().toISOString(), model: env.qaModel, effort: env.qaEffort, seed: env.seed, sampled: sample.keys.length, compared: pairs.length, failed: r.failed,
    usage: r.usage, cost_usd: +r.cost.toFixed(4), cache_stats: cacheStats(r.requests), problems: r.problems.slice(0, 100),
  };
  run.readings_unchanged = readingsCheck(env, run);
  saveRun(env, run);
  env.log(`qa: ${pairs.length} compared; themes overlap ${qa.all.themes_any_overlap}, exact ${qa.all.themes_exact}; event ${qa.all.event_match}; $${r.cost.toFixed(3)}`);
  return qa;
}

// ---------- stage 7: review page ----------
export function stageReview(env) {
  const p = paths(env);
  const run = loadRun(env);
  if (!run || !existsSync(p.tags)) throw new Error('review needs a finished tagging stage');
  const records = readJsonl(p.tags);
  const { items } = loadMonthItems(env);
  const sidOf = new Map(items.map((i) => [i.key, i.sid]));
  const themes = readJson(p.themes).themes;
  const briefing = readJson(p.briefingJson);
  const themesMeta = readJson(p.themes).meta;
  const rank = { high: 0, medium: 1, low: 2 };
  const pick = (recs) => [...recs].sort((a, b) => rank[a.confidence] - rank[b.confidence] || a.key.localeCompare(b.key)).slice(0, 3).map((r) => r.key);
  const themeRows = themes.map((t) => {
    const hits = records.filter((r) => r.themes.includes(t.name));
    return { name: t.name, definition: t.definition, count: hits.length, examples: pick(shuffle(hits, 7)) };
  }).sort((a, b) => b.count - a.count);
  const entCounts = new Map();
  for (const r of records) for (const e of r.entities) entCounts.set(e, (entCounts.get(e) || 0) + 1);
  const entities = [...entCounts.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).slice(0, 50).map(([name, count]) => ({ name, count }));
  const eventRows = briefing.events.map((e) => {
    const hits = records.filter((r) => r.event === e.id);
    return { ...e, count: hits.length, examples: pick(shuffle(hits, 11)) };
  });
  const evName = new Map(briefing.events.map((e) => [e.id, `${e.id} · ${e.name}`]));
  let v1 = new Map();
  const v1Tags = join(p.v1Run, 'tags.jsonl');
  if (existsSync(v1Tags)) v1 = new Map(readJsonl(v1Tags).map((r) => [r.key, r.topics]));
  const compare = shuffle(records.filter((r) => v1.has(r.key)), env.seed + 2).slice(0, 15)
    .map((r) => ({ key: r.key, v1: v1.get(r.key), themes: r.themes, entities: r.entities, event: r.event ? evName.get(r.event) : null }));
  const byKey = new Map(records.map((r) => [r.key, r]));
  const wanted = new Set([...themeRows.flatMap((t) => t.examples), ...eventRows.flatMap((e) => e.examples), ...compare.map((c) => c.key)]);
  const examples = {};
  for (const k of wanted) {
    const rec = byKey.get(k);
    examples[k] = { ...exampleFor(env.archiveRoot, rec), sid: sidOf.get(k), path: rec.path, relevance: rec.relevance, themes: rec.themes, entities: rec.entities };
  }
  const costs = [
    { stage: 'Month briefing', model: briefing.meta.model, usd: briefing.meta.cost_usd },
    { stage: 'Themes draft', model: themesMeta.model, usd: themesMeta.cost_usd },
    { stage: 'Tagging', model: run.model, usd: run.stages.tagging && run.stages.tagging.cost_usd },
    { stage: 'QA re-tag', model: run.qa_model, usd: run.stages.qa && run.stages.qa.cost_usd },
  ];
  run.cost_usd_by_stage = Object.fromEntries(costs.map((c) => [c.stage, c.usd || 0]));
  run.cost_usd_total = +costs.reduce((s, c) => s + (c.usd || 0), 0).toFixed(4);
  run.cost_note = 'per request from response usage at list prices: haiku-5-5 $0.10/$0.50, sonnet-5-5 $2/$10, opus-5-5 $4/$20 per MTok; cache reads 0.1x (opus 0.05x), 5-minute cache writes 1.25x';
  run.context_files = {
    reader_profile: relative(env.archiveRoot, p.profile), readwise_tags: relative(env.archiveRoot, p.readwiseTags),
    briefing: relative(env.archiveRoot, p.briefingMd), briefing_json: relative(env.archiveRoot, p.briefingJson), events: relative(env.archiveRoot, p.eventsJson),
    themes_draft: relative(env.archiveRoot, p.themes),
  };
  run.entities_distinct = entCounts.size;
  run.event_tagged_items = records.filter((r) => r.event).length;
  run.finished = new Date().toISOString();
  run.readings_unchanged = readingsCheck(env, run);
  saveRun(env, run);
  const html = renderReviewV2({
    run, profileMd: readFileSync(p.profile, 'utf8'), briefingMd: readFileSync(p.briefingMd, 'utf8'),
    profileHref: relative(p.run, p.profile), briefingHref: relative(p.run, p.briefingMd),
    themes: themeRows, entities, events: eventRows, qa: run.qa, costs, compare, examples,
  });
  writeText(p.review, html);
  env.log(`review: ${p.review}`);
  return { review: p.review, total: run.cost_usd_total };
}

export { estimateTokens };

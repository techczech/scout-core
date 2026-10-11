// Validation of v2 model output (themes from a fixed list, free entities, event id, relevance,
// confidence) and conversion to tags.jsonl records. Ids in the batch are month-index sids.
import { MAX_ENTITIES, MAX_THEMES_PER_ITEM } from './tag-prompt.js';

export const MAX_RELEVANCE_WORDS = 15;
export const MAX_ENTITY_CHARS = 60;
const CONF = new Set(['low', 'medium', 'high']);

const squash = (s) => String(s ?? '').replace(/\s+/g, ' ').trim();

/**
 * ctx: {themes:[{name}], events:[{id}], aliases: Map(lowercased alias -> canonical name)}.
 * Build aliases with aliasMap(briefing). Returns {records, missing, problems}.
 */
export function validateTagBatch(parsed, batch, ctx) {
  const bySid = new Map(batch.items.map((it) => [it.sid, it]));
  const themeByLower = new Map(ctx.themes.map((t) => [t.name.toLowerCase(), t.name]));
  const eventIds = new Set(ctx.events.map((e) => e.id));
  const aliases = ctx.aliases || new Map();
  const problems = [];
  const records = new Map();
  const rows = parsed && Array.isArray(parsed.results) ? parsed.results : null;
  if (!rows) return { records: [], missing: batch.items.map((i) => i.key), problems: ['no results array'] };
  for (const r of rows) {
    const it = bySid.get(r && r.id);
    if (!it) { problems.push(`unknown id ${r && r.id}`); continue; }
    if (records.has(it.key)) { problems.push(`duplicate id ${r.id}`); continue; }
    if (typeof r.ai_related !== 'boolean') { problems.push(`bad ai_related for ${r.id}`); continue; }
    // themes: exact list names (case-insensitive match), deduped, max 3
    const themes = [];
    for (const raw of Array.isArray(r.themes) ? r.themes : []) {
      const name = themeByLower.get(squash(raw).toLowerCase());
      if (!name) { problems.push(`theme outside list for ${r.id}: ${squash(raw)}`); continue; }
      if (!themes.includes(name)) themes.push(name);
    }
    if (themes.length > MAX_THEMES_PER_ITEM) themes.length = MAX_THEMES_PER_ITEM;
    // entities: free, alias-normalised to the briefing's full names, deduped case-insensitively
    const entities = [];
    const seen = new Set();
    for (const raw of Array.isArray(r.entities) ? r.entities : []) {
      let e = squash(raw);
      if (!e || e.length > MAX_ENTITY_CHARS) { if (e) problems.push(`entity too long for ${r.id}`); continue; }
      e = aliases.get(e.toLowerCase()) || e;
      if (seen.has(e.toLowerCase())) continue;
      seen.add(e.toLowerCase());
      entities.push(e);
    }
    if (entities.length > MAX_ENTITIES) entities.length = MAX_ENTITIES;
    // event: a known id or null
    let event = squash(r.event).toUpperCase() || null;
    if (event && !eventIds.has(event)) { problems.push(`unknown event ${event} for ${r.id}`); event = null; }
    let confidence = CONF.has(r.confidence) ? r.confidence : 'low';
    if (!CONF.has(r.confidence)) problems.push(`bad confidence for ${r.id}`);
    if (r.ai_related && themes.length === 0) { problems.push(`AI item without themes ${r.id}`); confidence = 'low'; }
    const relevance = squash(r.relevance).split(' ').filter(Boolean).slice(0, MAX_RELEVANCE_WORDS).join(' ');
    records.set(it.key, {
      key: it.key, path: it.path, date: it.date, type: it.type,
      ai_related: r.ai_related, themes, entities, event, relevance, confidence,
    });
  }
  const missing = batch.items.filter((i) => !records.has(i.key)).map((i) => i.key);
  return { records: [...records.values()], missing, problems };
}

// Lower-cased alias -> canonical name, from the briefing's names list. Only safe aliases are used:
// every word of the alias must occur in the full name ("Astra" -> "GPT-6 Astra", "Opus 5.5" -> "Claude Opus 5.5"),
// so a wrong or looser alias ("Fable 5" listed under "Claude Fable 5.1") cannot rewrite a different entity.
// Combined entries ("GPT-6 Sol / Luna") are skipped.
const words = (s) => squash(s).toLowerCase().split(/[\s/]+/).filter(Boolean);
export function aliasMap(briefing) {
  const m = new Map();
  const names = (briefing.names || []).filter((n) => !n.name.includes('/'));
  for (const n of names) {
    const full = new Set(words(n.name));
    for (const a of n.aliases || []) {
      const k = squash(a).toLowerCase();
      if (!k || m.has(k)) continue;
      if (words(a).every((w) => full.has(w))) m.set(k, n.name);
    }
  }
  for (const n of names) m.set(n.name.toLowerCase(), n.name); // canonical spelling wins
  return m;
}

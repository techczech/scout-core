// Validation of v3 model output (themes from a fixed list, typed entities minus the item's own author/site,
// event id, related items by month-index id, confidence) and conversion to tags.jsonl records. Ids in the batch are month-index sids.
import { MAX_ENTITIES, MAX_THEMES_PER_ITEM, MAX_RELATED, ENTITY_TYPES, RELATIONS } from './tag-prompt.js';
import { dropOwnEntities } from './entity-guard.js';

export const AUTHOR_DROP_PREFIX = 'entity is own author/site ';
export const MAX_ENTITY_CHARS = 60;
const CONF = new Set(['low', 'medium', 'high']);

const squash = (s) => String(s ?? '').replace(/\s+/g, ' ').trim();

/**
 * ctx: {themes:[{name}], events:[{id}], aliases: Map(lowercased alias -> canonical name), sidKey: Map(month-index id -> key)}.
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
    // entities: {name, type}; alias-normalised to the briefing's full names, deduped case-insensitively,
    // own author/handle/site removed (entity-guard), capped
    let entities = [];
    const seen = new Set();
    for (const raw of Array.isArray(r.entities) ? r.entities : []) {
      let e = squash(raw && typeof raw === 'object' ? raw.name : raw);
      if (!e || e.length > MAX_ENTITY_CHARS) { if (e) problems.push(`entity too long for ${r.id}`); continue; }
      e = aliases.get(e.toLowerCase()) || e;
      if (seen.has(e.toLowerCase())) continue;
      seen.add(e.toLowerCase());
      const type = raw && ENTITY_TYPES.includes(raw.type) ? raw.type : 'other';
      entities.push({ name: e, type });
    }
    const g = dropOwnEntities(entities, it);
    for (const n of g.dropped) problems.push(`${AUTHOR_DROP_PREFIX}${r.id}: ${n}`);
    entities = g.kept;
    if (entities.length > MAX_ENTITIES) entities.length = MAX_ENTITIES;
    // related: other items of the month by index id, stored as stable keys; self-links, unknown ids and repeats dropped
    const related = [];
    for (const raw of Array.isArray(r.related) ? r.related : []) {
      const id = squash(raw && raw.id);
      const key = ctx.sidKey && ctx.sidKey.get(id);
      if (!key) { problems.push(`unknown related id ${id} for ${r.id}`); continue; }
      if (key === it.key || related.some((x) => x.key === key)) continue;
      if (!RELATIONS.includes(raw.relation)) { problems.push(`bad relation for ${r.id}`); continue; }
      related.push({ key, relation: raw.relation });
    }
    if (related.length > MAX_RELATED) related.length = MAX_RELATED;
    // event: a known id or null
    let event = squash(r.event).toUpperCase() || null;
    if (event && !eventIds.has(event)) { problems.push(`unknown event ${event} for ${r.id}`); event = null; }
    let confidence = CONF.has(r.confidence) ? r.confidence : 'low';
    if (!CONF.has(r.confidence)) problems.push(`bad confidence for ${r.id}`);
    if (r.ai_related && themes.length === 0) { problems.push(`AI item without themes ${r.id}`); confidence = 'low'; }
    records.set(it.key, {
      key: it.key, path: it.path, date: it.date, type: it.type,
      ai_related: r.ai_related, themes, entities, event, related, confidence,
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

// Entity merge pass across months: one normalisation map raw -> canonical, each entry with provenance.
// Rules run first (spelling variants, safe briefing aliases, unique longer name); one model call then groups
// remaining variants. Raw entities stay in each month's tags.jsonl; the map is applied when writing current.jsonl.
export const ENTITY_PROMPT_VERSION = 'entities-v1';

const squash = (s) => String(s ?? '').replace(/\s+/g, ' ').trim();
// Spelling key: case, spaces, hyphens/underscores, quotes and trailing punctuation ignored.
export const spellingKey = (s) => squash(s).toLowerCase().replace(/[‘’'"“”]/g, '').replace(/[-_\s]+/g, ' ').replace(/[.,;:!]+$/, '').trim();
const words = (s) => spellingKey(s).split(/[\s/]+/).filter(Boolean);

export function entityCounts(records) {
  const m = new Map();
  for (const r of records) for (const e of r.entities || []) m.set(e, (m.get(e) || 0) + 1);
  return m;
}

// The longer name is the short one with words added only in front (a maker prefix):
// "Fable 5.1" -> "Claude Fable 5.1" yes; "GPT-2" -> "GPT Realtime 2", "GLM-4.7" -> "GLM-4.7-Flash" no.
export function prefixOnly(short, long) {
  const s = words(short), l = words(long);
  if (l.length <= s.length) return false;
  return s.every((w, i) => l[l.length - s.length + i] === w);
}

const byCount = (counts) => (a, b) => (counts.get(b) || 0) - (counts.get(a) || 0) || a.localeCompare(b);

/**
 * counts: Map raw -> n. aliasMaps: [Map lower alias -> canonical] (from aliasMap(briefing) per month).
 * Returns {map: {raw: {to, rule}}, canonical: [names that are not mapped away]}.
 */
export function ruleMap(counts, aliasMaps = []) {
  const map = {};
  const names = [...counts.keys()];
  // 1. spelling variants -> the most frequent spelling
  const groups = new Map();
  for (const n of names) { const k = spellingKey(n); if (!groups.has(k)) groups.set(k, []); groups.get(k).push(n); }
  for (const g of groups.values()) {
    if (g.length < 2) continue;
    const top = [...g].sort(byCount(counts))[0];
    for (const n of g) if (n !== top) map[n] = { to: top, rule: 'rule:spelling' };
  }
  // 2. briefing aliases (only aliases whose words all occur in the full name; see aliasMap)
  const alias = new Map();
  for (const am of aliasMaps) for (const [k, v] of am) if (!alias.has(k)) alias.set(k, v);
  // Across months a bare family alias ("Opus", "Fable") names different models in different months, so only
  // aliases that are the full name minus a maker prefix are used ("Opus 4.5" -> "Claude Opus 4.5", "Astra" ->
  // "GPT-6 Astra"); "Opus" -> "Opus 4.6" adds a suffix and is refused.
  for (const n of names) {
    if (map[n]) continue;
    const to = alias.get(n.toLowerCase());
    if (to && to !== n && prefixOnly(n, to)) map[n] = { to, rule: 'rule:briefing-alias' };
  }
  // 3. a versioned name of 2+ words that is exactly one more frequent longer name minus a maker prefix
  //    ("Fable 5.1" -> "Claude Fable 5.1"; "GPT-2" never -> "GPT-Realtime-2"). Names without a
  //    digit are left alone so that a person is never folded into an event named after them ("Terence Tao letter").
  const canon = names.filter((n) => !map[n]);
  for (const a of canon) {
    const wa = words(a);
    if (!/\d/.test(a) || wa.length < 2) continue;
    const hits = canon.filter((b) => b !== a && (counts.get(b) || 0) >= (counts.get(a) || 0) && prefixOnly(a, b));
    if (hits.length === 1) map[a] = { to: hits[0], rule: 'rule:unique-longer-name' };
  }
  return { map, canonical: names.filter((n) => !map[n]) };
}

export const ENTITY_SYSTEM = `You clean up the names of entities (models, products, companies, people, papers, benchmarks, events) tagged in a reading archive about AI and other subjects, 2025-2026. Group names that refer to exactly the same thing written differently: abbreviations, missing company prefixes, nicknames, translations, typos.
Never merge different things: different model versions (GPT-6 and GPT-6.1 Sol; Claude Opus 5 and Claude Opus 5.5), a model family and one of its sizes or variants (Gemini 2.0 and Gemini 2.0 Flash), products of different companies that share a generic name (Google Deep Research and OpenAI Deep Research), different products of one company, a company and its product, a person or group of people and their organisation or the document or event they produced (Fields Medallists and their declaration). When unsure, leave it out.
For each group give the canonical name (the fullest common form, copied from the list) and the variants (copied exactly from the list). Leave out names that have no variants.`;

export const ENTITY_SCHEMA = {
  type: 'object',
  properties: { groups: { type: 'array', items: { type: 'object', properties: { canonical: { type: 'string' }, variants: { type: 'array', items: { type: 'string' } } }, required: ['canonical', 'variants'], additionalProperties: false } } },
  required: ['groups'], additionalProperties: false,
};

const DOC_WORDS = /\b(declaration|letter|incident|report|paper|essay|statement|project|study|post|thread|interview)\b/i;
// Deterministic vetoes on model merges: a document/event never becomes a person or org (and back), and the
// version numbers must match ("Claude Mythos" -/-> "Claude Mythos 5", "DeepSeek-V3.1" -/-> "DeepSeek v3").
export function vetoMerge(variant, canonical) {
  if (DOC_WORDS.test(variant) !== DOC_WORDS.test(canonical)) return 'document vs non-document';
  const nums = (s) => (String(s).match(/\d+(?:\.\d+)*/g) || []).join(',');
  if (nums(variant) !== nums(canonical)) return 'different version numbers'; // also family vs member, either way
  return null;
}

// Validate model groups: canonical and variants must be names from the list; a name is mapped once.
export function modelGroups(parsed, names, existing, model) {
  const known = new Set(names);
  const out = {};
  let dropped = 0;
  for (const g of (parsed && parsed.groups) || []) {
    const c = squash(g.canonical);
    if (!known.has(c)) { dropped++; continue; }
    // variants that each add their own leading words to the canonical name are rival products, not spellings
    const wc = words(c);
    const extras = new Set((g.variants || []).map((v) => words(v)).filter((wv) => wv.length > wc.length && prefixOnly(c, wv.join(' '))).map((wv) => wv.slice(0, wv.length - wc.length).join(' ')));
    const rival = extras.size > 1;
    for (const v of g.variants || []) {
      if (rival && prefixOnly(c, v)) { dropped++; continue; }
      if (vetoMerge(v, c)) { dropped++; continue; }
      if (!known.has(v) || v === c || existing[v] || out[v] || out[c]) { dropped++; continue; }
      out[v] = { to: c, rule: `model:${model}` };
    }
  }
  return { map: out, dropped };
}

// Re-apply the vetoes to a stored map (model entries only); returns {map, vetoed:[{from,to,why}]}.
export function revalidate(map) {
  const out = {}, vetoed = [];
  for (const [k, v] of Object.entries(map)) {
    const why = v.rule.startsWith('model:') ? vetoMerge(k, v.to) : null;
    if (why) vetoed.push({ from: k, to: v.to, why }); else out[k] = v;
  }
  return { map: out, vetoed };
}

// Follow chains (a -> b -> c) with a cycle guard.
export function resolveEntity(map, name) {
  let n = name;
  const seen = new Set();
  while (map[n] && !seen.has(n)) { seen.add(n); n = map[n].to; }
  return n;
}

export function normaliseEntities(map, ents) {
  const out = [];
  for (const e of ents || []) { const n = resolveEntity(map, e); if (!out.includes(n)) out.push(n); }
  return out;
}

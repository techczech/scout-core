// QA: re-tag a sample with a stronger model and measure agreement field by field.
export const QA_RANDOM = 100;
export const QA_CAP = 150;

// Deterministic PRNG (mulberry32) so a sample can be reproduced from its seed.
export function rng(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6D2B79F5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

export function shuffle(arr, seed) {
  const r = rng(seed);
  const a = [...arr];
  for (let i = a.length - 1; i > 0; i--) {
    const j = Math.floor(r() * (i + 1));
    [a[i], a[j]] = [a[j], a[i]];
  }
  return a;
}

// Random 100 plus all low-confidence records, capped at 150 (low-confidence extras sampled if over).
export function qaSample(records, { seed = 20261011, random = QA_RANDOM, cap = QA_CAP } = {}) {
  const rand = shuffle(records, seed).slice(0, Math.min(random, records.length));
  const inRand = new Set(rand.map((r) => r.key));
  const lows = shuffle(records.filter((r) => r.confidence === 'low' && !inRand.has(r.key)), seed + 1);
  const extra = lows.slice(0, Math.max(0, cap - rand.length));
  return {
    keys: [...rand.map((r) => r.key), ...extra.map((r) => r.key)],
    group: Object.fromEntries([...rand.map((r) => [r.key, 'random']), ...extra.map((r) => [r.key, 'low-confidence'])]),
    low_total: records.filter((r) => r.confidence === 'low').length,
    low_included: extra.length + rand.filter((r) => r.confidence === 'low').length,
  };
}

const lc = (xs) => new Set((xs || []).map((x) => String(x).toLowerCase()));
function setStats(a, b) {
  const A = lc(a), B = lc(b);
  const inter = [...A].filter((x) => B.has(x)).length;
  const union = new Set([...A, ...B]).size;
  return { overlap: inter > 0 || (A.size === 0 && B.size === 0), exact: union === inter, jaccard: union ? inter / union : 1 };
}

// pairs: [{a: record (tagger), b: record (QA model)}]. Rates in 0..1.
export function agreement(pairs) {
  const n = pairs.length;
  const z = { n, themes_any_overlap: 0, themes_exact: 0, themes_jaccard: 0, entities_any_overlap: 0, entities_exact: 0, entities_jaccard: 0, event_match: 0, event_both_set_match: 0, event_both_set: 0, ai_related_match: 0 };
  if (!n) return z;
  for (const { a, b } of pairs) {
    const t = setStats(a.themes, b.themes);
    const e = setStats(a.entities, b.entities);
    z.themes_any_overlap += t.overlap; z.themes_exact += t.exact; z.themes_jaccard += t.jaccard;
    z.entities_any_overlap += e.overlap; z.entities_exact += e.exact; z.entities_jaccard += e.jaccard;
    z.event_match += (a.event || null) === (b.event || null);
    if (a.event && b.event) { z.event_both_set++; z.event_both_set_match += a.event === b.event; }
    z.ai_related_match += a.ai_related === b.ai_related;
  }
  const r = (x) => +(x / n).toFixed(3);
  return {
    n,
    themes_any_overlap: r(z.themes_any_overlap), themes_exact: r(z.themes_exact), themes_mean_jaccard: r(z.themes_jaccard),
    entities_any_overlap: r(z.entities_any_overlap), entities_exact: r(z.entities_exact), entities_mean_jaccard: r(z.entities_jaccard),
    event_match: r(z.event_match),
    event_both_set: z.event_both_set,
    event_match_when_both_set: z.event_both_set ? +(z.event_both_set_match / z.event_both_set).toFixed(3) : null,
    ai_related_match: r(z.ai_related_match),
  };
}

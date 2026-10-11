// events-propose-merges, step 1 (no model): which pairs of unmerged lasting-event records might be one story.
// Pure functions over the reader's event list (`scout events --json`) and the tag rows (current.jsonl).
// A pair is a candidate when the spans are close in time AND the two share a signal:
//   entity          a named entity in >= ENTITY_SHARE of each event's items (and in >= 2 items of each), not one of the
//                   ARCHIVE_TOP most frequent entities archive-wide;
//   alias-token     a word of >= 5 letters in both events' names/aliases that is not a stopword, not a word of an
//                   archive-wide frequent entity's name, and not in the names of more than TOKEN_DF_CAP events;
//   name-similarity names share >= 2 words (or Dice >= 0.6) after dropping stopwords.
// Two records from the same month came from two different monthly events, so they are paired only on strong overlap.
export const SPAN_DAYS = 45;
export const ENTITY_SHARE = 0.3;
export const ARCHIVE_TOP = 30;
export const MAX_PARTNERS = 6;
export const TOKEN_DF_CAP = 30;
export const SIGNALS = ['entity', 'alias-token', 'name-similarity'];

const STOP = new Set(`about above after again against among around because been before being between both could does doing down during each from further have having here hers himself into itself just more only other ours over same should some such than that their theirs them then there these they this those through under until very were what when where which while whom will with within would your yours
launch launches launched announces announced announcement release released releases model models wave begins begin debate debates claims claim public new latest first second third week weeks month months year years says said news update updates version versions open new over rollout rolling story reaction reactions response responses discussion talk talks post posts thread threads paper papers study studies report reports`.split(/\s+/));

const DAY = 86400000;
const dayNum = (d) => Math.floor(Date.parse(`${d}T00:00:00Z`) / DAY);
export const gapDays = (a, b) => Math.max(0, Math.max(dayNum(a.start), dayNum(b.start)) - Math.min(dayNum(a.end), dayNum(b.end)));
const words = (s, min) => String(s || '').toLowerCase().normalize('NFKD').replace(/[̀-ͯ]/g, '').match(/[a-z][a-z0-9]*/g)?.filter((w) => w.replace(/[^a-z]/g, '').length >= min && !STOP.has(w)) || [];
const monthOf = (e) => e.start.slice(0, 7);

/** Tag rows grouped per event record: Map(recordId -> rows[]), by the monthly ids in record.members. */
export function rowsByRecord(events, rows) {
  const owner = new Map();
  for (const e of events) for (const m of e.members || []) owner.set(m, e.id);
  const out = new Map(events.map((e) => [e.id, []]));
  for (const r of rows) { const id = owner.get(r.event); if (id) out.get(id).push(r); }
  return out;
}

const entityNames = (r) => [...new Set((r.entities_typed || (r.entities || []).map((name) => ({ name }))).map((x) => x.name).filter(Boolean))];

export function archiveFrequentEntities(rows, top = ARCHIVE_TOP) {
  const n = new Map();
  for (const r of rows) for (const e of entityNames(r)) n.set(e, (n.get(e) || 0) + 1);
  return [...n].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).slice(0, top).map(([name]) => name);
}

/** Per-event profile: entity counts, name/alias tokens, themes. */
function profile(e, rows) {
  const ent = new Map(), th = new Map();
  for (const r of rows) {
    for (const x of entityNames(r)) ent.set(x, (ent.get(x) || 0) + 1);
    for (const t of r.themes || []) th.set(t, (th.get(t) || 0) + 1);
  }
  const top = (m, n) => [...m].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).slice(0, n);
  return {
    n: rows.length, ent, topEntities: top(ent, 6).map(([k]) => k),
    topThemes: top(th, 4).map(([k]) => k).concat(top(th, 4).length ? [] : e.themes || []),
    nameTokens: new Set(words(e.name, 3)),
    tokens: new Set([e.name, ...(e.aliases || [])].flatMap((s) => words(s, 5))),
  };
}

/**
 * events: reader output (id, name, aliases, start, end, items, members, themes, description).
 * rows: current.jsonl rows. Returns {pairs, profiles, stats}. pairs: {a, b (records, a earlier), signals, score, gap_days, ...}.
 */
export function buildCandidates({ events, rows, minItems = 2, maxPartners = MAX_PARTNERS, spanDays = SPAN_DAYS, archiveTop = ARCHIVE_TOP }) {
  const by = rowsByRecord(events, rows);
  const frequent = archiveFrequentEntities(rows, archiveTop);
  const frequentSet = new Set(frequent);
  const frequentTokens = new Set(frequent.flatMap((n) => words(n, 1)));
  const live = events.filter((e) => !e.merged_into && e.items >= minItems && by.get(e.id).length >= Math.max(1, minItems));
  const prof = new Map(live.map((e) => [e.id, profile(e, by.get(e.id))]));
  const df = new Map();
  for (const e of live) for (const t of new Set([...words(e.name, 5), ...(e.aliases || []).flatMap((s) => words(s, 5))])) df.set(t, (df.get(t) || 0) + 1);
  const tokenOk = (t) => !frequentTokens.has(t) && (df.get(t) || 0) <= TOKEN_DF_CAP;
  const ordered = [...live].sort((x, y) => x.start.localeCompare(y.start) || x.end.localeCompare(y.end) || x.id.localeCompare(y.id));
  const stats = { events_considered: ordered.length, archive_frequent_entities: frequent, pairs_in_window: 0, by_signal: Object.fromEntries(SIGNALS.map((s) => [s, 0])), candidate_pairs: 0, same_month_dropped: 0, kept_pairs: 0, kept_by_signal: Object.fromEntries(SIGNALS.map((s) => [s, 0])), dropped_by_partner_cap: 0 };
  const all = [];
  for (let i = 0; i < ordered.length; i++) {
    const a = ordered[i], pa = prof.get(a.id);
    for (let j = i + 1; j < ordered.length; j++) {
      const b = ordered[j];
      const gap = gapDays(a, b);
      if (gap > spanDays) continue;
      stats.pairs_in_window++;
      const pb = prof.get(b.id);
      const signals = [];
      const sharedEnt = [];
      for (const [name, ca] of pa.ent) {
        const cb = pb.ent.get(name);
        if (!cb || frequentSet.has(name)) continue;
        if (ca >= 2 && cb >= 2 && ca / pa.n >= ENTITY_SHARE && cb / pb.n >= ENTITY_SHARE) sharedEnt.push({ name, share: Math.min(ca / pa.n, cb / pb.n) });
      }
      if (sharedEnt.length) signals.push('entity');
      const sharedTok = [...pa.tokens].filter((t) => pb.tokens.has(t) && tokenOk(t));
      if (sharedTok.length) signals.push('alias-token');
      const nameShared = [...pa.nameTokens].filter((t) => pb.nameTokens.has(t));
      const dice = pa.nameTokens.size + pb.nameTokens.size ? (2 * nameShared.length) / (pa.nameTokens.size + pb.nameTokens.size) : 0;
      if (nameShared.length >= 2 || (nameShared.length >= 1 && dice >= 0.6)) signals.push('name-similarity');
      if (!signals.length) continue;
      if (monthOf(a) === monthOf(b) && !(sharedEnt.length >= 3 || (signals.includes('entity') && signals.length >= 2 && sharedEnt.length >= 2))) { stats.same_month_dropped++; continue; }
      for (const s of signals) stats.by_signal[s]++;
      stats.candidate_pairs++;
      const score = sharedEnt.reduce((n, x) => n + 2 + 2 * x.share, 0) + sharedTok.length * 1.5 + dice * 2 + (gap === 0 ? 1 : 1 - gap / spanDays);
      all.push({ a, b, signals, score: +score.toFixed(3), gap_days: gap, shared_entities: sharedEnt.map((x) => x.name), shared_tokens: sharedTok });
    }
  }
  // keep the best pairs first, at most maxPartners per event
  const count = new Map();
  const kept = [];
  for (const p of [...all].sort((x, y) => y.score - x.score || x.a.id.localeCompare(y.a.id) || x.b.id.localeCompare(y.b.id))) {
    if ((count.get(p.a.id) || 0) >= maxPartners || (count.get(p.b.id) || 0) >= maxPartners) { stats.dropped_by_partner_cap++; continue; }
    count.set(p.a.id, (count.get(p.a.id) || 0) + 1); count.set(p.b.id, (count.get(p.b.id) || 0) + 1);
    kept.push(p);
    for (const s of p.signals) stats.kept_by_signal[s]++;
  }
  stats.kept_pairs = kept.length;
  return { pairs: kept, profiles: prof, rows: by, stats };
}

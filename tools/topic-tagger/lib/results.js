// Validation of model output for one batch and conversion to tags.jsonl records.
import { idMap } from './prompts.js';

export const MAX_TOPICS = 4;
export const MAX_NOTE_WORDS = 15;

const clean = (s) => String(s).replace(/\s+/g, ' ').trim();

function trimNote(note) {
  const words = clean(note || '').split(' ').filter(Boolean);
  return words.slice(0, MAX_NOTE_WORDS).join(' ');
}

// parsed: model JSON {results:[...]}. vocabulary: null (discovery) or {topics:[{topic}]}.
// Returns {records, problems}; items the model skipped or garbled are in `missing`.
export function validateBatch(parsed, batch, { vocabulary = null } = {}) {
  const byId = idMap(batch);
  const problems = [];
  const records = new Map();
  const vocab = vocabulary ? new Set(vocabulary.topics.map((t) => t.topic)) : null;
  const rows = parsed && Array.isArray(parsed.results) ? parsed.results : null;
  if (!rows) return { records: [], missing: batch.items.map((i) => i.key), problems: ['no results array'] };
  for (const r of rows) {
    const item = byId[r && r.id];
    if (!item) { problems.push(`unknown id ${r && r.id}`); continue; }
    if (records.has(item.key)) { problems.push(`duplicate id ${r.id}`); continue; }
    if (typeof r.ai_related !== 'boolean') { problems.push(`bad ai_related for ${r.id}`); continue; }
    let topics = [];
    if (r.ai_related) {
      const seen = new Set();
      let news = 0;
      for (const raw of Array.isArray(r.topics) ? r.topics : []) {
        let t = clean(raw);
        if (!t) continue;
        if (vocab) {
          if (/^new:/i.test(t)) {
            t = 'new: ' + clean(t.replace(/^new:/i, ''));
            if (t === 'new:' || news >= 1) { problems.push(`dropped extra new topic for ${r.id}`); continue; }
            news++;
          } else if (!vocab.has(t)) {
            problems.push(`topic outside vocabulary for ${r.id}: ${t}`);
            continue;
          }
        }
        if (seen.has(t)) continue;
        seen.add(t);
        topics.push(t);
      }
      if (topics.length > MAX_TOPICS) topics = topics.slice(0, MAX_TOPICS);
      if (topics.length === 0) { problems.push(`AI item without topics ${r.id}`); }
    }
    const rec = {
      key: item.key,
      path: item.path,
      date: item.date,
      type: item.type,
      ai_related: r.ai_related,
      topics,
    };
    const note = trimNote(r.note);
    if (note) rec.note = note;
    records.set(item.key, rec);
  }
  const missing = batch.items.filter((i) => !records.has(i.key)).map((i) => i.key);
  // an AI item left with no topics is treated as missing so it can be retried
  const okRecords = [...records.values()].filter((r) => !(r.ai_related && r.topics.length === 0));
  const bad = [...records.values()].filter((r) => r.ai_related && r.topics.length === 0).map((r) => r.key);
  return { records: okRecords, missing: [...missing, ...bad], problems };
}

// Count AI share and topic frequencies.
export function summarise(records) {
  const topicCounts = new Map();
  let ai = 0;
  for (const r of records) {
    if (r.ai_related) ai++;
    for (const t of r.topics) topicCounts.set(t, (topicCounts.get(t) || 0) + 1);
  }
  return { items: records.length, ai, topicCounts };
}

// Merge a vocabulary mapping into counts: returns topic rows with count and 3 example keys.
export function applyVocabulary(records, topics, mapping) {
  const rows = new Map(topics.map((t) => [t.topic, { topic: t.topic, definition: t.definition, count: 0, examples: [] }]));
  const unmapped = new Map();
  for (const r of records) {
    const hit = new Set();
    for (const label of r.topics) {
      const target = mapping[label] ?? (rows.has(label) ? label : null);
      if (!target || !rows.has(target)) { unmapped.set(label, (unmapped.get(label) || 0) + 1); continue; }
      hit.add(target);
    }
    for (const t of hit) {
      const row = rows.get(t);
      row.count++;
      if (row.examples.length < 3) row.examples.push(r.key);
    }
  }
  return { topics: [...rows.values()].sort((a, b) => b.count - a.count), unmapped: Object.fromEntries(unmapped) };
}

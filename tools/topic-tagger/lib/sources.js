// Context sources read from disk: the reader's Readwise tags, his own tweets for a month,
// and the MonDAI roundup for a month. Each condenser returns plain text under a char budget.
import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { clip, cut } from './text.js';

// Tags that record an action or a workflow state, not a subject.
export const SYSTEM_TAGS = new Set([
  'like', 'liked', 'favorite', 'favourite', 'important', 'shortlist', 'discard', 'bookmark',
  '👻 ai highlighted', 'h1', 'h2', 'h3', 'tweet about', 'archive', 'paper', 'roundup', 'disagree',
  'interesting hypothesis', 'my writing', 'personal account', 'funny',
]);

const squash = (s) => String(s ?? '').replace(/\s+/g, ' ').trim();

// Counts tags from "highlighted_at: ... | tags: a, b" lines, case-folded; system tags excluded.
export function countTagsInText(text, counts = new Map()) {
  for (const m of text.matchAll(/^highlighted_at:[^\n|]*\|\s*tags:\s*(.+)$/gm)) {
    for (const raw of m[1].split(',')) {
      const t = squash(raw).toLowerCase();
      if (!t || SYSTEM_TAGS.has(t)) continue;
      counts.set(t, (counts.get(t) || 0) + 1);
    }
  }
  return counts;
}

export function readwiseTagCounts(worksDir, { top = 150 } = {}) {
  const counts = new Map();
  let files = 0;
  for (const f of readdirSync(worksDir)) {
    if (!f.endsWith('.md')) continue;
    files++;
    countTagsInText(readFileSync(join(worksDir, f), 'utf8'), counts);
  }
  const all = [...counts.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
  return {
    files_scanned: files,
    distinct_tags: all.length,
    excluded_system_tags: [...SYSTEM_TAGS],
    tags: all.slice(0, top).map(([tag, count]) => ({ tag, count })),
  };
}

// ---- the reader's own tweets (writing/tweets/stream/YYYY-MM.md) ----

const TWEET_RE = /## (\S+) — tweet (\d+) · kind: (\w+)\n\n<!-- tweet id="\d+" -->\n~~~\n([\s\S]*?)\n~~~/g;

export function parseTweetStream(text) {
  return [...text.matchAll(TWEET_RE)].map((m) => ({ created: m[1], id: m[2], kind: m[3], text: m[4] }));
}

// One line per tweet: "MM-DD kind: text"; leading @handles of replies dropped; t.co / status links shortened.
// Per-tweet char cap (replies half of it) shrinks until the whole block fits maxChars.
export function condenseTweets(tweets, { maxChars = 25000, startCap = 280, minCap = 40 } = {}) {
  const clean = (s) => squash(s).replace(/^(@\w+\s+)+/, '').replace(/https?:\/\/(twitter|x)\.com\/\S+/g, '[quoted tweet]').replace(/https?:\/\/t\.co\/\S+/g, '[link]');
  for (let cap = startCap; cap >= minCap; cap -= 20) {
    const lines = tweets.map((t) => {
      const body = clean(t.text);
      const c = t.kind === 'original' ? cap : Math.max(minCap, Math.floor(cap / 2)); // replies get half
      return `${t.created.slice(5, 10)} ${t.kind === 'original' ? 'post' : t.kind}: ${clip(body, c)}`;
    });
    const text = lines.join('\n');
    if (text.length <= maxChars) return { text, perTweetCap: cap, tweets: tweets.length };
  }
  const lines = tweets.map((t) => `${t.created.slice(5, 10)}: ${cut(clean(t.text), minCap)}`);
  let text = '';
  let kept = 0;
  for (const l of lines) { if (text.length + l.length + 1 > maxChars) break; text += (text ? '\n' : '') + l; kept++; }
  return { text, perTweetCap: minCap, tweets: kept, dropped: tweets.length - kept };
}

// All the reader's tweets across stream files: id -> {text, created}, for parent/quote lookups.
export function loadOwnTweets(streamDir) {
  const out = new Map();
  if (!existsSync(streamDir)) return out;
  for (const f of readdirSync(streamDir)) {
    if (!f.endsWith('.md')) continue;
    for (const t of parseTweetStream(readFileSync(join(streamDir, f), 'utf8'))) out.set(t.id, t);
  }
  return out;
}

// ---- MonDAI roundup (ai-news-tracking roundup JSON) ----

const MONTHS = ['january', 'february', 'march', 'april', 'may', 'june', 'july', 'august', 'september', 'october', 'november', 'december'];

// month "2026-09" -> candidate roundup file names, most specific first.
export function roundupCandidates(month) {
  const [y, m] = month.split('-');
  const name = MONTHS[Number(m) - 1];
  return [`mondai-${name.slice(0, 3)}-${y}.json`, `mondai-${name}-${y}.json`];
}

export function findRoundup(roundupDir, month) {
  for (const c of roundupCandidates(month)) {
    const p = join(roundupDir, c);
    if (existsSync(p)) return p;
  }
  return null;
}

const stripMd = (s) => squash(String(s ?? '').replace(/\[([^\]]*)\]\([^)]*\)/g, '$1').replace(/[*_`#>]/g, ''));

// Title page + intro + one line per library item ("date [section] title: summary...").
export function condenseRoundup(json, { maxChars = 24000, startCap = 260, minCap = 80 } = {}) {
  const p = json.presentation || {};
  const sectionOf = new Map();
  for (const s of json.sections || []) for (const id of s.items || []) sectionOf.set(id, s.title);
  const items = [...(json.content_library || [])].sort((a, b) => String(a.date).localeCompare(String(b.date)));
  const head = `${p.title || 'MonDAI roundup'} (${p.title_page_subtitle || ''})\n${stripMd(json.landing_intro || '')}`;
  for (let cap = startCap; cap >= minCap; cap -= 20) {
    const lines = items.map((it) => {
      const sum = stripMd(it.summary);
      return `${it.date || '?'} [${sectionOf.get(it.external_id) || it.content_type || ''}] ${stripMd(it.title)}: ${clip(sum, cap)}`;
    });
    const text = `${head}\n${lines.join('\n')}`;
    if (text.length <= maxChars || cap - 20 < minCap) return { text: cut(text, maxChars), perItemCap: cap, items: items.length };
  }
  return { text: cut(head, maxChars), perItemCap: 0, items: 0 };
}

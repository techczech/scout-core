// Context enrichment for one month's items. Joins, all local and read-only:
//  - reply parent and quoted tweet: ids from the X import snapshot (likes-saved.jsonl), texts from
//    archive works (tweet id = source_id) or the reader's own tweets; status links in the body count as quotes;
//  - linked article: expanded links in the body ("🔗 url") and the snapshot's article_urls, joined to
//    archive works by normalised URL; text from readings/fulltext/<same file> or the work's highlights;
//  - thread hint: other items by the same author on the same day.
import { existsSync, readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { parseWork } from './works.js';
import { clip as clipText } from './text.js';

export const PARENT_CHARS = 500;
export const ARTICLE_CHARS = 400;
export const THREAD_MAX = 6;

const squash = (s) => String(s ?? '').replace(/\s+/g, ' ').trim();
const clip = (s, n) => clipText(squash(s), n);

const DROP_PARAMS = /^(utm_.*|ref|ref_src|s|t|r|si|triedredirect|publication_id|post_id|isfreemail|source|mc_cid|mc_eid|fbclid|gclid|cmpid|smid)$/i;

export function normaliseUrl(raw) {
  let u;
  try { u = new URL(String(raw).trim()); } catch { return null; }
  const host = u.hostname.toLowerCase().replace(/^(www\.|m\.|mobile\.)/, '');
  for (const k of [...u.searchParams.keys()]) if (DROP_PARAMS.test(k)) u.searchParams.delete(k);
  const q = u.searchParams.toString();
  const path = u.pathname.replace(/\/+$/, '');
  return `${host === 'twitter.com' ? 'x.com' : host}${path}${q ? '?' + q : ''}`;
}

export function tweetIdFromUrl(raw) {
  const m = String(raw).match(/^https?:\/\/(?:www\.|mobile\.)?(?:x|twitter)\.com\/[A-Za-z0-9_]+\/status(?:es)?\/(\d+)/);
  return m ? m[1] : null;
}

const workText = (w) => w.highlights.map((h) => h.text).filter(Boolean).join(' ');
const stripMedia = (s) => String(s).replace(/!\[[^\]]*\]\([^)]*\)/g, '').replace(/^🔗.*$/gm, '');

// One pass over all works: tweet id -> work summary; normalised url -> work summary.
export function buildArchiveIndex(archiveRoot, { relBase = 'readings/works' } = {}) {
  const worksDir = join(archiveRoot, relBase);
  const byTweetId = new Map();
  const byUrl = new Map();
  for (const f of readdirSync(worksDir)) {
    if (!f.endsWith('.md')) continue;
    const w = parseWork(readFileSync(join(worksDir, f), 'utf8'), `${relBase}/${f}`);
    const entry = { path: w.path, file: f, key: w.key, author: w.author, title: w.title, type: w.type, url: w.url, text: workText(w) };
    const id = w.key.startsWith('x:') ? w.key.slice(2) : tweetIdFromUrl(w.url);
    if (id) byTweetId.set(id, entry);
    const n = w.url && normaliseUrl(w.url);
    if (n && !byUrl.has(n)) byUrl.set(n, entry);
  }
  return { byTweetId, byUrl };
}

// tweet_id -> {reply_to_id, parent_text, parent_handle, quoted_tweet_id, quoted_text, quoted_handle, article_urls}
export function loadXSnapshot(importsDir) {
  const out = new Map();
  if (!existsSync(importsDir)) return out;
  for (const d of readdirSync(importsDir).filter((n) => n.startsWith('x-api-')).sort()) {
    const p = join(importsDir, d, 'likes-saved.jsonl');
    if (!existsSync(p)) continue;
    for (const line of readFileSync(p, 'utf8').split('\n')) {
      if (!line.trim()) continue;
      let r;
      try { r = JSON.parse(line); } catch { continue; }
      if (r.tweet_id) out.set(String(r.tweet_id), r); // later snapshots win
    }
  }
  return out;
}

// Links written into the highlight text: "🔗 url" lines and bare URLs (t.co excluded: unresolvable offline).
export function bodyLinks(work) {
  const text = work.highlights.map((h) => h.text).join('\n');
  const urls = new Set();
  for (const m of text.matchAll(/https?:\/\/[^\s)\]>"']+/g)) {
    const u = m[0].replace(/[.,;:!?…]+$/, '');
    if (/^https?:\/\/(t\.co|pbs\.twimg\.com)\//.test(u)) continue;
    urls.add(u);
  }
  return [...urls];
}

// Look up a tweet text by id: archive work first, then the reader's own tweets.
function tweetById(id, archive, ownTweets) {
  const w = archive.byTweetId.get(id);
  if (w) return { id, handle: w.author, text: stripMedia(w.text), source: 'archive', path: w.path };
  const o = ownTweets && ownTweets.get(id);
  if (o) return { id, handle: 'techczech (the reader)', text: o.text, source: 'own-tweets' };
  return null;
}

function articleFor(entry, archiveRoot) {
  let body = '';
  const ft = archiveRoot && join(archiveRoot, 'readings/fulltext', entry.file);
  if (ft && existsSync(ft)) body = readFileSync(ft, 'utf8').replace(/^---\n[\s\S]*?\n---\n?/, '');
  if (!squash(body)) body = entry.text;
  return { title: entry.title, author: entry.author, path: entry.path, text: clip(body.replace(/\[([^\]]*)\]\([^)]*\)/g, '$1'), ARTICLE_CHARS) };
}

/**
 * items: selected works (parseWork shape + key). Returns {contexts: Map(key -> context), counts}.
 * context: {parent?, parent_missing?, quoted?, quoted_missing?, linked[], links_missing, thread[]}
 */
export function enrichItems(items, { archive, snapshot = new Map(), ownTweets = new Map(), archiveRoot = null }) {
  const counts = {
    items: items.length, tweets: 0,
    replies: 0, reply_parent_found: 0,
    quotes: 0, quote_text_found: 0,
    with_links: 0, link_article_found: 0,
    thread_grouped: 0,
    any_enrichment: 0,
  };
  const contexts = new Map();
  const groups = new Map();
  for (const it of items) {
    const g = `${it.author.toLowerCase()}|${it.date}`;
    if (!groups.has(g)) groups.set(g, []);
    groups.get(g).push(it.key);
  }
  for (const it of items) {
    const ctx = { linked: [], links_missing: 0, thread: [] };
    const tweetId = it.key.startsWith('x:') ? it.key.slice(2) : null;
    const isTweet = it.type === 'tweet';
    if (isTweet) counts.tweets++;
    const snap = tweetId ? snapshot.get(tweetId) : null;
    const urls = new Set(bodyLinks(it));
    for (const u of (snap && snap.article_urls) || []) urls.add(u);

    // reply parent
    if (snap && snap.reply_to_id) {
      counts.replies++;
      const p = snap.parent_text ? { id: String(snap.reply_to_id), handle: snap.parent_handle || '', text: snap.parent_text, source: 'snapshot' }
        : tweetById(String(snap.reply_to_id), archive, ownTweets);
      if (p) { ctx.parent = { ...p, text: clip(p.text, PARENT_CHARS) }; counts.reply_parent_found++; } else ctx.parent_missing = String(snap.reply_to_id);
    }
    // quoted tweet: snapshot id, else the first status link in the body that is not the item itself
    let qid = snap && snap.quoted_tweet_id ? String(snap.quoted_tweet_id) : null;
    const statusLinks = [...urls].map(tweetIdFromUrl).filter((id) => id && id !== tweetId);
    if (!qid && statusLinks.length) qid = statusLinks[0];
    if (qid) {
      counts.quotes++;
      const q = snap && snap.quoted_text ? { id: qid, handle: snap.quoted_handle || '', text: snap.quoted_text, source: 'snapshot' }
        : tweetById(qid, archive, ownTweets);
      if (q) { ctx.quoted = { ...q, text: clip(q.text, PARENT_CHARS) }; counts.quote_text_found++; } else ctx.quoted_missing = qid;
    }
    // linked articles (non-status links)
    const articleUrls = [...urls].filter((u) => !tweetIdFromUrl(u));
    if (articleUrls.length) counts.with_links++;
    for (const u of articleUrls) {
      const n = normaliseUrl(u);
      const hit = n && archive.byUrl.get(n);
      if (hit && hit.key !== it.key) ctx.linked.push({ url: u, ...articleFor(hit, archiveRoot) });
      else ctx.links_missing++;
    }
    ctx.linked = ctx.linked.slice(0, 2);
    if (ctx.linked.length) counts.link_article_found++;
    // thread hint
    const g = groups.get(`${it.author.toLowerCase()}|${it.date}`).filter((k) => k !== it.key);
    if (g.length) { ctx.thread = g.slice(0, THREAD_MAX); counts.thread_grouped++; }
    if (ctx.parent || ctx.quoted || ctx.linked.length || ctx.thread.length) counts.any_enrichment++;
    contexts.set(it.key, ctx);
  }
  return { contexts, counts };
}

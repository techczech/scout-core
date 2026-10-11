// Deterministic post-filter for entities: an item's own author, account handle, publisher or site is metadata
// the archive already holds, so an entity equal to one of them is dropped unless the item's text mentions that
// name beyond the byline (the author is then the subject: a tweet about Terence Tao by someone else, or an
// article that discusses its own author by name).

// Lower-case, diacritics removed, every non-alphanumeric run -> one space.
export const fold = (s) => String(s ?? '').normalize('NFKD').replace(/[̀-ͯ]/g, '').toLowerCase().replace(/[^\p{L}\p{N}]+/gu, ' ').trim();
const compact = (s) => fold(s).replace(/ /g, '');

// Site name stem of a url: "www.astralcodexten.com" -> "astralcodexten", "x.substack.com" -> "x".
function siteStem(url) {
  let host;
  try { host = new URL(url).hostname.toLowerCase().replace(/^(www|m|mobile)\./, ''); } catch { return ''; }
  const parts = host.split('.');
  if (parts.length >= 3 && ['substack', 'medium', 'wordpress', 'blogspot', 'github'].includes(parts[parts.length - 2])) return parts[0];
  return parts.length >= 2 ? parts[parts.length - 2] : parts[0];
}

const handleOf = (url) => { const m = String(url || '').match(/^https?:\/\/(?:www\.|mobile\.)?(?:x|twitter)\.com\/([A-Za-z0-9_]+)\/status/); return m ? m[1] : ''; };

// Names that are the item's own metadata (compact folded forms: "Vtrivedy10", "terence tao" and "terencetao" alike).
export function ownNames(item) {
  const raw = [item.author, item.authorName, handleOf(item.url), siteStem(item.url)];
  return [...new Set(raw.map(compact).filter((n) => n.length >= 3))];
}

// Does the text mention `name` other than in a "by <name>" byline?
export function mentionedInText(name, text) {
  const n = fold(name);
  if (!n) return false;
  const t = ` ${fold(text)} `;
  const withoutByline = t.split(` by ${n} `).join(' ');
  return withoutByline.includes(` ${n} `);
}

/**
 * entities: [{name, type}]; item: {author, authorName, url, highlights:[{text}]}.
 * Returns {kept, dropped:[name]}.
 */
export function dropOwnEntities(entities, item) {
  const own = new Set(ownNames(item));
  const text = (item.highlights || []).map((h) => h.text).join(' ');
  const kept = [], dropped = [];
  for (const e of entities) {
    if (own.has(compact(e.name)) && !mentionedInText(e.name, text)) dropped.push(e.name); else kept.push(e);
  }
  return { kept, dropped };
}

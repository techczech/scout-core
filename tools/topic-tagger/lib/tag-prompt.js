// v2 tagging prompt: cached system blocks (instructions, reader profile, themes, month briefing,
// the reader's tweets, the MonDAI roundup, the month index) + a user message of ~20 items with context.
import { PROMPT_VERSION } from './prompts.js';
import { buildMonthIndex } from './month-index.js';
import { cut, wellFormed } from './text.js';

export { PROMPT_VERSION };
export const PROMPT_CAP = 100000; // Haiku 5.5 doubles its price above 100K prompt tokens
export const ITEM_TEXT_CHARS = 2500;
export const MAX_THEMES_PER_ITEM = 3;
export const MAX_ENTITIES = 8;
export const MAX_RELATED = 3;
export const ENTITY_TYPES = ['person', 'org', 'model', 'product', 'work', 'event', 'place', 'other'];
export const RELATIONS = ['reply_or_quote', 'reacts_to', 'same_story', 'continues_thread'];
// Conservative local estimate: measured Haiku 5.5 ratios on this material are 2.2-3.2 chars/token.
export const promptEstimate = (s) => Math.ceil(String(s).length / 2.2);

export const TAG_INSTRUCTIONS = `You tag items from the reading archive of one reader, Dominik Lukeš (profile below): tweets he liked and articles he highlighted. He uses the tags to research how debates, practices and reactions developed over time. Your training data ends before the month being tagged, so rely on the month briefing, the names list and the month index below for what happened and what things were called; do not assume you know what a product or model name refers to.

For each item return:
- "ai_related": true if the item is about AI or machine learning in any way (models, products, coding tools, research, AI in education, work, science, society, policy, safety, commentary), else false.
- "themes": 0 to 3 names from the themes list, copied exactly. Pick by what the item is about, using each theme's include/exclude notes, not by keyword. An AI item gets at least one theme. A non-AI item gets a theme only if one plainly fits.
- "entities": 0 to 8 named things MENTIONED IN the item's own content (its text, and the linked article or quoted tweet it shows): people, organisations, models, products, works (books, papers, articles, reports), events, places. Each is {"name", "type"} with type one of person, org, model, product, work, event, place, other. Use the full name from the names list ("GPT-6 Astra", not "Astra"; "Claude Opus 5.5", not "Opus"). Never list the item's own author, account handle, publisher or site: the archive already holds those as metadata, and the reader, Dominik Lukeš, is never the author of an archive item. List the author only when the text itself discusses that person (a tweet by someone else about Terence Tao lists Terence Tao; Tao's own blog post does not list him for writing it). No generic concepts (those are themes). Keep the same spelling every time. An item that names nothing gets an empty list.
- "event": the id (E01, E02, ...) of the briefing event the item is about or reacts to, or "" if none. A reply or quote that reacts to an event belongs to it even if it does not name it: use the reply/quote context, the thread hint and the month index to see what conversation the item is part of.
- "related": 0 to 3 OTHER items from the month index that this item directly connects to, each {"id", "relation"} with the index id (m0042) and relation one of reply_or_quote (this item replies to or quotes that one), reacts_to (it responds to what that one says), same_story (both report or discuss the same specific news item, post or paper), continues_thread (the same author's next post in one thread). Only when the connection is evident from the texts or the index, never merely because both share a theme. Most items have none.
- "confidence": "high" when themes and event are clear; "medium" when a judgement call; "low" when the item is too short or ambiguous, or its context is missing (a reply whose parent is unknown, a link-only tweet whose article is not available, an image-only post).

How to read an item: "text" is what he highlighted or liked. Blocks in square brackets come from elsewhere in the archive: [replying to], [quoting], [linked article] give the context the item responds to, so tag what the item says about it; [same author same day] lists other items (month index ids) that may belong to the same thread. Return one result per item, using the item id exactly as given. Write tags in English.`;

export const TAG_SCHEMA = {
  type: 'object',
  properties: {
    results: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          id: { type: 'string' },
          ai_related: { type: 'boolean' },
          themes: { type: 'array', items: { type: 'string' } },
          entities: { type: 'array', items: { type: 'object', properties: { name: { type: 'string' }, type: { type: 'string', enum: ENTITY_TYPES } }, required: ['name', 'type'], additionalProperties: false } },
          event: { type: 'string' },
          related: { type: 'array', items: { type: 'object', properties: { id: { type: 'string' }, relation: { type: 'string', enum: RELATIONS } }, required: ['id', 'relation'], additionalProperties: false } },
          confidence: { type: 'string', enum: ['low', 'medium', 'high'] },
        },
        required: ['id', 'ai_related', 'themes', 'entities', 'event', 'related', 'confidence'],
        additionalProperties: false,
      },
    },
  },
  required: ['results'],
  additionalProperties: false,
};

const squash = (s) => String(s ?? '').replace(/\s+/g, ' ').trim();

// One item for the user message. ctx is the enrichment context (may be undefined); sidOf maps key -> sid.
export function renderItem(it, ctx, sidOf = new Map(), { textChars = ITEM_TEXT_CHARS } = {}) {
  const raw = (it.highlights || []).map((h) => h.text).filter(Boolean).join('\n---\n')
    .replace(/!\[[^\]]*\]\([^)]*\)/g, '[image]');
  const text = raw.length > textChars ? cut(raw, textChars) + ' […]' : raw;
  const lines = [`<item id="${it.sid}">`, `date: ${it.date} · type: ${it.type} · author: ${squash(it.author)}`];
  if (it.type !== 'tweet' && it.title) lines.push(`title: ${squash(it.title)}`);
  lines.push('text:', text);
  if (ctx) {
    if (ctx.parent) lines.push(`[replying to @${ctx.parent.handle}]: ${ctx.parent.text}`);
    else if (ctx.parent_missing) lines.push('[replying to: parent tweet not in the archive]');
    if (ctx.quoted) lines.push(`[quoting @${ctx.quoted.handle}]: ${ctx.quoted.text}`);
    else if (ctx.quoted_missing) lines.push('[quoting: quoted tweet not in the archive]');
    for (const l of ctx.linked || []) lines.push(`[linked article: ${squash(l.title)}${l.author ? ` — ${squash(l.author)}` : ''}]: ${l.text}`);
    const th = (ctx.thread || []).map((k) => sidOf.get(k)).filter(Boolean);
    if (th.length) lines.push(`[same author same day]: ${th.join(', ')}`);
  }
  lines.push('</item>');
  return lines.join('\n');
}

export const userMessageV2 = (batch) => batch.items.map((i) => i.text).join('\n\n');

// parts: {instructions, profile, themes, briefing, tweets, roundup, index}. Order = most stable first.
export const BLOCK_ORDER = ['instructions', 'reader_profile', 'themes', 'month_briefing', 'reader_tweets', 'mondai_roundup', 'month_index'];

export function assembleSystem(parts, { ttl } = {}) {
  const blocks = BLOCK_ORDER.filter((n) => parts[n]).map((n) => ({ type: 'text', text: wellFormed(n === 'instructions' ? parts[n] : `<${n}>\n${parts[n]}\n</${n}>`) }));
  if (!blocks.length) throw new Error('empty system prompt');
  blocks[blocks.length - 1].cache_control = ttl ? { type: 'ephemeral', ttl } : { type: 'ephemeral' };
  return blocks;
}

export const systemText = (blocks) => blocks.map((b) => b.text).join('\n\n');

/**
 * Fit the system prompt plus the largest user batch under the cap by shrinking the month index.
 * count(systemBlocks, userText) -> tokens (async; the real one calls messages.countTokens).
 * Returns {system, index, sizes, systemTokens, maxUserTokens, totalTokens}.
 */
export async function fitPrompt(fixedParts, indexItems, { cap = PROMPT_CAP, userReserve = 12000, margin = 1500, count, maxGist = 100 } = {}) {
  const countFn = count || (async (sys, user) => promptEstimate(systemText(sys)) + promptEstimate(user || ''));
  const fixed = { ...fixedParts };
  const fixedTokens = await countFn(assembleSystem(fixed), '');
  let indexBudget = cap - userReserve - margin - fixedTokens;
  if (indexBudget < 2000) throw new Error(`fixed context alone is ${fixedTokens} tokens; no room for the month index under ${cap}`);
  for (let attempt = 0; attempt < 10; attempt++) {
    const index = buildMonthIndex(indexItems, { maxTokens: indexBudget, maxGist });
    const system = assembleSystem({ ...fixed, month_index: index.text });
    const systemTokens = await countFn(system, '');
    if (systemTokens + userReserve + margin <= cap) {
      const sizes = BLOCK_ORDER.filter((n) => n === 'month_index' || fixed[n]).map((n) => {
        const t = n === 'month_index' ? index.text : fixed[n];
        return { block: n, chars: t.length, est_tokens: promptEstimate(t) };
      });
      return { system, index, sizes, systemTokens, maxUserTokens: cap - margin - systemTokens };
    }
    indexBudget -= Math.ceil((systemTokens + userReserve + margin - cap) * 1.15) + 500;
    if (indexBudget < 2000) break;
  }
  throw new Error('could not fit the tagging prompt under the cap');
}

export function buildTagParams({ model, effort, system, batch, maxTokens = 16000 }) {
  return {
    model,
    max_tokens: maxTokens,
    thinking: { type: 'adaptive' },
    output_config: { effort, format: { type: 'json_schema', schema: TAG_SCHEMA } },
    system: system.map((b) => ({ ...b, text: wellFormed(b.text) })),
    messages: [{ role: 'user', content: wellFormed(userMessageV2(batch)) }],
  };
}

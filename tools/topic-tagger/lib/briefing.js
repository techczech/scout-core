// Month briefing: one model call over a condensed view of every item of the month.
// Output is JSON (events, names/aliases, debates, engagement); the markdown is rendered from it,
// so event ids in the briefing and in events.json are the same by construction.
import { clip } from './text.js';
export const BRIEFING_PROMPT_VERSION = 'briefing-v1';
export const VIEW_CHARS = 300;

const squash = (s) => String(s ?? '').replace(/\s+/g, ' ').trim();

// items carry sid. "m0042 | 2026-09-04 | tweet | Vtrivedy10 | <title if not a tweet> | first 300 chars"
export function condensedView(items, { chars = VIEW_CHARS } = {}) {
  return [...items].sort((a, b) => a.sid.localeCompare(b.sid)).map((it) => {
    const body = squash((it.highlights || []).map((h) => h.text).join(' ')
      .replace(/!\[[^\]]*\]\([^)]*\)/g, '').replace(/https?:\/\/t\.co\/\S+/g, ''));
    const t = clip(body, chars);
    const title = it.type === 'tweet' ? '' : ` | ${squash(it.title)}`;
    return `${it.sid} | ${it.date} | ${it.type} | ${squash(it.author)}${title} | ${t}`;
  }).join('\n');
}

export function briefingSystem(month) {
  return `You write a factual briefing of one month (${month}) as seen through one person's reading archive: the tweets he liked and the articles he highlighted that month. The briefing will be given to a smaller model that tags each item, and whose training data ends before this month, so it must learn from you what happened, what things were called, and what people argued about.

Rules:
- Use only the items given. Do not add facts from your own knowledge, even when you think you know them; your knowledge may be out of date. If items disagree, say so.
- Date events from the items (the earliest item that reports the event; say "around" when unsure).
- Events: releases, announcements, papers, incidents, controversies, open letters, notable posts that many items react to. Give each an id E01, E02, ... in date order, a short name, a date (YYYY-MM-DD), the aliases people used for it or for the thing it is about, a one-line description, and 2-5 item ids (m-numbers) that report or discuss it. Aim for 20-45 events; only include an event if at least two items refer to it, or one item and it is plainly major.
- Names and aliases: products, models, companies, people and coinages as people wrote them, mapped to the full name, e.g. "Astra" = "OpenAI GPT-6 Astra". Include nicknames, version shorthands and coined terms. Note what each is in a few words.
- Debates: the main arguments running through the month (5-12), each with what the sides said and when it peaked.
- Engagement: which events and debates this reader saved the most items about, with rough counts from the items.
- Overview: 3-5 sentences on the month as a whole.
- Plain English, short sentences, no evaluation of who was right.`;
}

export const BRIEFING_SCHEMA = {
  type: 'object',
  properties: {
    overview: { type: 'string' },
    events: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          id: { type: 'string' }, name: { type: 'string' }, date: { type: 'string' },
          aliases: { type: 'array', items: { type: 'string' } },
          description: { type: 'string' },
          items: { type: 'array', items: { type: 'string' } },
        },
        required: ['id', 'name', 'date', 'aliases', 'description', 'items'],
        additionalProperties: false,
      },
    },
    names: {
      type: 'array',
      items: {
        type: 'object',
        properties: { name: { type: 'string' }, aliases: { type: 'array', items: { type: 'string' } }, note: { type: 'string' } },
        required: ['name', 'aliases', 'note'],
        additionalProperties: false,
      },
    },
    debates: {
      type: 'array',
      items: {
        type: 'object',
        properties: { title: { type: 'string' }, summary: { type: 'string' }, when: { type: 'string' } },
        required: ['title', 'summary', 'when'],
        additionalProperties: false,
      },
    },
    engagement: { type: 'string' },
  },
  required: ['overview', 'events', 'names', 'debates', 'engagement'],
  additionalProperties: false,
};

// Re-number events E01.. in date order, drop unknown item ids, dedupe aliases.
export function validateBriefing(parsed, knownSids) {
  const problems = [];
  if (!parsed || !Array.isArray(parsed.events)) throw new Error('briefing has no events array');
  const known = new Set(knownSids);
  const events = parsed.events
    .filter((e) => squash(e.name))
    .map((e) => ({ ...e, date: /^\d{4}-\d{2}-\d{2}$/.test(e.date) ? e.date : String(e.date || '').slice(0, 10) }))
    .sort((a, b) => a.date.localeCompare(b.date) || a.name.localeCompare(b.name))
    .map((e, i) => {
      const items = (e.items || []).filter((s) => known.has(s));
      if (items.length < (e.items || []).length) problems.push(`event ${e.name}: dropped ${(e.items || []).length - items.length} unknown item ids`);
      return {
        id: `E${String(i + 1).padStart(2, '0')}`,
        name: squash(e.name), date: e.date,
        aliases: [...new Set((e.aliases || []).map(squash).filter(Boolean))],
        description: squash(e.description), items,
      };
    });
  const names = (parsed.names || []).filter((n) => squash(n.name)).map((n) => ({
    name: squash(n.name), aliases: [...new Set((n.aliases || []).map(squash).filter((a) => a && a !== squash(n.name)))], note: squash(n.note),
  }));
  return { overview: squash(parsed.overview), events, names, debates: parsed.debates || [], engagement: String(parsed.engagement || '').trim(), problems };
}

export function renderBriefingMarkdown(month, b, meta) {
  const ev = b.events.map((e) => `- **${e.id} · ${e.name}** (${e.date})${e.aliases.length ? ` — also: ${e.aliases.join(', ')}` : ''}. ${e.description}${e.items.length ? ` Items: ${e.items.join(', ')}.` : ''}`).join('\n');
  const names = b.names.map((n) => `- **${n.name}**${n.aliases.length ? ` = ${n.aliases.map((a) => `"${a}"`).join(', ')}` : ''}: ${n.note}`).join('\n');
  const debates = b.debates.map((d) => `- **${squash(d.title)}** (${squash(d.when)}): ${squash(d.summary)}`).join('\n');
  return `---
title: "Month briefing ${month}"
month: ${month}
status: generated
model: ${meta.model}
prompt_version: ${BRIEFING_PROMPT_VERSION}
generated: ${meta.generated}
source: "${meta.items} archive items whose latest highlight is in ${month} (condensed: date, author, title, first ${meta.view_chars_per_item || VIEW_CHARS} chars)"
---

# Month briefing: ${month}

Written by ${meta.model} from the reading archive only. Event ids match \`events.json\` and the \`event\` field in the topic layer.

## Overview

${b.overview}

## Events

${ev}

## Names and aliases

${names}

## Debates

${debates}

## What this reader engaged with most

${b.engagement}
`;
}

// Compact text for the tagging prompt (no item lists).
export function briefingForPrompt(b) {
  return [
    `Overview: ${b.overview}`,
    'Events (use these ids in "event"):',
    ...b.events.map((e) => `${e.id} ${e.date} ${e.name}${e.aliases.length ? ` [also: ${e.aliases.join('; ')}]` : ''}: ${e.description}`),
    'Names and aliases (use the full name as the entity):',
    ...b.names.map((n) => `${n.name}${n.aliases.length ? ` = ${n.aliases.join('; ')}` : ''}: ${n.note}`),
    'Debates:',
    ...b.debates.map((d) => `- ${squash(d.title)} (${squash(d.when)}): ${squash(d.summary)}`),
    `Engagement: ${squash(b.engagement)}`,
  ].join('\n');
}

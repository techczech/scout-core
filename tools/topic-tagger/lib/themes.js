// Themes draft: one call that turns the reader profile, a month briefing, the v1 free-form vocabulary
// and the reader's own Readwise tags into ~40 lasting research themes.
export const THEMES_PROMPT_VERSION = 'themes-v1';
export const MIN_THEMES = 25;
export const MAX_THEMES = 55;

const squash = (s) => String(s ?? '').replace(/\s+/g, ' ').trim();

export const THEMES_SYSTEM = `You design the fixed list of research themes for one person's reading archive (tweets he liked, articles he highlighted). Every item in the archive will be tagged with 0-3 of these themes by a smaller model, month after month, and he will use the themes to research how debates and practices developed over time.

What a theme is:
- A lasting subject he would research across years: a debate, a practice, a domain, a field's reaction, a recurring question. Examples of the right grain: "AI and mathematics", "vibe coding and AI-assisted programming", "AI in education and assessment", "anthropomorphism and how we talk about AI", "AI safety, alignment and risk", "epistemology and expertise".
- Not a product, model, company, person or one-month event. "GPT-6 Astra release", "Claude Code", "Muse", "OpenAI DevDay" are entities, recorded in a separate field; never make them themes. A theme must still make sense in two years.
- Distinct: two themes should rarely both fit for the same reason. Write include and exclude notes that settle the borderline cases between neighbouring themes.
- Cover the non-AI part of his reading too (his Readwise tags show it: epistemology, politics and culture war, history of science, linguistics and metaphor, education), with fewer, broader themes there.

Produce about 40 themes (between ${MIN_THEMES} and ${MAX_THEMES}). For each: a short name (2-7 words, sentence case), a one-sentence definition, "include" (what belongs, with concrete examples from the material), "exclude" (what goes elsewhere and to which theme). Also add short "notes" on how you drew the list. Use his own vocabulary where it is clear.`;

export const THEMES_SCHEMA = {
  type: 'object',
  properties: {
    themes: {
      type: 'array',
      items: {
        type: 'object',
        properties: { name: { type: 'string' }, definition: { type: 'string' }, include: { type: 'string' }, exclude: { type: 'string' } },
        required: ['name', 'definition', 'include', 'exclude'],
        additionalProperties: false,
      },
    },
    notes: { type: 'string' },
  },
  required: ['themes', 'notes'],
  additionalProperties: false,
};

export function themesUserMessage({ profile, briefingMd, v1Vocabulary, readwiseTags }) {
  const vocab = (v1Vocabulary.topics || []).map((t) => `${t.count}\t${t.topic}: ${t.definition}`).join('\n');
  const tags = readwiseTags.tags.map((t) => `${t.count}\t${t.tag}`).join('\n');
  return `<reader_profile>\n${profile}\n</reader_profile>\n\n<month_briefing month="2026-09">\n${briefingMd}\n</month_briefing>\n\n<v1_free_form_topics note="count, topic: definition; mixes lasting themes with one-month product names">\n${vocab}\n</v1_free_form_topics>\n\n<readwise_tags note="count, tag; tags the reader applied himself over years">\n${tags}\n</readwise_tags>`;
}

// Dedupe by case-folded name, enforce count bounds, assign ids T01...
export function validateThemes(parsed) {
  const seen = new Set();
  const themes = [];
  for (const t of (parsed && parsed.themes) || []) {
    const name = squash(t.name);
    const k = name.toLowerCase();
    if (!name || seen.has(k)) continue;
    seen.add(k);
    themes.push({ id: `T${String(themes.length + 1).padStart(2, '0')}`, name, definition: squash(t.definition), include: squash(t.include), exclude: squash(t.exclude) });
  }
  if (themes.length < MIN_THEMES || themes.length > MAX_THEMES) throw new Error(`themes draft has ${themes.length} themes; expected ${MIN_THEMES}-${MAX_THEMES}`);
  return { themes, notes: squash(parsed.notes) };
}

export function themesForPrompt(themes) {
  return themes.map((t) => `- ${t.name}: ${t.definition} Include: ${t.include} Exclude: ${t.exclude}`).join('\n');
}

// ---- final themes (all monthly briefings) ----
export const THEMES_FINAL_PROMPT_VERSION = 'themes-final-v1';

export const THEMES_FINAL_SYSTEM = `${THEMES_SYSTEM.split('Produce about 40 themes')[0]}This is the final list for the 2025-2026 part of the archive. You get the reader profile, a briefing for every month from January 2025 to October 2026, the draft list with how often each draft theme was used when tagging September 2026 (and which names the tagger tried that were not on the list), and his own Readwise tags.
- Keep what worked in the draft; merge themes that were rarely used or overlapped; add themes that the briefings show recurring across months but the draft lacks. A theme must cover material in several months, not one.
- Non-AI themes: include them only where the 2025-2026 material needs them (items here are mostly about AI). Older, non-AI parts of the archive (anthropology, history, religion) will get their own pass later, so do not force such themes now, but do not drop one the evidence supports.

Produce 40 to 50 themes. For each: a short name (2-7 words, sentence case), a one-sentence definition, "include" (what belongs, with concrete examples from the briefings), "exclude" (what goes elsewhere and to which theme). Add short "notes" on what changed from the draft and why. Use his own vocabulary where it is clear.`;

export function themesFinalUserMessage({ profile, briefings, draft, septStats, readwiseTags }) {
  const months = briefings.map(({ month, text }) => `<briefing month="${month}">\n${text}\n</briefing>`).join('\n\n');
  const d = draft.themes.map((t) => `${septStats.themeCounts[t.name] || 0}\t${t.name}: ${t.definition} Include: ${t.include} Exclude: ${t.exclude}`).join('\n');
  const off = Object.entries(septStats.offList || {}).map(([n, c]) => `${c}\t${n}`).join('\n') || '(none)';
  const tags = readwiseTags.tags.map((t) => `${t.count}\t${t.tag}`).join('\n');
  return `<reader_profile>\n${profile}\n</reader_profile>\n\n<monthly_briefings>\n${months}\n</monthly_briefings>\n\n<draft_themes note="items tagged in September 2026 (of ${septStats.items}), name: definition">\n${d}\n</draft_themes>\n\n<names_tried_off_list note="count, name the tagger proposed that was not a theme">\n${off}\n</names_tried_off_list>\n\n<readwise_tags note="count, tag">\n${tags}\n</readwise_tags>`;
}

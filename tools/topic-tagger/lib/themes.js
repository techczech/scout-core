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

// ---- extension of the final list to older months (append-only) ----
export const THEMES_EXTEND_PROMPT_VERSION = 'themes-extend-v1';
export const MIN_ADDED = 8;
export const MAX_ADDED = 30;

export const THEMES_EXTEND_SYSTEM = `${THEMES_SYSTEM.split('Produce about 40 themes')[0]}This is an extension of an existing list. The {N} themes already on the list were made for 2025-2026 (mostly AI) and have been used to tag thousands of items; they cannot be changed, renamed or removed. You add themes so that the January 2022 to December 2024 part of the archive can be tagged too. ALL items in that range will be tagged, AI and non-AI. Before 2023 most items are not about AI at all: articles and books highlighted on their own terms (anthropology, history, religion, philosophy, education, linguistics, metaphor, economics, politics, science and technology, writing and so on), plus a growing share about machine learning and language models from late 2022.

You get the reader profile, a briefing for each month (or quarter, where a month had too few items) of 2022-2024, the existing list with definitions and include/exclude notes, his Readwise tags (over the whole archive and over the 2022-2024 items alone).
- Read the existing list first. Add a theme only where the 2022-2024 material has a recurring subject that no existing theme covers well, or where an existing theme is too broad for the volume of material (for example, one "Culture, anthropology and religion" theme may need finer neighbours so that anthropology, religion and kinship can be separated). A new theme must cover material in several months, not one.
- New themes must not duplicate an existing one. Write include and exclude notes that say where the borderline cases go, naming existing themes exactly where they are the neighbour.
- Cover the early AI period too if the existing AI themes do not fit it (for example machine learning before language models, the arrival of ChatGPT in education, early image generators), but do not recreate 2025-2026 themes under new names.
- Products, models, companies, people and one-month events are entities, never themes.

Return between ${MIN_ADDED} and ${MAX_ADDED} new themes only (not the existing ones). For each: a short name (2-7 words, sentence case), a one-sentence definition, "include" (what belongs, with concrete examples from the briefings), "exclude" (what goes elsewhere and to which theme). Add short "notes" on which gaps you found and why each theme is needed. Use his own vocabulary where it is clear.`;

export const themesExtendSystem = (n) => THEMES_EXTEND_SYSTEM.replace('{N}', String(n));

export function themesExtendUserMessage({ profile, briefings, existing, readwiseTags, rangeTags }) {
  const months = briefings.map(({ month, text }) => `<briefing period="${month}">\n${text}\n</briefing>`).join('\n\n');
  const ex = existing.map((t) => `- ${t.name}: ${t.definition} Include: ${t.include} Exclude: ${t.exclude}`).join('\n');
  const tags = readwiseTags.tags.map((t) => `${t.count}\t${t.tag}`).join('\n');
  const rtags = rangeTags.map((t) => `${t.count}\t${t.tag}`).join('\n');
  return `<reader_profile>\n${profile}\n</reader_profile>\n\n<existing_themes note="fixed; do not repeat these">\n${ex}\n</existing_themes>\n\n<briefings_2022_2024>\n${months}\n</briefings_2022_2024>\n\n<readwise_tags_whole_archive note="count, tag">\n${tags}\n</readwise_tags_whole_archive>\n\n<readwise_tags_2022_2024_items note="count, tag; only items highlighted in 2022-2024">\n${rtags}\n</readwise_tags_2022_2024_items>`;
}

/**
 * Append-only guard. existingDoc = the stored themes file; parsed = model output {themes, notes}.
 * Returns the new document's theme list: every existing theme untouched (same object content, same order),
 * then the added ones (ids continue, added_in set). Throws if the existing part would change or counts are out of bounds;
 * added themes whose name case-folds to an existing or earlier added name are dropped (reported in `dropped`).
 */
export function extendThemes(existingThemes, parsed, { addedIn, min = MIN_ADDED, max = MAX_ADDED } = {}) {
  const seen = new Set(existingThemes.map((t) => t.name.toLowerCase()));
  const added = [];
  const dropped = [];
  for (const t of (parsed && parsed.themes) || []) {
    const name = squash(t.name);
    const k = name.toLowerCase();
    if (!name || seen.has(k)) { dropped.push(name || '(empty)'); continue; }
    seen.add(k);
    added.push({ id: `T${String(existingThemes.length + added.length + 1).padStart(2, '0')}`, name, definition: squash(t.definition), include: squash(t.include), exclude: squash(t.exclude), added_in: addedIn });
  }
  if (added.length < min || added.length > max) throw new Error(`theme extension has ${added.length} new themes; expected ${min}-${max}`);
  const themes = [...existingThemes, ...added];
  assertAppendOnly(existingThemes, themes);
  return { themes, added, dropped, notes: squash(parsed.notes) };
}

// The first existingThemes.length entries of `next` must be byte-identical (as JSON) to `existingThemes`.
export function assertAppendOnly(existingThemes, next) {
  if (next.length < existingThemes.length) throw new Error('theme list shrank; the list is append-only');
  existingThemes.forEach((t, i) => {
    if (JSON.stringify(t) !== JSON.stringify(next[i])) throw new Error(`theme ${t.name} changed; the list is append-only`);
  });
}

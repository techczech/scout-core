// Prompts, schemas and request builders. Bump PROMPT_VERSION on any wording change.
export const PROMPT_VERSION = 'topics-v1';

const COMMON = `You tag items from a personal reading archive (tweets, articles, podcasts, books) so the owner can later research how topics and opinions developed over time.

For each item decide "ai_related": true if the item is about AI or machine learning in any way: models and model releases, AI products and tools, AI coding tools, AI research, AI in education, work, science or society, AI policy and safety, AI commentary. Otherwise false.
Rules:
- Judge from the highlighted text, the title and the author. Do not invent facts.
- Non-AI items get an empty topics list.
- "note": optional, at most 15 words saying why you chose these topics. Leave it empty when obvious.
- Return one result per item, using the item's id exactly as given.`;

export const DISCOVERY_SYSTEM = `${COMMON}
- For AI items give 1 to 4 short topic labels in English, specific enough to research (examples: "AI and mathematics: proof results", "vibe coding", "Claude Code", "model release: GPT-6", "AI in higher education"). Prefer a reusable name shared by many items over a one-off name. Use the same wording every time for the same topic.`;

export function vocabularySystem(vocabulary) {
  const list = vocabulary.topics.map((t) => `- ${t.topic}: ${t.definition}`).join('\n');
  return `${COMMON}
- For AI items pick 1 to 4 topics from the vocabulary below, copying the topic name exactly.
- If no vocabulary topic fits an item, you may add at most ONE new topic for that item, written as "new: <short topic name>". Use this sparingly.

Vocabulary:
${list}`;
}

export const BATCH_SCHEMA = {
  type: 'object',
  properties: {
    results: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          id: { type: 'string' },
          ai_related: { type: 'boolean' },
          topics: { type: 'array', items: { type: 'string' } },
          note: { type: 'string' },
        },
        required: ['id', 'ai_related', 'topics', 'note'],
        additionalProperties: false,
      },
    },
  },
  required: ['results'],
  additionalProperties: false,
};

export const VOCAB_SYSTEM = `You design a controlled topic vocabulary for a personal archive of AI reading, from free-form topic labels (each with a usage count).
- Produce about 40 to 80 topics. Each has a short English name and a one-line definition.
- Keep topics specific enough to research (a named product, a named release, a recurring debate, a research area); do not collapse everything into broad buckets like "AI".
- Frequent labels may stand as topics by themselves. Cover the whole range of labels, including the rarer ones.`;

export const VOCAB_SCHEMA = {
  type: 'object',
  properties: {
    topics: {
      type: 'array',
      items: {
        type: 'object',
        properties: { topic: { type: 'string' }, definition: { type: 'string' } },
        required: ['topic', 'definition'],
        additionalProperties: false,
      },
    },
  },
  required: ['topics'],
  additionalProperties: false,
};

export const ASSIGN_SYSTEM = `You file free-form topic labels under a fixed vocabulary of topics. For every numbered label give the single best vocabulary topic, copying its name exactly. Every label is about AI, so always choose the closest topic even when the fit is loose; use an empty string only if no topic is remotely related. Copy topic names exactly, character for character.`;

export const ASSIGN_SCHEMA = {
  type: 'object',
  properties: {
    assignments: {
      type: 'array',
      items: {
        type: 'object',
        properties: { n: { type: 'integer' }, topic: { type: 'string' } },
        required: ['n', 'topic'],
        additionalProperties: false,
      },
    },
  },
  required: ['assignments'],
  additionalProperties: false,
};

export function userMessage(batch) {
  return batch.items
    .map((it, i) => `<item id="i${i + 1}">\n${it.text}\n</item>`)
    .join('\n\n');
}

// Short per-batch ids i1..iN map back to item keys by position.
export function idMap(batch) {
  return Object.fromEntries(batch.items.map((it, i) => [`i${i + 1}`, it]));
}

export function buildParams({ model, effort, system, batch, maxTokens = 16000 }) {
  return {
    model,
    max_tokens: maxTokens,
    thinking: { type: 'adaptive' },
    output_config: { effort, format: { type: 'json_schema', schema: BATCH_SCHEMA } },
    system,
    messages: [{ role: 'user', content: userMessage(batch) }],
  };
}

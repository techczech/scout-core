// Token estimate and cost from usage. Prices are USD per million tokens.
// cacheRead / cacheWrite are multipliers on the input rate (5-minute cache writes).
export const PRICES = {
  'claude-haiku-5-5': { in: 0.1, out: 0.5, inLong: 0.5, outLong: 2.5, cacheRead: 0.1, cacheWrite: 1.25 },
  'claude-sonnet-5-5': { in: 2, out: 10, inLong: 2, outLong: 10, cacheRead: 0.1, cacheWrite: 1.25 },
  'claude-opus-5-5': { in: 4, out: 20, inLong: 4, outLong: 20, cacheRead: 0.05, cacheWrite: 1.25 },
};
export const LONG_PROMPT = 100000;

// chars/3: the Haiku 5.5 tokenizer counts ~30% more tokens than chars/4 suggests.
export const estimateTokens = (s) => Math.ceil(s.length / 3);

// The long-prompt rate card is chosen by the whole prompt (uncached + cache write + cache read).
export function promptTokens(usage) {
  return (usage.input_tokens || 0) + (usage.cache_creation_input_tokens || 0) + (usage.cache_read_input_tokens || 0);
}

export function costFromUsage(model, usage, { batch = false } = {}) {
  const p = PRICES[model];
  if (!p) throw new Error(`no price for model ${model}`);
  const long = promptTokens(usage) > LONG_PROMPT;
  const rin = long ? p.inLong : p.in;
  const usd = ((usage.input_tokens || 0) * rin
    + (usage.cache_creation_input_tokens || 0) * rin * p.cacheWrite
    + (usage.cache_read_input_tokens || 0) * rin * p.cacheRead
    + (usage.output_tokens || 0) * (long ? p.outLong : p.out)) / 1e6;
  return batch ? usd / 2 : usd;
}

export function addUsage(a, b) {
  const out = {
    input_tokens: (a.input_tokens || 0) + (b.input_tokens || 0),
    output_tokens: (a.output_tokens || 0) + (b.output_tokens || 0),
  };
  const cw = (a.cache_creation_input_tokens || 0) + (b.cache_creation_input_tokens || 0);
  const cr = (a.cache_read_input_tokens || 0) + (b.cache_read_input_tokens || 0);
  if (cw) out.cache_creation_input_tokens = cw;
  if (cr) out.cache_read_input_tokens = cr;
  return out;
}

// Upper-bound estimate for a planned run: every batch's input plus an output guess per item.
export function estimateRunUsd(model, batches, { batch = false, outTokensPerItem = 120, overheadTokens = 1500 } = {}) {
  let inTok = 0, outTok = 0, usd = 0;
  for (const b of batches) {
    const i = b.estTokens + overheadTokens;
    const o = b.items.length * outTokensPerItem;
    inTok += i;
    outTok += o;
    usd += costFromUsage(model, { input_tokens: i, output_tokens: o }, { batch }); // per request: the price step is per prompt
  }
  return { inTok, outTok, usd };
}

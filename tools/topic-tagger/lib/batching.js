// Greedy packing of items into requests under an item count and a token cap.
import { estimateTokens } from './cost.js';

export const DEFAULT_MAX_ITEMS = 20;
export const DEFAULT_MAX_TOKENS = 25000; // far under the 100K price step

export function packItems(items, { maxItems = DEFAULT_MAX_ITEMS, maxTokens = DEFAULT_MAX_TOKENS } = {}) {
  const batches = [];
  let cur = null;
  for (const item of items) {
    const t = estimateTokens(item.text);
    if (!cur || cur.items.length >= maxItems || (cur.estTokens + t > maxTokens && cur.items.length > 0)) {
      cur = { items: [], estTokens: 0 };
      batches.push(cur);
    }
    cur.items.push(item);
    cur.estTokens += t;
  }
  return batches.map((b, i) => ({ ...b, id: `b${String(i + 1).padStart(4, '0')}` }));
}

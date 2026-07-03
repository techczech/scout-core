// Common English stop words excluded from keyword matching and term
// highlighting (so "the", "of", etc. don't dominate). Quoted phrases keep them,
// and an all-stopword query falls back to matching them (see buildFreeText).

export const STOPWORDS = new Set([
  "a", "an", "and", "are", "as", "at", "be", "been", "but", "by", "can",
  "could", "did", "do", "does", "for", "from", "had", "has", "have", "he",
  "her", "here", "him", "his", "how", "i", "if", "in", "into", "is", "it",
  "its", "just", "may", "me", "might", "more", "most", "my", "no", "not",
  "of", "on", "one", "or", "our", "out", "over", "she", "should", "so",
  "some", "such", "than", "that", "the", "their", "them", "then", "there",
  "these", "they", "this", "those", "to", "too", "up", "us", "was", "we",
  "were", "what", "when", "which", "while", "who", "why", "will", "with",
  "would", "you", "your",
]);

export function isStopword(term: string): boolean {
  return STOPWORDS.has(term.trim().toLowerCase());
}

/** Drop stop words from a term list, unless that would empty it. */
export function withoutStopwords(terms: string[]): string[] {
  const kept = terms.filter((t) => !isStopword(t));
  return kept.length ? kept : terms;
}

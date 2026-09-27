// Search-query grammar ported from the Raycast extension (raycast-highlights-search
// src/lib/query.ts). Parses the free-text mini-language into a structured query
// the Rust backend executes. Behaviour is intentionally identical:
//   1 word -> as-is . 2 words -> AND . 3+ words -> OR ranked by coverage
//   AND / OR / | . -exclude . "phrase" . prefix* . /regex/flags . field tokens

import type { RegexFilter, SearchMode, SortMode } from "./types";
import { isStopword } from "./stopwords";

const TYPE_SHORTCUTS: Record<string, string> = {
  art: "articles",
  article: "articles",
  articles: "articles",
  bo: "books",
  boo: "books",
  book: "books",
  books: "books",
  tw: "tweets",
  tweet: "tweets",
  tweets: "tweets",
  pdf: "pdfs",
  pdfs: "pdfs",
  pod: "podcasts",
  podcast: "podcasts",
  podcasts: "podcasts",
  sup: "supplementals",
  supplemental: "supplementals",
  supplementals: "supplementals",
};

const FILTER_TOKENS = [
  "author", "au", "title", "ti", "type", "ty", "tag", "date", "d",
  "year", "y", "after", "since", "from", "before", "until", "to",
  "i", "zo", "source", "color", "colour", "co",
  // Corpus-engine fields (scout-corpus): corpus id, language, genre.
  "in", "lang", "genre",
  ...Object.keys(TYPE_SHORTCUTS),
];

const TOKEN_RE = new RegExp(`\\b(${FILTER_TOKENS.join("|")}):("[^"]+"|\\S*)`, "gi");
const REGEX_RE = /\/((?:\\.|[^/\\])+)\/([a-z]*)/g;
const OR_THRESHOLD = 3;

export interface ParsedQuery {
  fts: string;
  has_positive: boolean;
  positive_terms: string[];
  negatives: string[];
  regexes: RegexFilter[];
  author: string | null;
  title: string | null;
  type: string | null;
  tag: string | null;
  color: string | null;
  date: string | null;
  after: string | null;
  before: string | null;
  favorite: boolean;
  zotero: boolean;
  has_image: boolean;
  /** `in:` corpus id(s), comma-separated (scout-corpus). */
  in: string | null;
  /** `lang:` language code(s), comma-separated. */
  lang: string | null;
  /** `genre:` genre(s), comma-separated. */
  genre: string | null;
  /** `source:` value as typed (readwise, x, zotero); `zotero` is also set for zotero. */
  source: string | null;
}

export interface SearchQueryPayload
  extends Omit<ParsedQuery, "date" | "in" | "lang" | "genre" | "source"> {
  /** Combinable work-type filter (OR across the list). */
  types: string[];
  source: string | null;
  sort: SortMode;
  page: number;
  page_size: number;
}

/** Combinable filter state held by the UI (the filter popover). Quick toggles
 * and types stack; time is a single recency window. */
export interface Filters {
  favorite: boolean;
  zotero: boolean;
  hasImage: boolean;
  types: string[];
  time: string; // "" | "t:30d" | "t:6m" | "t:12m"
}

export const EMPTY_FILTERS: Filters = {
  favorite: false,
  zotero: false,
  hasImage: false,
  types: [],
  time: "",
};

export function filtersActive(f: Filters): boolean {
  return f.favorite || f.zotero || f.hasImage || f.types.length > 0 || !!f.time;
}

function canonical(key: string): keyof ParsedQuery | "after" | "before" {
  switch (key.toLowerCase()) {
    case "au": return "author";
    case "ti": return "title";
    case "ty": return "type";
    case "co": case "colour": return "color";
    case "d": case "year": case "y": return "date";
    case "since": case "from": return "after";
    case "until": case "to": return "before";
    default: return key.toLowerCase() as keyof ParsedQuery;
  }
}

/** A `y:` / `date:` value as a half-open ISO range: `start` inclusive, `end`
 * exclusive. `null` when the value is not a date form. */
export function dateRange(value: string): { start?: string; end?: string } | null {
  const clean = value.trim();
  const year = clean.match(/^(\d{4})$/);
  if (year) {
    const y = Number(year[1]);
    return { start: `${year[1]}-01-01`, end: `${y + 1}-01-01` };
  }
  const span = clean.match(/^(\d{4})-(\d{4})$/);
  if (span) {
    const a = Number(span[1]);
    const b = Number(span[2]);
    if (b < a) return null;
    return { start: `${span[1]}-01-01`, end: `${b + 1}-01-01` };
  }
  const since = clean.match(/^(\d{4})-$/);
  if (since) return { start: `${since[1]}-01-01` };
  const until = clean.match(/^-(\d{4})$/);
  if (until) return { end: `${Number(until[1]) + 1}-01-01` };
  const month = clean.match(/^(\d{4})-(\d{2})$/);
  if (month) {
    const y = Number(month[1]);
    const m = Number(month[2]);
    if (m < 1 || m > 12) return null;
    const nextY = m === 12 ? y + 1 : y;
    const nextM = m === 12 ? 1 : m + 1;
    return {
      start: `${month[1]}-${month[2]}-01`,
      end: `${nextY}-${String(nextM).padStart(2, "0")}-01`,
    };
  }
  const day = clean.match(/^(\d{4})-(\d{2})-(\d{2})$/);
  if (day) {
    const date = new Date(`${clean}T00:00:00.000Z`);
    if (Number.isNaN(date.getTime())) return null;
    date.setUTCDate(date.getUTCDate() + 1);
    return { start: clean, end: date.toISOString().slice(0, 10) };
  }
  return null;
}

function escFts(term: string): string {
  return term.replace(/"/g, "");
}

interface FreeText {
  fts: string;
  hasPositive: boolean;
  positiveTerms: string[];
  negatives: string[];
}

function buildFreeText(text: string, partial: boolean): FreeText {
  const tokens = text.match(/"[^"]*"|\S+/g) || [];
  const positives: string[] = [];
  const positiveTerms: string[] = [];
  const negatives: string[] = [];
  const seq: Array<{ fts: string } | "OR" | "AND"> = [];
  let explicitOp = false;

  // Drop bare stop words from matching - unless every bare word is a stop word
  // (then keep them so the query isn't empty). Quoted phrases are never dropped.
  const bareWords = tokens.filter((t) => !t.startsWith('"') && !/^(\||and|or)$/i.test(t));
  const hasContent = bareWords.some((t) => !isStopword(t.replace(/^-/, "")));

  for (const raw of tokens) {
    const upper = raw.toUpperCase();
    if (raw === "|" || upper === "OR") { explicitOp = true; seq.push("OR"); continue; }
    if (upper === "AND") { explicitOp = true; seq.push("AND"); continue; }

    let tok = raw;
    let negate = false;
    if (tok.startsWith("-") && tok.length > 1) { negate = true; tok = tok.slice(1); }

    // Skip bare stop words (keep quoted phrases and prefix* terms).
    const isBare = !(tok.startsWith('"') && tok.endsWith('"')) && !/\*$/.test(tok);
    if (hasContent && isBare && isStopword(tok)) continue;

    let ftsTerm: string;
    let cleanTerm: string;
    if (tok.startsWith('"') && tok.endsWith('"') && tok.length >= 2) {
      cleanTerm = escFts(tok.slice(1, -1)).trim();
      if (!cleanTerm) continue;
      ftsTerm = `"${cleanTerm}"`;
    } else if (/^[\p{L}\p{N}_]+\*$/u.test(tok)) {
      ftsTerm = tok;
      cleanTerm = tok.slice(0, -1);
    } else {
      cleanTerm = escFts(tok).trim();
      if (!cleanTerm) continue;
      // Partial mode: prefix-match bare alphanumeric terms (cat -> category).
      // Whole-word mode (default): exact token via a quoted phrase.
      ftsTerm =
        partial && /^[\p{L}\p{N}_]+$/u.test(cleanTerm) ? `${cleanTerm}*` : `"${cleanTerm}"`;
    }

    if (negate) { negatives.push(cleanTerm); continue; }
    positives.push(ftsTerm);
    positiveTerms.push(cleanTerm);
    seq.push({ fts: ftsTerm });
  }

  const hasPositive = positives.length > 0;
  if (!hasPositive) return { fts: "", hasPositive, positiveTerms, negatives };

  let pos: string;
  if (explicitOp) {
    const parts: string[] = [];
    for (const item of seq) {
      if (item === "OR") parts.push("OR");
      else if (item === "AND") continue;
      else parts.push(item.fts);
    }
    const cleaned = parts.filter(
      (part, i) =>
        part !== "OR" ||
        (i > 0 && i < parts.length - 1 && parts[i - 1] !== "OR" && parts[i + 1] !== "OR")
    );
    pos = cleaned.join(" ");
  } else if (positives.length >= OR_THRESHOLD) {
    pos = positives.join(" OR ");
  } else {
    pos = positives.join(" ");
  }

  let fts = negatives.length ? `(${pos})` : pos;
  fts += negatives.map((n) => ` NOT "${escFts(n)}"`).join("");
  return { fts, hasPositive, positiveTerms, negatives };
}

export function parseSearch(raw: string, partial = false): ParsedQuery {
  const parsed: ParsedQuery = {
    fts: "", has_positive: false, positive_terms: [], negatives: [], regexes: [],
    author: null, title: null, type: null, tag: null, color: null,
    date: null, after: null, before: null, favorite: false, zotero: false, has_image: false,
    in: null, lang: null, genre: null, source: null,
  };

  let rest = raw.replace(REGEX_RE, (_m, source: string, flags: string) => {
    parsed.regexes.push({ source, flags });
    return " ";
  });

  const shortcutText: string[] = [];
  const bag = parsed as unknown as Record<string, unknown>;
  rest = rest.replace(TOKEN_RE, (_m, rawKey: string, rawValue: string) => {
    const key = rawKey.toLowerCase();
    const value = rawValue.replace(/^"|"$/g, "");
    if (key === "i") { parsed.has_image = true; return " "; }
    if (key === "zo") { parsed.zotero = true; return " "; }
    if (key === "source") {
      parsed.source = value;
      if (value.toLowerCase().includes("zotero")) parsed.zotero = true;
      return " ";
    }
    const shortcutType = TYPE_SHORTCUTS[key];
    const field = shortcutType ? "type" : canonical(key);
    bag[field] = shortcutType || value;
    if (shortcutType && value) shortcutText.push(value);
    return " ";
  });

  const freeText = [rest, ...shortcutText].join(" ").replace(/\s+/g, " ").trim();
  const free = buildFreeText(freeText, partial);
  parsed.fts = free.fts;
  parsed.has_positive = free.hasPositive;
  parsed.positive_terms = free.positiveTerms;
  parsed.negatives = free.negatives;
  return parsed;
}

export function scopeToFilters(scope: string): Partial<ParsedQuery> {
  if (!scope) return {};
  if (scope.startsWith("t:")) {
    const v = scope.slice(2);
    const now = new Date();
    if (v === "30d") now.setDate(now.getDate() - 30);
    else if (v === "6m") now.setMonth(now.getMonth() - 6);
    else if (v === "12m") now.setFullYear(now.getFullYear() - 1);
    else return {};
    return { after: now.toISOString().slice(0, 10) };
  }
  if (scope.startsWith("ty:")) return { type: scope.slice(3) };
  if (scope === "fav") return { favorite: true };
  if (scope === "zo") return { zotero: true };
  if (scope === "img") return { has_image: true };
  return {};
}

/** Build the snake_case payload the Rust `search_query` command expects.
 * UI filters (popover) and typed tokens combine: booleans are OR'd, and the
 * popover's type checkboxes merge with any `ty:` token into one `types` list. */
export function buildSearchQuery(opts: {
  raw: string;
  filters: Filters;
  source: string | null;
  color: string | null;
  sort: SortMode;
  mode: SearchMode;
  partial?: boolean;
  page: number;
  pageSize: number;
}): SearchQueryPayload {
  const parsed = parseSearch(opts.raw, opts.partial ?? false);
  const f = opts.filters;

  // The time window comes from the popover (a single recency range).
  if (f.time) Object.assign(parsed, scopeToFilters(f.time));

  // Fold a year/date token into after/before so the backend only sees a range.
  let { after, before } = parsed;
  if (parsed.date) {
    const range = dateRange(parsed.date);
    if (range?.start) after = range.start;
    if (range?.end) before = range.end;
  }

  // Merge popover type checkboxes with a `ty:` token into one OR'd list.
  const types = Array.from(
    new Set([...f.types, ...(parsed.type ? [parsed.type] : [])]),
  );

  return {
    fts: parsed.fts,
    has_positive: parsed.has_positive,
    positive_terms: parsed.positive_terms,
    negatives: parsed.negatives,
    regexes: parsed.regexes,
    author: parsed.author,
    title: parsed.title,
    type: null,
    types,
    tag: parsed.tag,
    color: opts.color ?? parsed.color,
    after,
    before,
    favorite: parsed.favorite || f.favorite,
    zotero: parsed.zotero || f.zotero,
    has_image: parsed.has_image || f.hasImage,
    source: opts.source,
    sort: opts.sort,
    page: opts.page,
    page_size: opts.pageSize,
  };
}

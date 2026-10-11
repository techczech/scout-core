// The tokenizer of the @scout/query grammar. The normative rules are in
// docs/cli-json.md ("Query grammar"); crates/scout-query/src/scan.rs is the
// same algorithm line for line. No regular expression and no built-in
// whitespace class (\s, trim) is used here, so the two cannot drift.
//
// The query is split left to right into tokens separated by WS. A construct
// is recognised ONLY at a token start (the start of the input or right after
// whitespace):
//   a. phrase: an opening quote (" “ „) up to the next closing quote (any
//      double-quote variant), optionally after one "-"; a backslash before a
//      double-quote variant or another backslash escapes it;
//   b. regex: /body/flags followed by whitespace or the end;
//   c. field: name:value with a known name; the value is quoted (as a
//      phrase) or runs to the next whitespace;
//   d. otherwise a term: everything to the next whitespace.
// Whatever follows a closed phrase or quoted value without whitespace is a
// plain term (rule d), never a construct.

/** The whitespace of the grammar: Unicode White_Space, listed. */
export const WS: readonly string[] = [
  "\u0009", "\u000A", "\u000B", "\u000C", "\u000D", " ", "\u0085", " ",
  " ", " ", " ", " ", " ", " ", " ", " ",
  " ", " ", " ", " ", " ", " ", " ", " ",
  "　",
];

/** Double-quote variants: any of them closes a phrase and may be escaped. */
export const DOUBLE_QUOTES: readonly string[] = ['"', "“", "”", "„", "‟"];

/** The quotes that open a phrase or a quoted value: " “ „ */
export const OPEN_QUOTES: readonly string[] = ['"', "“", "„"];

const WS_SET = new Set(WS);
const DQ_SET = new Set(DOUBLE_QUOTES);
const OPEN_SET = new Set(OPEN_QUOTES);

export function isWs(c: string): boolean {
  return WS_SET.has(c);
}

/** `s` without leading and trailing WS. */
export function trimWs(s: string): string {
  const chars = Array.from(s);
  let a = 0;
  let b = chars.length;
  while (a < b && isWs(chars[a])) a += 1;
  while (b > a && isWs(chars[b - 1])) b -= 1;
  return chars.slice(a, b).join("");
}

/** The WS-separated pieces of `s`. */
export function splitWs(s: string): string[] {
  const out: string[] = [];
  let cur = "";
  for (const c of Array.from(s)) {
    if (isWs(c)) {
      if (cur) out.push(cur);
      cur = "";
    } else {
      cur += c;
    }
  }
  if (cur) out.push(cur);
  return out;
}

export type Token =
  /** A quoted phrase (text unescaped, quotes removed); `negated` for -"…". */
  | { kind: "phrase"; negated: boolean; text: string }
  /** A plain term as typed, a leading "-" included. */
  | { kind: "term"; text: string }
  | { kind: "regex"; source: string; flags: string }
  /** A known field, its name lowercased; the value unquoted and unescaped. */
  | { kind: "field"; name: string; value: string };

function asciiLower(c: string): string {
  return c >= "A" && c <= "Z" ? String.fromCharCode(c.charCodeAt(0) + 32) : c;
}

/** A quoted run starting at chars[at] (an opening quote): its unescaped text
 * and the index after the closing quote; null when it never closes. */
function readQuoted(chars: string[], at: number): [string, number] | null {
  let out = "";
  let i = at + 1;
  while (i < chars.length) {
    const c = chars[i];
    if (c === "\\" && i + 1 < chars.length && (chars[i + 1] === "\\" || DQ_SET.has(chars[i + 1]))) {
      out += chars[i + 1];
      i += 2;
    } else if (DQ_SET.has(c)) {
      return [out, i + 1];
    } else {
      out += c;
      i += 1;
    }
  }
  return null;
}

/** A regex token starting at chars[at] (a "/"): source, flags and the index
 * after it; null unless it closes, has a body and ends the token. */
function readRegex(chars: string[], at: number): [string, string, number] | null {
  let source = "";
  let i = at + 1;
  for (;;) {
    if (i >= chars.length) return null;
    const c = chars[i];
    if (c === "\\") {
      if (i + 1 >= chars.length) return null;
      source += c + chars[i + 1];
      i += 2;
    } else if (c === "/") {
      break;
    } else {
      source += c;
      i += 1;
    }
  }
  if (!source) return null;
  i += 1;
  let flags = "";
  while (i < chars.length && chars[i] >= "a" && chars[i] <= "z") {
    flags += chars[i];
    i += 1;
  }
  return i === chars.length || isWs(chars[i]) ? [source, flags, i] : null;
}

/** Split `raw` into tokens; `isField` says which lowercased names are fields. */
export function tokenize(raw: string, isField: (name: string) => boolean): Token[] {
  const chars = Array.from(raw);
  const n = chars.length;
  const out: Token[] = [];
  let i = 0;
  // After a closed quote: the rest of the run is a plain term.
  const restAsTerm = (from: number): number => {
    let j = from;
    while (j < n && !isWs(chars[j])) j += 1;
    if (j > from) out.push({ kind: "term", text: chars.slice(from, j).join("") });
    return j;
  };
  while (i < n) {
    if (isWs(chars[i])) {
      i += 1;
      continue;
    }
    let end = i;
    while (end < n && !isWs(chars[end])) end += 1;
    // a. phrase, optionally negated
    const negated = chars[i] === "-" && i + 1 < n;
    const q = negated ? i + 1 : i;
    if (OPEN_SET.has(chars[q])) {
      const r = readQuoted(chars, q);
      if (r) {
        out.push({ kind: "phrase", negated, text: r[0] });
        i = restAsTerm(r[1]);
        continue;
      }
    }
    // b. regex
    if (chars[i] === "/") {
      const r = readRegex(chars, i);
      if (r) {
        out.push({ kind: "regex", source: r[0], flags: r[1] });
        i = r[2];
        continue;
      }
    }
    // c. field
    let colon = -1;
    for (let k = i; k < end; k += 1) {
      if (chars[k] === ":") {
        colon = k;
        break;
      }
    }
    if (colon >= 0) {
      const name = chars.slice(i, colon).map(asciiLower).join("");
      if (isField(name)) {
        const v = colon + 1;
        if (v < n && OPEN_SET.has(chars[v])) {
          const r = readQuoted(chars, v);
          if (r) {
            out.push({ kind: "field", name, value: r[0] });
            i = restAsTerm(r[1]);
            continue;
          }
        }
        out.push({ kind: "field", name, value: chars.slice(v, end).join("") });
        i = end;
        continue;
      }
    }
    // d. term
    out.push({ kind: "term", text: chars.slice(i, end).join("") });
    i = end;
  }
  return out;
}

/** Write a field value so that the tokenizer reads it back unchanged: bare
 * when it is non-empty and holds no WS, no double-quote variant and no
 * backslash; else in "…" with every double-quote variant and backslash
 * escaped by a backslash. */
export function quoteValue(v: string): string {
  const chars = Array.from(v);
  if (chars.length && !chars.some((c) => isWs(c) || DQ_SET.has(c) || c === "\\")) return v;
  let out = '"';
  for (const c of chars) {
    if (c === "\\" || DQ_SET.has(c)) out += "\\";
    out += c;
  }
  return `${out}"`;
}

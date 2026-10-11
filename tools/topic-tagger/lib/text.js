// Text helpers. JS strings are UTF-16: a plain slice can cut an emoji in half and leave a lone
// surrogate, which makes the request body invalid JSON for the API. cut() never does that.
export function cut(s, n) {
  const t = String(s ?? '');
  if (t.length <= n) return t;
  let end = Math.max(0, n);
  const c = t.charCodeAt(end - 1);
  if (c >= 0xd800 && c <= 0xdbff) end--; // drop a dangling high surrogate
  return t.slice(0, end);
}

// cut with an ellipsis that keeps the total within n chars.
export const clip = (s, n) => { const t = String(s ?? ''); return t.length > n ? cut(t, n - 1) + '…' : t; };

// Replace any lone surrogate with U+FFFD (backstop at the request boundary).
export const wellFormed = (s) => String(s ?? '').toWellFormed();

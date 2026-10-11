// Minimal markdown to HTML for the review page: headings, paragraphs, bullet lists, bold, code.
// Escapes first, so nothing in the source can inject markup. Frontmatter is dropped.
export const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));

const inline = (s) => esc(s).replace(/\*\*([^*]+)\*\*/g, '<b>$1</b>').replace(/`([^`]+)`/g, '<code>$1</code>');

export function mdToHtml(md) {
  const lines = String(md ?? '').replace(/^---\n[\s\S]*?\n---\n?/, '').split('\n');
  const out = [];
  let para = [];
  let list = false;
  const flush = () => { if (para.length) { out.push(`<p>${inline(para.join(' '))}</p>`); para = []; } };
  const endList = () => { if (list) { out.push('</ul>'); list = false; } };
  for (const line of lines) {
    const h = line.match(/^(#{1,4})\s+(.*)$/);
    const li = line.match(/^\s*[-*]\s+(.*)$/);
    if (h) { flush(); endList(); const n = Math.min(h[1].length + 1, 5); out.push(`<h${n}>${inline(h[2])}</h${n}>`); }
    else if (li) { flush(); if (!list) { out.push('<ul>'); list = true; } out.push(`<li>${inline(li[1])}</li>`); }
    else if (!line.trim()) { flush(); endList(); }
    else { endList(); para.push(line.trim()); }
  }
  flush(); endList();
  return out.join('\n');
}

# 06: Highlight citations carry the source's public link and a proper title

**What to build:** Dominik pasted a Highlight Scout citation of a highlighted X post into WriteFlex and got "— atroyn, metaphors bewitch people at every scale, choosing the right ones is im…" with **no links at all** (DTC archive-panel-and-links-alpha19, 2026-09-28).
- **Cause 1:** `cite` for the `highlights` corpus emits only a `[highlight](file://…)` link, which is local and dropped by design, and never the work's public URL.
- **Cause 2:** the "title" of an X-post work is the post text, truncated.

Fix in the highlights adapter and `cite`:
- the public URL comes from the work's frontmatter (`url:` / `source_url` / the X post URL / DOI) as `[public](…)`;
- X posts cite as `— @author, post, D Month YYYY · [public](https://x.com/…)`, and never use the post text as a title;
- articles and books cite as `— Author, *Title*, Year · [public](url)`;
- Zotero items get the DOI or URL when present.
The local `file://` link stays only in the Markdown flavour (the apps already drop it on plain and export).

**Blocked by:** None. **Seams under test:** facade `cite` over a highlights fixture (X post, Readwise article, Zotero item with a DOI); the CLI parity test.
**Status:** ready

- [ ] Citing a highlight of an X post gives the x.com link; the post text never appears as the title.
- [ ] Articles and Zotero items carry their public URL or DOI.

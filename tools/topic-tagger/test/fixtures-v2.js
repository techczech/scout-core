// Small on-disk archive for v2 tests: tweets (reply, quote, link-only, thread), an article with fulltext,
// an X import snapshot, the reader's tweet stream and a roundup. No network.
import { mkdtempSync, mkdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

export const tweet = (id, author, text, date) => `---
title: ${text.slice(0, 40)}
author: ${author}
type: tweet
source_system: x
source_id: "${id}"
url: https://x.com/${author}/status/${id}
source_data: {"author_name":"${author}","saved_as":"likes"}
---

${text.split('\n').map((l) => `> ${l}`).join('\n')}

highlighted_at: ${date} | tags: like

---
`;

export const article = (id, author, title, url, hl, date, tags = 'ai, epistemology') => `---
title: ${title}
author: ${author}
type: article
source_system: readwise
source_id: "${id}"
url: ${url}
---

> ${hl}

highlighted_at: ${date} | tags: ${tags}

---
`;

export function buildFixture() {
  const root = mkdtempSync(join(tmpdir(), 'tt-v2-'));
  const works = join(root, 'readings/works');
  const full = join(root, 'readings/fulltext');
  mkdirSync(works, { recursive: true });
  mkdirSync(full, { recursive: true });
  const w = (f, s) => writeFileSync(join(works, f), s);
  // parent tweet from August (outside the month) that a September reply answers
  w('lab-astra-is-here-100.md', tweet('100', 'openai', 'Introducing GPT-6 Astra, our new frontier model.', '2026-08-30'));
  // September items
  w('alice-reply-201.md', tweet('201', 'alice', 'This changes everything for proofs.', '2026-09-02'));
  w('bob-quote-202.md', tweet('202', 'bob', 'Agree with this take', '2026-09-03'));
  w('carol-link-203.md', tweet('203', 'carol', 'Worth reading\n🔗 https://www.example.org/essay/?utm_source=x', '2026-09-04'));
  w('carol-second-204.md', tweet('204', 'carol', 'Part two of my thoughts on mathematicians and AI', '2026-09-04'));
  w('dave-plain-205.md', tweet('205', 'dave', 'Gardening is relaxing.\n![image](https://pbs.twimg.com/media/x.jpg)', '2026-09-05'));
  w('erin-missing-parent-206.md', tweet('206', 'erin', 'No way', '2026-09-06'));
  w('essay-on-proofs-300.md', article('300', 'Frank', 'An essay on AI proofs', 'https://example.org/essay', 'Mathematicians disagree about credit.', '2026-07-01', 'ai, mathematics, like'));
  writeFileSync(join(full, 'essay-on-proofs-300.md'), '---\ntitle: x\n---\nThe essay body begins here and argues about credit for AI proofs.');
  w('undated-400.md', '---\ntitle: U\nauthor: U\ntype: article\nsource_system: readwise\nsource_id: "400"\n---\n\n> no date\n');
  // X import snapshot with reply / quote ids
  const imp = join(root, 'imports/x-api-2026-10-10');
  mkdirSync(imp, { recursive: true });
  const rows = [
    { tweet_id: '201', reply_to_id: '100', parent_text: null, quoted_tweet_id: null, article_urls: [] },
    { tweet_id: '202', reply_to_id: null, quoted_tweet_id: '900', article_urls: [] }, // 900 = reader's own tweet
    { tweet_id: '206', reply_to_id: '999', quoted_tweet_id: null, article_urls: [] }, // parent nowhere
  ];
  writeFileSync(join(imp, 'likes-saved.jsonl'), rows.map((r) => JSON.stringify(r)).join('\n') + '\n');
  // reader's tweet stream
  const stream = join(root, 'tweets/stream');
  mkdirSync(stream, { recursive: true });
  writeFileSync(join(stream, '2026-09.md'), `---\ntitle: s\n---\n\n## 2026-09-01T10:00:00Z — tweet 900 · kind: original\n\n<!-- tweet id="900" -->\n~~~\nMathematicians are being protectionist about AI proofs. https://twitter.com/x/status/1\n~~~\n\n## 2026-09-02T10:00:00Z — tweet 901 · kind: reply\n\n<!-- tweet id="901" -->\n~~~\n@someone Not even a little bit.\n~~~\n`);
  // roundup
  const rdir = join(root, 'roundups');
  mkdirSync(rdir, { recursive: true });
  writeFileSync(join(rdir, 'mondai-sep-2026.json'), JSON.stringify({
    presentation: { title: 'MondAI Round Up - September 2026', title_page_subtitle: 'July 13 - September 13, 2026' },
    landing_intro: 'Model releases take up **much** of it.',
    sections: [{ title: 'Model Releases', items: ['a'] }],
    content_library: [{ external_id: 'a', title: 'GPT-6 Astra', date: '2026-08-30', summary: 'OpenAI released [Astra](https://x).' },
      { external_id: 'b', title: 'Maths letter', date: '2026-09-04', summary: 'Fields Medallists wrote.' }, { external_id: 'c', title: 'c', date: 'Sep 5, 2026', summary: 's' }, { external_id: 'd', title: 'd', date: '2026-09-10', summary: 's' }],
  }));
  return { root, stream, rdir };
}

// A fake Anthropic client. Tagging answers derive from each item's text; usage mimics caching.
export function fakeClient({ calls = [], clean = false } = {}) {
  const msg = (obj, usage) => ({ stop_reason: 'end_turn', content: [{ type: 'text', text: JSON.stringify(obj) }], usage });
  let tagRequests = 0;
  const answer = (params) => {
    calls.push(params);
    const sys = Array.isArray(params.system) ? params.system.map((b) => b.text).join('\n') : params.system;
    if (sys.includes('factual briefing')) {
      return msg({
        overview: 'A month of model releases and maths arguments.',
        events: [
          { id: 'X', name: 'Mathematicians letter', date: '2026-09-04', aliases: ['the letter'], description: 'Open letter.', items: ['m0003', 'm9999'] },
          { id: 'Y', name: 'GPT-6 Astra release', date: '2026-08-30', aliases: ['Astra'], description: 'OpenAI release.', items: ['m0001'] },
        ],
        names: [{ name: 'GPT-6 Astra', aliases: ['Astra', 'GPT-6 Astra'], note: 'OpenAI model' }],
        debates: [{ title: 'Credit for AI proofs', summary: 'Who gets credit.', when: 'early September' }],
        engagement: 'Most items are about the maths debate.',
      }, { input_tokens: 5000, output_tokens: 800 });
    }
    if (sys.includes('fixed list of research themes')) {
      const themes = Array.from({ length: 30 }, (_, i) => ({ name: i === 0 ? 'AI and mathematics' : i === 1 ? 'Gardening and leisure' : `Theme ${i}`, definition: 'd', include: 'i', exclude: 'e' }));
      return msg({ themes: [...themes, { name: 'ai and mathematics', definition: 'dup', include: '', exclude: '' }], notes: 'n' }, { input_tokens: 4000, output_tokens: 2000 });
    }
    // tagging / QA
    tagRequests++;
    const content = params.messages[0].content;
    const ids = [...content.matchAll(/<item id="(m\d+)">([\s\S]*?)<\/item>/g)];
    const results = ids.map(([, id, body]) => {
      const ai = /AI|Astra|proof|mathematic/i.test(body);
      return {
        id, ai_related: ai,
        themes: ai ? (clean ? ['AI and mathematics'] : ['AI and mathematics', 'Not a theme']) : [],
        entities: /Astra/.test(body) ? ['Astra', 'astra', 'OpenAI'] : [],
        event: /proof|mathematic/i.test(body) ? 'E02' : (/Astra/.test(body) ? 'E01' : ''),
        relevance: ai ? 'Fits his interest in how mathematicians reacted to AI proof claims this month' : '',
        confidence: body.includes('No way') ? 'low' : 'high',
      };
    });
    const sysTok = Math.ceil(sys.length / 3);
    const usage = tagRequests === 1
      ? { input_tokens: 300, cache_creation_input_tokens: sysTok, cache_read_input_tokens: 0, output_tokens: 400 }
      : { input_tokens: 300, cache_creation_input_tokens: 0, cache_read_input_tokens: sysTok, output_tokens: 400 };
    return msg({ results }, usage);
  };
  return {
    calls,
    messages: {
      create: async (params) => answer(params),
      stream: (params) => ({ finalMessage: async () => answer(params) }),
      countTokens: async ({ system, messages }) => ({ input_tokens: Math.ceil((system.map((b) => b.text).join('\n\n').length + messages[0].content.length) / 3) }),
    },
  };
}

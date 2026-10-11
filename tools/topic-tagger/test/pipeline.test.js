import { test } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { buildFixture, fakeClient } from './fixtures-v2.js';
import * as v2 from '../lib/pipeline.js';
import { countTagsInText, condenseTweets, parseTweetStream, condenseRoundup, findRoundup } from '../lib/sources.js';
import { renderReviewV2 } from '../lib/review-v2.js';

test('Readwise tag counting: case-folded, system tags excluded', () => {
  const c = countTagsInText('highlighted_at: 2026-01-01 | tags: like, AI, Epistemology\nhighlighted_at: 2026-01-02 | tags: ai, favorite, 👻 ai highlighted\nnot a tag line | tags: zzz');
  assert.deepEqual([...c.entries()], [['ai', 2], ['epistemology', 1]]);
});

test('tweet stream and roundup condensers stay under their char budgets', () => {
  const tweets = Array.from({ length: 400 }, (_, i) => ({ created: '2026-09-02T00:00:00Z', id: String(i), kind: i % 2 ? 'reply' : 'original', text: `post ${i} ${'word '.repeat(80)} https://twitter.com/a/status/1 https://t.co/x` }));
  const c = condenseTweets(tweets, { maxChars: 30000 });
  assert.ok(c.text.length <= 30000);
  assert.ok(c.perTweetCap < 280);
  assert.ok(!c.text.includes('t.co'));
  assert.equal(parseTweetStream('## 2026-09-01T10:00:00Z — tweet 9 · kind: reply\n\n<!-- tweet id="9" -->\n~~~\nhi\n~~~')[0].text, 'hi');
  const lib = Array.from({ length: 80 }, (_, i) => ({ external_id: String(i), title: `T${i}`, date: '2026-08-01', summary: 's'.repeat(1000) }));
  const r = condenseRoundup({ presentation: {}, content_library: lib, sections: [] }, { maxChars: 20000 });
  assert.ok(r.text.length <= 20000);
});

test('roundup lookup by month name', () => {
  const { rdir } = buildFixture();
  assert.match(findRoundup(rdir, '2026-09'), /mondai-sep-2026\.json$/);
  assert.equal(findRoundup(rdir, '2026-08'), null);
});

function env(root, stream, rdir, client, extra = {}) {
  return {
    archiveRoot: root, month: '2026-09', runId: 'test-v2', v1RunId: 'v1run', tweetsDir: stream, roundupDir: rdir,
    model: 'claude-haiku-5-5', effort: 'medium', briefingModel: 'claude-sonnet-5-5', themesModel: 'claude-opus-5-5',
    qaModel: 'claude-sonnet-5-5', qaEffort: 'medium', tweetsChars: 30000, roundupChars: 20000, userReserve: 14000,
    maxUsd: 5, concurrency: 2, seed: 1, force: false, client, log: () => {}, ...extra,
  };
}

function seedProfileAndV1(root) {
  mkdirSync(join(root, 'analysis/context'), { recursive: true });
  writeFileSync(join(root, 'analysis/context/reader-profile.md'), '---\nstatus: DRAFT\n---\n# Reader\nHe researches AI and mathematics.');
  mkdirSync(join(root, 'analysis/topics/v1run'), { recursive: true });
  writeFileSync(join(root, 'analysis/topics/v1run/vocabulary.json'), JSON.stringify({ topics: [{ topic: 'GPT-6 Astra release', definition: 'd', count: 3 }] }));
  writeFileSync(join(root, 'analysis/topics/v1run/tags.jsonl'), ['x:201', 'x:203', 'x:205'].map((k) => JSON.stringify({ key: k, topics: ['old topic'] })).join('\n') + '\n');
}

test('v2 stages end to end with a fake client: files, ids, cache stats, cost cap, readings untouched', async () => {
  const { root, stream, rdir } = buildFixture();
  execFileSync('git', ['init', '-q'], { cwd: root });
  execFileSync('git', ['add', 'readings'], { cwd: root });
  execFileSync('git', ['-c', 'user.email=t@t', '-c', 'user.name=t', 'commit', '-qm', 'fixture'], { cwd: root });
  seedProfileAndV1(root);
  const calls = [];
  const e = env(root, stream, rdir, fakeClient({ calls }));
  const p = v2.paths(e);

  v2.stageProfileSources(e);
  const ph = readFileSync(v2.stageProfileHtml(e).html, 'utf8');
  assert.match(ph, /<h2>Reader<\/h2>/);
  assert.ok(!/<script|src=/.test(ph));
  const tags = JSON.parse(readFileSync(p.readwiseTags, 'utf8'));
  assert.deepEqual(tags.tags.map((t) => t.tag), ['ai', 'mathematics']); // 'like' excluded

  await v2.stageBriefing(e);
  const events = JSON.parse(readFileSync(p.eventsJson, 'utf8')).events;
  assert.deepEqual(events.map((x) => x.id), ['E01', 'E02']); // renumbered by date
  assert.equal(events[0].name, 'GPT-6 Astra release');
  assert.deepEqual(events[1].items, ['m0003']); // unknown m9999 dropped
  const md = readFileSync(p.briefingMd, 'utf8');
  assert.match(md, /\*\*E02 · Mathematicians letter\*\*/);
  assert.match(md, /\*\*GPT-6 Astra\*\* = "Astra"/);

  await v2.stageThemes(e);
  const th = JSON.parse(readFileSync(p.themes, 'utf8'));
  assert.equal(th.meta.status, 'DRAFT');
  assert.equal(th.themes.length, 30);

  const counts = v2.stageEnrich(e);
  assert.equal(counts.reply_parent_found, 1);
  assert.equal(readFileSync(p.enrichment, 'utf8').trim().split('\n').length, 6);

  const t = await v2.stageTag(e);
  assert.equal(t.tagged, 6);
  const recs = readFileSync(p.tags, 'utf8').trim().split('\n').map((l) => JSON.parse(l));
  const astra = recs.find((r) => r.key === 'x:201');
  assert.deepEqual(astra.entities, ['GPT-6 Astra', 'OpenAI']); // alias + case dedupe
  assert.deepEqual(astra.themes, ['AI and mathematics']); // "Not a theme" dropped
  assert.equal(astra.event, 'E02');
  assert.equal(recs.find((r) => r.key === 'x:205').ai_related, false);
  const run = JSON.parse(readFileSync(p.runJson, 'utf8'));
  assert.equal(run.prompt_version, 'topics-v2');
  assert.equal(run.readings_unchanged, true);
  assert.ok(run.prompt_tokens_system > 0 && run.prompt_tokens_system < 100000);
  assert.ok(existsSync(p.promptText) && readFileSync(p.promptText, 'utf8').includes('<month_index>'));
  assert.ok(readFileSync(p.promptText, 'utf8').includes('<mondai_roundup>'));
  assert.ok(readFileSync(p.promptText, 'utf8').includes('protectionist')); // reader's own tweets
  const tagCall = calls.find((c) => Array.isArray(c.system));
  assert.match(tagCall.messages[0].content, /\[replying to @openai\]/);
  await assert.rejects(() => v2.stageTag(e), /--force/);

  await v2.stageQa(e);
  const run2 = JSON.parse(readFileSync(p.runJson, 'utf8'));
  assert.equal(run2.qa.all.n, 6);
  assert.equal(run2.qa.all.themes_any_overlap, 1);

  const rv = v2.stageReview(e);
  const html = readFileSync(rv.review, 'utf8');
  assert.match(html, /v1 vs v2/);
  assert.match(html, /E02 · Mathematicians letter/);
  assert.match(html, /Reader profile/);
  assert.ok(!/<script|src=|<link /.test(html));
  const run3 = JSON.parse(readFileSync(p.runJson, 'utf8'));
  assert.ok(run3.cost_usd_total > 0);
  assert.equal(run3.readings_unchanged, true);
  assert.equal(execFileSync('git', ['status', '--porcelain', '--', 'readings'], { cwd: root, encoding: 'utf8' }), '');
});

test('cost cap aborts a paid stage before any request', async () => {
  const { root, stream, rdir } = buildFixture();
  seedProfileAndV1(root);
  const calls = [];
  await assert.rejects(() => v2.stageBriefing(env(root, stream, rdir, fakeClient({ calls }), { maxUsd: 0.0001 })), /exceeds cap/);
  assert.equal(calls.length, 0);
});

test('markdown: bold, italics, lists; asterisks inside words left alone', async () => {
  const { mdToHtml } = await import('../lib/markdown.js');
  assert.equal(mdToHtml('He translated *Women, Fire* and **this** a*b*c'), '<p>He translated <i>Women, Fire</i> and <b>this</b> a*b*c</p>');
  assert.equal(mdToHtml('- *A* x'), '<ul>\n<li><i>A</i> x</li>\n</ul>');
});

test('review page escapes model text and renders profile markdown', () => {
  const html = renderReviewV2({
    run: { run_id: 'r', model: 'm', effort: 'medium', prompt_version: 'topics-v2', selection_rule: 's', item_count: 1, tagged_items: 1, ai_share: 1, cache_stats: { token_hit_rate: 0.9 } },
    profileMd: '---\na: b\n---\n# Hi\n- **bold** <b>', briefingMd: '', profileHref: 'p', briefingHref: 'b',
    themes: [{ name: 'T <i>', definition: 'd', count: 1, examples: ['k'] }], entities: [{ name: 'E', count: 1 }],
    events: [], qa: null, costs: [{ stage: 's', model: 'm', usd: 0.5 }], compare: [],
    examples: { k: { author: 'a', date: 'd', url: 'https://x.com/a', snippet: '<script>x</script> ![image](https://pbs.twimg.com/a.jpg) https://t.co/abc' } },
  });
  assert.ok(html.includes('&lt;script&gt;'));
  assert.ok(!html.includes('pbs.twimg') && !html.includes('t.co/abc'));
  assert.ok(html.includes('<li><b>bold</b> &lt;b&gt;</li>'));
  assert.ok(html.includes('T &lt;i&gt;'));
  assert.ok(!html.includes('a: b'));
  assert.ok(html.includes('prefers-color-scheme:dark'));
  assert.ok(html.includes('width=device-width'));
  assert.ok(html.includes('flex-wrap:wrap'));
});

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { join } from 'node:path';
import { normaliseUrl, tweetIdFromUrl, buildArchiveIndex, loadXSnapshot, enrichItems, bodyLinks } from '../lib/enrich.js';
import { loadOwnTweets } from '../lib/sources.js';
import { selectWorks } from '../lib/works.js';
import { buildFixture } from './fixtures-v2.js';

test('URL normalisation drops tracking params, www, trailing slash; twitter = x', () => {
  assert.equal(normaliseUrl('https://www.example.org/essay/?utm_source=x&id=3#top'), 'example.org/essay?id=3');
  assert.equal(normaliseUrl('http://example.org/essay'), 'example.org/essay');
  assert.equal(normaliseUrl('https://twitter.com/a/status/1'), 'x.com/a/status/1');
  assert.equal(normaliseUrl('not a url'), null);
  assert.equal(tweetIdFromUrl('https://twitter.com/foo/status/123?s=20'), '123');
  assert.equal(tweetIdFromUrl('https://example.org/status/123'), null);
});

test('body links skip t.co and images', () => {
  const links = bodyLinks({ highlights: [{ text: 'see https://t.co/abc and https://pbs.twimg.com/x.jpg\n🔗 https://example.org/a.' }] });
  assert.deepEqual(links, ['https://example.org/a']);
});

test('enrichment joins: reply parent from archive, quote from own tweets, linked article from fulltext, threads', () => {
  const { root, stream } = buildFixture();
  const { items } = selectWorks(join(root, 'readings/works'), '2026-09-01', '2026-09-30');
  assert.equal(items.length, 6);
  const archive = buildArchiveIndex(root);
  const snapshot = loadXSnapshot(join(root, 'imports'));
  const ownTweets = loadOwnTweets(stream);
  const { contexts, counts } = enrichItems(items, { archive, snapshot, ownTweets, archiveRoot: root });

  const reply = contexts.get('x:201');
  assert.equal(reply.parent.handle, 'openai');
  assert.match(reply.parent.text, /GPT-6 Astra/);
  const quote = contexts.get('x:202');
  assert.equal(quote.quoted.source, 'own-tweets');
  assert.match(quote.quoted.text, /protectionist/);
  const link = contexts.get('x:203');
  assert.equal(link.linked.length, 1);
  assert.match(link.linked[0].text, /essay body begins/); // fulltext preferred over highlights
  assert.equal(link.linked[0].title, 'An essay on AI proofs');
  assert.deepEqual(link.thread, ['x:204']);
  assert.deepEqual(contexts.get('x:204').thread, ['x:203']);
  assert.equal(contexts.get('x:206').parent_missing, '999');
  assert.equal(contexts.get('x:205').thread.length, 0);

  assert.deepEqual(counts, {
    items: 6, tweets: 6, replies: 2, reply_parent_found: 1, quotes: 1, quote_text_found: 1,
    with_links: 1, link_article_found: 1, thread_grouped: 2, any_enrichment: 4,
  });
});

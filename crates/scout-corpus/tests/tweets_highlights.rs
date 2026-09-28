//! Seam tests for the T2 corpora: tweet files (one document per tweet) and
//! the Highlight Scout archive (a work is a document, a highlight a passage),
//! the query grammar in search, cross-corpus merging and `cite`. The T3/T4
//! agreement checks (invariant 3) run over both corpora here too.

use scout_corpus::colloc::CollocRequest;
use scout_corpus::concord::{node_tokens, DistBy, KwicRequest, Near};
use scout_corpus::ngrams::NgramRequest;
use scout_corpus::registry::{CorpusConfig, CorpusKind, DocumentUnit, FieldMap, WRITEFLEX_LINK};
use scout_corpus::{search_all, Corpus, DocFilter, PassageId, SearchRequest, SearchResults};
use std::fs;
use std::path::Path;

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

fn snapshot(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    for e in walkdir(root) {
        out.push((e.display().to_string(), fs::read(&e).unwrap()));
    }
    out.sort();
    out
}

fn walkdir(root: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for e in fs::read_dir(root).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            out.extend(walkdir(&p));
        } else {
            out.push(p);
        }
    }
    out
}

const STREAM: &str = "---
title: \"Twitter/X stream — July 2025\"
date: 2025-07
venue: \"Twitter/X (@techczech)\"
lang: en
genre: digest
tweet_collection: stream
---

# Twitter/X stream — July 2025

## 2025-07-01T04:17:24Z — tweet 1939901126326764018 · kind: original

<!-- tweet id=\"1939901126326764018\" -->
~~~
A metaphor is a model. Every metaphor breaks somewhere.

1. metaphor as frame
~~~

## 2025-07-26T10:12:00Z — tweet 1949000000000000001 · kind: reply

<!-- tweet id=\"1949000000000000001\" -->
~~~
@someone The conceptual metaphor frame again, metaphor.
~~~

Reply to: @someone metaphor

## 2025-07-27T08:00:00Z — tweet 1949000000000000002 · kind: original

<!-- tweet id=\"1949000000000000002\" -->
~~~
https://t.co/abc
~~~

Media: https://pbs.twimg.com/media/x.jpg
";

const THREAD: &str = "---
title: \"One word story is a metaphor\"
date: 2023-10-27
venue: \"Twitter/X (@techczech)\"
lang: en
genre: essay
tweet_collection: thread
---

# One word story is a metaphor

## 2023-10-27T07:34:15Z — tweet 1717806900421742847

<!-- tweet id=\"1717806900421742847\" -->
~~~
One word story is a metaphor for how models generate text.
~~~

## 2023-10-27T07:35:04Z — tweet 1717807107582669195

<!-- tweet id=\"1717807107582669195\" -->
~~~
And the frame metaphor breaks.
~~~
";

fn tweets_fixture(root: &Path) {
    write(root, "stream/2025-07.md", STREAM);
    write(root, "threads/2023-10-27-one-word.md", THREAD);
}

const ZOTERO: &str = "---
title: \"Metaphors We Live By: a
  \\\"classic\\\" study\"
author: George Lakoff
type: book
source_system: zotero
source_id: \"ABC\"
url: mailto:lakoff@example.org
imported_at: 2026-06-17T08:18:52Z
updated_at: 2026-06-17T08:18:52Z
source_data: {\"date\":\"1980-00-00 1980\",\"fields\":{\"DOI\":\"10.1000/mwlb\"},\"zotero_key\":\"ABC\"}
---

> Metaphor is pervasive in everyday life, not just in language but in thought and action.
> The essence of metaphor is understanding one kind of thing in terms of another.

highlighted_at: 2024-01-15 | tags: linguistics, metaphor | color: yellow

A note of mine about metaphor that must not count.

---

![](../assets/h9.png)

format: image

---

```latex
\\metaphor
```

highlighted_at: 2024-01-16 | format: latex

---

";

const XWORK: &str = "---
title: Frames and metaphor…
author: 0xabi
type: tweet
source_system: x
source_id: \"1939901126326764018\"
url: https://x.com/0xabi/status/1939901126326764018
imported_at: 2026-06-21T00:43:05Z
updated_at: 2026-06-21T00:43:05Z
source_data: {\"author_name\":\"Abi\",\"saved_as\":\"likes\"}
---

> Frames and metaphor: a metaphor frames the problem.
>
> Second paragraph without the word.

highlighted_at: 2026-01-26 | tags: like

---

";

const READWISE: &str = "---
title: Towards a Bayesian Theory of Willpower
author: astralcodexten.substack.com
type: article
source_system: readwise
source_id: \"rw_book_8497389\"
url: https://astralcodexten.substack.com/p/towards
imported_at: 2026-06-17T08:18:52Z
updated_at: 2021-04-03T15:14:29Z
source_data: {\"reader_document_id\":\"\",\"readwise_url\":\"https://readwise.io/bookreview/8497389\"}
---

> The brain is an inference engine, a metaphor we should take seriously.

highlighted_at: 2021-04-05 | tags: Health

---

> Another highlight, on scaffolding.

highlighted_at: 2021-04-03 | color: blue

---

";

fn highlights_fixture(root: &Path) {
    write(
        root,
        "readings/works/lakoff-metaphors-we-live-by-abc.md",
        ZOTERO,
    );
    write(
        root,
        "readings/works/0xabi-frames-1939901126326764018.md",
        XWORK,
    );
    write(
        root,
        "readings/works/astral-bayesian-willpower-rw-8497389.md",
        READWISE,
    );
    // Not works: never indexed.
    write(
        root,
        "readings/fulltext/astral.md",
        "metaphor metaphor metaphor\n",
    );
    write(root, "README.md", "# metaphor\n");
}

fn tweets_cfg(root: &Path) -> CorpusConfig {
    CorpusConfig {
        id: "tweets".into(),
        name: "Tweets".into(),
        kind: CorpusKind::MarkdownFolder,
        path: root.display().to_string(),
        include: vec!["**/*.md".into()],
        exclude: vec![],
        require_frontmatter: vec![],
        field_map: FieldMap::default(),
        default_author: Some("Dominik Lukeš".into()),
        boilerplate: vec![],
        link: Some(WRITEFLEX_LINK.into()),
        document_unit: DocumentUnit::Tweet,
    }
}

fn highlights_cfg(root: &Path) -> CorpusConfig {
    CorpusConfig {
        id: "highlights".into(),
        name: "Highlights".into(),
        kind: CorpusKind::HighlightScoutArchive,
        path: root.display().to_string(),
        include: vec![],
        exclude: vec![],
        require_frontmatter: vec![],
        field_map: FieldMap::default(),
        default_author: None,
        boilerplate: vec![],
        link: None,
        document_unit: DocumentUnit::File,
    }
}

struct Fixture {
    _dirs: Vec<tempfile::TempDir>,
    tweets_src: std::path::PathBuf,
    hl_src: std::path::PathBuf,
    tweets: Corpus,
    highlights: Corpus,
}

fn fixture() -> Fixture {
    let (t, h, data) = (
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
        tempfile::tempdir().unwrap(),
    );
    tweets_fixture(t.path());
    highlights_fixture(h.path());
    let tweets = Corpus::from_config(tweets_cfg(t.path()), data.path());
    let highlights = Corpus::from_config(highlights_cfg(h.path()), data.path());
    tweets.build_index(false).unwrap();
    highlights.build_index(false).unwrap();
    Fixture {
        tweets_src: t.path().to_path_buf(),
        hl_src: h.path().to_path_buf(),
        _dirs: vec![t, h, data],
        tweets,
        highlights,
    }
}

fn search(c: &Corpus, q: &str) -> SearchResults {
    c.search(&SearchRequest::new(q)).unwrap()
}

fn keys(r: &SearchResults) -> Vec<String> {
    r.results.iter().map(|d| d.rel_path.clone()).collect()
}

// ---------- tweets ----------

#[test]
fn every_tweet_is_a_document_with_its_own_date_and_url() {
    let f = fixture();
    let st = f.tweets.status().unwrap();
    assert_eq!(st.docs, 5, "three stream tweets and two thread tweets");
    let r = search(&f.tweets, "metaphor");
    let mut got = keys(&r);
    got.sort();
    assert_eq!(
        got,
        vec![
            "stream/2025-07.md#1939901126326764018",
            "stream/2025-07.md#1949000000000000001",
            "threads/2023-10-27-one-word.md#1717806900421742847",
            "threads/2023-10-27-one-word.md#1717807107582669195",
        ]
    );
    let d = r
        .results
        .iter()
        .find(|d| d.rel_path == "stream/2025-07.md#1949000000000000001")
        .unwrap();
    assert_eq!(d.date.as_deref(), Some("2025-07-26T10:12:00Z"));
    assert_eq!(d.date_display.as_deref(), Some("26 July 2025"));
    assert_eq!(d.genre.as_deref(), Some("reply"));
    assert_eq!(d.kind.as_deref(), Some("tweet"));
    assert_eq!(
        d.public_url.as_deref(),
        Some("https://x.com/techczech/status/1949000000000000001")
    );
    assert!(d.path.ends_with("stream/2025-07.md"), "{}", d.path);
    let hit = &d.hits[0];
    assert!(scout_corpus::search::quote_is_original(Path::new(&d.path), hit).unwrap());
    // The tweet text is line 25 of the file.
    assert_eq!(hit.line, 25);
    assert!(hit.link.as_deref().unwrap().ends_with("2025-07.md&line=25"));
    assert_eq!(
        d.citation.markdown,
        format!(
            "> {}\n\n— Dominik Lukeš (@techczech), tweet, 26 July 2025 · [public](https://x.com/techczech/status/1949000000000000001)\n",
            hit.quote
        )
    );
    assert!(d.citation.plain.ends_with(
        "— Dominik Lukeš (@techczech), tweet, 26 July 2025 · public: https://x.com/techczech/status/1949000000000000001\n"
    ));
    // A thread tweet keeps the thread's title.
    let t = r
        .results
        .iter()
        .find(|d| d.rel_path.ends_with("#1717807107582669195"))
        .unwrap();
    assert_eq!(t.title, "One word story is a metaphor");
    assert_eq!(t.genre.as_deref(), Some("thread"));
}

#[test]
fn tweet_metadata_lines_are_not_text() {
    let f = fixture();
    // "Reply to: @someone metaphor", "Media:", the H1 and the headings do
    // not count: 3 + 2 in the stream, 1 + 1 in the thread.
    let k = f.tweets.kwic(&KwicRequest::new("metaphor")).unwrap();
    assert_eq!(k.total, 7);
    assert_eq!(k.pieces, 4);
    assert_eq!(f.tweets.kwic(&KwicRequest::new("reply")).unwrap().total, 0);
    assert_eq!(f.tweets.kwic(&KwicRequest::new("media")).unwrap().total, 0);
    assert_eq!(f.tweets.kwic(&KwicRequest::new("tweet")).unwrap().total, 0);
}

// ---------- highlights ----------

#[test]
fn a_work_is_a_document_and_a_highlight_a_passage() {
    let f = fixture();
    let st = f.highlights.status().unwrap();
    assert_eq!(
        (st.docs, st.passages),
        (3, 4),
        "image/latex records and notes carry no text"
    );
    assert_eq!(
        st.stale.as_ref().map(|s| s.added + s.changed + s.removed),
        Some(0)
    );
    let r = search(&f.highlights, "pervasive");
    let d = &r.results[0];
    assert_eq!(d.title, "Metaphors We Live By: a \"classic\" study");
    assert_eq!(d.date.as_deref(), Some("1980"));
    assert_eq!(d.date_source.as_deref(), Some("published"));
    assert_eq!(d.source.as_deref(), Some("zotero"));
    assert_eq!(d.kind.as_deref(), Some("book"));
    // A Zotero item with no web `url:` carries its DOI.
    assert_eq!(
        d.public_url.as_deref(),
        Some("https://doi.org/10.1000/mwlb")
    );
    let hit = &d.hits[0];
    assert_eq!(
        hit.quote,
        "Metaphor is pervasive in everyday life, not just in language but in thought and action."
    );
    assert_eq!(hit.line, 14);
    assert_eq!(hit.tags, vec!["linguistics", "metaphor"]);
    assert_eq!(hit.color.as_deref(), Some("yellow"));
    assert_eq!(hit.saved_at.as_deref(), Some("2024-01-15"));
    let link = hit.link.as_deref().unwrap();
    assert!(link.starts_with("file:///"), "{link}");
    assert!(link.ends_with("/readings/works/lakoff-metaphors-we-live-by-abc.md"));
    assert_eq!(
        d.citation.markdown,
        format!("> {}\n\n— George Lakoff, *Metaphors We Live By: a \"classic\" study*, 1980 · [public](https://doi.org/10.1000/mwlb) · [highlight]({link})\n", hit.quote)
    );
    // The quote is the text of source line 14 without its `> ` marker.
    let lines: Vec<String> = fs::read_to_string(&d.path)
        .unwrap()
        .lines()
        .map(String::from)
        .collect();
    assert_eq!(lines[13], format!("> {}", hit.quote));

    // A tweet's publication date comes from its id.
    let d = &search(&f.highlights, "problem").results[0];
    assert_eq!(d.date.as_deref(), Some("2025-07-01T04:17:24Z"));
    assert_eq!(d.date_source.as_deref(), Some("published"));
    // An X post cites as a post with its x.com link; its text is no title.
    assert_eq!(
        d.citation.plain,
        "“Frames and metaphor: a metaphor frames the problem.”\n— @0xabi, post, 1 July 2025 · public: https://x.com/0xabi/status/1939901126326764018\n"
    );
    assert!(d.citation.markdown.contains(
        "— @0xabi, post, 1 July 2025 · [public](https://x.com/0xabi/status/1939901126326764018) · [highlight](file:///"
    ));
    assert!(!d.citation.markdown.contains("Frames and metaphor…"));
    // No publication date: the earliest highlight date, marked as saved.
    let d = &search(&f.highlights, "inference").results[0];
    assert_eq!(d.date.as_deref(), Some("2021-04-03"));
    assert_eq!(d.date_source.as_deref(), Some("saved"));
    assert!(d
        .citation
        .plain
        .ends_with("— astralcodexten.substack.com, Towards a Bayesian Theory of Willpower, saved 2021 · public: https://astralcodexten.substack.com/p/towards\n"),
        "an article carries its public URL; the local link stays out of plain");
    // Notes, fulltext and non-work files are not indexed.
    assert!(search(&f.highlights, "note").results.is_empty());
    let k = f.highlights.kwic(&KwicRequest::new("metaphor")).unwrap();
    assert_eq!((k.total, k.pieces), (5, 3));
}

#[test]
fn highlight_fields_filter() {
    let f = fixture();
    let h = &f.highlights;
    let base = keys(&search(h, "metaphor"));
    assert_eq!(base.len(), 3);
    assert_eq!(
        keys(&search(h, "metaphor source:zotero")),
        vec!["readings/works/lakoff-metaphors-we-live-by-abc.md"]
    );
    assert_eq!(
        keys(&search(h, "metaphor zo:")),
        keys(&search(h, "metaphor source:zotero"))
    );
    assert_eq!(
        keys(&search(h, "metaphor ty:tweets")),
        vec!["readings/works/0xabi-frames-1939901126326764018.md"]
    );
    assert_eq!(
        keys(&search(h, "tw: metaphor")),
        keys(&search(h, "metaphor ty:tweets"))
    );
    assert_eq!(keys(&search(h, "metaphor au:lakoff")).len(), 1);
    assert_eq!(keys(&search(h, "metaphor ti:bayesian")).len(), 1);
    // Passage fields: tag and colour pick highlights, not works.
    let r = search(h, "co:blue");
    assert_eq!(r.total_passages, 1);
    assert_eq!(
        r.results[0].hits[0].quote,
        "Another highlight, on scaffolding."
    );
    assert_eq!(search(h, "tag:linguistics").total_passages, 1);
    assert_eq!(search(h, "metaphor tag:health").total_passages, 1);
    assert_eq!(search(h, "scaffolding tag:health").total_passages, 0);
    // Dates: the publication (or saved) date.
    assert_eq!(keys(&search(h, "metaphor y:1980")).len(), 1);
    assert_eq!(keys(&search(h, "metaphor before:2022")).len(), 2);
    assert_eq!(keys(&search(h, "metaphor y:2020-2025")).len(), 2);
    // The same fields are document filters for the analytics views.
    let mut k = KwicRequest::new("metaphor");
    k.filter.source = vec!["x".into()];
    assert_eq!(h.kwic(&k).unwrap().total, 2);
}

// ---------- grammar ----------

#[test]
fn search_speaks_the_query_grammar() {
    let f = fixture();
    let (t, h) = (&f.tweets, &f.highlights);
    // Plain words: prefix-matched and ANDed (T1), even three of them.
    assert_eq!(keys(&search(t, "metaph frame")).len(), 3);
    assert_eq!(keys(&search(t, "metaphor frame conceptual")).len(), 1);
    // OR, exclusion, phrase, prefix.
    assert_eq!(keys(&search(h, "pervasive OR inference")).len(), 2);
    assert_eq!(keys(&search(h, "pervasive | inference")).len(), 2);
    assert_eq!(keys(&search(h, "metaphor -frames")).len(), 2);
    assert_eq!(keys(&search(t, "\"conceptual metaphor\"")).len(), 1);
    assert_eq!(keys(&search(t, "\"metaphor conceptual\"")).len(), 0);
    assert_eq!(keys(&search(t, "concept*")).len(), 1);
    // A regex alone lists the passages it matches, from original text.
    let r = search(t, "/every metaphor/i");
    assert_eq!(keys(&r), vec!["stream/2025-07.md#1939901126326764018"]);
    assert_eq!(
        r.results[0].hits[0].quote,
        "Every metaphor breaks somewhere."
    );
    assert_eq!(
        keys(&search(t, "metaphor /FRAME/")).len(),
        0,
        "case-sensitive without i"
    );
    // Dates, genre and lang on tweets.
    assert_eq!(keys(&search(t, "metaphor y:2025")).len(), 2);
    assert_eq!(keys(&search(t, "metaphor genre:thread")).len(), 2);
    assert_eq!(keys(&search(t, "metaphor lang:cs")).len(), 0);
    assert!(t
        .search(&SearchRequest::new("metaphor after:soon"))
        .is_err());
    assert!(t.search(&SearchRequest::new("/(unclosed/")).is_err());
    // Negation alone: every passage without the word.
    let r = search(t, "-metaphor");
    assert!(r
        .results
        .iter()
        .all(|d| d.hits.iter().all(|p| !p.quote.contains("metaphor"))));
    assert_eq!(r.total_passages, 0, "the media tweet has no text passage");
}

// ---------- cross-corpus ----------

#[test]
fn cross_corpus_search_merges_deterministically() {
    let f = fixture();
    let both = vec![f.tweets.clone(), f.highlights.clone()];
    let req = SearchRequest::new("metaphor");
    let a = search_all(&both, &req).unwrap();
    let b = search_all(&[f.highlights.clone(), f.tweets.clone()], &req).unwrap();
    let json = |r: &SearchResults| {
        let mut v = serde_json::to_value(r).unwrap();
        v["corpora"] = serde_json::Value::Null;
        v.to_string()
    };
    assert_eq!(json(&a), json(&b), "corpus order does not change the merge");
    assert_eq!(a.total_documents, 7);
    // Each corpus's best document has rank 1, so both lead the merge.
    let firsts: std::collections::BTreeSet<&str> = a
        .results
        .iter()
        .take_while(|d| d.rank == 1.0)
        .map(|d| d.corpus.as_str())
        .collect();
    assert_eq!(
        firsts.into_iter().collect::<Vec<_>>(),
        vec!["highlights", "tweets"]
    );
    for w in a.results.windows(2) {
        assert!(w[0].rank >= w[1].rank);
    }
    // `in:` narrows the corpora searched.
    let r = search_all(&both, &SearchRequest::new("metaphor in:highlights")).unwrap();
    assert_eq!(r.corpora, vec!["highlights"]);
    assert!(r.results.iter().all(|d| d.corpus == "highlights"));
    assert!(search_all(&both, &SearchRequest::new("metaphor in:writing")).is_err());
}

#[test]
fn cite_takes_a_passage_id_from_any_view() {
    let f = fixture();
    let r = search(&f.tweets, "somewhere");
    let hit = &r.results[0].hits[0];
    let id: PassageId = hit.passage_id.parse().unwrap();
    assert_eq!(id.doc_key, "stream/2025-07.md#1939901126326764018");
    let c = f.tweets.cite(&id).unwrap();
    assert_eq!(
        c.quote,
        "A metaphor is a model. Every metaphor breaks somewhere."
    );
    assert_eq!(
        c.citation.markdown,
        "> A metaphor is a model. Every metaphor breaks somewhere.\n\n— Dominik Lukeš (@techczech), tweet, 1 July 2025 · [public](https://x.com/techczech/status/1939901126326764018)\n"
    );
    // A highlight id from KWIC.
    let k = f.highlights.kwic(&KwicRequest::new("inference")).unwrap();
    let id: PassageId = k.lines[0].passage_id.parse().unwrap();
    let c = f.highlights.cite(&id).unwrap();
    assert!(c
        .citation
        .markdown
        .starts_with("> The brain is an inference engine"));
    assert!(c.citation.markdown.contains(
        "saved 2021 · [public](https://astralcodexten.substack.com/p/towards) · [highlight](file:///"
    ));
    assert_eq!(
        c.public_url.as_deref(),
        Some("https://astralcodexten.substack.com/p/towards")
    );
    // An X post through the facade: the x.com link, never the post text.
    let k = f.highlights.kwic(&KwicRequest::new("frames")).unwrap();
    let id: PassageId = k.lines[0].passage_id.parse().unwrap();
    let c = f.highlights.cite(&id).unwrap();
    assert!(
        c.citation.plain.ends_with(
            "\n— @0xabi, post, 1 July 2025 · public: https://x.com/0xabi/status/1939901126326764018\n"
        ),
        "{}",
        c.citation.plain
    );
    // Unknown ids are typed errors.
    let bad: PassageId = "tweets:stream/2025-07.md#1:999".parse().unwrap();
    let e = f.tweets.cite(&bad).unwrap_err();
    assert!(e.downcast_ref::<scout_corpus::PassageNotFound>().is_some());
    assert!("nocolon".parse::<PassageId>().is_err());
}

// ---------- invariants ----------

fn unigram_count(c: &Corpus, word: &str, filter: &DocFilter) -> u64 {
    let req = NgramRequest {
        n_min: node_tokens(word).len(),
        n_max: node_tokens(word).len(),
        filter: filter.clone(),
        top: 1_000_000,
        containing: Some(node_tokens(word)),
        ..NgramRequest::default()
    };
    c.ngrams(&req)
        .unwrap()
        .grams
        .iter()
        .find(|g| g.gram == node_tokens(word).join(" "))
        .map(|g| g.count)
        .unwrap_or(0)
}

fn agreement(c: &Corpus, words: &[&str], filters: &[DocFilter]) -> usize {
    let mut nonzero = 0;
    for word in words {
        for f in filters {
            let mut req = KwicRequest::new(*word);
            req.filter = f.clone();
            req.limit = 2;
            let k = c.kwic(&req).unwrap();
            let p = c.profile(word, f).unwrap();
            assert_eq!(
                k.total as u64,
                unigram_count(c, word, f),
                "{word} {f:?}: kwic vs n-gram"
            );
            assert_eq!(k.total, p.frequency, "{word} {f:?}: kwic vs profile");
            for by in [DistBy::Year, DistBy::Corpus, DistBy::Genre, DistBy::Lang] {
                let d = c.distribution(word, by, f).unwrap();
                assert_eq!(d.buckets.iter().map(|b| b.hits).sum::<usize>(), k.total);
            }
            // Collocate rows count their concordance lines.
            for window in [1, 3] {
                let mut cr = CollocRequest::new(*word);
                cr.window = window;
                cr.min_freq = 1;
                cr.top = 1000;
                cr.filter = f.clone();
                for row in c.collocates(&cr).unwrap().rows {
                    let mut kr = KwicRequest::new(*word);
                    kr.filter = f.clone();
                    kr.limit = 0;
                    kr.near = Some(Near {
                        word: row.collocate.clone(),
                        window,
                    });
                    assert_eq!(
                        row.count as usize,
                        c.kwic(&kr).unwrap().total,
                        "{word} ~ {}",
                        row.collocate
                    );
                }
            }
            nonzero += (k.total > 0) as usize;
        }
    }
    nonzero
}

#[test]
fn counts_agree_across_views_for_tweets_and_highlights() {
    let f = fixture();
    let tweet_filters = [
        DocFilter::default(),
        DocFilter {
            year: Some(2025),
            ..DocFilter::default()
        },
        DocFilter {
            genre: vec!["thread".into()],
            ..DocFilter::default()
        },
    ];
    assert!(
        agreement(
            &f.tweets,
            &["metaphor", "frame", "conceptual metaphor", "breaks"],
            &tweet_filters
        ) >= 8
    );
    let hl_filters = [
        DocFilter::default(),
        DocFilter {
            source: vec!["readwise".into()],
            ..DocFilter::default()
        },
        DocFilter {
            before: Some("2022".into()),
            ..DocFilter::default()
        },
        DocFilter {
            kind: vec!["tweets".into()],
            ..DocFilter::default()
        },
    ];
    assert!(
        agreement(
            &f.highlights,
            &["metaphor", "engine", "frames", "metaphor frames"],
            &hl_filters
        ) >= 6
    );
}

#[test]
fn sources_are_read_only_and_rebuilds_are_identical() {
    let f = fixture();
    let (tb, hb) = (snapshot(&f.tweets_src), snapshot(&f.hl_src));
    let json = |c: &Corpus| {
        let s = serde_json::to_string(&search(c, "metaphor")).unwrap();
        let k = serde_json::to_string(&c.kwic(&KwicRequest::new("metaphor")).unwrap()).unwrap();
        (s, k)
    };
    let before = (json(&f.tweets), json(&f.highlights));
    let r = f.tweets.build_index(false).unwrap();
    assert_eq!(
        (r.full, r.indexed),
        (false, 0),
        "an unchanged rebuild is incremental"
    );
    f.tweets.build_index(true).unwrap();
    f.highlights.build_index(true).unwrap();
    assert_eq!(before, (json(&f.tweets), json(&f.highlights)));
    // An edited tweet file replaces all of its tweets.
    let p = f.tweets_src.join("stream/2025-07.md");
    fs::write(
        &p,
        STREAM.replace("Every metaphor breaks", "Every simile breaks"),
    )
    .unwrap();
    let r = f.tweets.build_index(false).unwrap();
    assert_eq!((r.indexed, r.docs), (1, 5));
    assert_eq!(
        f.tweets.kwic(&KwicRequest::new("metaphor")).unwrap().total,
        6
    );
    fs::write(&p, STREAM).unwrap();
    f.tweets.build_index(false).unwrap();
    assert_eq!(before.0, json(&f.tweets));
    assert_eq!(hb, snapshot(&f.hl_src));
    assert_eq!(tb, snapshot(&f.tweets_src));
}

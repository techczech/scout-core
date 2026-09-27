//! Seam tests for the corpus engine: normalisation, tokeniser, offset mapping,
//! frontmatter filtering, incremental build and deterministic output.

use scout_corpus::normalize::{compile_rules, normalize};
use scout_corpus::registry::{CorpusConfig, CorpusKind, FieldMap, WRITEFLEX_LINK};
use scout_corpus::search::{locate, parse_terms};
use scout_corpus::tokenize::{tokenize, words, IdentityLemmatizer};
use scout_corpus::{Corpus, SearchRequest};
use std::fs;
use std::path::Path;

fn cfg(root: &Path, require: &[&str]) -> CorpusConfig {
    CorpusConfig {
        id: "test".into(),
        name: "Test".into(),
        kind: CorpusKind::MarkdownFolder,
        path: root.display().to_string(),
        include: vec!["**/*.md".into()],
        exclude: vec!["cache/**".into()],
        require_frontmatter: require.iter().map(|s| s.to_string()).collect(),
        field_map: FieldMap {
            author: "authors".into(),
            ..FieldMap::default()
        },
        default_author: Some("Dominik Lukeš".into()),
        boilerplate: vec![r"^Originally published at ".into()],
        link: Some(WRITEFLEX_LINK.into()),
    }
}

fn write(root: &Path, rel: &str, body: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, body).unwrap();
}

// ---------- normalisation ----------

#[test]
fn entities_decode_including_double_encoded() {
    let m = normalize(
        "AT&amp;amp;T and &lt;b&gt; &#8217; &#x201C;x&#x201D; tea&nbsp;time",
        &[],
    );
    assert_eq!(m.text, "AT&T and   ' \"x\" tea time");
    let w = words(&m.text);
    assert!(!w.iter().any(|t| t == "amp"), "no amp tokens left: {w:?}");
}

#[test]
fn deeply_encoded_markup_decodes_and_strips() {
    let m = normalize("x [&amp;amp;amp;amp;lt;a href=\"u\"&amp;amp;amp;amp;gt;] y", &[]);
    assert_eq!(words(&m.text), vec!["x", "y"]);
}

#[test]
fn triple_encoded_amp_leaves_no_amp_token() {
    let m = normalize("rock &amp;amp;amp; roll", &[]);
    assert_eq!(words(&m.text), vec!["rock", "roll"]);
}

#[test]
fn curly_apostrophes_unify_and_dont_is_one_token() {
    let m = normalize("I don’t think ‘digital technology’ is “new”.", &[]);
    assert_eq!(m.text, "I don't think 'digital technology' is \"new\".");
    assert_eq!(
        words(&m.text),
        vec!["i", "don't", "think", "digital", "technology", "is", "new"]
    );
}

#[test]
fn tags_links_and_urls_are_stripped() {
    let m = normalize(
        "See <em>the</em> [Moodle](http://moodle.org) site at https://example.com/x?y=1 now",
        &[],
    );
    assert_eq!(
        words(&m.text),
        vec!["see", "the", "moodle", "site", "at", "now"]
    );
}

#[test]
fn boilerplate_lines_are_dropped() {
    let rules = compile_rules(&[r"^Originally published at ".to_string()]).unwrap();
    let m = normalize(
        "Real text here.\nOriginally published at medium.com on 2016.\nMore text.",
        &rules,
    );
    assert_eq!(words(&m.text), vec!["real", "text", "here", "more", "text"]);
}

#[test]
fn bad_boilerplate_regex_is_an_error() {
    assert!(compile_rules(&["(".to_string()]).is_err());
}

#[test]
fn czech_diacritics_are_single_tokens_and_nfc() {
    assert_eq!(
        words("Metafora je příklad řeči."),
        vec!["metafora", "je", "příklad", "řeči"]
    );
    // Decomposed input (NFD) normalises to the same single token.
    let nfd = "pr\u{0069}\u{0301}klad p\u{0072}\u{030C}i\u{0301}klad";
    let m = normalize(nfd, &[]);
    assert_eq!(words(&m.text), vec!["príklad", "příklad"]);
}

#[test]
fn tokeniser_trims_markdown_underscores_and_keeps_offsets() {
    let t = tokenize("an _emphatic_ word", &IdentityLemmatizer, None);
    assert_eq!(t[1].text, "emphatic");
    assert_eq!(&"an _emphatic_ word"[t[1].start..t[1].end], "emphatic");
    assert_eq!(t[1].lemma, "emphatic");
}

// ---------- offset mapping ----------

#[test]
fn every_token_maps_back_to_its_original_span() {
    let original =
        "Tom&amp;amp;Jerry’s [**bold** link](http://x.y/z) — &lt;i&gt;don’t&lt;/i&gt; příklad";
    let m = normalize(original, &[]);
    for t in tokenize(&m.text, &IdentityLemmatizer, None) {
        let (s, e) = m.original_range(t.start, t.end);
        let orig = &original[s..e];
        // The original span normalises back to the token (modulo case).
        assert_eq!(
            words(&normalize(orig, &[]).text).join(" "),
            t.text,
            "span {orig:?}"
        );
    }
}

#[test]
fn located_quote_is_an_original_substring() {
    let original = "First sentence here.\nThe **generative** [metaphors](http://x.org) of Schön’s work &amp; more. Last one.";
    let terms = parse_terms("generative metaphor");
    let loc = locate(original, &[], &terms, false);
    let quote = &original[loc.start..loc.end];
    assert_eq!(
        quote,
        "The **generative** [metaphors](http://x.org) of Schön’s work &amp; more."
    );
    assert!(original.contains(quote));
    assert_eq!(&original[loc.first_match..loc.first_match + 2], "ge");
}

#[test]
fn quoted_phrase_needs_consecutive_tokens() {
    let terms = parse_terms("\"generative metaphor\" the");
    assert_eq!(terms.len(), 1, "stopword dropped, phrase kept: {terms:?}");
    let original = "A generative, then metaphor. A generative metaphor here.";
    let loc = locate(original, &[], &terms, false);
    assert_eq!(&original[loc.start..loc.end], "A generative metaphor here.");
}

// ---------- corpus build + search ----------

const ESSAY: &str = "---\ntitle: \"Repaved paths and generative metaphors: Expressing purposes\"\ndate: 2016-06-23\ngenre: essay\nlang: en\ntopics: metaphor, technology\n---\n# Heading\n\nFirst paragraph about paths.\n\nOften, a poet will be led by rhythm. New analogies (Schön called these generative metaphors) help.\nA second line of the same paragraph.\n\n- a list item about metaphor\n- another item\n\nOriginally published at medium.com\n";

fn fixture(root: &Path) {
    write(root, "blogs/2016-essay.md", ESSAY);
    write(
        root,
        "blogs/copy-b.md",
        "---\ntitle: Twin\ngenre: note\n---\nIdentical metaphor passage.\n",
    );
    write(
        root,
        "blogs/copy-a.md",
        "---\ntitle: Twin\ngenre: note\n---\nIdentical metaphor passage.\n",
    );
    write(
        root,
        "notes/no-genre.md",
        "---\ntitle: Draft\n---\nA metaphor without genre.\n",
    );
    write(root, "notes/no-frontmatter.md", "Just a metaphor.\n");
    write(
        root,
        "notes/empty-genre.md",
        "---\ngenre: \"\"\n---\nA metaphor with empty genre.\n",
    );
    write(
        root,
        "cache/skip.md",
        "---\ngenre: essay\n---\nmetaphor in cache\n",
    );
    write(
        root,
        ".worktrees/x/hidden.md",
        "---\ngenre: essay\n---\nmetaphor hidden\n",
    );
}

fn snapshot(root: &Path) -> Vec<(String, u64, std::time::SystemTime)> {
    let mut v: Vec<_> = walkdir(root);
    v.sort();
    v
}

fn walkdir(root: &Path) -> Vec<(String, u64, std::time::SystemTime)> {
    let mut out = Vec::new();
    for e in fs::read_dir(root).unwrap() {
        let e = e.unwrap();
        let p = e.path();
        if p.is_dir() {
            out.extend(walkdir(&p));
        } else {
            let md = e.metadata().unwrap();
            out.push((p.display().to_string(), md.len(), md.modified().unwrap()));
        }
    }
    out
}

#[test]
fn require_frontmatter_filters_documents() {
    let src = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    fixture(src.path());
    let c = Corpus::from_config(cfg(src.path(), &["genre"]), data.path());
    let r = c.build_index(false).unwrap();
    assert!(r.full);
    assert_eq!(r.files_seen, 6, "cache/ excluded, hidden dirs skipped");
    assert_eq!(r.docs, 3);
    assert_eq!(r.not_documents, 3);

    let all = Corpus::from_config(
        CorpusConfig {
            id: "all".into(),
            ..cfg(src.path(), &[])
        },
        data.path(),
    );
    assert_eq!(all.build_index(false).unwrap().docs, 6);
}

#[test]
fn search_cites_original_text_with_link_and_british_date() {
    let src = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    fixture(src.path());
    let before = snapshot(src.path());
    let c = Corpus::from_config(cfg(src.path(), &["genre"]), data.path());
    c.build_index(false).unwrap();
    let res = c
        .search(&SearchRequest::new("generative metaphor"))
        .unwrap();
    assert_eq!(res.results[0].rel_path, "blogs/2016-essay.md");
    let d = &res.results[0];
    assert!(d.title_match);
    let hit = &d.hits[0];
    assert_eq!(
        hit.quote,
        "New analogies (Schön called these generative metaphors) help."
    );
    assert_eq!(hit.line, 12);
    assert!(scout_corpus::search::quote_is_original(&src.path().join(&d.rel_path), hit).unwrap());
    let link = hit.link.as_deref().unwrap();
    assert!(link.starts_with("writeflex://open?path=%2F"), "{link}");
    assert!(link.ends_with("2016-essay.md&line=12"), "{link}");
    assert!(d.citation.markdown.contains(
        "— Dominik Lukeš, *Repaved paths and generative metaphors: Expressing purposes*, 23 June 2016\n"
    ));
    assert!(d.citation.markdown.starts_with("> New analogies"));
    assert!(d.citation.plain.contains("— Dominik Lukeš, Repaved paths and generative metaphors: Expressing purposes, 23 June 2016\n"));
    // Boilerplate never matches.
    assert!(c
        .search(&SearchRequest::new("originally published"))
        .unwrap()
        .results
        .is_empty());
    // Sources untouched and the index is outside them.
    assert_eq!(before, snapshot(src.path()));
    assert!(!c.index_path().starts_with(src.path()));
}

#[test]
fn passages_split_on_headings_and_list_items() {
    let src = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    fixture(src.path());
    let c = Corpus::from_config(cfg(src.path(), &["genre"]), data.path());
    c.build_index(false).unwrap();
    let mut req = SearchRequest::new("metaphor");
    req.hits_per_doc = 10;
    req.whole_passage = true;
    let res = c.search(&req).unwrap();
    let essay = res
        .results
        .iter()
        .find(|d| d.rel_path == "blogs/2016-essay.md")
        .unwrap();
    let mut lines: Vec<(usize, usize)> = essay
        .hits
        .iter()
        .map(|h| (h.line_start, h.line_end))
        .collect();
    lines.sort();
    assert_eq!(lines, vec![(12, 13), (15, 15)]);
    let item = essay.hits.iter().find(|h| h.line_start == 15).unwrap();
    assert_eq!(item.quote, "a list item about metaphor");
}

#[test]
fn ordering_is_deterministic_with_path_tiebreak() {
    let src = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    fixture(src.path());
    let c = Corpus::from_config(cfg(src.path(), &["genre"]), data.path());
    c.build_index(false).unwrap();
    let res = c.search(&SearchRequest::new("identical")).unwrap();
    let paths: Vec<&str> = res.results.iter().map(|d| d.rel_path.as_str()).collect();
    assert_eq!(paths, vec!["blogs/copy-a.md", "blogs/copy-b.md"]);
    assert_eq!(res.results[0].score, res.results[1].score);
}

fn json_of(c: &Corpus, q: &str) -> String {
    serde_json::to_string(&c.search(&SearchRequest::new(q)).unwrap()).unwrap()
}

#[test]
fn rebuild_and_incremental_updates_give_identical_results() {
    let src = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    fixture(src.path());
    let c = Corpus::from_config(cfg(src.path(), &["genre"]), data.path());
    c.build_index(false).unwrap();
    let queries = ["generative metaphor", "metaphor", "\"list item\""];
    let first: Vec<String> = queries.iter().map(|q| json_of(&c, q)).collect();

    // No-change incremental build touches nothing.
    let r = c.build_index(false).unwrap();
    assert!(!r.full);
    assert_eq!((r.indexed, r.removed), (0, 0));
    assert_eq!(c.status().unwrap().stale.unwrap().changed, 0);

    // Edit, add, delete, then revert: results return to the original.
    let essay = src.path().join("blogs/2016-essay.md");
    fs::write(&essay, ESSAY.replace("generative", "creative")).unwrap();
    write(
        src.path(),
        "blogs/new.md",
        "---\ngenre: essay\n---\nnew metaphor\n",
    );
    fs::remove_file(src.path().join("blogs/copy-b.md")).unwrap();
    let st = c.status().unwrap().stale.unwrap();
    assert_eq!((st.added, st.removed), (1, 1));
    let r = c.build_index(false).unwrap();
    assert_eq!((r.indexed, r.removed), (2, 1));
    assert_ne!(json_of(&c, "generative metaphor"), first[0]);
    fs::write(&essay, ESSAY).unwrap();
    fs::remove_file(src.path().join("blogs/new.md")).unwrap();
    write(
        src.path(),
        "blogs/copy-b.md",
        "---\ntitle: Twin\ngenre: note\n---\nIdentical metaphor passage.\n",
    );
    c.build_index(false).unwrap();
    let incremental: Vec<String> = queries.iter().map(|q| json_of(&c, q)).collect();
    assert_eq!(first, incremental);

    // A forced rebuild from scratch gives byte-identical JSON.
    assert!(c.build_index(true).unwrap().full);
    let rebuilt: Vec<String> = queries.iter().map(|q| json_of(&c, q)).collect();
    assert_eq!(first, rebuilt);
}

#[test]
fn config_change_forces_full_rebuild() {
    let src = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    fixture(src.path());
    Corpus::from_config(cfg(src.path(), &["genre"]), data.path())
        .build_index(false)
        .unwrap();
    let mut changed = cfg(src.path(), &["genre"]);
    changed.boilerplate.push("^First paragraph".into());
    let c = Corpus::from_config(changed, data.path());
    assert!(!c.status().unwrap().config_current);
    assert!(c.build_index(false).unwrap().full);
}

#[test]
fn missing_index_is_a_typed_error() {
    let src = tempfile::tempdir().unwrap();
    let data = tempfile::tempdir().unwrap();
    let c = Corpus::from_config(cfg(src.path(), &[]), data.path());
    let e = c.search(&SearchRequest::new("x")).unwrap_err();
    assert!(e.downcast_ref::<scout_corpus::IndexMissing>().is_some());
}

#[test]
fn index_inside_source_is_refused() {
    let src = tempfile::tempdir().unwrap();
    fixture(src.path());
    let c = Corpus::from_config(cfg(src.path(), &[]), &src.path().join("idx"));
    assert!(c.build_index(false).is_err());
    assert!(!src.path().join("idx").exists());
}

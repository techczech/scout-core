//! `scout`: argument parsing and output formatting over `scout-corpus`.
//! No engine logic lives here (invariant 5): every command is one call into
//! `scout_corpus::api::Engine`, and `--json` prints that call's response.
//!
//! Exit codes: 0 ok, 1 no results, 2 usage, 3 index or registry missing.

mod views;

use anyhow::Result;
use clap::{Args, Parser, Subcommand, ValueEnum};
use scout_corpus::api::{self, *};
use scout_corpus::SearchMode;
use scout_corpus::{
    Engine, IndexMissing, NoIndexedCorpus, Outcome, PassageNotFound, RegistryMissing, Reply,
};
use std::io::IsTerminal;
use std::process::ExitCode;
use std::sync::Arc;

/// The embedder: the in-process ONNX model (loaded on first use), or the
/// deterministic hash fake when `SCOUT_EMBEDDER=hash` (tests; no model).
fn embedder() -> Result<Arc<dyn scout_corpus::Embedder>> {
    if std::env::var("SCOUT_EMBEDDER").as_deref() == Ok("hash") {
        return Ok(Arc::new(scout_corpus::HashEmbedder::default()));
    }
    Ok(Arc::new(
        scout_corpus::semantic::embed::OnnxEmbedder::from_env()?,
    ))
}

#[derive(Parser)]
#[command(
    name = "scout",
    version,
    about = "Search and cite across your registered corpora of writing and highlights"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Manage the corpus registry (~/.config/scout/corpora.toml).
    Corpora {
        #[command(subcommand)]
        cmd: CorporaCmd,
    },
    /// Build or inspect the per-corpus indexes.
    Index {
        #[command(subcommand)]
        cmd: IndexCmd,
    },
    /// Full-text search; hits are passages grouped by document.
    Search {
        /// The @scout/query grammar: words are prefix-matched and ANDed;
        /// "quoted phrases" match exactly; OR, -exclude, prefix*, /regex/i;
        /// fields in: au: ti: ty: tag: co: after: before: y: lang: genre: source:.
        query: Vec<String>,
        /// Comma-separated corpus ids (default: every indexable corpus).
        #[arg(long = "in", value_delimiter = ',')]
        in_: Vec<String>,
        #[arg(long, default_value_t = SearchQuery::default().limit)]
        limit: usize,
        /// Print a citation per document.
        #[arg(long, value_enum)]
        cite: Option<CiteArg>,
        /// Quote the whole paragraph instead of the hit sentence.
        #[arg(long)]
        passage: bool,
        /// Rank by meaning only (cosine to the query; needs `index build --semantic`).
        #[arg(long, conflicts_with_all = ["hybrid", "fts"])]
        semantic: bool,
        /// Full-text and semantic rankings fused (the default where vectors exist).
        #[arg(long, conflicts_with = "fts")]
        hybrid: bool,
        /// Full text only, even where vectors exist.
        #[arg(long)]
        fts: bool,
        #[arg(long)]
        json: bool,
    },
    /// Key word in context: every occurrence of a word or phrase.
    Kwic {
        /// A word or a phrase (consecutive tokens); exact tokens, no prefix.
        term: Vec<String>,
        #[command(flatten)]
        scope: Scope,
        /// Context tokens either side.
        #[arg(long, default_value_t = KwicQuery::default().width)]
        width: usize,
        /// L1..L9, R1..R9, date or source; ties break by source path, line.
        #[arg(long, default_value_t = KwicQuery::default().sort)]
        sort: String,
        #[arg(long, default_value_t = KwicQuery::default().limit)]
        limit: usize,
        /// Only lines with this word within --window tokens of the node (a
        /// collocate row's concordance).
        #[arg(long)]
        near: Option<String>,
        /// The --near window, tokens either side.
        #[arg(long, default_value_t = KwicQuery::default().window)]
        window: usize,
        #[arg(long)]
        json: bool,
    },
    /// Collocates: words within --window tokens of the node, by logDice or MI.
    Collocates {
        node: Vec<String>,
        #[command(flatten)]
        scope: Scope,
        #[arg(long, default_value_t = CollocatesQuery::default().window)]
        window: usize,
        /// logdice or mi.
        #[arg(long, default_value_t = CollocatesQuery::default().score)]
        score: String,
        /// Minimum co-occurrence count f(n,c).
        #[arg(long, default_value_t = CollocatesQuery::default().min)]
        min: u64,
        #[arg(long, default_value_t = CollocatesQuery::default().top)]
        top: usize,
        /// List stopwords as collocates too (default: dropped).
        #[arg(long)]
        keep_stopwords: bool,
        /// A second slice, side by side: a corpus id and/or filter terms,
        /// e.g. "before:2015", "y:2010-2015", "highlights", "in:writing genre:note".
        #[arg(long)]
        compare: Option<String>,
        /// Per-row distribution: year, doc or corpus.
        #[arg(long)]
        dist: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Keyness: words that set slice A apart from slice B (log-likelihood).
    Keyness {
        /// Slice A, e.g. "before:2015" or "in:writing genre:essay".
        #[arg(long)]
        a: String,
        /// Slice B.
        #[arg(long)]
        b: String,
        /// Corpora for a slice that names none (default: every indexed corpus).
        #[arg(long = "in", value_delimiter = ',')]
        in_: Vec<String>,
        #[arg(long, default_value_t = KeynessQuery::default().top)]
        top: usize,
        /// Minimum count in the slice where the word is key.
        #[arg(long, default_value_t = KeynessQuery::default().min)]
        min: u64,
        #[arg(long)]
        json: bool,
    },
    /// Hit counts of a word or phrase by year, corpus, genre or lang.
    Dist {
        term: Vec<String>,
        #[arg(long)]
        by: String,
        #[command(flatten)]
        scope: Scope,
        #[arg(long)]
        json: bool,
    },
    /// Frequent n-grams within passages.
    Ngrams {
        #[command(flatten)]
        scope: Scope,
        /// A range such as 3-5, or one n.
        #[arg(long, default_value_t = NgramsQuery::default().n)]
        n: String,
        /// First year (inclusive); adds per-year counts.
        #[arg(long)]
        since: Option<i32>,
        /// Last year (inclusive); adds per-year counts.
        #[arg(long)]
        until: Option<i32>,
        #[arg(long, default_value_t = NgramsQuery::default().top)]
        top: usize,
        /// Drop grams with at least n-1 stopwords (default: only all-stopword grams).
        #[arg(long)]
        strict_stopwords: bool,
        /// Per-row distribution: year, doc or corpus.
        #[arg(long)]
        dist: Option<String>,
        /// Only grams containing this word or phrase.
        #[arg(long)]
        containing: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Word profile: frequency, first use, per-year, top documents, collocates, n-grams.
    Profile {
        word: Vec<String>,
        #[command(flatten)]
        scope: Scope,
        /// A top document needs at least this many hits.
        #[arg(long, default_value_t = ProfileQuery::default().min_hits)]
        min_hits: usize,
        #[arg(long)]
        json: bool,
    },
    /// Cite one passage by its id (the passage_id of search, kwic, verify-quote).
    Cite {
        /// <corpus>:<path>:<line>, e.g. writing:blogs/2016-essay.md:12
        passage_id: String,
        #[arg(long, value_enum, default_value = "markdown")]
        format: CiteArg,
        #[arg(long)]
        json: bool,
    },
    /// Per normalisation rule: passages changed, with before/after samples.
    CleanReport {
        corpus: String,
        /// Samples per rule.
        #[arg(long, default_value_t = CleanReportQuery::default().samples)]
        samples: usize,
        /// Preview a candidate boilerplate regex (repeatable); not saved.
        #[arg(long = "rule")]
        rules: Vec<String>,
        #[arg(long)]
        json: bool,
    },
    /// More like these: passages ranked by tf-idf cosine to 1–10 seed passages.
    Similar {
        /// Seed passage ids, <corpus>:<path>:<line> (the passage_id of search, kwic, cite).
        #[arg(required = true, num_args = 1..=10)]
        seeds: Vec<String>,
        /// Comma-separated corpus ids to search (default: every indexable corpus).
        #[arg(long = "in", value_delimiter = ',')]
        in_: Vec<String>,
        #[arg(long, default_value_t = SimilarQuery::default().top)]
        top: usize,
        /// Leave the seeds themselves out of the results.
        #[arg(long)]
        exclude_seeds: bool,
        #[arg(long)]
        json: bool,
    },
    /// Is this exact text in a source? (Only quote and apostrophe forms are unified.)
    VerifyQuote {
        text: Vec<String>,
        #[arg(long = "in", value_delimiter = ',')]
        in_: Vec<String>,
        #[arg(long)]
        json: bool,
    },
}

/// Corpus selection and document filters shared by the analytics views.
#[derive(Args, Clone, Default)]
pub struct Scope {
    /// Comma-separated corpus ids (default: every indexable corpus).
    #[arg(long = "in", value_delimiter = ',')]
    pub in_: Vec<String>,
    /// Language code(s), comma-separated.
    #[arg(long, value_delimiter = ',')]
    pub lang: Vec<String>,
    /// Genre(s), comma-separated.
    #[arg(long, value_delimiter = ',')]
    pub genre: Vec<String>,
    /// From this date, inclusive (YYYY, YYYY-MM or YYYY-MM-DD).
    #[arg(long)]
    pub after: Option<String>,
    /// Before this date, exclusive (YYYY, YYYY-MM or YYYY-MM-DD).
    #[arg(long)]
    pub before: Option<String>,
    /// Only this year.
    #[arg(long = "y", alias = "year")]
    pub year: Option<i32>,
    /// Source system(s), comma-separated: readwise, x, zotero.
    #[arg(long, value_delimiter = ',')]
    pub source: Vec<String>,
}

impl Scope {
    fn api(self) -> api::Scope {
        api::Scope {
            in_: self.in_,
            lang: self.lang,
            genre: self.genre,
            after: self.after,
            before: self.before,
            year: self.year,
            source: self.source,
        }
    }
}

#[derive(Subcommand)]
enum CorporaCmd {
    /// List registered corpora.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Register a folder as a corpus (creates the registry when missing).
    /// Does not build its index: run `scout index build <id>` next.
    Add {
        /// The folder (`~` is expanded).
        path: String,
        /// Corpus id (default: a slug of the folder name).
        #[arg(long)]
        id: Option<String>,
        /// Display name (default: the folder name).
        #[arg(long)]
        name: Option<String>,
        /// Default: markdown-folder. A Highlight Scout archive must be named
        /// (`--kind highlight-scout-archive`); its folder needs readings/works/.
        #[arg(long, value_enum)]
        kind: Option<KindArg>,
        /// Author of documents that name none.
        #[arg(long)]
        author: Option<String>,
        /// Files to index, repeatable (default for markdown-folder: **/*.md, **/*.txt).
        #[arg(long)]
        include: Vec<String>,
        /// Files to skip, repeatable.
        #[arg(long)]
        exclude: Vec<String>,
        #[arg(long)]
        json: bool,
    },
    /// Unregister a corpus and delete its index. The folder is left alone.
    Remove {
        id: String,
        /// Keep the corpus's index files.
        #[arg(long)]
        keep_index: bool,
        #[arg(long)]
        json: bool,
    },
    /// Write an empty registry that explains `scout corpora add`.
    InitDefaults {
        /// Overwrite an existing registry.
        #[arg(long)]
        force: bool,
        // `--preset dominik` writes the maintainer's own three corpora
        // (registry::Preset::Maintainer). Hidden on purpose: not in --help,
        // not in any error message.
        #[arg(long, hide = true)]
        preset: Option<String>,
    },
    /// Show one corpus entry and its index location.
    Show { id: String },
}

#[derive(Clone, Copy, ValueEnum)]
enum KindArg {
    MarkdownFolder,
    HighlightScoutArchive,
}

impl From<KindArg> for scout_corpus::CorpusKind {
    fn from(k: KindArg) -> Self {
        match k {
            KindArg::MarkdownFolder => scout_corpus::CorpusKind::MarkdownFolder,
            KindArg::HighlightScoutArchive => scout_corpus::CorpusKind::HighlightScoutArchive,
        }
    }
}

#[derive(Subcommand)]
enum IndexCmd {
    /// Build indexes incrementally (mtime + hash).
    Build {
        /// Corpus ids (default: every indexable corpus).
        ids: Vec<String>,
        /// Delete the index and rebuild from scratch.
        #[arg(long)]
        force: bool,
        /// Also build passage vectors for semantic search (minutes; the
        /// model is downloaded once). Existing vectors update on every build.
        #[arg(long)]
        semantic: bool,
        #[arg(long)]
        json: bool,
    },
    /// Per corpus: docs, passages, tokens, built-at, stale files.
    Status {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum CiteArg {
    Markdown,
    Plain,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("scout: {e:#}");
            if e.downcast_ref::<PassageNotFound>().is_some() {
                return ExitCode::from(1);
            }
            if e.downcast_ref::<IndexMissing>().is_some()
                || e.downcast_ref::<RegistryMissing>().is_some()
                || e.downcast_ref::<NoIndexedCorpus>().is_some()
            {
                ExitCode::from(3)
            } else {
                ExitCode::from(2)
            }
        }
    }
}

/// Print the notes, then the JSON body or the human view; exit 0 with
/// results, 1 without.
fn out<T: serde::Serialize + Outcome>(
    reply: Result<Reply<T>>,
    json: bool,
    human: impl FnOnce(&T),
) -> Result<ExitCode> {
    let reply = reply?;
    for n in &reply.notes {
        eprintln!("{n}");
    }
    if json {
        println!("{}", api::to_json(&reply.body)?);
    } else {
        human(&reply.body);
    }
    Ok(views::code(reply.body.has_results()))
}

fn run(cli: Cli) -> Result<ExitCode> {
    // Registry writes run before the engine loads: the registry may not
    // exist yet.
    let cli = match cli.cmd {
        Cmd::Corpora { cmd } => match cmd {
            CorporaCmd::InitDefaults { force, preset } => {
                let preset = match preset.as_deref() {
                    None => scout_corpus::Preset::Empty,
                    Some("dominik") => scout_corpus::Preset::Maintainer,
                    Some(other) => anyhow::bail!("unknown preset {other:?}"),
                };
                let r = api::init_defaults(force, preset)?;
                views::print_init_defaults(&r);
                return Ok(ExitCode::SUCCESS);
            }
            CorporaCmd::Add {
                path,
                id,
                name,
                kind,
                author,
                include,
                exclude,
                json,
            } => {
                let cfg = api::add_corpus(AddCorpus {
                    path,
                    id,
                    name,
                    kind: kind.map(Into::into),
                    author,
                    include,
                    exclude,
                })?;
                if json {
                    println!("{}", api::to_json(&cfg)?);
                } else {
                    views::print_added(&cfg);
                }
                return Ok(ExitCode::SUCCESS);
            }
            CorporaCmd::Remove {
                id,
                keep_index,
                json,
            } => {
                let r = api::remove_corpus(&id, keep_index)?;
                if json {
                    println!("{}", api::to_json(&r)?);
                } else {
                    views::print_removed(&r);
                }
                return Ok(ExitCode::SUCCESS);
            }
            cmd => Cli {
                cmd: Cmd::Corpora { cmd },
            },
        },
        cmd => Cli { cmd },
    };
    let engine = Engine::from_env()?.with_embedder(embedder()?);
    match cli.cmd {
        Cmd::Search {
            query,
            in_,
            limit,
            cite,
            passage,
            semantic,
            hybrid,
            fts,
            json,
        } => {
            let mode = match (semantic, hybrid, fts) {
                (true, _, _) => SearchMode::Semantic,
                (_, true, _) => SearchMode::Hybrid,
                (_, _, true) => SearchMode::Fts,
                _ => SearchMode::Auto,
            };
            let q = SearchQuery {
                query: query.join(" "),
                in_,
                limit,
                passage,
                mode,
            };
            out(engine.search(&q), json, |res| match cite {
                Some(fmt) => {
                    for (i, d) in res.results.iter().enumerate() {
                        if i > 0 {
                            println!();
                        }
                        print!("{}", fmt.pick(&d.citation));
                    }
                }
                None => views::print_hits(res),
            })
        }
        Cmd::Kwic {
            term,
            scope,
            width,
            sort,
            limit,
            near,
            window,
            json,
        } => {
            let q = KwicQuery {
                term: term.join(" "),
                scope: scope.api(),
                width,
                sort,
                limit,
                near,
                window,
            };
            out(engine.kwic(&q), json, views::print_kwic)
        }
        Cmd::Collocates {
            node,
            scope,
            window,
            score,
            min,
            top,
            keep_stopwords,
            compare,
            dist,
            json,
        } => {
            let q = CollocatesQuery {
                node: node.join(" "),
                scope: scope.api(),
                window,
                score,
                min,
                top,
                keep_stopwords,
                compare,
                dist,
            };
            out(engine.collocates(&q), json, |r| match r {
                Collocates::Single(r) => views::print_collocates(r),
                Collocates::Compare(r) => views::print_compare(r),
            })
        }
        Cmd::Keyness {
            a,
            b,
            in_,
            top,
            min,
            json,
        } => {
            let q = KeynessQuery {
                a,
                b,
                in_,
                top,
                min,
            };
            out(engine.keyness(&q), json, views::print_keyness)
        }
        Cmd::Dist {
            term,
            by,
            scope,
            json,
        } => {
            let q = DistQuery {
                term: term.join(" "),
                by,
                scope: scope.api(),
            };
            out(engine.dist(&q), json, views::print_dist)
        }
        Cmd::Ngrams {
            scope,
            n,
            since,
            until,
            top,
            strict_stopwords,
            dist,
            containing,
            json,
        } => {
            let q = NgramsQuery {
                scope: scope.api(),
                n,
                since,
                until,
                top,
                strict_stopwords,
                dist,
                containing,
            };
            out(engine.ngrams(&q), json, views::print_ngrams)
        }
        Cmd::Profile {
            word,
            scope,
            min_hits,
            json,
        } => {
            let q = ProfileQuery {
                word: word.join(" "),
                scope: scope.api(),
                min_hits,
            };
            out(engine.profile(&q), json, views::print_profile)
        }
        Cmd::VerifyQuote { text, in_, json } => {
            let q = VerifyQuoteQuery {
                text: text.join(" "),
                in_,
            };
            out(engine.verify_quote(&q), json, views::print_verify)
        }
        Cmd::Similar {
            seeds,
            in_,
            top,
            exclude_seeds,
            json,
        } => {
            let q = SimilarQuery {
                seeds,
                in_,
                top,
                exclude_seeds,
            };
            out(engine.similar(&q), json, views::print_similar)
        }
        Cmd::Cite {
            passage_id,
            format,
            json,
        } => {
            let q = CiteQuery { passage_id };
            out(engine.cite(&q), json, |r| {
                print!("{}", format.pick(&r.citation))
            })
        }
        Cmd::CleanReport {
            corpus,
            samples,
            rules,
            json,
        } => {
            let q = CleanReportQuery {
                corpus,
                samples,
                rules,
            };
            out(engine.clean_report(&q), json, views::print_clean_report)
        }
        Cmd::Index { cmd } => match cmd {
            IndexCmd::Build {
                ids,
                force,
                semantic,
                json,
            } => {
                let q = IndexBuildQuery {
                    ids,
                    force,
                    semantic,
                };
                let tty = std::io::stderr().is_terminal();
                let mut on = |ev: BuildEvent| match ev {
                    BuildEvent::Download(note) => eprintln!("{note}"),
                    BuildEvent::Embedding {
                        corpus,
                        done,
                        total,
                    } if tty && total > 0 => {
                        eprint!("\rscout: embedding {corpus}: {done}/{total} passages");
                        if done == total {
                            eprintln!();
                        }
                    }
                    BuildEvent::Embedding { .. } => {}
                };
                out(
                    engine.index_build_with(&q, &mut on),
                    json,
                    views::print_build,
                )
            }
            IndexCmd::Status { json } => out(engine.index_status(), json, views::print_status),
        },
        Cmd::Corpora { cmd } => match cmd {
            CorporaCmd::List { json } => out(engine.corpora_list(), json, views::print_corpora),
            CorporaCmd::Show { id } => {
                let r = engine.corpus_show(&id)?;
                views::print_corpus_show(&r.body);
                Ok(ExitCode::SUCCESS)
            }
            CorporaCmd::InitDefaults { .. }
            | CorporaCmd::Add { .. }
            | CorporaCmd::Remove { .. } => {
                unreachable!("handled above")
            }
        },
    }
}

impl CiteArg {
    fn pick(self, c: &scout_corpus::search::Citation) -> &str {
        match self {
            CiteArg::Markdown => &c.markdown,
            CiteArg::Plain => &c.plain,
        }
    }
}

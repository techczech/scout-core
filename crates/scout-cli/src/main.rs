//! `scout`: argument parsing and output formatting over `scout-corpus`.
//! No engine logic lives here (invariant 5).
//!
//! Exit codes: 0 ok, 1 no results, 2 usage, 3 index or registry missing.

mod views;

use anyhow::{anyhow, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use scout_corpus::{
    index, registry, search_all, Corpus, CorpusKind, DocFilter, IndexMissing, Registry,
    RegistryMissing, SearchRequest,
};
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "scout",
    version,
    about = "Search and cite across Dominik's writing, tweets and highlights"
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
        /// Words are prefix-matched and ANDed; "quoted phrases" match exactly.
        query: Vec<String>,
        /// Comma-separated corpus ids (default: every indexable corpus).
        #[arg(long = "in", value_delimiter = ',')]
        in_: Vec<String>,
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Print a citation per document.
        #[arg(long, value_enum)]
        cite: Option<CiteArg>,
        /// Quote the whole paragraph instead of the hit sentence.
        #[arg(long)]
        passage: bool,
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
        #[arg(long, default_value_t = 8)]
        width: usize,
        /// L1..L9, R1..R9, date or source; ties break by source path, line.
        #[arg(long, default_value = "R1")]
        sort: String,
        #[arg(long, default_value_t = 200)]
        limit: usize,
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
        #[arg(long, default_value = "3-5")]
        n: String,
        /// First year (inclusive); adds per-year counts.
        #[arg(long)]
        since: Option<i32>,
        /// Last year (inclusive); adds per-year counts.
        #[arg(long)]
        until: Option<i32>,
        #[arg(long, default_value_t = 50)]
        top: usize,
        /// Drop grams with at least n-1 stopwords (default: only all-stopword grams).
        #[arg(long)]
        strict_stopwords: bool,
        #[arg(long)]
        json: bool,
    },
    /// Word profile: frequency, per million, pieces, first use, per-year.
    Profile {
        word: Vec<String>,
        #[command(flatten)]
        scope: Scope,
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
}

impl Scope {
    fn filter(&self) -> DocFilter {
        DocFilter {
            lang: self.lang.clone(),
            genre: self.genre.clone(),
            after: self.after.clone(),
            before: self.before.clone(),
            year: self.year,
        }
    }
}

#[derive(Subcommand)]
enum CorporaCmd {
    /// List registered corpora.
    List,
    /// Write the default registry (writing, tweets, highlights).
    InitDefaults {
        /// Overwrite an existing registry.
        #[arg(long)]
        force: bool,
    },
    /// Show one corpus entry and its index location.
    Show { id: String },
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
    },
    /// Per corpus: docs, passages, tokens, built-at, stale files.
    Status,
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
            if e.downcast_ref::<IndexMissing>().is_some()
                || e.downcast_ref::<RegistryMissing>().is_some()
            {
                ExitCode::from(3)
            } else {
                ExitCode::from(2)
            }
        }
    }
}

fn indexable(reg: &Registry) -> Vec<&scout_corpus::CorpusConfig> {
    reg.corpora
        .iter()
        .filter(|c| c.kind == CorpusKind::MarkdownFolder)
        .collect()
}

fn select(reg: &Registry, ids: &[String]) -> Result<Vec<Corpus>> {
    let dir = index::index_dir();
    if ids.is_empty() {
        return Ok(indexable(reg)
            .into_iter()
            .map(|c| Corpus::from_config(c.clone(), &dir))
            .collect());
    }
    ids.iter()
        .map(|id| Ok(Corpus::from_config(reg.get(id)?.clone(), &dir)))
        .collect()
}

fn run(cli: Cli) -> Result<ExitCode> {
    match cli.cmd {
        Cmd::Kwic {
            term,
            scope,
            width,
            sort,
            limit,
            json,
        } => {
            let reg = Registry::load()?;
            let corpora = select(&reg, &scope.in_)?;
            let mut req = scout_corpus::KwicRequest::new(term.join(" "));
            req.width = width;
            req.sort = sort.parse()?;
            req.limit = limit;
            req.filter = scope.filter();
            let res = scout_corpus::concord::kwic(&corpora, &req)?;
            views::emit(json, &res, || views::print_kwic(&res))?;
            Ok(views::code(res.total > 0))
        }
        Cmd::Dist {
            term,
            by,
            scope,
            json,
        } => {
            let reg = Registry::load()?;
            let corpora = select(&reg, &scope.in_)?;
            let res = scout_corpus::concord::distribution(
                &corpora,
                &term.join(" "),
                by.parse()?,
                &scope.filter(),
            )?;
            views::emit(json, &res, || views::print_dist(&res))?;
            Ok(views::code(res.total > 0))
        }
        Cmd::Ngrams {
            scope,
            n,
            since,
            until,
            top,
            strict_stopwords,
            json,
        } => {
            let reg = Registry::load()?;
            let corpora = select(&reg, &scope.in_)?;
            let (n_min, n_max) = scout_corpus::ngrams::parse_n_range(&n)?;
            let req = scout_corpus::NgramRequest {
                n_min,
                n_max,
                filter: scope.filter(),
                since,
                until,
                top,
                strict_stopwords,
                containing: None,
            };
            let res = scout_corpus::ngrams::ngrams(&corpora, &req)?;
            views::emit(json, &res, || views::print_ngrams(&res))?;
            Ok(views::code(!res.grams.is_empty()))
        }
        Cmd::Profile { word, scope, json } => {
            let reg = Registry::load()?;
            let corpora = select(&reg, &scope.in_)?;
            let res = scout_corpus::concord::profile(&corpora, &word.join(" "), &scope.filter())?;
            views::emit(json, &res, || views::print_profile(&res))?;
            Ok(views::code(res.frequency > 0))
        }
        Cmd::VerifyQuote { text, in_, json } => {
            let reg = Registry::load()?;
            let corpora = select(&reg, &in_)?;
            let res = scout_corpus::verify::verify_quote(&corpora, &text.join(" "))?;
            views::emit(json, &res, || views::print_verify(&res))?;
            Ok(views::code(res.found))
        }
        Cmd::Corpora { cmd } => corpora(cmd),
        Cmd::Index { cmd } => index_cmd(cmd),
        Cmd::Search {
            query,
            in_,
            limit,
            cite,
            passage,
            json,
        } => {
            let query = query.join(" ");
            if query.trim().is_empty() {
                return Err(anyhow!("empty query"));
            }
            let reg = Registry::load()?;
            let corpora = select(&reg, &in_)?;
            let mut req = SearchRequest::new(query);
            req.limit = limit;
            req.whole_passage = passage;
            let res = search_all(&corpora, &req)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&res)?);
            } else if let Some(fmt) = cite {
                for (i, d) in res.results.iter().enumerate() {
                    if i > 0 {
                        println!();
                    }
                    match fmt {
                        CiteArg::Markdown => print!("{}", d.citation.markdown),
                        CiteArg::Plain => print!("{}", d.citation.plain),
                    }
                }
            } else {
                print_hits(&res);
            }
            Ok(if res.results.is_empty() {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            })
        }
    }
}

fn print_hits(res: &scout_corpus::SearchResults) {
    println!(
        "{} documents, {} passages for {:?} in {}",
        res.total_documents,
        res.total_passages,
        res.query,
        res.corpora.join(", ")
    );
    for (i, d) in res.results.iter().enumerate() {
        let hit = &d.hits[0];
        println!();
        println!("{:>3}. {}", i + 1, d.title);
        let date = d.date.clone().unwrap_or_else(|| "—".into());
        let genre = d.genre.clone().unwrap_or_else(|| "—".into());
        println!(
            "     {date:<10}  {genre:<10}  {}:{}  ({} passages)",
            d.rel_path, hit.line, d.passage_count
        );
        let q: String = hit.quote.split_whitespace().collect::<Vec<_>>().join(" ");
        let q = if q.chars().count() > 220 {
            format!("{}…", q.chars().take(220).collect::<String>())
        } else {
            q
        };
        println!("     > {q}");
    }
}

fn corpora(cmd: CorporaCmd) -> Result<ExitCode> {
    match cmd {
        CorporaCmd::InitDefaults { force } => {
            let path = registry::registry_path();
            if registry::init_defaults(&path, force)? {
                println!("wrote {}", path.display());
            } else {
                println!(
                    "{} exists; left unchanged (use --force to overwrite)",
                    path.display()
                );
            }
            let reg = Registry::load_from(&path)?;
            for c in &reg.corpora {
                println!("  {:<11} {:<24} {}", c.id, c.kind.as_str(), c.path);
            }
            Ok(ExitCode::SUCCESS)
        }
        CorporaCmd::List => {
            let reg = Registry::load()?;
            let dir = index::index_dir();
            println!("{:<11} {:<24} {:<8} path", "id", "kind", "index");
            for c in &reg.corpora {
                let built = index::index_path_in(&dir, &c.id).exists();
                println!(
                    "{:<11} {:<24} {:<8} {}",
                    c.id,
                    c.kind.as_str(),
                    if built { "built" } else { "missing" },
                    c.path
                );
            }
            Ok(ExitCode::SUCCESS)
        }
        CorporaCmd::Show { id } => {
            let reg = Registry::load()?;
            let c = reg.get(&id)?;
            let one = Registry {
                corpora: vec![c.clone()],
            };
            print!("{}", one.to_toml()?);
            let p = index::index_path_in(&index::index_dir(), &c.id);
            println!("\n# root:  {}", c.root().display());
            println!(
                "# index: {} ({})",
                p.display(),
                if p.exists() { "built" } else { "missing" }
            );
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn index_cmd(cmd: IndexCmd) -> Result<ExitCode> {
    let reg = Registry::load()?;
    match cmd {
        IndexCmd::Build { ids, force } => {
            for c in select(&reg, &ids)? {
                let r = c.build_index(force)?;
                println!(
                    "{:<10} {} docs · {} passages · {} tokens  ({} {}: {} indexed, {} unchanged, {} removed, {} not documents) {:.1}s",
                    r.corpus,
                    r.docs,
                    r.passages,
                    r.tokens,
                    if r.full { "full" } else { "incremental" },
                    r.files_seen,
                    r.indexed,
                    r.unchanged,
                    r.removed,
                    r.not_documents,
                    r.elapsed_ms as f64 / 1000.0
                );
            }
            Ok(ExitCode::SUCCESS)
        }
        IndexCmd::Status => {
            let dir = index::index_dir();
            println!(
                "{:<11} {:>6} {:>9} {:>10}  {:<20}  stale",
                "corpus", "docs", "passages", "tokens", "built-at"
            );
            for c in &reg.corpora {
                let corpus = Corpus::from_config(c.clone(), &dir);
                let s = corpus.status()?;
                if !s.exists {
                    let note = if c.kind == CorpusKind::MarkdownFolder {
                        format!("no index; run `scout index build {}`", c.id)
                    } else {
                        format!("{} not indexable yet", c.kind.as_str())
                    };
                    println!(
                        "{:<11} {:>6} {:>9} {:>10}  {:<20}  {}",
                        c.id, "—", "—", "—", "—", note
                    );
                    continue;
                }
                let stale = match &s.stale {
                    Some(st) if st.added + st.changed + st.removed == 0 && s.config_current => {
                        "none".to_string()
                    }
                    Some(st) => format!(
                        "{} new · {} changed · {} removed{}",
                        st.added,
                        st.changed,
                        st.removed,
                        if s.config_current {
                            ""
                        } else {
                            " · config changed"
                        }
                    ),
                    None => "—".into(),
                };
                println!(
                    "{:<11} {:>6} {:>9} {:>10}  {:<20}  {}",
                    c.id,
                    s.docs,
                    s.passages,
                    s.tokens,
                    s.built_at.unwrap_or_default(),
                    stale
                );
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

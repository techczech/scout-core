//! `scout`: argument parsing and output formatting over `scout-corpus`.
//! No engine logic lives here (invariant 5).
//!
//! Exit codes: 0 ok, 1 no results, 2 usage, 3 index or registry missing.

use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand, ValueEnum};
use scout_corpus::{
    index, registry, search_all, Corpus, CorpusKind, IndexMissing, Registry, RegistryMissing,
    SearchRequest,
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

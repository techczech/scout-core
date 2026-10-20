//! Argument parsing and presentation only; capture is owned by the facade.
use crate::{out, term};
use anyhow::Result;
use clap::Args;
use scout_corpus::api::{CaptureQuery, Engine};
use std::process::ExitCode;

#[derive(Args)]
pub struct CaptureArgs {
    /// Existing project folder; captures go in its research/ folder.
    #[arg(long, required_unless_present = "where_used")]
    project: Option<String>,
    /// evidence, table, visualisation, snippet, search or widget.
    #[arg(long, required_unless_present_any = ["list", "where_used"], conflicts_with_all = ["list", "where_used"])]
    kind: Option<String>,
    /// Title of the reusable unit.
    #[arg(long, required_unless_present_any = ["list", "where_used"], conflicts_with_all = ["list", "where_used"])]
    title: Option<String>,
    /// Question that produced the material.
    #[arg(long, conflicts_with_all = ["list", "where_used"])]
    prompt: Option<String>,
    /// Repeatable provenance: scout:<passage-id>, URL, or file:<path>.
    #[arg(long = "source", conflicts_with_all = ["list", "where_used"])]
    sources: Vec<String>,
    /// Artefact to reference, copied beside the stub when outside the project.
    #[arg(long, conflicts_with_all = ["list", "where_used"])]
    artefact: Option<String>,
    /// Lower-case ASCII words separated by hyphens; default derives from title.
    #[arg(long, conflicts_with_all = ["list", "where_used"])]
    slug: Option<String>,
    /// verb, agent, app:talkweaver or app:writeflex.
    #[arg(long, default_value = "verb", conflicts_with_all = ["list", "where_used"])]
    by: String,
    /// Print the project's manifest receipts.
    #[arg(long, conflicts_with = "where_used")]
    list: bool,
    /// Find mentions of this file name in registered vault Markdown.
    #[arg(long, conflicts_with = "project")]
    where_used: Option<String>,
    /// Print the facade response as JSON.
    #[arg(long)]
    json: bool,
    /// Markdown input path, or - for stdin.
    #[arg(required_unless_present_any = ["list", "where_used"], conflicts_with_all = ["list", "where_used"])]
    input: Option<String>,
}

pub fn run(args: CaptureArgs) -> Result<ExitCode> {
    let engine = Engine::for_capture()?.with_embedder(crate::embedder()?);
    if args.list {
        return out(
            engine.capture_list(args.project.as_deref().unwrap()),
            args.json,
            |rows| {
                for row in &rows.entries {
                    println!("{}", term::safe(&serde_json::to_string(row).unwrap()));
                }
            },
        );
    }
    if let Some(file) = args.where_used {
        return out(engine.capture_where_used(&file), args.json, |rows| {
            for row in &rows.hits {
                println!(
                    "{}:{}: {}",
                    term::safe(&row.file),
                    row.line,
                    term::safe(&row.text)
                );
            }
        });
    }
    out(
        engine.capture(&CaptureQuery {
            project: args.project.unwrap(),
            kind: args.kind.unwrap(),
            title: args.title.unwrap(),
            prompt: args.prompt,
            sources: args.sources,
            artefact: args.artefact,
            slug: args.slug,
            by: args.by,
            input: args.input,
            ..Default::default()
        }),
        args.json,
        |r| {
            println!("{}", term::safe(&r.file));
            println!(
                "manifest: {}; corpus: {}; indexed: {}; passages: {}",
                r.manifest,
                term::safe(r.corpus.as_deref().unwrap_or("none")),
                r.indexed,
                r.passages
            );
        },
    )
}

//! `scout themes list`: the theme list of an archive. Like `scout
//! briefings list` it works on an archive folder, not on the registry or
//! an index, and prints one JSON line. Reads only.

use anyhow::Result;
use clap::Subcommand;
use scout_corpus::themes::{themes_list, ThemeEntry};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Subcommand)]
pub enum ThemesCmd {
    /// List the themes of the archive's theme list, in the list's order
    /// and spelling: the names `scout correct theme-add` accepts.
    List {
        /// The archive folder (holds analysis/context/themes.json). A
        /// path with `..` is refused.
        #[arg(long)]
        archive: PathBuf,
        /// Accepted; the output is always one JSON line.
        #[arg(long)]
        json: bool,
    },
}

/// The one line of `themes list`.
#[derive(serde::Serialize)]
struct Listed {
    themes: Vec<ThemeEntry>,
}

pub fn run(cmd: ThemesCmd) -> Result<ExitCode> {
    match cmd {
        ThemesCmd::List { archive, .. } => {
            let listed = Listed {
                themes: themes_list(&archive)?,
            };
            println!("{}", serde_json::to_string(&listed)?);
        }
    }
    Ok(ExitCode::SUCCESS)
}

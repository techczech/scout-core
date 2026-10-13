//! `scout briefings list|show`: the monthly briefings of an archive. Like
//! `scout events <subcommand>` they work on an archive folder, not on the
//! registry or an index, and each prints one JSON line. Reads only.

use anyhow::Result;
use clap::Subcommand;
use scout_corpus::briefings::{briefing, briefings, BriefingListing};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Subcommand)]
pub enum BriefingsCmd {
    /// List the briefings of an archive, newest period first. A file that
    /// cannot be read as a briefing is a row with `error`.
    List {
        /// The archive folder (holds analysis/context/briefings). A path
        /// with `..` is refused.
        #[arg(long)]
        archive: PathBuf,
        /// Accepted; the output is always one JSON line.
        #[arg(long)]
        json: bool,
    },
    /// One briefing, each event with the lasting event record that holds
    /// it now (`record`; null when none does).
    Show {
        /// YYYY-MM or YYYY-Qn.
        period: String,
        /// The archive folder (holds analysis/context/briefings and
        /// analysis/events). A path with `..` is refused.
        #[arg(long)]
        archive: PathBuf,
        /// Accepted; the output is always one JSON line.
        #[arg(long)]
        json: bool,
    },
}

/// The one line of `briefings list`.
#[derive(serde::Serialize)]
struct Listed {
    briefings: Vec<BriefingListing>,
}

pub fn run(cmd: BriefingsCmd) -> Result<ExitCode> {
    match cmd {
        BriefingsCmd::List { archive, .. } => {
            let listed = Listed {
                briefings: briefings(&archive)?,
            };
            println!("{}", serde_json::to_string(&listed)?);
        }
        BriefingsCmd::Show {
            period, archive, ..
        } => println!("{}", serde_json::to_string(&briefing(&archive, &period)?)?),
    }
    Ok(ExitCode::SUCCESS)
}

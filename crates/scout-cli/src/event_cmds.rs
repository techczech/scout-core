//! `scout events <subcommand>`: the seed and the edits of lasting event
//! records. They work on an archive folder, not on the registry or an
//! index; each prints one JSON line.

use anyhow::Result;
use clap::{Subcommand, ValueEnum};
use scout_corpus::event_edit::{By, Decision, EventEditor, FieldEdit, MergeRequest, NewEvent};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Clone, Copy, ValueEnum)]
pub enum ByArg {
    Dominik,
    Model,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum DecisionArg {
    Approve,
    Dismiss,
}

#[derive(Subcommand)]
pub enum EventsCmd {
    /// Create one record file for every monthly event no record holds yet
    /// (`<out>/<id>.md`). No model call. Without --apply nothing is
    /// written. Prints its counts as one JSON line. Stops before any write
    /// when a record already in the out folder, or one about to be
    /// written, is one `index build` would skip.
    Seed {
        /// The archive folder (holds analysis/context/briefings and
        /// analysis/topics/current.jsonl). A path with `..` is refused.
        #[arg(long)]
        archive: PathBuf,
        /// Where records go (default: <archive>/analysis/events). Must be
        /// the folder itself, not a symlink to it. A path with `..` is
        /// refused.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Write the records.
        #[arg(long)]
        apply: bool,
    },
    /// Change fields of one record and set its `edited` to today.
    Set {
        id: String,
        /// The archive folder (records are in <archive>/analysis/events).
        #[arg(long)]
        archive: PathBuf,
        #[arg(long)]
        name: Option<String>,
        /// Replaces the record's body.
        #[arg(long)]
        description: Option<String>,
        /// YYYY-MM-DD.
        #[arg(long)]
        start: Option<String>,
        /// YYYY-MM-DD; "" clears it.
        #[arg(long)]
        end: Option<String>,
        #[arg(long = "add-alias")]
        add_alias: Vec<String>,
        #[arg(long = "remove-alias")]
        remove_alias: Vec<String>,
    },
    /// Merge event A into event B (B survives) and log the merge.
    Merge {
        a: String,
        b: String,
        #[arg(long)]
        archive: PathBuf,
        #[arg(long, value_enum, default_value = "dominik")]
        by: ByArg,
        #[arg(long)]
        reason: Option<String>,
    },
    /// Undo the merge of one merge-log line (m000001).
    Unmerge {
        log_id: String,
        #[arg(long)]
        archive: PathBuf,
    },
    /// Merge every pair a proposals file judges same and confident.
    /// Without --apply nothing is written.
    ApplyProposals {
        /// A file inside <archive>/analysis/events/merge-proposals/.
        file: PathBuf,
        #[arg(long)]
        archive: PathBuf,
        #[arg(long)]
        apply: bool,
    },
    /// List the pairs of a proposals file still waiting for a decision.
    Proposals {
        file: PathBuf,
        #[arg(long)]
        archive: PathBuf,
        /// Accepted; the output is always one JSON line.
        #[arg(long)]
        json: bool,
    },
    /// Approve (merge) or dismiss one waiting pair of a proposals file.
    Decide {
        file: PathBuf,
        a: String,
        b: String,
        #[arg(value_enum)]
        decision: DecisionArg,
        #[arg(long)]
        archive: PathBuf,
    },
    /// Mark an event removed: the record stays, the event is not listed
    /// and no item has it.
    Remove {
        id: String,
        #[arg(long)]
        archive: PathBuf,
    },
    /// Take back `remove`.
    Restore {
        id: String,
        #[arg(long)]
        archive: PathBuf,
    },
    /// Create an event record (origin: dominik, no members).
    Create {
        #[arg(long)]
        archive: PathBuf,
        #[arg(long)]
        name: String,
        #[arg(long)]
        description: Option<String>,
        /// YYYY-MM-DD; its month starts the id (default: this month).
        #[arg(long)]
        start: Option<String>,
        /// YYYY-MM-DD.
        #[arg(long)]
        end: Option<String>,
    },
    /// Compare the merge log with the records; exit 1 when they disagree.
    Check {
        #[arg(long)]
        archive: PathBuf,
    },
    /// List the merges in effect, newest first, each with the log id
    /// `unmerge` takes. Reads only.
    Merges {
        #[arg(long)]
        archive: PathBuf,
        /// Also list the merges that were undone.
        #[arg(long)]
        all: bool,
        /// Accepted; the output is always one JSON line.
        #[arg(long)]
        json: bool,
    },
}

fn line<T: serde::Serialize>(report: &T) -> Result<ExitCode> {
    println!("{}", serde_json::to_string(report)?);
    Ok(ExitCode::SUCCESS)
}

pub fn run(cmd: EventsCmd) -> Result<ExitCode> {
    match cmd {
        EventsCmd::Seed {
            archive,
            out,
            apply,
        } => {
            let r = scout_corpus::event_seed::seed(&scout_corpus::event_seed::SeedOptions {
                archive,
                out,
                apply,
            })?;
            println!("{}", serde_json::to_string(&r)?);
            if !r.apply {
                eprintln!(
                    "dry run: nothing written; {} records would be created in {} (add --apply)",
                    r.to_create,
                    crate::term::safe(&r.out)
                );
            }
            Ok(ExitCode::SUCCESS)
        }
        EventsCmd::Set {
            id,
            archive,
            name,
            description,
            start,
            end,
            add_alias,
            remove_alias,
        } => line(&EventEditor::open(&archive)?.set(
            &id,
            &FieldEdit {
                name,
                description,
                start,
                end,
                add_aliases: add_alias,
                remove_aliases: remove_alias,
            },
        )?),
        EventsCmd::Merge {
            a,
            b,
            archive,
            by,
            reason,
        } => line(&EventEditor::open(&archive)?.merge(&MergeRequest {
            a,
            b,
            by: match by {
                ByArg::Dominik => By::Dominik,
                ByArg::Model => By::Model,
            },
            reason,
        })?),
        EventsCmd::Unmerge { log_id, archive } => {
            line(&EventEditor::open(&archive)?.unmerge(&log_id)?)
        }
        EventsCmd::ApplyProposals {
            file,
            archive,
            apply,
        } => {
            let r = EventEditor::open(&archive)?.apply_proposals(&file, apply)?;
            if !apply {
                eprintln!(
                    "dry run: nothing written; {} merges would be made (add --apply)",
                    r.merges.len()
                );
            }
            line(&r)
        }
        EventsCmd::Proposals { file, archive, .. } => {
            line(&EventEditor::open(&archive)?.proposals(&file)?)
        }
        EventsCmd::Decide {
            file,
            a,
            b,
            decision,
            archive,
        } => line(&EventEditor::open(&archive)?.decide(
            &file,
            &a,
            &b,
            match decision {
                DecisionArg::Approve => Decision::Approve,
                DecisionArg::Dismiss => Decision::Dismiss,
            },
        )?),
        EventsCmd::Remove { id, archive } => line(&EventEditor::open(&archive)?.remove(&id)?),
        EventsCmd::Restore { id, archive } => line(&EventEditor::open(&archive)?.restore(&id)?),
        EventsCmd::Create {
            archive,
            name,
            description,
            start,
            end,
        } => line(&EventEditor::open(&archive)?.create(&NewEvent {
            name,
            description,
            start,
            end,
        })?),
        EventsCmd::Merges { archive, all, .. } => {
            line(&EventEditor::open(&archive)?.merge_list(all)?)
        }
        EventsCmd::Check { archive } => {
            let r = EventEditor::open(&archive)?.check()?;
            println!("{}", serde_json::to_string(&r)?);
            Ok(if r.ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            })
        }
    }
}

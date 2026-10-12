//! `scout correct <subcommand>`: his fixes to one item's tags, appended to
//! `<archive>/analysis/topics/corrections.jsonl`. They work on an archive
//! folder, not on the registry or an index; each prints one JSON line. A
//! refusal exits 2 with nothing written.

use anyhow::Result;
use clap::Subcommand;
use scout_corpus::corrections::CorrectionEditor;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Subcommand)]
pub enum CorrectCmd {
    /// Add a theme of the archive's theme list to an item.
    ThemeAdd {
        /// The theme's name, as in analysis/context/themes.json.
        theme: String,
        /// The item: <source_system>:<source_id>.
        #[arg(long)]
        key: String,
        /// The archive folder (holds analysis/topics). A path with `..`
        /// is refused.
        #[arg(long)]
        archive: PathBuf,
    },
    /// Take a theme off an item.
    ThemeRemove {
        theme: String,
        #[arg(long)]
        key: String,
        #[arg(long)]
        archive: PathBuf,
    },
    /// Add a named entity to an item.
    EntityAdd {
        name: String,
        /// person, org, model, product, work, event, place or other.
        #[arg(long = "type")]
        kind: String,
        #[arg(long)]
        key: String,
        #[arg(long)]
        archive: PathBuf,
    },
    /// Take a named entity off an item.
    EntityRemove {
        name: String,
        #[arg(long)]
        key: String,
        #[arg(long)]
        archive: PathBuf,
    },
    /// Put an item in a lasting event.
    EventAdd {
        /// A lasting id, a merged-away id or a monthly id.
        event: String,
        #[arg(long)]
        key: String,
        #[arg(long)]
        archive: PathBuf,
    },
    /// Take an item out of a lasting event.
    EventRemove {
        event: String,
        #[arg(long)]
        key: String,
        #[arg(long)]
        archive: PathBuf,
    },
    /// Take back one correction (c000001), the last change first.
    Undo {
        id: String,
        #[arg(long)]
        archive: PathBuf,
    },
    /// An item's tags, the model's and those in effect, and its
    /// corrections, newest first.
    List {
        #[arg(long)]
        key: String,
        #[arg(long)]
        archive: PathBuf,
        /// Accepted; the output is always one JSON line.
        #[arg(long)]
        json: bool,
    },
}

fn line<T: serde::Serialize>(report: &T) -> Result<ExitCode> {
    println!("{}", serde_json::to_string(report)?);
    Ok(ExitCode::SUCCESS)
}

pub fn run(cmd: CorrectCmd) -> Result<ExitCode> {
    let open = |archive: &PathBuf| CorrectionEditor::open(archive);
    match cmd {
        CorrectCmd::ThemeAdd {
            theme,
            key,
            archive,
        } => line(&open(&archive)?.add_theme(&key, &theme)?),
        CorrectCmd::ThemeRemove {
            theme,
            key,
            archive,
        } => line(&open(&archive)?.remove_theme(&key, &theme)?),
        CorrectCmd::EntityAdd {
            name,
            kind,
            key,
            archive,
        } => line(&open(&archive)?.add_entity(&key, &name, &kind)?),
        CorrectCmd::EntityRemove { name, key, archive } => {
            line(&open(&archive)?.remove_entity(&key, &name)?)
        }
        CorrectCmd::EventAdd {
            event,
            key,
            archive,
        } => line(&open(&archive)?.add_to_event(&key, &event)?),
        CorrectCmd::EventRemove {
            event,
            key,
            archive,
        } => line(&open(&archive)?.remove_from_event(&key, &event)?),
        CorrectCmd::Undo { id, archive } => line(&open(&archive)?.undo(&id)?),
        CorrectCmd::List { key, archive, .. } => line(&open(&archive)?.list(&key)?),
    }
}

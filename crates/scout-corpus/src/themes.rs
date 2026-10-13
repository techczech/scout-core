//! The theme list of an archive: the tagger's fixed list of themes,
//! `<archive>/analysis/context/themes.json`.
//!
//! Two callers, one loader ([`load`]): the correction writer
//! (`CorrectionEditor::add_theme`), which adds a theme only from this
//! list and writes the list's spelling, and the public read
//! ([`themes_list`]), which gives an app exactly the names `add_theme`
//! accepts. [`same_theme`] is the comparison both make.
//!
//! Read only. Nothing is written, no lock is taken, and neither the
//! registry nor an index is read. The read carries no item counts: an app
//! joins `Engine::topics` (field `theme`) to the list with
//! [`same_theme`].
//!
//! The file: `{"themes": [{"name": "…", "id": "T01", "definition": "…",
//! …}, …]}`. `name` is required text; `id` and `definition` are returned
//! when present; nothing else is read.
//!
//! The open is the writer's (`event_edit::store::read_plain`): the entry
//! is looked at without following a link, opened with `O_NOFOLLOW`, the
//! handle checked to be a regular file, at most [`MAX_THEMES_BYTES`]
//! read. The read adds, for itself only, the briefings read's rule for
//! the folders on the way (`analysis`, `analysis/context`): each must be
//! a folder, not a link to one. The writer finds the file as it always
//! did.

use crate::briefings::plain_folder_below;
use crate::event_edit::plain_text;
use crate::event_edit::store::read_plain;
use crate::event_seed::render::body_text;
use crate::event_seed::sources::one_line;
use crate::normalize::fold;
use anyhow::{bail, Result};
use serde::Serialize;
use serde_json::Value;
use std::path::Path;

/// The theme list, relative to the archive.
pub const THEMES_FILE: &str = "analysis/context/themes.json";
/// The most the theme list may hold.
pub const MAX_THEMES_BYTES: u64 = 16 << 20;

/// One theme of the list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ThemeEntry {
    /// The name in the list's spelling: what `add_theme` writes.
    pub name: String,
    /// The tagger's id of the theme (`T01`), when the list gives one.
    pub id: Option<String>,
    /// The tagger's description of the theme, on one line, when the list
    /// gives one.
    pub definition: Option<String>,
}

/// The archive has no theme list (`themes.json`, or a folder on the way
/// to it, is not there).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NoThemeList;

impl std::fmt::Display for NoThemeList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "No theme list in this archive.")
    }
}

impl std::error::Error for NoThemeList {}

/// Whether `a` and `b` name the same theme: equal under the one fold of
/// field values ([`crate::normalize::fold`]: NFC, quote and apostrophe
/// variants unified, whitespace runs as one space, case ignored). It is
/// the comparison `add_theme` makes between the name it is given and the
/// names of the list, and the index makes for `theme:`.
pub fn same_theme(a: &str, b: &str) -> bool {
    fold(a) == fold(b)
}

/// An optional text field of a stored theme.
pub(crate) enum Field {
    /// Absent or null.
    None,
    Text(String),
    /// Present and not text.
    NotText,
}

impl Field {
    fn of(theme: &Value, key: &str) -> Field {
        match theme.get(key) {
            None | Some(Value::Null) => Field::None,
            Some(Value::String(s)) => Field::Text(s.clone()),
            Some(_) => Field::NotText,
        }
    }
}

/// One theme as the file holds it.
pub(crate) struct Stored {
    /// Trimmed, not empty.
    pub(crate) name: String,
    pub(crate) id: Field,
    pub(crate) definition: Field,
}

/// The themes of `<archive>/analysis/context/themes.json`, in the file's
/// order; `None` when no file is there. The one reader of the file. A
/// file that is not a plain file, is too large, is not JSON, has no
/// `themes` list or has a theme without a text `name` is an error naming
/// the file. `id` and `definition` are passed on as found and never an
/// error here.
pub(crate) fn load(archive: &Path) -> Result<Option<Vec<Stored>>> {
    let path = archive.join(THEMES_FILE);
    let at = path.display();
    let Some(plain) = read_plain(&path, MAX_THEMES_BYTES)? else {
        return Ok(None);
    };
    let v: Value = serde_json::from_slice(&plain.bytes)
        .map_err(|e| anyhow::anyhow!("{at}: not JSON ({e}) (nothing was changed)"))?;
    let Some(list) = v.get("themes").and_then(|t| t.as_array()) else {
        bail!("{at}: no `themes` list (nothing was changed)");
    };
    let mut themes = Vec::with_capacity(list.len());
    for (i, t) in list.iter().enumerate() {
        match t.get("name").and_then(|n| n.as_str()).map(str::trim) {
            Some(n) if !n.is_empty() => themes.push(Stored {
                name: n.to_string(),
                id: Field::of(t, "id"),
                definition: Field::of(t, "definition"),
            }),
            _ => bail!(
                "{at}: theme {} has no text `name` (nothing was changed)",
                i + 1
            ),
        }
    }
    Ok(Some(themes))
}

/// The theme list of `archive`, in the list's order and spelling: the
/// themes `CorrectionEditor::add_theme` accepts, read by the loader it
/// uses. Every `name` returned is accepted by `add_theme` and written as
/// returned; a name that is none of them under [`same_theme`] is refused
/// by it.
///
/// Errors: [`NoThemeList`] when the file, or a folder on the way to it,
/// is not there; the archive path with a `..` component or not a folder;
/// `analysis` or `analysis/context` a link or a file; the file not a
/// plain file, over [`MAX_THEMES_BYTES`], not JSON, without a `themes`
/// list, or with a theme whose `name` is not text; an `id` or
/// `definition` that is not text; a `name` holding a control character
/// (`add_theme` cannot be given it, and the read returns no altered
/// name).
///
/// A later name that is the same theme as an earlier one is left out:
/// `add_theme` would write the earlier spelling for it.
pub fn themes_list(archive: &Path) -> Result<Vec<ThemeEntry>> {
    let folder = Path::new(THEMES_FILE)
        .parent()
        .and_then(Path::to_str)
        .unwrap_or_default();
    if plain_folder_below(archive, folder, "the theme list was not read")?.is_none() {
        return Err(NoThemeList.into());
    }
    let Some(stored) = load(archive)? else {
        return Err(NoThemeList.into());
    };
    let at = archive.join(THEMES_FILE);
    let at = at.display();
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::with_capacity(stored.len());
    for (i, t) in stored.into_iter().enumerate() {
        let n = i + 1;
        // The check `add_theme` makes of the theme it is given.
        let name = plain_text(&format!("{at}: theme {n}: `name`"), &t.name)?;
        let optional = |field: Field, key: &str| match field {
            Field::None => Ok(None),
            Field::Text(s) => Ok(Some(one_line(&body_text(&s))).filter(|s| !s.is_empty())),
            Field::NotText => Err(anyhow::anyhow!(
                "{at}: theme {n}: `{key}` is not text (nothing was changed)"
            )),
        };
        let id = optional(t.id, "id")?;
        let definition = optional(t.definition, "definition")?;
        if seen.insert(fold(&name)) {
            out.push(ThemeEntry {
                name,
                id,
                definition,
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_theme_is_the_fold_of_field_values() {
        for (a, b) in [
            ("AI and mathematics", "ai AND  mathematics"),
            (" AI and\tmathematics ", "AI and mathematics"),
            ("Reader’s tools", "reader's TOOLS"),
            ("GPT “Astra”", "gpt \"astra\""),
            ("Cafe\u{301}", "caf\u{e9}"),
        ] {
            assert!(same_theme(a, b), "{a:?} {b:?}");
            assert_eq!(fold(a), fold(b));
        }
        for (a, b) in [
            ("AI and mathematics", "AI & mathematics"),
            ("AI and mathematics", "AI and mathematic"),
            ("AIand mathematics", "AI and mathematics"),
            ("Cafe", "Café"),
            ("", "x"),
        ] {
            assert!(!same_theme(a, b), "{a:?} {b:?}");
        }
    }
}

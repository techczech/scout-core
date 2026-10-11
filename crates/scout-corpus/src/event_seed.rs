//! `scout events seed`: one record file for every monthly event that no
//! record holds yet (spec
//! `archive-scout/docs/specs/2026-10-11-lasting-events-spec.md`, "Where
//! lasting events come from", step 1). Deterministic: no model, no network.
//!
//! There is one implementation of the record format, the reader in
//! [`crate::events`], and the seed stands on it:
//!
//! - The records already in the out folder are read by
//!   [`events::read_dir`], the function an index build reads them with. If
//!   it skips any file, the seed stops: counting a skipped record's
//!   members as held would leave those events without a usable record.
//! - Every record the seed means to write is rendered to bytes and given
//!   to [`events::parse_bytes`], the function every record the reader
//!   accepts has passed through (size cap, UTF-8, control characters,
//!   frontmatter, YAML, fields). What comes back must equal the record meant. Then the whole
//!   set, old and new, goes through [`EventSet::from_records`]: no id may
//!   repeat and every new member must resolve to its new record.
//! - Only then is anything written, and a failure of any check leaves the
//!   out folder exactly as it was (it is not even created). Once writing
//!   has begun a run can still stop (a write fails, or a file of a
//!   record's name appeared since the folder was read): the records made
//!   so far stay, they are whole, and a second run makes the rest.
//!
//! Every input goes through the one reader of its format:
//!
//! - Tag rows, `<archive>/analysis/topics/current.jsonl`:
//!   `topics::read_tag_rows`, the function an index build reads the file
//!   with (its bad-line rules; the last valid row of a key counts). The
//!   seed takes each row's `key`, `event`, `date`, `themes` and
//!   `event_name` from it and parses nothing itself.
//! - Which monthly event an id names (a tag row's `event`, a briefing
//!   event's `id`, a record's `members`) is decided under the index's own
//!   comparison, `normalize::fold`: `naming::monthly_id` for the first
//!   two, [`EventSet::resolve`] for "a record already holds it". The seed
//!   writes the id as `<period>/E<digits>`.
//! - Corrections (`analysis/topics/corrections.jsonl`) and a row's
//!   `events` list are NOT read: the seed makes a record for every monthly
//!   id a tag row's `event` or a briefing file names, whatever a correction
//!   later says about one item. An event only a correction names gets no
//!   record here.
//! - Records already in the out folder: [`events::read_dir`], above.
//! - Briefing files,
//!   `<archive>/analysis/context/briefings/<period>-events.json` (names
//!   holding `.superseded-` are ignored; the folder is not walked): the
//!   seed is their only reader in this workspace (`event_seed/sources.rs`).
//!   A file that is not a plain file, not JSON, or wrongly shaped stops
//!   the run before any write, with the file named.
//! - The names in use in the out folder: a folder listing, no format.
//!
//! None of these is written.
//!
//! Writes (only with `apply`): the out folder when it is missing (with
//! any missing folder above it), and new record files directly inside it, each `<id>.md` with a slug id, each
//! opened create-exclusive and no-follow. No existing file is opened for
//! writing, renamed or removed; no temporary or log file is made. The
//! run's counts go to the caller.
//!
//! A control character in a briefing's name or alias is written to the
//! frontmatter as an escape inside a double-quoted scalar, so the file
//! holds none and the reader gives the value back whole. A description is
//! the body, plain prose without escapes: there the characters the reader
//! refuses are taken out before the record is made (`render::body_text`:
//! a bare CR, U+0085, U+2028 and U+2029 become LF, the rest are dropped).
//!
//! A path with a `..` component is refused, for the archive and for the
//! out folder alike, before anything is looked at (a dry run too): `..`
//! after a symlink means the parent of the link's target, which no
//! reading of the path's text can tell. The seed has no code that takes
//! a `..` out of a path.
//!
//! The out folder is the path given, made absolute: the current folder in
//! front of a relative path, and the path's components put back together
//! without its `.` components and trailing `/` (so `link/.` and `link/`
//! are the link, and refused as one). It must be a real folder, not a
//! symlink. Symlinks in the folders above it are accepted
//! (macOS `/tmp` is one). When the folder exists they are resolved as it
//! is looked at (before the plan and, with `apply`, once more after it is
//! made) and the report names the resolved folder; a dry run over a
//! folder that does not exist yet reports the absolute path unresolved.
//!
//! Threat model. A single-user desktop tool; no hostile process runs
//! beside it.
//!
//! - Holds, whatever is in the out folder when the run looks:
//!   - The out folder itself is not a symlink (refused), so records are
//!     not created through a link standing in its place.
//!   - An existing name is never opened for writing, truncated, renamed or
//!     removed, whatever it is (a file, a symlink dangling or not, a hard
//!     link to a file elsewhere, a folder). A new record takes a free name
//!     and is opened create-exclusive and no-follow, so its bytes go to a
//!     new file of its own and reach nothing through a link.
//!   - Symlinks and folders under a record name are not read as records:
//!     the reader refuses them and the seed stops. A hard-linked regular
//!     file is read like any other file: as a record when it is a valid
//!     one, refused when it is not. Reading it is harmless; there is no
//!     link-count check.
//!   - No traversal through ids: an id is a slug, the file name a bare
//!     `<slug>.md`. None through the paths given either: `..` is refused.
//!   - No partial state after a failed check: every check runs before the
//!     first write. (A stop during the writes leaves whole records only;
//!     see above.)
//! - Not defended: another process replacing the out folder while the seed
//!   runs. The folder is checked once and each record is then created by
//!   path, so if the folder is renamed away and a symlink put in its place
//!   between two writes, the later records land where the link points. The
//!   writes are not anchored to a directory descriptor (driver's ruling,
//!   2026-10-11: outside the threat model).

mod naming;
mod render;
mod sources;

use crate::events::{self, EventRecord, EventSet, EVENTS_DIR};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::collections::BTreeSet;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

pub use naming::{SLUG_MAX, THEMES_MAX};
pub use render::render_record;
pub use sources::BRIEFINGS_DIR;

/// The `tool` of a seed report.
pub const TOOL: &str = "scout events seed";

/// What to seed, and where.
#[derive(Debug, Clone, Default)]
pub struct SeedOptions {
    /// The archive root (the corpus folder).
    pub archive: PathBuf,
    /// Where records go; `None` is `<archive>/analysis/events`.
    pub out: Option<PathBuf>,
    /// Write the records. Without it nothing is written.
    pub apply: bool,
}

/// A record id that took a `-2`, `-3` … suffix.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Collision {
    pub id: String,
    /// The id without the suffix.
    pub base: String,
    pub member: String,
}

/// A record whose name gave no slug; its id is from the event number.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FallbackSlug {
    pub id: String,
    pub name: String,
    pub member: String,
}

/// How many monthly events each note applies to.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct NoteCounts {
    pub collisions: usize,
    pub fallback_slugs: usize,
    pub truncated_slugs: usize,
    pub non_ascii_names: usize,
    pub no_briefing_entry: usize,
    /// Events dated from their month or quarter (no dated item).
    pub month_dates: usize,
    pub briefing_events_without_items: usize,
}

/// The counts of one seed run (`scout events seed` prints this as one
/// JSON line).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SeedReport {
    /// When the run ended, UTC.
    pub date: String,
    pub tool: String,
    pub version: String,
    pub archive: String,
    /// The out folder, resolved.
    pub out: String,
    pub apply: bool,
    /// Monthly events known: briefing events plus ids only tag rows name.
    pub monthly_events: usize,
    pub briefing_events: usize,
    pub monthly_events_with_items: usize,
    pub briefing_events_without_items: usize,
    /// Monthly events an existing record holds.
    pub already_in_a_record: usize,
    /// Records this run creates, or would with `apply`.
    pub to_create: usize,
    /// Of `to_create`, records of an event without tagged items.
    pub no_items: usize,
    /// Of `to_create`, records of an event with one tagged item.
    pub one_item: usize,
    /// Record files written (0 without `apply`).
    pub created: usize,
    /// Monthly events left alone: `already_in_a_record`.
    pub skipped: usize,
    pub notes: NoteCounts,
    pub collisions: Vec<Collision>,
    pub fallback_slugs: Vec<FallbackSlug>,
    /// Monthly ids the tag rows name and no briefing file holds.
    pub no_briefing_entry: Vec<String>,
}

/// A record to create, with the bytes of its file.
struct Planned {
    member: String,
    record: EventRecord,
    text: String,
}

/// Refuse `path` when it has a `..` component. Nothing is looked at.
fn refuse_parent_dir(path: &Path, what: &str) -> Result<()> {
    if path.components().any(|c| c == Component::ParentDir) {
        bail!(
            "{}: {what} path has a `..` component; give the path without it (nothing was written)",
            path.display()
        );
    }
    Ok(())
}

/// Seed lasting-event records (see the module notes). Stops with an error
/// naming every file or monthly event at fault, before any write, when a
/// path given has a `..` component, when the out folder is a symlink or
/// not a folder, when the reader skips a record already there, or when it
/// would not read back a record about to be written.
pub fn seed(opts: &SeedOptions) -> Result<SeedReport> {
    refuse_parent_dir(&opts.archive, "the archive")?;
    if let Some(out) = &opts.out {
        refuse_parent_dir(out, "the out")?;
    }
    if !opts.archive.is_dir() {
        bail!("{}: the archive is not a folder", opts.archive.display());
    }
    let wanted = match &opts.out {
        Some(out) => out.clone(),
        None => opts.archive.join(EVENTS_DIR),
    };
    let dir = resolve_out_dir(&wanted)?;
    let existing = read_existing(&dir)?;
    let (planned, mut report) = plan(&opts.archive, &existing)?;
    verify(&planned, &existing)?;
    report.archive = opts.archive.display().to_string();
    report.apply = opts.apply;
    let dir = if opts.apply {
        std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
        // Looked at again now that it exists: still the folder itself.
        resolve_out_dir(&dir)?
    } else {
        dir
    };
    report.out = dir.display().to_string();
    if opts.apply {
        for p in &planned {
            let name = format!("{}.md", p.record.id);
            let made = create_exclusive(&dir, &name, |f| f.write_all(p.text.as_bytes()))
                .with_context(|| format!("write {}", dir.join(&name).display()))?;
            if !made {
                bail!(
                    "{}: appeared after the folder was read; it was not touched. {} of {} records were created; run again",
                    dir.join(&name).display(),
                    report.created,
                    planned.len()
                );
            }
            report.created += 1;
        }
    }
    report.date = chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string();
    Ok(report)
}

// ---------- the out folder ----------

/// The folder records are created in: `path` (which has no `..`, see
/// [`seed`]) made absolute and, when it exists, required to be a real
/// folder (not a symlink, dangling or not; not a file), with the symlinks
/// of the folders above it resolved. A missing folder is returned
/// absolute and unresolved. Nothing is written.
fn resolve_out_dir(path: &Path) -> Result<PathBuf> {
    refuse_parent_dir(path, "the out")?;
    // Not `std::path::absolute`: it keeps a trailing `/.` and `/`, under
    // which a symlink is looked at as the folder it points to.
    let mut wanted = if path.is_absolute() {
        PathBuf::new()
    } else {
        std::env::current_dir().context("the current folder")?
    };
    wanted.extend(path.components().filter(|c| *c != Component::CurDir));
    let at = wanted.display();
    match std::fs::symlink_metadata(&wanted) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(wanted),
        Err(e) => Err(e).with_context(|| format!("{at}: the out folder")),
        Ok(m) if m.file_type().is_symlink() => {
            bail!("{at}: the out folder is a symlink; give the folder itself")
        }
        Ok(m) if !m.is_dir() => bail!("{at}: the out folder is not a folder"),
        Ok(_) => std::fs::canonicalize(&wanted).with_context(|| format!("{at}: the out folder")),
    }
}

/// The records already in `dir`, as the reader has them, and the names in
/// use there.
struct Existing {
    set: Option<EventSet>,
    /// Lower-case ids no new record may take: every record's id, and the
    /// name of every entry of the folder without its `.md` (so a new file
    /// name is free on a file system that ignores case, too).
    taken: BTreeSet<String>,
}

fn read_existing(dir: &Path) -> Result<Existing> {
    let set = events::read_dir(dir)?;
    let mut taken = BTreeSet::new();
    let Some(set) = set else {
        return Ok(Existing { set: None, taken });
    };
    if !set.refused.is_empty() {
        let mut lines: Vec<String> = set
            .refused
            .iter()
            .map(|r| {
                format!(
                    "{}: cannot read the record ({})",
                    dir.join(&r.file).display(),
                    r.why
                )
            })
            .collect();
        lines.push(format!(
            "{} not read by scout; fix or remove before seeding (nothing was written)",
            match set.refused.len() {
                1 => "this record is".to_string(),
                n => format!("these {n} records are"),
            }
        ));
        bail!("{}", lines.join("\n"));
    }
    taken.extend(set.records.keys().cloned());
    for e in std::fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let name = e
            .with_context(|| format!("read {}", dir.display()))?
            .file_name()
            .to_string_lossy()
            .to_lowercase();
        taken.insert(name.strip_suffix(".md").unwrap_or(&name).to_string());
    }
    Ok(Existing {
        set: Some(set),
        taken,
    })
}

// ---------- the plan ----------

/// Work out the records to create. Reads only.
fn plan(archive: &Path, existing: &Existing) -> Result<(Vec<Planned>, SeedReport)> {
    let monthly = sources::read_monthly_events(archive)?;
    let tagged = sources::read_tagged_items(archive)?;
    let mut taken = existing.taken.clone();
    // Every briefing event, tagged or not, and every id only tag rows name.
    let mut all: Vec<&String> = monthly
        .keys()
        .chain(tagged.keys())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    all.sort_by(|a, b| naming::by_monthly_id(a, b));
    let mut planned = Vec::new();
    let mut notes = NoteCounts::default();
    let (mut collisions, mut fallback_slugs, mut no_briefing_entry) = (vec![], vec![], vec![]);
    let (mut already, mut no_items, mut one_item) = (0, 0, 0);
    for member in &all {
        let held = existing.set.as_ref().and_then(|s| s.resolve(member));
        if held.is_some() {
            already += 1;
            continue;
        }
        let Some((period, number)) = naming::split_monthly_id(member) else {
            bail!("{member}: not a monthly event id");
        };
        let items = tagged.get(*member).map(Vec::as_slice).unwrap_or_default();
        let brief = monthly.get(*member);
        if brief.is_none() {
            no_briefing_entry.push(member.to_string());
        }
        let name = brief
            .map(|b| b.name.clone())
            .filter(|n| !n.is_empty())
            .or_else(|| items.iter().find_map(|i| i.event_name.clone()))
            .unwrap_or_else(|| member.to_string());
        let picked = naming::assign_id(period, number, &name, &mut taken);
        if !naming::is_slug(&picked.id) {
            bail!("{member}: id {:?} is not a slug", picked.id);
        }
        if picked.suffix > 0 {
            collisions.push(Collision {
                id: picked.id.clone(),
                base: picked.base.clone(),
                member: member.to_string(),
            });
        }
        if picked.fallback {
            fallback_slugs.push(FallbackSlug {
                id: picked.id.clone(),
                name: name.clone(),
                member: member.to_string(),
            });
        }
        notes.truncated_slugs += usize::from(picked.truncated);
        notes.non_ascii_names += usize::from(!name.is_ascii());
        // No dated item (or no item): the first and last day of the
        // event's month or quarter.
        let dates: BTreeSet<&str> = items.iter().filter_map(|i| i.date.as_deref()).collect();
        let (start, end) = match (dates.first(), dates.last()) {
            (Some(a), Some(b)) => (a.to_string(), b.to_string()),
            _ => {
                notes.month_dates += 1;
                naming::period_span(period)
            }
        };
        match items.len() {
            0 => no_items += 1,
            1 => one_item += 1,
            _ => {}
        }
        let record = EventRecord {
            id: picked.id,
            name,
            aliases: brief.map(|b| b.aliases.clone()).unwrap_or_default(),
            start: Some(start),
            end: Some(end),
            members: vec![member.to_string()],
            themes: naming::majority_themes(items.iter().map(|i| i.themes.as_slice())),
            origin: Some("model".into()),
            edited: None,
            merged_into: None,
            description: brief.map(|b| b.description.clone()).unwrap_or_default(),
        };
        planned.push(Planned {
            member: member.to_string(),
            text: render_record(&record),
            record,
        });
    }
    notes.collisions = collisions.len();
    notes.fallback_slugs = fallback_slugs.len();
    notes.no_briefing_entry = no_briefing_entry.len();
    notes.briefing_events_without_items = monthly
        .keys()
        .filter(|id| !tagged.contains_key(*id))
        .count();
    let report = SeedReport {
        date: String::new(),
        tool: TOOL.into(),
        version: env!("CARGO_PKG_VERSION").into(),
        archive: String::new(),
        out: String::new(),
        apply: false,
        monthly_events: all.len(),
        briefing_events: monthly.len(),
        monthly_events_with_items: tagged.len(),
        briefing_events_without_items: notes.briefing_events_without_items,
        already_in_a_record: already,
        to_create: planned.len(),
        no_items,
        one_item,
        created: 0,
        skipped: already,
        notes,
        collisions,
        fallback_slugs,
        no_briefing_entry,
    };
    Ok((planned, report))
}

/// Most faults named in one error.
const MAX_FAULTS: usize = 20;

/// Give every planned record to the reader. An error names each monthly
/// event whose record the reader would refuse, or read as something other
/// than the record meant.
fn verify(planned: &[Planned], existing: &Existing) -> Result<()> {
    let mut faults: Vec<String> = Vec::new();
    for p in planned {
        let label = format!("{} ({:?})", p.member, p.record.name);
        match events::parse_bytes(p.text.as_bytes(), &p.record.id) {
            Err(why) => faults.push(format!(
                "{label}: scout would not read its record {}.md ({why})",
                p.record.id
            )),
            Ok(back) if back != p.record => faults.push(format!(
                "{label}: its record {}.md does not read back as written",
                p.record.id
            )),
            Ok(_) => {}
        }
    }
    if faults.is_empty() {
        // The whole folder as it would be: no id twice, every new member
        // resolving to its new record.
        let old = existing
            .set
            .iter()
            .flat_map(|s| s.records.values().cloned());
        let all: Vec<EventRecord> = old
            .chain(planned.iter().map(|p| p.record.clone()))
            .collect();
        let set = EventSet::from_records(PathBuf::new(), all);
        for r in &set.refused {
            faults.push(format!("{}: {}", r.file, r.why));
        }
        for p in planned {
            if set.resolve(&p.member) != Some(p.record.id.as_str()) {
                faults.push(format!(
                    "{}: would not resolve to its record {}.md",
                    p.member, p.record.id
                ));
            }
        }
    }
    if faults.is_empty() {
        return Ok(());
    }
    let more = faults.len().saturating_sub(MAX_FAULTS);
    faults.truncate(MAX_FAULTS);
    if more > 0 {
        faults.push(format!("and {more} more"));
    }
    faults.push("nothing was written".into());
    bail!("{}", faults.join("\n"));
}

// ---------- writing ----------

/// Create `<dir>/<name>` and let `fill` write it; false, with nothing
/// touched, when an entry of that name is there (a file, a folder, or a
/// link of any kind). The open makes the file itself (create-exclusive,
/// no-follow), so the bytes cannot land in a file that existed, through a
/// symlink or a hard link. When `fill` fails the file, which is this
/// call's own, is removed. `name` must be a bare `<slug>.md`.
fn create_exclusive(
    dir: &Path,
    name: &str,
    fill: impl FnOnce(&mut std::fs::File) -> std::io::Result<()>,
) -> Result<bool> {
    if !name.strip_suffix(".md").is_some_and(naming::is_slug) {
        bail!("{name:?}: not a record file name");
    }
    let target = dir.join(name);
    let mut open = std::fs::OpenOptions::new();
    open.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        open.custom_flags(libc::O_NOFOLLOW).mode(0o644);
    }
    let mut file = match open.open(&target) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Ok(false),
        Err(e) => return Err(e.into()),
    };
    if let Err(e) = fill(&mut file).and_then(|_| file.flush()) {
        drop(file);
        let _ = std::fs::remove_file(&target);
        return Err(e.into());
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        v.sort();
        v
    }

    #[cfg(unix)]
    #[test]
    fn creating_a_record_never_follows_a_link_and_never_replaces_anything() {
        use std::os::unix::fs::{symlink, MetadataExt};
        let (d, o) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let (dir, outside) = (d.path(), o.path());
        let victim = outside.join("victim.txt");
        fs::write(&victim, "keep\n").unwrap();
        symlink(&victim, dir.join("to-file.md")).unwrap();
        symlink(outside.join("not-there.md"), dir.join("dangling.md")).unwrap();
        symlink(outside, dir.join("to-folder.md")).unwrap();
        fs::hard_link(&victim, dir.join("hard.md")).unwrap();
        fs::create_dir(dir.join("folder.md")).unwrap();
        fs::write(dir.join("plain.md"), "mine\n").unwrap();
        let before = names(dir);
        let write = |name: &str| create_exclusive(dir, name, |f| f.write_all(b"NEW\n"));
        for name in [
            "to-file.md",
            "dangling.md",
            "to-folder.md",
            "hard.md",
            "folder.md",
            "plain.md",
        ] {
            assert!(!write(name).unwrap(), "{name}");
        }
        assert_eq!(names(dir), before);
        // The dangling link's target was not created; the victim is whole.
        assert_eq!(names(outside), ["victim.txt"]);
        assert_eq!(fs::read_to_string(&victim).unwrap(), "keep\n");
        assert_eq!(fs::read_to_string(dir.join("plain.md")).unwrap(), "mine\n");
        // A free name: one new file of its own and nothing beside it.
        assert!(write("new-one.md").unwrap());
        assert_eq!(fs::read_to_string(dir.join("new-one.md")).unwrap(), "NEW\n");
        assert_eq!(fs::metadata(dir.join("new-one.md")).unwrap().nlink(), 1);
        let mut after = before.clone();
        after.push("new-one.md".into());
        after.sort();
        assert_eq!(names(dir), after);
        // Only a bare `<slug>.md` is a record file name.
        for name in [
            "../x.md",
            "a/b.md",
            "/tmp/x.md",
            ".hidden.md",
            "x.txt",
            "seed-log.jsonl",
            "X.md",
            "a\u{0}.md",
            "a.md.md",
            "..",
            ".md",
            "",
        ] {
            let e = write(name).unwrap_err().to_string();
            assert!(e.contains("not a record file name"), "{name:?}: {e}");
        }
        assert_eq!(names(dir), after);
        assert_eq!(names(outside), ["victim.txt"]);
    }

    #[test]
    fn a_failed_write_removes_its_own_partial_file_and_nothing_else() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("other.md"), "mine\n").unwrap();
        let e = create_exclusive(d.path(), "half.md", |f| {
            f.write_all(b"---\nid: half")?;
            Err(std::io::Error::other("disk full"))
        })
        .unwrap_err();
        assert!(e.to_string().contains("disk full"));
        assert_eq!(names(d.path()), ["other.md"]);
        assert_eq!(
            fs::read_to_string(d.path().join("other.md")).unwrap(),
            "mine\n"
        );
    }

    #[test]
    fn a_path_with_a_parent_component_is_refused_and_no_other() {
        for p in ["..", "../x", "a/../b", "/a/b/..", "/a/../../b", "./a/.."] {
            let e = refuse_parent_dir(Path::new(p), "the out").unwrap_err();
            assert!(e.to_string().contains("the out path has a `..`"), "{p}");
            assert!(resolve_out_dir(Path::new(p)).is_err(), "{p}");
        }
        for p in [
            ".", "./a", "a/./b/", "/a/b", "a..b/c", "...", "a/..b", "a/b../c",
        ] {
            assert!(refuse_parent_dir(Path::new(p), "the out").is_ok(), "{p}");
        }
        // A missing folder: absolute, the `.` gone, nothing made.
        let here = std::env::current_dir().unwrap();
        assert_eq!(
            resolve_out_dir(Path::new("./no-such-folder-xyz/./sub/")).unwrap(),
            here.join("no-such-folder-xyz/sub")
        );
        assert!(!here.join("no-such-folder-xyz").exists());
    }
}

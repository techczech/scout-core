//! The records folder under an edit: what was read ([`Snapshot`]), the
//! checks every change passes before a write ([`Snapshot::rewrite`],
//! [`Snapshot::verify`]) and the two ways a file is written ([`replace`],
//! [`create`]). Nothing here knows what an operation means.

use super::Step;
use crate::event_seed::render::{render_front, render_record};
use crate::event_seed::{self, fail_with};
use crate::events::{self, EventRecord, EventSet};
use crate::normalize::fold;
use anyhow::{bail, Context, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Called at [`Step::Replacing`]; an error stops the operation there.
pub(super) type Hook<'a> = &'a dyn Fn(Step) -> std::io::Result<()>;

/// A file in the records folder that an edit left behind when the process
/// died between writing it and renaming it. No record can have this name
/// (a record is `*.md` without a leading `.`).
pub(super) const TEMP_PREFIX: &str = ".scout-edit-";
const TEMP_SUFFIX: &str = ".tmp";

/// The records of the folder as one read gave them, with the bytes each
/// was read from.
pub(super) struct Snapshot {
    pub dir: PathBuf,
    pub set: EventSet,
    /// File name → the bytes read, for every file read as a record.
    files: BTreeMap<String, Vec<u8>>,
}

/// One record about to replace its file.
pub(super) struct Rewrite {
    /// The file's name in the records folder.
    pub file: String,
    /// The bytes the snapshot read from it.
    pub old: Vec<u8>,
    /// The bytes that replace them.
    pub new: Vec<u8>,
}

impl Rewrite {
    /// Would the write change the file?
    pub fn changes(&self) -> bool {
        self.old != self.new
    }
}

impl Snapshot {
    /// Read the folder with the reader ([`events::read_dir_with_bytes`]).
    /// Stops, like the seed, when the reader skips a file or does not
    /// follow a `merged_into`: an edit checks the folder as it would be,
    /// and cannot while part of it is not read.
    pub fn read(dir: &Path) -> Result<Snapshot> {
        let Some((set, files)) = events::read_dir_with_bytes(dir)? else {
            bail!("{}: no records folder (nothing was changed)", dir.display());
        };
        let mut faults: Vec<String> = set
            .refused
            .iter()
            .map(|r| {
                format!(
                    "{}: cannot read the record ({}); fix or remove it first",
                    dir.join(&r.file).display(),
                    r.why
                )
            })
            .collect();
        faults.extend(set.unmerged.iter().map(|u| {
            format!(
                "{}: merged_into {:?} {}; fix or clear merged_into first",
                dir.join(format!("{}.md", u.id)).display(),
                u.merged_into,
                u.why
            )
        }));
        refuse(faults)?;
        Ok(Snapshot {
            dir: dir.to_path_buf(),
            set,
            files,
        })
    }

    /// The record `id` names (ids compare under the reader's fold).
    pub fn record(&self, id: &str) -> Result<&EventRecord> {
        match self.set.records.get(&fold(id)) {
            Some(r) => Ok(r),
            None => bail!("{}: no such event record (nothing was changed)", id.trim()),
        }
    }

    pub fn has(&self, id: &str) -> bool {
        self.set.records.contains_key(&fold(id))
    }

    /// `new` as the bytes that replace its record's file, checked against
    /// the reader. `new_body` says the operation sets the description;
    /// without it the bytes after the frontmatter are carried over as they
    /// are in the file.
    ///
    /// Refused: a record whose file does not start with exactly the
    /// frontmatter the writer writes for it (other keys, comments, another
    /// YAML style, a byte-order mark, CR LF). Rewriting it would drop what
    /// the reader does not read, so it is not rewritten.
    pub fn rewrite(&self, new: &EventRecord, new_body: bool) -> Result<Rewrite> {
        let old_record = self.record(&new.id)?;
        if old_record.id != new.id {
            bail!("{}: an edit cannot change a record's id", old_record.id);
        }
        let file = format!("{}.md", new.id);
        let at = self.dir.join(&file);
        let Some(old) = self.files.get(&file) else {
            bail!(
                "{}: the record {:?} is not in a file of this name (nothing was changed)",
                at.display(),
                new.id
            );
        };
        let old_front = render_front(old_record);
        if !old.starts_with(old_front.as_bytes()) {
            bail!(
                "{}: the frontmatter is not as scout writes it (a key scout does not read, a comment, another YAML style or line ending); rewriting it would lose that, so the record is left alone (nothing was changed)",
                at.display()
            );
        }
        let tail = &old[old_front.len()..];
        let new_bytes = if new_body {
            render_record(new).into_bytes()
        } else {
            if new.description != old_record.description {
                bail!("{}: the description would change", at.display());
            }
            let mut bytes = render_front(new).into_bytes();
            bytes.extend_from_slice(tail);
            bytes
        };
        match events::parse_bytes(&new_bytes, &new.id) {
            Err(why) => bail!(
                "{}: scout would not read the record as changed ({why}) (nothing was changed)",
                at.display()
            ),
            Ok(back) if back != *new => bail!(
                "{}: the record as changed does not read back as meant (nothing was changed)",
                at.display()
            ),
            Ok(_) => {}
        }
        if !new_body && new_bytes[new_bytes.len() - tail.len()..] != *tail {
            bail!("{}: the body would change", at.display());
        }
        Ok(Rewrite {
            file,
            old: old.clone(),
            new: new_bytes,
        })
    }

    /// The folder as it would be: the records as their files have them,
    /// each of `changed` in place of the record of its id (or added). The
    /// reader must read every record, follow every `merged_into`, warn
    /// about nothing it does not warn about now, and its id table must be
    /// exactly `expected` (what the operation means to change, worked out
    /// from the table as it is now).
    pub fn verify(
        &self,
        changed: &[&EventRecord],
        expected: &BTreeMap<String, String>,
    ) -> Result<()> {
        let mut all: BTreeMap<String, EventRecord> = self
            .set
            .as_on_disk()
            .into_iter()
            .map(|r| (fold(&r.id), r))
            .collect();
        let mut extra = Vec::new();
        for r in changed {
            match all.get_mut(&fold(&r.id)) {
                Some(slot) => *slot = (*r).clone(),
                None => extra.push((*r).clone()),
            }
        }
        let records = all.into_values().chain(extra).collect();
        let after = EventSet::from_records(self.dir.clone(), records);
        let mut faults: Vec<String> = after
            .refused
            .iter()
            .map(|r| format!("{}: {}", r.file, r.why))
            .collect();
        faults.extend(
            after
                .unmerged
                .iter()
                .map(|u| format!("{}: merged_into {:?} {}", u.id, u.merged_into, u.why)),
        );
        let known: BTreeSet<&String> = self.set.warnings.iter().collect();
        faults.extend(
            after
                .warnings
                .iter()
                .filter(|w| !known.contains(w))
                .map(|w| format!("the reader would warn: {w}")),
        );
        let keys: BTreeSet<&String> = expected.keys().chain(after.aliases.keys()).collect();
        for k in keys {
            let (want, got) = (expected.get(k), after.aliases.get(k));
            if want != got {
                let show =
                    |v: Option<&String>| v.map_or("no event".to_string(), |v| format!("{v:?}"));
                faults.push(format!(
                    "{k}: would be held by {}, where this operation means {}",
                    show(got),
                    show(want)
                ));
            }
        }
        refuse(faults)
    }

    /// The lower-case names no new record may take (the seed's rule).
    pub fn names_in_use(&self) -> Result<BTreeSet<String>> {
        event_seed::names_in_use(&self.dir, &self.set)
    }
}

/// `Ok` for no faults, else one error listing them and saying that
/// nothing was changed.
pub(super) fn refuse(faults: Vec<String>) -> Result<()> {
    match fail_with(faults) {
        Ok(()) => Ok(()),
        // The seed's last line is about its own writes.
        Err(e) => bail!(
            "{}",
            e.to_string()
                .replace("\nnothing was written", "\nnothing was changed")
        ),
    }
}

/// What reading a plain file gave.
pub(super) struct Plain {
    pub bytes: Vec<u8>,
    /// Device and inode of the file read (0, 0 off unix).
    pub ident: (u64, u64),
}

pub(super) fn ident_of(meta: &std::fs::Metadata) -> (u64, u64) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        (meta.dev(), meta.ino())
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        (0, 0)
    }
}

/// `open` with no-follow (a symlink at the path is an error, not its
/// target) and non-blocking (a FIFO does not hang the open).
pub(super) fn no_follow(open: &mut std::fs::OpenOptions) -> &mut std::fs::OpenOptions {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        open.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    open
}

/// The whole of a plain file of at most `cap` bytes; `None` when nothing
/// is at `path`. A symlink (dangling or not), a folder or any other
/// non-regular file is refused, as is a larger file.
pub(super) fn read_plain(path: &Path, cap: u64) -> Result<Option<Plain>> {
    use std::io::Read;
    let at = path.display();
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).with_context(|| format!("look at {at}")),
        Ok(m) if !m.is_file() => {
            bail!("{at}: not a plain file (a symlink, a folder or a special file); nothing was changed")
        }
        Ok(_) => {}
    }
    let file = no_follow(std::fs::OpenOptions::new().read(true))
        .open(path)
        .with_context(|| format!("open {at}"))?;
    let meta = file.metadata().with_context(|| format!("look at {at}"))?;
    if !meta.is_file() {
        bail!("{at}: not a plain file; nothing was changed");
    }
    let mut bytes = Vec::new();
    file.take(cap + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("read {at}"))?;
    if bytes.len() as u64 > cap {
        bail!("{at}: larger than {cap} bytes; nothing was changed");
    }
    Ok(Some(Plain {
        bytes,
        ident: ident_of(&meta),
    }))
}

/// Is `<dir>/<file>` still a plain file holding exactly `old`? Read the
/// way the reader reads a record (no-follow, a regular file).
pub(super) fn still_as_read(dir: &Path, file: &str, old: &[u8]) -> Result<()> {
    let at = dir.join(file);
    let changed = || {
        format!(
            "{}: changed since it was read (edited by hand or by another tool meanwhile); it was not touched",
            at.display()
        )
    };
    match std::fs::symlink_metadata(&at) {
        Ok(m) if m.is_file() => {}
        Ok(_) => bail!(
            "{}: no longer a plain file (a symlink, a folder or a special file); it was not touched",
            at.display()
        ),
        Err(_) => bail!("{}", changed()),
    }
    match events::read_capped(&at) {
        Ok(Some(now)) if now == old => Ok(()),
        _ => bail!("{}", changed()),
    }
}

fn temp_name() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(
        "{TEMP_PREFIX}{}-{nanos:x}-{}{TEMP_SUFFIX}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    )
}

/// The names in `dir` an interrupted edit left behind.
pub(super) fn leftover_temps(dir: &Path) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let name = e
            .with_context(|| format!("read {}", dir.display()))?
            .file_name()
            .to_string_lossy()
            .into_owned();
        if name.starts_with(TEMP_PREFIX) && name.ends_with(TEMP_SUFFIX) {
            out.push(name);
        }
    }
    out.sort();
    Ok(out)
}

/// Make the rename (or the new file) last through a power cut, as far as
/// the platform allows. The change is already made, so a failure here is
/// not reported as one.
fn sync_dir(dir: &Path) {
    if let Ok(d) = std::fs::File::open(dir) {
        let _ = d.sync_all();
    }
}

/// Replace `<dir>/<w.file>` by `w.new`:
///
/// 1. a new file of a name no record can have is created in `dir`
///    (create-exclusive, no-follow), written and synced;
/// 2. the target is read again and must be a plain file holding exactly
///    the bytes the snapshot read; if not, the new file is removed and
///    the target is not touched;
/// 3. the new file is renamed over the target.
///
/// An error means the target was not replaced, and the new file is
/// removed. Between step 2 and
/// step 3 nothing guards the target: a write landing in that instant is
/// lost (see the module notes of [`super`]).
pub(super) fn replace(dir: &Path, w: &Rewrite, hook: Hook) -> Result<()> {
    let target = dir.join(&w.file);
    let temp = dir.join(temp_name());
    let mut open = std::fs::OpenOptions::new();
    open.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        open.custom_flags(libc::O_NOFOLLOW).mode(0o644);
    }
    let mut file = open
        .open(&temp)
        .with_context(|| format!("create {}", temp.display()))?;
    let done = (|| -> Result<()> {
        file.write_all(&w.new)
            .and_then(|_| file.sync_all())
            .with_context(|| format!("write {}", temp.display()))?;
        hook(Step::Replacing(&w.file))?;
        still_as_read(dir, &w.file, &w.old)?;
        std::fs::rename(&temp, &target).with_context(|| format!("rename over {}", target.display()))
    })();
    drop(file);
    if let Err(e) = done {
        let _ = std::fs::remove_file(&temp);
        return Err(e);
    }
    sync_dir(dir);
    Ok(())
}

/// Create the record `record` as `<dir>/<id>.md` through the seed's
/// create-exclusive, no-follow open; the bytes are synced.
pub(super) fn create(dir: &Path, record: &EventRecord, bytes: &[u8]) -> Result<()> {
    let name = format!("{}.md", record.id);
    let at = dir.join(&name);
    let made = event_seed::create_exclusive(dir, &name, |f| {
        f.write_all(bytes)?;
        f.sync_all()
    })
    .with_context(|| format!("write {}", at.display()))?;
    if !made {
        bail!(
            "{}: appeared after the names were looked up; it was not touched (nothing was changed)",
            at.display()
        );
    }
    sync_dir(dir);
    Ok(())
}

//! The permission bits of what the edits write: a replaced record keeps
//! the bits of the file it replaces; a record `create` makes, and a log
//! made by its first line, has 0644 whatever the umask. And a record with
//! a second hard link is replaced under its own name only.
//!
//! One test, alone in this file: it sets the process's umask.
#![cfg(unix)]

mod event_edit_support;
use event_edit_support::*;
use std::os::unix::fs::PermissionsExt;

fn mode(path: &Path) -> u32 {
    fs::symlink_metadata(path).unwrap().permissions().mode() & 0o7777
}

fn chmod(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

#[test]
fn a_replaced_record_keeps_its_mode_and_new_files_are_0644_whatever_the_umask() {
    // SAFETY: `umask` only sets the calling process's file mode mask.
    unsafe { libc::umask(0o077) };
    let fx = fixture();
    let at = |id: &str| fx.dir.join(format!("{id}.md"));
    assert_eq!(mode(&at(ALPHA)), 0o600);
    chmod(&at(ALPHA), 0o640);
    chmod(&at(BETA), 0o604);
    chmod(&at(GAMMA), 0o444);
    chmod(&at(DELTA), 0o666);
    let e = fx.editor();
    // set, remove, restore: one record each.
    e.set(
        ALPHA,
        &FieldEdit {
            name: Some("Alpha, renamed".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(fx.record(ALPHA).name, "Alpha, renamed");
    assert_eq!(mode(&at(ALPHA)), 0o640);
    // A record he made read-only is still replaced (the folder is his),
    // and is read-only after.
    e.remove(GAMMA).unwrap();
    assert!(fx.record(GAMMA).removed.is_some());
    assert_eq!(mode(&at(GAMMA)), 0o444);
    e.restore(GAMMA).unwrap();
    assert_eq!(mode(&at(GAMMA)), 0o444);
    // merge and unmerge: two records each, and the log made here.
    let m = e.merge(&merge_req(BETA, ALPHA, By::Dominik)).unwrap();
    assert_eq!((mode(&at(ALPHA)), mode(&at(BETA))), (0o640, 0o604));
    assert_eq!(mode(&fx.dir.join(MERGE_LOG)), 0o644);
    e.unmerge(&m.log_id).unwrap();
    assert_eq!((mode(&at(ALPHA)), mode(&at(BETA))), (0o640, 0o604));
    assert_eq!(mode(&fx.dir.join(MERGE_LOG)), 0o644);
    // create.
    let made = e
        .create(&NewEvent {
            name: "Made here".into(),
            start: Some("2026-10-01".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(mode(&at(&made.id)), 0o644);
    // decide: the decisions log made here.
    let folder = fx.dir.join(PROPOSALS_DIR);
    fs::create_dir_all(&folder).unwrap();
    let file = folder.join("2026-10-11-merge-1.jsonl");
    let line = json!({"a": DELTA, "b": BETA, "verdict": "same", "confidence": "doubtful"});
    fs::write(&file, format!("{line}\n")).unwrap();
    e.decide(&file, DELTA, BETA, Decision::Dismiss).unwrap();
    assert_eq!(mode(&fx.dir.join(DECISIONS_LOG)), 0o644);
    assert_eq!(mode(&at(DELTA)), 0o666);
    // No temporary file is left, and none was left readable meanwhile:
    // the temporary file is made 0600 and given the record's bits before
    // any byte is written.
    assert!(fs::read_dir(&fx.dir).unwrap().all(|e| !e
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with(".scout-edit-")));

    // A record with a second name outside the folder: the record's name
    // gets the new file; the other name keeps the old bytes and mode.
    let outside = tempfile::tempdir().unwrap();
    let other = outside.path().join("other-name.md");
    fs::hard_link(at(DELTA), &other).unwrap();
    let old = fs::read(&other).unwrap();
    e.set(
        DELTA,
        &FieldEdit {
            name: Some("Delta, renamed".into()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(fx.record(DELTA).name, "Delta, renamed");
    assert_eq!(fs::read(&other).unwrap(), old);
    assert_eq!((mode(&other), mode(&at(DELTA))), (0o666, 0o666));
    assert_eq!(fs::metadata(&other).unwrap().len(), old.len() as u64);
    use std::os::unix::fs::MetadataExt;
    assert_eq!(fs::metadata(&other).unwrap().nlink(), 1);
}

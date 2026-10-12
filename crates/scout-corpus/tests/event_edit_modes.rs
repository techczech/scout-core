//! The permission bits of what scout writes beside a corpus: a replaced
//! record keeps the bits of the file it replaces; a NEW file (a record
//! `create` or the seed makes, a log or the corrections file made by its
//! first line) is created with the default mode of a new file, so its
//! bits are what the process's umask leaves of 0666, and nothing sets a
//! mode on it afterwards. And a record with a second hard link is
//! replaced under its own name only.
//!
//! One test, alone in this file: it sets the process's umask.
#![cfg(unix)]

mod event_edit_support;
use event_edit_support::*;
use scout_corpus::corrections::CorrectionEditor;
use scout_corpus::event_seed::{seed, SeedOptions};
use scout_corpus::topics;
use std::os::unix::fs::PermissionsExt;

fn mode(path: &Path) -> u32 {
    fs::symlink_metadata(path).unwrap().permissions().mode() & 0o7777
}

fn chmod(path: &Path, mode: u32) {
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

fn umask(mask: u32) {
    // SAFETY: `umask` only sets the calling process's file mode mask.
    unsafe { libc::umask(mask as libc::mode_t) };
}

#[test]
fn a_replaced_record_keeps_its_mode_and_a_new_file_has_what_the_umask_leaves() {
    umask(0o077);
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
    // merge and unmerge: one record each (its bits kept), and the log
    // made here under umask 077: 0600. A later append leaves its bits.
    let m = e.merge(&merge_req(BETA, ALPHA, By::Dominik)).unwrap();
    assert_eq!((mode(&at(ALPHA)), mode(&at(BETA))), (0o640, 0o604));
    assert_eq!(mode(&fx.dir.join(MERGE_LOG)), 0o600);
    umask(0o022);
    e.unmerge(&m.log_id).unwrap();
    assert_eq!((mode(&at(ALPHA)), mode(&at(BETA))), (0o640, 0o604));
    assert_eq!(mode(&fx.dir.join(MERGE_LOG)), 0o600);

    // New files under three umasks: each has what the umask leaves of
    // 0666, never a mode of scout's own choosing.
    // create.
    umask(0o027);
    let made = e
        .create(&NewEvent {
            name: "Made here".into(),
            start: Some("2026-10-01".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(mode(&at(&made.id)), 0o640);
    umask(0o002);
    let wide = e
        .create(&NewEvent {
            name: "Made wide".into(),
            start: Some("2026-10-02".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(mode(&at(&wide.id)), 0o664);
    // decide: the decisions log made here.
    umask(0o026);
    let folder = fx.dir.join(PROPOSALS_DIR);
    fs::create_dir_all(&folder).unwrap();
    let file = folder.join("2026-10-11-merge-1.jsonl");
    let line = json!({"a": DELTA, "b": BETA, "verdict": "same", "confidence": "doubtful"});
    fs::write(&file, format!("{line}\n")).unwrap();
    e.decide(&file, DELTA, BETA, Decision::Dismiss).unwrap();
    assert_eq!(mode(&fx.dir.join(DECISIONS_LOG)), 0o640);
    assert_eq!(mode(&at(DELTA)), 0o666);
    // The corrections file, made by its first line.
    umask(0o037);
    let layer = fx.root().join(topics::LAYER_FILE);
    fs::create_dir_all(layer.parent().unwrap()).unwrap();
    fs::write(
        &layer,
        "{\"key\":\"x:1\",\"themes\":[],\"entities\":[],\"event\":\"2026-09/E01\"}\n",
    )
    .unwrap();
    let corrections = fx.root().join(topics::CORRECTIONS_FILE);
    let c = CorrectionEditor::open(fx.root()).unwrap();
    c.add_entity("x:1", "E", "person").unwrap();
    assert_eq!(mode(&corrections), 0o640);
    umask(0o077);
    c.add_entity("x:1", "F", "person").unwrap();
    assert_eq!(mode(&corrections), 0o640);
    // A record the seed makes.
    umask(0o007);
    let briefings = fx.root().join("analysis/context/briefings");
    fs::create_dir_all(&briefings).unwrap();
    fs::write(
        briefings.join("2026-07-events.json"),
        json!({"events": [{"id": "E01", "name": "Seeded here", "start": "2026-07-01"}]})
            .to_string(),
    )
    .unwrap();
    let report = seed(&SeedOptions {
        archive: fx.root().to_path_buf(),
        out: None,
        apply: true,
    })
    .unwrap();
    assert_eq!(report.created, 1, "{report:?}");
    let seeded = fx.set().resolve("2026-07/E01").unwrap().to_string();
    assert_eq!(mode(&at(&seeded)), 0o660);
    umask(0o077);
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

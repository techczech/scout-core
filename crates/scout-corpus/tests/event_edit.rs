//! Seam tests for the one-record edits of `scout events`: `set`,
//! `create`, `remove`, `restore`. Each success is read back with the
//! reader; each refusal must leave every file under the archive byte for
//! byte as it was. Merges are in `event_edit_merge.rs`, proposals in
//! `event_edit_proposals.rs`.

mod event_edit_support;
use event_edit_support::*;

fn edit() -> FieldEdit {
    FieldEdit::default()
}

fn temp_files(dir: &Path) -> Vec<String> {
    fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.starts_with(".scout-edit-"))
        .collect()
}

#[test]
fn set_changes_the_named_fields_marks_edited_and_keeps_every_other_byte() {
    let fx = fixture();
    let before = fx.tree();
    let (was, body) = (fx.record(ALPHA), body_bytes(&fx.dir, ALPHA));
    let r = fx
        .editor()
        .set(
            &format!(" {} ", ALPHA.to_uppercase()),
            &FieldEdit {
                name: Some("  He said: \"it's #1\" — naïve  ".into()),
                start: Some("2026-09-01".into()),
                end: Some("".into()),
                add_aliases: strs(&["New: alias", "the alpha"]),
                remove_aliases: strs(&["SHARED"]),
                ..edit()
            },
        )
        .unwrap();
    assert!(r.changed);
    assert_eq!(
        (r.tool.as_str(), r.id.as_str(), r.at.as_str()),
        ("scout events set", ALPHA, NOW)
    );
    let now = fx.record(ALPHA);
    assert_eq!(now, r.record);
    assert_eq!(
        now,
        EventRecord {
            name: "He said: \"it's #1\" — naïve".into(),
            start: Some("2026-09-01".into()),
            end: None,
            aliases: strs(&["The Alpha", "New: alias"]),
            edited: Some(TODAY.into()),
            ..was
        }
    );
    // The bytes after the frontmatter are the ones that were there, and
    // no other file changed.
    assert_eq!(body_bytes(&fx.dir, ALPHA), body);
    let mut after = fx.tree();
    after.insert(
        fx.dir.join(format!("{ALPHA}.md")),
        before[&fx.dir.join(format!("{ALPHA}.md"))].clone(),
    );
    assert!(after == before);
    assert!(temp_files(&fx.dir).is_empty());
    // The same values again: the file already is as asked.
    let again = fx.editor().set(
        ALPHA,
        &FieldEdit {
            start: Some("2026-09-01".into()),
            ..edit()
        },
    );
    assert!(!again.unwrap().changed);
}

#[test]
fn set_description_replaces_the_body_cleaned_as_the_seed_cleans_one() {
    let fx = fixture();
    let r = fx
        .editor()
        .set(
            GAMMA,
            &FieldEdit {
                description: Some(
                    "  new\u{0} text\rmac line\u{2028}sep\u{feff}\n---\nkept: \"all\"  ".into(),
                ),
                ..edit()
            },
        )
        .unwrap();
    assert_eq!(
        r.record.description,
        "new text\nmac line\nsep\n---\nkept: \"all\""
    );
    let now = fx.record(GAMMA);
    assert_eq!(now, r.record);
    // His earlier edit date is replaced; nothing else of the record moved.
    assert_eq!(now.edited.as_deref(), Some(TODAY));
    assert_eq!(
        (now.origin.as_deref(), now.aliases, now.members),
        (
            Some("dominik"),
            strs(&["His alias"]),
            strs(&["2026-11/E02"])
        )
    );
    assert_eq!(fx.bytes(GAMMA), render_record(&r.record).into_bytes());
    // An empty description: an empty body.
    let r = fx
        .editor()
        .set(
            GAMMA,
            &FieldEdit {
                description: Some(" \n".into()),
                ..edit()
            },
        )
        .unwrap();
    assert_eq!(fx.record(GAMMA).description, "");
    assert!(r.changed);
}

#[test]
fn a_body_he_wrote_by_hand_is_carried_over_byte_for_byte() {
    let fx = fixture();
    // The writer's frontmatter, then a body no writer would write: blank
    // lines, trailing spaces, CR LF, no final newline.
    let mut hand = fx.bytes(GAMMA);
    let body = body_bytes(&fx.dir, GAMMA);
    hand.truncate(hand.len() - body.len());
    let his = b"\n\n\nFirst line.  \r\nSecond\tline.\n\n---\nid: not-frontmatter\n---\n   last, no newline";
    hand.extend_from_slice(his);
    fs::write(fx.dir.join(format!("{GAMMA}.md")), &hand).unwrap();
    let e = fx.editor();
    e.set(
        GAMMA,
        &FieldEdit {
            name: Some("Gamma renamed".into()),
            ..edit()
        },
    )
    .unwrap();
    assert_eq!(body_bytes(&fx.dir, GAMMA), his);
    e.remove(GAMMA).unwrap();
    assert_eq!(body_bytes(&fx.dir, GAMMA), his);
    e.restore(GAMMA).unwrap();
    assert_eq!(body_bytes(&fx.dir, GAMMA), his);
    e.merge(&merge_req(DELTA, GAMMA, By::Dominik)).unwrap();
    assert_eq!(body_bytes(&fx.dir, GAMMA), his);
    assert_eq!(fx.record(GAMMA).name, "Gamma renamed");
}

#[test]
fn set_refuses_bad_input_and_changes_nothing() {
    let fx = fixture();
    let e = fx.editor();
    let set = |id: &str, f: FieldEdit| e.set(id, &f);
    let name = |n: &str| FieldEdit {
        name: Some(n.into()),
        ..edit()
    };
    refused(&fx, "no such event record", || {
        set("2026-09-nobody", name("x"))
    });
    refused(&fx, "no such event record", || {
        set("2026-09/E01", name("x"))
    });
    refused(&fx, "nothing to set", || set(ALPHA, edit()));
    refused(&fx, "the name is empty", || set(ALPHA, name("  ")));
    for bad in [
        "two\nlines",
        "tab\there",
        "nul\u{0}",
        "esc\u{1b}[0m",
        "sep\u{2028}",
        "bom\u{feff}",
        "c1\u{85}",
    ] {
        refused(&fx, "the name holds a control character", || {
            set(ALPHA, name(bad))
        });
        refused(&fx, "an alias holds a control character", || {
            set(
                ALPHA,
                FieldEdit {
                    add_aliases: strs(&[bad]),
                    ..edit()
                },
            )
        });
    }
    refused(&fx, "an alias is empty", || {
        set(
            ALPHA,
            FieldEdit {
                add_aliases: strs(&["ok", " "]),
                ..edit()
            },
        )
    });
    refused(&fx, "has no alias \"Nope\" to remove", || {
        set(
            ALPHA,
            FieldEdit {
                remove_aliases: strs(&["Nope"]),
                ..edit()
            },
        )
    });
    for bad in [
        "2026-9-1",
        "2026-02-30",
        "",
        "tomorrow",
        "2026-09-01T00:00:00Z",
    ] {
        refused(&fx, "is not a date YYYY-MM-DD", || {
            set(
                ALPHA,
                FieldEdit {
                    start: Some(bad.into()),
                    ..edit()
                },
            )
        });
    }
    refused(&fx, "end \"09/2026\" is not a date", || {
        set(
            ALPHA,
            FieldEdit {
                end: Some("09/2026".into()),
                ..edit()
            },
        )
    });
    refused(&fx, "end 2026-09-04 is before start 2026-09-05", || {
        set(
            ALPHA,
            FieldEdit {
                end: Some("2026-09-04".into()),
                ..edit()
            },
        )
    });
    refused(&fx, "is before start 2026-12-01", || {
        set(
            ALPHA,
            FieldEdit {
                start: Some("2026-12-01".into()),
                ..edit()
            },
        )
    });
    // A record the reader would not read back: over its size cap.
    refused(&fx, "larger than 1 MiB", || {
        set(
            ALPHA,
            FieldEdit {
                description: Some("x".repeat(1 << 20)),
                ..edit()
            },
        )
    });
    refused(&fx, "larger than 1 MiB", || {
        set(ALPHA, name(&"n".repeat(1 << 20)))
    });
}

#[test]
fn a_record_that_is_not_as_the_writer_writes_it_is_left_alone() {
    let fx = fixture();
    let e = fx.editor();
    for (id, text) in [
        ("extra-key", "---\nid: \"extra-key\"\nname: \"X\"\naliases: []\nstart:\nend:\nmembers: []\nthemes: []\norigin:\nedited:\nmerged_into:\nnote: \"his own key\"\n---\n\nBody.\n"),
        ("plain-style", "---\nid: plain-style\nname: X\n---\n\nBody.\n"),
        ("commented", "---\nid: \"commented\" # mine\nname: \"X\"\naliases: []\nstart:\nend:\nmembers: []\nthemes: []\norigin:\nedited:\nmerged_into:\n---\n"),
        ("crlf", "---\r\nid: \"crlf\"\r\nname: \"X\"\r\n---\r\n"),
        ("bom", "\u{feff}---\nid: \"bom\"\nname: \"bom\"\naliases: []\nstart:\nend:\nmembers: []\nthemes: []\norigin:\nedited:\nmerged_into:\n---\n\n"),
    ] {
        fs::write(fx.dir.join(format!("{id}.md")), text).unwrap();
        // The reader reads it; the edits do not rewrite it.
        assert!(fx.set().records.contains_key(id), "{id}");
        let why = "the frontmatter is not as scout writes it";
        refused(&fx, why, || e.set(id, &FieldEdit { name: Some("Y".into()), ..edit() }));
        refused(&fx, why, || e.set(id, &FieldEdit { description: Some("Y".into()), ..edit() }));
        refused(&fx, why, || e.remove(id));
        refused(&fx, why, || e.merge(&merge_req(id, ALPHA, By::Dominik)));
        // Merging another record INTO it does not write it: allowed, and
        // its file stays as he has it, through the merge and its undo.
        let m = e.merge(&merge_req(ALPHA, id, By::Dominik)).unwrap();
        assert_eq!(fx.set().resolve("2026-09/E01"), Some(id));
        e.unmerge(&m.log_id).unwrap();
        assert_eq!(fs::read_to_string(fx.dir.join(format!("{id}.md"))).unwrap(), text);
    }
    // A record in a file of another name (`x.md.md` holds the id `x`).
    let twice = rec("twice", "Twice", ("2026-01-01", "2026-01-02"), &[]);
    fs::write(fx.dir.join("twice.md.md"), render_record(&twice)).unwrap();
    refused(&fx, "is not in a file of this name", || e.remove("twice"));
}

#[test]
fn a_folder_the_reader_does_not_read_whole_stops_every_edit() {
    let fx = fixture();
    let e = fx.editor();
    fs::write(fx.dir.join("broken.md"), "---\nid: other\n---\n").unwrap();
    let all = |fx: &Fx, why: &str| {
        refused(fx, why, || {
            e.set(
                ALPHA,
                &FieldEdit {
                    name: Some("x".into()),
                    ..edit()
                },
            )
        });
        refused(fx, why, || e.remove(ALPHA));
        refused(fx, why, || e.restore(ALPHA));
        refused(fx, why, || {
            e.create(&NewEvent {
                name: "x".into(),
                ..Default::default()
            })
        });
        refused(fx, why, || e.merge(&merge_req(ALPHA, BETA, By::Dominik)));
        refused(fx, why, || e.unmerge("m000001"));
        refused(fx, why, || e.check());
    };
    all(
        &fx,
        "broken.md: cannot read the record (id \"other\" does not match the file name)",
    );
    fs::remove_file(fx.dir.join("broken.md")).unwrap();
    // A `merged_into` the reader does not follow.
    let mut dangling = rec("2026-01-dangling", "D", ("2026-01-01", "2026-01-02"), &[]);
    dangling.merged_into = Some("2026-01-gone".into());
    put(&fx.dir, &dangling);
    all(&fx, "merged_into \"2026-01-gone\" names no record");
}

#[cfg(unix)]
#[test]
fn the_archive_path_follows_the_seed_rules() {
    let fx = fixture();
    let open = |p: PathBuf| EventEditor::open(&p).map(|_| ()).unwrap_err().to_string();
    let e = open(fx.root().join("sub/.."));
    assert!(e.contains("the archive path has a `..` component"), "{e}");
    let e = open(fx.root().join("no-such-archive"));
    assert!(e.contains("the archive is not a folder"), "{e}");
    // No records folder: no edit makes one.
    let empty = tempfile::tempdir().unwrap();
    let e = open(empty.path().to_path_buf());
    assert!(e.contains("no records folder"), "{e}");
    assert!(fs::read_dir(empty.path()).unwrap().next().is_none());
    // The records folder is a symlink to a real one.
    fs::create_dir_all(empty.path().join("analysis")).unwrap();
    std::os::unix::fs::symlink(&fx.dir, empty.path().join(events::EVENTS_DIR)).unwrap();
    let before = fx.tree();
    let e = format!(
        "{:#}",
        EventEditor::open(empty.path()).map(|_| ()).unwrap_err()
    );
    assert!(e.contains("is a symlink"), "{e}");
    assert!(fx.tree() == before);
}

#[test]
fn a_record_changed_between_the_read_and_the_write_is_not_replaced() {
    for at in ["read", "replacing"] {
        let fx = fixture();
        let target = fx.dir.join(format!("{ALPHA}.md"));
        let his = "---\nid: \"2026-09-alpha\"\nname: \"His edit, made meanwhile\"\n---\n";
        let (t, when) = (target.clone(), at.to_string());
        let e = fx.editor().with_hook(move |s| match (when.as_str(), s) {
            ("read", Step::Read) | ("replacing", Step::Replacing(_)) => fs::write(&t, his),
            _ => Ok(()),
        });
        let mut before = fx.tree();
        let err = e
            .set(
                ALPHA,
                &FieldEdit {
                    name: Some("Mine".into()),
                    ..edit()
                },
            )
            .unwrap_err();
        assert!(
            err.to_string().contains("changed since it was read"),
            "{at}: {err}"
        );
        // His bytes stand, no temporary file is left, nothing else moved.
        before.insert(target.clone(), his.as_bytes().to_vec());
        assert!(fx.tree() == before, "{at}");
        assert!(temp_files(&fx.dir).is_empty(), "{at}");
    }
}

#[cfg(unix)]
#[test]
fn a_target_that_became_a_link_is_not_written_through() {
    let fx = fixture();
    let outside = tempfile::tempdir().unwrap();
    let victim = outside.path().join("victim.md");
    fs::write(&victim, fx.bytes(ALPHA)).unwrap();
    let (target, v) = (fx.dir.join(format!("{ALPHA}.md")), victim.clone());
    let e = fx.editor().with_hook(move |s| {
        if let Step::Replacing(_) = s {
            fs::remove_file(&target)?;
            std::os::unix::fs::symlink(&v, &target)?;
        }
        Ok(())
    });
    let kept = fs::read(&victim).unwrap();
    let err = e.remove(ALPHA).unwrap_err().to_string();
    assert!(err.contains("no longer a plain file"), "{err}");
    assert_eq!(fs::read(&victim).unwrap(), kept);
    assert!(fs::symlink_metadata(fx.dir.join(format!("{ALPHA}.md")))
        .unwrap()
        .file_type()
        .is_symlink());
    assert!(temp_files(&fx.dir).is_empty());
}

#[test]
fn remove_marks_the_record_and_restore_gives_back_its_bytes() {
    let fx = fixture();
    let e = fx.editor();
    let before = fx.tree();
    let table = fx.set().aliases;
    let r = e.remove(GAMMA).unwrap();
    assert!(r.changed);
    let now = fx.record(GAMMA);
    assert_eq!(now.removed.as_deref(), Some(TODAY));
    // Kept: the record, its hold on its ids, his edit date, his body.
    assert_eq!(now.edited.as_deref(), Some("2026-10-10"));
    assert_eq!(now.description, GAMMA_BODY);
    let set = fx.set();
    assert_eq!(set.aliases, table);
    assert_eq!(set.resolve("2026-11/E02"), Some(GAMMA));
    assert!(set.is_removed("2026-11/E02") && set.is_removed(GAMMA));
    assert!(!set.is_removed(ALPHA) && !set.is_removed("2026-01/E99"));
    assert!(String::from_utf8(fx.bytes(GAMMA))
        .unwrap()
        .contains("merged_into:\nremoved: \"2026-10-12\"\n---\n"));
    // Again: nothing to do.
    let again = fx.tree();
    assert!(!e.remove(GAMMA).unwrap().changed);
    assert!(fx.tree() == again);
    // A removed record takes no part in a merge.
    refused(&fx, "a removed event; restore it first", || {
        e.merge(&merge_req(GAMMA, ALPHA, By::Dominik))
    });
    refused(&fx, "a removed event; restore it first", || {
        e.merge(&merge_req(ALPHA, GAMMA, By::Dominik))
    });
    // Restore: the folder is byte for byte what it was.
    assert!(e.restore(GAMMA).unwrap().changed);
    assert!(fx.tree() == before);
    assert!(!e.restore(GAMMA).unwrap().changed);
    assert!(fx.tree() == before);
    refused(&fx, "no such event record", || e.remove("2026-01-nobody"));
    refused(&fx, "no such event record", || e.restore("2026-01-nobody"));
    // A record merged into another is not an event to remove.
    e.merge(&merge_req(ALPHA, BETA, By::Dominik)).unwrap();
    refused(
        &fx,
        "merged into 2026-10-beta; remove 2026-10-beta, or undo the merge first",
        || e.remove(ALPHA),
    );
}

#[test]
fn create_writes_a_new_record_of_his_under_a_seed_style_id() {
    let fx = fixture();
    let e = fx.editor();
    let before = fx.tree();
    let r = e
        .create(&NewEvent {
            name: " Fields Medallists' declaration: the \"long\" story ".into(),
            description: Some("What it is.\u{0}\r\nSecond line.".into()),
            start: Some("2026-07-04".into()),
            end: Some("2026-07-09".into()),
        })
        .unwrap();
    let id = "2026-07-fields-medallists-declaration-the-long-story";
    assert_eq!((r.id.as_str(), r.changed), (id, true));
    let want = EventRecord {
        id: id.into(),
        name: "Fields Medallists' declaration: the \"long\" story".into(),
        start: Some("2026-07-04".into()),
        end: Some("2026-07-09".into()),
        origin: Some("dominik".into()),
        edited: Some(TODAY.into()),
        description: "What it is.\r\nSecond line.".into(),
        ..EventRecord::default()
    };
    assert_eq!(r.record, want);
    assert_eq!(fx.record(id), want);
    assert_eq!(fx.bytes(id), render_record(&want).into_bytes());
    // One new file, nothing else touched.
    let mut after = fx.tree();
    assert!(after.remove(&fx.dir.join(format!("{id}.md"))).is_some());
    assert!(after == before);
    // The same name again takes a suffix; no start: this month; a name
    // without a slug: the seed's stand-in.
    let two = e
        .create(&NewEvent {
            name: "Fields Medallists' declaration: the long story".into(),
            start: Some("2026-07-30".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(two.id, format!("{id}-2"));
    let bare = e
        .create(&NewEvent {
            name: "Alpha".into(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        (bare.id.as_str(), &bare.record.start, &bare.record.end),
        ("2026-10-alpha", &None, &None)
    );
    assert_eq!(
        e.create(&NewEvent {
            name: "日本語".into(),
            ..Default::default()
        })
        .unwrap()
        .id,
        "2026-10-event-x"
    );
    // An id in use, by a record or by any entry of the folder, is kept off.
    assert_eq!(
        e.create(&NewEvent {
            name: "Alpha".into(),
            start: Some("2026-09-01".into()),
            ..Default::default()
        })
        .unwrap()
        .id,
        "2026-09-alpha-2"
    );
    fs::create_dir(fx.dir.join("2026-09-taken.md")).unwrap();
    let e2 = fx.editor();
    let taken = NewEvent {
        name: "Taken".into(),
        start: Some("2026-09-01".into()),
        ..Default::default()
    };
    // (the folder under a record name is one the reader refuses)
    refused(
        &fx,
        "2026-09-taken.md: cannot read the record (not a plain file)",
        || e2.create(&taken),
    );
    fs::remove_dir(fx.dir.join("2026-09-taken.md")).unwrap();
    fs::write(fx.dir.join("2026-09-taken"), "not a record\n").unwrap();
    assert_eq!(e2.create(&taken).unwrap().id, "2026-09-taken-2");
    read_all(&fx.dir);
}

#[test]
fn create_refuses_bad_input_and_changes_nothing() {
    let fx = fixture();
    let e = fx.editor();
    let new = |name: &str, start: Option<&str>, end: Option<&str>| NewEvent {
        name: name.into(),
        description: None,
        start: start.map(String::from),
        end: end.map(String::from),
    };
    refused(&fx, "the name is empty", || e.create(&new(" ", None, None)));
    refused(&fx, "the name holds a control character", || {
        e.create(&new("a\nb", None, None))
    });
    refused(&fx, "start \"2026-07\" is not a date", || {
        e.create(&new("x", Some("2026-07"), None))
    });
    refused(&fx, "end \"soon\" is not a date", || {
        e.create(&new("x", Some("2026-07-01"), Some("soon")))
    });
    refused(&fx, "end 2026-06-30 is before start 2026-07-01", || {
        e.create(&new("x", Some("2026-07-01"), Some("2026-06-30")))
    });
    refused(&fx, "larger than 1 MiB", || {
        e.create(&new(&"long ".repeat(300_000), None, None))
    });
    // The name appears between the look-up and the create: not touched.
    let dir = fx.dir.clone();
    let racing = fx.editor().with_hook(move |s| match s {
        Step::Read => fs::write(dir.join("2026-10-racer.md"), "his\n"),
        _ => Ok(()),
    });
    let err = racing
        .create(&new("Racer", None, None))
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("appeared after the names were looked up"),
        "{err}"
    );
    assert_eq!(
        fs::read_to_string(fx.dir.join("2026-10-racer.md")).unwrap(),
        "his\n"
    );
}

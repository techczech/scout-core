//! The layer files (`analysis/topics/current.jsonl` and
//! `corrections.jsonl`) are read only when they are regular files (review
//! finding, 2026-10-13). A link to a regular file is followed; a link to a
//! device, a FIFO, a socket or a directory at either name is an error that
//! names the path, for the layer reader and for an index build alike, and at once:
//! every read here runs under a time limit, so a reader that waits on a
//! FIFO or reads `/dev/zero` without end fails its test instead of hanging
//! the suite.
#![cfg(unix)]

use scout_corpus::registry::{CorpusConfig, CorpusKind, DocumentUnit, FieldMap};
use scout_corpus::{topics, Corpus, SearchRequest};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

const LIMIT: Duration = Duration::from_secs(20);

/// `f` on a thread of its own; a panic when it has not answered in
/// [`LIMIT`] (the thread is then left behind, and ends with the process).
fn within<T: Send + 'static>(what: &str, f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(LIMIT)
        .unwrap_or_else(|_| panic!("{what}: no answer within {LIMIT:?}"))
}

/// What stands at a layer file's name instead of a regular file.
#[derive(Debug, Clone, Copy)]
enum Bad {
    /// A link to `/dev/zero`: bytes without end.
    LinkToDevice,
    /// A FIFO nobody writes to: a plain open of it waits.
    Fifo,
    Directory,
    /// A link to a directory.
    LinkToDirectory,
    /// A Unix-domain socket: it cannot be opened as a file at all.
    Socket,
}

const BAD: [Bad; 5] = [
    Bad::LinkToDevice,
    Bad::Fifo,
    Bad::Directory,
    Bad::LinkToDirectory,
    Bad::Socket,
];

/// Put `bad` at `path`, in place of whatever is there.
fn put_bad(path: &Path, bad: Bad) {
    if fs::remove_file(path).is_err() {
        let _ = fs::remove_dir(path);
    }
    match bad {
        Bad::LinkToDevice => std::os::unix::fs::symlink("/dev/zero", path).unwrap(),
        Bad::Fifo => {
            use std::os::unix::ffi::OsStrExt;
            let c = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
            assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
        }
        Bad::Directory => fs::create_dir(path).unwrap(),
        Bad::LinkToDirectory => {
            let dir = path.with_extension("d");
            fs::create_dir_all(&dir).unwrap();
            std::os::unix::fs::symlink(&dir, path).unwrap();
        }
        Bad::Socket => {
            // Bound under a short name (a socket's path has a small
            // length limit) and moved into place: the name is the socket.
            let short = tempfile::tempdir().unwrap();
            let made = short.path().join("s");
            drop(std::os::unix::net::UnixListener::bind(&made).unwrap());
            let moved = path.with_extension("s");
            // Into the archive's volume first when the two differ.
            if fs::rename(&made, &moved).is_err() {
                panic!("cannot move a socket to {}", moved.display());
            }
            fs::rename(&moved, path).unwrap();
        }
    }
}

/// An archive with one work, two tag rows and one correction, and a data
/// folder for its index.
struct Fx {
    tmp: tempfile::TempDir,
    data: tempfile::TempDir,
}

const LAYER: &str = concat!(
    r#"{"key":"x:1","date":"2026-09-06","themes":["T"],"entities":[]}"#,
    "\n",
    r#"{"key":"x:2","date":"2026-09-07","themes":["U"],"entities":[]}"#,
    "\n"
);
const CORRECTIONS: &str = concat!(r#"{"key":"x:1","themes":["Corrected"]}"#, "\n");

impl Fx {
    fn new() -> Fx {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("readings/works")).unwrap();
        fs::write(
            root.join("readings/works/w-1.md"),
            "---\ntitle: W1\nauthor: A\ntype: tweet\nsource_system: x\nsource_id: \"1\"\n---\n\n> A proof.\n\nhighlighted_at: 2026-09-01\n\n---\n",
        )
        .unwrap();
        let layer = root.join(topics::LAYER_FILE);
        fs::create_dir_all(layer.parent().unwrap()).unwrap();
        fs::write(&layer, LAYER).unwrap();
        fs::write(root.join(topics::CORRECTIONS_FILE), CORRECTIONS).unwrap();
        Fx {
            tmp,
            data: tempfile::tempdir().unwrap(),
        }
    }

    fn root(&self) -> PathBuf {
        self.tmp.path().to_path_buf()
    }

    fn corpus(&self) -> Corpus {
        let config = CorpusConfig {
            id: "highlights".into(),
            name: "Highlights".into(),
            kind: CorpusKind::HighlightScoutArchive,
            path: self.tmp.path().display().to_string(),
            include: vec![],
            exclude: vec![],
            require_frontmatter: vec![],
            field_map: FieldMap::default(),
            default_author: None,
            boilerplate: vec![],
            link: None,
            document_unit: DocumentUnit::File,
        };
        Corpus::from_config(config, self.data.path())
    }

    /// The layer reader's answer, in words when it is an error.
    fn read(&self) -> Result<Option<topics::Layer>, String> {
        let root = self.root();
        within("read_layer", move || {
            topics::read_layer(&root).map_err(|e| format!("{e:#}"))
        })
    }

    /// An index build's answer: how many corrections it applied, or the
    /// error in words.
    fn build(&self) -> Result<usize, String> {
        let corpus = self.corpus();
        within("build_index", move || {
            corpus
                .build_index(false)
                .map(|r| r.topics.expect("a layer").corrections)
                .map_err(|e| format!("{e:#}"))
        })
    }
}

/// Both readers refuse `file` when `bad` stands at its name, saying which
/// path, that it is not a regular file, and what it is.
fn refused(file: &str, bad: Bad) {
    let what = match bad {
        Bad::LinkToDevice => "(a device)",
        Bad::Fifo => "(a FIFO)",
        Bad::Directory | Bad::LinkToDirectory => "(a directory)",
        Bad::Socket => "(a socket)",
    };
    let fx = Fx::new();
    let path = fx.root().join(file);
    put_bad(&path, bad);
    for (who, e) in [
        ("read_layer", fx.read().map(|_| ()).unwrap_err()),
        ("build_index", fx.build().map(|_| ()).unwrap_err()),
    ] {
        assert!(
            e.contains("is not a regular file")
                && e.contains(what)
                && e.contains(&path.display().to_string()),
            "{who} over {file} as {bad:?}: {e}"
        );
    }
}

#[test]
fn tag_rows_that_are_a_link_to_a_device_are_refused_at_once() {
    refused(topics::LAYER_FILE, Bad::LinkToDevice);
}

#[test]
fn tag_rows_that_are_a_fifo_are_refused_without_waiting_for_a_writer() {
    refused(topics::LAYER_FILE, Bad::Fifo);
}

#[test]
fn tag_rows_that_are_a_directory_or_a_link_to_one_are_refused() {
    refused(topics::LAYER_FILE, Bad::Directory);
    refused(topics::LAYER_FILE, Bad::LinkToDirectory);
}

#[test]
fn corrections_that_are_a_link_to_a_device_are_refused_at_once() {
    refused(topics::CORRECTIONS_FILE, Bad::LinkToDevice);
}

#[test]
fn corrections_that_are_a_fifo_are_refused_without_waiting_for_a_writer() {
    refused(topics::CORRECTIONS_FILE, Bad::Fifo);
}

#[test]
fn corrections_that_are_a_directory_or_a_link_to_one_are_refused() {
    refused(topics::CORRECTIONS_FILE, Bad::Directory);
    refused(topics::CORRECTIONS_FILE, Bad::LinkToDirectory);
}

/// A socket is refused by the check before the open, with the same error
/// as the rest (an open of it fails with an error of the system's own).
#[test]
fn a_socket_at_either_name_is_refused_as_not_a_regular_file() {
    refused(topics::LAYER_FILE, Bad::Socket);
    refused(topics::CORRECTIONS_FILE, Bad::Socket);
}

/// A refusal is not a missing file: the build does not go on without the
/// layer it could not read, and after an ordinary build (not forced, the
/// configuration unchanged) an index built before keeps what it held.
#[test]
fn a_build_over_a_refused_file_fails_and_leaves_the_index_as_it_was() {
    for file in [topics::LAYER_FILE, topics::CORRECTIONS_FILE] {
        let fx = Fx::new();
        assert_eq!(fx.build(), Ok(1));
        let before = fx.corpus().search(&SearchRequest::new("theme:Corrected"));
        assert_eq!(before.unwrap().total_documents, 1);
        for bad in BAD {
            put_bad(&fx.root().join(file), bad);
            let e = fx.build().unwrap_err();
            assert!(e.contains("is not a regular file"), "{file} {bad:?}: {e}");
            let after = fx.corpus().search(&SearchRequest::new("theme:Corrected"));
            assert_eq!(after.unwrap().total_documents, 1, "{file} {bad:?}");
        }
    }
}

/// The index is left as it was only by an ordinary build. A forced
/// rebuild deletes the old index before the layer files are looked at
/// (as does a build after a change of configuration), so a refusal then
/// leaves an index that holds no document, until a build succeeds. This
/// is what the code does today, not a rule: the spec's follow-up is to
/// check the layer files before the old index is deleted, and this test
/// changes with it.
#[test]
fn a_forced_build_over_a_refused_file_does_not_keep_the_old_index() {
    let fx = Fx::new();
    assert_eq!(fx.build(), Ok(1));
    let before = fx.corpus().search(&SearchRequest::new("proof"));
    assert_eq!(before.unwrap().total_documents, 1);
    put_bad(&fx.root().join(topics::CORRECTIONS_FILE), Bad::Directory);
    let corpus = fx.corpus();
    let e = within("build_index", move || {
        corpus
            .build_index(true)
            .map(|_| ())
            .map_err(|e| format!("{e:#}"))
    })
    .unwrap_err();
    assert!(e.contains("is not a regular file"), "{e}");
    let after = fx.corpus().search(&SearchRequest::new("proof"));
    assert_eq!(after.unwrap().total_documents, 0);
}

/// A link to a regular file is followed, for each file and for both: the
/// layer read and the build are what they are over the files themselves.
#[test]
fn a_link_to_a_regular_file_reads_as_the_file_itself() {
    let plain = Fx::new();
    let layer = plain.read().unwrap().expect("a layer");
    assert_eq!((layer.items.len(), layer.corrections), (2, 1));
    assert_eq!(plain.build(), Ok(1));

    for files in [
        &[topics::LAYER_FILE][..],
        &[topics::CORRECTIONS_FILE][..],
        &[topics::LAYER_FILE, topics::CORRECTIONS_FILE][..],
    ] {
        let fx = Fx::new();
        let elsewhere = tempfile::tempdir().unwrap();
        for (n, file) in files.iter().enumerate() {
            let at = fx.root().join(file);
            let target = elsewhere.path().join(format!("{n}.jsonl"));
            fs::rename(&at, &target).unwrap();
            std::os::unix::fs::symlink(&target, &at).unwrap();
        }
        let linked = fx.read().unwrap().expect("a layer");
        assert_eq!(linked.items, layer.items, "{files:?}");
        assert_eq!(
            (
                linked.bad_lines,
                linked.corrections,
                linked.corrections_unmatched
            ),
            (
                layer.bad_lines,
                layer.corrections,
                layer.corrections_unmatched
            ),
            "{files:?}"
        );
        assert_eq!(fx.build(), Ok(1), "{files:?}");
    }
}

/// A link to nothing is a missing file, as it was: no layer, and with a
/// layer no corrections.
#[test]
fn a_dangling_link_is_a_missing_file() {
    let fx = Fx::new();
    let at = fx.root().join(topics::CORRECTIONS_FILE);
    fs::remove_file(&at).unwrap();
    std::os::unix::fs::symlink(fx.root().join("nowhere"), &at).unwrap();
    assert_eq!(fx.read().unwrap().expect("a layer").corrections, 0);

    let at = fx.root().join(topics::LAYER_FILE);
    fs::remove_file(&at).unwrap();
    std::os::unix::fs::symlink(fx.root().join("nowhere"), &at).unwrap();
    assert_eq!(fx.read(), Ok(None));
}

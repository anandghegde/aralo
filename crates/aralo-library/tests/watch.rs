//! Watching a real folder. These tests drive the file system and wait for the
//! operating system to notice, so every wait is generous: a slow machine
//! should make them slow, not flaky.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::sleep;
use std::time::{Duration, Instant};

use aralo_library::{Changes, Library, OwnWrites, Watch};

/// Long enough for FSEvents or inotify to deliver on a loaded machine.
const PATIENCE: Duration = Duration::from_secs(10);

/// How long to wait before believing that nothing is coming. Shorter, because
/// every negative test pays it in full.
const SILENCE: Duration = Duration::from_secs(2);

const DEBOUNCE: Duration = Duration::from_millis(50);

fn write(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn snippet(label: &str, body: &str) -> String {
    format!("---\nlabel: {label}\nabbr: \";{label}\"\n---\n{body}\n")
}

fn start(root: &Path, writes: OwnWrites) -> (Watch, Receiver<Changes>) {
    let (sender, receiver) = mpsc::channel();
    let watch = Watch::with_debounce(root, writes, DEBOUNCE, move |changes| {
        let _ = sender.send(changes);
    })
    .unwrap();
    // Give the platform's watch a moment to actually be watching before the
    // test writes the thing it expects to hear about.
    sleep(Duration::from_millis(300));
    (watch, receiver)
}

/// Collects reports until `paths` have all been seen, or gives up. Batching is
/// the platform's business: one write can arrive as one report or as two.
fn until(receiver: &Receiver<Changes>, paths: &[&str]) -> Vec<PathBuf> {
    let wanted: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    let mut seen: Vec<PathBuf> = Vec::new();
    while !wanted.iter().all(|path| seen.contains(path)) {
        match receiver.recv_timeout(PATIENCE) {
            Ok(changes) => {
                for path in changes.paths {
                    if !seen.contains(&path) {
                        seen.push(path);
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => panic!("expected {paths:?}, saw {seen:?}"),
            Err(RecvTimeoutError::Disconnected) => panic!("the watch stopped"),
        }
    }
    seen
}

/// Waits out the silence, and fails on any report that names a path outside
/// `late`.
///
/// `late` is a real snippet the test wrote or changed that may still be
/// reported: FSEvents can deliver a write made just before the stream began
/// after it has begun, and can report one write twice. A report of a snippet
/// that did change is not what these tests are about; a report of anything
/// else is.
fn nothing_but(receiver: &Receiver<Changes>, late: &[&str]) {
    let deadline = Instant::now() + SILENCE;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return;
        }
        match receiver.recv_timeout(left) {
            Err(RecvTimeoutError::Timeout) => return,
            Ok(changes) => assert!(
                changes
                    .paths
                    .iter()
                    .all(|path| late.iter().any(|late| path == Path::new(late))),
                "expected silence, got {changes:?}"
            ),
            Err(RecvTimeoutError::Disconnected) => panic!("the watch stopped"),
        }
    }
}

#[test]
fn an_edit_from_outside_aralo_is_reported() {
    let folder = aralo_testkit::tempdir().unwrap();
    write(folder.path(), "Work/note.md", &snippet("note", "First"));
    let (_watch, changes) = start(folder.path(), OwnWrites::new());

    write(folder.path(), "Work/note.md", &snippet("note", "Second"));
    assert_eq!(
        until(&changes, &["Work/note.md"]),
        [PathBuf::from("Work/note.md")]
    );
}

#[test]
fn a_new_file_and_a_deleted_one_are_both_reported() {
    let folder = aralo_testkit::tempdir().unwrap();
    write(folder.path(), "old.md", &snippet("old", "Here"));
    let (_watch, changes) = start(folder.path(), OwnWrites::new());

    write(folder.path(), "new.md", &snippet("new", "Fresh"));
    fs::remove_file(folder.path().join("old.md")).unwrap();
    until(&changes, &["new.md", "old.md"]);
}

#[test]
fn a_group_file_says_so() {
    let folder = aralo_testkit::tempdir().unwrap();
    write(folder.path(), "Work/note.md", &snippet("note", "First"));
    let (_watch, changes) = start(folder.path(), OwnWrites::new());

    write(
        folder.path(),
        "Work/_group.yaml",
        "name: Work\nenabled: false\n",
    );
    loop {
        let report = changes.recv_timeout(PATIENCE).expect("no report");
        if report.paths.contains(&PathBuf::from("Work/_group.yaml")) {
            // Inherited settings reach snippets that did not change, so the
            // caller is told to reload the library rather than these paths.
            assert!(report.groups_changed);
            return;
        }
    }
}

#[test]
fn a_save_by_aralo_is_not_reported_but_an_edit_beside_it_is() {
    let folder = aralo_testkit::tempdir().unwrap();
    write(folder.path(), "Work/mine.md", &snippet("mine", "First"));
    write(folder.path(), "Work/theirs.md", &snippet("theirs", "First"));
    let library = Library::load(folder.path()).unwrap();
    let (_watch, changes) = start(folder.path(), library.writes().clone());

    // Aralo saves one file while a text editor writes the other.
    let mut file = library.snippets()[0].file.clone();
    file.body = "Saved by Aralo".to_owned();
    library
        .write_snippet(Path::new("Work/mine.md"), &file)
        .unwrap();
    write(
        folder.path(),
        "Work/theirs.md",
        &snippet("theirs", "Second"),
    );

    let seen = until(&changes, &["Work/theirs.md"]);
    assert_eq!(seen, [PathBuf::from("Work/theirs.md")]);
    assert_eq!(library.writes().outstanding(), 0, "the save was claimed");
    nothing_but(&changes, &["Work/theirs.md"]);
}

#[test]
fn an_edit_that_lands_on_top_of_a_save_is_still_reported() {
    let folder = aralo_testkit::tempdir().unwrap();
    write(folder.path(), "note.md", &snippet("note", "First"));
    let library = Library::load(folder.path()).unwrap();
    let (_watch, changes) = start(folder.path(), library.writes().clone());

    let mut file = library.snippets()[0].file.clone();
    file.body = "Saved by Aralo".to_owned();
    library.write_snippet(Path::new("note.md"), &file).unwrap();
    // Someone else got there before the event did. Suppressing by path alone
    // would lose this edit for good.
    write(folder.path(), "note.md", &snippet("note", "Theirs"));

    until(&changes, &["note.md"]);
}

#[test]
fn files_the_loader_ignores_are_not_reported() {
    let folder = aralo_testkit::tempdir().unwrap();
    write(folder.path(), "Work/note.md", &snippet("note", "First"));
    let (_watch, changes) = start(folder.path(), OwnWrites::new());

    write(
        folder.path(),
        "Work/.note.md.swp",
        "an editor's scratch file",
    );
    write(folder.path(), "_drafts/wip.md", &snippet("wip", "Later"));
    write(folder.path(), "assets/logo.md", "not a snippet");
    write(folder.path(), "README.txt", "about this folder");
    nothing_but(&changes, &["Work/note.md"]);

    // The watch is still live, and still hears about a real snippet.
    write(folder.path(), "Work/note.md", &snippet("note", "Second"));
    until(&changes, &["Work/note.md"]);
}

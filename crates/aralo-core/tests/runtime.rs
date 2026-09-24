//! The library running: a folder that changes underneath Aralo, an index that
//! keeps up, and a shell that is told about both.
//!
//! These tests touch real files and real threads, because that is what is
//! being tested. Each one waits for a condition rather than for a length of
//! time: a file system watcher is not punctual, and a test that sleeps for a
//! fixed period is a test that fails on a busy machine.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use aralo_core::{
    Core, Draft, LibraryChange, LibraryListener, Query, Runtime, RuntimeOptions, SetAside,
};

const DEBOUNCE: Duration = Duration::from_millis(30);
/// Long enough for a file system event to make its way through FSEvents on a
/// machine that is busy, short enough that a broken test is not a coffee break.
const PATIENCE: Duration = Duration::from_secs(10);

/// What a shell would do with the events: keep them.
#[derive(Debug, Default, Clone)]
struct Heard(Arc<Mutex<Vec<LibraryChange>>>);

impl Heard {
    fn changes(&self) -> Vec<LibraryChange> {
        self.0.lock().unwrap().clone()
    }

    fn outside_paths(&self) -> Vec<PathBuf> {
        self.changes()
            .into_iter()
            .filter_map(|change| match change {
                LibraryChange::Outside { paths } => Some(paths),
                _ => None,
            })
            .flatten()
            .collect()
    }

    fn count(&self, matches: impl Fn(&LibraryChange) -> bool) -> usize {
        self.changes()
            .iter()
            .filter(|change| matches(change))
            .count()
    }

    fn clear(&self) {
        self.0.lock().unwrap().clear();
    }
}

impl LibraryListener for Heard {
    fn changed(&self, change: LibraryChange, library: &Core) {
        // The library handed over is the one the change is about: a listener
        // that is told a file arrived can read it without asking again.
        if let LibraryChange::Outside { paths } = &change {
            for path in paths {
                assert!(
                    library.snippets().iter().any(|s| &s.path == path)
                        || !library.root().join(path).exists(),
                    "told about {path:?} before the library had read it"
                );
            }
        }
        self.0.lock().unwrap().push(change);
    }
}

/// Waits for `condition`, checking often. Panics with `what` if it never holds.
fn until(what: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline {
        if condition() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("waited {PATIENCE:?} for {what} and it never happened");
}

fn runtime(folder: &Path) -> (Runtime, Heard) {
    let heard = Heard::default();
    let core = Core::open_without_starter(&folder.join("Aralo")).unwrap();
    let options = RuntimeOptions {
        index: Some(folder.join("state/index.sqlite3")),
        watch: true,
        debounce: DEBOUNCE,
        discard: Some(Arc::new(SetAside::new(folder.join("state/merged")))),
        model: None,
    };
    let runtime = Runtime::with_core(
        core,
        options,
        Arc::new(heard.clone()) as Arc<dyn LibraryListener>,
    )
    .unwrap();
    (runtime, heard)
}

fn draft(label: &str, abbr: &str, body: &str) -> Draft {
    Draft {
        label: label.to_owned(),
        abbr: vec![abbr.to_owned()],
        body: body.to_owned(),
        ..Draft::default()
    }
}

#[test]
fn a_file_someone_else_writes_is_read_and_reported() {
    let folder = tempfile::tempdir().unwrap();
    let (runtime, heard) = runtime(folder.path());
    let root = runtime.root();
    assert_eq!(runtime.snapshot().len(), 0);

    fs::write(
        root.join("hand-written.md"),
        "---\nlabel: Best regards\nabbr: [br]\n---\nBest,\nAnand\n",
    )
    .unwrap();

    until("the outside change to be reported", || {
        !heard.outside_paths().is_empty()
    });
    assert_eq!(heard.outside_paths(), [PathBuf::from("hand-written.md")]);
    assert_eq!(runtime.snapshot().len(), 1, "the engine expands it at once");
    assert_eq!(runtime.read(|core| core.snippets().len()), 1);
}

#[test]
fn aralo_does_not_hear_its_own_writes() {
    let folder = tempfile::tempdir().unwrap();
    let (runtime, heard) = runtime(folder.path());

    let id = runtime
        .edit(|core| {
            core.create_snippet(&["Work".to_owned()], &draft("Best regards", "br", "Best"))
        })
        .unwrap();
    runtime
        .edit(|core| core.save_snippet(id, &draft("Best regards", "br", "Best wishes")))
        .unwrap();
    runtime.edit(|core| core.delete_snippet(id)).unwrap();

    // Two debounces: long enough that a watcher event would have arrived.
    std::thread::sleep(DEBOUNCE * 8);
    runtime.flush();

    assert_eq!(
        heard.count(|change| matches!(change, LibraryChange::Edited)),
        3
    );
    assert!(
        heard.outside_paths().is_empty(),
        "Aralo's own writes came back as someone else's: {:?}",
        heard.changes()
    );
    assert_eq!(runtime.snapshot().len(), 0);
}

#[test]
fn a_folder_that_someone_moves_away_leaves_the_library_that_loaded() {
    let folder = tempfile::tempdir().unwrap();
    let (runtime, heard) = runtime(folder.path());
    runtime
        .edit(|core| core.create_snippet(&[], &draft("Best regards", "br", "Best")))
        .unwrap();
    assert_eq!(runtime.snapshot().len(), 1);
    heard.clear();

    fs::rename(runtime.root(), folder.path().join("Moved")).unwrap();

    // Whatever the watcher makes of a root that is gone, the snippets that
    // loaded are still expandable: Aralo does not stop working because a sync
    // client is halfway through something.
    std::thread::sleep(DEBOUNCE * 8);
    assert_eq!(runtime.snapshot().len(), 1);
    assert!(runtime.read(|core| !core.snippets().is_empty()));
}

#[test]
fn the_index_follows_the_library_without_being_asked() {
    let folder = tempfile::tempdir().unwrap();
    let (runtime, heard) = runtime(folder.path());
    runtime.flush();

    let id = runtime
        .edit(|core| {
            core.create_snippet(&["Work".to_owned()], &draft("Best regards", "br", "Best"))
        })
        .unwrap();
    runtime.flush();
    assert!(
        heard.count(|change| matches!(change, LibraryChange::Indexed { added: 1, .. })) == 1,
        "{:?}",
        heard.changes()
    );

    runtime.edit(|core| core.delete_snippet(id)).unwrap();
    runtime.flush();
    assert!(
        heard.count(|change| matches!(change, LibraryChange::Indexed { removed: 1, .. })) == 1,
        "{:?}",
        heard.changes()
    );
}

#[test]
fn an_expansion_is_counted_without_the_injector_waiting() {
    let folder = tempfile::tempdir().unwrap();
    let (runtime, _heard) = runtime(folder.path());
    let id = runtime
        .edit(|core| core.create_snippet(&[], &draft("Best regards", "br", "Best")))
        .unwrap();
    runtime.flush();

    assert_eq!(runtime.stats(id).unwrap().expansions, 0);
    runtime.record_expansion(id);
    runtime.record_expansion(id);
    runtime.flush();

    assert_eq!(runtime.stats(id).unwrap().expansions, 2);
    let recents = runtime.recents(10).unwrap();
    assert_eq!(recents.len(), 1);
    assert_eq!(recents[0].id, id);
    assert_eq!(recents[0].expansions, 2);
}

#[test]
fn what_was_counted_last_time_is_still_there_next_time() {
    let folder = tempfile::tempdir().unwrap();
    let id = {
        let (runtime, _heard) = runtime(folder.path());
        let id = runtime
            .edit(|core| core.create_snippet(&[], &draft("Best regards", "br", "Best")))
            .unwrap();
        runtime.record_expansion(id);
        runtime.flush();
        id
    };

    let (runtime, _heard) = runtime(folder.path());
    runtime.flush();
    assert_eq!(runtime.stats(id).unwrap().expansions, 1);
    assert_eq!(runtime.recents(10).unwrap()[0].id, id);
}

#[test]
fn reading_the_index_does_not_wait_for_the_indexer() {
    let folder = tempfile::tempdir().unwrap();
    let (runtime, _heard) = runtime(folder.path());
    for n in 0..200 {
        runtime
            .edit(|core| {
                core.create_snippet(
                    &[],
                    &draft(&format!("Snippet {n}"), &format!("s{n}"), "body"),
                )
            })
            .unwrap();
    }

    // Every one of those edits queued a sync; the reads below run while the
    // indexer is still working through them, and must not block on it.
    let started = Instant::now();
    for _ in 0..50 {
        runtime.recents(10).unwrap();
    }
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "reads waited {:?} for the indexer",
        started.elapsed()
    );

    runtime.flush();
    assert_eq!(runtime.read(|core| core.snippets().len()), 200);
}

#[test]
fn a_library_whose_index_will_not_open_still_watches_and_expands() {
    let folder = tempfile::tempdir().unwrap();
    let heard = Heard::default();
    let core = Core::open_without_starter(&folder.path().join("Aralo")).unwrap();
    let options = RuntimeOptions {
        // An index whose folder is a file: the cache cannot be opened, the way
        // a read-only disk or a missing home folder would stop it.
        index: Some(folder.path().join("Aralo/aralo.yaml/index.sqlite3")),
        watch: true,
        debounce: DEBOUNCE,
        discard: Some(Arc::new(SetAside::new(folder.path().join("merged")))),
        model: None,
    };
    let runtime = Runtime::with_core(
        core,
        options,
        Arc::new(heard.clone()) as Arc<dyn LibraryListener>,
    )
    .expect("the library loaded, so the runtime opens");

    assert!(!runtime.has_index());
    assert!(runtime.index_error().is_some());

    fs::write(
        runtime.root().join("hand-written.md"),
        "---\nlabel: Best regards\nabbr: [br]\n---\nBest\n",
    )
    .unwrap();
    until("the library to be read again", || {
        runtime.snapshot().len() == 1
    });

    let id = runtime
        .edit(|core| core.create_snippet(&[], &draft("On my way", "omw", "On my way")))
        .unwrap();
    assert_eq!(runtime.snapshot().len(), 2, "editing works without a cache");

    assert_eq!(
        heard.count(|change| matches!(change, LibraryChange::Edited)),
        1,
        "the shell is still told about edits: {:?}",
        heard.changes()
    );
    runtime.reload().unwrap();
    assert_eq!(
        heard.count(|change| matches!(change, LibraryChange::Reloaded)),
        1
    );

    // Recents and counts are what is lost, and they are lost quietly.
    runtime.record_expansion(id);
    runtime.flush();
    assert!(runtime.recents(10).unwrap().is_empty());
    assert_eq!(runtime.stats(id).unwrap().expansions, 0);
}

#[test]
fn search_and_snapshot_see_an_edit_the_moment_it_returns() {
    let folder = tempfile::tempdir().unwrap();
    let (runtime, _heard) = runtime(folder.path());
    runtime
        .edit(|core| {
            core.create_snippet(&["Work".to_owned()], &draft("Best regards", "br", "Best"))
        })
        .unwrap();

    let hits = runtime.search(&Query::new("regards"));
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].name, "Best regards");
    assert_eq!(hits[0].group, ["Work"]);
    assert_eq!(runtime.snapshot().len(), 1);
}

#[test]
fn dropping_the_runtime_stops_both_threads() {
    let folder = tempfile::tempdir().unwrap();
    let (runtime, heard) = runtime(folder.path());
    runtime
        .edit(|core| core.create_snippet(&[], &draft("Best regards", "br", "Best")))
        .unwrap();
    let root = runtime.root();
    drop(runtime);
    heard.clear();

    // Nothing is listening any more, so this must reach no one.
    fs::write(
        root.join("after.md"),
        "---\nlabel: Later\nabbr: [l]\n---\nlate\n",
    )
    .unwrap();
    std::thread::sleep(DEBOUNCE * 8);
    assert!(heard.changes().is_empty(), "{:?}", heard.changes());
}

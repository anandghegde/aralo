//! A sync client's conflict copy, folded back into the snippet it copies: by
//! the core when asked, and by the runtime whenever it reads the folder.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use aralo_core::snippet::SnippetFile;
use aralo_core::{
    Core, CoreError, Discard, Issue, LibraryChange, LibraryListener, MergeReport, Resolution,
    Runtime, RuntimeOptions, SetAside,
};

const ID: &str = "01J8ZK3V5Q8W6T9X2N4R7M0ABC";
const PATIENCE: Duration = Duration::from_secs(10);

fn write(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn snippet(label: &str, body: &str) -> String {
    format!("---\nid: {ID}\nlabel: {label}\nabbr: [;sig]\n---\n{body}\n")
}

const BASE_BODY: &str = "Best,\nSam\n\nSent from Aralo";

fn parse(text: &str) -> SnippetFile {
    SnippetFile::parse(text).unwrap()
}

fn files_in(folder: &Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(folder)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

#[test]
fn a_clean_merge_is_written_over_the_original_and_the_copy_set_aside() {
    let library = aralo_testkit::tempdir().unwrap();
    let aside = aralo_testkit::tempdir().unwrap();
    let root = library.path();
    write(root, "Work/sig.md", &snippet("Email signature", BASE_BODY));
    write(
        root,
        "Work/sig (conflicted copy 2026-09-23).md",
        &snippet("Signature", "Best,\nSam\n\nSent from my Mac"),
    );
    let mut core = Core::open_read_only(root).unwrap();
    assert_eq!(core.conflicts().len(), 1);

    let base = parse(&snippet("Signature", BASE_BODY));
    let report = core
        .merge_conflicts(|_| Some(base.clone()), &SetAside::new(aside.path()))
        .unwrap();

    assert_eq!(
        report,
        MergeReport {
            merged: vec![PathBuf::from("Work/sig (conflicted copy 2026-09-23).md")],
            unresolved: vec![],
        }
    );
    assert!(core.conflicts().is_empty());
    assert_eq!(files_in(&root.join("Work")), ["sig.md"]);
    let merged = parse(&fs::read_to_string(root.join("Work/sig.md")).unwrap());
    assert_eq!(merged.front.label, "Email signature");
    assert_eq!(merged.body, "Best,\nSam\n\nSent from my Mac");
    let aside = files_in(aside.path());
    assert_eq!(aside.len(), 1);
    assert!(aside[0].ends_with(" sig (conflicted copy 2026-09-23).md"));
}

#[test]
fn a_clash_leaves_both_files_and_says_so() {
    let library = aralo_testkit::tempdir().unwrap();
    let aside = aralo_testkit::tempdir().unwrap();
    let root = library.path();
    write(root, "sig.md", &snippet("Mine", BASE_BODY));
    write(root, "sig 2.md", &snippet("Theirs", BASE_BODY));
    let mut core = Core::open_read_only(root).unwrap();

    let base = parse(&snippet("Signature", BASE_BODY));
    let report = core
        .merge_conflicts(|_| Some(base.clone()), &SetAside::new(aside.path()))
        .unwrap();

    assert_eq!(report.merged, Vec::<PathBuf>::new());
    assert_eq!(report.unresolved, [PathBuf::from("sig 2.md")]);
    assert_eq!(files_in(root), ["sig 2.md", "sig.md"]);
    assert!(core.diagnostics().any(|diagnostic| diagnostic.path == Path::new("sig 2.md")
        && matches!(&diagnostic.issue, Issue::ConflictCopy { original } if original == Path::new("sig.md"))));
    assert!(files_in(aside.path()).is_empty());
}

#[test]
fn without_a_base_a_copy_that_only_repeats_the_original_still_goes() {
    let library = aralo_testkit::tempdir().unwrap();
    let aside = aralo_testkit::tempdir().unwrap();
    let root = library.path();
    write(root, "sig.md", &snippet("Signature", BASE_BODY));
    write(root, "sig (1).md", &snippet("Signature", BASE_BODY));
    let mut core = Core::open_read_only(root).unwrap();

    let report = core
        .merge_conflicts(|_| None, &SetAside::new(aside.path()))
        .unwrap();
    assert_eq!(report.merged, [PathBuf::from("sig (1).md")]);
    assert_eq!(files_in(root), ["sig.md"]);
}

/// A library with one clash in it: both machines renamed the signature.
fn clashing(root: &Path) -> Core {
    write(root, "sig.md", &snippet("Mine", BASE_BODY));
    write(
        root,
        "sig 2.md",
        &snippet("Theirs", "Best,\nSam\n\nSent from my Mac"),
    );
    Core::open_read_only(root).unwrap()
}

#[test]
fn a_clash_shows_both_sides_the_base_and_what_they_changed_differently() {
    let library = aralo_testkit::tempdir().unwrap();
    let core = clashing(library.path());
    let base = parse(&snippet("Signature", BASE_BODY));

    let sides = core
        .conflict(Path::new("sig 2.md"), |_| Some(base.clone()))
        .unwrap();

    assert_eq!(sides.conflict.original, Path::new("sig.md"));
    assert_eq!(sides.original, snippet("Mine", BASE_BODY));
    assert_eq!(
        sides.copy,
        snippet("Theirs", "Best,\nSam\n\nSent from my Mac")
    );
    assert!(sides.base.unwrap().contains("label: Signature"));
    // Only the label clashes: the body changed on one side only.
    assert_eq!(sides.clashes.keys, ["label"]);
    assert!(!sides.clashes.body);
    assert_eq!(sides.copy_problem, None);
}

#[test]
fn a_file_that_is_not_a_waiting_copy_is_not_a_conflict() {
    let library = aralo_testkit::tempdir().unwrap();
    let mut core = clashing(library.path());
    assert!(matches!(
        core.conflict(Path::new("sig.md"), |_| None),
        Err(CoreError::NoSuchConflict(_))
    ));
    let aside = aralo_testkit::tempdir().unwrap();
    assert!(matches!(
        core.resolve_conflict(
            Path::new("elsewhere.md"),
            Resolution::KeepOriginal,
            &SetAside::new(aside.path())
        ),
        Err(CoreError::NoSuchConflict(_))
    ));
}

#[test]
fn keeping_the_original_discards_the_copy_and_leaves_the_original_alone() {
    let library = aralo_testkit::tempdir().unwrap();
    let aside = aralo_testkit::tempdir().unwrap();
    let root = library.path();
    let mut core = clashing(root);

    core.resolve_conflict(
        Path::new("sig 2.md"),
        Resolution::KeepOriginal,
        &SetAside::new(aside.path()),
    )
    .unwrap();

    assert_eq!(files_in(root), ["sig.md"]);
    assert_eq!(
        fs::read_to_string(root.join("sig.md")).unwrap(),
        snippet("Mine", BASE_BODY)
    );
    assert!(core.conflicts().is_empty());
    assert_eq!(files_in(aside.path()).len(), 1);
}

#[test]
fn keeping_the_copy_writes_it_into_the_original_file() {
    let library = aralo_testkit::tempdir().unwrap();
    let aside = aralo_testkit::tempdir().unwrap();
    let root = library.path();
    let mut core = clashing(root);

    core.resolve_conflict(
        Path::new("sig 2.md"),
        Resolution::KeepCopy,
        &SetAside::new(aside.path()),
    )
    .unwrap();

    assert_eq!(files_in(root), ["sig.md"]);
    let kept = parse(&fs::read_to_string(root.join("sig.md")).unwrap());
    assert_eq!(kept.front.label, "Theirs");
    assert_eq!(kept.body, "Best,\nSam\n\nSent from my Mac");
    assert!(core.conflicts().is_empty());
    assert_eq!(core.snippets().len(), 1);
}

#[test]
fn a_version_the_user_wrote_is_kept_under_the_snippets_own_id() {
    let library = aralo_testkit::tempdir().unwrap();
    let aside = aralo_testkit::tempdir().unwrap();
    let root = library.path();
    let mut core = clashing(root);

    // Without an id, as a person writing it by hand might leave it.
    let written =
        "---\nlabel: Ours and theirs\nabbr: [;sig]\n---\nBest,\nSam\n\nSent from my Mac\n";
    core.resolve_conflict(
        Path::new("sig 2.md"),
        Resolution::Write(written.to_owned()),
        &SetAside::new(aside.path()),
    )
    .unwrap();

    assert_eq!(files_in(root), ["sig.md"]);
    let kept = parse(&fs::read_to_string(root.join("sig.md")).unwrap());
    assert_eq!(kept.front.label, "Ours and theirs");
    assert_eq!(kept.front.id.unwrap().to_string(), ID);
}

#[test]
fn a_written_version_that_is_not_a_snippet_changes_nothing() {
    let library = aralo_testkit::tempdir().unwrap();
    let aside = aralo_testkit::tempdir().unwrap();
    let root = library.path();
    let mut core = clashing(root);

    let outcome = core.resolve_conflict(
        Path::new("sig 2.md"),
        Resolution::Write("---\nabbr: [unclosed\n---\nbody\n".to_owned()),
        &SetAside::new(aside.path()),
    );

    assert!(matches!(outcome, Err(CoreError::NotASnippet(_))));
    assert_eq!(files_in(root), ["sig 2.md", "sig.md"]);
    assert_eq!(core.conflicts().len(), 1);
    assert!(files_in(aside.path()).is_empty());
}

#[test]
fn the_runtime_resolves_a_clash_with_the_base_it_keeps() {
    let library = aralo_testkit::tempdir().unwrap();
    let state = aralo_testkit::tempdir().unwrap();
    let root = library.path();
    write(root, "sig.md", &snippet("Signature", BASE_BODY));
    let bin = Arc::new(Bin(
        SetAside::new(state.path().join("aside")),
        Mutex::default(),
    ));
    let runtime = Runtime::with_core(
        Core::open_read_only(root).unwrap(),
        RuntimeOptions {
            index: Some(state.path().join("index.sqlite3")),
            watch: false,
            discard: Some(bin.clone()),
            ..RuntimeOptions::default()
        },
        Arc::new(Merges::default()),
    )
    .unwrap();
    runtime.flush();

    write(root, "sig 2.md", &snippet("Theirs", BASE_BODY));
    write(root, "sig.md", &snippet("Mine", BASE_BODY));
    runtime.reload().unwrap();

    let sides = runtime.conflict(Path::new("sig 2.md")).unwrap();
    assert!(sides.base.unwrap().contains("label: Signature"));
    assert_eq!(sides.clashes.keys, ["label"]);

    runtime
        .resolve_conflict(Path::new("sig 2.md"), Resolution::KeepCopy)
        .unwrap();
    assert_eq!(bin.1.lock().unwrap().len(), 1);
    assert_eq!(files_in(root), ["sig.md"]);
    runtime.read(|core| assert!(core.conflicts().is_empty()));
}

/// Keeps the copies a runtime merged, as a shell would.
#[derive(Debug, Default, Clone)]
struct Merges(Arc<Mutex<Vec<PathBuf>>>);

impl LibraryListener for Merges {
    fn changed(&self, change: LibraryChange, _library: &Core) {
        if let LibraryChange::Merged { copies } = change {
            self.0.lock().unwrap().extend(copies);
        }
    }
}

/// Records what it was asked to discard, and discards it.
#[derive(Debug)]
struct Bin(SetAside, Mutex<Vec<PathBuf>>);

impl Discard for Bin {
    fn discard(&self, path: &Path) -> std::io::Result<()> {
        self.1.lock().unwrap().push(path.to_owned());
        self.0.discard(path)
    }
}

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

#[test]
fn the_runtime_merges_a_copy_that_arrives_against_the_version_it_last_saw() {
    let library = aralo_testkit::tempdir().unwrap();
    let state = aralo_testkit::tempdir().unwrap();
    let root = library.path();
    write(root, "Work/sig.md", &snippet("Signature", BASE_BODY));

    let bin = Arc::new(Bin(
        SetAside::new(state.path().join("aside")),
        Mutex::default(),
    ));
    let merges = Merges::default();
    let runtime = Runtime::with_core(
        Core::open_read_only(root).unwrap(),
        RuntimeOptions {
            index: Some(state.path().join("index.sqlite3")),
            // The two files of a conflict land a moment apart, and a watcher
            // could read the folder between them. Asking for the read is what
            // makes this test the same every time; the watcher reads through
            // the same path.
            watch: false,
            discard: Some(bin.clone()),
            ..RuntimeOptions::default()
        },
        Arc::new(merges.clone()),
    )
    .unwrap();
    // The first sync is what records the base.
    runtime.flush();

    // Another machine changed the last line while this one changed the label,
    // and the sync client kept both.
    write(
        root,
        "Work/sig (conflicted copy 2026-09-23).md",
        &snippet("Signature", "Best,\nSam\n\nSent from my Mac"),
    );
    write(root, "Work/sig.md", &snippet("Email signature", BASE_BODY));
    runtime.reload().unwrap();

    assert_eq!(
        *merges.0.lock().unwrap(),
        [PathBuf::from("Work/sig (conflicted copy 2026-09-23).md")]
    );
    assert_eq!(bin.1.lock().unwrap().len(), 1);
    let merged = parse(&fs::read_to_string(root.join("Work/sig.md")).unwrap());
    assert_eq!(merged.front.label, "Email signature");
    assert_eq!(merged.body, "Best,\nSam\n\nSent from my Mac");
    runtime.read(|core| {
        assert!(core.conflicts().is_empty());
        assert_eq!(core.snippets().len(), 1);
    });

    // The merged file is the new base: the next conflict is merged against it.
    runtime.flush();
    write(
        root,
        "Work/sig 2.md",
        &snippet("Email signature", "Best wishes,\nSam\n\nSent from my Mac"),
    );
    runtime.reload().unwrap();
    assert_eq!(merges.0.lock().unwrap().len(), 2);
    let merged = parse(&fs::read_to_string(root.join("Work/sig.md")).unwrap());
    assert_eq!(merged.body, "Best wishes,\nSam\n\nSent from my Mac");
}

#[test]
fn copies_waiting_when_the_runtime_opens_are_merged_before_it_returns() {
    let library = aralo_testkit::tempdir().unwrap();
    let state = aralo_testkit::tempdir().unwrap();
    let root = library.path();
    write(root, "sig.md", &snippet("Signature", BASE_BODY));
    write(
        root,
        "sig.sync-conflict-20260923-101500-ABCDEFG.md",
        &snippet("Signature", BASE_BODY),
    );

    let runtime = Runtime::with_core(
        Core::open_read_only(root).unwrap(),
        RuntimeOptions {
            index: Some(state.path().join("index.sqlite3")),
            watch: false,
            discard: Some(Arc::new(SetAside::new(state.path().join("aside")))),
            ..RuntimeOptions::default()
        },
        Arc::new(Merges::default()),
    )
    .unwrap();
    runtime.read(|core| assert!(core.conflicts().is_empty()));
    assert_eq!(files_in(root), ["sig.md"]);
}

#[test]
fn the_watcher_merges_a_copy_it_sees_arrive() {
    let library = aralo_testkit::tempdir().unwrap();
    let state = aralo_testkit::tempdir().unwrap();
    let root = library.path();
    write(root, "sig.md", &snippet("Signature", BASE_BODY));

    let merges = Merges::default();
    let _runtime = Runtime::with_core(
        Core::open_read_only(root).unwrap(),
        RuntimeOptions {
            index: Some(state.path().join("index.sqlite3")),
            debounce: Duration::from_millis(30),
            discard: Some(Arc::new(SetAside::new(state.path().join("aside")))),
            ..RuntimeOptions::default()
        },
        Arc::new(merges.clone()),
    )
    .unwrap();

    // Only the copy changes, so however the watcher batches it the merge is
    // the copy's change.
    write(
        root,
        "sig (conflicted copy 2026-09-23).md",
        &snippet("Signature", "Best,\nSam\n\nSent from my Mac"),
    );
    until("the copy to be merged", || {
        !merges.0.lock().unwrap().is_empty()
    });
    assert_eq!(files_in(root), ["sig.md"]);
    let merged = parse(&fs::read_to_string(root.join("sig.md")).unwrap());
    assert_eq!(merged.body, "Best,\nSam\n\nSent from my Mac");
}

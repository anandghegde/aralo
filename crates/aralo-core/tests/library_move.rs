//! Moving the library, and running it from a folder iCloud Drive or Dropbox
//! syncs (plan 5.2).
//!
//! No account is needed: each provider is played by hand in a temporary
//! folder laid out the way the provider lays out its own — iCloud Drive's
//! `Library/Mobile Documents/com~apple~CloudDocs`, Dropbox's
//! `Library/CloudStorage/Dropbox` and the older `~/Dropbox` with its
//! `.dropbox` marker — and doing what the provider does to files: evicting
//! them to `.name.icloud` placeholders and bringing them back, delivering a
//! file through a hidden temporary one, and leaving a conflict copy under its
//! own name. What only a real account on two real Macs can show, how long the
//! provider takes, is in docs/progress.md.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use aralo_core::snippet::SnippetId;
use aralo_core::state::index_in;
use aralo_core::{
    move_library, relocate, Contents, Core, Issue, LibraryChange, LibraryListener, MoveError,
    Provider, Runtime, RuntimeOptions, SetAside,
};

const SIGNATURE: &str = "---\nid: 01J8ZK3V5Q8W6T9X2N4R7M0ABC\nlabel: Signature\n\
                         abbr: [;sig]\n---\nBest,\nSam\n\nSent from Aralo\n";
const ADDRESS: &str = "---\nid: 01J8ZK3V5Q8W6T9X2N4R7M0ABD\nlabel: Address\n\
                       abbr: [;addr]\n---\n1 Infinite Loop\n";
const PATIENCE: Duration = Duration::from_secs(10);

struct Quiet;

impl LibraryListener for Quiet {
    fn changed(&self, _change: LibraryChange, _library: &Core) {}
}

/// A home folder of its own, with a library in `~/Aralo` and a cache.
struct Home {
    folder: aralo_testkit::TempDir,
}

impl Home {
    fn new() -> Self {
        let home = Self {
            folder: aralo_testkit::tempdir().unwrap(),
        };
        let library = home.library();
        fs::create_dir_all(library.join("Work")).unwrap();
        fs::create_dir_all(library.join("assets")).unwrap();
        fs::write(library.join("aralo.yaml"), "format: 0\n").unwrap();
        fs::write(library.join("Work/sig.md"), SIGNATURE).unwrap();
        fs::write(library.join("Work/_group.yaml"), "name: Work\n").unwrap();
        fs::write(library.join("address.md"), ADDRESS).unwrap();
        fs::write(library.join("README.txt"), "notes").unwrap();
        fs::write(library.join("assets/logo.png"), [0u8, 1, 2, 3, 255]).unwrap();
        fs::write(library.join(".DS_Store"), b"finder").unwrap();
        home
    }

    fn path(&self) -> &Path {
        self.folder.path()
    }

    fn library(&self) -> PathBuf {
        self.path().join("Aralo")
    }

    fn cache(&self) -> PathBuf {
        self.path().join("state")
    }

    fn icloud(&self) -> PathBuf {
        self.path()
            .join("Library/Mobile Documents/com~apple~CloudDocs")
    }

    fn dropbox(&self) -> PathBuf {
        self.path().join("Library/CloudStorage/Dropbox")
    }

    /// The runtime the app would run on `root`, with its index where the app
    /// keeps it.
    fn runtime(&self, root: &Path, watch: bool) -> Runtime {
        Runtime::with_core(
            Core::open_without_starter(root).unwrap(),
            RuntimeOptions {
                index: Some(index_in(&self.cache(), root)),
                watch,
                debounce: Duration::from_millis(30),
                discard: Some(Arc::new(SetAside::new(self.cache().join("aside")))),
            },
            Arc::new(Quiet),
        )
        .unwrap()
    }
}

fn signature_id() -> SnippetId {
    "01J8ZK3V5Q8W6T9X2N4R7M0ABC".parse().unwrap()
}

/// Every file under `root` but Finder's, relative to it, with its bytes.
fn files(root: &Path) -> Vec<(String, Vec<u8>)> {
    fn walk(root: &Path, at: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        for entry in fs::read_dir(at).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else if path.file_name().unwrap() != ".DS_Store" {
                let name = path.strip_prefix(root).unwrap().to_string_lossy().into();
                out.push((name, fs::read(&path).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, root, &mut out);
    out.sort();
    out
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

/// Nothing is left beside the destination: no staging folder, no half copy.
fn nothing_beside(folder: &Path) -> Vec<String> {
    match fs::read_dir(folder) {
        Ok(entries) => entries
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect(),
        Err(_) => Vec::new(),
    }
}

#[test]
fn a_move_to_icloud_drive_copies_every_file_and_brings_the_counts() {
    let home = Home::new();
    let from = home.library();
    let before = files(&from);
    {
        let runtime = home.runtime(&from, false);
        runtime.record_expansion(signature_id());
        runtime.record_expansion(signature_id());
        runtime.flush();
    }

    let to = home.icloud().join("Aralo");
    assert_eq!(relocate::inspect(&to).contents, Contents::Empty);
    assert_eq!(relocate::inspect(&to).provider, Some(Provider::ICloudDrive));
    let moved = move_library(&from, &to, Some(&home.cache())).unwrap();

    assert_eq!(moved.to, to);
    assert_eq!(moved.snippets, 2);
    assert_eq!(moved.files, before.len());
    assert_eq!(moved.index_problem, None);
    assert_eq!(files(&to), before, "every file, byte for byte");
    assert_eq!(files(&from), before, "the old folder is left as it was");
    assert!(
        !to.join(".DS_Store").exists(),
        "Finder's settings stay behind"
    );
    assert_eq!(nothing_beside(&home.icloud()), ["Aralo"]);
    assert_eq!(moved.changed_since(), Vec::<PathBuf>::new());
    assert_eq!(
        relocate::inspect(&to).contents,
        Contents::Library { snippets: 2 }
    );

    // The moved library opens with the counts this Mac kept, which live in
    // an index keyed by the library's path.
    let runtime = home.runtime(&to, false);
    runtime.flush();
    assert_eq!(runtime.stats(signature_id()).unwrap().expansions, 2);
    assert_eq!(runtime.recents(5).unwrap()[0].id, signature_id());
}

#[test]
fn what_reaches_the_old_folder_after_the_copy_is_named() {
    let home = Home::new();
    let from = home.library();
    let moved = move_library(&from, &home.path().join("Moved"), None).unwrap();
    assert!(moved.changed_since().is_empty());

    fs::write(from.join("Work/sig.md"), SIGNATURE.replace("Sam", "Samira")).unwrap();
    fs::write(from.join("late.md"), "---\nabbr: [;late]\n---\nlate\n").unwrap();
    fs::remove_file(from.join("README.txt")).unwrap();
    assert_eq!(
        moved.changed_since(),
        [
            PathBuf::from("README.txt"),
            PathBuf::from("Work/sig.md"),
            PathBuf::from("late.md")
        ]
    );
}

#[test]
fn a_destination_that_is_not_empty_or_overlaps_is_refused_and_nothing_is_written() {
    let home = Home::new();
    let from = home.library();
    let before = files(&from);

    assert!(matches!(
        move_library(&from, &from, None),
        Err(MoveError::SameFolder(_))
    ));
    assert!(matches!(
        move_library(&from, &from.join("Work/Inner"), None),
        Err(MoveError::InsideLibrary { .. })
    ));
    assert!(matches!(
        move_library(&from, home.path(), None),
        Err(MoveError::ContainsLibrary { .. })
    ));
    let occupied = home.path().join("Documents");
    fs::create_dir_all(&occupied).unwrap();
    fs::write(occupied.join("taxes.pdf"), b"%PDF").unwrap();
    assert!(matches!(
        move_library(&from, &occupied, None),
        Err(MoveError::NotEmpty(_))
    ));
    assert_eq!(relocate::inspect(&occupied).contents, Contents::Occupied);

    assert_eq!(files(&from), before);
    assert!(!from.join("Work/Inner").exists());
    assert_eq!(nothing_beside(&occupied), ["taxes.pdf"]);

    // An empty folder is fine, with or without Finder's file in it.
    let empty = home.path().join("Empty");
    fs::create_dir_all(&empty).unwrap();
    fs::write(empty.join(".DS_Store"), b"finder").unwrap();
    move_library(&from, &empty, None).unwrap();
    assert_eq!(files(&empty), before);
}

#[test]
fn files_icloud_drive_keeps_only_in_the_cloud_stop_the_move() {
    let home = Home::new();
    let from = home.icloud().join("Aralo");
    fs::create_dir_all(from.parent().unwrap()).unwrap();
    fs::rename(home.library(), &from).unwrap();
    // Optimise Mac Storage moved the signature to the cloud.
    fs::remove_file(from.join("Work/sig.md")).unwrap();
    fs::write(from.join("Work/.sig.md.icloud"), b"bplist00").unwrap();

    // The library still opens, and says which snippet is missing and why
    // rather than dropping it without a word.
    let core = Core::open_without_starter(&from).unwrap();
    assert!(core
        .diagnostics()
        .any(|diagnostic| diagnostic.path == Path::new("Work/sig.md")
            && diagnostic.issue == Issue::NotDownloaded));

    let to = home.path().join("Aralo");
    match move_library(&from, &to, None) {
        Err(MoveError::NotDownloaded { paths }) => {
            assert_eq!(paths, [PathBuf::from("Work/sig.md")]);
        }
        other => panic!("expected the move to be refused, got {other:?}"),
    }
    assert!(!to.exists());
    assert_eq!(nothing_beside(home.path()), ["Library"]);
}

#[test]
fn an_evicted_snippet_leaves_and_comes_back_while_the_app_runs() {
    let home = Home::new();
    let to = home.icloud().join("Aralo");
    move_library(&home.library(), &to, Some(&home.cache())).unwrap();
    let runtime = home.runtime(&to, true);
    let expands = |abbr: &str| {
        runtime.read(|core| {
            core.snippets()
                .iter()
                .any(|snippet| snippet.file.front.abbr.iter().any(|a| a == abbr))
        })
    };
    assert!(expands(";sig"));

    // iCloud evicts: the file goes and a placeholder takes its place.
    fs::write(to.join("Work/.sig.md.icloud"), b"bplist00").unwrap();
    fs::remove_file(to.join("Work/sig.md")).unwrap();
    until("the evicted snippet to go", || !expands(";sig"));
    assert!(runtime.read(|core| core
        .diagnostics()
        .any(|diagnostic| diagnostic.issue == Issue::NotDownloaded)));

    // And downloads it again, through a hidden file renamed into place.
    fs::write(to.join("Work/.sig.md.sb-download"), SIGNATURE).unwrap();
    fs::rename(to.join("Work/.sig.md.sb-download"), to.join("Work/sig.md")).unwrap();
    fs::remove_file(to.join("Work/.sig.md.icloud")).unwrap();
    until("the snippet to come back", || expands(";sig"));
}

#[test]
fn a_conflict_copy_dropbox_leaves_after_the_move_merges_against_the_carried_base() {
    let home = Home::new();
    let from = home.library();
    home.runtime(&from, false).flush();

    // The older Dropbox client: `~/Dropbox`, with its marker at the top.
    let dropbox = home.path().join("Dropbox");
    fs::create_dir_all(&dropbox).unwrap();
    fs::write(dropbox.join(".dropbox"), b"{}").unwrap();
    let to = dropbox.join("Aralo");
    assert_eq!(relocate::inspect(&to).provider, Some(Provider::Dropbox));
    move_library(&from, &to, Some(&home.cache())).unwrap();

    // The other Mac changed the last line; Dropbox kept its version as a
    // conflicted copy beside this Mac's, which it had not seen change.
    let theirs = SIGNATURE.replace("Sent from Aralo", "Sent from my other Mac");
    fs::write(
        to.join("Work/sig (Sam's conflicted copy 2026-09-26).md"),
        &theirs,
    )
    .unwrap();

    // The base the index kept at the old path is the one the merge uses at
    // the new path: this is a clean merge, not a clash for the user.
    let runtime = home.runtime(&to, false);
    runtime.flush();
    let merged = fs::read_to_string(to.join("Work/sig.md")).unwrap();
    assert!(
        merged.ends_with("Best,\nSam\n\nSent from my other Mac\n"),
        "{merged}"
    );
    assert!(!to
        .join("Work/sig (Sam's conflicted copy 2026-09-26).md")
        .exists());
    assert!(runtime.read(|core| core.conflicts().is_empty()));
}

#[test]
fn a_library_in_the_new_dropbox_folder_is_one_another_mac_can_use_as_it_is() {
    let home = Home::new();
    let to = home.dropbox().join("Aralo");
    move_library(&home.library(), &to, None).unwrap();

    // The second Mac finds the library already there, synced.
    let second = relocate::inspect(&to);
    assert_eq!(second.provider, Some(Provider::Dropbox));
    assert_eq!(second.contents, Contents::Library { snippets: 2 });

    // Dropbox delivers a file from the first Mac through a temporary one.
    let runtime = home.runtime(&to, true);
    fs::write(
        to.join("Work/.~new.md"),
        "---\nabbr: [;new]\nlabel: New\n---\nfrom the first Mac\n",
    )
    .unwrap();
    fs::rename(to.join("Work/.~new.md"), to.join("Work/new.md")).unwrap();
    until("the delivered snippet to load", || {
        runtime.read(|core| core.snippets().len() == 3)
    });
}

#[cfg(unix)]
#[test]
fn a_file_that_cannot_be_read_stops_the_move_and_the_copy_is_removed() {
    use std::os::unix::fs::PermissionsExt;

    let home = Home::new();
    let from = home.library();
    let locked = from.join("Work/locked.md");
    fs::write(&locked, SIGNATURE.replace("ABC", "ABE")).unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    // Root reads anything, and then there is nothing to test.
    if fs::read(&locked).is_ok() {
        return;
    }

    let to = home.path().join("Moved");
    let error = move_library(&from, &to, None).unwrap_err();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(
        matches!(error, MoveError::Read { .. } | MoveError::Write { .. }),
        "{error:?}"
    );
    assert!(!to.exists());
    assert_eq!(nothing_beside(home.path()), ["Aralo"]);
}

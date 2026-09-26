//! Moving the library through the bridge, the way the settings window does
//! (plan 5.2): the app keeps running, and keeps expanding, from the new
//! folder.

use std::fs;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use aralo_ffi::{
    inspect_library_location, Core, CoreEvents, InsertMethod, KeyAction, KeyInput, LibraryEvent,
    LocationContents, SnippetDraft, SnippetType,
};

const PATIENCE: Duration = Duration::from_secs(10);

fn until(what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline {
        if condition() {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("waited {PATIENCE:?} for {what} and it never happened");
}

#[derive(Clone, Default)]
struct Shell(Arc<Mutex<Vec<LibraryEvent>>>);

impl CoreEvents for Shell {
    fn library_changed(&self, event: LibraryEvent) {
        self.0.lock().unwrap().push(event);
    }
}

fn type_str(core: &Core, text: &str) -> Vec<KeyAction> {
    let engine = core.engine();
    text.chars()
        .map(|c| engine.on_key(KeyInput::Char { scalar: c as u32 }))
        .filter(|action| *action != KeyAction::Pass)
        .collect()
}

fn draft(abbr: &str, body: &str) -> SnippetDraft {
    SnippetDraft {
        label: "Moved".into(),
        abbreviations: vec![abbr.into()],
        body: body.into(),
        tags: Vec::new(),
        kind: SnippetType::Text,
        trigger: None,
        case: None,
        whole_word: None,
        keep_delimiter: None,
        enabled: None,
    }
}

#[test]
fn the_library_moves_to_icloud_drive_and_keeps_expanding_from_there() {
    let folder = aralo_testkit::tempdir().unwrap();
    let from = folder.path().join("Aralo");
    let shell = Shell::default();
    let core = Core::open_library(
        from.to_string_lossy().into_owned(),
        Some(folder.path().join("cache").to_string_lossy().into_owned()),
        Some(Arc::new(shell.clone())),
        None,
    )
    .unwrap();
    core.engine().set_front_app("com.apple.TextEdit".into());
    let actions = type_str(&core, "ty ");
    let [KeyAction::Expand { snippet_id, .. }] = actions.as_slice() else {
        panic!("{actions:?}");
    };
    core.engine()
        .expansion_done(snippet_id.clone(), 10, InsertMethod::Typed);
    let counted = snippet_id.clone();

    let icloud = folder
        .path()
        .join("Library/Mobile Documents/com~apple~CloudDocs");
    let to = icloud.join("Aralo");
    let picked = inspect_library_location(to.to_string_lossy().into_owned());
    assert_eq!(picked.provider.as_deref(), Some("iCloud Drive"));
    assert_eq!(picked.contents, LocationContents::Empty);

    let moved = core
        .move_library(to.to_string_lossy().into_owned())
        .unwrap();
    assert_eq!(moved.to, to.to_string_lossy());
    assert!(moved.changed_after_copy.is_empty(), "{moved:?}");
    assert_eq!(moved.index_problem, None);
    assert_eq!(core.library_path(), to.to_string_lossy());
    assert!(shell.0.lock().unwrap().contains(&LibraryEvent::Reloaded));

    // It expands from the new folder, and remembers what it expanded.
    assert!(matches!(
        type_str(&core, "ty ").as_slice(),
        [KeyAction::Expand { .. }]
    ));
    assert_eq!(core.recents(5).first(), Some(&counted));

    // What is written goes to the new folder; the old one is not watched.
    let id = core
        .create_snippet(Vec::new(), draft(";mv", "moved"))
        .unwrap();
    let path = core
        .snippets()
        .into_iter()
        .find(|snippet| snippet.id == id)
        .unwrap()
        .path;
    assert!(to.join(&path).is_file());
    assert!(!from.join(&path).exists());
    fs::write(from.join("stale.md"), "---\nabbr: [;stale]\n---\nstale\n").unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert!(!core
        .snippets()
        .iter()
        .any(|snippet| snippet.abbreviations == [";stale"]));
    // The watch on the new folder is running.
    fs::write(to.join("fresh.md"), "---\nabbr: [;fresh]\n---\nfresh\n").unwrap();
    until("the new folder's watch to see a file", || {
        core.snippets()
            .iter()
            .any(|snippet| snippet.abbreviations == [";fresh"])
    });
}

#[test]
fn a_move_that_is_refused_leaves_the_library_running_where_it_was() {
    let folder = aralo_testkit::tempdir().unwrap();
    let from = folder.path().join("Aralo");
    let core = Core::open_library(from.to_string_lossy().into_owned(), None, None, None).unwrap();
    let occupied = folder.path().join("Documents");
    fs::create_dir_all(&occupied).unwrap();
    fs::write(occupied.join("taxes.pdf"), b"%PDF").unwrap();

    assert_eq!(
        inspect_library_location(occupied.to_string_lossy().into_owned()).contents,
        LocationContents::Occupied
    );
    assert!(core
        .move_library(occupied.to_string_lossy().into_owned())
        .is_err());
    assert!(core
        .switch_library(occupied.to_string_lossy().into_owned())
        .is_err());
    assert_eq!(core.library_path(), from.to_string_lossy());
}

#[test]
fn a_library_another_mac_synced_is_used_as_it_is() {
    let folder = aralo_testkit::tempdir().unwrap();
    let other = folder.path().join("Dropbox/Aralo");
    fs::create_dir_all(&other).unwrap();
    fs::write(other.join("aralo.yaml"), "format: 0\n").unwrap();
    fs::write(
        other.join("theirs.md"),
        "---\nabbr: [;theirs]\n---\nfrom the other Mac\n",
    )
    .unwrap();
    fs::write(folder.path().join("Dropbox/.dropbox"), b"{}").unwrap();

    let picked = inspect_library_location(other.to_string_lossy().into_owned());
    assert_eq!(picked.provider.as_deref(), Some("Dropbox"));
    assert_eq!(picked.contents, LocationContents::Library { snippets: 1 });

    let core = Core::open_library(
        folder.path().join("Aralo").to_string_lossy().into_owned(),
        None,
        None,
        None,
    )
    .unwrap();
    core.engine().set_front_app("com.apple.TextEdit".into());
    core.switch_library(other.to_string_lossy().into_owned())
        .unwrap();
    assert_eq!(core.snippets().len(), 1);
    assert!(matches!(
        type_str(&core, ";theirs ").as_slice(),
        [KeyAction::Expand { .. }]
    ));
    assert!(folder.path().join("Aralo").is_dir(), "left where it was");
}

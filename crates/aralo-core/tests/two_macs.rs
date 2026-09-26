//! Two Macs editing one snippet in a synced folder, scripted end to end (plan
//! 5.1): each Mac is a runtime on its own copy of the folder, each saves
//! through the editor's call, and the sync client is played by hand, naming
//! its conflict copy the way each provider does. Edits to different parts of
//! the snippet merge on both Macs to the same file; edits to the same part
//! leave both versions for the user.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use aralo_core::snippet::SnippetFile;
use aralo_core::{Core, Draft, LibraryChange, LibraryListener, Runtime, RuntimeOptions, SetAside};

const SIGNATURE: &str = "---\nid: 01J8ZK3V5Q8W6T9X2N4R7M0ABC\nlabel: Signature\n\
                         abbr: [;sig]\n---\nBest,\nSam\n\nSent from Aralo\n";

/// Each provider's name for a copy of `Work/sig.md`.
const PROVIDERS: [(&str, &str); 7] = [
    ("Dropbox", "Work/sig (Sam's conflicted copy 2026-09-26).md"),
    ("iCloud Drive", "Work/sig 2.md"),
    (
        "Syncthing",
        "Work/sig.sync-conflict-20260926-101500-ABCDEFG.md",
    ),
    ("OneDrive", "Work/sig-MacBook-Air.md"),
    ("Google Drive", "Work/sig (1).md"),
    (
        "Nextcloud",
        "Work/sig (conflicted copy 2026-09-26 101500).md",
    ),
    ("ownCloud", "Work/sig_conflict-20260926-101500.md"),
];

struct Quiet;

impl LibraryListener for Quiet {
    fn changed(&self, _change: LibraryChange, _library: &Core) {}
}

/// One Mac: its copy of the folder, and the runtime its app runs.
struct Mac {
    root: PathBuf,
    runtime: Runtime,
    _folders: tempfile::TempDir,
}

impl Mac {
    fn new() -> Self {
        let folders = tempfile::tempdir().unwrap();
        let root = folders.path().join("Aralo");
        fs::create_dir_all(root.join("Work")).unwrap();
        fs::write(root.join("Work/sig.md"), SIGNATURE).unwrap();
        let state = folders.path().join("state");
        let runtime = Runtime::with_core(
            Core::open_without_starter(&root).unwrap(),
            RuntimeOptions {
                index: Some(state.join("index.sqlite3")),
                watch: false,
                discard: Some(Arc::new(SetAside::new(state.join("aside")))),
                ..RuntimeOptions::default()
            },
            Arc::new(Quiet),
        )
        .unwrap();
        runtime.flush();
        Self {
            root,
            runtime,
            _folders: folders,
        }
    }

    /// Opens the snippet in the editor, changes it and saves.
    fn edits(&self, change: impl FnOnce(&mut Draft)) {
        let (id, opened) = self.runtime.read(|core| {
            let snippet = &core.snippets()[0];
            (snippet.id, Draft::of(snippet))
        });
        let mut draft = opened.clone();
        change(&mut draft);
        self.runtime
            .edit(|core| core.save_snippet_since(id, &opened, &draft))
            .unwrap();
        self.runtime.flush();
    }

    fn text(&self, relative: &str) -> String {
        fs::read_to_string(self.root.join(relative)).unwrap()
    }

    fn write(&self, relative: &str, text: &str) {
        fs::write(self.root.join(relative), text).unwrap();
    }

    /// What the sync client does on this Mac, then the folder read again.
    fn syncs(&self, arrive: impl FnOnce(&Self)) {
        arrive(self);
        self.runtime.reload().unwrap();
        self.runtime.flush();
    }

    fn files(&self) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(self.root.join("Work"))
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn conflicts(&self) -> Vec<PathBuf> {
        self.runtime.read(|core| {
            core.conflicts()
                .iter()
                .map(|conflict| conflict.copy.clone())
                .collect()
        })
    }
}

/// Both Macs change the signature while apart, then sync. The first to upload
/// keeps its file and is sent the other's as a copy; the second finds its own
/// file renamed to the copy and the first's in its place.
fn apart(copy: &str, a: impl FnOnce(&mut Draft), b: impl FnOnce(&mut Draft)) -> (Mac, Mac) {
    let (first, second) = (Mac::new(), Mac::new());
    first.edits(a);
    second.edits(b);
    let firsts = first.text("Work/sig.md");
    let seconds = second.text("Work/sig.md");
    first.syncs(|mac| mac.write(copy, &seconds));
    second.syncs(|mac| {
        mac.write(copy, &seconds);
        mac.write("Work/sig.md", &firsts);
    });
    (first, second)
}

#[test]
fn edits_to_different_parts_merge_the_same_on_both_macs_for_every_provider() {
    for (provider, copy) in PROVIDERS {
        let (first, second) = apart(
            copy,
            |draft| draft.label = "Email signature".to_owned(),
            |draft| draft.body = "Best,\nSam\n\nSent from my Mac".to_owned(),
        );
        for mac in [&first, &second] {
            assert_eq!(mac.files(), ["sig.md"], "{provider}");
            assert!(mac.conflicts().is_empty(), "{provider}");
            let merged = SnippetFile::parse(&mac.text("Work/sig.md")).unwrap();
            assert_eq!(merged.front.label, "Email signature", "{provider}");
            assert_eq!(merged.body, "Best,\nSam\n\nSent from my Mac", "{provider}");
        }
        assert_eq!(
            first.text("Work/sig.md"),
            second.text("Work/sig.md"),
            "{provider}"
        );
    }
}

#[test]
fn edits_to_the_same_part_keep_both_versions_for_every_provider() {
    for (provider, copy) in PROVIDERS {
        let (first, second) = apart(
            copy,
            |draft| draft.body = "Best,\nSam\n\nSent from my desk".to_owned(),
            |draft| draft.body = "Best,\nSam\n\nSent from my Mac".to_owned(),
        );
        for mac in [&first, &second] {
            assert_eq!(mac.conflicts(), [PathBuf::from(copy)], "{provider}");
            let sides = mac.runtime.conflict(Path::new(copy)).unwrap();
            let both = [sides.original.as_str(), sides.copy.as_str()];
            assert!(
                both.iter().any(|side| side.contains("my desk")),
                "{provider}"
            );
            assert!(
                both.iter().any(|side| side.contains("my Mac")),
                "{provider}"
            );
            assert!(sides.clashes.body, "{provider}");
            assert!(
                sides.base.unwrap().contains("Sent from Aralo"),
                "{provider}"
            );
        }
    }
}

//! Saving an editor's draft over a file that changed while it was open
//! (plan 5.1): another Mac's edit, brought down by a sync client, is merged in
//! rather than written over, and one that touched what the draft touched is
//! handed back to the user with nothing written.
//!
//! "Another Mac" is a second `Core` on the same folder, which is what a sync
//! client makes of two machines once it has caught up.

use std::fs;
use std::path::Path;

use aralo_core::{Core, Draft, Saved};

const SIGNATURE: &str = "---\nid: 01J0000000000000000000000A\nlabel: Signature\nabbr: [sig]\n\
                         future_key: keep me\n---\nBest,\nSam\n\nSent from Aralo\n";

fn library() -> (tempfile::TempDir, Core) {
    let folder = tempfile::tempdir().unwrap();
    let root = folder.path().join("Aralo");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("sig.md"), SIGNATURE).unwrap();
    let core = Core::open_without_starter(&root).unwrap();
    (folder, core)
}

fn read(root: &Path) -> String {
    fs::read_to_string(root.join("sig.md")).unwrap()
}

/// The other Mac: opens the folder, changes the snippet and saves, the way
/// its own editor would.
fn other_mac_saves(root: &Path, change: impl FnOnce(&mut Draft)) {
    let mut other = Core::open_without_starter(root).unwrap();
    let snippet = &other.snippets()[0];
    let id = snippet.id;
    let mut draft = Draft::of(snippet);
    change(&mut draft);
    other.save_snippet(id, &draft).unwrap();
}

#[test]
fn a_change_from_another_mac_is_kept_beside_the_one_on_screen() {
    let (_folder, mut core) = library();
    let root = core.root().to_owned();
    let id = core.snippets()[0].id;
    let opened = Draft::of(&core.snippets()[0]);

    // This Mac's editor rewrites the last line; the other renames the snippet
    // and changes the first. This core has not read the folder since.
    other_mac_saves(&root, |draft| {
        draft.label = "Email signature".to_owned();
        draft.body = "Kind regards,\nSam\n\nSent from Aralo".to_owned();
    });
    let mut mine = opened.clone();
    mine.body = "Best,\nSam\n\nSent from my Mac".to_owned();

    assert_eq!(
        core.save_snippet_since(id, &opened, &mine).unwrap(),
        Saved::Written(id)
    );
    let text = read(&root);
    assert!(text.contains("label: Email signature"), "{text}");
    assert!(text.contains("future_key: keep me"), "{text}");
    assert!(
        text.trim_end()
            .ends_with("Kind regards,\nSam\n\nSent from my Mac"),
        "{text}"
    );
    assert_eq!(core.snippets()[0].file.front.label, "Email signature");
}

#[test]
fn a_change_to_what_the_draft_changed_writes_nothing_and_shows_both() {
    let (_folder, mut core) = library();
    let root = core.root().to_owned();
    let id = core.snippets()[0].id;
    let opened = Draft::of(&core.snippets()[0]);

    other_mac_saves(&root, |draft| {
        draft.body = "Best,\nSamuel\n\nSent from Aralo".to_owned()
    });
    let on_disk = read(&root);
    let mut mine = opened.clone();
    mine.body = "Best,\nSam L.\n\nSent from Aralo".to_owned();

    let Saved::Clashed(clash) = core.save_snippet_since(id, &opened, &mine).unwrap() else {
        panic!("a clash was written");
    };
    assert_eq!(read(&root), on_disk, "nothing is written on a clash");
    assert!(clash.clashes.body);
    assert!(clash.clashes.keys.is_empty());
    assert_eq!(clash.disk_text, on_disk);
    assert!(clash.mine_text.contains("Sam L."), "{}", clash.mine_text);
    assert!(
        clash.base_text.contains("Best,\nSam\n"),
        "{}",
        clash.base_text
    );
    let disk = clash.disk.expect("the file on disk reads");
    assert_eq!(disk.body, "Best,\nSamuel\n\nSent from Aralo");

    // Keeping this Mac's: the file as it now is becomes what the draft is
    // saved over, so the draft wins where the two disagree.
    assert_eq!(
        core.save_snippet_since(id, &disk, &mine).unwrap(),
        Saved::Written(id)
    );
    assert!(read(&root).contains("Sam L."));
}

#[test]
fn a_clash_on_a_setting_names_the_setting() {
    let (_folder, mut core) = library();
    let root = core.root().to_owned();
    let id = core.snippets()[0].id;
    let opened = Draft::of(&core.snippets()[0]);

    other_mac_saves(&root, |draft| draft.abbr = vec![";sig".to_owned()]);
    let mut mine = opened.clone();
    mine.abbr = vec!["sig!".to_owned()];

    let Saved::Clashed(clash) = core.save_snippet_since(id, &opened, &mine).unwrap() else {
        panic!("a clash was written");
    };
    assert_eq!(clash.clashes.keys, ["abbr"]);
    assert!(!clash.clashes.body);
}

#[test]
fn with_nothing_changed_on_disk_it_is_a_plain_save() {
    let (_folder, mut core) = library();
    let root = core.root().to_owned();
    let id = core.snippets()[0].id;
    let opened = Draft::of(&core.snippets()[0]);
    let mut mine = opened.clone();
    mine.body = "Cheers".to_owned();

    assert_eq!(
        core.save_snippet_since(id, &opened, &mine).unwrap(),
        Saved::Written(id)
    );
    let text = read(&root);
    assert!(text.trim_end().ends_with("Cheers"), "{text}");
    assert!(text.contains("future_key: keep me"), "{text}");
}

#[test]
fn a_file_deleted_under_the_editor_is_written_again() {
    let (_folder, mut core) = library();
    let root = core.root().to_owned();
    let id = core.snippets()[0].id;
    let opened = Draft::of(&core.snippets()[0]);
    fs::remove_file(root.join("sig.md")).unwrap();
    let mut mine = opened.clone();
    mine.body = "Cheers".to_owned();

    assert_eq!(
        core.save_snippet_since(id, &opened, &mine).unwrap(),
        Saved::Written(id)
    );
    assert!(read(&root).trim_end().ends_with("Cheers"));
}

#[test]
fn a_file_moved_by_another_mac_is_saved_where_it_went() {
    let (_folder, mut core) = library();
    let root = core.root().to_owned();
    let id = core.snippets()[0].id;
    let opened = Draft::of(&core.snippets()[0]);

    let mut other = Core::open_without_starter(&root).unwrap();
    other.create_group(&["Work".to_owned()]).unwrap();
    other.move_snippet(id, &["Work".to_owned()]).unwrap();
    let moved = other.snippets()[0].path.clone();
    let mut mine = opened.clone();
    mine.body = "Cheers".to_owned();

    assert_eq!(
        core.save_snippet_since(id, &opened, &mine).unwrap(),
        Saved::Written(id)
    );
    assert!(!root.join("sig.md").exists(), "no second file with its ID");
    let text = fs::read_to_string(root.join(&moved)).unwrap();
    assert!(text.trim_end().ends_with("Cheers"), "{text}");
    assert_eq!(core.snippets().len(), 1);
}

#[test]
fn a_file_that_no_longer_reads_is_not_written_over() {
    let (_folder, mut core) = library();
    let root = core.root().to_owned();
    let id = core.snippets()[0].id;
    let opened = Draft::of(&core.snippets()[0]);
    let broken = "---\nlabel: [unclosed\n---\nhalf a sync\n";
    fs::write(root.join("sig.md"), broken).unwrap();
    let mut mine = opened.clone();
    mine.body = "Cheers".to_owned();

    let Saved::Clashed(clash) = core.save_snippet_since(id, &opened, &mine).unwrap() else {
        panic!("a file that does not read was written over");
    };
    assert_eq!(read(&root), broken);
    assert_eq!(clash.disk, None);
    assert_eq!(clash.disk_text, broken);
    assert!(clash.mine_text.trim_end().ends_with("Cheers"));
}

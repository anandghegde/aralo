//! The index over a real library folder: that an incremental sync lands in the
//! same place as a rebuild, that it only writes what moved, and that the two
//! tables a rebuild must not touch survive one.

use std::fs;
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

use aralo_library::{Index, Library};

fn write(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

/// A small library with a group tree, a disabled snippet and two snippets that
/// carry a real ID, so that a move can be followed.
fn library(root: &Path) -> Library {
    write(
        root,
        "Work/best-regards.md",
        "---\nid: 01J8ZK3V5Q8W6T9X2N4R7M0AA1\nlabel: Best regards\nabbr: \";br\"\ntags: [email, sign-off]\n---\nBest regards,\nSam\n",
    );
    write(
        root,
        "Work/Billing/invoice.md",
        "---\nid: 01J8ZK3V5Q8W6T9X2N4R7M0AA2\nlabel: Invoice line\nabbr: \";inv\"\n---\nInvoice due on Friday\n",
    );
    write(
        root,
        "Personal/thanks.md",
        "---\nlabel: Thanks\nabbr: [\";ty\", \";thx\"]\nenabled: false\n---\nThanks so much!\n",
    );
    write(
        root,
        "Personal/no-label.md",
        "---\nabbr: \";addr\"\n---\n12 Example Street\n",
    );
    write(
        root,
        "Work/_group.yaml",
        "colour: \"#3478F6\"\nicon: briefcase\n",
    );
    Library::load(root).unwrap()
}

fn indexed(library: &Library) -> Index {
    let mut index = Index::open_in_memory().unwrap();
    index.rebuild(library).unwrap();
    index
}

#[test]
fn an_incremental_sync_lands_where_a_rebuild_lands() {
    let folder = tempfile::tempdir().unwrap();
    let library = library(folder.path());

    let mut incremental = Index::open_in_memory().unwrap();
    incremental.sync(&library).unwrap();

    // Every kind of change at once: an edit, a new snippet in a new group, a
    // deletion that empties a group, and a move that keeps its ID.
    write(
        folder.path(),
        "Work/best-regards.md",
        "---\nid: 01J8ZK3V5Q8W6T9X2N4R7M0AA1\nlabel: Kind regards\nabbr: \";kr\"\ntags: [email]\n---\nKind regards,\nSam\n",
    );
    write(
        folder.path(),
        "Shared/Legal/nda.md",
        "---\nlabel: NDA\nabbr: \";nda\"\n---\nThis agreement is made on\n",
    );
    fs::remove_file(folder.path().join("Personal/thanks.md")).unwrap();
    fs::rename(
        folder.path().join("Work/Billing/invoice.md"),
        folder.path().join("Personal/invoice.md"),
    )
    .unwrap();

    let library = library.reload().unwrap();
    incremental.sync(&library).unwrap();

    let fresh = indexed(&library);
    assert_eq!(incremental.rows().unwrap(), fresh.rows().unwrap());
    assert_eq!(incremental.len().unwrap(), 4);
}

#[test]
fn a_sync_writes_only_what_moved() {
    let folder = tempfile::tempdir().unwrap();
    let library = library(folder.path());
    let mut index = Index::open_in_memory().unwrap();

    let first = index.sync(&library).unwrap();
    assert_eq!(first.added, 4);
    assert_eq!(first.unchanged, 0);

    // Nothing has changed on disk, so a second sync writes nothing at all.
    let again = index.sync(&library.reload().unwrap()).unwrap();
    assert_eq!(again.unchanged, 4);
    assert!(!again.changed());

    write(
        folder.path(),
        "Personal/thanks.md",
        "---\nlabel: Thanks\nabbr: [\";ty\", \";thx\"]\nenabled: false\n---\nThank you very much!\n",
    );
    fs::remove_file(folder.path().join("Personal/no-label.md")).unwrap();
    let after = index.sync(&library.reload().unwrap()).unwrap();
    assert_eq!(after.updated, 1);
    assert_eq!(after.removed, 1);
    assert_eq!(after.unchanged, 2);
    assert_eq!(after.added, 0);
}

#[test]
fn a_file_with_no_id_is_not_reindexed_on_every_load() {
    let folder = tempfile::tempdir().unwrap();
    write(
        folder.path(),
        "Personal/no-label.md",
        "---\nabbr: \";addr\"\n---\n12 Example Street\n",
    );
    let library = Library::load(folder.path()).unwrap();
    let mut index = Index::open_in_memory().unwrap();
    index.sync(&library).unwrap();

    // A hand-written file without an `id` gets one derived from its path, so
    // loading the folder again does not look like a different snippet.
    let again = index.sync(&Library::load(folder.path()).unwrap()).unwrap();
    assert_eq!(again.unchanged, 1);
    assert!(!again.changed());
}

#[test]
fn moving_a_snippet_keeps_its_content_hash() {
    let folder = tempfile::tempdir().unwrap();
    let library = library(folder.path());
    let index = indexed(&library);
    let before = hash_of(&index, "Work/best-regards.md");

    fs::create_dir_all(folder.path().join("Archive")).unwrap();
    fs::rename(
        folder.path().join("Work/best-regards.md"),
        folder.path().join("Archive/best-regards.md"),
    )
    .unwrap();

    let mut index = index;
    let moved = index.sync(&library.reload().unwrap()).unwrap();
    assert_eq!(moved.updated, 1);
    // The hash covers the content and nothing else, which is what lets the
    // embedding cache in `vectors` survive a move.
    assert_eq!(hash_of(&index, "Archive/best-regards.md"), before);
}

/// The `hash` column of the row at `path`, read out of the dump.
fn hash_of(index: &Index, path: &str) -> String {
    let row = index
        .rows()
        .unwrap()
        .into_iter()
        .find(|row| row.starts_with("snippet\t") && row.contains(&format!("\t{path}\t")))
        .unwrap_or_else(|| panic!("no row for {path}"));
    row.split('\t').nth(4).unwrap().to_owned()
}

#[test]
fn search_finds_a_snippet_by_any_of_its_words() {
    let folder = tempfile::tempdir().unwrap();
    let library = library(folder.path());
    let index = indexed(&library);

    let names = |text: &str| -> Vec<String> {
        index
            .search(text, 10)
            .unwrap()
            .into_iter()
            .map(|id| library.snippet(id).unwrap().display_name().to_owned())
            .collect()
    };

    assert_eq!(names("regards"), ["Best regards"]);
    assert_eq!(names("invoice"), ["Invoice line"]);
    assert_eq!(names("street"), [";addr"]);
    // Search-as-you-type: the last word matches as a prefix.
    assert_eq!(names("regar"), ["Best regards"]);
    // A tag is searchable, and so is a disabled snippet: the user is looking
    // for it in order to turn it back on.
    assert_eq!(names("sign-off"), ["Best regards"]);
    assert_eq!(names("thanks"), ["Thanks"]);
    assert!(names("nothing here matches").is_empty());
}

#[test]
fn what_the_user_types_is_never_read_as_fts_syntax() {
    let folder = tempfile::tempdir().unwrap();
    let library = library(folder.path());
    let index = indexed(&library);

    // Each of these is a syntax error to FTS5 if passed through unquoted.
    for text in ["\"", "*", "NEAR(", "a OR", "-", "^regards", "()"] {
        index
            .search(text, 10)
            .unwrap_or_else(|error| panic!("`{text}` failed: {error}"));
    }
}

#[test]
fn a_rebuild_keeps_the_counts_it_cannot_recompute() {
    let folder = tempfile::tempdir().unwrap();
    let library = library(folder.path());
    let mut index = indexed(&library);

    let id = library
        .snippets()
        .iter()
        .find(|snippet| snippet.display_name() == "Best regards")
        .unwrap()
        .id;
    let at = UNIX_EPOCH + Duration::from_millis(1_700_000_000_000);
    index.record_expansion(id, at).unwrap();
    index
        .record_expansion(id, at + Duration::from_secs(1))
        .unwrap();

    index.rebuild(&library).unwrap();

    // `stats` is local, is never synced and is not rebuildable from the files,
    // so a rebuild must leave it alone.
    let stats = index.stats(id).unwrap();
    assert_eq!(stats.expansions, 2);
    assert_eq!(stats.last_used, Some(at + Duration::from_secs(1)));
    assert_eq!(index.recents(10).unwrap()[0].id, id);
}

#[test]
fn the_group_tree_follows_the_folders() {
    let folder = tempfile::tempdir().unwrap();
    let library = library(folder.path());
    let index = indexed(&library);

    let groups: Vec<String> = index
        .rows()
        .unwrap()
        .into_iter()
        .filter(|row| row.starts_with("group\t"))
        .collect();
    assert_eq!(
        groups,
        [
            // path, name, parent, depth, snippets directly inside, colour,
            // icon, enabled
            "group\tPersonal\tPersonal\t\t1\t2\t\t\t1",
            "group\tWork\tWork\t\t1\t1\t#3478F6\tbriefcase\t1",
            // A group that only holds other groups is still in the tree.
            "group\tWork/Billing\tBilling\tWork\t2\t1\t\t\t1",
        ]
    );
}

#[test]
fn a_group_that_is_switched_off_switches_off_the_groups_inside_it() {
    let folder = tempfile::tempdir().unwrap();
    let root = folder.path();
    library(root);
    write(
        root,
        "Work/_group.yaml",
        "name: Client work
enabled: false
",
    );
    // A sub-group cannot switch itself back on.
    write(
        root,
        "Work/Billing/_group.yaml",
        "enabled: true
",
    );
    let library = Library::load(root).unwrap();
    let index = indexed(&library);

    let groups: Vec<String> = index
        .rows()
        .unwrap()
        .into_iter()
        .filter(|row| row.starts_with("group\t"))
        .collect();
    assert_eq!(
        groups,
        [
            "group\tPersonal\tPersonal\t\t1\t2\t\t\t1",
            "group\tWork\tClient work\t\t1\t1\t\t\t0",
            "group\tWork/Billing\tBilling\tWork\t2\t1\t\t\t0",
        ]
    );

    // And the snapshot the engine matches on has lost their abbreviations.
    let (snapshot, _) = library.snapshot();
    assert_eq!(snapshot.len(), 1, "only Personal/no-label.md is left");
}

#[test]
fn an_index_reopened_on_disk_is_the_one_that_was_written() {
    let folder = tempfile::tempdir().unwrap();
    let library = library(folder.path());
    // The index never lives inside the library folder; a SQLite file in a
    // synced folder is asking for corruption.
    let elsewhere = tempfile::tempdir().unwrap();
    let path = elsewhere.path().join("state/index.sqlite3");

    let rows = {
        let mut index = Index::open(&path).unwrap();
        index.sync(&library).unwrap();
        index.rows().unwrap()
    };

    let reopened = Index::open(&path).unwrap();
    assert_eq!(reopened.rows().unwrap(), rows);

    // And nothing is left to do.
    let mut reopened = reopened;
    let again = reopened.sync(&library.reload().unwrap()).unwrap();
    assert!(!again.changed());
}

/// The size the plan measures M2 task 2.1 against: ten thousand snippets.
fn ten_thousand(root: &Path) -> Library {
    for group in 0..100 {
        for snippet in 0..100 {
            write(
                root,
                &format!("Group{group:03}/snippet-{snippet:03}.md"),
                &format!(
                    "---\nlabel: Snippet {group}-{snippet}\nabbr: \";g{group}s{snippet}\"\n---\nBody of snippet {snippet} in group {group}\n"
                ),
            );
        }
    }
    let library = Library::load(root).unwrap();
    assert_eq!(library.snippets().len(), 10_000);
    library
}

/// Ten thousand snippets index without the work growing faster than the folder
/// does, and a second pass over an unchanged folder writes nothing at all.
#[test]
fn ten_thousand_snippets_index_and_then_stay_put() {
    let folder = tempfile::tempdir().unwrap();
    let library = ten_thousand(folder.path());

    let mut index = Index::open_in_memory().unwrap();
    let first = index.sync(&library).unwrap();
    assert_eq!(first.added, 10_000);
    assert_eq!(index.len().unwrap(), 10_000);

    let again = index.sync(&library.reload().unwrap()).unwrap();
    assert_eq!(again.unchanged, 10_000);
    assert!(!again.changed());

    write(
        folder.path(),
        "Group042/snippet-042.md",
        "---\nlabel: Snippet 42-42\nabbr: \";g42s42\"\n---\nEdited\n",
    );
    let after = index.sync(&library.reload().unwrap()).unwrap();
    assert_eq!(after.updated, 1);
    assert_eq!(after.unchanged, 9_999);
}

/// The other half of the plan's bar: the cold rebuild is a background job that
/// expansion does not wait for. It cannot wait for it, by construction —
/// abbreviations reach the engine through the snapshot, which is built from the
/// loaded files and never consults the index — and this is that, demonstrated.
#[test]
fn a_cold_index_does_not_hold_up_an_expansion() {
    use std::sync::Arc;

    use aralo_engine::{Engine, KeyEvent, KeyVerdict};

    let folder = tempfile::tempdir().unwrap();
    let library = ten_thousand(folder.path());
    let (snapshot, rejected) = library.snapshot();
    assert!(rejected.is_empty());

    let indexing = std::thread::spawn(move || {
        let mut index = Index::open_in_memory().unwrap();
        let counted = index.sync(&library).unwrap();
        (index, counted)
    });

    // Meanwhile, on this thread and with the index still being written, an
    // abbreviation expands.
    let mut engine = Engine::new();
    engine.set_snapshot(Arc::new(snapshot));
    let mut verdict = KeyVerdict::Pass;
    for character in ";g42s42 ".chars() {
        verdict = engine.on_key(KeyEvent::Char(character));
    }
    assert!(
        matches!(verdict, KeyVerdict::Match { .. }),
        "the space after the abbreviation should have expanded it, got {verdict:?}"
    );

    let (index, counted) = indexing.join().unwrap();
    assert_eq!(counted.added, 10_000);
    assert_eq!(index.len().unwrap(), 10_000);
}

#[test]
fn a_merge_base_is_the_last_settled_version_and_waits_out_a_conflict() {
    let folder = tempfile::tempdir().unwrap();
    let root = folder.path();
    let library = library(root);
    let mut index = indexed(&library);
    let id = "01J8ZK3V5Q8W6T9X2N4R7M0AA1".parse().unwrap();
    assert_eq!(index.base(id).unwrap().unwrap().body, "Best regards,\nSam");
    // A file with no id has nothing for a copy to share, so no base.
    let no_id = library
        .snippets()
        .iter()
        .find(|s| s.id_is_temporary)
        .unwrap();
    assert_eq!(index.base(no_id.id).unwrap(), None);

    // A conflict arrives with the original changed under it: the base stays
    // where both machines last agreed.
    write(
        root,
        "Work/best-regards.md",
        "---\nid: 01J8ZK3V5Q8W6T9X2N4R7M0AA1\nlabel: Best regards\nabbr: \";br\"\n---\nBest,\nSam\n",
    );
    write(
        root,
        "Work/best-regards 2.md",
        "---\nid: 01J8ZK3V5Q8W6T9X2N4R7M0AA1\nlabel: Kind regards\nabbr: \";br\"\n---\nBest regards,\nSam\n",
    );
    index.sync(&library.reload().unwrap()).unwrap();
    assert_eq!(index.base(id).unwrap().unwrap().body, "Best regards,\nSam");

    // Once the copy has gone, the file is the settled version again.
    fs::remove_file(root.join("Work/best-regards 2.md")).unwrap();
    index.sync(&library.reload().unwrap()).unwrap();
    assert_eq!(index.base(id).unwrap().unwrap().body, "Best,\nSam");

    // And a snippet that has gone takes its base with it.
    fs::remove_file(root.join("Work/best-regards.md")).unwrap();
    index.sync(&library.reload().unwrap()).unwrap();
    assert_eq!(index.base(id).unwrap(), None);
}

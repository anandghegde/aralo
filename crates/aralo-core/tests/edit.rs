//! Editing a library through the core: what lands on disk, and what a second
//! reader sees afterwards.
//!
//! The assertions are about files, because the files are the library. A test
//! that only asked the in-memory `Core` what it believes would pass just as
//! happily if nothing had been written at all.

use std::fs;
use std::path::Path;

use aralo_core::snippet::{SnippetFile, SnippetKind, TriggerMode};
use aralo_core::{Core, Draft, Problem};

fn library() -> (tempfile::TempDir, Core) {
    let folder = tempfile::tempdir().unwrap();
    let root = folder.path().join("Aralo");
    let core = Core::open_without_starter(&root).unwrap();
    (folder, core)
}

fn draft(label: &str, abbr: &str, body: &str) -> Draft {
    Draft {
        label: label.to_owned(),
        abbr: vec![abbr.to_owned()],
        body: body.to_owned(),
        ..Draft::default()
    }
}

fn read(root: &Path, relative: &str) -> String {
    fs::read_to_string(root.join(relative)).unwrap()
}

fn group_of(core: &Core, label: &str) -> Vec<String> {
    core.snippets()
        .iter()
        .find(|snippet| snippet.file.front.label == label)
        .unwrap_or_else(|| panic!("no snippet called {label}"))
        .group
        .clone()
}

#[test]
fn a_new_snippet_is_a_file_named_after_it() {
    let (_folder, mut core) = library();
    let root = core.root().to_owned();
    let id = core
        .create_snippet(
            &["Work".to_owned()],
            &draft("Best regards", "br", "Best,\nAnand"),
        )
        .unwrap();

    let text = read(&root, "Work/best-regards.md");
    assert!(text.contains(&format!("id: {id}")), "{text}");
    assert!(text.trim_end().ends_with("Best,\nAnand"), "{text}");

    let reopened = Core::open_read_only(&root).unwrap();
    let snippet = reopened.library().snippet(id).unwrap();
    assert_eq!(snippet.group, ["Work"]);
    assert_eq!(snippet.file.front.abbr, ["br"]);
    assert!(!snippet.id_is_temporary);
}

#[test]
fn two_snippets_with_one_name_get_two_files() {
    let (_folder, mut core) = library();
    let root = core.root().to_owned();
    let first = core
        .create_snippet(&[], &draft("Address", "addr", "one"))
        .unwrap();
    let second = core
        .create_snippet(&[], &draft("Address", "addr2", "two"))
        .unwrap();

    assert_ne!(first, second);
    assert!(read(&root, "address.md").trim_end().ends_with("one"));
    assert!(read(&root, "address-2.md").trim_end().ends_with("two"));
    assert_eq!(core.snippets().len(), 2);
}

#[test]
fn a_snippet_with_nothing_to_be_named_after_still_gets_a_file() {
    let (_folder, mut core) = library();
    let root = core.root().to_owned();
    core.create_snippet(&[], &Draft::default()).unwrap();
    core.create_snippet(&[], &draft("", ";;", "punctuation only"))
        .unwrap();
    assert!(root.join("snippet.md").exists());
    assert!(root.join("snippet-2.md").exists());
}

#[test]
fn a_save_keeps_the_keys_this_version_does_not_know() {
    let (_folder, mut core) = library();
    let root = core.root().to_owned();
    fs::write(
        root.join("hand-written.md"),
        "---\nid: 01J0000000000000000000000A\nlabel: Signature\nabbr: [sig]\n\
         ai:\n  profile: work\n  tone: warm\nfuture_key: keep me\n---\nold body\n",
    )
    .unwrap();
    core.reload().unwrap();

    let id = core.snippets()[0].id;
    let mut edited = Draft::of(&core.snippets()[0]);
    edited.body = "new body".to_owned();
    edited.abbr.push("sign".to_owned());
    edited.trigger = Some(TriggerMode::Immediate);
    assert_eq!(core.save_snippet(id, &edited).unwrap(), id);

    let text = read(&root, "hand-written.md");
    assert!(text.contains("future_key: keep me"), "{text}");
    assert!(text.contains("tone: warm"), "{text}");
    assert!(text.contains("profile: work"), "{text}");
    assert!(text.contains("trigger: immediate"), "{text}");
    assert!(text.trim_end().ends_with("new body"), "{text}");
    assert_eq!(core.snippets()[0].file.front.abbr, ["sig", "sign"]);
}

#[test]
fn saving_a_file_that_carries_no_id_gives_it_one() {
    let (_folder, mut core) = library();
    let root = core.root().to_owned();
    fs::write(
        root.join("no-id.md"),
        "---\nlabel: Hello\nabbr: [hi]\n---\nhello\n",
    )
    .unwrap();
    core.reload().unwrap();

    let temporary = core.snippets()[0].id;
    assert!(core.snippets()[0].id_is_temporary);
    let saved = core
        .save_snippet(temporary, &Draft::of(&core.snippets()[0]))
        .unwrap();

    assert_ne!(saved, temporary);
    assert!(!core.snippets()[0].id_is_temporary);
    assert_eq!(core.snippets()[0].id, saved);
    assert!(read(&root, "no-id.md").contains(&format!("id: {saved}")));
}

#[test]
fn a_snippet_moves_to_another_group_keeping_its_id() {
    let (_folder, mut core) = library();
    let root = core.root().to_owned();
    let id = core
        .create_snippet(&[], &draft("Best regards", "br", "Best"))
        .unwrap();
    core.move_snippet(id, &["Work".to_owned(), "Email".to_owned()])
        .unwrap();

    assert!(!root.join("best-regards.md").exists());
    assert!(root.join("Work/Email/best-regards.md").exists());
    assert_eq!(group_of(&core, "Best regards"), ["Work", "Email"]);
    assert_eq!(core.library().snippet(id).unwrap().id, id);
}

#[test]
fn a_move_onto_a_taken_name_does_not_write_over_it() {
    let (_folder, mut core) = library();
    let root = core.root().to_owned();
    core.create_snippet(
        &["Work".to_owned()],
        &draft("Address", "waddr", "the office"),
    )
    .unwrap();
    let home = core
        .create_snippet(&[], &draft("Address", "haddr", "the flat"))
        .unwrap();
    core.move_snippet(home, &["Work".to_owned()]).unwrap();

    assert!(read(&root, "Work/address.md")
        .trim_end()
        .ends_with("the office"));
    assert!(read(&root, "Work/address-2.md")
        .trim_end()
        .ends_with("the flat"));
    assert_eq!(core.snippets().len(), 2);
}

#[test]
fn deleting_a_snippet_takes_its_file_and_nothing_else() {
    let (_folder, mut core) = library();
    let root = core.root().to_owned();
    let kept = core
        .create_snippet(&["Work".to_owned()], &draft("Kept", "k", "stay"))
        .unwrap();
    let gone = core
        .create_snippet(&["Work".to_owned()], &draft("Gone", "g", "go"))
        .unwrap();

    let path = core.snippet_path(gone).unwrap();
    assert!(path.exists());
    core.delete_snippet(gone).unwrap();

    assert!(!path.exists());
    assert!(root.join("Work").is_dir());
    assert!(core.library().snippet(gone).is_none());
    assert!(core.library().snippet(kept).is_some());
    assert!(matches!(
        core.delete_snippet(gone),
        Err(aralo_core::CoreError::NoSuchSnippet(_))
    ));
}

#[test]
fn a_group_is_a_folder_and_its_settings_are_one_file() {
    let (_folder, mut core) = library();
    let root = core.root().to_owned();
    core.create_group(&["Work".to_owned()]).unwrap();
    assert!(root.join("Work").is_dir());
    assert!(
        !root.join("Work/_group.yaml").exists(),
        "an empty group needs no file"
    );

    core.edit_group(&["Work".to_owned()], |file| {
        file.colour = Some("#3478F6".to_owned());
        file.icon = Some("briefcase".to_owned());
    })
    .unwrap();

    let group = core.library().group(&["Work".to_owned()]).unwrap();
    assert_eq!(group.colour.as_deref(), Some("#3478F6"));
    assert_eq!(group.icon.as_deref(), Some("briefcase"));
    assert!(group.enabled);
    assert!(read(&root, "Work/_group.yaml").contains("briefcase"));
}

#[test]
fn switching_a_group_off_switches_off_what_is_inside_it() {
    let (_folder, mut core) = library();
    core.create_snippet(&["Work".to_owned()], &draft("Best regards", "br", "Best"))
        .unwrap();
    assert_eq!(core.snapshot().len(), 1);

    core.set_group_enabled(&["Work".to_owned()], false).unwrap();
    assert_eq!(
        core.snapshot().len(),
        0,
        "a snippet in a group that is off never fires"
    );
    assert!(!core.library().group(&["Work".to_owned()]).unwrap().enabled);

    core.set_group_enabled(&["Work".to_owned()], true).unwrap();
    assert_eq!(core.snapshot().len(), 1);
}

#[test]
fn renaming_a_group_renames_the_folder_and_what_the_user_reads() {
    let (_folder, mut core) = library();
    let root = core.root().to_owned();
    let id = core
        .create_snippet(&["Wrok".to_owned()], &draft("Best regards", "br", "Best"))
        .unwrap();
    core.edit_group(&["Wrok".to_owned()], |file| {
        file.name = Some("Wrok".to_owned())
    })
    .unwrap();

    let renamed = core.rename_group(&["Wrok".to_owned()], "Work").unwrap();

    assert_eq!(renamed, ["Work"]);
    assert!(!root.join("Wrok").exists());
    assert!(root.join("Work/best-regards.md").exists());
    assert_eq!(
        core.library().group(&["Work".to_owned()]).unwrap().name,
        "Work"
    );
    assert!(read(&root, "Work/_group.yaml").contains("Work"));
    assert_eq!(group_of(&core, "Best regards"), ["Work"]);
    assert_eq!(
        core.library().snippet(id).unwrap().id,
        id,
        "a rename is not a new snippet"
    );
}

#[test]
fn a_group_moves_with_everything_in_it() {
    let (_folder, mut core) = library();
    let root = core.root().to_owned();
    core.create_snippet(&["Email".to_owned()], &draft("Best regards", "br", "Best"))
        .unwrap();
    core.create_snippet(
        &["Email".to_owned(), "Replies".to_owned()],
        &draft("On my way", "omw", "On my way"),
    )
    .unwrap();

    let moved = core
        .move_group(&["Email".to_owned()], &["Work".to_owned()])
        .unwrap();

    assert_eq!(moved, ["Work", "Email"]);
    assert!(root.join("Work/Email/best-regards.md").exists());
    assert!(root.join("Work/Email/Replies/on-my-way.md").exists());
    assert_eq!(group_of(&core, "On my way"), ["Work", "Email", "Replies"]);
}

#[test]
fn a_group_cannot_be_moved_inside_itself() {
    let (_folder, mut core) = library();
    core.create_snippet(
        &["Work".to_owned(), "Email".to_owned()],
        &draft("A", "a", "a"),
    )
    .unwrap();
    let work = vec!["Work".to_owned()];
    assert!(core
        .move_group(&work, &["Work".to_owned(), "Email".to_owned()])
        .is_err());
    assert!(core.move_group(&work, &work).is_err());
    assert!(core
        .library()
        .group(&["Work".to_owned(), "Email".to_owned()])
        .is_some());
}

#[test]
fn deleting_a_group_says_what_it_took() {
    let (_folder, mut core) = library();
    let root = core.root().to_owned();
    core.create_snippet(&["Work".to_owned()], &draft("A", "a", "a"))
        .unwrap();
    core.create_snippet(
        &["Work".to_owned(), "Email".to_owned()],
        &draft("B", "b", "b"),
    )
    .unwrap();
    core.create_snippet(&[], &draft("C", "c", "c")).unwrap();
    core.edit_group(&["Work".to_owned()], |file| {
        file.icon = Some("briefcase".to_owned())
    })
    .unwrap();

    assert_eq!(core.group_contents(&["Work".to_owned()]), 2);
    let removed = core.delete_group(&["Work".to_owned()]).unwrap();

    assert_eq!(
        removed.len(),
        3,
        "two snippets and the group file: {removed:?}"
    );
    assert!(!root.join("Work").exists());
    assert!(root.join("c.md").exists());
    assert_eq!(core.snippets().len(), 1);
    assert!(matches!(
        core.delete_group(&["Work".to_owned()]),
        Err(aralo_core::CoreError::NoSuchGroup(_))
    ));
}

#[test]
fn the_library_root_is_not_a_group_that_can_be_taken_away() {
    let (_folder, mut core) = library();
    assert!(core.delete_group(&[]).is_err());
    assert!(core.rename_group(&[], "Anything").is_err());
    assert!(core.move_group(&[], &["Work".to_owned()]).is_err());
    assert!(core.root().join("aralo.yaml").exists());
}

#[test]
fn a_group_name_cannot_reach_outside_the_library() {
    let (folder, mut core) = library();
    let outside = folder.path().join("escaped");
    fs::create_dir_all(&outside).unwrap();

    assert!(core
        .create_group(&["..".to_owned(), "escaped".to_owned()])
        .is_err());
    assert!(core
        .create_snippet(&["../escaped".to_owned()], &draft("A", "a", "a"))
        .is_err());
    assert!(core.create_group(&[".hidden".to_owned()]).is_err());
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
    assert!(core.snippets().is_empty());
}

#[test]
fn the_editor_is_told_which_abbreviation_is_already_taken() {
    let (_folder, mut core) = library();
    let taken = core
        .create_snippet(&["Work".to_owned()], &draft("Best regards", "br", "Best"))
        .unwrap();

    let mut new = draft("Bad request", "br", "400");
    new.abbr.push("  ".to_owned());
    new.abbr.push("bad".to_owned());
    new.abbr.push("bad".to_owned());
    new.kind = SnippetKind::Script;

    let issues = core.check_draft(&new, None);
    assert_eq!(issues.len(), 4, "{issues:#?}");
    assert!(matches!(
        issues[0].problem,
        Problem::UnsupportedKind(SnippetKind::Script)
    ));
    match &issues[1].problem {
        Problem::Taken { by, name, path } => {
            assert_eq!(*by, taken);
            assert_eq!(name, "Best regards");
            assert_eq!(path, Path::new("Work/best-regards.md"));
        }
        other => panic!("expected the other snippet to be named: {other:?}"),
    }
    assert_eq!(issues[2].problem, Problem::Blank);
    assert_eq!(issues[3].problem, Problem::Repeated);

    assert!(
        core.check_draft(&draft("Best regards", "br", "Best"), Some(taken))
            .is_empty(),
        "a snippet does not take its own abbreviation from itself"
    );
}

#[test]
fn a_suggested_abbreviation_is_one_nothing_answers_to() {
    let (_folder, mut core) = library();
    assert_eq!(core.suggest_abbreviation("Best regards"), "br");
    assert_eq!(core.suggest_abbreviation("Signature"), "sig");
    assert_eq!(
        core.suggest_abbreviation("On my way, sorry, running late"),
        "omws"
    );
    assert_eq!(core.suggest_abbreviation("!!!"), "");

    core.create_snippet(&[], &draft("Best regards", "br", "Best"))
        .unwrap();
    assert_eq!(core.suggest_abbreviation("Best regards"), "br2");
    core.create_snippet(&[], &draft("Bad request", "br2", "400"))
        .unwrap();
    assert_eq!(core.suggest_abbreviation("Best regards"), "br3");
}

#[test]
fn an_edit_is_expandable_the_moment_it_is_saved() {
    let (_folder, mut core) = library();
    let id = core
        .create_snippet(&[], &draft("Best regards", "br", "Best,\nAnand"))
        .unwrap();
    let snapshot = core.snapshot();
    assert_eq!(snapshot.len(), 1);

    core.set_snippet_enabled(id, false).unwrap();
    assert_eq!(core.snapshot().len(), 0);
    assert!(core.snippets()[0].file.front.enabled == Some(false));

    core.set_snippet_enabled(id, true).unwrap();
    let mut changed = Draft::of(&core.snippets()[0]);
    changed.abbr = vec![";br".to_owned()];
    core.save_snippet(id, &changed).unwrap();
    assert_eq!(core.snapshot().len(), 1);
    assert_eq!(core.snippets()[0].file.front.abbr, [";br"]);
}

#[test]
fn every_file_aralo_writes_reads_back_as_the_file_it_wrote() {
    let (_folder, mut core) = library();
    let root = core.root().to_owned();
    let id = core
        .create_snippet(
            &["Work".to_owned()],
            &draft(
                "Tricky",
                "tr",
                "--- not front matter ---\n\ttabs and  spaces\n",
            ),
        )
        .unwrap();

    let text = read(&root, "Work/tricky.md");
    let parsed = SnippetFile::parse(&text).unwrap();
    assert_eq!(parsed.front.id, Some(id));
    assert_eq!(
        parsed.body,
        "--- not front matter ---\n\ttabs and  spaces\n"
    );
    assert!(core.diagnostics().next().is_none());
}

#[test]
fn a_preview_is_what_typing_the_abbreviation_would_produce() {
    let (_folder, mut core) = library();
    let plain = core
        .create_snippet(&[], &draft("Best regards", "br", "Best,\nAnand"))
        .unwrap();
    let placeholder = core
        .create_snippet(
            &[],
            &draft("Address", "addr", "12 Somewhere Street\n{{cursor}}"),
        )
        .unwrap();

    assert_eq!(core.preview(plain).as_deref(), Some("Best,\nAnand"));
    assert_eq!(
        core.preview(placeholder).as_deref(),
        Some("12 Somewhere Street\n{{cursor}}"),
        "a placeholder expands as its own source until the evaluator lands in M3, \
         and the preview shows what would really be typed rather than pretending"
    );
    core.delete_snippet(plain).unwrap();
    assert_eq!(core.preview(plain), None);
}

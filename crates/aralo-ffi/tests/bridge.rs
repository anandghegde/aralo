//! The bridge as Swift will use it, driven from Rust.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use aralo_ffi::{
    Core, CoreEvents, DeleteStrategy, ImportSettings, InsertChoice, InsertMethod, KeyAction,
    KeyInput, LibraryEvent, MacroHandling, PlanKey, PlanStep, ResetReason, SearchQuery,
    SnippetDraft, SnippetType, UndoStyle,
};

/// How long a test waits for the watch and the indexer. Long enough that a
/// loaded machine does not fail the run; never reached when things work.
const PATIENCE: Duration = Duration::from_secs(10);

/// Polls until `condition` holds, so a test says what it is waiting for rather
/// than how long it sleeps.
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

/// A shell that writes down what it was told.
#[derive(Clone, Default)]
struct Shell(Arc<Mutex<Vec<LibraryEvent>>>);

impl Shell {
    fn heard(&self) -> Vec<LibraryEvent> {
        self.0.lock().unwrap().clone()
    }
}

impl CoreEvents for Shell {
    fn library_changed(&self, event: LibraryEvent) {
        self.0.lock().unwrap().push(event);
    }
}

fn draft(label: &str, abbr: &str, body: &str) -> SnippetDraft {
    SnippetDraft {
        label: label.into(),
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
fn the_editor_previews_what_is_being_typed() {
    let (_folder, core) = open();
    let id = core
        .create_snippet(vec![], draft("Sign off", "sig", "Best,\nAnand"))
        .unwrap();

    // The body on screen, not the one in the file: an editor's preview has to
    // keep up with the keystroke.
    assert_eq!(core.preview(id.clone()).as_deref(), Some("Best,\nAnand"));
    assert_eq!(
        core.preview_draft("Kind regards,\nA".into()),
        "Kind regards,\nA"
    );
    assert_eq!(
        core.preview_draft("{{cursor}} here".into()),
        "{{cursor}} here",
        "a placeholder previews as its own source until the evaluator lands in M3"
    );
}

/// The whole library, in list order: what the snippet list opens with, and
/// the base a test narrows one field at a time.
fn query(text: &str) -> SearchQuery {
    SearchQuery {
        text: text.into(),
        group: None,
        tag: None,
        enabled_only: false,
        limit: 0,
    }
}

fn key(c: char) -> KeyInput {
    KeyInput::Char { scalar: c as u32 }
}

/// Types text and returns the actions that were not `Pass`.
fn type_str(engine: &aralo_ffi::Engine, text: &str) -> Vec<KeyAction> {
    text.chars()
        .map(|c| engine.on_key(key(c)))
        .filter(|action| *action != KeyAction::Pass)
        .collect()
}

fn open() -> (tempfile::TempDir, Arc<Core>) {
    let (folder, core, _) = listening();
    (folder, core)
}

/// The same, with a shell attached, and with the cache inside the temporary
/// folder so that a test run never touches the one on the machine.
fn listening() -> (tempfile::TempDir, Arc<Core>, Shell) {
    let folder = tempfile::tempdir().unwrap();
    let shell = Shell::default();
    let core = Core::open_library(
        folder.path().join("Aralo").to_string_lossy().into_owned(),
        Some(folder.path().join("cache").to_string_lossy().into_owned()),
        Some(Arc::new(shell.clone())),
    )
    .unwrap();
    core.engine().set_front_app("com.apple.TextEdit".into());
    (folder, core, shell)
}

#[test]
fn a_match_comes_back_with_its_plan() {
    let (_folder, core) = open();
    let engine = core.engine();
    let actions = type_str(&engine, "Ty ");
    let [KeyAction::Expand {
        snippet_id,
        consume: true,
        steps,
        undo_delete_count: Some(10),
        profile,
    }] = actions.as_slice()
    else {
        panic!("{actions:?}");
    };
    assert_eq!(
        steps,
        &[
            PlanStep::Delete { count: 2 },
            PlanStep::InsertText {
                text: "Thank you ".into()
            },
        ]
    );

    engine.expansion_done(snippet_id.clone(), 10, InsertMethod::Typed);
    assert_eq!(
        engine.on_key(KeyInput::Undo),
        KeyAction::UndoExpansion {
            delete_count: 10,
            retype: "Ty ".into(),
            method: InsertMethod::Typed,
            profile: *profile,
        }
    );
}

#[test]
fn a_plan_carries_the_front_apps_row_of_the_compatibility_table() {
    let (_folder, core) = open();
    let engine = core.engine();
    let textedit = engine.injection_profile();
    assert_eq!(textedit.insert, InsertChoice::Auto);
    assert_eq!(textedit.undo, UndoStyle::Native);

    engine.set_front_app("com.apple.Terminal".into());
    let actions = type_str(&engine, "ty ");
    let [KeyAction::Expand { profile, .. }] = actions.as_slice() else {
        panic!("{actions:?}");
    };
    assert_eq!(profile.insert, InsertChoice::Type);
    assert_eq!(profile.undo, UndoStyle::Backspace);

    // An app the table has never heard of gets the defaults.
    engine.set_front_app("com.example.unheard-of".into());
    assert_eq!(engine.injection_profile(), textedit);
}

#[test]
fn a_compat_file_replaces_the_table_and_a_bad_one_changes_nothing() {
    let (folder, core) = open();
    let engine = core.engine();
    let path = folder.path().join("apps.toml");
    let load = || core.load_compat_table(path.to_string_lossy().into_owned());

    std::fs::write(
        &path,
        "version = 0
[[app]]
bundle_id = \"com.apple.TextEdit\"
delete = \"select\"
key_delay_ms = 7
",
    )
    .unwrap();
    load().unwrap();
    // The front app did not change, but its row did.
    let profile = engine.injection_profile();
    assert_eq!(profile.delete, DeleteStrategy::Select);
    assert_eq!(profile.key_delay_ms, 7);

    std::fs::write(
        &path,
        "version = 0
[defaults]
key_delay_ms = 99999
",
    )
    .unwrap();
    assert!(load().is_err());
    std::fs::remove_file(&path).unwrap();
    assert!(load().is_err());
    assert_eq!(engine.injection_profile(), profile);
}

#[test]
fn the_tables_apps_are_listed_for_the_matrix() {
    let apps = aralo_ffi::compat_apps(None).unwrap();
    assert_eq!(apps.len(), 15);
    assert_eq!(apps[0].name, "TextEdit");
    let terminal = apps
        .iter()
        .find(|app| app.bundle_id == "com.apple.Terminal")
        .unwrap();
    assert_eq!(terminal.profile.insert, InsertChoice::Type);

    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("apps.toml");
    std::fs::write(&path, "version = 0\n[defaults]\ninsert = \"paste\"\n").unwrap();
    let path = path.to_string_lossy().into_owned();
    assert!(aralo_ffi::compat_apps(Some(path)).unwrap().is_empty());
    assert!(aralo_ffi::compat_apps(Some("/nowhere/apps.toml".into())).is_err());
}

#[test]
fn return_is_sent_back_as_a_key_and_offers_no_undo() {
    let (_folder, core) = open();
    let actions = type_str(&core.engine(), "omw\r");
    let [KeyAction::Expand {
        steps,
        undo_delete_count: None,
        ..
    }] = actions.as_slice()
    else {
        panic!("{actions:?}");
    };
    assert_eq!(
        steps.last(),
        Some(&PlanStep::KeyPress {
            key: PlanKey::Return
        })
    );
}

#[test]
fn resets_pause_and_bad_input_never_expand() {
    let (_folder, core) = open();
    let engine = core.engine();

    type_str(&engine, "t");
    engine.reset(ResetReason::MouseDown);
    assert!(engine.holds_no_keystrokes());
    assert!(type_str(&engine, "y ").is_empty());

    // Not a Unicode scalar: treated as unmappable input.
    type_str(&engine, "t");
    assert_eq!(
        engine.on_key(KeyInput::Char { scalar: 0xD800 }),
        KeyAction::Pass
    );
    assert!(engine.holds_no_keystrokes());

    engine.set_paused(true);
    assert!(engine.is_paused());
    assert!(type_str(&engine, "ty ").is_empty());
    engine.set_paused(false);
    assert_eq!(type_str(&engine, "ty ").len(), 1);

    // A garbage id must not panic or arm anything.
    engine.expansion_done("not-an-id".into(), 3, InsertMethod::Typed);
    assert_eq!(engine.on_key(KeyInput::Undo), KeyAction::Pass);
}

#[test]
fn excluded_apps_never_expand() {
    let (_folder, core) = open();
    let engine = core.engine();
    engine.set_excluded_apps(vec!["com.example.Secrets".into()]);
    engine.set_front_app("com.example.Secrets".into());
    assert!(type_str(&engine, "ty ").is_empty());
    engine.set_front_app(aralo_ffi::excluded_app_presets()[0].clone());
    assert!(type_str(&engine, "ty ").is_empty());
}

/// Types `text` from a clean slate, so a poll that did not match leaves
/// nothing behind for the next attempt.
fn retype(engine: &aralo_ffi::Engine, text: &str) -> Vec<KeyAction> {
    engine.reset(ResetReason::MouseDown);
    type_str(engine, text)
}

#[test]
fn a_file_someone_else_writes_reaches_the_engine_unasked() {
    let (_folder, core, shell) = listening();
    let root = std::path::PathBuf::from(core.library_path());
    std::fs::write(root.join("hello.md"), "---\nabbr: ;hi\n---\nhello there").unwrap();

    let engine = core.engine();
    until("the watch to read a file Aralo did not write", || {
        !retype(&engine, ";hi ").is_empty()
    });
    assert!(core.snippets().iter().any(|s| s.abbreviations == [";hi"]));
    assert!(shell.heard().iter().any(|event| matches!(
        event,
        LibraryEvent::Outside { paths } if paths.iter().any(|path| path == "hello.md")
    )));
}

#[test]
fn reload_reaches_the_engine() {
    let (_folder, core, shell) = listening();
    let root = std::path::PathBuf::from(core.library_path());
    std::fs::write(root.join("hello.md"), "---\nabbr: ;hi\n---\nhello there").unwrap();
    core.reload().unwrap();
    assert_eq!(type_str(&core.engine(), ";hi ").len(), 1);
    assert!(shell.heard().contains(&LibraryEvent::Reloaded));

    // The folder disappears: reload fails, the loaded library keeps working.
    std::fs::remove_dir_all(&root).unwrap();
    assert!(core.reload().is_err());
    assert_eq!(retype(&core.engine(), ";hi ").len(), 1);
}

#[test]
fn a_snippet_the_shell_writes_is_a_file_and_expands() {
    let (_folder, core, shell) = listening();
    let id = core
        .create_snippet(
            vec!["Work".into()],
            draft("Address", ";addr", "1 Long Road"),
        )
        .unwrap();

    let root = std::path::PathBuf::from(core.library_path());
    let detail = core.snippet(id.clone()).unwrap();
    assert_eq!(detail.group, ["Work"]);
    assert_eq!(detail.preview, "1 Long Road");
    assert!(root.join(&detail.path).is_file());
    assert_eq!(type_str(&core.engine(), ";addr ").len(), 1);

    // Inherited, because the draft said nothing about either.
    assert!(detail.resolved.enabled);
    assert!(detail.draft.enabled.is_none());

    let saved = core
        .save_snippet(id.clone(), draft("Address", ";addr", "2 Short Road"))
        .unwrap();
    assert_eq!(saved, id);
    assert_eq!(core.preview(id.clone()).unwrap(), "2 Short Road");
    assert!(std::fs::read_to_string(root.join(&detail.path))
        .unwrap()
        .contains("2 Short Road"));

    core.set_snippet_enabled(id.clone(), false).unwrap();
    assert!(!core.snippet(id.clone()).unwrap().resolved.enabled);

    core.delete_snippet(id.clone()).unwrap();
    assert!(core.snippet(id).is_none());
    assert!(!root.join(&detail.path).exists());
    assert!(shell.heard().contains(&LibraryEvent::Edited));
}

#[test]
fn groups_are_made_renamed_moved_and_taken_away() {
    let (_folder, core) = open();
    core.create_group(vec!["Work".into()]).unwrap();
    core.create_snippet(vec!["Work".into()], draft("Note", ";n", "note"))
        .unwrap();
    assert_eq!(core.group_contents(vec!["Work".into()]), 1);

    let renamed = core
        .rename_group(vec!["Work".into()], "Office".into())
        .unwrap();
    assert_eq!(renamed, ["Office"]);

    core.create_group(vec!["Home".into()]).unwrap();
    let moved = core
        .move_group(vec!["Office".into()], vec!["Home".into()])
        .unwrap();
    assert_eq!(moved, ["Home", "Office"]);

    core.set_group_enabled(vec!["Home".into()], false).unwrap();
    core.set_group_appearance(vec!["Home".into()], Some("#ff0000".into()), None)
        .unwrap();
    let home = core
        .groups()
        .into_iter()
        .find(|group| group.path == ["Home"])
        .unwrap();
    assert!(!home.enabled);
    assert_eq!(home.colour.as_deref(), Some("#ff0000"));
    // Off is sticky: nothing inside a group that is off expands.
    assert!(type_str(&core.engine(), ";n ").is_empty());

    let removed = core.delete_group(vec!["Home".into()]).unwrap();
    assert!(removed.iter().any(|path| path.ends_with("note.md")));
    assert!(!core.groups().iter().any(|group| group.path == ["Home"]));
    // The library root is not a group anything can take away.
    assert!(core.delete_group(Vec::new()).is_err());
}

#[test]
fn the_editor_is_warned_about_a_draft_before_it_saves_it() {
    let (_folder, core) = open();
    let taken = core
        .create_snippet(Vec::new(), draft("Address", ";addr", "1 Long Road"))
        .unwrap();

    let mut clash = draft("Other", ";addr", "somewhere else");
    clash.abbreviations.push("  ".into());
    let problems = core.check_draft(clash.clone(), None);
    assert_eq!(problems.len(), 2);
    let conflict = problems
        .iter()
        .find(|problem| problem.abbreviation.as_deref() == Some(";addr"))
        .unwrap();
    assert_eq!(conflict.conflicts_with.as_deref(), Some(taken.as_str()));
    assert!(conflict.message.contains("Address"));

    // The snippet on screen does not clash with itself.
    assert!(core
        .check_draft(draft("Address", ";addr", "1 Long Road"), Some(taken))
        .is_empty());

    assert_eq!(
        core.suggest_abbreviation("Thank You Very Much".into()),
        "tyvm"
    );
    assert_ne!(core.suggest_abbreviation("Address".into()), ";addr");
}

#[test]
fn search_finds_what_was_just_written() {
    let (_folder, core) = open();
    core.create_snippet(
        vec!["Work".into()],
        draft("Standup", ";stand", "Yesterday I ..."),
    )
    .unwrap();

    let hits = core.search(query("stand"));
    let standup = hits.first().expect("a hit");
    assert_eq!(standup.name, "Standup");
    assert_eq!(standup.group, ["Work"]);
    assert!(!standup.matched.is_empty());

    // An empty query is the whole library, which is what the list shows.
    assert_eq!(core.search(query("")).len(), core.snippets().len());
    assert_eq!(
        core.search(SearchQuery {
            limit: 2,
            ..query("")
        })
        .len(),
        2
    );
    assert!(core.search(query("nothing answers to this")).is_empty());
}

#[test]
fn the_snippet_list_asks_for_one_group_at_a_time() {
    let (_folder, core) = open();
    core.create_snippet(
        vec!["Work".into()],
        draft("Standup", ";stand", "Yesterday I ..."),
    )
    .unwrap();
    core.create_snippet(
        vec!["Work".into(), "Reviews".into()],
        draft("Nit", ";nit", "Nit: "),
    )
    .unwrap();
    let home = core
        .create_snippet(
            vec!["Home".into()],
            draft("Address", ";addr", "1 Long Road"),
        )
        .unwrap();

    // A group carries what is inside it, so the sidebar's parent row is not
    // emptier than the rows under it.
    let work = SearchQuery {
        group: Some(vec!["Work".into()]),
        ..query("")
    };
    let names: Vec<_> = core
        .search(work.clone())
        .into_iter()
        .map(|h| h.name)
        .collect();
    assert_eq!(names, ["Nit", "Standup"]);

    // The search box narrows what the sidebar already chose, rather than
    // replacing it: a hit in another group stays out.
    assert!(core.search(SearchQuery { ..query("addr") }).len() == 1);
    assert!(core
        .search(SearchQuery {
            text: "addr".into(),
            ..work
        })
        .is_empty());

    core.set_snippet_enabled(home.clone(), false).unwrap();
    let live = SearchQuery {
        enabled_only: true,
        ..query("")
    };
    assert!(!core.search(live).into_iter().any(|hit| hit.id == home));
}

#[test]
fn an_expansion_is_counted_and_comes_back_as_a_recent() {
    let (_folder, core) = open();
    let engine = core.engine();
    let actions = type_str(&engine, "ty ");
    let [KeyAction::Expand { snippet_id, .. }] = actions.as_slice() else {
        panic!("{actions:?}");
    };
    engine.expansion_done(snippet_id.clone(), 10, InsertMethod::Typed);

    assert_eq!(core.index_problem(), None);
    let wanted = snippet_id.clone();
    until("the indexer to count the expansion", || {
        core.recents(5).first() == Some(&wanted)
    });
}

#[test]
fn an_import_says_what_it_would_do_before_it_does_it() {
    let (folder, core) = open();
    let source = folder.path().join("from-elsewhere.csv");
    std::fs::write(
        &source,
        "abbr,label,group,body\n;sig,Signature,Work,Yours\n;brb,Back soon,,Back in five\n",
    )
    .unwrap();
    let source = source.to_string_lossy().into_owned();

    let settings = |dry_run| ImportSettings {
        format: None,
        into_group: vec!["Imported".into()],
        macros: MacroHandling::Auto,
        dry_run,
    };

    let planned = core.import(source.clone(), settings(true)).unwrap();
    assert_eq!(planned.format, "csv");
    assert_eq!(planned.total, 2);
    assert_eq!(planned.imported, 2);
    assert_eq!(planned.fidelity, 1.0);
    assert!(planned.report.starts_with("Would import 2 snippets"));
    assert!(planned.report.contains("100% fidelity"));
    // A dry run wrote nothing.
    assert!(!core.snippets().iter().any(|s| s.abbreviations == [";sig"]));

    let done = core.import(source, settings(false)).unwrap();
    assert_eq!(done.imported, 2);
    let signature = done
        .entries
        .iter()
        .find(|entry| entry.label == "Signature")
        .unwrap();
    assert_eq!(signature.group, ["Imported", "Work"]);
    assert!(signature.path.is_some());
    assert_eq!(type_str(&core.engine(), ";sig ").len(), 1);

    assert!(core
        .import("/nowhere/at/all.csv".into(), settings(true))
        .is_err());
}

#[test]
fn the_library_comes_back_out_in_a_format_something_else_can_read() {
    let (_folder, core) = open();
    let json = core.export("json".into(), Vec::new()).unwrap();
    let text = String::from_utf8(json).unwrap();
    assert!(text.contains("\"abbr\""));
    assert!(text.contains("thank you"));

    // One group of it, and nothing else.
    let basics = core.export("csv".into(), vec!["Basics".into()]).unwrap();
    assert!(!basics.is_empty());
    assert!(core.export("textexpander".into(), Vec::new()).is_err());
    assert!(core.export("not-a-format".into(), Vec::new()).is_err());
}

#[test]
fn lists_snippets_and_diagnostics() {
    let (_folder, core) = open();
    let snippets = core.snippets();
    let thanks = snippets.iter().find(|s| s.abbreviations == ["ty"]).unwrap();
    assert_eq!(thanks.group, ["Basics"]);
    assert_eq!(thanks.preview, "thank you");
    assert_eq!(thanks.id.len(), 26);

    let diagnostics = core.diagnostics();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].path, "README.md");
    assert_eq!(diagnostics[0].level, aralo_ffi::DiagnosticLevel::Note);
}

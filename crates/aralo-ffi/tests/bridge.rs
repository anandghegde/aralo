//! The bridge as Swift will use it, driven from Rust.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use aralo_ffi::{
    BodyProblem, BridgeError, ConflictChoice, ContextNeed, ContextSupply, Core, CoreEvents,
    DeleteStrategy, DiagnosticLevel, ImportSettings, InsertChoice, InsertMethod, InsertOutcome,
    InsertRefusal, KeyAction, KeyInput, LibraryEvent, MacroHandling, PlanKey, PlanStep,
    ResetReason, SaveOutcome, SearchQuery, SessionAction, SnippetDraft, SnippetType, Trash,
    TrialAction, TrialField, UndoStyle,
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
        " here",
        "a cursor stop is where the caret lands, not text"
    );
    assert_eq!(
        core.preview_draft("Dear {{field: name | default: friend}},".into()),
        "Dear friend,",
        "nobody has been asked yet, so a form field previews its default"
    );
    assert_eq!(
        core.preview_draft("{{nonsense}}".into()),
        "{{nonsense}}",
        "a placeholder Aralo cannot expand stays as written rather than \
         becoming nothing"
    );
}

#[test]
fn the_editor_is_told_where_to_highlight_and_what_to_underline() {
    let (_folder, core) = open();

    // A body that reads: one placeholder, nothing to say about it.
    let outline = core.outline_draft("Hi {{field: name}}!".into());
    assert_eq!(outline.problems, []);
    assert_eq!(outline.placeholders.len(), 1);
    let field = &outline.placeholders[0];
    assert_eq!(field.name, "field");
    assert!(field.known);
    assert!(field.evaluated, "a form field is expanded, given an answer");
    assert_eq!((field.start, field.end), (3, 18));

    // An unclosed `{{` is underlined over the rest of the body, which is what
    // an expansion does with it: insert it as literal text.
    let outline = core.outline_draft("before {{date: %Y".into());
    assert_eq!(outline.placeholders, []);
    assert_eq!(outline.problems.len(), 1);
    assert_eq!(outline.problems[0].level, DiagnosticLevel::Error);
    assert_eq!(
        (outline.problems[0].start, outline.problems[0].end),
        (7, 17)
    );
    assert!(!outline.problems[0].message.is_empty());

    // Ranges are UTF-16 code units: the wave is a surrogate pair, so a text
    // view counts it twice where Rust counts four bytes.
    let outline = core.outline_draft("👋 {{cursor}}".into());
    assert_eq!(
        (outline.placeholders[0].start, outline.placeholders[0].end),
        (3, 13)
    );
}

#[test]
fn the_insert_menu_is_offered_the_placeholders_the_core_knows() {
    let choices = aralo_ffi::placeholder_choices();
    let names: Vec<&str> = choices.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"date"), "{names:?}");
    assert!(names.contains(&"clipboard"), "{names:?}");

    // Every choice inserts something the core then reads back as that same
    // placeholder, so the menu cannot offer a body the editor would underline.
    let (_folder, core) = open();
    for choice in &choices {
        let outline = core.outline_draft(choice.insert.clone());
        // A note is allowed: `{{selection}}` and the rest are known
        // placeholders Aralo does not expand yet, and the outline says so
        // rather than pretending. An error would mean the menu offered a body
        // the editor calls wrong. The sample for `{{snippet: …}}` is the one
        // that depends on the library, and it is checked below.
        if choice.name != "snippet" {
            let errors: Vec<&BodyProblem> = outline
                .problems
                .iter()
                .filter(|problem| problem.level == DiagnosticLevel::Error)
                .collect();
            assert!(errors.is_empty(), "{}: {errors:?}", choice.name);
        }
        assert_eq!(outline.placeholders.len(), 1, "{}", choice.name);
        assert_eq!(outline.placeholders[0].name, choice.name);
        let length = u32::try_from(choice.insert.encode_utf16().count()).unwrap();
        assert!(choice.select_start <= choice.select_end, "{}", choice.name);
        assert!(choice.select_end <= length, "{}", choice.name);
    }

    // A nested reference is read against the library, so the editor underlines
    // the name of a snippet that is not there and says nothing about one that
    // is.
    let nested = core.outline_draft("{{snippet: greeting}}".into());
    assert_eq!(nested.problems.len(), 1);
    assert_eq!(nested.problems[0].level, DiagnosticLevel::Error);
    assert!(
        nested.problems[0].message.contains("greeting"),
        "{}",
        nested.problems[0].message
    );
    core.create_snippet(vec![], draft("Greeting", "greeting", "Hello!"))
        .unwrap();
    assert_eq!(
        core.outline_draft("{{snippet: greeting}}".into()).problems,
        []
    );
    assert_eq!(core.preview_draft("{{snippet: greeting}}".into()), "Hello!");
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
        kind: None,
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

fn open() -> (aralo_testkit::TempDir, Arc<Core>) {
    let (folder, core, _) = listening();
    (folder, core)
}

/// The same, with a shell attached, and with the cache inside the temporary
/// folder so that a test run never touches the one on the machine.
fn listening() -> (aralo_testkit::TempDir, Arc<Core>, Shell) {
    let folder = aralo_testkit::tempdir().unwrap();
    let shell = Shell::default();
    let core = Core::open_library(
        folder.path().join("Aralo").to_string_lossy().into_owned(),
        Some(folder.path().join("cache").to_string_lossy().into_owned()),
        Some(Arc::new(shell.clone())),
        None,
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

/// A body that asks something first: the shell gets a session instead of a
/// plan, and nothing reaches the document until it has been driven.
#[test]
fn a_body_with_a_form_hands_the_shell_a_session_to_drive() {
    let (_folder, core) = open();
    let engine = core.engine();
    core.create_snippet(
        vec![],
        draft(
            "Ticket",
            ";tkt",
            concat!(
                "Hi {{field: who | label: Their name | default: friend}}, ",
                "about {{clipboard}} by {{choice: how | options: post, e-mail}}."
            ),
        ),
    )
    .unwrap();

    let actions = type_str(&engine, ";tkt ");
    let [KeyAction::StartSession {
        snippet_id,
        consume: true,
        session,
    }] = actions.as_slice()
    else {
        panic!("{actions:?}");
    };
    assert_eq!(session.snippet_id(), *snippet_id);

    // The form first, with the boxes as a panel draws them.
    let SessionAction::Form { fields } = session.next() else {
        panic!("the form comes first");
    };
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].name, "who");
    assert_eq!(fields[0].label, "Their name");
    assert_eq!(fields[0].default, "friend");
    assert_eq!(fields[0].options, [] as [String; 0]);
    assert_eq!(fields[1].options, ["post", "e-mail"]);
    // A drop-down stands on its first option, so the panel opens on it.
    assert_eq!(fields[1].default, "post");
    // Asking twice asks the same question.
    assert!(matches!(session.next(), SessionAction::Form { .. }));

    // A panel previews every keystroke, which changes nothing: the clipboard
    // has not been fetched yet, so it previews as itself.
    assert_eq!(
        session.preview_with([("who".to_owned(), "Da".to_owned())].into()),
        "Hi Da, about {{clipboard}} by post."
    );
    assert_eq!(session.preview(), "Hi friend, about {{clipboard}} by post.");

    // The context second, and only what the body asked for.
    let SessionAction::Context { kinds } = session.submit_form(
        [
            ("who".to_owned(), "Dana".to_owned()),
            ("how".to_owned(), "e-mail".to_owned()),
        ]
        .into(),
    ) else {
        panic!("the clipboard is asked for after the form");
    };
    assert_eq!(kinds, [ContextNeed::Clipboard]);

    // Then the plan, which is a typed expansion's plan: the abbreviation goes
    // first, and undo takes the whole thing back.
    let SessionAction::Expand {
        snippet_id: expanding,
        steps,
        undo_delete_count,
        profile: session_profile,
    } = session.provide_context(ContextSupply {
        clipboard: Some("PO-8841".into()),
        // Nothing asked for these, and the core drops them rather than
        // trusting a shell not to send them (PRD P4).
        selection: Some("secret".into()),
        app: Some("com.apple.TextEdit".into()),
        window: None,
    })
    else {
        panic!("the expansion comes last");
    };
    assert_eq!(expanding, *snippet_id);
    assert_eq!(
        steps,
        [
            PlanStep::Delete { count: 4 },
            PlanStep::InsertText {
                text: "Hi Dana, about PO-8841 by e-mail. ".into()
            },
        ]
    );
    assert_eq!(undo_delete_count, Some(34));

    // Reading it again reads the same plan rather than a second one: the plan
    // is a value, and the shell runs it once.
    assert_eq!(
        session.next(),
        SessionAction::Expand {
            snippet_id: snippet_id.clone(),
            steps,
            undo_delete_count,
            profile: session_profile,
        }
    );
}

/// Nothing was inserted, so cancelling has only one thing to put back: the key
/// the engine swallowed to open the panel.
#[test]
fn a_cancelled_session_puts_back_the_key_that_opened_it() {
    let (_folder, core) = open();
    let engine = core.engine();
    core.create_snippet(vec![], draft("Greeting", ";dear", "Dear {{field: who}},"))
        .unwrap();

    let actions = type_str(&engine, ";dear ");
    let [KeyAction::StartSession { session, .. }] = actions.as_slice() else {
        panic!("{actions:?}");
    };
    assert_eq!(
        session.cancel(),
        [PlanStep::InsertText { text: " ".into() }]
    );
    // Cancelled twice is still cancelled, and there is nothing left to ask.
    assert_eq!(session.cancel(), []);
    assert_eq!(session.next(), SessionAction::Done);
    assert_eq!(session.preview(), "");
}

/// The same from the palette: a picked snippet that asks something first.
#[test]
fn a_picked_snippet_with_a_form_starts_a_session_too() {
    let (_folder, core) = open();
    let engine = core.engine();
    let id = core
        .create_snippet(vec![], draft("Ticket", ";tkt", "Hi {{field: who}}"))
        .unwrap();
    engine.set_front_app("app.aralo.Aralo".into());

    let InsertOutcome::StartSession {
        snippet_id,
        session,
    } = engine.insert(id.clone(), "com.apple.Terminal".into())
    else {
        panic!("a form field cannot be answered on the keystroke");
    };
    assert_eq!(snippet_id, id);
    assert!(matches!(session.next(), SessionAction::Form { .. }));

    // Nothing was typed, so the plan has nothing to delete, and it is for the
    // app that was named rather than for Aralo's own window.
    let SessionAction::Expand { steps, profile, .. } =
        session.submit_form([("who".to_owned(), "Dana".to_owned())].into())
    else {
        panic!("one answer is all it wanted");
    };
    assert_eq!(
        steps,
        [PlanStep::InsertText {
            text: "Hi Dana".into()
        }]
    );
    // Terminal's row of the table, not Aralo's own window's.
    assert_eq!(profile.insert, InsertChoice::Type);
}

/// The palette's Enter: a snippet picked from a list, typed into the app the
/// user came from rather than the one Aralo's own window is.
#[test]
fn a_picked_snippet_inserts_its_body_into_the_app_that_was_named() {
    let (_folder, core) = open();
    let engine = core.engine();
    let id = core
        .create_snippet(vec![], draft("Sign off", "sig", "Best,\nAnand"))
        .unwrap();
    // The picker has the keyboard while the user chooses.
    engine.set_front_app("app.aralo.Aralo".into());

    let outcome = engine.insert(id.clone(), "com.apple.Terminal".into());
    let InsertOutcome::Insert {
        snippet_id,
        steps,
        undo_delete_count: Some(11),
        profile,
    } = outcome
    else {
        panic!("{outcome:?}");
    };
    assert_eq!(snippet_id, id);
    assert_eq!(
        steps,
        [PlanStep::InsertText {
            text: "Best,\nAnand".into()
        }],
        "nothing was typed, so there is nothing to delete"
    );
    // Terminal's row of the table, not Aralo's own window's.
    assert_eq!(profile.insert, InsertChoice::Type);
    assert_eq!(profile.undo, UndoStyle::Backspace);

    // The app the text went into is the front app now, so the system's own
    // report of the switch arrives as news of nothing and leaves undo armed.
    assert_eq!(engine.injection_profile(), profile);
    engine.set_front_app("com.apple.Terminal".into());

    // And the undo key takes it back, with nothing to retype.
    engine.expansion_done(snippet_id, 11, InsertMethod::Typed);
    assert_eq!(
        engine.on_key(KeyInput::Undo),
        KeyAction::UndoExpansion {
            delete_count: 11,
            retype: String::new(),
            method: InsertMethod::Typed,
            profile,
        }
    );
}

#[test]
fn a_picked_snippet_is_refused_when_it_is_gone_paused_or_for_an_app_aralo_stays_out_of() {
    let (_folder, core) = open();
    let engine = core.engine();
    let id = core
        .create_snippet(vec![], draft("Sign off", "sig", "Best"))
        .unwrap();
    let refusal = |outcome| match outcome {
        InsertOutcome::Refused { reason } => reason,
        other => panic!("{other:?}"),
    };

    let excluded = aralo_ffi::excluded_app_presets()[0].clone();
    assert_eq!(
        refusal(engine.insert(id.clone(), excluded)),
        InsertRefusal::ExcludedApp,
        "a deliberate pick is still not typed into a password manager"
    );

    engine.set_paused(true);
    assert_eq!(
        refusal(engine.insert(id.clone(), "com.apple.TextEdit".into())),
        InsertRefusal::Paused
    );
    // A refusal arms nothing, so the undo key stays the app's own.
    assert_eq!(engine.on_key(KeyInput::Undo), KeyAction::Pass);
    engine.set_paused(false);

    assert_eq!(
        refusal(engine.insert("not-an-id".into(), "com.apple.TextEdit".into())),
        InsertRefusal::SnippetGone
    );
    core.delete_snippet(id.clone()).unwrap();
    assert_eq!(
        refusal(engine.insert(id, "com.apple.TextEdit".into())),
        InsertRefusal::SnippetGone,
        "the list was drawn before the file went"
    );
}

#[test]
fn a_picked_snippet_cannot_complete_an_abbreviation_across_what_it_inserted() {
    let (_folder, core) = open();
    let engine = core.engine();
    let id = core
        .create_snippet(vec![], draft("Sign off", "sig", "Best"))
        .unwrap();

    // Half of the starter library's "ty" is in the document.
    assert_eq!(type_str(&engine, "t"), []);
    assert!(matches!(
        engine.insert(id, "com.apple.TextEdit".into()),
        InsertOutcome::Insert { .. }
    ));
    // The "y " lands after the inserted text, so "ty " is not what was typed.
    assert_eq!(type_str(&engine, "y "), []);
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

    let folder = aralo_testkit::tempdir().unwrap();
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
fn a_save_keeps_a_change_made_on_disk_and_hands_back_a_clash() {
    let (_folder, core) = open();
    let id = core
        .create_snippet(
            vec!["Work".into()],
            draft("Address", ";addr", "1 Long Road\nTown"),
        )
        .unwrap();
    let root = std::path::PathBuf::from(core.library_path());
    let detail = core.snippet(id.clone()).unwrap();
    let file = root.join(&detail.path);
    let opened = detail.draft.clone();

    // Another Mac renames it while this one's editor has it open.
    let text = std::fs::read_to_string(&file).unwrap();
    std::fs::write(&file, text.replace("label: Address", "label: Home address")).unwrap();
    let mut mine = opened.clone();
    mine.body = "1 Long Road\nCity".into();
    let SaveOutcome::Written { id: saved } = core
        .save_snippet_since(id.clone(), opened.clone(), mine.clone())
        .unwrap()
    else {
        panic!("a clean merge clashed");
    };
    assert_eq!(saved, id);
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("label: Home address"), "{text}");
    assert!(text.contains("City"), "{text}");

    // Then changes the line this one changes too.
    let opened = core.snippet(id.clone()).unwrap().draft;
    std::fs::write(&file, text.replace("City", "Village")).unwrap();
    let mut mine = opened.clone();
    mine.body = "1 Long Road\nHamlet".into();
    let SaveOutcome::Clashed { clash } = core
        .save_snippet_since(id.clone(), opened, mine.clone())
        .unwrap()
    else {
        panic!("a clash was written");
    };
    assert!(clash.body_clashes);
    assert!(clash.disk_text.contains("Village"));
    assert!(clash.mine_text.contains("Hamlet"));
    assert!(std::fs::read_to_string(&file).unwrap().contains("Village"));

    let disk = clash.disk.expect("the file reads");
    assert!(matches!(
        core.save_snippet_since(id, disk, mine).unwrap(),
        SaveOutcome::Written { .. }
    ));
    assert!(std::fs::read_to_string(&file).unwrap().contains("Hamlet"));
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

fn try_typing(trial: &aralo_ffi::DraftTrial, text: &str) -> Vec<TrialAction> {
    text.chars().map(|c| trial.key(key(c))).collect()
}

#[test]
fn the_test_field_expands_the_draft_on_screen_without_saving_it() {
    let (_folder, core) = open();
    let before = core.snippets().len();
    let trial = core.try_draft(
        draft("Sign off", ";sig", "Bye {{cursor}}!"),
        Vec::new(),
        None,
    );
    assert_eq!(trial.abbreviations(), 1);

    let actions = try_typing(&trial, "ok ;sig ");
    assert_eq!(actions.last(), Some(&TrialAction::Expanded));
    // "ok Bye |! " in UTF-16: the caret sits before the "!".
    assert_eq!(
        trial.field(),
        TrialField {
            text: "ok Bye ! ".into(),
            caret: 7,
            selected: 0,
        }
    );
    assert_eq!(core.snippets().len(), before, "trying a draft wrote it");

    // The caret moved, so there is no undo to offer: Backspace is Backspace,
    // as it would be in any app.
    trial.key(KeyInput::Backspace);
    assert_eq!(trial.field().text, "ok Bye! ");

    trial.clear();
    assert_eq!(trial.field().text, "");
}

#[test]
fn the_test_field_counts_in_utf16_like_a_text_view() {
    let (_folder, core) = open();
    let trial = core.try_draft(draft("Smile", ";sm", "\u{1F600}"), Vec::new(), None);
    try_typing(&trial, "\u{e9} ;sm ");
    let field = trial.field();
    assert_eq!(field.text, "\u{e9} \u{1F600} ");
    assert_eq!(field.caret, 5, "one unit, a space, two units, a space");

    // A click between the two halves of the emoji lands before it.
    trial.move_caret(3);
    trial.key(key('x'));
    assert_eq!(trial.field().text, "\u{e9} x\u{1F600} ");
}

#[test]
fn a_draft_with_a_form_hands_the_test_field_a_session() {
    let (_folder, core) = open();
    let body = "Hi {{field: who | default: friend}}.";
    let trial = core.try_draft(draft("Hello", ";hi", body), Vec::new(), None);

    let Some(TrialAction::Session { session }) = try_typing(&trial, ";hi ").pop() else {
        panic!("expected a session");
    };
    assert_eq!(trial.field().text, ";hi", "nothing goes in while it asks");
    assert!(matches!(session.next(), SessionAction::Form { .. }));
    let answers = [("who".to_owned(), "Dana".to_owned())]
        .into_iter()
        .collect();
    let SessionAction::Expand {
        steps,
        undo_delete_count,
        ..
    } = session.submit_form(answers)
    else {
        panic!("expected a plan");
    };
    trial.finish(steps, undo_delete_count);
    assert_eq!(trial.field().text, "Hi Dana. ");

    // And one the user closes puts the space back.
    trial.clear();
    let Some(TrialAction::Session { session }) = try_typing(&trial, ";hi ").pop() else {
        panic!("expected a session");
    };
    trial.cancel(session);
    assert_eq!(trial.field().text, ";hi ");
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
    // The report names the snippet each entry became, so it can open it.
    let id = signature.id.clone().unwrap();
    assert_eq!(core.snippet(id).unwrap().draft.label, "Signature");
    assert!(planned.entries.iter().all(|entry| entry.id.is_none()));
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

/// A shell's Trash: it moves what it is given into a folder of its own and
/// writes down what that was, or refuses when told to.
#[derive(Clone)]
struct Bin {
    folder: std::path::PathBuf,
    taken: Arc<Mutex<Vec<String>>>,
    refuse: bool,
}

impl Trash for Bin {
    fn discard(&self, path: String) -> Result<(), BridgeError> {
        if self.refuse {
            return Err(BridgeError::Trash {
                message: "the Trash is full".into(),
            });
        }
        let name = std::path::Path::new(&path).file_name().unwrap().to_owned();
        std::fs::create_dir_all(&self.folder).unwrap();
        std::fs::rename(&path, self.folder.join(name)).unwrap();
        self.taken.lock().unwrap().push(path);
        Ok(())
    }
}

const SIG_ID: &str = "01J8ZK3V5Q8W6T9X2N4R7M0ABC";

/// A library where both machines renamed one snippet, opened with `bin` as its
/// Trash. With no base yet, nothing about the two can merge.
fn clashing(bin: &Bin) -> (aralo_testkit::TempDir, Arc<Core>) {
    let folder = aralo_testkit::tempdir().unwrap();
    let library = folder.path().join("Aralo");
    std::fs::create_dir_all(&library).unwrap();
    let sig =
        |label: &str| format!("---\nid: {SIG_ID}\nlabel: {label}\nabbr: [;sig]\n---\nBest,\nSam\n");
    std::fs::write(library.join("sig.md"), sig("Mine")).unwrap();
    std::fs::write(library.join("sig 2.md"), sig("Theirs")).unwrap();
    let core = Core::open_library(
        library.to_string_lossy().into_owned(),
        Some(folder.path().join("cache").to_string_lossy().into_owned()),
        None,
        Some(Arc::new(bin.clone())),
    )
    .unwrap();
    (folder, core)
}

#[test]
fn a_conflict_is_shown_and_resolved_into_the_shells_trash() {
    let trash = aralo_testkit::tempdir().unwrap();
    let bin = Bin {
        folder: trash.path().to_owned(),
        taken: Arc::default(),
        refuse: false,
    };
    let (_folder, core) = clashing(&bin);

    let waiting = core.conflicts();
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].copy, "sig 2.md");
    assert_eq!(waiting[0].original, "sig.md");
    assert_eq!(waiting[0].snippet_id, SIG_ID);

    let detail = core.conflict("sig 2.md".into()).unwrap();
    assert!(detail.original_text.contains("label: Mine"));
    assert!(detail.copy_text.contains("label: Theirs"));
    assert_eq!(detail.base_text, None);
    assert_eq!(detail.clashing_keys, ["label"]);
    assert!(!detail.body_clashes);
    assert!(core.conflict("sig.md".into()).is_err());

    core.resolve_conflict("sig 2.md".into(), ConflictChoice::KeepCopy)
        .unwrap();
    assert!(core.conflicts().is_empty());
    let taken = bin.taken.lock().unwrap().clone();
    assert_eq!(taken.len(), 1);
    assert!(taken[0].ends_with("sig 2.md"));
    assert!(trash.path().join("sig 2.md").exists());
    let kept = core.snippet(SIG_ID.into()).unwrap();
    assert_eq!(kept.draft.label, "Theirs");
}

#[test]
fn a_trash_that_refuses_leaves_the_conflict_waiting() {
    let trash = aralo_testkit::tempdir().unwrap();
    let bin = Bin {
        folder: trash.path().to_owned(),
        taken: Arc::default(),
        refuse: true,
    };
    let (folder, core) = clashing(&bin);

    let refused = core.resolve_conflict("sig 2.md".into(), ConflictChoice::KeepOriginal);
    assert!(refused
        .unwrap_err()
        .to_string()
        .contains("the Trash is full"));
    assert_eq!(core.conflicts().len(), 1);
    assert!(folder.path().join("Aralo/sig 2.md").exists());

    // A version that is not a snippet file is refused before anything moves.
    let unreadable = core.resolve_conflict(
        "sig 2.md".into(),
        ConflictChoice::Write {
            text: "---\nabbr: [\n---\n".into(),
        },
    );
    assert!(unreadable.is_err());
    assert_eq!(core.conflicts().len(), 1);
}

#[test]
fn the_first_run_can_tell_the_starter_snippets_from_the_users_own() {
    let (folder, core) = open();
    assert!(
        core.starter_files_written() > 0,
        "a new folder gets the starter set"
    );
    drop(core);

    // The same folder again: everything in it is the user's now.
    let again = Core::open_library(
        folder.path().join("Aralo").to_string_lossy().into_owned(),
        Some(folder.path().join("cache").to_string_lossy().into_owned()),
        None,
        None,
    )
    .unwrap();
    assert_eq!(again.starter_files_written(), 0);
    assert!(!again.snippets().is_empty());
}

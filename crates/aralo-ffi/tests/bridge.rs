//! The bridge as Swift will use it, driven from Rust.

use aralo_ffi::{Core, InsertMethod, KeyAction, KeyInput, PlanKey, PlanStep, ResetReason};

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

fn open() -> (tempfile::TempDir, std::sync::Arc<Core>) {
    let folder = tempfile::tempdir().unwrap();
    let core =
        Core::open_library(folder.path().join("Aralo").to_string_lossy().into_owned()).unwrap();
    core.engine().set_front_app("com.apple.TextEdit".into());
    (folder, core)
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
        }
    );
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

#[test]
fn reload_reaches_the_engine() {
    let (_folder, core) = open();
    let root = std::path::PathBuf::from(core.library_path());
    std::fs::write(root.join("hello.md"), "---\nabbr: ;hi\n---\nhello there").unwrap();
    assert!(type_str(&core.engine(), ";hi ").is_empty());
    core.reload().unwrap();
    assert_eq!(type_str(&core.engine(), ";hi ").len(), 1);
    assert!(core.snippets().iter().any(|s| s.abbreviations == [";hi"]));

    // The folder disappears: reload fails, the loaded library keeps working.
    std::fs::remove_dir_all(&root).unwrap();
    assert!(core.reload().is_err());
    assert_eq!(type_str(&core.engine(), ";hi ").len(), 1);
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

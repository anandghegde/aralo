//! PRD P1: typed characters stay in memory and are zeroed on every reset.
//!
//! These tests block merge. If one fails, a privacy promise is broken.

use std::sync::Arc;

use aralo_engine::{
    Abbreviation, Engine, ExpansionRecord, InsertMethod, KeyEvent, KeyVerdict, ResetReason,
    SnapshotBuilder, SnippetId, CAPACITY,
};

fn engine() -> Engine {
    let mut builder = SnapshotBuilder::new();
    builder.add(Abbreviation::new(SnippetId(1), ";rf"));
    let mut engine = Engine::new();
    engine.set_snapshot(Arc::new(builder.build().0));
    engine.set_front_app("com.apple.TextEdit");
    engine
}

fn type_str(engine: &mut Engine, text: &str) {
    for c in text.chars() {
        engine.on_key(KeyEvent::Char(c));
    }
}

#[test]
fn the_buffer_is_zero_after_every_reset_reason() {
    for reason in ResetReason::ALL {
        let mut engine = engine();
        // Wrap the ring so every slot has held a character.
        type_str(&mut engine, &"secret ".repeat(CAPACITY));
        assert!(!engine.holds_no_keystrokes());
        engine.reset(reason);
        assert!(engine.holds_no_keystrokes(), "not zeroed after {reason:?}");
    }
}

#[test]
fn the_undo_record_is_dropped_by_a_reset_too() {
    let mut engine = engine();
    type_str(&mut engine, ";rf ");
    engine.expansion_done(ExpansionRecord {
        snippet_id: SnippetId(1),
        delete_count: 5,
        method: InsertMethod::Typed,
    });
    assert!(
        !engine.holds_no_keystrokes(),
        "the typed abbreviation is held for undo"
    );
    engine.reset(ResetReason::FocusChange);
    assert!(engine.holds_no_keystrokes());
}

#[test]
fn every_state_change_that_invalidates_the_buffer_zeroes_it() {
    type Change = (&'static str, fn(&mut Engine));
    let changes: [Change; 4] = [
        ("app switch", |e| e.set_front_app("com.apple.mail")),
        ("pause", |e| e.set_paused(true)),
        ("exclusion change", |e| e.set_excluded_apps(vec![])),
        ("snapshot swap", |e| {
            e.set_snapshot(Arc::new(SnapshotBuilder::new().build().0));
        }),
    ];
    for (name, change) in changes {
        let mut engine = engine();
        type_str(&mut engine, "some private text");
        change(&mut engine);
        assert!(engine.holds_no_keystrokes(), "not zeroed after {name}");
    }
}

#[test]
fn nothing_is_recorded_while_paused_or_in_an_excluded_app() {
    let mut engine = engine();
    engine.set_paused(true);
    type_str(&mut engine, "typed while paused");
    assert!(engine.holds_no_keystrokes());

    let mut engine = self::engine();
    engine.set_front_app("com.apple.keychainaccess");
    type_str(&mut engine, "typed in a password manager");
    assert!(engine.holds_no_keystrokes());
}

#[test]
fn a_match_empties_the_buffer() {
    let mut engine = engine();
    type_str(&mut engine, "context before ;rf");
    assert!(matches!(
        engine.on_key(KeyEvent::Char(' ')),
        KeyVerdict::Match { .. }
    ));
    // Only the typed abbreviation survives, for undo, until the next key.
    engine.on_key(KeyEvent::Char('x'));
    engine.on_key(KeyEvent::Backspace);
    assert!(engine.holds_no_keystrokes());
}

#[test]
fn debug_output_carries_no_typed_text() {
    let mut engine = engine();
    type_str(&mut engine, "hunter2");
    let printed = format!("{engine:?}");
    assert!(!printed.contains("hunter2"), "{printed}");
}

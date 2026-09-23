//! Behaviour of `Engine::on_key`, rule by rule (PRD E1, E2, E3, E6, E8, E10).

use std::sync::Arc;

use aralo_engine::{
    Abbreviation, CaseMode, CasePattern, Engine, ExpansionRecord, InsertMethod, InsertRefusal,
    KeyEvent, KeyVerdict, ResetReason, Scope, SnapshotBuilder, SnippetId, Trigger,
};

fn engine_with(abbreviations: Vec<Abbreviation>) -> Engine {
    let mut builder = SnapshotBuilder::new();
    for abbreviation in abbreviations {
        builder.add(abbreviation);
    }
    let (snapshot, rejections) = builder.build();
    assert!(rejections.is_empty(), "{rejections:?}");
    let mut engine = Engine::new();
    engine.set_snapshot(Arc::new(snapshot));
    engine.set_front_app("com.apple.TextEdit");
    engine
}

fn immediate(id: u128, text: &str) -> Abbreviation {
    Abbreviation {
        trigger: Trigger::Immediate,
        ..Abbreviation::new(SnippetId(id), text)
    }
}

/// Types `text` and returns the verdict of every key.
fn type_str(engine: &mut Engine, text: &str) -> Vec<KeyVerdict> {
    text.chars()
        .map(|c| engine.on_key(KeyEvent::Char(c)))
        .collect()
}

/// Types `text` and returns the verdict of the last key, asserting the rest passed.
fn last_verdict(engine: &mut Engine, text: &str) -> KeyVerdict {
    let mut verdicts = type_str(engine, text);
    let last = verdicts.pop().expect("text is not empty");
    assert!(
        verdicts.iter().all(|v| *v == KeyVerdict::Pass),
        "an earlier key matched: {verdicts:?}"
    );
    last
}

fn matched(id: u128, delete_count: u32, case: CasePattern, trailing: Option<char>) -> KeyVerdict {
    KeyVerdict::Match {
        snippet_id: SnippetId(id),
        delete_count,
        consume: true,
        case,
        trailing,
    }
}

#[test]
fn delimiter_trigger_fires_on_the_delimiter_and_hands_it_back() {
    let mut engine = engine_with(vec![Abbreviation::new(SnippetId(1), ";rf")]);
    assert_eq!(
        last_verdict(&mut engine, ";rf "),
        matched(1, 3, CasePattern::AsDefined, Some(' '))
    );
}

#[test]
fn delimiter_is_dropped_when_the_snippet_says_so() {
    let mut engine = engine_with(vec![Abbreviation {
        keep_delimiter: false,
        ..Abbreviation::new(SnippetId(1), ";rf")
    }]);
    assert_eq!(
        last_verdict(&mut engine, ";rf."),
        matched(1, 3, CasePattern::AsDefined, None)
    );
}

#[test]
fn only_the_groups_own_delimiters_trigger() {
    let mut engine = engine_with(vec![Abbreviation {
        delimiters: vec![' '],
        ..Abbreviation::new(SnippetId(1), "addr")
    }]);
    assert_eq!(last_verdict(&mut engine, "addr."), KeyVerdict::Pass);
    engine.reset(ResetReason::Manual);
    assert!(matches!(
        last_verdict(&mut engine, "addr "),
        KeyVerdict::Match { .. }
    ));
}

#[test]
fn immediate_trigger_fires_on_the_last_character_which_never_reaches_the_app() {
    let mut engine = engine_with(vec![immediate(1, ";sig")]);
    assert_eq!(
        last_verdict(&mut engine, ";sig"),
        matched(1, 3, CasePattern::AsDefined, None)
    );
}

#[test]
fn whole_word_needs_a_non_word_character_or_the_buffer_start_before_it() {
    let mut engine = engine_with(vec![immediate(1, "ty")]);
    assert_eq!(last_verdict(&mut engine, "party"), KeyVerdict::Pass);
    engine.reset(ResetReason::Manual);
    assert!(matches!(
        last_verdict(&mut engine, "ty"),
        KeyVerdict::Match { .. }
    ));
    assert!(matches!(
        last_verdict(&mut engine, "so (ty"),
        KeyVerdict::Match { .. }
    ));
}

#[test]
fn whole_word_off_matches_inside_a_word() {
    let mut engine = engine_with(vec![Abbreviation {
        whole_word: false,
        ..immediate(1, "ty")
    }]);
    assert!(matches!(
        last_verdict(&mut engine, "party"),
        KeyVerdict::Match { .. }
    ));
}

#[test]
fn ty_goldens_for_adaptive_case() {
    for (typed, expected) in [
        ("ty ", CasePattern::AsDefined),
        ("Ty ", CasePattern::Title),
        ("TY ", CasePattern::Upper),
    ] {
        let mut engine = engine_with(vec![Abbreviation::new(SnippetId(1), "ty")]);
        assert_eq!(
            last_verdict(&mut engine, typed),
            matched(1, 2, expected, Some(' ')),
            "typed {typed:?}"
        );
    }
}

#[test]
fn exact_case_rejects_other_casings_and_ignore_case_does_not_transform() {
    let mut engine = engine_with(vec![
        Abbreviation {
            case: CaseMode::Exact,
            ..Abbreviation::new(SnippetId(1), "Sig")
        },
        Abbreviation {
            case: CaseMode::Ignore,
            ..Abbreviation::new(SnippetId(2), "addr")
        },
    ]);
    assert_eq!(last_verdict(&mut engine, "sig "), KeyVerdict::Pass);
    engine.reset(ResetReason::Manual);
    assert_eq!(
        last_verdict(&mut engine, "Sig "),
        matched(1, 3, CasePattern::AsDefined, Some(' '))
    );
    assert_eq!(
        last_verdict(&mut engine, "ADDR "),
        matched(2, 4, CasePattern::AsDefined, Some(' '))
    );
}

#[test]
fn longest_abbreviation_wins() {
    let mut engine = engine_with(vec![
        Abbreviation {
            whole_word: false,
            ..Abbreviation::new(SnippetId(1), "rf")
        },
        Abbreviation::new(SnippetId(2), ";rf"),
    ]);
    assert_eq!(
        last_verdict(&mut engine, ";rf "),
        matched(2, 3, CasePattern::AsDefined, Some(' '))
    );
}

#[test]
fn a_longer_immediate_match_beats_a_shorter_delimiter_match_on_the_same_key() {
    let mut engine = engine_with(vec![
        Abbreviation {
            delimiters: vec!['.'],
            ..Abbreviation::new(SnippetId(1), "a")
        },
        immediate(2, "a."),
    ]);
    assert_eq!(
        last_verdict(&mut engine, "a."),
        matched(2, 1, CasePattern::AsDefined, None)
    );
}

#[test]
fn the_most_specific_scope_wins_between_equal_abbreviations() {
    let everywhere = Abbreviation::new(SnippetId(1), ";sig");
    let mail_only = Abbreviation {
        scope: Scope::Only(vec!["com.apple.mail".into()]),
        ..Abbreviation::new(SnippetId(2), ";sig")
    };
    let mut engine = engine_with(vec![everywhere, mail_only]);

    engine.set_front_app("com.apple.mail");
    assert!(matches!(
        last_verdict(&mut engine, ";sig "),
        KeyVerdict::Match {
            snippet_id: SnippetId(2),
            ..
        }
    ));
    engine.set_front_app("com.apple.Notes");
    assert!(matches!(
        last_verdict(&mut engine, ";sig "),
        KeyVerdict::Match {
            snippet_id: SnippetId(1),
            ..
        }
    ));
}

#[test]
fn equal_candidates_resolve_to_the_lowest_id_whatever_the_build_order() {
    for order in [[7, 3], [3, 7]] {
        let mut engine = engine_with(
            order
                .iter()
                .map(|&id| Abbreviation::new(SnippetId(id), ";x"))
                .collect(),
        );
        assert!(matches!(
            last_verdict(&mut engine, ";x "),
            KeyVerdict::Match {
                snippet_id: SnippetId(3),
                ..
            }
        ));
    }
}

#[test]
fn scope_except_disables_an_abbreviation_in_the_listed_app() {
    let mut engine = engine_with(vec![Abbreviation {
        scope: Scope::Except(vec!["com.apple.Terminal".into()]),
        ..Abbreviation::new(SnippetId(1), ";x")
    }]);
    engine.set_front_app("com.apple.Terminal");
    assert_eq!(last_verdict(&mut engine, ";x "), KeyVerdict::Pass);
    engine.set_front_app("com.apple.mail");
    assert!(matches!(
        last_verdict(&mut engine, ";x "),
        KeyVerdict::Match { .. }
    ));
}

#[test]
fn backspace_edits_the_buffer_like_the_document() {
    let mut engine = engine_with(vec![Abbreviation::new(SnippetId(1), ";rf")]);
    type_str(&mut engine, ";rg");
    assert_eq!(engine.on_key(KeyEvent::Backspace), KeyVerdict::Pass);
    assert!(matches!(
        last_verdict(&mut engine, "f "),
        KeyVerdict::Match { .. }
    ));
}

#[test]
fn a_reset_in_the_middle_of_an_abbreviation_prevents_the_match() {
    let mut engine = engine_with(vec![Abbreviation {
        whole_word: false,
        ..Abbreviation::new(SnippetId(1), ";rf")
    }]);
    type_str(&mut engine, ";r");
    engine.reset(ResetReason::MouseDown);
    assert_eq!(last_verdict(&mut engine, "f "), KeyVerdict::Pass);
}

#[test]
fn switching_app_resets_the_buffer() {
    let mut engine = engine_with(vec![Abbreviation {
        whole_word: false,
        ..Abbreviation::new(SnippetId(1), ";rf")
    }]);
    type_str(&mut engine, ";r");
    engine.set_front_app("com.apple.mail");
    assert_eq!(last_verdict(&mut engine, "f "), KeyVerdict::Pass);
}

#[test]
fn paused_engine_neither_matches_nor_records() {
    let mut engine = engine_with(vec![Abbreviation::new(SnippetId(1), ";rf")]);
    engine.set_paused(true);
    assert_eq!(last_verdict(&mut engine, ";rf "), KeyVerdict::Pass);
    assert!(engine.holds_no_keystrokes());
    engine.set_paused(false);
    assert!(matches!(
        last_verdict(&mut engine, ";rf "),
        KeyVerdict::Match { .. }
    ));
}

#[test]
fn preset_password_managers_are_excluded_until_the_list_is_replaced() {
    let mut engine = engine_with(vec![Abbreviation::new(SnippetId(1), ";rf")]);
    engine.set_front_app("com.1password.1password");
    assert_eq!(last_verdict(&mut engine, ";rf "), KeyVerdict::Pass);
    assert!(engine.holds_no_keystrokes());

    engine.set_excluded_apps(vec!["com.example.Bank".into()]);
    assert!(matches!(
        last_verdict(&mut engine, ";rf "),
        KeyVerdict::Match { .. }
    ));
    engine.set_front_app("com.example.bank");
    assert_eq!(last_verdict(&mut engine, ";rf "), KeyVerdict::Pass);
}

fn expand(engine: &mut Engine, typed: &str, method: InsertMethod) {
    let KeyVerdict::Match { snippet_id, .. } = last_verdict(engine, typed) else {
        panic!("{typed:?} did not match");
    };
    engine.expansion_done(ExpansionRecord {
        snippet_id,
        delete_count: 9,
        method,
    });
}

#[test]
fn backspace_right_after_an_expansion_undoes_it_once() {
    let mut engine = engine_with(vec![Abbreviation::new(SnippetId(1), ";rf")]);
    expand(&mut engine, ";Rf ", InsertMethod::Typed);
    assert_eq!(
        engine.on_key(KeyEvent::Backspace),
        KeyVerdict::UndoLast {
            delete_count: 9,
            retype: ";Rf ".into(),
            method: InsertMethod::Typed,
        }
    );
    assert_eq!(engine.on_key(KeyEvent::Backspace), KeyVerdict::Pass);
}

#[test]
fn cmd_z_right_after_a_pasted_expansion_reports_the_method() {
    let mut engine = engine_with(vec![immediate(1, ";sig")]);
    expand(&mut engine, ";sig", InsertMethod::Pasted);
    assert_eq!(
        engine.on_key(KeyEvent::Undo),
        KeyVerdict::UndoLast {
            delete_count: 9,
            retype: ";sig".into(),
            method: InsertMethod::Pasted,
        }
    );
}

#[test]
fn undo_is_not_offered_after_another_key_a_reset_or_an_unfinished_expansion() {
    let mut engine = engine_with(vec![Abbreviation::new(SnippetId(1), ";rf")]);

    expand(&mut engine, ";rf ", InsertMethod::Typed);
    engine.on_key(KeyEvent::Char('x'));
    assert_eq!(engine.on_key(KeyEvent::Backspace), KeyVerdict::Pass);

    expand(&mut engine, ";rf ", InsertMethod::Typed);
    engine.reset(ResetReason::MouseDown);
    assert_eq!(engine.on_key(KeyEvent::Backspace), KeyVerdict::Pass);

    // Matched, but the shell has not reported the insertion yet.
    assert!(matches!(
        last_verdict(&mut engine, ";rf "),
        KeyVerdict::Match { .. }
    ));
    assert_eq!(engine.on_key(KeyEvent::Backspace), KeyVerdict::Pass);
}

#[test]
fn a_picked_snippet_is_undone_like_one_that_was_typed_and_retypes_nothing() {
    let mut engine = engine_with(vec![Abbreviation::new(SnippetId(1), ";rf")]);
    // Half an abbreviation is in the document when the picker opens.
    type_str(&mut engine, ";r");

    engine
        .chosen(SnippetId(1), "com.apple.TextEdit")
        .expect("TextEdit is not excluded and the engine is running");
    engine.expansion_done(ExpansionRecord {
        snippet_id: SnippetId(1),
        delete_count: 9,
        method: InsertMethod::Pasted,
    });
    assert_eq!(
        engine.on_key(KeyEvent::Undo),
        KeyVerdict::UndoLast {
            delete_count: 9,
            // Nothing was typed to trigger it, so nothing comes back.
            retype: String::new(),
            method: InsertMethod::Pasted,
        }
    );
    // The ";r" that preceded the insertion cannot complete an abbreviation
    // across it: the picked text is what the caret sits after now.
    assert_eq!(last_verdict(&mut engine, "f "), KeyVerdict::Pass);
}

#[test]
fn a_picked_snippet_is_refused_while_paused_or_in_an_app_aralo_stays_out_of() {
    let mut engine = engine_with(vec![Abbreviation::new(SnippetId(1), ";rf")]);

    assert_eq!(
        engine.chosen(SnippetId(1), "com.1password.1password"),
        Err(InsertRefusal::ExcludedApp),
        "a deliberate pick is still not typed into a password manager"
    );

    engine.set_paused(true);
    assert_eq!(
        engine.chosen(SnippetId(1), "com.apple.TextEdit"),
        Err(InsertRefusal::Paused)
    );
    // A refusal arms nothing: the undo key is the app's own again.
    assert!(engine.holds_no_keystrokes());
    assert_eq!(engine.on_key(KeyEvent::Undo), KeyVerdict::Pass);

    engine.set_paused(false);
    assert_eq!(engine.chosen(SnippetId(1), "com.apple.TextEdit"), Ok(()));
}

#[test]
fn the_app_a_pick_goes_into_is_the_one_named_not_the_one_in_front() {
    let mut engine = engine_with(vec![Abbreviation::new(SnippetId(1), ";rf")]);
    // The picker has the keyboard, so Aralo itself is the front app. What the
    // exclusion list is checked against is the app the text goes into.
    engine.set_front_app("app.aralo.Aralo");
    assert_eq!(engine.chosen(SnippetId(1), "com.apple.TextEdit"), Ok(()));
    assert_eq!(
        engine.chosen(SnippetId(1), "org.keepassxc.keepassxc"),
        Err(InsertRefusal::ExcludedApp)
    );
}

#[test]
fn a_late_expansion_report_cannot_arm_undo_for_a_different_snippet() {
    let mut engine = engine_with(vec![
        Abbreviation::new(SnippetId(1), ";a"),
        Abbreviation::new(SnippetId(2), ";b"),
    ]);
    assert!(matches!(
        last_verdict(&mut engine, ";b "),
        KeyVerdict::Match { .. }
    ));
    engine.expansion_done(ExpansionRecord {
        snippet_id: SnippetId(1),
        delete_count: 4,
        method: InsertMethod::Typed,
    });
    assert_eq!(engine.on_key(KeyEvent::Backspace), KeyVerdict::Pass);
}

#[test]
fn a_new_snapshot_takes_effect_on_the_next_key() {
    let mut engine = engine_with(vec![Abbreviation::new(SnippetId(1), ";old")]);
    let mut builder = SnapshotBuilder::new();
    builder.add(Abbreviation::new(SnippetId(2), ";new"));
    let old = engine.set_snapshot(Arc::new(builder.build().0));
    assert_eq!(old.len(), 1);
    assert_eq!(last_verdict(&mut engine, ";old "), KeyVerdict::Pass);
    assert!(matches!(
        last_verdict(&mut engine, ";new "),
        KeyVerdict::Match {
            snippet_id: SnippetId(2),
            ..
        }
    ));
}

#[test]
fn non_ascii_abbreviations_and_folding_work() {
    let mut engine = engine_with(vec![Abbreviation::new(SnippetId(1), "grün")]);
    assert_eq!(
        last_verdict(&mut engine, "GRÜN "),
        matched(1, 4, CasePattern::Upper, Some(' '))
    );
}

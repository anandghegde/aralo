//! The editor's test field: a draft tried before it is saved.
//!
//! Each test types into a [`Trial`] and asserts on the field, because the
//! field is what the user sees. The library is checked afterwards where it
//! matters: trying a draft must never write it.

use std::sync::Arc;

use aralo_core::engine::KeyEvent;
use aralo_core::snippet::TriggerMode;
use aralo_core::{CivilTime, Core, Draft, FixedClock, Trial, TrialKey};

fn library() -> (tempfile::TempDir, Core) {
    let folder = tempfile::tempdir().unwrap();
    let core = Core::open_without_starter(&folder.path().join("Aralo"))
        .unwrap()
        .with_clock(Arc::new(FixedClock(CivilTime::new(2026, 3, 9, 14, 5, 7))))
        .in_locale("en_GB");
    (folder, core)
}

fn draft(abbr: &str, body: &str) -> Draft {
    Draft {
        label: "Draft".to_owned(),
        abbr: vec![abbr.to_owned()],
        body: body.to_owned(),
        ..Draft::default()
    }
}

/// Types `text`, failing if the draft asks anything.
fn type_str(trial: &mut Trial, core: &Core, text: &str) {
    if trial.type_str(core, text).is_some() {
        panic!("{text:?} opened a session");
    }
}

#[test]
fn a_draft_expands_before_it_is_saved() {
    let (_folder, core) = library();
    let mut trial = core.try_draft(&draft(";br", "Best regards,"), &[], None);
    type_str(&mut trial, &core, "so ;br ");

    assert_eq!(trial.marked(), "so Best regards, |");
    assert!(core.snippets().is_empty(), "trying a draft wrote it");
}

#[test]
fn the_draft_on_screen_wins_over_the_file() {
    let (_folder, mut core) = library();
    let id = core.create_snippet(&[], &draft(";br", "Old")).unwrap();
    let mut trial = core.try_draft(&draft(";regards", "New"), &[], Some(id));

    type_str(&mut trial, &core, ";br ;regards ");
    assert_eq!(trial.text(), ";br New ");
}

#[test]
fn it_follows_the_groups_settings_under_the_drafts_own() {
    let (_folder, mut core) = library();
    let work = vec!["Work".to_owned()];
    core.create_group(&work).unwrap();
    core.edit_group(&work, |file| {
        file.defaults.trigger = Some(TriggerMode::Immediate);
    })
    .unwrap();

    // The group says "as soon as it is typed", so no delimiter is needed.
    let mut trial = core.try_draft(&draft("zz", "sleep"), &work, None);
    type_str(&mut trial, &core, "zz");
    assert_eq!(trial.text(), "sleep");

    // The draft says otherwise, and the draft is what would be saved.
    let mut own = draft("zz", "sleep");
    own.trigger = Some(TriggerMode::Delimiter);
    let mut trial = core.try_draft(&own, &work, None);
    type_str(&mut trial, &core, "zz");
    assert_eq!(trial.text(), "zz");
    type_str(&mut trial, &core, " ");
    assert_eq!(trial.text(), "sleep ");
}

#[test]
fn a_draft_that_is_switched_off_can_still_be_tried() {
    let (_folder, core) = library();
    let mut off = draft(";br", "Best");
    off.enabled = Some(false);
    let mut trial = core.try_draft(&off, &[], None);
    type_str(&mut trial, &core, ";br ");
    assert_eq!(trial.text(), "Best ");
}

#[test]
fn backspace_straight_after_takes_the_expansion_back() {
    let (_folder, core) = library();
    let mut trial = core.try_draft(&draft(";br", "Best regards,"), &[], None);
    type_str(&mut trial, &core, ";br ");
    assert!(matches!(
        trial.key(&core, KeyEvent::Backspace),
        TrialKey::Expanded
    ));
    assert_eq!(trial.text(), ";br ");
}

#[test]
fn a_cursor_stop_puts_the_caret_where_the_body_says() {
    let (_folder, core) = library();
    let mut trial = core.try_draft(&draft(";dear", "Dear {{cursor}},"), &[], None);
    type_str(&mut trial, &core, ";dear ");
    assert_eq!(trial.marked(), "Dear |, ");
    type_str(&mut trial, &core, "Dana");
    assert_eq!(trial.text(), "Dear Dana, ");
}

#[test]
fn typing_over_a_selection_replaces_it() {
    let (_folder, core) = library();
    let body = "Hi {{cursor: select}}there{{cursor: select}}!";
    let mut trial = core.try_draft(&draft(";hi", body), &[], None);
    type_str(&mut trial, &core, ";hi ");
    assert_eq!(trial.marked(), "Hi [there]! ");
    type_str(&mut trial, &core, "Dana");
    assert_eq!(trial.marked(), "Hi Dana|! ");
}

#[test]
fn a_form_is_a_session_and_nothing_goes_in_until_it_ends() {
    let (_folder, core) = library();
    let body = "Hi {{field: who | default: friend}}.";
    let mut trial = core.try_draft(&draft(";hi", body), &[], None);

    let mut session = trial.type_str(&core, "well ;hi ").expect("a form");
    assert_eq!(trial.text(), "well ;hi", "the space is the session's");
    assert_eq!(session.snippet_id(), trial.snippet_id());

    let answers = [("who".to_owned(), "Dana".to_owned())]
        .into_iter()
        .collect();
    match session.submit_form(answers) {
        aralo_core::SessionStep::Ready(expansion) => trial.finish(&expansion),
        other => panic!("expected a plan, got {other:?}"),
    }
    assert_eq!(trial.text(), "well Hi Dana. ");
}

#[test]
fn a_cancelled_form_puts_the_key_back() {
    let (_folder, core) = library();
    let body = "Hi {{field: who}}.";
    let mut trial = core.try_draft(&draft(";hi", body), &[], None);
    let session = trial.type_str(&core, ";hi ").expect("a form");
    trial.put_back(&session.cancel());
    assert_eq!(trial.marked(), ";hi |");
}

#[test]
fn moving_the_caret_forgets_what_was_typed_before_it() {
    let (_folder, core) = library();
    let mut trial = core.try_draft(&draft(";br", "Best"), &[], None);
    type_str(&mut trial, &core, "x ;b");
    trial.move_caret(0);
    trial.move_caret(trial.text().len());
    // ";b" and "r " were typed on either side of a click; they are not one
    // abbreviation.
    type_str(&mut trial, &core, "r ");
    assert_eq!(trial.text(), "x ;br ");
}

#[test]
fn a_draft_with_no_abbreviation_says_so() {
    let (_folder, core) = library();
    let mut empty = draft("", "Best");
    empty.abbr.clear();
    assert_eq!(core.try_draft(&empty, &[], None).abbreviations(), 0);
    // The one the editor adds for the user to fill in, before they have.
    assert_eq!(
        core.try_draft(&draft("", "Best"), &[], None)
            .abbreviations(),
        0
    );
}

#[test]
fn a_nested_snippet_is_read_from_the_library() {
    let (_folder, mut core) = library();
    core.create_snippet(&[], &draft(";sig", "Sam")).unwrap();
    let mut trial = core.try_draft(&draft(";bye", "Bye, {{snippet: ;sig}}"), &[], None);
    type_str(&mut trial, &core, ";bye ");
    assert_eq!(trial.text(), "Bye, Sam ");
}

#[test]
fn return_is_a_line_break_whether_or_not_it_ends_an_abbreviation() {
    let (_folder, core) = library();
    let mut trial = core.try_draft(&draft(";br", "Best"), &[], None);
    type_str(&mut trial, &core, "x\r;br\r");
    assert_eq!(trial.text(), "x\nBest\n");
}

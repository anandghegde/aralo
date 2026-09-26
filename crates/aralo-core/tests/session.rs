//! Expansions that cannot finish on the keystroke: a form to fill in, a
//! clipboard to fetch, snippets that nest.
//!
//! The tests drive a session the way a shell does — ask what is needed, answer
//! it, run what comes out — because the order the questions come in is the
//! part that has to be right. Where the caret ends up is checked against a
//! simulated text field, since that is the only place a cursor stop can be
//! seen at all.

use std::sync::Arc;

use aralo_core::clock::FixedClock;
use aralo_core::snippet::SnippetId;
use aralo_core::template::{DiagnosticKind, MAX_SNIPPET_DEPTH};
use aralo_core::{
    Answers, CivilTime, ContextKind, ContextValues, Core, Draft, Expansion, FieldKind, Session,
    SessionStep, Simulator,
};

const APP: &str = "com.apple.TextEdit";

/// 9 March 2026 at 14:05:07, an hour east of UTC. A Monday, so the weekday
/// names in the golden files are not all the same word.
fn moment() -> CivilTime {
    CivilTime::new(2026, 3, 9, 14, 5, 7).with_offset(60)
}

/// A library holding `snippets` as (label, abbreviation, body), with the clock
/// stopped so a `{{date}}` is the same answer tomorrow.
fn library(snippets: &[(&str, &str, &str)]) -> (aralo_testkit::TempDir, Core, Vec<SnippetId>) {
    let folder = aralo_testkit::tempdir().unwrap();
    let mut core = Core::open_without_starter(&folder.path().join("Aralo")).unwrap();
    let ids = snippets
        .iter()
        .map(|(label, abbr, body)| {
            let draft = Draft {
                label: (*label).to_owned(),
                abbr: vec![(*abbr).to_owned()],
                body: (*body).to_owned(),
                ..Draft::default()
            };
            core.create_snippet(&[], &draft).unwrap()
        })
        .collect();
    let core = core
        .with_clock(Arc::new(FixedClock(moment())))
        .in_locale("en_GB");
    (folder, core, ids)
}

/// One snippet, which is what most of these need.
fn one(abbr: &str, body: &str) -> (aralo_testkit::TempDir, Core, SnippetId) {
    let (folder, core, ids) = library(&[("The snippet", abbr, body)]);
    (folder, core, ids[0])
}

fn answers(pairs: &[(&str, &str)]) -> Answers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

/// The session for a snippet picked from a list: the expansion that has
/// something to ask before it can finish.
fn session(core: &Core, id: SnippetId) -> Session {
    core.insert(id)
        .expect("the snippet is in the library")
        .session()
        .expect("the body asks for something")
}

fn ready(step: SessionStep) -> Expansion {
    match step {
        SessionStep::Ready(expansion) => expansion,
        other => panic!("expected an expansion, got {other:?}"),
    }
}

fn form(step: SessionStep) -> aralo_core::Form {
    match step {
        SessionStep::Form(form) => form,
        other => panic!("expected a form, got {other:?}"),
    }
}

fn kinds(diagnostics: &[aralo_core::template::Diagnostic]) -> Vec<DiagnosticKind> {
    diagnostics
        .iter()
        .map(|diagnostic| diagnostic.kind)
        .collect()
}

#[test]
fn a_body_that_needs_nothing_but_the_clock_expands_without_asking() {
    let (_folder, core, id) = one(";ty", "thank you, {{date: %-d %B}}");
    let expansion = core
        .insert(id)
        .unwrap()
        .ready()
        .expect("nothing to ask about");
    assert_eq!(expansion.plan.inserted_text(), "thank you, 9 March");
    assert_eq!(expansion.undo_delete_count, Some(18));
    assert_eq!(kinds(&expansion.template_diagnostics), []);
}

#[test]
fn a_form_is_answered_before_anything_reaches_the_document() {
    let (_folder, core, id) = one(
        ";dear",
        "Hi {{field: who | label: Their name | default: friend}},\nthanks.",
    );
    let mut session = session(&core, id);

    let asked = form(session.step());
    assert_eq!(asked.fields.len(), 1);
    let field = &asked.fields[0];
    assert_eq!(field.name, "who");
    assert_eq!(field.label, "Their name");
    assert_eq!(field.kind, FieldKind::Line);
    assert_eq!(field.default(), "friend");
    // Asking twice asks the same question: a session moves on when it is
    // answered, not when it is read.
    assert_eq!(session.step(), SessionStep::Form(asked));

    // The preview stands on the defaults, because nobody has answered yet.
    assert_eq!(session.preview(), "Hi friend,\nthanks.");

    let expansion = ready(session.submit_form(answers(&[("who", "Dana")])));
    assert_eq!(expansion.plan.inserted_text(), "Hi Dana,\nthanks.");
    // Nothing was typed, so there is nothing to delete first.
    assert_eq!(expansion.plan.steps.len(), 1);
    // What the panel showed last is what went in, byte for byte.
    assert_eq!(session.preview(), expansion.plan.inserted_text());
}

#[test]
fn a_panel_can_preview_a_form_it_is_still_being_typed_into() {
    let (_folder, core, id) = one(
        ";dear",
        "Hi {{field: who | default: friend}}, {{choice: how | options: post, e-mail}}.",
    );
    let mut session = session(&core, id);
    form(session.step());

    // Half-typed answers, straight from the boxes.
    assert_eq!(
        session.preview_with(&answers(&[("who", "Da"), ("how", "e-mail")])),
        "Hi Da, e-mail."
    );
    // A name the form does not have changes nothing, as on submitting.
    assert_eq!(
        session.preview_with(&answers(&[("nobody", "x")])),
        "Hi friend, post."
    );
    // Previewing changed nothing: the session still has the defaults, and is
    // still waiting for the form.
    assert_eq!(session.preview(), "Hi friend, post.");
    assert!(matches!(session.step(), SessionStep::Form(_)));

    let expansion = ready(session.submit_form(answers(&[("who", "Dana")])));
    assert_eq!(expansion.plan.inserted_text(), "Hi Dana, post.");
}

#[test]
fn a_field_nobody_filled_in_keeps_its_default() {
    let (_folder, core, id) = one(";dear", "Hi {{field: who | default: friend}}.");
    let mut session = session(&core, id);
    session.step();

    // A name the form does not have is not an answer to anything.
    let expansion = ready(session.submit_form(answers(&[("nonsense", "ignored")])));
    assert_eq!(session.answers().len(), 1);
    assert_eq!(session.answers()["who"], "friend");
    assert_eq!(expansion.plan.inserted_text(), "Hi friend.");
}

#[test]
fn a_drop_down_stands_on_its_first_option() {
    let (_folder, core, id) = one(
        ";ship",
        "Ships by {{choice: speed | options: post, courier}}.",
    );
    let mut session = session(&core, id);

    let asked = form(session.step());
    let field = &asked.fields[0];
    let FieldKind::Choice { options } = &field.kind else {
        panic!("expected a drop-down, got {:?}", field.kind);
    };
    assert_eq!(options, &["post".to_owned(), "courier".to_owned()]);
    assert_eq!(field.default(), "post");
    assert_eq!(session.preview(), "Ships by post.");

    let expansion = ready(session.submit_form(answers(&[("speed", "courier")])));
    assert_eq!(expansion.plan.inserted_text(), "Ships by courier.");
}

#[test]
fn the_form_comes_first_the_context_second_and_the_plan_after_both() {
    let (_folder, core, id) = one(";ref", "Hi {{field: who}}, about {{clipboard}}.");
    let mut session = session(&core, id);

    assert!(matches!(session.step(), SessionStep::Form(_)));
    assert_eq!(
        session.submit_form(answers(&[("who", "Dana")])),
        SessionStep::Context(vec![ContextKind::Clipboard])
    );

    let expansion = ready(session.provide_context(ContextValues {
        clipboard: Some("PO-8841".to_owned()),
        ..ContextValues::default()
    }));
    assert_eq!(expansion.plan.inserted_text(), "Hi Dana, about PO-8841.");
    assert_eq!(kinds(&expansion.template_diagnostics), []);
}

#[test]
fn a_body_that_asks_for_no_context_is_never_asked_for_any() {
    // PRD P4: the clipboard is read when a body says it needs it, and at no
    // other time. A form is not a reason to go and read one.
    let (_folder, core, id) = one(";dear", "Hi {{field: who | default: friend}}.");
    let mut session = session(&core, id);

    assert_eq!(session.needs(), []);
    assert!(matches!(session.step(), SessionStep::Form(_)));
    assert!(matches!(
        session.submit_form(Answers::new()),
        SessionStep::Ready(_)
    ));
}

#[test]
fn a_kind_the_body_did_not_ask_for_is_dropped_by_the_core() {
    let (_folder, core, id) = one(";ref", "Ref {{clipboard}}.");
    let mut session = session(&core, id);

    // No form, so the first question is the only one.
    assert_eq!(
        session.step(),
        SessionStep::Context(vec![ContextKind::Clipboard])
    );

    // A shell that hands over more than it was asked for has the rest dropped
    // here rather than trusted: the rule is the core's, not the shell's. And
    // nothing stands in for the one value that was asked for and not given.
    let expansion = ready(session.provide_context(ContextValues {
        clipboard: None,
        selection: Some("private".to_owned()),
        app: Some("com.example.Bank".to_owned()),
        window: Some("Statements".to_owned()),
    }));
    assert_eq!(expansion.plan.inserted_text(), "Ref {{clipboard}}.");
    assert_eq!(
        kinds(&expansion.template_diagnostics),
        [DiagnosticKind::NothingSupplied]
    );
}

#[test]
fn a_nested_snippet_brings_its_own_questions_with_it() {
    let (_folder, core, ids) = library(&[
        ("Greeting", ";hi", "Hi {{field: who | default: friend}}"),
        ("Letter", ";letter", "{{snippet: Greeting}},\nthanks."),
    ]);
    let mut session = session(&core, ids[1]);

    // The field is asked for at the body that pulled the snippet in, so the
    // user answers one form however deeply the question was written.
    let asked = form(session.step());
    assert_eq!(asked.fields.len(), 1);
    assert_eq!(asked.fields[0].name, "who");
    let expansion = ready(session.submit_form(answers(&[("who", "Dana")])));
    assert_eq!(expansion.plan.inserted_text(), "Hi Dana,\nthanks.");
}

#[test]
fn a_snippet_that_reaches_itself_is_left_as_written() {
    let (_folder, core, id) = one(";echo", "a {{snippet: The snippet}} b");
    let expansion = core.insert(id).unwrap().ready().expect("nothing to ask");
    // It goes in once, and the reference that would go round again stays.
    assert_eq!(
        expansion.plan.inserted_text(),
        "a a {{snippet: The snippet}} b b"
    );
    assert_eq!(
        kinds(&expansion.template_diagnostics),
        [DiagnosticKind::SnippetCycle]
    );
}

#[test]
fn nesting_stops_at_the_depth_limit() {
    // A chain one longer than Aralo follows: the last reference is left as
    // written, so the body cannot be made to expand without end.
    let last = MAX_SNIPPET_DEPTH + 1;
    let mut snippets: Vec<(String, String, String)> = (0..=last)
        .map(|step| {
            let body = if step == last {
                "the end".to_owned()
            } else {
                format!("{{{{snippet: s{}}}}}", step + 1)
            };
            (format!("s{step}"), format!(";s{step}"), body)
        })
        .collect();
    snippets.reverse();
    let borrowed: Vec<(&str, &str, &str)> = snippets
        .iter()
        .map(|(label, abbr, body)| (label.as_str(), abbr.as_str(), body.as_str()))
        .collect();
    let (_folder, core, _) = library(&borrowed);

    let first = core
        .snippets()
        .iter()
        .find(|snippet| snippet.file.front.label == "s0")
        .expect("s0 is in the library")
        .id;
    let expansion = core.insert(first).unwrap().ready().expect("nothing to ask");
    assert_eq!(
        expansion.plan.inserted_text(),
        format!("{{{{snippet: s{last}}}}}")
    );
    assert_eq!(
        kinds(&expansion.template_diagnostics),
        [DiagnosticKind::SnippetTooDeep]
    );
}

#[test]
fn a_reference_to_no_snippet_says_so_and_stays() {
    let (_folder, core, id) = one(";ref", "see {{snippet: nowhere}}");
    let expansion = core.insert(id).unwrap().ready().expect("nothing to ask");
    assert_eq!(expansion.plan.inserted_text(), "see {{snippet: nowhere}}");
    assert_eq!(
        kinds(&expansion.template_diagnostics),
        [DiagnosticKind::SnippetMissing]
    );
}

#[test]
fn cancelling_puts_back_the_key_that_opened_the_panel() {
    let (_folder, core, _) = one(";dear", "Hi {{field: who | default: friend}}.");
    let mut field = Simulator::new(&core, APP).cancelling();
    field.type_str("well, ;dear ");

    // Nothing was deleted, because the plan that deletes the abbreviation is
    // the plan that replaces it, and that plan never ran. All that goes back
    // is the space the engine swallowed to make the match.
    assert_eq!(field.text(), "well, ;dear ");
    assert_eq!(field.marked(), "well, ;dear |");
    assert_eq!(field.cancellations(), 1);
    assert_eq!(field.expansions(), 0);
}

#[test]
fn cancelling_does_not_press_a_key_the_user_did_not_ask_for() {
    let (_folder, core, _) = one(";dear", "Hi {{field: who | default: friend}}.");
    let mut field = Simulator::new(&core, APP).cancelling();
    field.type_str(";dear\n");

    // A Return would have sent the message. The panel opened on it, the user
    // changed their mind, and Aralo does not send it afterwards.
    assert_eq!(field.text(), ";dear");
    assert_eq!(field.cancellations(), 1);
}

#[test]
fn a_form_filled_in_replaces_the_abbreviation_that_asked_for_it() {
    let (_folder, core, _) = one(";dear", "Hi {{field: who | default: friend}},");
    let mut field = Simulator::new(&core, APP).with_answer("who", "Dana");
    field.type_str("well, ;dear ");

    assert_eq!(field.text(), "well, Hi Dana, ");
    assert_eq!(field.expansions(), 1);
}

#[test]
fn a_cursor_stop_leaves_the_caret_inside_what_went_in() {
    let (_folder, core, _) = one(";dear", "Dear {{cursor}},\nthanks");
    let mut field = Simulator::new(&core, APP);
    field.type_str(";dear ");
    assert_eq!(field.marked(), "Dear |,\nthanks ");
    // What the user types next lands where they are looking.
    field.type_str("Dana");
    assert_eq!(field.text(), "Dear Dana,\nthanks ");
}

#[test]
fn a_pair_of_stops_leaves_the_words_between_them_selected() {
    let (_folder, core, _) = one(
        ";intro",
        "Hi {{cursor: select}}there{{cursor: select}}, welcome",
    );
    let mut field = Simulator::new(&core, APP);
    field.type_str(";intro ");
    assert_eq!(field.selected(), "there");
    assert_eq!(field.marked(), "Hi [there], welcome ");
}

#[test]
fn a_session_can_leave_the_caret_where_the_answer_ends() {
    let (_folder, core, _) = one(";sig", "{{field: who | default: Sam}}{{cursor}} — sent");
    let mut field = Simulator::new(&core, APP).with_answer("who", "Dana");
    field.type_str(";sig ");
    assert_eq!(field.marked(), "Dana| — sent ");
}

#[test]
fn the_locale_a_shell_sets_is_the_one_dates_are_written_in() {
    let (_folder, mut core, id) = one(";today", "{{date: %A %-d %B}}");
    assert_eq!(core.preview(id).unwrap(), "Monday 9 March");
    core.set_locale("de_DE");
    assert_eq!(core.locale(), "de_DE");
    assert_eq!(core.preview(id).unwrap(), "Montag 9 März");
    // A placeholder that names its own locale is not the system's business.
    let (_folder, core, id) = one(";today", "{{date: %-d %B | locale: fr_FR}}");
    assert_eq!(core.preview(id).unwrap(), "9 mars");
}

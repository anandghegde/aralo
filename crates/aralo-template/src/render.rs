//! From a body to the [`ExpansionPlan`] a shell's injector runs.

use crate::eval::{
    push_text, render, resolve, Context, Cursor, Part, Rendered, Resolved, Snippets,
};
use crate::{CaseTransform, Diagnostic, ExpansionPlan, Key, Step};

/// What the engine reported about the match, which is everything a plan needs
/// besides the text itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Shape {
    /// Characters of the abbreviation that already reached the app.
    pub delete_count: u32,
    pub case: CaseTransform,
    /// The delimiter that triggered the match, to put back after the text.
    pub trailing: Option<char>,
}

/// Reads `body`, expands it and builds the plan: the whole path, for a caller
/// that has everything a body needs already.
///
/// The diagnostics are the parser's, resolving's and the expansion's, in the
/// order they appear in the body.
pub fn expand(
    body: &str,
    context: Context<'_>,
    shape: Shape,
    snippets: Option<&dyn Snippets>,
) -> (ExpansionPlan, Vec<Diagnostic>) {
    finish(&resolve(body, snippets), context, shape)
}

/// The second half of the same path, for a caller that resolved the body
/// earlier and has been collecting what it asked for: form answers, the
/// clipboard. An expansion session ends here.
pub fn finish(
    resolved: &Resolved,
    context: Context<'_>,
    shape: Shape,
) -> (ExpansionPlan, Vec<Diagnostic>) {
    let rendered = render(resolved, context);
    let mut diagnostics = resolved.diagnostics().to_vec();
    diagnostics.extend_from_slice(rendered.diagnostics());
    diagnostics.sort_by_key(|diagnostic| (diagnostic.span.start, diagnostic.span.end));
    // A date whose format Aralo cannot write is found twice: once when the
    // body is read, once when it is written out. The user is told once.
    diagnostics.dedup();
    (plan(rendered, shape), diagnostics)
}

/// The plan for text that has already been rendered: delete the abbreviation,
/// insert the text, put the delimiter back, then leave the cursor where the
/// body asked for it.
///
/// The cursor moves last and counts what the plan put in after the stop,
/// including every Return or Tab that went back as a key. An app that acted on
/// such a key has moved on, and the move then does nothing useful; a body that
/// wants a cursor stop is a body to keep keys out of.
pub fn plan(rendered: Rendered, shape: Shape) -> ExpansionPlan {
    let cursor = rendered.cursor();
    let mut segments = recase(shape.case, rendered.into_segments());
    let last = segments
        .last_mut()
        .expect("a rendering has at least one segment");

    // Return and Tab go back as key presses: the user pressed a key the app
    // may act on (send the message, move to the next cell), and text would not
    // do that. Every other delimiter is ordinary text. Either way it joins the
    // last segment, so the cursor stop counts it and lands where the body put
    // it.
    match shape.trailing {
        Some('\r' | '\n') => last.push(Part::Key(Key::Return)),
        Some('\t') => last.push(Part::Key(Key::Tab)),
        Some(other) => push_text(last, other.encode_utf8(&mut [0; 4])),
        None => {}
    }

    let mut steps = Vec::with_capacity(5);
    if shape.delete_count > 0 {
        steps.push(Step::Delete {
            count: shape.delete_count,
        });
    }
    // One insert step per run of text: a key press interrupts the typing,
    // because the app has to see it as a key rather than as a character.
    let mut text = String::new();
    for part in segments.iter().flatten() {
        match part {
            Part::Text(run) => text.push_str(run),
            Part::Key(key) => {
                insert(&mut text, &mut steps);
                steps.push(Step::KeyPress { key: *key });
            }
        }
    }
    insert(&mut text, &mut steps);

    let width = |segment: &Vec<Part>| segment.iter().map(Part::width).sum::<u32>();
    let left = |graphemes: u32, select: bool, steps: &mut Vec<Step>| {
        if graphemes > 0 {
            steps.push(Step::MoveCursor { graphemes, select });
        }
    };
    match (cursor, segments.as_slice()) {
        (Cursor::Caret, [_, tail]) => left(width(tail), false, &mut steps),
        (Cursor::Selection, [_, selected, tail]) => {
            left(width(tail), false, &mut steps);
            left(width(selected), true, &mut steps);
        }
        // A rendering always splits into one segment per stop; a body with no
        // stop is one segment and has nothing to move.
        _ => {}
    }
    ExpansionPlan { steps }
}

/// Puts what has been gathered in, and starts gathering again. Nothing gathered
/// is nothing to insert.
fn insert(text: &mut String, steps: &mut Vec<Step>) {
    if !text.is_empty() {
        steps.push(Step::InsertText {
            text: std::mem::take(text),
        });
    }
}

/// Re-cases the expansion the way the abbreviation was typed. The text runs are
/// re-cased as one expansion, in the order they go in, so a snippet that starts
/// with a `{{cursor}}` or a `{{key}}` still capitalises its first letter.
fn recase(case: CaseTransform, mut segments: Vec<Vec<Part>>) -> Vec<Vec<Part>> {
    if case == CaseTransform::AsDefined {
        return segments;
    }
    let runs = segments
        .iter()
        .flatten()
        .filter_map(|part| match part {
            Part::Text(text) => Some(text.clone()),
            Part::Key(_) => None,
        })
        .collect();
    let mut recased = case.apply_all(runs).into_iter();
    for part in segments.iter_mut().flatten() {
        if let Part::Text(text) = part {
            *text = recased.next().expect("one re-cased run per text run");
        }
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::{Answers, ContextValues, FieldKind, MAX_FIELD_LINES};
    use crate::{CivilTime, DiagnosticKind};

    fn at(hour: u8) -> CivilTime {
        CivilTime::new(2026, 3, 9, hour, 5, 7)
    }

    fn plan_for(body: &str, shape: Shape) -> ExpansionPlan {
        expand(body, Context::at(at(14)), shape, None).0
    }

    fn typed(delete_count: u32, case: CaseTransform, trailing: Option<char>) -> Shape {
        Shape {
            delete_count,
            case,
            trailing,
        }
    }

    #[test]
    fn deletes_the_abbreviation_then_inserts_text_and_delimiter() {
        assert_eq!(
            plan_for("thank you", typed(2, CaseTransform::Title, Some(' '))).steps,
            [
                Step::Delete { count: 2 },
                Step::InsertText {
                    text: "Thank you ".into()
                },
            ]
        );
    }

    #[test]
    fn return_and_tab_delimiters_are_key_presses() {
        assert_eq!(
            plan_for("on my way", typed(3, CaseTransform::AsDefined, Some('\r'))).steps,
            [
                Step::Delete { count: 3 },
                Step::InsertText {
                    text: "on my way".into()
                },
                Step::KeyPress { key: Key::Return },
            ]
        );
        assert_eq!(
            plan_for("x", typed(1, CaseTransform::AsDefined, Some('\t'))).steps[2],
            Step::KeyPress { key: Key::Tab }
        );
    }

    #[test]
    fn nothing_to_delete_means_no_delete_step() {
        // A one-character immediate abbreviation: the key was swallowed.
        assert_eq!(
            plan_for("§", Shape::default()).steps,
            [Step::InsertText { text: "§".into() }]
        );
    }

    #[test]
    fn a_placeholder_aralo_cannot_expand_stays_as_written() {
        let (plan, diagnostics) = expand(
            "\\{{not one}} {{nonsense}} {{broken",
            Context::at(at(14)),
            Shape::default(),
            None,
        );
        assert_eq!(plan.inserted_text(), "{{not one}} {{nonsense}} {{broken");
        let messages: Vec<_> = diagnostics.iter().map(Diagnostic::message).collect();
        assert_eq!(messages.len(), 2, "{messages:?}");
        assert!(messages[0].contains("nonsense"), "{messages:?}");
        assert!(messages[1].contains("has no }}"), "{messages:?}");
    }

    #[test]
    fn a_problem_found_twice_is_reported_once() {
        // The lint reads the format when the body is read; the expansion
        // reads it again when it writes the date out.
        let (plan, diagnostics) =
            expand("{{date: %Q}}", Context::at(at(14)), Shape::default(), None);
        assert_eq!(plan.inserted_text(), "{{date: %Q}}");
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].kind, DiagnosticKind::BadFormat);
        assert_eq!(diagnostics[0].detail.as_deref(), Some("%Q"));
    }

    #[test]
    fn the_clock_the_clipboard_and_the_answers_come_from_the_context() {
        let values = ContextValues {
            clipboard: Some("PO-8841".into()),
            ..ContextValues::default()
        };
        let answers = Answers::from([("who".to_owned(), "Dana".to_owned())]);
        let context = Context::at(at(14))
            .with_values(&values)
            .with_answers(&answers);
        let (plan, diagnostics) = expand(
            "Hi {{field: who}}, order {{clipboard}} shipped {{date: %-d %B}} at {{time: %H:%M}}.",
            context,
            Shape::default(),
            None,
        );
        assert_eq!(
            plan.inserted_text(),
            "Hi Dana, order PO-8841 shipped 9 March at 14:05."
        );
        assert_eq!(diagnostics, []);
    }

    #[test]
    fn a_cursor_stop_moves_back_over_what_follows_it() {
        let plan = plan_for("Dear {{cursor}},\nthanks", Shape::default());
        assert_eq!(
            plan.steps,
            [
                Step::InsertText {
                    text: "Dear ,\nthanks".into()
                },
                Step::MoveCursor {
                    graphemes: 8,
                    select: false
                },
            ]
        );
        // Backspace cannot undo an expansion that moved the cursor.
        assert_eq!(plan.undo_delete_count(), None);
    }

    #[test]
    fn a_cursor_stop_at_the_end_moves_nothing_and_keeps_undo() {
        let plan = plan_for("see you{{cursor}}", Shape::default());
        assert_eq!(
            plan.steps,
            [Step::InsertText {
                text: "see you".into()
            }]
        );
        assert_eq!(plan.undo_delete_count(), Some(7));
    }

    #[test]
    fn a_pair_of_select_stops_selects_what_is_between_them() {
        let plan = plan_for(
            "Hi {{cursor: select}}there{{cursor: select}}, welcome",
            typed(0, CaseTransform::AsDefined, Some(' ')),
        );
        assert_eq!(
            plan.steps,
            [
                Step::InsertText {
                    text: "Hi there, welcome ".into()
                },
                Step::MoveCursor {
                    graphemes: 10,
                    select: false
                },
                Step::MoveCursor {
                    graphemes: 5,
                    select: true
                },
            ]
        );
    }

    #[test]
    fn the_cursor_counts_what_a_user_sees_as_a_character() {
        // A family emoji is one grapheme and seven code points; a delimiter
        // that goes back as text is counted, a key press is counted once.
        let plan = plan_for(
            "a{{cursor}}b👨‍👩‍👧‍👦c",
            typed(0, CaseTransform::AsDefined, Some('.')),
        );
        assert_eq!(
            plan.steps[1],
            Step::MoveCursor {
                graphemes: 4,
                select: false
            }
        );
        let plan = plan_for(
            "a{{cursor}}b",
            typed(0, CaseTransform::AsDefined, Some('\r')),
        );
        assert_eq!(
            plan.steps[2],
            Step::MoveCursor {
                graphemes: 2,
                select: false
            }
        );
    }

    #[test]
    fn a_key_goes_in_as_a_key_press_between_what_it_interrupts() {
        // The snippet that fills a form in: a box, the key that moves to the
        // next one, the next box.
        let answers = Answers::from([
            ("name".to_owned(), "Dana".to_owned()),
            ("email".to_owned(), "dana@acme.example".to_owned()),
        ]);
        let context = Context::at(at(14)).with_answers(&answers);
        let (plan, diagnostics) = expand(
            "{{field: name}}{{key: tab}}{{field: email}}",
            context,
            Shape::default(),
            None,
        );
        assert_eq!(diagnostics, []);
        assert_eq!(
            plan.steps,
            [
                Step::InsertText {
                    text: "Dana".into()
                },
                Step::KeyPress { key: Key::Tab },
                Step::InsertText {
                    text: "dana@acme.example".into()
                },
            ]
        );
        // A key the app may have acted on is a key Backspace cannot take back.
        assert_eq!(plan.undo_delete_count(), None);
        // A preview writes the key as the character it types.
        assert_eq!(plan.inserted_text(), "Dana\tdana@acme.example");
    }

    #[test]
    fn a_key_at_the_start_needs_nothing_inserted_before_it() {
        assert_eq!(
            plan_for(
                "{{key: return}}done",
                typed(2, CaseTransform::AsDefined, None)
            )
            .steps,
            [
                Step::Delete { count: 2 },
                Step::KeyPress { key: Key::Return },
                Step::InsertText {
                    text: "done".into()
                },
            ]
        );
    }

    #[test]
    fn a_key_aralo_cannot_press_stays_as_written() {
        let (plan, diagnostics) = expand(
            "left: {{key: left}}",
            Context::at(at(14)),
            Shape::default(),
            None,
        );
        assert_eq!(plan.inserted_text(), "left: {{key: left}}");
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].kind, DiagnosticKind::UnknownKey);
        assert!(diagnostics[0].message().contains("left"), "{diagnostics:?}");
        // A body that names no key at all says which keys there are.
        let (_, diagnostics) = expand("{{key}}", Context::at(at(14)), Shape::default(), None);
        assert_eq!(diagnostics[0].kind, DiagnosticKind::UnknownKey);
        assert!(
            diagnostics[0].message().contains("{{key: tab}}"),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn a_cursor_stop_moves_back_over_a_key_as_over_a_character() {
        let plan = plan_for("a{{cursor}}b{{key: tab}}c", Shape::default());
        assert_eq!(
            plan.steps,
            [
                Step::InsertText { text: "ab".into() },
                Step::KeyPress { key: Key::Tab },
                Step::InsertText { text: "c".into() },
                Step::MoveCursor {
                    graphemes: 3,
                    select: false
                },
            ]
        );
    }

    #[test]
    fn a_key_has_no_case_and_does_not_stop_the_expansion_being_re_cased() {
        let shout = plan_for(
            "{{key: tab}}hi there",
            typed(0, CaseTransform::Upper, Some(' ')),
        );
        assert_eq!(shout.inserted_text(), "\tHI THERE ");
        let title = plan_for(
            "{{key: tab}}hi there",
            typed(0, CaseTransform::Title, Some(' ')),
        );
        assert_eq!(title.inserted_text(), "\tHi there ");
    }

    #[test]
    fn a_field_the_body_asks_for_several_lines_of_is_a_box_that_tall() {
        let lines = |body: &str| resolve(body, None).form().fields[0].kind.clone();
        assert_eq!(
            lines("{{field: notes | lines: 4}}"),
            FieldKind::Area { lines: 4 }
        );
        // One line is the ordinary box, and so is a body that asks for none.
        assert_eq!(lines("{{field: notes | lines: 1}}"), FieldKind::Line);
        assert_eq!(lines("{{field: notes}}"), FieldKind::Line);
        // A panel taller than the screen has no buttons on it.
        assert_eq!(
            lines("{{field: notes | lines: 900}}"),
            FieldKind::Area {
                lines: MAX_FIELD_LINES
            }
        );
    }

    #[test]
    fn a_line_count_aralo_cannot_read_is_left_out_and_said_so() {
        let (plan, diagnostics) = expand(
            "{{field: notes | lines: plenty}}",
            Context::at(at(14)),
            Shape::default(),
            None,
        );
        assert_eq!(plan.inserted_text(), "");
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
        assert_eq!(diagnostics[0].kind, DiagnosticKind::BadOption);
        assert!(
            diagnostics[0].message().contains("lines"),
            "{diagnostics:?}"
        );
        assert_eq!(
            resolve("{{field: notes | lines: plenty}}", None)
                .form()
                .fields[0]
                .kind,
            FieldKind::Line
        );
    }

    #[test]
    fn the_case_of_the_abbreviation_re_cases_the_whole_expansion() {
        let answers = Answers::from([("who".to_owned(), "dana".to_owned())]);
        let context = Context::at(at(14)).with_answers(&answers);
        let shout = expand(
            "hi {{field: who}}, bye",
            context,
            typed(0, CaseTransform::Upper, None),
            None,
        );
        assert_eq!(shout.0.inserted_text(), "HI DANA, BYE");
        // Title case applies once, across the cursor stops, wherever the
        // first letter of the expansion turns out to be.
        let title = expand(
            "{{cursor}}hi there",
            Context::at(at(14)),
            typed(0, CaseTransform::Title, None),
            None,
        );
        assert_eq!(title.0.inserted_text(), "Hi there");
    }
}

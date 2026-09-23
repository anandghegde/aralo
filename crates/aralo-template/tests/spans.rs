//! Property test: whatever a body says, reading it and expanding it come back
//! with something the shells can use.
//!
//! A body is user text. It arrives half-written from an editor, pasted from
//! another expander, or nested inside another snippet, so nothing here may
//! depend on it being well formed. Three properties, for any body at all:
//!
//! * no step of the path panics — the bridge turns a panic into a dead app;
//! * every range handed out is a byte range of the body, on character
//!   boundaries, so an editor can slice the body with it;
//! * the plan inserts what the expansion rendered, and leaves the cursor
//!   inside it.

use std::collections::BTreeMap;
use std::ops::Range;

use aralo_template::{
    expand, outline_with, parse, plan, render, resolve, BodyRange, CivilTime, Context,
    ContextValues, Cursor, Diagnostic, Key, Nested, Node, Resolved, Shape, Snippets, Step,
};
use proptest::prelude::*;
use unicode_segmentation::UnicodeSegmentation;

/// A handful of snippets to nest, with a cycle and a nest of a nest: resolving
/// walks them, so the ranges it hands back come from more than one body.
struct Library;

impl Snippets for Library {
    fn body(&self, reference: &str) -> Option<Nested> {
        let body = match reference {
            "one" => "Dear {{field: who}},",
            "two" => "See {{snippet: one}} and {{cursor}}.",
            "loop" => "Round {{snippet: loop}} again.",
            "empty" => "",
            _ => return None,
        };
        Some(Nested {
            key: reference.to_owned(),
            body: body.to_owned(),
        })
    }
}

/// The pieces a body is built from: enough placeholder grammar to parse, and
/// enough broken grammar to fail to.
fn fragment() -> impl Strategy<Value = String> {
    prop_oneof![
        6 => prop::sample::select(vec![
            "{{date}}",
            "{{date: format: %Y-%m-%d | locale: de}}",
            "{{time: format: %H:%M}}",
            "{{clipboard}}",
            "{{selection}}",
            "{{app}}",
            "{{cursor}}",
            "{{cursor: select}}",
            "{{field: who}}",
            "{{field: who | label: Their name | default: friend}}",
            "{{field: notes | lines: 3}}",
            "{{field: notes | lines: plenty}}",
            "{{key: tab}}",
            "{{key: return}}",
            "{{key: left}}",
            "{{choice: how | options: post, e-mail}}",
            "{{snippet: one}}",
            "{{snippet: two}}",
            "{{snippet: loop}}",
            "{{snippet: empty}}",
            "{{snippet: nobody}}",
            "{{ai: write something | fallback: never mind}}",
            "{{nosuchthing}}",
        ]).prop_map(String::from),
        // The half-written, the malformed and the merely awkward.
        4 => prop::sample::select(vec![
            "{{", "}}", "{", "}", "|", ":", ",", "\\{", "\\\\", "{{}}", "{{ }}", "{{date",
            "{{field:}}", "{{field: }}", "{{DATE}}", "{{date | | }}", "{{choice: how}}",
        ]).prop_map(String::from),
        3 => prop::sample::select(vec![
            "Dear ", " and ", ".\n", "\t", " ", "é", "👩‍💻", "日本語", "ß", "\u{301}", "\r\n",
        ]).prop_map(String::from),
        // Grammar nobody would write, in case the pieces above are too kind.
        2 => prop::collection::vec(
            prop::sample::select(vec![
                '{', '}', '|', ':', ',', '\\', 'a', ' ', 'é', '😀', '\u{301}', '%',
            ]),
            1..9,
        ).prop_map(|characters| characters.into_iter().collect::<String>()),
    ]
}

fn body() -> impl Strategy<Value = String> {
    prop::collection::vec(fragment(), 0..14).prop_map(|parts| parts.concat())
}

fn shape() -> impl Strategy<Value = Shape> {
    (
        0u32..4,
        prop::option::of(prop::sample::select(vec![' ', '.', '\t', '\n', '\r', 'x'])),
    )
        .prop_map(|(delete_count, trailing)| Shape {
            delete_count,
            case: Default::default(),
            trailing,
        })
}

/// A byte range the core is about to slice the body with: it must be in bounds,
/// the right way round, and on character boundaries, or slicing the body with it
/// panics.
fn check(body: &str, range: &Range<usize>, what: &str) -> Result<(), TestCaseError> {
    prop_assert!(
        range.start <= range.end
            && range.end <= body.len()
            && body.is_char_boundary(range.start)
            && body.is_char_boundary(range.end),
        "{what}: {range:?} is not a range of {body:?}"
    );
    Ok(())
}

/// A range an editor is about to slice the body with. An outline counts in
/// UTF-16 code units, because that is what a macOS text view and a JavaScript
/// string count in, so the check is that the units it names make a string: a
/// range that cut an emoji's surrogate pair in half would throw in the shell.
fn check_units(units: &[u16], range: BodyRange, what: &str) -> Result<(), TestCaseError> {
    let (start, end) = (range.start as usize, range.end as usize);
    prop_assert!(
        start <= end && end <= units.len(),
        "{what}: {range:?} is not a range of {} code units",
        units.len()
    );
    prop_assert!(
        String::from_utf16(&units[start..end]).is_ok(),
        "{what}: {range:?} cuts a character in half"
    );
    Ok(())
}

fn check_diagnostics(
    body: &str,
    diagnostics: &[Diagnostic],
    what: &str,
) -> Result<(), TestCaseError> {
    for diagnostic in diagnostics {
        check(body, &diagnostic.span, what)?;
    }
    Ok(())
}

/// Everything `resolve` says about a body, checked against the body itself: a
/// placeholder from a nested snippet carries the span of the `{{snippet}}` that
/// pulled it in, so every range still points into the body the user can see.
fn check_resolved(body: &str, resolved: &Resolved) -> Result<(), TestCaseError> {
    check_diagnostics(body, resolved.diagnostics(), "resolve diagnostic")?;
    for placeholder in resolved.placeholders() {
        check(body, &placeholder.span, "resolved placeholder")?;
    }
    for field in &resolved.form().fields {
        check(body, &field.span, "form field")?;
        prop_assert!(
            !field.name.is_empty(),
            "a field with no name is not a box to fill in: {field:?}"
        );
    }
    let mut names: Vec<&str> = resolved
        .form()
        .fields
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    let asked = names.len();
    names.sort_unstable();
    names.dedup();
    prop_assert_eq!(names.len(), asked, "a name used twice is one box");
    let mut needs = resolved.needs().to_vec();
    let wanted = needs.len();
    needs.sort_unstable();
    needs.dedup();
    prop_assert_eq!(needs.len(), wanted, "a kind is asked for once");
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(3000))]

    /// Reading a body: the parser, the resolver and the editor's outline.
    #[test]
    fn every_range_a_body_yields_is_a_range_of_that_body(body in body()) {
        let template = parse(&body);
        check_diagnostics(&body, &template.diagnostics, "parse diagnostic")?;
        for node in &template.nodes {
            if let Node::Placeholder(placeholder) = node {
                check(&body, &placeholder.span, "parsed placeholder")?;
                prop_assert_eq!(
                    &placeholder.name, &placeholder.name.to_lowercase(),
                    "the parser lower-cases every name"
                );
            }
        }

        // With a library and without: an editor drawing a draft has none, and a
        // nested snippet then stays as written.
        check_resolved(&body, &resolve(&body, Some(&Library)))?;
        check_resolved(&body, &resolve(&body, None))?;

        let units: Vec<u16> = body.encode_utf16().collect();
        for outline in [outline_with(&body, Some(&Library)), outline_with(&body, None)] {
            for placeholder in &outline.placeholders {
                check_units(&units, placeholder.range, "outlined placeholder")?;
            }
            for problem in &outline.problems {
                check_units(&units, problem.range, "outlined problem")?;
                prop_assert!(!problem.message.is_empty(), "a problem the editor cannot show");
            }
        }
    }

    /// Expanding it: the plan inserts what was rendered and leaves the cursor
    /// inside it.
    #[test]
    fn a_plan_inserts_what_was_rendered_and_stops_inside_it(body in body(), shape in shape()) {
        let values = ContextValues {
            clipboard: Some("PO-8841".to_owned()),
            selection: Some("what was selected".to_owned()),
            app: Some("Mail".to_owned()),
            window: Some("Re: the kitchen".to_owned()),
        };
        let resolved = resolve(&body, Some(&Library));
        let answers: BTreeMap<String, String> = resolved
            .form()
            .fields
            .iter()
            .map(|field| (field.name.clone(), "Dana".to_owned()))
            .collect();
        let context = Context::at(CivilTime::new(2026, 2, 28, 23, 59, 30))
            .in_locale("de")
            .with_values(&values)
            .with_answers(&answers);

        let rendered = render(&resolved, context);
        check_diagnostics(&body, rendered.diagnostics(), "render diagnostic")?;
        prop_assert_eq!(rendered.text(), rendered.segments().concat());
        let stops = match rendered.cursor() {
            Cursor::None => 0,
            Cursor::Caret => 1,
            Cursor::Selection => 2,
        };
        prop_assert_eq!(
            rendered.segments().len(), stops + 1,
            "one part per stop, and one more: {:?}", rendered.segments()
        );

        let text = rendered.text();
        let cursor = rendered.cursor();
        let plan = plan(rendered, shape);

        let mut inserted = String::new();
        let mut deleted = 0;
        // How far back the cursor may travel: a grapheme for each one typed,
        // and one for each key, whatever the key joins into in the string.
        let mut room = 0;
        let mut moved = 0;
        for step in &plan.steps {
            match step {
                Step::InsertText { text } => {
                    inserted.push_str(text);
                    room += text.graphemes(true).count();
                }
                Step::InsertRich { plain, .. } => {
                    inserted.push_str(plain);
                    room += plain.graphemes(true).count();
                }
                Step::Delete { count } => deleted += *count,
                Step::KeyPress { key } => {
                    inserted.push(match key {
                        Key::Return => '\n',
                        Key::Tab => '\t',
                    });
                    room += 1;
                }
                Step::MoveCursor { graphemes, select } => {
                    prop_assert!(*graphemes > 0, "a move of nothing is a step to leave out");
                    if !select {
                        moved += *graphemes as usize;
                    }
                }
                Step::Delay { .. } => {}
            }
        }
        prop_assert_eq!(deleted, shape.delete_count, "the plan deletes what the engine typed");

        // A delimiter goes back as text, or as the key an app acts on, which
        // types the character that key types.
        let trailing = match shape.trailing {
            Some('\r' | '\n') => "\n".to_owned(),
            Some(other) => other.to_string(),
            None => String::new(),
        };
        prop_assert_eq!(&inserted, &format!("{text}{trailing}"));

        // The cursor moves back over what the plan put in after the stop, so it
        // lands inside the text and never in what the user wrote before it.
        prop_assert!(moved <= room, "moved {moved} back over {room} graphemes");
        if cursor == Cursor::None {
            prop_assert_eq!(moved, 0, "with no stop the cursor stays where typing left it");
        }
    }

    /// The crate is pure: the same body and the same context, twice, give the
    /// same plan and the same diagnostics.
    #[test]
    fn expanding_twice_gives_the_same_plan(body in body(), shape in shape()) {
        let context = Context::at(CivilTime::new(2026, 6, 1, 12, 0, 0));
        let first = expand(&body, context, shape, Some(&Library));
        let second = expand(&body, context, shape, Some(&Library));
        prop_assert_eq!(first, second);
    }
}

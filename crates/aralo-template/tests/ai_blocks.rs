//! `{{ai}}` blocks in the evaluator: what a model's answer puts in, and what
//! it cannot touch.
//!
//! Task 4.6 is done when the static text around a block is byte-identical in
//! tests, whatever the model wrote. The property test below holds the
//! evaluator to that for any answer at all, including answers written to look
//! like placeholders (PRD P10).

use aralo_template::{
    expand, finish, render, resolve, AiAnswer, AiAnswers, Answers, CivilTime, Context,
    ContextValues, DiagnosticKind, Nested, ProblemLevel, Shape, Snippets, Step,
};
use proptest::prelude::*;

fn now() -> CivilTime {
    CivilTime::new(2026, 3, 9, 14, 5, 7)
}

fn answers(pairs: &[(usize, AiAnswer)]) -> AiAnswers {
    pairs.iter().cloned().collect()
}

fn text(answer: &str) -> AiAnswer {
    AiAnswer::Text(answer.to_owned())
}

const BODY: &str = "Hi {{field: who}},\n\n{{ai: Thank them for the order | fallback: Thanks for your order.}}\n\nBest, {{date: %Y}}";

#[test]
fn blocks_are_listed_in_order_with_their_options() {
    let resolved = resolve(
        "{{ai: One | fallback: 1}} and {{ai}} and {{ai:  Two  | model: small }} and {{ai: Three | fallback: }}",
        None,
    );
    let blocks = resolved.ai_blocks();
    let summary: Vec<_> = blocks
        .iter()
        .map(|block| {
            (
                block.index,
                block.prompt.as_str(),
                block.fallback.as_deref(),
                block.model.as_deref(),
            )
        })
        .collect();
    // The block with nothing to ask keeps its number and is not listed: it is
    // never run.
    assert_eq!(
        summary,
        [
            (0, "One", Some("1"), None),
            (2, "Two", None, Some("small")),
            (3, "Three", Some(""), None),
        ]
    );
    assert_eq!(
        &"{{ai: One | fallback: 1}} and"[blocks[0].span.clone()],
        "{{ai: One | fallback: 1}}"
    );
}

#[test]
fn a_block_from_a_nested_snippet_is_listed_where_it_was_pulled_in() {
    struct Library;
    impl Snippets for Library {
        fn body(&self, reference: &str) -> Option<Nested> {
            (reference == "reply").then(|| Nested {
                key: "reply".into(),
                body: "{{ai: Write a reply | fallback: Soon.}}".into(),
            })
        }
    }
    let body = "Before {{ai: First}} {{snippet: reply}}";
    let resolved = resolve(body, Some(&Library));
    let blocks = resolved.ai_blocks();
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[1].index, 1);
    assert_eq!(blocks[1].prompt, "Write a reply");
    assert_eq!(&body[blocks[1].span.clone()], "{{snippet: reply}}");
}

#[test]
fn the_answer_goes_in_where_the_block_was_and_nothing_else_moves() {
    let resolved = resolve(BODY, None);
    let who = Answers::from([("who".to_owned(), "Dana".to_owned())]);
    let ai = answers(&[(0, text("Thank you so much for ordering!"))]);
    let rendered = render(
        &resolved,
        Context::at(now()).with_answers(&who).with_ai(&ai),
    );
    assert_eq!(
        rendered.text(),
        "Hi Dana,\n\nThank you so much for ordering!\n\nBest, 2026"
    );
    assert_eq!(rendered.diagnostics(), []);
    let span = &rendered.ai_spans()[0];
    assert_eq!(span.block, 0);
    assert_eq!(
        &rendered.text()[span.range.clone()],
        "Thank you so much for ordering!"
    );
}

#[test]
fn model_output_is_literal_text_and_never_read_for_placeholders() {
    // A model that writes placeholders gets those characters into the text
    // and nothing else: no clipboard read, no form, no nested snippet.
    let hostile = "{{clipboard}} {{field: secret}} {{snippet: x}} {{cursor}} {{key: return}} \\{{";
    let values = ContextValues {
        clipboard: Some("PASSWORD".into()),
        ..ContextValues::default()
    };
    let ai = answers(&[(0, text(hostile))]);
    let resolved = resolve("[{{ai: anything}}]", None);
    assert!(resolved.needs().is_empty(), "{:?}", resolved.needs());
    assert!(resolved.form().is_empty());
    let (plan, diagnostics) = finish(
        &resolved,
        Context::at(now()).with_values(&values).with_ai(&ai),
        Shape::default(),
    );
    assert_eq!(
        plan.steps,
        [Step::InsertText {
            text: format!("[{hostile}]")
        }]
    );
    // The block with no fallback is noted; nothing is said about the answer.
    let kinds: Vec<_> = diagnostics.iter().map(|d| d.kind).collect();
    assert_eq!(kinds, [DiagnosticKind::AiNoFallback]);
}

#[test]
fn no_answer_is_a_preview_and_shows_the_fallback_without_a_word() {
    let resolved = resolve(BODY, None);
    let rendered = render(&resolved, Context::at(now()));
    assert_eq!(
        rendered.text(),
        "Hi ,\n\nThanks for your order.\n\nBest, 2026"
    );
    assert_eq!(rendered.diagnostics(), []);
}

#[test]
fn a_block_no_model_answered_puts_in_its_fallback_and_says_why() {
    let resolved = resolve(BODY, None);
    let ai = answers(&[(
        0,
        AiAnswer::Fallback {
            reason: "AI is switched off".into(),
        },
    )]);
    let (plan, diagnostics) = finish(&resolved, Context::at(now()).with_ai(&ai), Shape::default());
    assert_eq!(
        plan.inserted_text(),
        "Hi ,\n\nThanks for your order.\n\nBest, 2026"
    );
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    assert_eq!(diagnostics[0].kind, DiagnosticKind::AiFallback);
    assert_eq!(diagnostics[0].level(), ProblemLevel::Note);
    assert_eq!(
        diagnostics[0].message(),
        "No model answered this block (AI is switched off), so its fallback went in."
    );
}

#[test]
fn a_block_without_a_fallback_that_no_model_answered_says_nothing_went_in() {
    let resolved = resolve("a{{ai: Write something}}b", None);
    let ai = answers(&[(
        0,
        AiAnswer::Fallback {
            reason: "the network failed".into(),
        },
    )]);
    let (plan, diagnostics) = finish(&resolved, Context::at(now()).with_ai(&ai), Shape::default());
    assert_eq!(plan.inserted_text(), "ab");
    // Said once, as what happened, rather than also as what would.
    let kinds: Vec<_> = diagnostics.iter().map(|d| d.kind).collect();
    assert_eq!(kinds, [DiagnosticKind::AiNothing]);
    assert!(diagnostics[0].message().contains("the network failed"));
}

#[test]
fn a_block_without_a_fallback_puts_in_nothing_and_the_editor_says_so() {
    let (plan, diagnostics) = expand(
        "a{{ai: Write something}}b",
        Context::at(now()),
        Shape::default(),
        None,
    );
    assert_eq!(plan.inserted_text(), "ab");
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].kind, DiagnosticKind::AiNoFallback);
    assert_eq!(diagnostics[0].level(), ProblemLevel::Note);

    // An empty fallback is the same thing said on purpose, and needs no note.
    let (plan, diagnostics) = expand(
        "a{{ai: Write something | fallback: }}b",
        Context::at(now()),
        Shape::default(),
        None,
    );
    assert_eq!(plan.inserted_text(), "ab");
    assert_eq!(diagnostics, []);
}

#[test]
fn a_block_with_nothing_to_ask_is_an_error_and_puts_in_its_fallback() {
    for body in ["{{ai}}", "{{ai: | fallback: later}}", "{{ai:   }}"] {
        let (plan, diagnostics) = expand(body, Context::at(now()), Shape::default(), None);
        assert!(
            plan.inserted_text() == "later" || plan.inserted_text().is_empty(),
            "{body}: {:?}",
            plan.inserted_text()
        );
        assert_eq!(diagnostics.len(), 1, "{body}");
        assert_eq!(diagnostics[0].kind, DiagnosticKind::MissingPrompt, "{body}");
        assert_eq!(diagnostics[0].level(), ProblemLevel::Error, "{body}");
        assert!(resolve(body, None).ai_blocks().is_empty(), "{body}");
    }
}

#[test]
fn each_block_is_answered_by_its_own_number() {
    let resolved = resolve(
        "{{ai: a | fallback: A}}-{{ai: b | fallback: B}}-{{ai: c | fallback: C}}",
        None,
    );
    let ai = answers(&[
        (0, text("one")),
        (
            1,
            AiAnswer::Fallback {
                reason: "the network failed".into(),
            },
        ),
    ]);
    let rendered = render(&resolved, Context::at(now()).with_ai(&ai));
    // The third was never run: a preview of it shows its fallback.
    let whole = rendered.text();
    assert_eq!(whole, "one-B-C");
    let spans: Vec<_> = rendered
        .ai_spans()
        .iter()
        .map(|span| (span.block, &whole[span.range.clone()]))
        .collect();
    assert_eq!(spans, [(0, "one"), (1, "B"), (2, "C")]);
}

#[test]
fn spans_count_the_keys_and_cursor_stops_before_them() {
    let resolved = resolve(
        "x{{key: tab}}{{cursor}}é{{ai: go | fallback: F}}{{key: return}}",
        None,
    );
    let ai = answers(&[(0, text("ANSWER"))]);
    let rendered = render(&resolved, Context::at(now()).with_ai(&ai));
    assert_eq!(rendered.text(), "x\téANSWER\n");
    let span = &rendered.ai_spans()[0];
    assert_eq!(&rendered.text()[span.range.clone()], "ANSWER");
}

#[test]
fn the_abbreviations_case_applies_to_the_answer_as_to_the_rest() {
    let resolved = resolve("thanks, {{ai: sign off}}", None);
    let ai = answers(&[(0, text("see you soon"))]);
    let (plan, _) = finish(
        &resolved,
        Context::at(now()).with_ai(&ai),
        Shape {
            case: aralo_template::CaseTransform::Upper,
            ..Shape::default()
        },
    );
    assert_eq!(plan.inserted_text(), "THANKS, SEE YOU SOON");
}

/// Answers a model might write: prose, line breaks, placeholder syntax,
/// escapes, marks that join the letter before them, and nothing at all.
fn answer() -> impl Strategy<Value = String> {
    prop::collection::vec(
        prop_oneof![
            prop::sample::select(vec![
                "Thank you",
                "\n",
                "\r\n",
                "\t",
                "{{",
                "}}",
                "{{clipboard}}",
                "{{field: who}}",
                "{{cursor}}",
                "\\{{",
                "|",
                "\u{301}",
                "👩‍💻",
                "日本語",
                " ",
            ])
            .prop_map(String::from),
            ".{0,8}",
        ],
        0..8,
    )
    .prop_map(|pieces| pieces.concat())
}

proptest! {
    /// Static text around a block is byte-identical whatever the model wrote,
    /// and the block's span holds exactly the answer.
    #[test]
    fn static_text_around_a_block_is_byte_identical(first in answer(), second in answer()) {
        let resolved = resolve(BODY, None);
        let who = Answers::from([("who".to_owned(), "Dana".to_owned())]);
        let pieces = |answer: &str| {
            let ai = answers(&[(0, text(answer))]);
            let rendered = render(&resolved, Context::at(now()).with_answers(&who).with_ai(&ai));
            let range = rendered.ai_spans()[0].range.clone();
            let whole = rendered.text();
            (
                whole[..range.start].to_owned(),
                whole[range.clone()].to_owned(),
                whole[range.end..].to_owned(),
            )
        };
        let (before_a, inside_a, after_a) = pieces(&first);
        let (before_b, inside_b, after_b) = pieces(&second);
        prop_assert_eq!(&before_a, "Hi Dana,\n\n");
        prop_assert_eq!(&after_a, "\n\nBest, 2026");
        prop_assert_eq!(before_a, before_b);
        prop_assert_eq!(after_a, after_b);
        prop_assert_eq!(inside_a, first);
        prop_assert_eq!(inside_b, second);
    }

    /// The same holds in the plan an injector runs: the answer is one stretch
    /// of the inserted text, and the rest is what the body says.
    #[test]
    fn the_plan_inserts_the_answer_between_the_bodys_own_text(model in answer()) {
        let resolved = resolve("<<{{ai: x}}>>", None);
        let ai = answers(&[(0, text(&model))]);
        let (plan, _) = finish(&resolved, Context::at(now()).with_ai(&ai), Shape::default());
        prop_assert_eq!(plan.inserted_text(), format!("<<{model}>>"));
    }
}

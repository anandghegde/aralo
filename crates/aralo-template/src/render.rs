use crate::{parse, CaseTransform, Diagnostic, ExpansionPlan, Key, Node, Step};

/// Everything a static expansion needs. The first three fields come straight
/// from the engine's match verdict.
#[derive(Debug, Clone, Copy)]
pub struct StaticExpansion<'a> {
    pub body: &'a str,
    /// Characters of the abbreviation that already reached the app.
    pub delete_count: u32,
    pub case: CaseTransform,
    /// The delimiter that triggered the match, to put back after the text.
    pub trailing: Option<char>,
}

/// Builds the plan for a snippet without evaluating placeholders: delete the
/// abbreviation, insert the text, put the delimiter back.
///
/// Until the evaluator lands (M3) a placeholder is inserted as its source
/// text. The returned diagnostics are the parser's.
pub fn static_plan(expansion: StaticExpansion<'_>) -> (ExpansionPlan, Vec<Diagnostic>) {
    let template = parse(expansion.body);
    let mut text = String::with_capacity(expansion.body.len() + 1);
    for node in &template.nodes {
        match node {
            Node::Text(literal) => text.push_str(literal),
            Node::Placeholder(placeholder) => {
                text.push_str(&expansion.body[placeholder.span.clone()]);
            }
        }
    }
    let mut text = expansion.case.apply(&text);

    // Return and Tab go back as key presses: the user pressed a key the app
    // may act on (send the message, move to the next cell), and text would not
    // do that. Every other delimiter is ordinary text.
    let key = match expansion.trailing {
        Some('\r' | '\n') => Some(Key::Return),
        Some('\t') => Some(Key::Tab),
        Some(other) => {
            text.push(other);
            None
        }
        None => None,
    };

    let mut steps = Vec::with_capacity(3);
    if expansion.delete_count > 0 {
        steps.push(Step::Delete {
            count: expansion.delete_count,
        });
    }
    if !text.is_empty() {
        steps.push(Step::InsertText { text });
    }
    if let Some(key) = key {
        steps.push(Step::KeyPress { key });
    }
    (ExpansionPlan { steps }, template.diagnostics)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(
        body: &str,
        delete_count: u32,
        case: CaseTransform,
        trailing: Option<char>,
    ) -> ExpansionPlan {
        static_plan(StaticExpansion {
            body,
            delete_count,
            case,
            trailing,
        })
        .0
    }

    #[test]
    fn deletes_the_abbreviation_then_inserts_text_and_delimiter() {
        assert_eq!(
            plan("thank you", 2, CaseTransform::Title, Some(' ')).steps,
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
            plan("on my way", 3, CaseTransform::AsDefined, Some('\r')).steps,
            [
                Step::Delete { count: 3 },
                Step::InsertText {
                    text: "on my way".into()
                },
                Step::KeyPress { key: Key::Return },
            ]
        );
        assert_eq!(
            plan("x", 1, CaseTransform::AsDefined, Some('\t')).steps[2],
            Step::KeyPress { key: Key::Tab }
        );
    }

    #[test]
    fn nothing_to_delete_means_no_delete_step() {
        // A one-character immediate abbreviation: the key was swallowed.
        assert_eq!(
            plan("§", 0, CaseTransform::AsDefined, None).steps,
            [Step::InsertText { text: "§".into() }]
        );
    }

    #[test]
    fn escapes_resolve_and_placeholders_stay_verbatim_for_now() {
        let (plan, diagnostics) = static_plan(StaticExpansion {
            body: "\\{{not one}} {{date: yyyy}} {{broken",
            delete_count: 0,
            case: CaseTransform::AsDefined,
            trailing: None,
        });
        assert_eq!(plan.inserted_text(), "{{not one}} {{date: yyyy}} {{broken");
        assert_eq!(diagnostics.len(), 1);
    }
}

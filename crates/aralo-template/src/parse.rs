//! The placeholder grammar, v0 without blocks:
//!
//! ```text
//! body        := (text | placeholder)*
//! placeholder := "{{" name (":" argument)? ("|" option)* "}}"
//! option      := key ":" value
//! escape      := "\{{"  ->  literal "{{"
//! ```
//!
//! The parser never fails. Input it cannot read becomes literal text plus a
//! [`Diagnostic`] with a byte range, and the editor highlights from the same
//! diagnostics, so what the editor shows is what an expansion does.

use std::ops::Range;

use crate::highlight::ProblemLevel;

const OPEN: &str = "{{";
const CLOSE: &str = "}}";

/// A parsed snippet body.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Template {
    pub nodes: Vec<Node>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    /// Literal text, with escapes already resolved.
    Text(String),
    Placeholder(Placeholder),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placeholder {
    /// Lower-case name, e.g. `date`, `field`, `ai`.
    pub name: String,
    pub argument: Option<String>,
    /// `key: value` pairs in source order. A repeated key keeps every value.
    pub options: Vec<(String, String)>,
    /// Byte range of the whole `{{…}}` in the body.
    pub span: Range<usize>,
}

impl Placeholder {
    pub fn option(&self, key: &str) -> Option<&str> {
        self.options
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }
}

/// Something worth saying about one range of a body: from the parser, from
/// resolving the body's nested snippets, or from the expansion itself.
///
/// The editor and an expansion report from the same list, so what the editor
/// underlines is what an expansion does (PRD L10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub kind: DiagnosticKind,
    pub span: Range<usize>,
    /// The name, locale, reference or directive the message names. The kinds
    /// that take one are in the table in `docs/format/placeholders.md`.
    pub detail: Option<String>,
}

impl Diagnostic {
    pub fn new(kind: DiagnosticKind, span: Range<usize>) -> Self {
        Self {
            kind,
            span,
            detail: None,
        }
    }

    /// A diagnostic whose message names something in the body.
    pub fn about(kind: DiagnosticKind, span: Range<usize>, detail: impl Into<String>) -> Self {
        Self {
            kind,
            span,
            detail: Some(detail.into()),
        }
    }

    pub fn level(&self) -> ProblemLevel {
        self.kind.level()
    }

    /// One sentence for an editor to show beside the body.
    pub fn message(&self) -> String {
        self.kind.message(self.detail.as_deref())
    }
}

/// Why a body does not read the way it was written, or why part of it will not
/// expand. The table, with the detail each kind carries, is in
/// `docs/format/placeholders.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagnosticKind {
    /// `{{` with no `}}` after it. Kept as literal text.
    Unclosed,
    /// The name is empty or not `[a-z][a-z0-9_-]*`. Kept as literal text.
    InvalidName,
    /// An option without `key:`. The option is ignored.
    MalformedOption,
    /// A well-formed name the format does not define. Detail: the name.
    UnknownName,
    /// A name Aralo will expand in a later release. Detail: the name.
    NotImplemented,
    /// No model answered an AI block, so it put in its `fallback:` text.
    /// Detail: why, such as "AI is switched off".
    AiFallback,
    /// No model answered an AI block, and it has no fallback text, so it put
    /// in nothing. Detail: why.
    AiNothing,
    /// An AI block with no `fallback:`, which puts in nothing when no model
    /// answers.
    AiNoFallback,
    /// An AI block with nothing to ask. It is never run, and puts in its
    /// fallback.
    MissingPrompt,
    /// A placeholder that needs a name after the colon has none. Detail: the
    /// placeholder's name.
    MissingName,
    /// A `{{choice}}` with nothing to choose from. Detail: the field's name.
    MissingOptions,
    /// An option whose value Aralo cannot read, such as `| lines: plenty`. The
    /// option is ignored and the placeholder stays. Detail: the option's key.
    BadOption,
    /// A `{{key}}` naming a key Aralo cannot press. Detail: what was written,
    /// or none when the body wrote no key at all.
    UnknownKey,
    /// A date or time format Aralo cannot write. Detail: the directive.
    BadFormat,
    /// A locale Aralo does not carry. Detail: the tag that was asked for.
    UnknownLocale,
    /// Nothing supplied a value the body asks for, such as the clipboard.
    /// Detail: what was missing.
    NothingSupplied,
    /// `{{snippet}}` naming nothing in the library. Detail: the reference.
    SnippetMissing,
    /// A snippet that reaches itself. Detail: the reference.
    SnippetCycle,
    /// Nesting past [`crate::MAX_SNIPPET_DEPTH`]. Detail: the reference.
    SnippetTooDeep,
    /// A second cursor stop, which is ignored.
    ExtraCursor,
    /// One `{{cursor: select}}`, which needs a pair to select anything.
    LoneSelection,
}

impl DiagnosticKind {
    /// Whether the body still reads the way it was written.
    pub fn level(self) -> ProblemLevel {
        match self {
            Self::Unclosed
            | Self::InvalidName
            | Self::MalformedOption
            | Self::MissingName
            | Self::MissingOptions
            | Self::MissingPrompt
            | Self::BadFormat
            | Self::SnippetMissing
            | Self::SnippetCycle
            | Self::SnippetTooDeep => ProblemLevel::Error,
            Self::UnknownName
            | Self::NotImplemented
            | Self::AiFallback
            | Self::AiNothing
            | Self::AiNoFallback
            | Self::BadOption
            | Self::UnknownKey
            | Self::UnknownLocale
            | Self::NothingSupplied
            | Self::ExtraCursor
            | Self::LoneSelection => ProblemLevel::Note,
        }
    }

    /// One sentence for an editor to show beside the body. The words are the
    /// core's, so every shell says the same thing about the same body.
    ///
    /// `detail` is what the body named: see the variants for which kinds carry
    /// one. A missing detail leaves the sentence readable rather than empty.
    pub fn message(self, detail: Option<&str>) -> String {
        let it = detail.unwrap_or("this");
        match self {
            Self::Unclosed => {
                "This {{ has no }} after it, so the rest of the body is literal text.".to_owned()
            }
            Self::InvalidName => "A placeholder name starts with a lower-case letter and holds \
                 only letters, digits, - and _. This one is literal text."
                .to_owned(),
            Self::MalformedOption => {
                "An option is written key: value. This one is ignored, and the placeholder stays."
                    .to_owned()
            }
            Self::UnknownName => {
                format!(
                    "No placeholder is called {it}. The insert menu lists the ones Aralo knows."
                )
            }
            Self::NotImplemented => {
                format!("Aralo does not expand {it} yet, so it stays as written.")
            }
            Self::AiFallback => match detail {
                Some(reason) => {
                    format!("No model answered this block ({reason}), so its fallback went in.")
                }
                None => "No model answered this block, so its fallback went in.".to_owned(),
            },
            Self::AiNothing => match detail {
                Some(reason) => format!(
                    "No model answered this block ({reason}), and it has no fallback text, so \
                     nothing went in."
                ),
                None => "No model answered this block, and it has no fallback text, so nothing \
                     went in."
                    .to_owned(),
            },
            Self::AiNoFallback => "With AI off, or no network, this block puts in nothing. \
                 Write | fallback: text to say what goes in instead."
                .to_owned(),
            Self::MissingPrompt => "Write what the model is to write: {{ai: a short thank-you}}. \
                 Until then this block puts in its fallback."
                .to_owned(),
            Self::MissingName => {
                format!("Write {{{{{it}: a-name}}}}: without a name there is no answer to put in.")
            }
            Self::MissingOptions => {
                format!("{it} has nothing to choose from. Write | options: a, b, c.")
            }
            Self::BadOption => {
                format!("Aralo cannot read the {it} option here, so it is left out.")
            }
            Self::UnknownKey => match detail {
                Some(written) => format!(
                    "Aralo can press Tab and Return, not {written}, so this stays as written."
                ),
                None => "Write {{key: tab}} or {{key: return}}: those are the keys Aralo can \
                     press."
                    .to_owned(),
            },
            Self::BadFormat => {
                format!(
                    "Aralo cannot write {it} in a date or time. The formats it knows are in \
                     the placeholder help."
                )
            }
            Self::UnknownLocale => {
                format!("Aralo does not carry the locale {it}, so this reads in English.")
            }
            Self::NothingSupplied => {
                format!("Nothing supplied the {it}, so this stays as written.")
            }
            Self::SnippetMissing => {
                format!("No snippet in the library answers to {it}.")
            }
            Self::SnippetCycle => {
                format!("{it} reaches this snippet again, so it is left as written.")
            }
            Self::SnippetTooDeep => {
                format!(
                    "Snippets are nested {} deep before {it}, which is as far as Aralo goes.",
                    crate::MAX_SNIPPET_DEPTH
                )
            }
            Self::ExtraCursor => {
                "An expansion leaves the cursor in one place. This stop is ignored.".to_owned()
            }
            Self::LoneSelection => {
                "Selecting text needs two {{cursor: select}}. This one just leaves the cursor \
                 here."
                    .to_owned()
            }
        }
    }
}

impl Template {
    /// True when the body is literal text only.
    pub fn is_static(&self) -> bool {
        self.nodes.iter().all(|node| matches!(node, Node::Text(_)))
    }

    pub fn placeholders(&self) -> impl Iterator<Item = &Placeholder> {
        self.nodes.iter().filter_map(|node| match node {
            Node::Placeholder(placeholder) => Some(placeholder),
            Node::Text(_) => None,
        })
    }
}

pub fn parse(body: &str) -> Template {
    let mut template = Template::default();
    let mut text = String::new();
    let mut at = 0;

    while at < body.len() {
        let rest = &body[at..];
        if rest.starts_with("\\{{") {
            text.push_str(OPEN);
            at += 3;
        } else if let Some(after_open) = rest.strip_prefix(OPEN) {
            let Some(length) = after_open.find(CLOSE) else {
                template
                    .diagnostics
                    .push(Diagnostic::new(DiagnosticKind::Unclosed, at..body.len()));
                text.push_str(rest);
                break;
            };
            let end = at + OPEN.len() + length + CLOSE.len();
            let inner = &after_open[..length];
            match placeholder(inner, at..end, &mut template.diagnostics) {
                Some(found) => {
                    if !text.is_empty() {
                        template.nodes.push(Node::Text(std::mem::take(&mut text)));
                    }
                    template.nodes.push(Node::Placeholder(found));
                }
                None => text.push_str(&body[at..end]),
            }
            at = end;
        } else {
            let next = rest.chars().next().map_or(1, char::len_utf8);
            text.push_str(&rest[..next]);
            at += next;
        }
    }
    if !text.is_empty() {
        template.nodes.push(Node::Text(text));
    }
    template
}

fn placeholder(
    inner: &str,
    span: Range<usize>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<Placeholder> {
    let mut parts = split_unescaped_pipes(inner).into_iter();
    let head = parts.next().unwrap_or_default();
    let (name, argument) = match head.split_once(':') {
        Some((name, argument)) => (name.trim(), Some(argument.trim().to_owned())),
        None => (head.trim(), None),
    };
    if !is_name(name) {
        diagnostics.push(Diagnostic::new(DiagnosticKind::InvalidName, span));
        return None;
    }

    let mut options = Vec::new();
    for part in parts {
        match part.split_once(':') {
            Some((key, value)) if is_name(key.trim()) => {
                options.push((key.trim().to_owned(), value.trim().to_owned()));
            }
            _ => diagnostics.push(Diagnostic::new(
                DiagnosticKind::MalformedOption,
                span.clone(),
            )),
        }
    }
    Some(Placeholder {
        name: name.to_owned(),
        argument: argument.filter(|a| !a.is_empty()),
        options,
        span,
    })
}

fn is_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// Splits on `|`; `\|` is a literal pipe, so an AI prompt can contain one.
fn split_unescaped_pipes(inner: &str) -> Vec<String> {
    let mut parts = vec![String::new()];
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&'|') => {
                chars.next();
                parts.last_mut().expect("never empty").push('|');
            }
            '|' => parts.push(String::new()),
            other => parts.last_mut().expect("never empty").push(other),
        }
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Node {
        Node::Text(s.to_owned())
    }

    #[test]
    fn plain_text_is_one_static_node() {
        let template = parse("Best regards,\nSam — 100% {literal} }} braces");
        assert!(template.is_static());
        assert!(template.diagnostics.is_empty());
        assert_eq!(
            template.nodes,
            [text("Best regards,\nSam — 100% {literal} }} braces")]
        );
        assert!(parse("").nodes.is_empty());
    }

    #[test]
    fn parses_name_argument_and_options() {
        let body = "Hi {{field: name | default: there}}, {{clipboard}}";
        let template = parse(body);
        assert!(template.diagnostics.is_empty());
        let found: Vec<_> = template.placeholders().collect();
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].name, "field");
        assert_eq!(found[0].argument.as_deref(), Some("name"));
        assert_eq!(found[0].option("default"), Some("there"));
        assert_eq!(
            &body[found[0].span.clone()],
            "{{field: name | default: there}}"
        );
        assert_eq!(found[1].name, "clipboard");
        assert_eq!(found[1].argument, None);
        assert_eq!(template.nodes[0], text("Hi "));
        assert_eq!(template.nodes[2], text(", "));
    }

    #[test]
    fn the_argument_may_contain_colons_and_escaped_pipes() {
        let template = parse("{{ai: Rewrite as a list: a \\| b | fallback: n/a | model: x:y}}");
        let ai = template.placeholders().next().unwrap();
        assert_eq!(ai.argument.as_deref(), Some("Rewrite as a list: a | b"));
        assert_eq!(ai.option("fallback"), Some("n/a"));
        assert_eq!(ai.option("model"), Some("x:y"));
    }

    #[test]
    fn an_escaped_opening_is_literal() {
        let template = parse("Use \\{{date}} for dates");
        assert!(template.is_static());
        assert_eq!(template.nodes, [text("Use {{date}} for dates")]);
    }

    #[test]
    fn unclosed_braces_become_text_with_a_diagnostic() {
        let body = "before {{date: yyyy";
        let template = parse(body);
        assert_eq!(template.nodes, [text(body)]);
        assert_eq!(
            template.diagnostics,
            [Diagnostic::new(DiagnosticKind::Unclosed, 7..body.len())]
        );
    }

    #[test]
    fn invalid_names_become_text_with_a_diagnostic() {
        for body in ["{{}}", "{{ }}", "{{Date}}", "{{1st}}", "{{two words}}"] {
            let template = parse(body);
            assert_eq!(template.nodes, [text(body)], "{body}");
            assert_eq!(template.diagnostics[0].kind, DiagnosticKind::InvalidName);
            assert_eq!(template.diagnostics[0].span, 0..body.len());
        }
    }

    #[test]
    fn a_malformed_option_is_dropped_but_the_placeholder_stays() {
        let template = parse("{{date: yyyy | oops | locale: de}}");
        let date = template.placeholders().next().unwrap();
        assert_eq!(date.options, [("locale".to_owned(), "de".to_owned())]);
        assert_eq!(
            template.diagnostics[0].kind,
            DiagnosticKind::MalformedOption
        );
    }

    #[test]
    fn multibyte_text_keeps_byte_spans_valid() {
        let body = "grüße 👋 {{cursor}} später";
        let template = parse(body);
        let cursor = template.placeholders().next().unwrap();
        assert_eq!(&body[cursor.span.clone()], "{{cursor}}");
    }
}

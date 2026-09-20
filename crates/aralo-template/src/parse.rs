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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub kind: DiagnosticKind,
    pub span: Range<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagnosticKind {
    /// `{{` with no `}}` after it. Kept as literal text.
    Unclosed,
    /// The name is empty or not `[a-z][a-z0-9_-]*`. Kept as literal text.
    InvalidName,
    /// An option without `key:`. The option is ignored.
    MalformedOption,
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
                template.diagnostics.push(Diagnostic {
                    kind: DiagnosticKind::Unclosed,
                    span: at..body.len(),
                });
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
        diagnostics.push(Diagnostic {
            kind: DiagnosticKind::InvalidName,
            span,
        });
        return None;
    }

    let mut options = Vec::new();
    for part in parts {
        match part.split_once(':') {
            Some((key, value)) if is_name(key.trim()) => {
                options.push((key.trim().to_owned(), value.trim().to_owned()));
            }
            _ => diagnostics.push(Diagnostic {
                kind: DiagnosticKind::MalformedOption,
                span: span.clone(),
            }),
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
            [Diagnostic {
                kind: DiagnosticKind::Unclosed,
                span: 7..body.len()
            }]
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

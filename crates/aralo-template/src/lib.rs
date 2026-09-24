//! Snippet bodies: the placeholder grammar, rendering and the [`ExpansionPlan`]
//! a shell's injector executes.
//!
//! The crate is pure: no files, no network, no clock of its own. Anything a
//! placeholder needs from the outside world (time, clipboard, form answers)
//! arrives as an argument. That keeps expansion testable with fixed inputs and
//! lets the browser extension run the same code as WebAssembly.
//!
//! State of the implementation: the parser covers the v0 grammar without
//! `{{if}}` blocks. The evaluator expands dates, times, the clipboard, form
//! fields, nested snippets, cursor stops and AI blocks. It asks no model
//! itself: a session collects what the model wrote and hands it in as an
//! [`AiAnswer`], which goes into the text as it is. A placeholder Aralo
//! cannot expand stays as written, with a diagnostic. [`outline`] turns a body
//! into what an editor draws over it, from the same reading the expansion
//! does.

mod case;
mod datetime;
mod eval;
mod highlight;
mod parse;
mod plan;
mod render;

pub use case::CaseTransform;
pub use datetime::{default_locale, format_time, locale, locales, BadDirective, CivilTime, Locale};
pub use eval::{
    render, resolve, AiAnswer, AiAnswers, AiBlock, AiSpan, Answers, Context, ContextKind,
    ContextValues, Cursor, FieldKind, Form, FormField, Nested, Rendered, Resolved, Snippets,
    MAX_FIELD_LINES, MAX_SNIPPET_DEPTH,
};
pub use highlight::{
    catalogue, known, outline, outline_with, BodyOutline, BodyPlaceholder, BodyProblem, BodyRange,
    PlaceholderInfo, ProblemLevel,
};
pub use parse::{parse, Diagnostic, DiagnosticKind, Node, Placeholder, Template};
pub use plan::{ExpansionPlan, Key, Step};
pub use render::{expand, finish, plan, Shape};

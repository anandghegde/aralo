//! Snippet bodies: the placeholder grammar, rendering and the [`ExpansionPlan`]
//! a shell's injector executes.
//!
//! The crate is pure: no files, no network, no clock of its own. Anything a
//! placeholder needs from the outside world (time, clipboard, form answers)
//! arrives as an argument. That keeps expansion testable with fixed inputs and
//! lets the browser extension run the same code as WebAssembly.
//!
//! State of the implementation: the parser covers the v0 grammar without
//! `{{if}}` blocks. The renderer expands literal text and escapes only;
//! placeholders are left in place with a diagnostic until the evaluator lands
//! in M3.

mod case;
mod parse;
mod plan;
mod render;

pub use case::CaseTransform;
pub use parse::{parse, Diagnostic, DiagnosticKind, Node, Placeholder, Template};
pub use plan::{ExpansionPlan, Key, Step};
pub use render::{static_plan, StaticExpansion};

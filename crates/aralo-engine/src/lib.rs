//! Aralo's keystroke matcher.
//!
//! This is the only Rust code that sees what the user types. It is deliberately
//! small so that "Aralo is not a keylogger" can be checked by reading it:
//!
//! * Typed characters live in one fixed [`CAPACITY`]-character ring buffer,
//!   in memory only, zeroed on every [`Engine::reset`] and on drop.
//! * The crate is `no_std`. It cannot name a file, a socket, a clock or a
//!   logger, and CI rejects any dependency outside a short allow-list.
//! * It holds abbreviations, flags and snippet IDs. It never holds snippet
//!   bodies.
//!
//! The shell feeds [`KeyEvent`]s to [`Engine::on_key`] from the event-tap
//! thread and acts on the [`KeyVerdict`]. The library publishes a new immutable
//! [`Snapshot`] whenever snippets change and hands it over with
//! [`Engine::set_snapshot`].
//!
//! Matching rules (PRD E1–E3, E8, E10) are documented on [`Engine::on_key`].

#![no_std]

extern crate alloc;
#[cfg(test)]
extern crate std;

mod buffer;
mod case;
mod engine;
mod presets;
mod snapshot;

pub use buffer::CAPACITY;
pub use case::CasePattern;
pub use engine::{
    Engine, ExpansionRecord, InsertMethod, InsertRefusal, KeyEvent, KeyVerdict, ResetReason,
};
pub use presets::EXCLUDED_APP_PRESETS;
pub use snapshot::{
    Abbreviation, CaseMode, Rejection, RejectionReason, Scope, Snapshot, SnapshotBuilder,
    SnippetId, Trigger, DEFAULT_DELIMITERS, MAX_ABBREVIATION_CHARS,
};

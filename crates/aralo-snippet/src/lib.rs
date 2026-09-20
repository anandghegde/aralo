//! Aralo's on-disk data model (format v0, see `docs/format/`).
//!
//! A library is a folder. `aralo.yaml` is its [`Manifest`], every sub-folder is
//! a group described by an optional `_group.yaml` ([`GroupFile`]), and every
//! `.md` file is one snippet ([`SnippetFile`]): YAML front matter plus a body.
//!
//! This crate only parses and writes those files. It does not touch the file
//! system, resolve inheritance or know about the engine; `aralo-library` does.
//!
//! Every file type keeps the keys it does not understand in `extra` and writes
//! them back, so a newer Aralo and an older one can share a folder.

mod error;
mod group;
mod id;
mod manifest;
mod snippet;

pub use error::ParseError;
pub use group::{Defaults, GroupFile, ScopeSpec, GROUP_FILE_NAME};
pub use id::{InvalidId, SnippetId};
pub use manifest::{Manifest, FORMAT_VERSION, MANIFEST_FILE_NAME};
pub use snippet::{
    AiSettings, CaseMode, FrontMatter, SnippetFile, SnippetKind, TriggerMode, SNIPPET_EXTENSION,
};

/// Keys this version does not know, kept verbatim for the next save.
pub type Extra = std::collections::BTreeMap<String, serde_yaml_ng::Value>;

//! The facade the shells talk to.
//!
//! This is the M0/M1 slice: open a library folder, hand the engine its
//! snapshot, and turn a match into an [`ExpansionPlan`]. Expansion sessions
//! (forms, AI, scripts), settings and events arrive with the milestones that
//! need them.

mod simulate;
mod starter;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use aralo_engine::{CasePattern, Engine, Snapshot};
use aralo_library::{Library, LibraryError, LoadedSnippet};
use aralo_template::{static_plan, CaseTransform, StaticExpansion};

pub use aralo_engine as engine;
pub use aralo_library::{Diagnostic, Issue, Settings};
pub use aralo_snippet as snippet;
pub use aralo_template::{ExpansionPlan, Key, Step};
pub use simulate::Simulator;
pub use starter::FILES as STARTER_FILES;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error(transparent)]
    Library(#[from] LibraryError),
    #[error("cannot write the starter snippets to {path}: {source}")]
    Starter {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// What the engine reported about a match; the fields of
/// [`aralo_engine::KeyVerdict::Match`] that shape the expansion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatchInfo {
    pub snippet_id: aralo_engine::SnippetId,
    pub delete_count: u32,
    pub case: CasePattern,
    pub trailing: Option<char>,
}

/// A plan, plus what the shell reports back once it has run it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expansion {
    pub plan: ExpansionPlan,
    /// Backspaces that undo the insertion, or `None` when undo must not be
    /// offered (see [`ExpansionPlan::undo_delete_count`]).
    pub undo_delete_count: Option<u32>,
    /// Problems in the snippet's placeholders. The plan is still usable.
    pub template_diagnostics: Vec<aralo_template::Diagnostic>,
}

/// An open library and the engine snapshot built from it.
#[derive(Debug)]
pub struct Core {
    library: Library,
    snapshot: Arc<Snapshot>,
    rejected: Vec<Diagnostic>,
    starter_files_written: usize,
}

impl Core {
    /// Opens the library at `root`, creating the folder and its manifest when
    /// they are missing. A library that is new and has no snippets gets the
    /// starter snippets; nothing the user already has is ever replaced.
    pub fn open(root: &Path) -> Result<Self, CoreError> {
        let is_new = Library::create(root, None)?;
        let mut library = Library::load(root)?;
        let mut starter_files_written = 0;
        if is_new && library.snippets().is_empty() {
            starter_files_written =
                starter::install(root).map_err(|source| CoreError::Starter {
                    path: root.to_owned(),
                    source,
                })?;
            library = Library::load(root)?;
        }
        Ok(Self::with_library(library, starter_files_written))
    }

    /// Opens a folder without writing anything to it, for validation tools.
    pub fn open_read_only(root: &Path) -> Result<Self, CoreError> {
        Ok(Self::with_library(Library::load(root)?, 0))
    }

    fn with_library(library: Library, starter_files_written: usize) -> Self {
        let (snapshot, rejected) = library.snapshot();
        Self {
            library,
            snapshot: Arc::new(snapshot),
            rejected,
            starter_files_written,
        }
    }

    /// Reads the folder again. On failure the old library stays in use.
    pub fn reload(&mut self) -> Result<(), CoreError> {
        let library = Library::load(self.library.root())?;
        *self = Self::with_library(library, self.starter_files_written);
        Ok(())
    }

    pub fn root(&self) -> &Path {
        self.library.root()
    }

    pub fn library(&self) -> &Library {
        &self.library
    }

    pub fn snippets(&self) -> &[LoadedSnippet] {
        self.library.snippets()
    }

    /// Starter files written when this library was opened for the first time.
    pub fn starter_files_written(&self) -> usize {
        self.starter_files_written
    }

    /// The snapshot to give [`Engine::set_snapshot`] after `open` and `reload`.
    pub fn snapshot(&self) -> Arc<Snapshot> {
        Arc::clone(&self.snapshot)
    }

    /// A new engine that already has this library's snapshot.
    pub fn engine(&self) -> Engine {
        let mut engine = Engine::new();
        engine.set_snapshot(self.snapshot());
        engine
    }

    /// Everything in the folder that needs attention: files that did not load
    /// and abbreviations the engine refused.
    pub fn diagnostics(&self) -> impl Iterator<Item = &Diagnostic> {
        self.library.diagnostics().iter().chain(&self.rejected)
    }

    /// The plan for a match the engine reported. `None` when the snippet has
    /// gone since the snapshot was built; the shell then lets the key through.
    pub fn expand(&self, matched: MatchInfo) -> Option<Expansion> {
        let id = aralo_snippet::SnippetId::from_u128(matched.snippet_id.0);
        let snippet = self.library.snippet(id)?;
        let (plan, template_diagnostics) = static_plan(StaticExpansion {
            body: &snippet.file.body,
            delete_count: matched.delete_count,
            case: match matched.case {
                CasePattern::AsDefined => CaseTransform::AsDefined,
                CasePattern::Title => CaseTransform::Title,
                CasePattern::Upper => CaseTransform::Upper,
            },
            trailing: matched.trailing,
        });
        Some(Expansion {
            undo_delete_count: plan.undo_delete_count(),
            plan,
            template_diagnostics,
        })
    }
}

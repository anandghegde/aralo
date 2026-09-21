//! The facade the shells talk to.
//!
//! This is the M0/M1 slice: open a library folder, hand the engine its
//! snapshot, and turn a match into an [`ExpansionPlan`]. Expansion sessions
//! (forms, AI, scripts), settings and events arrive with the milestones that
//! need them.

pub mod compat;
mod edit;
mod runtime;
mod simulate;
mod starter;
pub mod state;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use aralo_engine::{CasePattern, Engine, Snapshot};
use aralo_library::{Library, LibraryError, Searcher};
use aralo_template::{static_plan, CaseTransform, StaticExpansion};

pub use aralo_engine as engine;
pub use aralo_import as import;
pub use aralo_import::{
    ExportOptions, Format, ImportOptions, ImportReport, MacroPolicy, Outcome, SnippetRecord,
};
pub use aralo_library::{
    Diagnostic, Field, Issue, LoadedGroup, LoadedSnippet, Query, Recent, Settings, Stats,
};
pub use aralo_snippet as snippet;
pub use aralo_template::{ExpansionPlan, Key, Step};
pub use compat::{CompatError, CompatTable, InjectionProfile};
pub use edit::{Draft, DraftIssue, Problem};
pub use runtime::{LibraryChange, LibraryListener, Runtime, RuntimeOptions};
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
    #[error(transparent)]
    Import(#[from] aralo_import::ImportError),
    #[error("no snippet with the id {0} is in the library")]
    NoSuchSnippet(String),
    #[error("no group at {0} is in the library")]
    NoSuchGroup(String),
    #[error("{name:?} cannot be a folder name: {reason}")]
    InvalidName { name: String, reason: String },
    #[error(transparent)]
    Index(#[from] aralo_library::IndexError),
    #[error(transparent)]
    Watch(#[from] aralo_library::WatchError),
    #[error("cannot start the indexing thread: {source}")]
    Thread { source: std::io::Error },
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

/// One snippet a search found: where it matched, and what a list needs to
/// draw the row without looking anything else up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    pub id: aralo_snippet::SnippetId,
    /// Path relative to the library root.
    pub path: PathBuf,
    /// What to call it in a list: the label, an abbreviation, or the first
    /// line of the body.
    pub name: String,
    pub abbr: Vec<String>,
    /// Folder names from the root down to the snippet's group.
    pub group: Vec<String>,
    pub enabled: bool,
    /// Which field the query matched.
    pub field: Field,
    /// The text that matched, and the character positions in it that the
    /// query used, for an editor to highlight.
    pub text: String,
    pub matched: Vec<u32>,
}

/// An open library and the engine snapshot built from it.
#[derive(Debug)]
pub struct Core {
    library: Library,
    snapshot: Arc<Snapshot>,
    rejected: Vec<Diagnostic>,
    starter_files_written: usize,
    /// The fuzzy matcher's scratch buffers, kept between searches because the
    /// editor searches on every keystroke.
    searcher: Mutex<Searcher>,
}

impl Core {
    /// Opens the library at `root`, creating the folder and its manifest when
    /// they are missing. A library that is new and has no snippets gets the
    /// starter snippets; nothing the user already has is ever replaced.
    pub fn open(root: &Path) -> Result<Self, CoreError> {
        Self::open_inner(root, true)
    }

    /// The same, without the starter snippets, for a library that is about to
    /// be filled from somewhere else. An import brings its own snippets, and
    /// the starter set would collide with them.
    pub fn open_without_starter(root: &Path) -> Result<Self, CoreError> {
        Self::open_inner(root, false)
    }

    fn open_inner(root: &Path, starter: bool) -> Result<Self, CoreError> {
        let is_new = Library::create(root, None)?;
        let mut library = Library::load(root)?;
        let mut starter_files_written = 0;
        if starter && is_new && library.snippets().is_empty() {
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
            searcher: Mutex::new(Searcher::new()),
        }
    }

    /// Reads the folder again. On failure the old library stays in use.
    ///
    /// The record of Aralo's own writes carries across, so a save that has not
    /// yet reached the file system watcher is still recognised as Aralo's after
    /// the reload rather than being taken for someone else's edit.
    pub fn reload(&mut self) -> Result<(), CoreError> {
        let library = self.library.reload()?;
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

    /// Reads `source` into the library and reloads, so the imported snippets
    /// expand straight away. A dry run reads the source, reports what it
    /// would do and leaves the folder alone.
    ///
    /// The report is the answer, not a side effect: nothing here fails
    /// because one snippet did not convert, and the caller shows the report.
    pub fn import(
        &mut self,
        source: &Path,
        options: &ImportOptions,
    ) -> Result<ImportReport, CoreError> {
        let report = aralo_import::import(source, &self.library, options)?;
        if !options.dry_run {
            self.reload()?;
        }
        Ok(report)
    }

    /// The library, or one group of it, as the bytes of an interchange file.
    pub fn export(&self, options: &ExportOptions) -> Result<Vec<u8>, CoreError> {
        Ok(aralo_import::export(&self.library, options)?)
    }

    /// Every snippet the query matches, best first. An empty query is the
    /// whole library in list order, so an editor's list and its search box are
    /// one call.
    ///
    /// What is searched and how it is ranked is in [`aralo_library::search`].
    pub fn search(&self, query: &Query) -> Vec<SearchHit> {
        // A poisoned lock means a search panicked, which leaves nothing behind
        // but scratch buffers. Taking them back beats failing every search
        // after it, and nothing in the bridge may panic.
        let mut searcher = self
            .searcher
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        searcher
            .search(&self.library, query)
            .into_iter()
            .filter_map(|hit| {
                let snippet = self.library.snippet(hit.id)?;
                Some(SearchHit {
                    id: hit.id,
                    path: snippet.path.clone(),
                    name: snippet.display_name().to_owned(),
                    abbr: snippet.file.front.abbr.clone(),
                    group: snippet.group.clone(),
                    enabled: snippet.settings.enabled,
                    field: hit.field,
                    text: hit.text,
                    matched: hit.matched,
                })
            })
            .collect()
    }

    /// What a snippet expands to, for an editor that shows the result beside
    /// the body. `None` when there is no such snippet.
    ///
    /// It is the expansion path, not a second reading of the template: what
    /// this shows is what typing the abbreviation would produce, minus the
    /// cursor and the keys. A snippet whose template is wrong previews as far
    /// as it parses, which is what makes the preview useful while typing it.
    pub fn preview(&self, id: aralo_snippet::SnippetId) -> Option<String> {
        let snippet = self.library.snippet(id)?;
        Some(Self::preview_body(&snippet.file.body))
    }

    /// The same for a body that is still being typed, which is what an editor
    /// wants: the library is not consulted, so the preview keeps up with the
    /// keystroke rather than with the last save.
    pub fn preview_body(body: &str) -> String {
        let (plan, _) = static_plan(StaticExpansion {
            body,
            delete_count: 0,
            case: CaseTransform::AsDefined,
            trailing: None,
        });
        plan.inserted_text()
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

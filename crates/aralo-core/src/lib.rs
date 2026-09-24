//! The facade the shells talk to.
//!
//! Open a library folder, hand the engine its snapshot, and turn a match into
//! an [`ExpansionPlan`]. A body that needs nothing but the clock expands on
//! the keystroke; one that needs a form filled in, the clipboard read or a
//! model asked opens a [`Session`] the shell drives. Scripts arrive with the
//! milestone that needs them.
//!
//! This is also where the clock is. `aralo-template` formats a moment it is
//! handed, so every expansion takes the time from the [`Clock`] on the core,
//! and a test can stop it.

pub mod ai;
pub mod clock;
pub mod compat;
pub mod diff;
mod edit;
mod field;
mod merge;
mod runtime;
mod session;
mod simulate;
mod starter;
pub mod state;
mod trial;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use aralo_engine::{Engine, Snapshot};
use aralo_library::{Library, LibraryError, Searcher};
use aralo_template::{Context, Nested, Resolved, Shape, Snippets};

use crate::clock::{Clock, SystemClock};

pub use aralo_engine as engine;
pub use aralo_import as import;
pub use aralo_import::{
    ExportOptions, Format, ImportOptions, ImportReport, MacroPolicy, Outcome, SnippetRecord,
};
pub use aralo_library::{
    Conflict, Diagnostic, Field, Issue, LoadedGroup, LoadedSnippet, Query, Recent, Settings, Stats,
};
pub use aralo_snippet as snippet;
pub use aralo_template as template;
pub use aralo_template::{
    Answers, BodyOutline, BodyPlaceholder, BodyProblem, BodyRange, CivilTime, ContextKind,
    ContextValues, ExpansionPlan, FieldKind, Form, FormField, Key, PlaceholderInfo, ProblemLevel,
    Step,
};
pub use clock::{FixedClock, SystemClock as SystemTimeClock};
pub use compat::{CompatError, CompatTable, InjectionProfile};
pub use edit::{Draft, DraftIssue, Problem};
pub use merge::{ConflictSides, Discard, MergeReport, Resolution, SetAside};
pub use runtime::{LibraryChange, LibraryListener, Runtime, RuntimeOptions};
pub use session::{Expand, Session, SessionStep};
pub use simulate::Simulator;
pub use starter::FILES as STARTER_FILES;
pub use trial::{Trial, TrialKey};

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
    #[error("no conflict copy at {0} is waiting in the library")]
    NoSuchConflict(String),
    #[error("that is not a snippet file: {0}")]
    NotASnippet(String),
    #[error("this machine will not say where a discarded conflict copy can go")]
    NowhereToDiscard,
    #[error("cannot read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
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
    pub case: aralo_engine::CasePattern,
    pub trailing: Option<char>,
    /// The key the engine swallowed to make the match, when it swallowed one.
    /// A session that is cancelled puts it back; an expansion that runs
    /// replaces it. `None` when the key reached the app.
    pub swallowed: Option<char>,
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
    /// Where `{{date}}` and `{{time}}` read the moment.
    clock: Arc<dyn Clock>,
    /// The locale tag they are written in, for example `de_DE`.
    locale: String,
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
            clock: Arc::new(SystemClock),
            locale: clock::environment_locale(),
        }
    }

    /// Reads the time from `clock` instead of the machine's. For tests and
    /// the golden files, where the answer has to be the same tomorrow.
    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Writes dates in `tag`, for example `de_DE` or `ja-JP`. An unknown tag
    /// falls back to the nearest language, then to `en_US`; see
    /// [`aralo_template::locale`].
    #[must_use]
    pub fn in_locale(mut self, tag: &str) -> Self {
        self.locale = tag.to_owned();
        self
    }

    /// The same after opening, for a shell that learns the user's locale from
    /// the operating system rather than from the environment.
    pub fn set_locale(&mut self, tag: &str) {
        self.locale = tag.to_owned();
    }

    pub fn locale(&self) -> &str {
        &self.locale
    }

    /// The moment an expansion asked for now would use.
    pub fn now(&self) -> CivilTime {
        self.clock.now()
    }

    /// Reads the folder again. On failure the old library stays in use.
    ///
    /// The record of Aralo's own writes carries across, so a save that has not
    /// yet reached the file system watcher is still recognised as Aralo's after
    /// the reload rather than being taken for someone else's edit.
    pub fn reload(&mut self) -> Result<(), CoreError> {
        let library = self.library.reload()?;
        let clock = Arc::clone(&self.clock);
        let locale = std::mem::take(&mut self.locale);
        *self = Self::with_library(library, self.starter_files_written);
        self.clock = clock;
        self.locale = locale;
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
        Some(self.preview_body(&snippet.file.body))
    }

    /// The same for a body that is still being typed, which is what an editor
    /// wants: the library is read for nested snippets, but the body itself
    /// comes from the caller, so the preview keeps up with the keystroke
    /// rather than with the last save.
    ///
    /// Dates and times are the real ones. A form field shows its default,
    /// since no one has been asked yet, and a placeholder waiting on the
    /// shell — `{{clipboard}}` — stays as written: the preview panel does not
    /// read the clipboard, and an editor that wants the real thing opens a
    /// session (PRD P4).
    pub fn preview_body(&self, body: &str) -> String {
        let resolved = self.resolve(body);
        // The form's defaults, which is what a session starts with: a
        // drop-down stands on its first option, so the preview stands there
        // too rather than showing a gap.
        let answers = resolved.form().defaults();
        aralo_template::render(&resolved, self.context().with_answers(&answers)).text()
    }

    /// What an editor draws over a body being typed: where its placeholders
    /// are, and what is wrong with them. The ranges are UTF-16 code units,
    /// which is what a text view counts in.
    ///
    /// It is the same reading the expansion path runs, against the same
    /// library, so what the editor underlines is what an expansion does, down
    /// to a `{{snippet: …}}` that names nothing (PRD L10).
    pub fn outline_body(&self, body: &str) -> BodyOutline {
        let snippets = self.nested();
        aralo_template::outline_with(body, Some(&snippets))
    }

    /// Every placeholder the format defines, for an editor's insert menu. The
    /// list is the core's, so two shells offer the same placeholders in the
    /// same words.
    pub fn placeholders() -> &'static [PlaceholderInfo] {
        aralo_template::catalogue()
    }

    /// The plan for a match the engine reported, or the session that gets
    /// there. `None` when the snippet has gone since the snapshot was built;
    /// the shell then lets the key through.
    pub fn expand(&self, matched: MatchInfo) -> Option<Expand> {
        self.begin(
            aralo_snippet::SnippetId::from_u128(matched.snippet_id.0),
            session::shape(matched),
            matched.swallowed,
        )
    }

    /// The same for a snippet the user picked from a list rather than typed.
    ///
    /// It is the same expansion an abbreviation produces, with nothing to
    /// delete because nothing was typed, and in the snippet's own case because
    /// there is no typing to take a case from. So a snippet inserts the same
    /// text however it was asked for, and a snippet with a form asks the same
    /// question.
    ///
    /// `None` when the snippet has gone since the list was drawn.
    ///
    /// A `command` snippet is not text to insert: it runs on a selection
    /// ([`ai::Command`]), so asking to insert one is `None` too.
    pub fn insert(&self, id: aralo_snippet::SnippetId) -> Option<Expand> {
        let snippet = self.library.snippet(id)?;
        if snippet.file.front.kind == aralo_snippet::SnippetKind::Command {
            return None;
        }
        self.begin(id, Shape::default(), None)
    }

    /// Reads the body, and either finishes it or opens a session.
    ///
    /// The decision is the body's: a form field, a context placeholder or an
    /// AI block means someone has to be asked, and everything else is already
    /// here.
    fn begin(
        &self,
        id: aralo_snippet::SnippetId,
        shape: Shape,
        swallowed: Option<char>,
    ) -> Option<Expand> {
        let snippet = self.library.snippet(id)?;
        Some(self.begin_body(
            id,
            &snippet.file.body,
            snippet.file.front.ai.as_ref(),
            shape,
            swallowed,
        ))
    }

    /// The same for a body that need not be in the library: a draft in the
    /// editor expands through here exactly as its saved file would.
    ///
    /// `ai` is the snippet's `ai` front matter. Every `{{ai}}` block in the
    /// expansion runs under it, a nested snippet's included: it is the
    /// snippet the user asked for, and the one whose declarations they read.
    fn begin_body(
        &self,
        id: aralo_snippet::SnippetId,
        body: &str,
        ai: Option<&aralo_snippet::AiSettings>,
        shape: Shape,
        swallowed: Option<char>,
    ) -> Expand {
        let resolved = self.resolve(body);
        if resolved.form().fields.is_empty()
            && resolved.needs().is_empty()
            && resolved.ai_blocks().is_empty()
        {
            return Expand::Ready(self.settle(&resolved, shape));
        }
        Expand::Session(Box::new(Session::new(
            id,
            resolved,
            ai::BlockSettings::of(ai),
            shape,
            self.clock.now(),
            self.locale.clone(),
            swallowed,
        )))
    }

    /// The body with its nested snippets pulled in, and everything that can be
    /// said about it before the clock is read.
    fn resolve(&self, body: &str) -> Resolved {
        let snippets = self.nested();
        aralo_template::resolve(body, Some(&snippets))
    }

    /// Everything a body needs from outside itself, except what only the shell
    /// can fetch: that arrives through a [`Session`].
    fn context(&self) -> Context<'_> {
        Context::at(self.clock.now()).in_locale(&self.locale)
    }

    fn nested(&self) -> LibrarySnippets<'_> {
        LibrarySnippets {
            library: &self.library,
        }
    }

    fn settle(&self, resolved: &Resolved, shape: Shape) -> Expansion {
        let (plan, template_diagnostics) = aralo_template::finish(resolved, self.context(), shape);
        Expansion {
            undo_delete_count: plan.undo_delete_count(),
            plan,
            template_diagnostics,
        }
    }
}

/// Where `{{snippet: name-or-id}}` looks.
///
/// A reference is an id, an abbreviation, or the name the snippet goes by in a
/// list, tried in that order: an id is unambiguous, an abbreviation is what the
/// user types, and a name is what they read. Nothing is matched loosely, so
/// renaming a snippet breaks the reference visibly instead of quietly nesting
/// a different one.
///
/// A snippet that is switched off is still nested when another one names it.
/// Being switched off means it does not expand on its own; the body that names
/// it asked for it.
struct LibrarySnippets<'a> {
    library: &'a Library,
}

impl Snippets for LibrarySnippets<'_> {
    fn body(&self, reference: &str) -> Option<Nested> {
        let wanted = reference.trim();
        if wanted.is_empty() {
            return None;
        }
        let by_id = wanted
            .parse::<aralo_snippet::SnippetId>()
            .ok()
            .and_then(|id| self.library.snippet(id));
        let found = by_id
            .or_else(|| {
                self.library
                    .snippets()
                    .iter()
                    .find(|snippet| snippet.file.front.abbr.iter().any(|abbr| abbr == wanted))
            })
            .or_else(|| {
                self.library
                    .snippets()
                    .iter()
                    .find(|snippet| snippet.display_name() == wanted)
            })?;
        Some(Nested {
            // The id, whichever name the body used, so two references to one
            // snippet are one snippet when a cycle is looked for.
            key: found.id.to_string(),
            body: found.file.body.clone(),
        })
    }
}

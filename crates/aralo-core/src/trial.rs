//! Trying a snippet before it is saved.
//!
//! The editor's test field is a [`Trial`]: a text field in memory with an
//! engine of its own that knows one snippet, the draft on screen. Typing into
//! it runs the same matcher and the same expansion path as typing into any
//! app, so a user can find out what an abbreviation does, and whether it
//! fires at all, without saving the file or leaving the window.
//!
//! Nothing here touches the library. The draft is read as it stands, against
//! the settings of the group it would be saved in, and a nested
//! `{{snippet: …}}` is looked up in the library as it is on disk.

use aralo_engine::{
    Abbreviation, Engine, ExpansionRecord, InsertMethod, KeyEvent, KeyVerdict, ResetReason, Scope,
    SnapshotBuilder,
};
use aralo_library::Settings;
use aralo_snippet::{FrontMatter, SnippetId};
use aralo_template::ExpansionPlan;

use crate::field::Field;
use crate::{session, Core, Draft, Expand, Expansion, MatchInfo, Session};

/// What a key typed into a [`Trial`] did.
#[derive(Debug)]
pub enum TrialKey {
    /// Nothing matched. The key went into the field, or Backspace took a
    /// character out of it.
    Typed,
    /// The draft expanded, or an expansion was taken back.
    Expanded,
    /// The draft asks something first. Drive the session as for any other
    /// expansion, then hand what it comes to back to [`Trial::finish`], or
    /// its cancel plan to [`Trial::put_back`]. The field is as it was before
    /// the key, whose character the session holds.
    Session(Box<Session>),
}

/// A text field in memory that expands one draft.
pub struct Trial {
    engine: Engine,
    field: Field,
    id: SnippetId,
    body: String,
    /// The `ai` front matter of the snippet being edited, which the draft on
    /// screen does not carry: its AI blocks run as the saved file's would.
    ai: Option<aralo_snippet::AiSettings>,
    /// Abbreviations the engine took. Zero means nothing typed here can
    /// expand, which the editor says rather than leaving the user typing.
    abbreviations: usize,
}

// Like the engine, never prints what was typed.
impl std::fmt::Debug for Trial {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Trial")
            .field("id", &self.id)
            .field("field", &self.field)
            .field("abbreviations", &self.abbreviations)
            .finish_non_exhaustive()
    }
}

impl Core {
    /// A test field for `draft`, as if it were saved in `group`.
    ///
    /// `editing` is the snippet the draft is of, `None` for one not yet
    /// written. The settings are the group's with the draft's own on top,
    /// which is what a save would give it.
    ///
    /// Two things are left out on purpose. A draft that is switched off still
    /// expands here, since this is where a user checks one before switching it
    /// on; the editor already says that it will not expand elsewhere. And the
    /// apps a group is limited to do not apply: the field is in Aralo.
    pub fn try_draft(&self, draft: &Draft, group: &[String], editing: Option<SnippetId>) -> Trial {
        let inherited = self
            .library
            .group(group)
            .or_else(|| self.library.group(&[]))
            .map_or_else(Settings::default, |group| group.settings.clone());
        let mut front = FrontMatter::default();
        draft.apply(&mut front);
        let settings = inherited.for_snippet(&front);

        let id = editing.unwrap_or_else(SnippetId::generate);
        let mut builder = SnapshotBuilder::new();
        for text in &draft.abbr {
            builder.add(Abbreviation {
                snippet_id: aralo_engine::SnippetId(id.as_u128()),
                text: text.clone(),
                trigger: settings.trigger,
                case: settings.case,
                whole_word: settings.whole_word,
                keep_delimiter: settings.keep_delimiter,
                delimiters: settings.delimiters.clone(),
                scope: Scope::Everywhere,
            });
        }
        let (snapshot, _rejected) = builder.build();
        let abbreviations = snapshot.len();
        let mut engine = Engine::new();
        engine.set_snapshot(std::sync::Arc::new(snapshot));
        let ai = editing
            .and_then(|id| self.library.snippet(id))
            .and_then(|snippet| snippet.file.front.ai.clone());
        Trial {
            engine,
            field: Field::default(),
            id,
            body: draft.body.clone(),
            ai,
            abbreviations,
        }
    }
}

impl Trial {
    /// What the field holds now.
    pub fn text(&self) -> &str {
        self.field.text()
    }

    /// Where the caret is, as a byte offset into [`text`](Self::text).
    pub fn caret(&self) -> usize {
        self.field.caret()
    }

    /// The selected byte range, `None` when nothing is selected.
    pub fn selection(&self) -> Option<std::ops::Range<usize>> {
        self.field.selection()
    }

    /// The field with the caret as `|` and a selection in brackets.
    pub fn marked(&self) -> String {
        self.field.marked()
    }

    /// How many of the draft's abbreviations can be typed here. Zero when it
    /// has none, or none the engine would take.
    pub fn abbreviations(&self) -> usize {
        self.abbreviations
    }

    /// The snippet the draft is of, or the ID it stands in with when it is
    /// new. A session this trial opens is for this ID.
    pub fn snippet_id(&self) -> SnippetId {
        self.id
    }

    /// One key, typed at the caret. `core` is the library as it is now, for
    /// the clock, the locale and nested snippets.
    pub fn key(&mut self, core: &Core, event: KeyEvent) -> TrialKey {
        match self.engine.on_key(event) {
            KeyVerdict::Pass => {
                match event {
                    // The shell reports Return as a carriage return, the way
                    // the tap does; the field holds it as the line break a
                    // plan's Return key puts there.
                    KeyEvent::Char('\r') => self.field.write('\n'),
                    KeyEvent::Char(c) => self.field.write(c),
                    KeyEvent::Backspace => self.field.backspace(1),
                    // Without an expansion to take back there is nothing to
                    // undo: the field keeps no history.
                    KeyEvent::Undo => {}
                }
                TrialKey::Typed
            }
            KeyVerdict::Match {
                delete_count,
                case,
                trailing,
                consume,
                snippet_id,
            } => {
                let swallowed = match (consume, event) {
                    (true, KeyEvent::Char(c)) => Some(c),
                    _ => None,
                };
                let shape = session::shape(MatchInfo {
                    snippet_id,
                    delete_count,
                    case,
                    trailing,
                    swallowed,
                });
                match core.begin_body(self.id, &self.body, self.ai.as_ref(), shape, swallowed) {
                    Expand::Ready(expansion) => {
                        self.finish(&expansion);
                        TrialKey::Expanded
                    }
                    Expand::Session(session) => TrialKey::Session(session),
                }
            }
            KeyVerdict::UndoLast {
                delete_count,
                retype,
                ..
            } => {
                self.field.backspace(delete_count);
                let retype = retype.clone();
                self.field.insert_text(&retype);
                TrialKey::Expanded
            }
        }
    }

    /// Types `text` a character at a time. Stops at a session and returns it;
    /// the rest of the text is dropped, as a real app drops what arrives while
    /// a panel has the keyboard.
    pub fn type_str(&mut self, core: &Core, text: &str) -> Option<Box<Session>> {
        for c in text.chars() {
            if let TrialKey::Session(session) = self.key(core, KeyEvent::Char(c)) {
                return Some(session);
            }
        }
        None
    }

    /// Runs what a session came to, and arms undo as a shell does.
    pub fn finish(&mut self, expansion: &Expansion) {
        for step in &expansion.plan.steps {
            self.field.apply(step);
        }
        if let Some(delete_count) = expansion.undo_delete_count {
            self.engine.expansion_done(ExpansionRecord {
                snippet_id: aralo_engine::SnippetId(self.id.as_u128()),
                delete_count,
                method: InsertMethod::Typed,
            });
        }
    }

    /// Runs what a cancelled session gives back: usually the key it swallowed.
    pub fn put_back(&mut self, plan: &ExpansionPlan) {
        for step in &plan.steps {
            self.field.apply(step);
        }
    }

    /// The user clicked or used an arrow key: the caret is at `at`, a byte
    /// offset, and what was typed before it no longer leads up to it.
    pub fn move_caret(&mut self, at: usize) {
        self.field.set_caret(at);
        self.engine.reset(ResetReason::Navigation);
    }

    /// Empties the field.
    pub fn clear(&mut self) {
        self.field.clear();
        self.engine.reset(ResetReason::Manual);
    }
}

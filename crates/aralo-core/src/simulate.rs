use std::collections::BTreeMap;

use aralo_engine::{Engine, ExpansionRecord, InsertMethod, KeyEvent, KeyVerdict};
use aralo_template::{Answers, ContextValues};

use crate::ai::BlockRequest;
use crate::field::Field;
use crate::{Core, Expand, Expansion, MatchInfo, Session, SessionStep};

/// What the simulated user does when a snippet asks a question.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Reply {
    /// Fill the form in from [`Simulator::with_answer`], and hand over
    /// whatever context the simulator was given.
    #[default]
    Fill,
    /// Press Escape at the first panel: the form, or the AI preview. The
    /// session is cancelled and the plan it gives back runs instead.
    Cancel,
}

/// Why a block the simulator was given no answer for puts in its fallback.
pub const NO_MODEL: &str = "nothing here asks a model";

/// Asks a model for a block: the request in, the answer as it goes in or why
/// there is none out. `aralo expand --ai` blocks on the gateway in one.
pub type AskModel<'a> = Box<dyn FnMut(&BlockRequest) -> Result<String, String> + 'a>;

/// A text field that exists only in memory, with a caret.
///
/// It runs keys through the engine, drives expansion sessions and applies the
/// plans exactly as a shell would, so the whole expansion path can be tested,
/// and tried from the command line, without a window server.
pub struct Simulator<'a> {
    core: &'a Core,
    engine: Engine,
    field: Field,
    answers: Answers,
    values: ContextValues,
    /// What "the model" wrote for each block, by its number.
    ai: BTreeMap<usize, String>,
    /// Who answers a block with nothing in `ai`, if anyone.
    model: Option<AskModel<'a>>,
    /// Why a block nobody answers falls back.
    no_model: String,
    reply: Reply,
    expansions: usize,
    cancellations: usize,
    /// The last expansion this field ran, which is what a shell would report
    /// back: what it inserted, whether Backspace can undo it, and what was
    /// wrong with the body.
    last: Option<Expansion>,
}

// Like the engine, never prints what was typed.
impl std::fmt::Debug for Simulator<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Simulator")
            .field("field", &self.field)
            .field("expansions", &self.expansions)
            .field("cancellations", &self.cancellations)
            .finish_non_exhaustive()
    }
}

impl<'a> Simulator<'a> {
    pub fn new(core: &'a Core, front_app: &str) -> Self {
        let mut engine = core.engine();
        engine.set_front_app(front_app);
        Self {
            core,
            engine,
            field: Field::default(),
            answers: Answers::new(),
            values: ContextValues::default(),
            ai: BTreeMap::new(),
            model: None,
            no_model: NO_MODEL.to_owned(),
            reply: Reply::default(),
            expansions: 0,
            cancellations: 0,
            last: None,
        }
    }

    /// What a form field is answered with when a snippet asks for it. A field
    /// nothing was given for keeps its default.
    #[must_use]
    pub fn with_answer(mut self, name: &str, value: &str) -> Self {
        self.answers.insert(name.to_owned(), value.to_owned());
        self
    }

    /// What the shell would have fetched: the clipboard, the selection. A kind
    /// the body did not ask for is dropped by the session, not by this.
    #[must_use]
    pub fn with_context(mut self, values: ContextValues) -> Self {
        self.values = values;
        self
    }

    #[must_use]
    pub fn with_clipboard(mut self, text: &str) -> Self {
        self.values.clipboard = Some(text.to_owned());
        self
    }

    /// What a model writes for the `{{ai}}` block numbered `index`, as the
    /// user accepts it. A block with no answer puts in its fallback.
    #[must_use]
    pub fn with_ai_answer(mut self, index: usize, text: &str) -> Self {
        self.ai.insert(index, text.to_owned());
        self
    }

    /// Asks `model` for every block the simulator was given no answer for.
    #[must_use]
    pub fn asking(
        mut self,
        model: impl FnMut(&BlockRequest) -> Result<String, String> + 'a,
    ) -> Self {
        self.model = Some(Box::new(model));
        self
    }

    /// What a block nobody answers says about falling back, in place of
    /// [`NO_MODEL`].
    #[must_use]
    pub fn without_a_model(mut self, reason: &str) -> Self {
        self.no_model = reason.to_owned();
        self
    }

    /// The user presses Escape instead of filling the form in.
    #[must_use]
    pub fn cancelling(mut self) -> Self {
        self.reply = Reply::Cancel;
        self
    }

    pub fn engine_mut(&mut self) -> &mut Engine {
        &mut self.engine
    }

    /// What the text field holds now.
    pub fn text(&self) -> &str {
        self.field.text()
    }

    /// Where the caret is, as a byte offset into [`text`](Self::text).
    pub fn caret(&self) -> usize {
        self.field.caret()
    }

    /// What is selected, empty when nothing is.
    pub fn selected(&self) -> &str {
        self.field.selected()
    }

    /// The field with the caret written in as `|`, and a selection in
    /// brackets: what a test asserts on when where the caret ended up is the
    /// point. A body that contains those characters itself reads ambiguously,
    /// so such a test compares [`text`](Self::text) and
    /// [`caret`](Self::caret) instead.
    pub fn marked(&self) -> String {
        self.field.marked()
    }

    pub fn expansions(&self) -> usize {
        self.expansions
    }

    /// Sessions the simulated user cancelled.
    pub fn cancellations(&self) -> usize {
        self.cancellations
    }

    /// The last expansion this field ran, `None` before one has: a session
    /// the user cancelled never becomes one.
    pub fn last_expansion(&self) -> Option<&Expansion> {
        self.last.as_ref()
    }

    /// What was wrong with the body the last expansion came from: the
    /// placeholders it could not expand, and why. Empty until one runs.
    pub fn diagnostics(&self) -> &[aralo_template::Diagnostic] {
        match &self.last {
            Some(expansion) => &expansion.template_diagnostics,
            None => &[],
        }
    }

    pub fn type_str(&mut self, text: &str) -> &mut Self {
        for c in text.chars() {
            self.key(KeyEvent::Char(c));
        }
        self
    }

    /// The snippet is picked from a list rather than typed, as the palette
    /// does it.
    pub fn insert(&mut self, id: aralo_snippet::SnippetId) -> &mut Self {
        if let Some(expand) = self.core.insert(id) {
            self.carry_out(expand, aralo_engine::SnippetId(id.as_u128()));
        }
        self
    }

    pub fn key(&mut self, event: KeyEvent) -> &mut Self {
        match self.engine.on_key(event) {
            KeyVerdict::Pass => match event {
                KeyEvent::Char(c) => self.field.write(c),
                KeyEvent::Backspace => self.field.backspace(1),
                // Without an expansion to undo, the app's own undo runs; the
                // simulator has no history to model it with.
                KeyEvent::Undo => {}
            },
            KeyVerdict::Match {
                snippet_id,
                delete_count,
                case,
                trailing,
                consume,
            } => {
                let swallowed = match (consume, event) {
                    (true, KeyEvent::Char(c)) => Some(c),
                    _ => None,
                };
                let expand = self.core.expand(MatchInfo {
                    snippet_id,
                    delete_count,
                    case,
                    trailing,
                    swallowed,
                });
                match expand {
                    Some(expand) => self.carry_out(expand, snippet_id),
                    None => {
                        if let (false, KeyEvent::Char(c)) = (consume, event) {
                            self.field.write(c);
                        }
                    }
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
            }
        }
        self
    }

    /// Runs a plan, or answers the session's questions until there is one.
    fn carry_out(&mut self, expand: Expand, snippet_id: aralo_engine::SnippetId) {
        match expand {
            Expand::Ready(expansion) => self.run(expansion, snippet_id),
            Expand::Session(session) => self.drive(*session, snippet_id),
        }
    }

    fn drive(&mut self, mut session: Session, snippet_id: aralo_engine::SnippetId) {
        loop {
            match session.step() {
                SessionStep::Form(_) | SessionStep::Ai(_) if self.reply == Reply::Cancel => {
                    let plan = session.cancel();
                    for step in &plan.steps {
                        self.field.apply(step);
                    }
                    self.cancellations += 1;
                    return;
                }
                SessionStep::Form(_) => {
                    session.submit_form(self.answers.clone());
                }
                SessionStep::Context(_) => {
                    session.provide_context(self.values.clone());
                }
                SessionStep::Ai(waiting) => {
                    for block in waiting {
                        let index = block.index;
                        let answer = match (self.ai.get(&index), self.model.as_mut()) {
                            (Some(text), _) => Ok(text.clone()),
                            (None, Some(model)) => match session.block_request(index) {
                                Some(request) => model(&request),
                                None => Err(self.no_model.clone()),
                            },
                            (None, None) => Err(self.no_model.clone()),
                        };
                        match answer {
                            Ok(text) => session.answer_block(index, text),
                            Err(reason) => session.fall_back(index, reason),
                        };
                    }
                }
                SessionStep::Ready(expansion) => {
                    self.run(expansion, snippet_id);
                    return;
                }
            }
        }
    }

    fn run(&mut self, expansion: Expansion, snippet_id: aralo_engine::SnippetId) {
        for step in &expansion.plan.steps {
            self.field.apply(step);
        }
        self.expansions += 1;
        if let Some(delete_count) = expansion.undo_delete_count {
            self.engine.expansion_done(ExpansionRecord {
                snippet_id,
                delete_count,
                method: InsertMethod::Typed,
            });
        }
        self.last = Some(expansion);
    }
}

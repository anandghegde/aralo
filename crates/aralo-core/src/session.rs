//! An expansion that cannot finish on the keystroke.
//!
//! Most snippets expand in the tap callback: the body needs nothing but the
//! clock, so [`Core::expand`] hands back a plan and the shell runs it. A body
//! with a form field or a `{{clipboard}}` cannot: someone has to be asked
//! first. That snippet opens a [`Session`], which the shell drives one
//! question at a time until a plan comes out.
//!
//! The order is fixed — form, then context, then the AI blocks, then the plan —
//! and a session only ever asks for what the body named or the snippet
//! declared. Nothing else is fetched, so an undeclared context kind is never so
//! much as requested (PRD P4).
//!
//! An `{{ai}}` block is the one question a session cannot answer by asking the
//! shell for a value. The shell asks a model for it through
//! [`AiSettings::run_block`](crate::ai::AiSettings::run_block), shows the
//! answer as it streams, and settles the block with
//! [`Session::answer_block`] or [`Session::fall_back`]. The session keeps the
//! answer as literal text; it is never read for placeholders (PRD P10).
//!
//! [`Core::expand`]: crate::Core::expand

use std::collections::BTreeMap;

use aralo_template::{
    AiAnswer, AiAnswers, AiBlock, Answers, Context, ContextKind, ContextValues, Diagnostic,
    ExpansionPlan, Form, Rendered, Resolved, Shape, Step,
};

use crate::ai::{BlockRequest, BlockSettings};
use crate::{Expansion, MatchInfo};

/// What the shell must do next to finish an expansion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionStep {
    /// Put this form in front of the user, then call
    /// [`Session::submit_form`]. Cancelling calls [`Session::cancel`].
    Form(Form),
    /// Fetch these, then call [`Session::provide_context`]. The list is what
    /// the body asked for and what the snippet declared for its AI blocks,
    /// and never longer.
    Context(Vec<ContextKind>),
    /// Ask a model for each of these blocks, then settle each one with
    /// [`Session::answer_block`] or [`Session::fall_back`]. The list is the
    /// blocks not yet settled.
    Ai(Vec<AiBlock>),
    /// Nothing left to ask. Run this.
    Ready(Expansion),
}

/// Which question a session is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Form,
    Context,
    Ai,
    Ready,
}

/// One expansion in progress.
///
/// The session owns everything it needs, so the shell can hold it while a
/// panel is open and the library changes underneath.
#[derive(Debug)]
pub struct Session {
    snippet_id: aralo_snippet::SnippetId,
    resolved: Resolved,
    shape: Shape,
    now: aralo_template::CivilTime,
    locale: String,
    /// The key the engine swallowed at the match, to put back if the user
    /// changes their mind.
    swallowed: Option<char>,
    answers: Answers,
    /// What the shell fetched for the body's own placeholders.
    values: ContextValues,
    /// The `{{ai}}` blocks there is a model to ask for.
    blocks: Vec<AiBlock>,
    /// What the snippet's `ai` front matter says: who answers the blocks and
    /// what they may see.
    ai: BlockSettings,
    /// What the blocks have come to so far.
    ai_answers: AiAnswers,
    /// What the shell fetched for the AI blocks. It is kept apart from
    /// `values`, so a kind declared for a model is not also written into the
    /// text by a placeholder that does not expand it yet.
    ai_values: ContextValues,
    /// Every kind the shell is asked for: the body's placeholders' first, then
    /// the kinds declared for its AI blocks.
    needs: Vec<ContextKind>,
    phase: Phase,
}

impl Session {
    pub(crate) fn new(
        snippet_id: aralo_snippet::SnippetId,
        resolved: Resolved,
        ai: BlockSettings,
        shape: Shape,
        now: aralo_template::CivilTime,
        locale: String,
        swallowed: Option<char>,
    ) -> Self {
        let answers = resolved.form().defaults();
        let blocks = resolved.ai_blocks();
        let mut needs = resolved.needs().to_vec();
        // The declared kinds are asked for only when there is a block to send
        // them with: a snippet whose AI block was deleted reads nothing more
        // because its front matter still declares a selection.
        if !blocks.is_empty() {
            for kind in ai.shell_kinds() {
                if !needs.contains(&kind) {
                    needs.push(kind);
                }
            }
        }
        Self {
            snippet_id,
            resolved,
            shape,
            now,
            locale,
            swallowed,
            answers,
            values: ContextValues::default(),
            blocks,
            ai,
            ai_answers: AiAnswers::new(),
            ai_values: ContextValues::default(),
            needs,
            phase: Phase::Form,
        }
    }

    /// Which snippet is expanding. The shell reports it back to
    /// [`Core::record_expansion`](crate::Runtime::record_expansion) and uses
    /// it to arm undo.
    pub fn snippet_id(&self) -> aralo_snippet::SnippetId {
        self.snippet_id
    }

    /// The form the body asks for, empty when it asks for none.
    pub fn form(&self) -> &Form {
        self.resolved.form()
    }

    /// The context kinds the shell is asked for, in a stable order: what the
    /// body's placeholders read, then what the snippet declared for its AI
    /// blocks.
    pub fn needs(&self) -> &[ContextKind] {
        &self.needs
    }

    /// The `{{ai}}` blocks there is a model to ask for, settled or not.
    pub fn ai_blocks(&self) -> &[AiBlock] {
        &self.blocks
    }

    /// What the blocks have come to so far.
    pub fn ai_answers(&self) -> &AiAnswers {
        &self.ai_answers
    }

    /// What is wrong with the body, from resolving it. The expansion still
    /// happens; the panel can show these beside the form.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        self.resolved.diagnostics()
    }

    /// The answers as they stand: the defaults at first, then whatever was
    /// submitted.
    pub fn answers(&self) -> &Answers {
        &self.answers
    }

    /// The next question, or the finished expansion.
    ///
    /// Asking twice asks the same question again: the session moves on when
    /// it is answered, not when it is read.
    pub fn step(&mut self) -> SessionStep {
        loop {
            match self.phase {
                Phase::Form if self.form().fields.is_empty() => self.phase = Phase::Context,
                Phase::Form => return SessionStep::Form(self.form().clone()),
                Phase::Context if self.needs.is_empty() => self.phase = Phase::Ai,
                Phase::Context => return SessionStep::Context(self.needs.clone()),
                Phase::Ai => {
                    let waiting = self.waiting();
                    if waiting.is_empty() {
                        self.phase = Phase::Ready;
                    } else {
                        return SessionStep::Ai(waiting);
                    }
                }
                Phase::Ready => return SessionStep::Ready(self.finish()),
            }
        }
    }

    /// The form is filled in. A field the answers leave out keeps its default.
    pub fn submit_form(&mut self, answers: Answers) -> SessionStep {
        for (name, value) in answers {
            if self.form().field(&name).is_some() {
                self.answers.insert(name, value);
            }
        }
        self.phase = Phase::Context;
        self.step()
    }

    /// What the shell fetched. Anything that was not asked for is dropped
    /// here rather than trusted: the rule is a property of the core, not of
    /// the shell that calls it (PRD P4).
    ///
    /// The body's placeholders see what they asked for; the AI blocks see
    /// what the snippet declared for them, and only when a request is made.
    pub fn provide_context(&mut self, values: ContextValues) -> SessionStep {
        let body = self.resolved.needs();
        let declared: Vec<ContextKind> = if self.blocks.is_empty() {
            Vec::new()
        } else {
            self.ai.shell_kinds().collect()
        };
        self.values = keep(&values, body);
        self.ai_values = keep(&values, &declared);
        self.phase = Phase::Ai;
        self.step()
    }

    /// Everything a request for one block needs: the prompt, who answers it,
    /// and the declared context with the text the session holds for it.
    /// `None` for a number that is not one of [`ai_blocks`](Self::ai_blocks).
    ///
    /// The form's answers go only to a snippet that declared `fillins`, and
    /// the text around the block goes to none: it holds the answers and the
    /// clipboard, which a snippet may not have declared.
    pub fn block_request(&self, index: usize) -> Option<BlockRequest> {
        let block = self.blocks.iter().find(|block| block.index == index)?;
        Some(BlockRequest::new(
            block.clone(),
            &self.ai,
            self.resolved.form(),
            &self.answers,
            &self.ai_values,
        ))
    }

    /// What a model wrote for a block, as the user accepted it: edited, or
    /// as it came. It goes into the text as it is, and is never read for
    /// placeholders (PRD P10). A block can be settled again, to regenerate,
    /// until the session is finished.
    ///
    /// A number that is not a block is ignored.
    pub fn answer_block(&mut self, index: usize, text: String) -> SessionStep {
        self.settle(index, AiAnswer::Text(text))
    }

    /// No model answered a block: AI is off, the network failed, or the user
    /// chose the fallback. The block puts in its `fallback:` text, and the
    /// expansion carries a note saying `reason`.
    pub fn fall_back(&mut self, index: usize, reason: String) -> SessionStep {
        self.settle(index, AiAnswer::Fallback { reason })
    }

    fn settle(&mut self, index: usize, answer: AiAnswer) -> SessionStep {
        if self.blocks.iter().any(|block| block.index == index) {
            self.ai_answers.insert(index, answer);
        }
        self.step()
    }

    /// The blocks not yet settled.
    fn waiting(&self) -> Vec<AiBlock> {
        self.blocks
            .iter()
            .filter(|block| !self.ai_answers.contains_key(&block.index))
            .cloned()
            .collect()
    }

    /// The text this session would insert as it stands, for a panel that
    /// shows the result while the form is being filled in.
    ///
    /// It is the expansion path with the same answers, so the preview is the
    /// expansion, minus the cursor and the keys.
    pub fn preview(&self) -> String {
        self.preview_with(&Answers::new())
    }

    /// The same, for a form that is being typed into: `answers` are the boxes
    /// as they stand now, over the ones the session already has.
    ///
    /// It changes nothing. A session's answers are the ones
    /// [`submit_form`](Self::submit_form) gave it, so a panel can preview
    /// every keystroke and still let the user change their mind. A name the
    /// form does not have is ignored, as it is on submitting.
    pub fn preview_with(&self, answers: &Answers) -> String {
        self.preview_blocks(answers, &BTreeMap::new()).text()
    }

    /// The same, with the text of blocks still being written: `drafts` are
    /// what the models have written so far, or what the user edited them to,
    /// over the blocks already settled. A block in neither shows its
    /// fallback.
    ///
    /// It is a rendering rather than a string so the panel can mark each
    /// block's text: [`Rendered::ai_spans`] says where each one is, and
    /// everything outside them is the body's own, byte for byte.
    pub fn preview_blocks(&self, answers: &Answers, drafts: &BTreeMap<usize, String>) -> Rendered {
        let mut merged = self.answers.clone();
        for (name, value) in answers {
            if self.form().field(name).is_some() {
                merged.insert(name.clone(), value.clone());
            }
        }
        let mut ai = self.ai_answers.clone();
        for (&index, text) in drafts {
            if self.blocks.iter().any(|block| block.index == index) {
                ai.insert(index, AiAnswer::Text(text.clone()));
            }
        }
        let context = Context::at(self.now)
            .in_locale(&self.locale)
            .with_values(&self.values)
            .with_answers(&merged)
            .with_ai(&ai);
        aralo_template::render(&self.resolved, context)
    }

    /// The user changed their mind.
    ///
    /// Nothing has been deleted yet — the abbreviation is still in the
    /// document, because the plan that removes it is the plan that replaces
    /// it — so all this puts back is the key the engine swallowed to open the
    /// session. Return and Tab are not put back: the app may act on them, and
    /// Aralo does not send a keystroke the user did not ask for to undo its
    /// own panel.
    pub fn cancel(self) -> ExpansionPlan {
        let steps = match self.swallowed {
            Some(character) if !character.is_control() => vec![Step::InsertText {
                text: character.to_string(),
            }],
            _ => Vec::new(),
        };
        ExpansionPlan { steps }
    }

    fn finish(&self) -> Expansion {
        let context = Context::at(self.now)
            .in_locale(&self.locale)
            .with_values(&self.values)
            .with_answers(&self.answers)
            .with_ai(&self.ai_answers);
        let (plan, template_diagnostics) =
            aralo_template::finish(&self.resolved, context, self.shape);
        Expansion {
            undo_delete_count: plan.undo_delete_count(),
            plan,
            template_diagnostics,
        }
    }
}

/// The values of `kinds`, and nothing else.
fn keep(values: &ContextValues, kinds: &[ContextKind]) -> ContextValues {
    let allowed = |kind: ContextKind| kinds.contains(&kind);
    ContextValues {
        clipboard: values
            .clipboard
            .clone()
            .filter(|_| allowed(ContextKind::Clipboard)),
        selection: values
            .selection
            .clone()
            .filter(|_| allowed(ContextKind::Selection)),
        app: values.app.clone().filter(|_| allowed(ContextKind::App)),
        window: values
            .window
            .clone()
            .filter(|_| allowed(ContextKind::Window)),
    }
}

/// What [`Core::expand`](crate::Core::expand) found: a plan to run now, or a
/// session to drive.
///
/// The session is boxed because most expansions are not one, and this is
/// returned on the keystroke path: the common answer stays small.
#[derive(Debug)]
pub enum Expand {
    /// The body needed nothing but the clock.
    Ready(Expansion),
    /// The body needs a form filled in, context fetched, or a model asked.
    Session(Box<Session>),
}

impl Expand {
    /// The plan, when there was nothing to ask. `None` for a session.
    pub fn ready(self) -> Option<Expansion> {
        match self {
            Self::Ready(expansion) => Some(expansion),
            Self::Session(_) => None,
        }
    }

    /// The session, when there was something to ask.
    pub fn session(self) -> Option<Session> {
        match self {
            Self::Session(session) => Some(*session),
            Self::Ready(_) => None,
        }
    }
}

/// What the engine reported, as the shape of an expansion.
pub(crate) fn shape(matched: MatchInfo) -> Shape {
    Shape {
        delete_count: matched.delete_count,
        case: match matched.case {
            aralo_engine::CasePattern::AsDefined => aralo_template::CaseTransform::AsDefined,
            aralo_engine::CasePattern::Title => aralo_template::CaseTransform::Title,
            aralo_engine::CasePattern::Upper => aralo_template::CaseTransform::Upper,
        },
        trailing: matched.trailing,
    }
}

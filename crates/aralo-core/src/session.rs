//! An expansion that cannot finish on the keystroke.
//!
//! Most snippets expand in the tap callback: the body needs nothing but the
//! clock, so [`Core::expand`] hands back a plan and the shell runs it. A body
//! with a form field or a `{{clipboard}}` cannot: someone has to be asked
//! first. That snippet opens a [`Session`], which the shell drives one
//! question at a time until a plan comes out.
//!
//! The order is fixed — form, then context, then the plan — and a session only
//! ever asks for what the body named. Nothing else is fetched, so an
//! undeclared context kind is never so much as requested (PRD P4).
//!
//! [`Core::expand`]: crate::Core::expand

use aralo_template::{
    Answers, Context, ContextKind, ContextValues, Diagnostic, ExpansionPlan, Form, Resolved, Shape,
    Step,
};

use crate::{Expansion, MatchInfo};

/// What the shell must do next to finish an expansion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionStep {
    /// Put this form in front of the user, then call
    /// [`Session::submit_form`]. Cancelling calls [`Session::cancel`].
    Form(Form),
    /// Fetch these, then call [`Session::provide_context`]. The list is what
    /// the body asked for, and never longer.
    Context(Vec<ContextKind>),
    /// Nothing left to ask. Run this.
    Ready(Expansion),
}

/// Which question a session is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Form,
    Context,
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
    values: ContextValues,
    phase: Phase,
}

impl Session {
    pub(crate) fn new(
        snippet_id: aralo_snippet::SnippetId,
        resolved: Resolved,
        shape: Shape,
        now: aralo_template::CivilTime,
        locale: String,
        swallowed: Option<char>,
    ) -> Self {
        let answers = resolved.form().defaults();
        Self {
            snippet_id,
            resolved,
            shape,
            now,
            locale,
            swallowed,
            answers,
            values: ContextValues::default(),
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

    /// The context kinds the body asks for, in a stable order.
    pub fn needs(&self) -> &[ContextKind] {
        self.resolved.needs()
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
                Phase::Context if self.needs().is_empty() => self.phase = Phase::Ready,
                Phase::Context => return SessionStep::Context(self.needs().to_vec()),
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

    /// What the shell fetched. Anything the body did not ask for is dropped
    /// here rather than trusted: the rule is a property of the core, not of
    /// the shell that calls it (PRD P4).
    pub fn provide_context(&mut self, values: ContextValues) -> SessionStep {
        let allowed = |kind: ContextKind| self.needs().contains(&kind);
        self.values = ContextValues {
            clipboard: values.clipboard.filter(|_| allowed(ContextKind::Clipboard)),
            selection: values.selection.filter(|_| allowed(ContextKind::Selection)),
            app: values.app.filter(|_| allowed(ContextKind::App)),
            window: values.window.filter(|_| allowed(ContextKind::Window)),
        };
        self.phase = Phase::Ready;
        self.step()
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
        let mut merged = self.answers.clone();
        for (name, value) in answers {
            if self.form().field(name).is_some() {
                merged.insert(name.clone(), value.clone());
            }
        }
        let context = Context::at(self.now)
            .in_locale(&self.locale)
            .with_values(&self.values)
            .with_answers(&merged);
        aralo_template::render(&self.resolved, context).text()
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
            .with_answers(&self.answers);
        let (plan, template_diagnostics) =
            aralo_template::finish(&self.resolved, context, self.shape);
        Expansion {
            undo_delete_count: plan.undo_delete_count(),
            plan,
            template_diagnostics,
        }
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
    /// The body needs a form filled in, or context fetched, or both.
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

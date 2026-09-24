//! AI blocks in snippets (plan 7.3, PRD A2, task 4.6).
//!
//! A snippet body asks a model for part of its text with
//! `{{ai: prompt | fallback: … | model: …}}`. The session collects the form
//! and the context first; then each block becomes a [`BlockRequest`], goes
//! through the gateway as `Feature::Block`, and streams back through a
//! [`BlockRun`]. What the user accepts goes into the expansion as literal
//! text (PRD P10).
//!
//! What a block may send is what its snippet declared in `ai.context`, and
//! nothing else (PRD P4). The body's own text is never sent: around the block
//! it holds the form's answers and the clipboard, which the snippet may not
//! have declared.

use std::collections::BTreeMap;

use aralo_ai::{
    AiError, AiRequest, AiStream, ContextKind, ContextSource, Delta, Feature, ManifestEntry,
    StopReason,
};
use aralo_template::{AiBlock, Answers, ContextKind as SessionKind, ContextValues, Form};

use super::{AiSettings, AiSettingsError, CommandProblem};
use crate::{Session, SessionStep};

/// What every block's model is told. The prompt is the snippet's own words and
/// follows as the user's message.
const SYSTEM: &str = "You write one piece of a text for Aralo, a text expander. The user \
typed an abbreviation, and the snippet it stands for asks you for this piece. Reply with that \
text and nothing else: no introduction, no explanation, no quotation marks around it and no \
Markdown code fence. It goes into the user's document exactly as you write it, between text of \
the snippet's own that you do not see.";

/// Who answers a snippet's blocks and what they may see: the snippet's `ai`
/// front matter, read once when the session opens.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlockSettings {
    /// The kinds `ai.context` names that Aralo knows, in the order written.
    pub declared: Vec<ContextKind>,
    /// Names `ai.context` holds that Aralo does not know. They grant nothing.
    pub unknown: Vec<String>,
    /// `ai.profile`: `None` is the default profile.
    pub profile: Option<String>,
    /// `ai.model`: a block's own `model:` wins over it.
    pub model: Option<String>,
}

impl BlockSettings {
    /// The settings a snippet's front matter gives its blocks.
    pub fn of(ai: Option<&aralo_snippet::AiSettings>) -> Self {
        let Some(ai) = ai else {
            return Self::default();
        };
        let (declared, unknown) = ContextKind::parse_declared(&ai.context);
        let named = |value: &Option<String>| {
            value
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        };
        Self {
            declared,
            unknown,
            profile: named(&ai.profile),
            model: named(&ai.model),
        }
    }

    /// The declared kinds only the shell can fetch. The form's answers are
    /// the session's own.
    pub(crate) fn shell_kinds(&self) -> impl Iterator<Item = SessionKind> + '_ {
        self.declared.iter().filter_map(|kind| match kind {
            ContextKind::Fillins => None,
            ContextKind::Selection => Some(SessionKind::Selection),
            ContextKind::Clipboard => Some(SessionKind::Clipboard),
            ContextKind::App => Some(SessionKind::App),
            ContextKind::Window => Some(SessionKind::Window),
        })
    }
}

/// One block, ready to send: the prompt, who answers it, and the declared
/// context with its text. [`Session::block_request`] makes one.
#[derive(Clone, PartialEq, Eq)]
pub struct BlockRequest {
    pub block: AiBlock,
    pub profile: Option<String>,
    /// The block's `model:`, else the snippet's `ai.model`. `None` is the
    /// profile's own.
    pub model: Option<String>,
    pub declared: Vec<ContextKind>,
    /// The text for each declared kind the session has one for.
    context: BTreeMap<ContextKind, String>,
}

// The context is what the user typed and copied: it is never printed.
impl std::fmt::Debug for BlockRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BlockRequest")
            .field("block", &self.block.index)
            .field("profile", &self.profile)
            .field("model", &self.model)
            .field("declared", &self.declared)
            .finish_non_exhaustive()
    }
}

impl BlockRequest {
    pub(crate) fn new(
        block: AiBlock,
        settings: &BlockSettings,
        form: &Form,
        answers: &Answers,
        values: &ContextValues,
    ) -> Self {
        let mut context = BTreeMap::new();
        for &kind in &settings.declared {
            let text = match kind {
                ContextKind::Fillins => fillins(form, answers),
                ContextKind::Selection => values.selection.clone(),
                ContextKind::Clipboard => values.clipboard.clone(),
                ContextKind::App => values.app.clone(),
                ContextKind::Window => values.window.clone(),
            };
            if let Some(text) = text {
                context.insert(kind, text);
            }
        }
        Self {
            model: block.model.clone().or_else(|| settings.model.clone()),
            profile: settings.profile.clone(),
            declared: settings.declared.clone(),
            block,
            context,
        }
    }
}

/// The form's answers as the model reads them: one `label: answer` per field,
/// in the body's order. `None` for a snippet with no form.
fn fillins(form: &Form, answers: &Answers) -> Option<String> {
    if form.is_empty() {
        return None;
    }
    let lines: Vec<String> = form
        .fields
        .iter()
        .map(|field| {
            let answer = answers
                .get(&field.name)
                .map_or_else(|| field.default(), String::as_str);
            format!("{}: {answer}", field.label)
        })
        .collect();
    Some(lines.join("\n"))
}

/// The gateway asks for each declared kind once; this answers from what the
/// session collected, and for nothing it did not.
struct Declared<'a>(&'a BlockRequest);

impl ContextSource for Declared<'_> {
    fn fetch(&mut self, kind: ContextKind) -> Option<String> {
        if !self.0.declared.contains(&kind) {
            return None;
        }
        self.0.context.get(&kind).cloned()
    }
}

impl AiSettings {
    /// Asks a model for one block and returns the answer as it streams.
    ///
    /// The block runs on its snippet's profile, or on the default one. The
    /// gateway asks for the declared kinds only (PRD P4), and refuses before
    /// anything is read or sent when AI is off or local-only mode forbids the
    /// profile's host. A refusal or a failure is the caller's cue to
    /// [`Session::fall_back`].
    pub async fn run_block(&self, request: &BlockRequest) -> Result<BlockRun, AiSettingsError> {
        if !self.switches()?.enabled {
            return Err(AiError::from(aralo_ai::Refusal::Off).into());
        }
        let saved = match &request.profile {
            Some(name) => self.profile(name)?,
            None => self.default_profile()?.ok_or(CommandProblem::NoProfile)?,
        };
        let key = self.key_for(&saved.profile)?;
        let ai_request = AiRequest {
            feature: Feature::Block,
            profile: saved.profile,
            model: request.model.clone(),
            system: SYSTEM.to_owned(),
            instruction: request.block.prompt.clone(),
            declared: request.declared.clone(),
            max_tokens: None,
            temperature: None,
            json_output: false,
        };
        let prepared = self.gateway().prepare(ai_request, &mut Declared(request))?;
        let manifest = prepared.manifest().to_vec();
        let profile = prepared.profile().name.clone();
        let model = prepared.chat().model.clone();
        let stream = self.gateway().send(prepared, key).await?;
        Ok(BlockRun {
            block: request.block.index,
            stream,
            manifest,
            profile,
            model,
            text: String::new(),
            stop: None,
        })
    }

    /// Settles every block the session is waiting on without asking anyone:
    /// each one gets the model's answer as it came, or its fallback with the
    /// reason no model gave one. For a caller with no panel to show, such as
    /// `aralo expand --ai`.
    pub async fn answer_blocks(&self, session: &mut Session) -> SessionStep {
        loop {
            let waiting = match session.step() {
                SessionStep::Ai(waiting) => waiting,
                other => return other,
            };
            for block in waiting {
                let Some(request) = session.block_request(block.index) else {
                    continue;
                };
                match self.run_block(&request).await {
                    Ok(run) => match run.finish().await {
                        Ok(answer) => session.answer_block(block.index, answer),
                        Err(reason) => session.fall_back(block.index, reason),
                    },
                    Err(error) => session.fall_back(block.index, error.to_string()),
                };
            }
        }
    }
}

/// A block's answer, arriving.
#[derive(Debug)]
pub struct BlockRun {
    block: usize,
    stream: AiStream,
    manifest: Vec<ManifestEntry>,
    profile: String,
    model: String,
    text: String,
    stop: Option<StopReason>,
}

impl BlockRun {
    /// The next piece of the answer. `None` once it has finished; an error
    /// ends it too, and what arrived before it is still in [`Self::text`].
    pub async fn next(&mut self) -> Option<Result<String, AiError>> {
        loop {
            match self.stream.next().await? {
                Ok(Delta::Text(piece)) => {
                    self.text.push_str(&piece);
                    return Some(Ok(piece));
                }
                Ok(Delta::Usage(_)) => {}
                Ok(Delta::Done(reason)) => {
                    self.stop = Some(reason);
                    return None;
                }
                Err(error) => return Some(Err(error)),
            }
        }
    }

    /// Reads the rest of the answer, and returns it ready to go in, or why it
    /// cannot: the stream failed, or the model wrote nothing.
    pub async fn finish(mut self) -> Result<String, String> {
        while let Some(piece) = self.next().await {
            piece.map_err(|error| AiSettingsError::from(error).to_string())?;
        }
        self.answer()
            .ok_or_else(|| "the model's answer was empty".to_owned())
    }

    /// Which block this answers.
    pub fn block(&self) -> usize {
        self.block
    }

    /// What was sent with the prompt: the kind of each piece, and its size.
    pub fn manifest(&self) -> &[ManifestEntry] {
        &self.manifest
    }

    pub fn profile(&self) -> &str {
        &self.profile
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    /// The answer so far, as the model wrote it.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Why the model stopped, once it has.
    pub fn stop_reason(&self) -> Option<&StopReason> {
        self.stop.as_ref()
    }

    /// The answer as it would go in, or `None` while there is nothing to put
    /// in.
    pub fn answer(&self) -> Option<String> {
        let answer = fit_block(&self.text);
        (!answer.is_empty()).then_some(answer)
    }
}

/// Shapes a model's answer to the place it goes: without the white space at
/// either end, which would break the line the block sits in, and without a
/// code fence around the whole of it. Nothing inside is changed.
pub fn fit_block(answer: &str) -> String {
    let body = answer.trim();
    let body = super::command::unfence(body).trim();
    body.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_loses_its_edges_and_a_fence_around_the_whole() {
        assert_eq!(fit_block("\n  Thanks a lot!\n\n"), "Thanks a lot!");
        assert_eq!(fit_block("```\nThanks\n```"), "Thanks");
        assert_eq!(fit_block("line one\n\nline two"), "line one\n\nline two");
        assert_eq!(fit_block("Use ```x``` here"), "Use ```x``` here");
        assert_eq!(fit_block("  \n"), "");
    }

    #[test]
    fn settings_read_the_front_matter_and_keep_unknown_names_apart() {
        let ai = aralo_snippet::AiSettings {
            context: vec!["fillins".into(), "selection".into(), "screen".into()],
            profile: Some("  ".into()),
            model: Some(" small ".into()),
            ..Default::default()
        };
        let settings = BlockSettings::of(Some(&ai));
        assert_eq!(
            settings.declared,
            [ContextKind::Fillins, ContextKind::Selection]
        );
        assert_eq!(settings.unknown, ["screen"]);
        assert_eq!(settings.profile, None);
        assert_eq!(settings.model.as_deref(), Some("small"));
        assert_eq!(
            settings.shell_kinds().collect::<Vec<_>>(),
            [SessionKind::Selection]
        );
        assert_eq!(BlockSettings::of(None), BlockSettings::default());
    }
}

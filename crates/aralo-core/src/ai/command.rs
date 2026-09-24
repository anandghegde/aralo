//! Commands on selected text (plan 7.3, PRD A3, task 4.5).
//!
//! A command is a snippet of `type: command`: its body is the instruction,
//! and the text the user selected is the one piece of context it sends. The
//! built-in commands are such files too, in `data/commands`, so a custom
//! command needs no machinery the built-in ones do not use. A library command
//! with a built-in's `id` takes its place.
//!
//! The shell reads the selection and hands it over; the command asks the
//! gateway for nothing else. What comes back is literal text (PRD P10): the
//! shell shows it as a diff against the selection and pastes it over the
//! selection only when the user says so.

use aralo_ai::{
    AiError, AiRequest, AiStream, ContextKind, ContextSource, Delta, Feature, ManifestEntry,
    StopReason,
};
use aralo_library::LoadedSnippet;
use aralo_snippet::{SnippetFile, SnippetId, SnippetKind};

use super::{AiSettings, AiSettingsError};
use crate::diff::{diff_words, DiffSpan};
use crate::Core;

/// The most a command will send, in bytes of UTF-8. A selection is something
/// a person means to rewrite, and a whole document pasted into a model by
/// accident is a bill and a privacy slip at once.
pub const MAX_SELECTION: usize = 100_000;

/// What every command's model is told, before the command's own instruction.
const SYSTEM: &str = "You are a text tool inside Aralo. The user selected text in one of \
their apps and asked for a change to it. Reply with the changed text and nothing else: no \
introduction, no explanation, no quotation marks around it and no Markdown code fence. Keep \
the text's language, its line breaks and its formatting unless the request is to change them.";

macro_rules! builtin_files {
    ($($name:literal),* $(,)?) => {
        &[$(include_str!(concat!("../../../../data/commands/", $name))),*]
    };
}

/// The built-in commands, in the order the palette lists them. A test checks
/// this list against the folder.
const BUILTIN_FILES: &[&str] = builtin_files![
    "proofread.md",
    "clearer.md",
    "shorter.md",
    "formal.md",
    "friendly.md",
    "english.md",
    "summarise.md",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub id: SnippetId,
    pub label: String,
    /// The snippet's body: what the model is asked to do with the selection.
    pub instruction: String,
    pub tags: Vec<String>,
    /// The profile it runs on. `None` is the default profile.
    pub profile: Option<String>,
    /// Overrides the profile's model.
    pub model: Option<String>,
    /// Shipped with Aralo rather than kept in the library.
    pub builtin: bool,
}

impl Command {
    /// The command a library snippet is, if it is an enabled `type: command`
    /// with an instruction.
    pub fn from_snippet(snippet: &LoadedSnippet) -> Option<Self> {
        if !snippet.settings.enabled {
            return None;
        }
        let mut command = Self::from_file(snippet.id, &snippet.file, false)?;
        if command.label.is_empty() {
            command.label = snippet.display_name().to_owned();
        }
        Some(command)
    }

    fn from_file(id: SnippetId, file: &SnippetFile, builtin: bool) -> Option<Self> {
        let front = &file.front;
        if front.kind != SnippetKind::Command || file.body.trim().is_empty() {
            return None;
        }
        let ai = front.ai.as_ref();
        Some(Self {
            id,
            label: front.label.clone(),
            instruction: file.body.trim().to_owned(),
            tags: front.tags.clone(),
            profile: ai
                .and_then(|ai| ai.profile.clone())
                .filter(|p| !p.trim().is_empty()),
            model: ai
                .and_then(|ai| ai.model.clone())
                .filter(|m| !m.trim().is_empty()),
            builtin,
        })
    }
}

/// The commands Aralo ships with.
pub fn builtin_commands() -> Vec<Command> {
    BUILTIN_FILES
        .iter()
        .filter_map(|text| {
            let file = SnippetFile::parse(text).ok()?;
            Command::from_file(file.front.id?, &file, true)
        })
        .collect()
}

impl Core {
    /// Every command there is to run: the built-in ones, then the library's
    /// by name. A library command with a built-in's `id` replaces it where it
    /// stands, so a user can reword "Make it shorter" by saving a copy.
    pub fn commands(&self) -> Vec<Command> {
        let mut commands = builtin_commands();
        let mut own: Vec<Command> = self
            .snippets()
            .iter()
            .filter_map(Command::from_snippet)
            .collect();
        own.sort_by_cached_key(|command| command.label.to_lowercase());
        for command in own {
            match commands
                .iter_mut()
                .find(|existing| existing.id == command.id)
            {
                Some(existing) => *existing = command,
                None => commands.push(command),
            }
        }
        commands
    }

    /// One command by its `id`.
    pub fn command(&self, id: SnippetId) -> Option<Command> {
        self.commands().into_iter().find(|command| command.id == id)
    }
}

/// Why a command did not run. Each comes before anything is sent.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CommandProblem {
    #[error("nothing is selected")]
    NothingSelected,
    #[error(
        "the selection is {bytes} bytes long, and a command sends at most {limit}; select less"
    )]
    TooLong { bytes: usize, limit: usize },
    #[error("there is no AI profile to run it with; add one in Settings, under AI")]
    NoProfile,
}

/// The selection, and nothing else, for the gateway to ask for.
struct Selection<'a>(&'a str);

impl ContextSource for Selection<'_> {
    fn fetch(&mut self, kind: ContextKind) -> Option<String> {
        (kind == ContextKind::Selection).then(|| self.0.to_owned())
    }
}

impl AiSettings {
    /// Runs `command` on `selection` and returns the answer as it streams.
    ///
    /// The command runs on its own profile, or on the default one. Only the
    /// selection is declared, so it is the only context the gateway asks for
    /// (PRD P4); the policy check, framing and the network guard are the
    /// gateway's, as for every other feature.
    pub async fn run_command(
        &self,
        command: &Command,
        selection: &str,
    ) -> Result<CommandRun, AiSettingsError> {
        if selection.trim().is_empty() {
            return Err(CommandProblem::NothingSelected.into());
        }
        if selection.len() > MAX_SELECTION {
            return Err(CommandProblem::TooLong {
                bytes: selection.len(),
                limit: MAX_SELECTION,
            }
            .into());
        }
        // The gateway refuses too, but only once there is a profile to ask;
        // with AI off, that it is off is the thing to say.
        if !self.switches()?.enabled {
            return Err(AiError::from(aralo_ai::Refusal::Off).into());
        }
        let saved = match &command.profile {
            Some(name) => self.profile(name)?,
            None => self.default_profile()?.ok_or(CommandProblem::NoProfile)?,
        };
        let key = self.key_for(&saved.profile)?;
        let request = AiRequest {
            feature: Feature::Command,
            profile: saved.profile,
            model: command.model.clone(),
            system: SYSTEM.to_owned(),
            instruction: command.instruction.clone(),
            declared: vec![ContextKind::Selection],
            max_tokens: None,
            temperature: None,
            json_output: false,
        };
        let prepared = self.gateway().prepare(request, &mut Selection(selection))?;
        let manifest = prepared.manifest().to_vec();
        let profile = prepared.profile().name.clone();
        let model = prepared.chat().model.clone();
        let stream = self.gateway().send(prepared, key).await?;
        Ok(CommandRun {
            stream,
            manifest,
            profile,
            model,
            selection: selection.to_owned(),
            text: String::new(),
            stop: None,
        })
    }
}

/// A command's answer, arriving.
#[derive(Debug)]
pub struct CommandRun {
    stream: AiStream,
    manifest: Vec<ManifestEntry>,
    profile: String,
    model: String,
    selection: String,
    text: String,
    stop: Option<StopReason>,
}

impl CommandRun {
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

    /// What was sent: the kind of each piece of context, and its size.
    pub fn manifest(&self) -> &[ManifestEntry] {
        &self.manifest
    }

    pub fn profile(&self) -> &str {
        &self.profile
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn selection(&self) -> &str {
        &self.selection
    }

    /// The answer so far, as the model wrote it.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Why the model stopped, once it has.
    pub fn stop_reason(&self) -> Option<&StopReason> {
        self.stop.as_ref()
    }

    /// The text that would replace the selection.
    pub fn replacement(&self) -> String {
        fit_to_selection(&self.selection, &self.text)
    }

    /// The replacement against the selection, word by word.
    pub fn diff(&self) -> Vec<DiffSpan> {
        diff_words(&self.selection, &self.replacement())
    }
}

/// Shapes a model's answer to the selection it replaces.
///
/// Models add things a text field did not ask for: a code fence around the
/// whole answer, a trailing new line, a leading space. The selection's own
/// white space at either end is what the surrounding document expects, so
/// the answer keeps that instead of its own, and it takes the selection's
/// line endings. A fence is removed only when the selection was not one.
pub fn fit_to_selection(selection: &str, answer: &str) -> String {
    let mut body = answer.trim();
    if !selection.trim_start().starts_with("```") {
        body = unfence(body);
    }
    let body = body.trim();
    if body.is_empty() {
        return String::new();
    }
    let body = if selection.contains("\r\n") {
        body.replace("\r\n", "\n").replace('\n', "\r\n")
    } else {
        body.to_owned()
    };
    let lead = &selection[..selection.len() - selection.trim_start().len()];
    let trail = &selection[selection.trim_end().len()..];
    format!("{lead}{body}{trail}")
}

/// The inside of a Markdown code fence that is the whole of `text`.
fn unfence(text: &str) -> &str {
    let Some(rest) = text.strip_prefix("```") else {
        return text;
    };
    let Some(inner) = rest.strip_suffix("```") else {
        return text;
    };
    // The opening line may name a language; the text starts after it.
    match inner.split_once('\n') {
        Some((info, body)) if !info.contains('`') && !info.trim().contains(' ') => body,
        _ => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_file_is_a_command() {
        let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/commands");
        let mut on_disk: Vec<_> = std::fs::read_dir(folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .filter(|name| name.ends_with(".md"))
            .collect();
        on_disk.sort();
        assert_eq!(on_disk.len(), BUILTIN_FILES.len(), "{on_disk:?}");

        let commands = builtin_commands();
        assert_eq!(commands.len(), BUILTIN_FILES.len());
        for command in &commands {
            assert!(command.builtin);
            assert!(!command.label.is_empty() && !command.instruction.is_empty());
        }
        let mut ids: Vec<_> = commands.iter().map(|command| command.id).collect();
        ids.dedup();
        assert_eq!(ids.len(), commands.len());
    }

    #[test]
    fn the_answer_takes_the_selections_edges_and_line_endings() {
        assert_eq!(
            fit_to_selection("  their going\r\n", "they're going\n\n"),
            "  they're going\r\n"
        );
        assert_eq!(
            fit_to_selection("a\r\nb", "A\nB"),
            "A\r\nB",
            "line endings follow the selection"
        );
        assert_eq!(fit_to_selection("x", "   \n"), "");
    }

    #[test]
    fn a_fence_around_the_whole_answer_is_removed_unless_the_selection_was_one() {
        assert_eq!(fit_to_selection("hi", "```\nHello\n```"), "Hello");
        assert_eq!(fit_to_selection("hi", "```text\nHello\n```"), "Hello");
        assert_eq!(
            fit_to_selection("```\nfn a() {}\n```", "```\nfn b() {}\n```"),
            "```\nfn b() {}\n```"
        );
        // Not a fence around the whole answer.
        assert_eq!(
            fit_to_selection("hi", "Use ```code``` here"),
            "Use ```code``` here"
        );
    }
}

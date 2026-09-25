//! AI actions in the snippet editor (plan 7.3, PRD A1, task 4.7).
//!
//! The editor asks a model to draft a body, proofread it, make it clearer or
//! shorter, change its tone, translate it or suggest variations. The text the
//! action works on — the selection in the editor, or the whole body — is the
//! one piece of context sent, framed as data like any other. Nothing about the
//! snippet is sent unasked: a draft sends only its label and the user's note.
//!
//! What comes back is text for the editor, which the user reads and saves or
//! throws away; the editor puts it in as one edit, so one undo takes it back.
//! A snippet body is a template, so the run also says which placeholders the
//! answer dropped or added: a model that writes `{{clipboard}}` into a body
//! has written something that reads the clipboard when the snippet expands,
//! and the user should know before saving it.

use std::collections::BTreeMap;

use aralo_ai::{
    AiError, AiRequest, AiStream, ContextKind, ContextSource, Delta, Feature, ManifestEntry,
    StopReason,
};

use super::block::fit_block;
use super::command::{fit_to_selection, unfence, CommandProblem, MAX_SELECTION};
use super::{AiSettings, AiSettingsError};
use crate::diff::Change;

/// How many versions [`Authoring::Variations`] asks for.
pub const VARIATIONS: usize = 3;

/// The line a model puts between one variation and the next.
const SEPARATOR: &str = "%%%";

/// What every editor action's model is told, before the action's instruction.
const SYSTEM: &str = "You help a person write snippets for Aralo, a text expander. A snippet is \
text that is typed out for them when they type its abbreviation. It may hold placeholders in \
double braces, such as {{field: name}}, {{date}}, {{clipboard}} or {{cursor}}, which Aralo fills \
in when the snippet expands. Keep every placeholder exactly as it is written, where it still makes \
sense, and do not add placeholders unless you are asked to. Reply with the text and nothing else: \
no introduction, no explanation, no quotation marks around it and no Markdown code fence.";

/// An action in the editor's AI menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Authoring {
    /// Write a body from the snippet's label and what the user adds. Nothing
    /// in the editor is sent.
    Draft {
        label: String,
        note: String,
    },
    /// Spelling, grammar and punctuation, and nothing else.
    Proofread,
    Clearer,
    Shorter,
    Friendlier,
    Formal,
    Casual,
    /// Into a language named in words, such as "German".
    Translate {
        language: String,
    },
    /// [`VARIATIONS`] versions to choose from.
    Variations,
}

impl Authoring {
    /// What a menu calls the action.
    pub fn label(&self) -> String {
        match self {
            Self::Draft { .. } => "Draft with AI".to_owned(),
            Self::Proofread => "Fix spelling and grammar".to_owned(),
            Self::Clearer => "Make it clearer".to_owned(),
            Self::Shorter => "Make it shorter".to_owned(),
            Self::Friendlier => "Make it friendlier".to_owned(),
            Self::Formal => "Make it more formal".to_owned(),
            Self::Casual => "Make it more casual".to_owned(),
            Self::Translate { language } => format!("Translate into {}", language.trim()),
            Self::Variations => "Suggest variations".to_owned(),
        }
    }

    /// Whether the action works on text in the editor, which is then what is
    /// sent. A draft writes from nothing.
    pub fn works_on_text(&self) -> bool {
        !matches!(self, Self::Draft { .. })
    }

    fn instruction(&self) -> String {
        match self {
            Self::Draft { label, note } => {
                let mut instruction = format!(
                    "Write the text of a snippet called \u{201c}{}\u{201d}.",
                    label.trim()
                );
                if !note.trim().is_empty() {
                    instruction.push(' ');
                    instruction.push_str(note.trim());
                }
                instruction.push_str(
                    " Where the person would fill something in each time, such as a name, write \
                     {{field: a-short-name}}. Where they would carry on typing, write {{cursor}}.",
                );
                instruction
            }
            Self::Proofread => {
                "Correct the spelling, grammar and punctuation of this text. Change nothing else."
                    .to_owned()
            }
            Self::Clearer => {
                "Rewrite this text so it is clearer and easier to read. Keep its meaning."
                    .to_owned()
            }
            Self::Shorter => {
                "Make this text shorter. Keep what matters and drop what does not.".to_owned()
            }
            Self::Friendlier => "Rewrite this text to sound warmer and friendlier. Keep its \
                 meaning and roughly its length."
                .to_owned(),
            Self::Formal => "Rewrite this text to sound more formal and professional. Keep its \
                 meaning and roughly its length."
                .to_owned(),
            Self::Casual => "Rewrite this text to sound more casual and relaxed. Keep its \
                 meaning and roughly its length."
                .to_owned(),
            Self::Translate { language } => format!(
                "Translate this text into {}. Leave every placeholder as it is: do not translate \
                 what is inside the double braces.",
                language.trim()
            ),
            Self::Variations => format!(
                "Write {VARIATIONS} different versions of this text, each complete on its own \
                 and keeping its meaning. Put a line holding only {SEPARATOR} between one version \
                 and the next."
            ),
        }
    }
}

/// Why an editor action did not run. Each comes before anything is sent.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AuthoringProblem {
    #[error("there is no text to work on; write some, or select the part to change")]
    NothingToWorkOn,
    #[error("give the snippet a label, or say what it should say, and it can be drafted")]
    NothingToDraft,
    #[error("say which language to translate into")]
    NoLanguage,
    #[error("the text is {bytes} bytes long, and an action sends at most {limit}; select less")]
    TooLong { bytes: usize, limit: usize },
}

/// The text an action works on, and nothing else, for the gateway to ask for.
struct Text<'a>(&'a str);

impl ContextSource for Text<'_> {
    fn fetch(&mut self, kind: ContextKind) -> Option<String> {
        (kind == ContextKind::Selection).then(|| self.0.to_owned())
    }
}

impl AiSettings {
    /// Runs an editor action on `text`: the selection in the editor, or the
    /// whole body. A draft ignores `text` and sends only its label and note.
    ///
    /// It runs on the default profile, through the gateway, as
    /// `Feature::Authoring`: the policy check, framing and the network guard
    /// are the gateway's, as for every feature.
    pub async fn run_authoring(
        &self,
        action: &Authoring,
        text: &str,
    ) -> Result<AuthoringRun, AiSettingsError> {
        match action {
            Authoring::Draft { label, note }
                if label.trim().is_empty() && note.trim().is_empty() =>
            {
                return Err(AuthoringProblem::NothingToDraft.into());
            }
            Authoring::Translate { language } if language.trim().is_empty() => {
                return Err(AuthoringProblem::NoLanguage.into());
            }
            _ => {}
        }
        let original = if action.works_on_text() {
            if text.trim().is_empty() {
                return Err(AuthoringProblem::NothingToWorkOn.into());
            }
            if text.len() > MAX_SELECTION {
                return Err(AuthoringProblem::TooLong {
                    bytes: text.len(),
                    limit: MAX_SELECTION,
                }
                .into());
            }
            text.to_owned()
        } else {
            String::new()
        };
        if !self.switches()?.enabled {
            return Err(AiError::from(aralo_ai::Refusal::Off).into());
        }
        let saved = self.default_profile()?.ok_or(CommandProblem::NoProfile)?;
        let key = self.key_for(&saved.profile)?;
        self.start_authoring(action, original, saved.profile, key, Feature::Authoring)
            .await
    }

    /// Sends an action whose text is checked already, on `profile` with
    /// `key`. The conformance suite's evaluation set runs the editor's
    /// actions this way, on the profile it tests.
    pub(super) async fn start_authoring(
        &self,
        action: &Authoring,
        original: String,
        profile: aralo_ai::Profile,
        key: Option<aralo_ai::Secret>,
        feature: Feature,
    ) -> Result<AuthoringRun, AiSettingsError> {
        let request = AiRequest {
            feature,
            profile,
            model: None,
            system: SYSTEM.to_owned(),
            instruction: action.instruction(),
            declared: if action.works_on_text() {
                vec![ContextKind::Selection]
            } else {
                Vec::new()
            },
            max_tokens: None,
            temperature: None,
            json_output: false,
        };
        let prepared = self.gateway().prepare(request, &mut Text(&original))?;
        let manifest = prepared.manifest().to_vec();
        let profile = prepared.profile().name.clone();
        let model = prepared.chat().model.clone();
        let stream = self.gateway().send(prepared, key).await?;
        Ok(AuthoringRun {
            action: action.clone(),
            stream,
            manifest,
            profile,
            model,
            original,
            text: String::new(),
            stop: None,
        })
    }
}

/// An editor action's answer, arriving.
#[derive(Debug)]
pub struct AuthoringRun {
    action: Authoring,
    stream: AiStream,
    manifest: Vec<ManifestEntry>,
    profile: String,
    model: String,
    original: String,
    text: String,
    stop: Option<StopReason>,
}

impl AuthoringRun {
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

    pub fn action(&self) -> &Authoring {
        &self.action
    }

    /// What was sent with the instruction: the text worked on, and its size.
    pub fn manifest(&self) -> &[ManifestEntry] {
        &self.manifest
    }

    pub fn profile(&self) -> &str {
        &self.profile
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    /// The text the action worked on. Empty for a draft.
    pub fn original(&self) -> &str {
        &self.original
    }

    /// The answer so far, as the model wrote it.
    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn stop_reason(&self) -> Option<&StopReason> {
        self.stop.as_ref()
    }

    /// The text that would replace what the action worked on: the answer
    /// with the original's edges and line endings, or the first variation.
    pub fn answer(&self) -> String {
        self.variations().into_iter().next().unwrap_or_default()
    }

    /// Every version the answer holds: [`VARIATIONS`] of them for
    /// [`Authoring::Variations`] when the model kept to the separator, and
    /// one otherwise. Each is fitted to the original.
    pub fn variations(&self) -> Vec<String> {
        versions(&self.action, &self.original, &self.text)
    }
}

/// What an answer to `action` on `original` comes to, version by version,
/// each fitted to the original: its edges and line endings, or, for a draft,
/// the answer without white space at its ends. A code fence around the whole
/// answer goes, unless the original was fenced too. Empty versions are left
/// out.
pub fn versions(action: &Authoring, original: &str, answer: &str) -> Vec<String> {
    let pieces: Vec<&str> = if *action == Authoring::Variations {
        // A fence around the whole answer goes before it is split, or the
        // first and last versions would each keep half of it.
        let whole = answer.trim();
        let whole = if original.trim_start().starts_with("```") {
            whole
        } else {
            unfence(whole)
        };
        split_variations(whole)
    } else {
        vec![answer]
    };
    pieces
        .into_iter()
        .map(|piece| {
            if original.is_empty() {
                fit_block(piece)
            } else {
                fit_to_selection(original, piece)
            }
        })
        .filter(|piece| !piece.trim().is_empty())
        .collect()
}

/// The versions a variations answer holds, split at lines that hold only the
/// separator. A model that did not use it wrote one version.
fn split_variations(text: &str) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut at = 0;
    for line in text.split_inclusive('\n') {
        if line.trim() == SEPARATOR {
            pieces.push(&text[start..at]);
            start = at + line.len();
        }
        at += line.len();
    }
    pieces.push(&text[start..]);
    pieces
}

/// A placeholder an answer dropped or added, as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaceholderChange {
    /// [`Change::Removed`] for one the answer dropped, [`Change::Added`] for
    /// one it wrote that the original did not have.
    pub change: Change,
    /// The `{{…}}` as it was written.
    pub placeholder: String,
}

/// The placeholders `after` dropped from `before`, then the ones it added, in
/// the order they appear. Two placeholders that read the same once spaces are
/// set aside are one placeholder, so `{{field:name}}` for `{{field: name}}` is
/// no change; one that appears twice has to appear twice.
pub fn placeholder_changes(before: &str, after: &str) -> Vec<PlaceholderChange> {
    let written = |text: &str| -> Vec<(String, String)> {
        let template = aralo_template::parse(text);
        template
            .nodes
            .iter()
            .filter_map(|node| match node {
                aralo_template::Node::Placeholder(placeholder) => {
                    Some((key(placeholder), text[placeholder.span.clone()].to_owned()))
                }
                aralo_template::Node::Text(_) => None,
            })
            .collect()
    };
    let before = written(before);
    let after = written(after);
    let count = |list: &[(String, String)]| {
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        for (key, _) in list {
            *counts.entry(key.clone()).or_default() += 1;
        }
        counts
    };
    let mut changes = Vec::new();
    let mut surplus = |from: &[(String, String)], against: &[(String, String)], change: Change| {
        let mut left = count(against);
        for (key, source) in from {
            match left.get_mut(key) {
                Some(n) if *n > 0 => *n -= 1,
                _ => changes.push(PlaceholderChange {
                    change,
                    placeholder: source.clone(),
                }),
            }
        }
    };
    surplus(&before, &after, Change::Removed);
    surplus(&after, &before, Change::Added);
    changes
}

/// What a placeholder says, with the spacing set aside.
fn key(placeholder: &aralo_template::Placeholder) -> String {
    let mut key = placeholder.name.clone();
    if let Some(argument) = &placeholder.argument {
        key.push(':');
        key.push_str(argument.trim());
    }
    for (option, value) in &placeholder.options {
        key.push('|');
        key.push_str(option.trim());
        key.push(':');
        key.push_str(value.trim());
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variations_split_at_a_separator_line_and_nowhere_else() {
        assert_eq!(
            split_variations("One\n%%%\nTwo\n  %%%  \nThree"),
            ["One\n", "Two\n", "Three"]
        );
        assert_eq!(split_variations("100%%% sure"), ["100%%% sure"]);
    }

    #[test]
    fn a_placeholder_moved_or_respaced_is_no_change() {
        assert_eq!(
            placeholder_changes(
                "Hi {{field: name}}, {{date}}",
                "{{date}}: hello {{field:name}}"
            ),
            []
        );
    }

    #[test]
    fn a_dropped_placeholder_and_an_added_one_are_both_named() {
        let changes = placeholder_changes(
            "Hi {{field: name}}, see {{cursor}}",
            "Hello {{clipboard}}, see {{cursor}} and {{cursor}}",
        );
        assert_eq!(
            changes,
            [
                PlaceholderChange {
                    change: Change::Removed,
                    placeholder: "{{field: name}}".into()
                },
                PlaceholderChange {
                    change: Change::Added,
                    placeholder: "{{clipboard}}".into()
                },
                PlaceholderChange {
                    change: Change::Added,
                    placeholder: "{{cursor}}".into()
                },
            ]
        );
    }

    #[test]
    fn every_action_has_a_label_and_says_what_it_wants() {
        let actions = [
            Authoring::Draft {
                label: "Refund reply".into(),
                note: "Polite.".into(),
            },
            Authoring::Proofread,
            Authoring::Clearer,
            Authoring::Shorter,
            Authoring::Friendlier,
            Authoring::Formal,
            Authoring::Casual,
            Authoring::Translate {
                language: " German ".into(),
            },
            Authoring::Variations,
        ];
        for action in &actions {
            assert!(!action.label().is_empty());
            assert!(!action.instruction().is_empty());
        }
        assert_eq!(actions[7].label(), "Translate into German");
        assert!(actions[0].instruction().contains("Refund reply"));
        assert!(actions[0].instruction().ends_with("write {{cursor}}."));
        assert!(!actions[0].works_on_text() && actions[1].works_on_text());
    }
}

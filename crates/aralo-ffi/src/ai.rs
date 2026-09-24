//! AI settings across the bridge: the switch, the profiles and their keys,
//! Test connection, the capability probe and local-server detection, for the
//! settings pane (plan 7.2, task 4.4); commands on selected text (task 4.5);
//! and the runs that answer a snippet's `{{ai}}` blocks (task 4.6).
//!
//! The calls that talk to an endpoint are `async`. UniFFI runs them on tokio,
//! so Swift awaits them like any other async call and the main thread never
//! waits on the network.
//!
//! A key crosses the bridge in one direction only: from the settings pane's
//! secure field into `save`, `test_connection` or `list_models`. Nothing here
//! returns one.

use std::future::Future;
use std::sync::{Arc, Mutex, PoisonError};

use aralo_core::ai::{
    self as settings, AiError, AiSettings, AiSettingsError, BlockRun, Capabilities, Check, Command,
    CommandRun, ContextKind, KeyChange, LocalServer, ManifestEntry, ProfileDraft, ProfileField,
    SavedProfile, Secret, StopReason,
};
use aralo_core::diff::{self, Change};
use aralo_core::snippet::SnippetId;
use tokio::sync::watch;

use crate::ExpansionSession;

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum AiBridgeError {
    /// The draft cannot be saved or used. `field` is where the pane shows the
    /// message.
    #[error("{message}")]
    Invalid { field: AiField, message: String },
    #[error("{message}")]
    NotFound { message: String },
    /// The policy stopped the request before anything was sent: AI is off,
    /// local-only mode is on, or an administrator does not allow the host.
    #[error("{message}")]
    Refused { message: String },
    /// The endpoint, the network, the keychain or the settings file failed.
    #[error("{message}")]
    Failed { message: String },
}

impl From<AiSettingsError> for AiBridgeError {
    fn from(error: AiSettingsError) -> Self {
        let message = error.to_string();
        match error {
            AiSettingsError::Invalid { field, .. } => Self::Invalid {
                field: field.into(),
                message,
            },
            AiSettingsError::NotFound(_) => Self::NotFound { message },
            AiSettingsError::Ai(settings::AiError::Refused(_)) => Self::Refused { message },
            _ => Self::Failed { message },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum AiField {
    Name,
    Adapter,
    BaseUrl,
    Model,
    Headers,
    Key,
}

impl From<ProfileField> for AiField {
    fn from(field: ProfileField) -> Self {
        match field {
            ProfileField::Name => Self::Name,
            ProfileField::Adapter => Self::Adapter,
            ProfileField::BaseUrl => Self::BaseUrl,
            ProfileField::Model => Self::Model,
            ProfileField::Headers => Self::Headers,
            ProfileField::Key => Self::Key,
        }
    }
}

/// Where keys are kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum KeyStorage {
    /// The system keychain. What the app uses.
    Keychain,
    /// In memory, gone when the object is. For tests and previews.
    Memory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct AiSwitches {
    pub enabled: bool,
    pub local_only: bool,
}

/// A header sent with every request, such as OpenRouter's `X-Title`.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AiHeader {
    pub name: String,
    pub value: String,
}

/// A profile as the editor holds it. Only OpenAI-compatible endpoints exist so
/// far, so there is no adapter to choose.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AiProfileDraft {
    /// The name it was saved under, when this edits a saved profile.
    pub original_name: Option<String>,
    pub name: String,
    pub base_url: String,
    pub default_model: String,
    pub headers: Vec<AiHeader>,
}

/// What to do with the profile's key.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum AiKeyChange {
    /// The key saved with `original_name`, if any.
    Keep,
    /// This key, from the secure field. Blank is the same as `Remove`.
    Set {
        key: String,
    },
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AiProfile {
    pub name: String,
    pub base_url: String,
    pub default_model: String,
    pub headers: Vec<AiHeader>,
    pub has_key: bool,
    pub is_default: bool,
    /// Whether the endpoint is on this Mac. Local-only mode allows only these.
    pub is_local: bool,
    /// The last probe's findings, until something they depended on changes.
    pub capabilities: Option<AiCapabilities>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum AiCheck {
    Yes,
    No { reason: String },
    NotChecked { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AiCapabilities {
    pub models_route: AiCheck,
    pub streaming: AiCheck,
    pub system_prompt: AiCheck,
    pub json_output: AiCheck,
    pub embeddings: AiCheck,
    /// RFC 3339. Empty on a result that has not been saved.
    pub probed_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AiConnectionReport {
    pub model: String,
    /// From sending the request to the first word of the answer.
    pub first_token_ms: u64,
    /// The start of what the model said.
    pub reply: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AiLocalServer {
    /// Such as "Ollama".
    pub name: String,
    pub base_url: String,
    pub models: Vec<String>,
}

/// A provider the editor offers by name.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AiProviderPreset {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub default_model: String,
    /// Where the user makes a key; none for a server that needs none.
    pub key_page: Option<String>,
}

/// The AI settings. One per app: it owns the gateway every AI request goes
/// through, so the switch and local-only mode apply the moment they change.
#[derive(uniffi::Object)]
pub struct AiProfiles {
    settings: AiSettings,
}

impl std::fmt::Debug for AiProfiles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AiProfiles").finish_non_exhaustive()
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl AiProfiles {
    /// Opens the settings in `path`, or in `profiles.toml` in Aralo's state
    /// folder when that is left out.
    #[uniffi::constructor]
    pub fn open(path: Option<String>, keys: KeyStorage) -> Result<Arc<Self>, AiBridgeError> {
        let path = match path {
            Some(path) => path.into(),
            None => settings::profiles_path().ok_or_else(|| AiBridgeError::Failed {
                message: "cannot find Aralo's state folder".into(),
            })?,
        };
        let settings = match keys {
            KeyStorage::Keychain => AiSettings::open(path)?,
            KeyStorage::Memory => {
                AiSettings::open_with_secrets(path, Arc::new(settings::MemorySecretStore::new()))?
            }
        };
        Ok(Arc::new(Self { settings }))
    }

    /// Reads the file again, after an edit made outside the app.
    pub fn reload(&self) -> Result<(), AiBridgeError> {
        Ok(self.settings.reload()?)
    }

    pub fn switches(&self) -> Result<AiSwitches, AiBridgeError> {
        let switches = self.settings.switches()?;
        Ok(AiSwitches {
            enabled: switches.enabled,
            local_only: switches.local_only,
        })
    }

    pub fn set_switches(&self, switches: AiSwitches) -> Result<(), AiBridgeError> {
        Ok(self.settings.set_switches(settings::AiSwitches {
            enabled: switches.enabled,
            local_only: switches.local_only,
        })?)
    }

    /// The saved profiles, in the order they were added.
    pub fn profiles(&self) -> Result<Vec<AiProfile>, AiBridgeError> {
        Ok(self
            .settings
            .profiles()?
            .iter()
            .map(AiProfile::from)
            .collect())
    }

    /// Checks the draft and saves it, with its key in the keychain. The first
    /// profile saved becomes the default.
    pub fn save(
        &self,
        draft: AiProfileDraft,
        key: AiKeyChange,
    ) -> Result<AiProfile, AiBridgeError> {
        Ok((&self.settings.save(&draft.into(), key.into())?).into())
    }

    /// Deletes the profile and its key.
    pub fn delete(&self, name: String) -> Result<(), AiBridgeError> {
        Ok(self.settings.delete(&name)?)
    }

    pub fn set_default(&self, name: String) -> Result<(), AiBridgeError> {
        Ok(self.settings.set_default_profile(Some(&name))?)
    }

    /// One short chat with the draft, saved or not.
    pub async fn test_connection(
        &self,
        draft: AiProfileDraft,
        key: AiKeyChange,
    ) -> Result<AiConnectionReport, AiBridgeError> {
        let report = self
            .settings
            .test_connection(&draft.into(), key.into())
            .await?;
        Ok(AiConnectionReport {
            model: report.model,
            first_token_ms: u64::try_from(report.first_token.as_millis()).unwrap_or(u64::MAX),
            reply: report.reply,
        })
    }

    /// The models the draft's endpoint lists, for the model picker.
    pub async fn list_models(
        &self,
        draft: AiProfileDraft,
        key: AiKeyChange,
    ) -> Result<Vec<String>, AiBridgeError> {
        Ok(self.settings.list_models(&draft.into(), key.into()).await?)
    }

    /// Probes a saved profile and keeps the result with it.
    pub async fn probe_capabilities(&self, name: String) -> Result<AiCapabilities, AiBridgeError> {
        let found = self.settings.probe(&name).await?;
        Ok(capabilities(&found, String::new()))
    }

    /// Model servers running on this Mac.
    pub async fn detect_local_servers(&self) -> Result<Vec<AiLocalServer>, AiBridgeError> {
        Ok(self
            .settings
            .detect_local_servers()
            .await?
            .iter()
            .map(AiLocalServer::from)
            .collect())
    }

    /// Runs a command on the selected text. The answer streams from the run
    /// this returns; nothing is replaced until the shell pastes it.
    pub async fn run_command(
        &self,
        command: AiCommand,
        selection: String,
    ) -> Result<Arc<AiCommandRun>, AiBridgeError> {
        let command = Command::try_from(command)?;
        let run = self.settings.run_command(&command, &selection).await?;
        Ok(Arc::new(AiCommandRun::new(run)))
    }

    /// Asks a model for one `{{ai}}` block of a session that is on its AI
    /// step. The answer streams from the run this returns; nothing goes in
    /// until the shell settles the block with `answer_block`.
    ///
    /// Only the context the snippet declared is sent, from what the session
    /// already holds. An error — AI off, local-only mode, no profile, the
    /// network — comes back before anything is sent where it can, and is
    /// the shell's cue to call `fall_back` with its message.
    pub async fn run_block(
        &self,
        session: Arc<ExpansionSession>,
        index: u32,
    ) -> Result<Arc<AiBlockRun>, AiBridgeError> {
        let request = session
            .held()
            .as_ref()
            .and_then(|session| session.block_request(index as usize))
            .ok_or_else(|| AiBridgeError::NotFound {
                message: "that expansion has no such AI block, or it has ended".into(),
            })?;
        let run = self.settings.run_block(&request).await?;
        Ok(Arc::new(AiBlockRun::new(run)))
    }
}

/// The providers the editor offers by name, with where to make a key.
#[uniffi::export]
pub fn ai_provider_presets() -> Vec<AiProviderPreset> {
    settings::PROVIDER_PRESETS
        .iter()
        .map(|preset| AiProviderPreset {
            id: preset.id.into(),
            name: preset.name.into(),
            base_url: preset.base_url.into(),
            default_model: preset.default_model.into(),
            key_page: preset.key_page.map(str::to_owned),
        })
        .collect()
}

impl From<AiProfileDraft> for ProfileDraft {
    fn from(draft: AiProfileDraft) -> Self {
        ProfileDraft {
            original_name: draft.original_name,
            name: draft.name,
            adapter: settings::AdapterKind::OpenAiCompat,
            base_url: draft.base_url,
            default_model: draft.default_model,
            headers: draft
                .headers
                .into_iter()
                .map(|header| (header.name, header.value))
                .collect(),
        }
    }
}

impl From<AiKeyChange> for KeyChange {
    fn from(change: AiKeyChange) -> Self {
        match change {
            AiKeyChange::Keep => KeyChange::Keep,
            AiKeyChange::Set { key } => KeyChange::Set(Secret::new(key)),
            AiKeyChange::Remove => KeyChange::Remove,
        }
    }
}

impl From<&SavedProfile> for AiProfile {
    fn from(saved: &SavedProfile) -> Self {
        let profile = &saved.profile;
        AiProfile {
            name: profile.name.clone(),
            base_url: profile.base_url.clone(),
            default_model: profile.default_model.clone(),
            headers: profile
                .headers
                .iter()
                .map(|(name, value)| AiHeader {
                    name: name.clone(),
                    value: value.clone(),
                })
                .collect(),
            has_key: saved.has_key(),
            is_default: saved.is_default,
            is_local: settings::is_local(&profile.base_url),
            capabilities: saved
                .capabilities
                .as_ref()
                .map(|(found, when)| capabilities(found, when.clone())),
        }
    }
}

impl From<&LocalServer> for AiLocalServer {
    fn from(server: &LocalServer) -> Self {
        AiLocalServer {
            name: server.kind.display_name().into(),
            base_url: server.base_url.clone(),
            models: server.models.clone(),
        }
    }
}

fn capabilities(found: &Capabilities, probed_at: String) -> AiCapabilities {
    AiCapabilities {
        models_route: check(&found.models_route),
        streaming: check(&found.streaming),
        system_prompt: check(&found.system_prompt),
        json_output: check(&found.json_output),
        embeddings: check(&found.embeddings),
        probed_at,
    }
}

fn check(check: &Check) -> AiCheck {
    match check {
        Check::Yes => AiCheck::Yes,
        Check::No(reason) => AiCheck::No {
            reason: reason.clone(),
        },
        Check::NotChecked(reason) => AiCheck::NotChecked {
            reason: reason.clone(),
        },
    }
}

// MARK: Commands on selected text (task 4.5)

/// A command the palette lists: a built-in one, or a `type: command` snippet.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AiCommand {
    pub id: String,
    pub label: String,
    /// What the model is asked to do with the selection.
    pub instruction: String,
    pub tags: Vec<String>,
    /// The profile it runs on; nothing is the default profile.
    pub profile: Option<String>,
    /// The model it asks for; nothing is the profile's.
    pub model: Option<String>,
    pub builtin: bool,
}

impl From<&Command> for AiCommand {
    fn from(command: &Command) -> Self {
        Self {
            id: command.id.to_string(),
            label: command.label.clone(),
            instruction: command.instruction.clone(),
            tags: command.tags.clone(),
            profile: command.profile.clone(),
            model: command.model.clone(),
            builtin: command.builtin,
        }
    }
}

impl TryFrom<AiCommand> for Command {
    type Error = AiBridgeError;

    fn try_from(command: AiCommand) -> Result<Self, Self::Error> {
        let id = command
            .id
            .parse::<SnippetId>()
            .map_err(|_| AiBridgeError::NotFound {
                message: format!("\u{201c}{}\u{201d} is not a command id", command.id),
            })?;
        Ok(Self {
            id,
            label: command.label,
            instruction: command.instruction,
            tags: command.tags,
            profile: command.profile,
            model: command.model,
            builtin: command.builtin,
        })
    }
}

/// A kind of context a request carried.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum AiContextKind {
    Fillins,
    Selection,
    Clipboard,
    App,
    Window,
}

impl From<ContextKind> for AiContextKind {
    fn from(kind: ContextKind) -> Self {
        match kind {
            ContextKind::Fillins => Self::Fillins,
            ContextKind::Selection => Self::Selection,
            ContextKind::Clipboard => Self::Clipboard,
            ContextKind::App => Self::App,
            ContextKind::Window => Self::Window,
        }
    }
}

/// One line of what a request sent: the kind, and how many bytes of it. The
/// chip beside the answer is drawn from these.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AiContextSent {
    pub kind: AiContextKind,
    /// Nothing when the kind was declared but there was none to send.
    pub bytes: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DiffChange {
    Same,
    Removed,
    Added,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DiffSpan {
    pub change: DiffChange,
    pub text: String,
}

impl From<diff::DiffSpan> for DiffSpan {
    fn from(span: diff::DiffSpan) -> Self {
        Self {
            change: match span.change {
                Change::Same => DiffChange::Same,
                Change::Removed => DiffChange::Removed,
                Change::Added => DiffChange::Added,
            },
            text: span.text,
        }
    }
}

/// `before` against `after`, word by word: the removed spans and the same
/// ones are `before`, the added spans and the same ones are `after`. For the
/// preview after the user edits an answer by hand.
#[uniffi::export]
pub fn diff_words(before: String, after: String) -> Vec<DiffSpan> {
    diff::diff_words(&before, &after)
        .into_iter()
        .map(DiffSpan::from)
        .collect()
}

/// An answer arriving in pieces: a command's or a block's. `next` gives each
/// piece as it comes; `cancel` stops it from anywhere, including while `next`
/// waits, and closes the connection.
struct Streamed<R> {
    run: tokio::sync::Mutex<Option<R>>,
    cancel: watch::Sender<bool>,
    text: Mutex<String>,
    cut_short: Mutex<bool>,
}

/// What a command run and a block run have in common.
trait Answering: Send + 'static {
    fn next_piece(&mut self) -> impl Future<Output = Option<Result<String, AiError>>> + Send;
    fn stop_reason(&self) -> Option<&StopReason>;
}

impl Answering for CommandRun {
    fn next_piece(&mut self) -> impl Future<Output = Option<Result<String, AiError>>> + Send {
        self.next()
    }

    fn stop_reason(&self) -> Option<&StopReason> {
        CommandRun::stop_reason(self)
    }
}

impl Answering for BlockRun {
    fn next_piece(&mut self) -> impl Future<Output = Option<Result<String, AiError>>> + Send {
        self.next()
    }

    fn stop_reason(&self) -> Option<&StopReason> {
        BlockRun::stop_reason(self)
    }
}

impl<R: Answering> Streamed<R> {
    fn new(run: R) -> Self {
        Self {
            run: tokio::sync::Mutex::new(Some(run)),
            cancel: watch::channel(false).0,
            text: Mutex::default(),
            cut_short: Mutex::default(),
        }
    }

    async fn next(&self) -> Result<Option<String>, AiBridgeError> {
        let mut cancelled = self.cancel.subscribe();
        if *cancelled.borrow() {
            return Ok(None);
        }
        let mut slot = self.run.lock().await;
        let Some(run) = slot.as_mut() else {
            return Ok(None);
        };
        let piece = tokio::select! {
            piece = run.next_piece() => piece,
            _ = cancelled.wait_for(|stop| *stop) => {
                *slot = None;
                return Ok(None);
            }
        };
        match piece {
            Some(Ok(piece)) => {
                self.text
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push_str(&piece);
                Ok(Some(piece))
            }
            Some(Err(error)) => {
                *slot = None;
                Err(AiSettingsError::from(error).into())
            }
            None => {
                *self
                    .cut_short
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner) =
                    matches!(run.stop_reason(), Some(StopReason::Length));
                *slot = None;
                Ok(None)
            }
        }
    }

    fn cancel(&self) {
        self.cancel.send_replace(true);
        if let Ok(mut slot) = self.run.try_lock() {
            *slot = None;
        }
    }

    fn text(&self) -> String {
        self.text
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn cut_short(&self) -> bool {
        *self
            .cut_short
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

fn sent(manifest: &[ManifestEntry]) -> Vec<AiContextSent> {
    manifest
        .iter()
        .map(|entry| AiContextSent {
            kind: entry.kind.into(),
            bytes: entry.bytes.map(|bytes| bytes as u64),
        })
        .collect()
}

/// A command's answer, arriving. `next` gives each piece as it comes;
/// `cancel` stops it from anywhere, including while `next` waits, and closes
/// the connection.
#[derive(uniffi::Object)]
pub struct AiCommandRun {
    stream: Streamed<CommandRun>,
    profile: String,
    model: String,
    sent: Vec<AiContextSent>,
    selection: String,
}

impl std::fmt::Debug for AiCommandRun {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AiCommandRun")
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

impl AiCommandRun {
    fn new(run: CommandRun) -> Self {
        Self {
            profile: run.profile().to_owned(),
            model: run.model().to_owned(),
            sent: sent(run.manifest()),
            selection: run.selection().to_owned(),
            stream: Streamed::new(run),
        }
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl AiCommandRun {
    /// The next piece of the answer, or nothing once it has finished or been
    /// cancelled. An error ends it; what came before stays in `text`.
    pub async fn next(&self) -> Result<Option<String>, AiBridgeError> {
        self.stream.next().await
    }

    /// Stops the answer and closes the connection. What arrived is kept.
    pub fn cancel(&self) {
        self.stream.cancel();
    }

    pub fn profile(&self) -> String {
        self.profile.clone()
    }

    pub fn model(&self) -> String {
        self.model.clone()
    }

    /// What was sent with the instruction.
    pub fn sent(&self) -> Vec<AiContextSent> {
        self.sent.clone()
    }

    /// The answer so far, as the model wrote it.
    pub fn text(&self) -> String {
        self.stream.text()
    }

    /// Whether the model stopped at its length limit rather than at the end.
    pub fn cut_short(&self) -> bool {
        self.stream.cut_short()
    }

    /// The text that would replace the selection: the answer without a fence
    /// the selection did not have, with the selection's edges.
    pub fn replacement(&self) -> String {
        settings::fit_to_selection(&self.selection, &self.stream.text())
    }

    /// The replacement against the selection.
    pub fn diff(&self) -> Vec<DiffSpan> {
        diff_words(self.selection.clone(), self.replacement())
    }
}

// MARK: AI blocks in snippets (task 4.6)

/// A block's answer, arriving, on the same terms as a command's. Nothing goes
/// in until the shell hands the answer to `ExpansionSession.answer_block`.
#[derive(uniffi::Object)]
pub struct AiBlockRun {
    stream: Streamed<BlockRun>,
    block: u32,
    profile: String,
    model: String,
    sent: Vec<AiContextSent>,
}

impl std::fmt::Debug for AiBlockRun {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AiBlockRun")
            .field("block", &self.block)
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

impl AiBlockRun {
    fn new(run: BlockRun) -> Self {
        Self {
            block: u32::try_from(run.block()).unwrap_or(u32::MAX),
            profile: run.profile().to_owned(),
            model: run.model().to_owned(),
            sent: sent(run.manifest()),
            stream: Streamed::new(run),
        }
    }
}

#[uniffi::export(async_runtime = "tokio")]
impl AiBlockRun {
    /// The next piece of the answer, or nothing once it has finished or been
    /// cancelled. An error ends it; what came before stays in `text`.
    pub async fn next(&self) -> Result<Option<String>, AiBridgeError> {
        self.stream.next().await
    }

    /// Stops the answer and closes the connection. What arrived is kept.
    pub fn cancel(&self) {
        self.stream.cancel();
    }

    /// Which block this answers.
    pub fn block(&self) -> u32 {
        self.block
    }

    pub fn profile(&self) -> String {
        self.profile.clone()
    }

    pub fn model(&self) -> String {
        self.model.clone()
    }

    /// What was sent with the prompt: the declared context, kind and size.
    pub fn sent(&self) -> Vec<AiContextSent> {
        self.sent.clone()
    }

    /// The answer so far, as the model wrote it.
    pub fn text(&self) -> String {
        self.stream.text()
    }

    /// The answer as it would go in: without white space at its ends or a
    /// code fence around the whole. Nothing while there is nothing to put in.
    pub fn answer(&self) -> Option<String> {
        let answer = settings::fit_block(&self.stream.text());
        (!answer.is_empty()).then_some(answer)
    }

    /// Whether the model stopped at its length limit rather than at the end.
    pub fn cut_short(&self) -> bool {
        self.stream.cut_short()
    }
}

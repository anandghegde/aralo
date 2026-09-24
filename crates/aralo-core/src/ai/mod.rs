//! AI settings: the switch, local-only mode and the saved profiles, with
//! their keys in the keychain (plan 7.2, PRD P13).
//!
//! [`AiSettings`] is what the settings pane and `aralo ai` call. It owns the
//! [`Gateway`], so Test connection, the capability probe and the model list
//! go through the same policy check and network guard as any feature.

mod block;
mod command;
mod file;
mod keychain;
mod presets;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use aralo_ai::guard::NetworkGuard;
use aralo_ai::{Gateway, Policy, Transport};

// What a caller of these settings needs from the layers below, so the app and
// the command line depend on the core alone.
pub use aralo_ai::{
    AdapterKind, AiError, Capabilities, Check, ConnectionReport, ContextKind, ManifestEntry,
    MemorySecretStore, Profile, Refusal, Secret, SecretError, SecretStore, StopReason,
};
pub use aralo_providers::{LocalServer, LocalServerKind};

pub use block::{fit_block, BlockRequest, BlockRun, BlockSettings};
pub use command::{
    builtin_commands, fit_to_selection, Command, CommandProblem, CommandRun, MAX_SELECTION,
};
use file::{same_name, ProfilesFile, StoredCapabilities, StoredProfile};
pub use keychain::{KeychainStore, KEYCHAIN_SERVICE};
pub use presets::{provider_preset, ProviderPreset, PROVIDER_PRESETS};

/// The file's name in the state folder.
pub const PROFILES_FILE: &str = "profiles.toml";
const MAX_NAME: usize = 64;

/// Header names that carry credentials. A key goes in the key field, where
/// it is kept in the keychain; in a header it would be written to
/// `profiles.toml`.
const CREDENTIAL_HEADERS: [&str; 6] = [
    "authorization",
    "proxy-authorization",
    "x-api-key",
    "api-key",
    "cookie",
    "x-goog-api-key",
];

/// Where `profiles.toml` belongs: the state folder, never the library.
pub fn profiles_path() -> Option<PathBuf> {
    Some(crate::state::state_folder()?.join(PROFILES_FILE))
}

/// A profile as the settings pane edits it. The key is passed beside it, as a
/// [`KeyChange`], and never stored in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileDraft {
    /// The name the profile was saved under, when this edits one. A rename
    /// keeps the key.
    pub original_name: Option<String>,
    pub name: String,
    pub adapter: AdapterKind,
    pub base_url: String,
    pub default_model: String,
    pub headers: Vec<(String, String)>,
}

/// What to do with a profile's key.
#[derive(Debug)]
pub enum KeyChange {
    /// Use the key saved for `original_name`, if there is one.
    Keep,
    /// Use this key. A blank one is the same as [`KeyChange::Remove`].
    Set(Secret),
    Remove,
}

/// A saved profile as the settings pane shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedProfile {
    pub profile: Profile,
    pub is_default: bool,
    /// What the last probe found, and when (RFC 3339). `None` until one ran,
    /// and again after a change it depended on.
    pub capabilities: Option<(Capabilities, String)>,
}

impl SavedProfile {
    /// The profile as a draft that edits it.
    pub fn draft(&self) -> ProfileDraft {
        ProfileDraft {
            original_name: Some(self.profile.name.clone()),
            name: self.profile.name.clone(),
            adapter: self.profile.adapter,
            base_url: self.profile.base_url.clone(),
            default_model: self.profile.default_model.clone(),
            headers: self.profile.headers.clone(),
        }
    }

    pub fn has_key(&self) -> bool {
        self.profile.key_ref.is_some()
    }
}

/// Whether an endpoint is on this machine, which is what local-only mode
/// allows. An address that does not parse is not.
pub fn is_local(base_url: &str) -> bool {
    aralo_ai::guard::parse_endpoint(base_url).is_ok_and(|endpoint| endpoint.host.is_loopback())
}

/// The switch and local-only mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AiSwitches {
    pub enabled: bool,
    pub local_only: bool,
}

/// The field a problem is about, so the pane can put the message beside it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileField {
    Name,
    Adapter,
    BaseUrl,
    Model,
    Headers,
    Key,
}

#[derive(Debug, thiserror::Error)]
pub enum AiSettingsError {
    #[error("{message}")]
    Invalid {
        field: ProfileField,
        message: String,
    },
    #[error("there is no profile named \u{201c}{0}\u{201d}")]
    NotFound(String),
    #[error("{path}: {message}")]
    File { path: String, message: String },
    #[error(transparent)]
    Secret(#[from] SecretError),
    #[error(transparent)]
    Ai(#[from] AiError),
    #[error("{0}")]
    Command(#[from] CommandProblem),
}

impl From<Refusal> for AiSettingsError {
    fn from(refusal: Refusal) -> Self {
        Self::Ai(refusal.into())
    }
}

fn invalid(field: ProfileField, message: impl Into<String>) -> AiSettingsError {
    AiSettingsError::Invalid {
        field,
        message: message.into(),
    }
}

pub struct AiSettings {
    path: PathBuf,
    secrets: Arc<dyn SecretStore>,
    gateway: Gateway,
    /// Set when the gateway sends through the real guard, which enforces
    /// local-only mode a second time, at the socket.
    guard: Option<Arc<NetworkGuard>>,
    transport: Arc<dyn Transport>,
    /// Held while the file is read, changed and written, so two changes from
    /// this process cannot interleave.
    lock: Mutex<()>,
    clock: fn() -> String,
}

impl std::fmt::Debug for AiSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AiSettings")
            .field("path", &self.path)
            .field("gateway", &self.gateway)
            .finish_non_exhaustive()
    }
}

impl AiSettings {
    /// The settings in `path`, with keys in the system keychain and requests
    /// through the network guard.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, AiSettingsError> {
        Self::open_with_secrets(path, Arc::new(KeychainStore::new()))
    }

    /// The same, with keys in a store the shell provides.
    pub fn open_with_secrets(
        path: impl Into<PathBuf>,
        secrets: Arc<dyn SecretStore>,
    ) -> Result<Self, AiSettingsError> {
        let path = path.into();
        let file = read(&path)?;
        let guard = Arc::new(NetworkGuard::new(file.local_only)?);
        let mut settings = Self::assemble(path, secrets, guard.clone(), &file);
        settings.guard = Some(guard);
        Ok(settings)
    }

    /// Settings that send through `transport` instead of the guard. For
    /// tests: nothing but the guard may reach the network.
    pub fn with_transport(
        path: impl Into<PathBuf>,
        secrets: Arc<dyn SecretStore>,
        transport: Arc<dyn Transport>,
    ) -> Result<Self, AiSettingsError> {
        let path = path.into();
        let file = read(&path)?;
        Ok(Self::assemble(path, secrets, transport, &file))
    }

    fn assemble(
        path: PathBuf,
        secrets: Arc<dyn SecretStore>,
        transport: Arc<dyn Transport>,
        file: &ProfilesFile,
    ) -> Self {
        let mut gateway = Gateway::new(policy(file), transport.clone());
        aralo_providers::register_all(&mut gateway);
        Self {
            path,
            secrets,
            gateway,
            guard: None,
            transport,
            lock: Mutex::new(()),
            clock: now,
        }
    }

    /// Replaces the clock that stamps probe results, for tests.
    pub fn with_clock(mut self, clock: fn() -> String) -> Self {
        self.clock = clock;
        self
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The gateway, for the features that send through it.
    pub fn gateway(&self) -> &Gateway {
        &self.gateway
    }

    /// Reads the file again, for a change made outside this process, and
    /// applies its switches.
    pub fn reload(&self) -> Result<(), AiSettingsError> {
        let file = read(&self.path)?;
        self.apply(&file)
    }

    pub fn switches(&self) -> Result<AiSwitches, AiSettingsError> {
        let file = read(&self.path)?;
        Ok(AiSwitches {
            enabled: file.enabled,
            local_only: file.local_only,
        })
    }

    pub fn set_switches(&self, switches: AiSwitches) -> Result<(), AiSettingsError> {
        self.change(|file| {
            file.enabled = switches.enabled;
            file.local_only = switches.local_only;
            Ok(())
        })
    }

    /// Every saved profile, in the order they were added.
    pub fn profiles(&self) -> Result<Vec<SavedProfile>, AiSettingsError> {
        let file = read(&self.path)?;
        Ok(file
            .profiles
            .iter()
            .map(|stored| saved(stored, file.default_profile.as_deref()))
            .collect())
    }

    pub fn profile(&self, name: &str) -> Result<SavedProfile, AiSettingsError> {
        let file = read(&self.path)?;
        let stored = file
            .find(name)
            .ok_or_else(|| AiSettingsError::NotFound(name.into()))?;
        Ok(saved(stored, file.default_profile.as_deref()))
    }

    /// The profile features use when nothing names another.
    pub fn default_profile(&self) -> Result<Option<SavedProfile>, AiSettingsError> {
        let file = read(&self.path)?;
        Ok(file
            .default_profile
            .as_deref()
            .and_then(|name| file.find(name))
            .map(|stored| saved(stored, file.default_profile.as_deref())))
    }

    pub fn set_default_profile(&self, name: Option<&str>) -> Result<(), AiSettingsError> {
        self.change(|file| {
            file.default_profile = match name {
                Some(name) => Some(
                    file.find(name)
                        .ok_or_else(|| AiSettingsError::NotFound(name.into()))?
                        .name
                        .clone(),
                ),
                None => None,
            };
            Ok(())
        })
    }

    /// Checks a draft and saves it, with its key in the secret store. The
    /// first profile saved becomes the default. A saved probe result is kept
    /// only when nothing it depended on changed; the caller probes again with
    /// [`AiSettings::probe`].
    pub fn save(
        &self,
        draft: &ProfileDraft,
        key: KeyChange,
    ) -> Result<SavedProfile, AiSettingsError> {
        let draft = checked(draft)?;
        let _held = self.hold();
        let mut file = read(&self.path)?;

        let original = match &draft.original_name {
            Some(original) => Some(
                file.find(original)
                    .ok_or_else(|| AiSettingsError::NotFound(original.clone()))?
                    .clone(),
            ),
            None => None,
        };
        let clash = file.profiles.iter().any(|existing| {
            same_name(&existing.name, &draft.name)
                && original
                    .as_ref()
                    .is_none_or(|original| !same_name(&existing.name, &original.name))
        });
        if clash {
            return Err(invalid(
                ProfileField::Name,
                format!(
                    "a profile named \u{201c}{}\u{201d} exists already",
                    draft.name
                ),
            ));
        }

        let old_ref = original
            .as_ref()
            .and_then(|profile| profile.key_ref.clone());
        let (key_ref, key_changed) = match key {
            KeyChange::Keep => (old_ref.clone(), false),
            KeyChange::Set(secret) if !secret.expose().trim().is_empty() => {
                let key_ref = old_ref.clone().unwrap_or_else(new_key_ref);
                self.secrets
                    .set(&key_ref, &Secret::new(secret.expose().trim()))?;
                (Some(key_ref), true)
            }
            KeyChange::Set(_) | KeyChange::Remove => (None, old_ref.is_some()),
        };

        let mut stored = StoredProfile {
            name: draft.name.clone(),
            adapter: draft.adapter.as_str().into(),
            base_url: draft.base_url.clone(),
            default_model: draft.default_model.clone(),
            key_ref: key_ref.clone(),
            headers: draft.headers.iter().cloned().collect(),
            capabilities: None,
        };
        if let Some(original) = &original {
            let same_endpoint = original.adapter == stored.adapter
                && original.base_url == stored.base_url
                && original.default_model == stored.default_model
                && original.headers == stored.headers;
            if same_endpoint && !key_changed {
                stored.capabilities = original.capabilities.clone();
            }
        }

        match &original {
            Some(original) => {
                let slot = file
                    .find_mut(&original.name)
                    .ok_or_else(|| AiSettingsError::NotFound(original.name.clone()))?;
                *slot = stored.clone();
                if file
                    .default_profile
                    .as_deref()
                    .is_some_and(|name| same_name(name, &original.name))
                {
                    file.default_profile = Some(stored.name.clone());
                }
            }
            None => file.profiles.push(stored.clone()),
        }
        if file.default_profile.is_none() {
            file.default_profile = Some(stored.name.clone());
        }

        if let Err(error) = write(&self.path, &file) {
            // Leave no key behind for a profile that was never saved.
            if original.is_none() {
                if let Some(key_ref) = &key_ref {
                    let _ = self.secrets.delete(key_ref);
                }
            }
            return Err(error);
        }
        if key_ref.is_none() {
            if let Some(old_ref) = &old_ref {
                self.secrets.delete(old_ref)?;
            }
        }
        Ok(saved(&stored, file.default_profile.as_deref()))
    }

    /// Removes a profile and its key.
    pub fn delete(&self, name: &str) -> Result<(), AiSettingsError> {
        let mut key_ref = None;
        self.change(|file| {
            let index = file
                .profiles
                .iter()
                .position(|profile| same_name(&profile.name, name))
                .ok_or_else(|| AiSettingsError::NotFound(name.into()))?;
            let removed = file.profiles.remove(index);
            if file
                .default_profile
                .as_deref()
                .is_some_and(|default| same_name(default, &removed.name))
            {
                file.default_profile = file.profiles.first().map(|first| first.name.clone());
            }
            key_ref = removed.key_ref;
            Ok(())
        })?;
        if let Some(key_ref) = key_ref {
            self.secrets.delete(&key_ref)?;
        }
        Ok(())
    }

    /// One short chat with a draft, saved or not, to show the key and the
    /// model work together.
    pub async fn test_connection(
        &self,
        draft: &ProfileDraft,
        key: KeyChange,
    ) -> Result<ConnectionReport, AiSettingsError> {
        let (profile, key) = self.resolve(draft, key)?;
        Ok(self.gateway.test_connection(&profile, key).await?)
    }

    /// The models a draft's endpoint lists.
    pub async fn list_models(
        &self,
        draft: &ProfileDraft,
        key: KeyChange,
    ) -> Result<Vec<String>, AiSettingsError> {
        let (profile, key) = self.resolve(draft, key)?;
        Ok(self.gateway.list_models(&profile, key).await?)
    }

    /// Probes a saved profile and keeps what it found with the profile.
    pub async fn probe(&self, name: &str) -> Result<Capabilities, AiSettingsError> {
        let stored = read(&self.path)?
            .find(name)
            .cloned()
            .ok_or_else(|| AiSettingsError::NotFound(name.into()))?;
        let profile = to_profile(&stored)?;
        let key = self.key_for(&profile)?;
        let found = self.gateway.probe(&profile, key).await?;

        let probed_at = (self.clock)();
        self.change(|file| {
            // Keep the result only if the profile is still the one probed.
            if let Some(current) = file.find_mut(name) {
                let unchanged = current.base_url == stored.base_url
                    && current.default_model == stored.default_model
                    && current.headers == stored.headers
                    && current.key_ref == stored.key_ref;
                if unchanged {
                    current.capabilities = Some(StoredCapabilities::new(&found, probed_at));
                }
            }
            Ok(())
        })?;
        Ok(found)
    }

    /// Model servers running on this Mac, found by their model lists. This
    /// never leaves the machine, but it is still a request to a model server,
    /// so it waits for the AI switch.
    pub async fn detect_local_servers(&self) -> Result<Vec<LocalServer>, AiSettingsError> {
        if !self.gateway.policy().ai_enabled {
            return Err(Refusal::Off.into());
        }
        Ok(aralo_providers::detect_local_servers(self.transport.as_ref()).await)
    }

    /// The key a saved profile names, from the store.
    pub fn key_for(&self, profile: &Profile) -> Result<Option<Secret>, AiSettingsError> {
        match &profile.key_ref {
            Some(key_ref) => Ok(self.secrets.get(key_ref)?),
            None => Ok(None),
        }
    }

    /// A draft as the gateway takes it, with its key. The draft does not
    /// have to be saved, so Test connection works before Save.
    fn resolve(
        &self,
        draft: &ProfileDraft,
        key: KeyChange,
    ) -> Result<(Profile, Option<Secret>), AiSettingsError> {
        let draft = checked(draft)?;
        let saved_ref = match &draft.original_name {
            Some(original) => read(&self.path)?
                .find(original)
                .and_then(|stored| stored.key_ref.clone()),
            None => None,
        };
        let key = match key {
            KeyChange::Keep => match &saved_ref {
                Some(key_ref) => self.secrets.get(key_ref)?,
                None => None,
            },
            KeyChange::Set(secret) if !secret.expose().trim().is_empty() => {
                Some(Secret::new(secret.expose().trim()))
            }
            KeyChange::Set(_) | KeyChange::Remove => None,
        };
        let profile = Profile {
            name: draft.name,
            adapter: draft.adapter,
            base_url: draft.base_url,
            headers: draft.headers,
            default_model: draft.default_model,
            key_ref: saved_ref,
        };
        Ok((profile, key))
    }

    fn hold(&self) -> MutexGuard<'_, ()> {
        self.lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Reads the file, changes it and writes it back. It is read afresh each
    /// time, so a change the command line made in between is kept.
    fn change(
        &self,
        edit: impl FnOnce(&mut ProfilesFile) -> Result<(), AiSettingsError>,
    ) -> Result<(), AiSettingsError> {
        let _held = self.hold();
        let mut file = read(&self.path)?;
        edit(&mut file)?;
        write(&self.path, &file)?;
        self.apply(&file)
    }

    fn apply(&self, file: &ProfilesFile) -> Result<(), AiSettingsError> {
        if let Some(guard) = &self.guard {
            guard.set_local_only(file.local_only)?;
        }
        self.gateway.set_policy(policy(file));
        Ok(())
    }
}

/// Whether text looks like an API key: a known provider prefix, or a long
/// run of letters and digits with both in it. Used to refuse a key where it
/// would be written down, and by the tests that look for leaks.
pub fn looks_like_key(text: &str) -> bool {
    const PREFIXES: [&str; 8] = ["sk-", "gsk_", "xai-", "inf_", "pplx-", "AIza", "hf_", "r8_"];
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        .any(|word| {
            let long = word.len() >= 24;
            let prefixed = PREFIXES.iter().any(|prefix| word.starts_with(prefix));
            let mixed = word.chars().any(|c| c.is_ascii_digit())
                && word.chars().any(|c| c.is_ascii_alphabetic());
            long && mixed && (prefixed || word.len() >= 32)
        })
}

fn checked(draft: &ProfileDraft) -> Result<ProfileDraft, AiSettingsError> {
    let name = draft.name.trim();
    if name.is_empty() {
        return Err(invalid(ProfileField::Name, "the profile needs a name"));
    }
    if name.chars().count() > MAX_NAME {
        return Err(invalid(
            ProfileField::Name,
            format!("a profile name is at most {MAX_NAME} characters"),
        ));
    }
    if draft.adapter != AdapterKind::OpenAiCompat {
        return Err(invalid(
            ProfileField::Adapter,
            "only OpenAI-compatible endpoints are supported so far",
        ));
    }
    let base_url = draft.base_url.trim().trim_end_matches('/');
    if let Err(refusal) = aralo_ai::guard::parse_endpoint(base_url) {
        return Err(invalid(ProfileField::BaseUrl, refusal.to_string()));
    }
    if looks_like_key(base_url) {
        return Err(invalid(
            ProfileField::BaseUrl,
            "the address looks like it holds a key; put the key in the key field",
        ));
    }
    let model = draft.default_model.trim();
    if model.is_empty() {
        return Err(invalid(ProfileField::Model, "choose a model"));
    }
    let mut headers = Vec::with_capacity(draft.headers.len());
    for (header, value) in &draft.headers {
        let header = header.trim();
        if header.is_empty()
            || !header
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
        {
            return Err(invalid(
                ProfileField::Headers,
                format!("\u{201c}{header}\u{201d} is not a header name"),
            ));
        }
        if CREDENTIAL_HEADERS
            .iter()
            .any(|credential| credential.eq_ignore_ascii_case(header))
            || looks_like_key(value)
        {
            return Err(invalid(
                ProfileField::Headers,
                format!(
                    "\u{201c}{header}\u{201d} would carry a key, and headers are saved as \
                     plain text; put the key in the key field, where it goes to the keychain"
                ),
            ));
        }
        if value.contains(['\r', '\n']) {
            return Err(invalid(
                ProfileField::Headers,
                format!("the value of \u{201c}{header}\u{201d} has a line break"),
            ));
        }
        headers.push((header.to_owned(), value.trim().to_owned()));
    }
    let mut seen = std::collections::HashSet::new();
    if let Some((duplicate, _)) = headers
        .iter()
        .find(|(header, _)| !seen.insert(header.to_ascii_lowercase()))
    {
        return Err(invalid(
            ProfileField::Headers,
            format!("\u{201c}{duplicate}\u{201d} is set twice"),
        ));
    }
    Ok(ProfileDraft {
        original_name: draft.original_name.clone(),
        name: name.to_owned(),
        adapter: draft.adapter,
        base_url: base_url.to_owned(),
        default_model: model.to_owned(),
        headers,
    })
}

fn to_profile(stored: &StoredProfile) -> Result<Profile, AiSettingsError> {
    let adapter = AdapterKind::parse(&stored.adapter).ok_or_else(|| {
        invalid(
            ProfileField::Adapter,
            format!(
                "\u{201c}{}\u{201d} is not an adapter Aralo knows",
                stored.adapter
            ),
        )
    })?;
    Ok(Profile {
        name: stored.name.clone(),
        adapter,
        base_url: stored.base_url.clone(),
        headers: stored
            .headers
            .iter()
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect(),
        default_model: stored.default_model.clone(),
        key_ref: stored.key_ref.clone(),
    })
}

fn saved(stored: &StoredProfile, default: Option<&str>) -> SavedProfile {
    // A profile whose adapter this build does not know is still listed, so it
    // can be deleted; the gateway refuses to use it.
    let profile = to_profile(stored).unwrap_or_else(|_| Profile {
        name: stored.name.clone(),
        adapter: AdapterKind::Anthropic,
        base_url: stored.base_url.clone(),
        headers: Vec::new(),
        default_model: stored.default_model.clone(),
        key_ref: stored.key_ref.clone(),
    });
    SavedProfile {
        is_default: default.is_some_and(|default| same_name(default, &stored.name)),
        capabilities: stored
            .capabilities
            .as_ref()
            .map(|stored| (stored.capabilities(), stored.probed_at.clone())),
        profile,
    }
}

fn policy(file: &ProfilesFile) -> Policy {
    Policy {
        ai_enabled: file.enabled,
        local_only: file.local_only,
        allowed_hosts: None,
    }
}

fn read(path: &Path) -> Result<ProfilesFile, AiSettingsError> {
    ProfilesFile::read(path).map_err(|message| AiSettingsError::File {
        path: path.display().to_string(),
        message,
    })
}

fn write(path: &Path, file: &ProfilesFile) -> Result<(), AiSettingsError> {
    file.write(path).map_err(|message| AiSettingsError::File {
        path: path.display().to_string(),
        message,
    })
}

/// A fresh name for a keychain item. It never changes, so a renamed profile
/// keeps its key.
fn new_key_ref() -> String {
    format!(
        "profile-{}",
        ulid::Ulid::generate().to_string().to_lowercase()
    )
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_recognised_and_ordinary_text_is_not() {
        for key in [
            "sk-proj-4f9aQ2mZ7xKc81LwPq0RtYb3",
            "gsk_8Hn2Kq9ZxW4mP7vR1tY6uB3cD5eF",
            "inf_8b75ba6cca8a481eb831393b77013b87",
            "Bearer 0123456789abcdef0123456789abcdef",
        ] {
            assert!(looks_like_key(key), "{key}");
        }
        for text in [
            "https://openrouter.ai/api/v1",
            "https://aralo.app",
            "Aralo",
            "llama-3.1-8b-instant",
            "anthropic/claude-sonnet-4.5",
            "sk-test",
            "application/json",
        ] {
            assert!(!looks_like_key(text), "{text}");
        }
    }
}

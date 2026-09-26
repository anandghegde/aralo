//! `profiles.toml`: the AI switch, local-only mode and the saved profiles,
//! in the state folder, outside the library so it never syncs (PRD P13).
//!
//! No key is ever written here. A profile names its key by `key_ref`, and the
//! key is in the [`SecretStore`](aralo_ai::SecretStore).

use std::collections::BTreeMap;
use std::path::Path;

use aralo_ai::{Capabilities, Check};
use serde::{Deserialize, Serialize};

const HEADER: &str = "\
# Aralo's AI settings. Keys are not in this file: each profile names its item
# in the system keychain. Edit it in Aralo's settings, or by hand while Aralo
# is not running.

";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProfilesFile {
    /// The master switch. Off until the user turns it on.
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub local_only: bool,
    /// The profile a feature uses when nothing names another.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_profile: Option<String>,
    #[serde(default, rename = "profile", skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<StoredProfile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredProfile {
    pub name: String,
    pub adapter: String,
    pub base_url: String,
    pub default_model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_ref: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    /// The last probe's findings. Cleared when anything they depend on
    /// changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<StoredCapabilities>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct StoredCapabilities {
    /// RFC 3339.
    pub probed_at: String,
    pub models_route: StoredCheck,
    pub streaming: StoredCheck,
    pub system_prompt: StoredCheck,
    pub json_output: StoredCheck,
    pub embeddings: StoredCheck,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "reason", rename_all = "snake_case")]
pub(crate) enum StoredCheck {
    Yes,
    No(String),
    NotChecked(String),
}

impl From<&Check> for StoredCheck {
    fn from(check: &Check) -> Self {
        match check {
            Check::Yes => Self::Yes,
            Check::No(reason) => Self::No(reason.clone()),
            Check::NotChecked(reason) => Self::NotChecked(reason.clone()),
        }
    }
}

impl From<&StoredCheck> for Check {
    fn from(check: &StoredCheck) -> Self {
        match check {
            StoredCheck::Yes => Self::Yes,
            StoredCheck::No(reason) => Self::No(reason.clone()),
            StoredCheck::NotChecked(reason) => Self::NotChecked(reason.clone()),
        }
    }
}

impl StoredCapabilities {
    pub fn new(found: &Capabilities, probed_at: String) -> Self {
        Self {
            probed_at,
            models_route: (&found.models_route).into(),
            streaming: (&found.streaming).into(),
            system_prompt: (&found.system_prompt).into(),
            json_output: (&found.json_output).into(),
            embeddings: (&found.embeddings).into(),
        }
    }

    pub fn capabilities(&self) -> Capabilities {
        Capabilities {
            models_route: (&self.models_route).into(),
            streaming: (&self.streaming).into(),
            system_prompt: (&self.system_prompt).into(),
            json_output: (&self.json_output).into(),
            embeddings: (&self.embeddings).into(),
        }
    }
}

impl ProfilesFile {
    /// Reads the file. One that does not exist yet is the default: AI off,
    /// no profiles.
    pub fn read(path: &Path) -> Result<Self, String> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default())
            }
            Err(error) => return Err(error.to_string()),
        };
        // Only the message and the line: toml's own display quotes the line,
        // and a hand-edited file may have a key pasted into it.
        toml::from_str(&text).map_err(|error: toml::de::Error| {
            let line = error
                .span()
                .map(|span| text[..span.start].matches('\n').count() + 1);
            match line {
                Some(line) => format!("line {line}: {}", error.message()),
                None => error.message().to_owned(),
            }
        })
    }

    pub fn write(&self, path: &Path) -> Result<(), String> {
        let body = toml::to_string(self).map_err(|error| error.to_string())?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        aralo_library::write_atomic(path, format!("{HEADER}{body}").as_bytes())
            .map_err(|error| error.to_string())
    }

    pub fn find(&self, name: &str) -> Option<&StoredProfile> {
        self.profiles
            .iter()
            .find(|profile| same_name(&profile.name, name))
    }

    pub fn find_mut(&mut self, name: &str) -> Option<&mut StoredProfile> {
        self.profiles
            .iter_mut()
            .find(|profile| same_name(&profile.name, name))
    }
}

/// Profile names are compared without regard to case, so "OpenAI" and
/// "openai" cannot both exist.
pub(crate) fn same_name(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_round_trips_and_starts_with_its_header() {
        let file = ProfilesFile {
            enabled: true,
            local_only: false,
            default_profile: Some("Groq".into()),
            profiles: vec![StoredProfile {
                name: "Groq".into(),
                adapter: "openai_compat".into(),
                base_url: "https://api.groq.com/openai/v1".into(),
                default_model: "llama-3.1-8b-instant".into(),
                key_ref: Some("01J00000000000000000000000".into()),
                headers: BTreeMap::from([("X-Title".into(), "Aralo".into())]),
                capabilities: Some(StoredCapabilities {
                    probed_at: "2026-09-24T10:00:00Z".into(),
                    models_route: StoredCheck::Yes,
                    streaming: StoredCheck::Yes,
                    system_prompt: StoredCheck::Yes,
                    json_output: StoredCheck::No("the endpoint answered 400".into()),
                    embeddings: StoredCheck::NotChecked("not used".into()),
                }),
            }],
        };
        let folder = aralo_testkit::tempdir().unwrap();
        let path = folder.path().join("nested").join("profiles.toml");
        file.write(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# Aralo's AI settings."), "{text}");
        assert_eq!(ProfilesFile::read(&path).unwrap(), file);
    }

    #[test]
    fn a_missing_file_is_ai_off_with_no_profiles() {
        let folder = aralo_testkit::tempdir().unwrap();
        assert_eq!(
            ProfilesFile::read(&folder.path().join("profiles.toml")).unwrap(),
            ProfilesFile::default()
        );
    }

    #[test]
    fn a_key_field_is_refused_rather_than_ignored() {
        let text = "[[profile]]\nname = \"x\"\nadapter = \"openai_compat\"\n\
                    base_url = \"https://x.test/v1\"\ndefault_model = \"m\"\napi_key = \"sk-1\"\n";
        let folder = aralo_testkit::tempdir().unwrap();
        let path = folder.path().join("profiles.toml");
        std::fs::write(&path, text).unwrap();
        let error = ProfilesFile::read(&path).unwrap_err();
        assert!(error.contains("api_key"), "{error}");
        assert!(!error.contains("sk-1"), "the error quotes the key: {error}");
    }
}

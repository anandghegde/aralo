//! Providers the settings pane offers by name, for guided key setup: picking
//! one fills in the endpoint and a model, and links to the page where the
//! user makes a key. Any other OpenAI-compatible endpoint is entered by hand.

use aralo_ai::AdapterKind;

use super::ProfileDraft;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderPreset {
    /// Short and stable, for `aralo ai add --preset`.
    pub id: &'static str,
    pub name: &'static str,
    pub base_url: &'static str,
    /// A small, cheap model the provider serves, to start with. The model
    /// list in settings offers the rest.
    pub default_model: &'static str,
    /// Where the user makes a key. `None` for a server that needs none.
    pub key_page: Option<&'static str>,
}

impl ProviderPreset {
    /// A new profile for this provider, named after it.
    pub fn draft(&self) -> ProfileDraft {
        ProfileDraft {
            original_name: None,
            name: self.name.into(),
            adapter: AdapterKind::OpenAiCompat,
            base_url: self.base_url.into(),
            default_model: self.default_model.into(),
            headers: Vec::new(),
        }
    }
}

/// Remote providers first, in no order of preference, then the local
/// servers, which need no key. Detect finds local servers on their own.
pub const PROVIDER_PRESETS: &[ProviderPreset] = &[
    ProviderPreset {
        id: "openai",
        name: "OpenAI",
        base_url: "https://api.openai.com/v1",
        default_model: "gpt-4o-mini",
        key_page: Some("https://platform.openai.com/api-keys"),
    },
    ProviderPreset {
        id: "anthropic",
        name: "Anthropic",
        base_url: "https://api.anthropic.com/v1",
        default_model: "claude-haiku-4-5",
        key_page: Some("https://console.anthropic.com/settings/keys"),
    },
    ProviderPreset {
        id: "gemini",
        name: "Google Gemini",
        base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
        default_model: "gemini-2.5-flash",
        key_page: Some("https://aistudio.google.com/apikey"),
    },
    ProviderPreset {
        id: "openrouter",
        name: "OpenRouter",
        base_url: "https://openrouter.ai/api/v1",
        default_model: "openai/gpt-4o-mini",
        key_page: Some("https://openrouter.ai/settings/keys"),
    },
    ProviderPreset {
        id: "groq",
        name: "Groq",
        base_url: "https://api.groq.com/openai/v1",
        default_model: "llama-3.1-8b-instant",
        key_page: Some("https://console.groq.com/keys"),
    },
    ProviderPreset {
        id: "mistral",
        name: "Mistral",
        base_url: "https://api.mistral.ai/v1",
        default_model: "mistral-small-latest",
        key_page: Some("https://console.mistral.ai/api-keys"),
    },
    ProviderPreset {
        id: "deepseek",
        name: "DeepSeek",
        base_url: "https://api.deepseek.com/v1",
        default_model: "deepseek-chat",
        key_page: Some("https://platform.deepseek.com/api_keys"),
    },
    ProviderPreset {
        id: "together",
        name: "Together AI",
        base_url: "https://api.together.xyz/v1",
        default_model: "meta-llama/Llama-3.3-70B-Instruct-Turbo",
        key_page: Some("https://api.together.ai/settings/api-keys"),
    },
    ProviderPreset {
        id: "ollama",
        name: "Ollama",
        base_url: "http://127.0.0.1:11434/v1",
        default_model: "llama3.2",
        key_page: None,
    },
    ProviderPreset {
        id: "lmstudio",
        name: "LM Studio",
        base_url: "http://127.0.0.1:1234/v1",
        default_model: "",
        key_page: None,
    },
];

/// The preset with this id, ignoring case.
pub fn provider_preset(id: &str) -> Option<&'static ProviderPreset> {
    PROVIDER_PRESETS
        .iter()
        .find(|preset| preset.id.eq_ignore_ascii_case(id.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_has_an_endpoint_that_parses_and_a_unique_id() {
        let mut ids = std::collections::HashSet::new();
        for preset in PROVIDER_PRESETS {
            assert!(ids.insert(preset.id), "{} twice", preset.id);
            let endpoint = aralo_ai::guard::parse_endpoint(preset.base_url)
                .unwrap_or_else(|error| panic!("{}: {error}", preset.id));
            // A key goes only to a remote endpoint over TLS; a local server
            // needs none.
            assert_eq!(
                preset.key_page.is_some(),
                !endpoint.host.is_loopback(),
                "{}",
                preset.id
            );
            if let Some(page) = preset.key_page {
                assert!(page.starts_with("https://"), "{page}");
                assert!(preset.base_url.starts_with("https://"), "{}", preset.id);
            }
        }
        assert_eq!(provider_preset("OpenRouter").unwrap().name, "OpenRouter");
        assert!(provider_preset("nope").is_none());
    }
}

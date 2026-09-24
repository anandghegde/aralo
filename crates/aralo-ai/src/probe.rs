//! Test connection and the capability probe, for the settings pane (plan
//! 7.2). Both go through the same stages as any feature's request: the policy
//! is checked, the network guard sends, and the tokens are metered under
//! [`Feature::Setup`].

use std::time::{Duration, Instant};

use crate::error::AiError;
use crate::gateway::{AiRequest, Gateway};
use crate::model::{Delta, Feature, Profile, Secret};

/// A reply longer than this is cut in a report. It is shown to the user as
/// proof the model answered, not read.
const MAX_REPLY: usize = 200;
/// Enough for a short answer from a model that reasons before it answers.
const PROBE_MAX_TOKENS: u32 = 512;

/// What Test connection found: the key and the model work, and how quickly
/// the first word came back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionReport {
    pub model: String,
    pub first_token: Duration,
    /// The start of what the model said.
    pub reply: String,
}

/// One capability, as the probe found it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    Yes,
    /// Missing, and why. A feature that needs it is disabled with this
    /// reason shown.
    No(String),
    /// Not tried, and why.
    NotChecked(String),
}

impl Check {
    pub fn is_yes(&self) -> bool {
        matches!(self, Self::Yes)
    }
}

/// What a profile's endpoint can do. Saving a profile probes it, and the
/// result is kept with the profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capabilities {
    /// `GET /models` lists the models to choose from.
    pub models_route: Check,
    /// Answers arrive in pieces as they are written.
    pub streaming: Check,
    /// The model follows the system prompt.
    pub system_prompt: Check,
    /// The endpoint can be asked for a JSON object.
    pub json_output: Check,
    /// The endpoint serves embeddings.
    pub embeddings: Check,
}

impl Gateway {
    /// Sends one short chat with the profile's model and reports on it. This
    /// is the check a key and a model name work together; a models list alone
    /// is not, because some endpoints list models without a key.
    pub async fn test_connection(
        &self,
        profile: &Profile,
        key: Option<Secret>,
    ) -> Result<ConnectionReport, AiError> {
        let started = Instant::now();
        let prepared = self.prepare(
            setup_request(profile, "", "Reply with the single word: ok", false),
            &mut (),
        )?;
        let model = prepared.chat().model.clone();
        let mut stream = self.send(prepared, key).await?;
        let mut first_token = None;
        let mut reply = String::new();
        while let Some(delta) = stream.next().await {
            if let Delta::Text(text) = delta? {
                first_token.get_or_insert_with(|| started.elapsed());
                reply.push_str(&text);
            }
        }
        Ok(ConnectionReport {
            model,
            first_token: first_token.unwrap_or_else(|| started.elapsed()),
            reply: clip(reply.trim()),
        })
    }

    /// Tries each capability with the cheapest request that shows it: the
    /// models route, one chat for streaming and the system prompt, and one
    /// for JSON output. A refusal by the policy is an error, since nothing
    /// was tried; anything the endpoint says is a finding.
    pub async fn probe(
        &self,
        profile: &Profile,
        key: Option<Secret>,
    ) -> Result<Capabilities, AiError> {
        let models_route = match self
            .list_models(profile, key.as_ref().map(Secret::duplicate))
            .await
        {
            Ok(models) if models.is_empty() => {
                Check::No("the models route answered with no models".into())
            }
            Ok(_) => Check::Yes,
            Err(AiError::Refused(refusal)) => return Err(refusal.into()),
            Err(error) => Check::No(error.to_string()),
        };

        // The system prompt asks for something the user message does not, so
        // the answer shows whether it was read. The answer is long enough to
        // arrive in several pieces from any endpoint that streams.
        let (streaming, system_prompt) = match self
            .ask(
                profile,
                key.as_ref(),
                "Whatever the user writes, reply with the numbers from one to ten \
                 in words, separated by spaces, followed by the word PINEAPPLE.",
                "Hello.",
                false,
            )
            .await
        {
            Ok((pieces, text)) => {
                let streaming = if pieces > 1 {
                    Check::Yes
                } else {
                    Check::No("the answer came back whole, not as a stream".into())
                };
                let lower = text.to_lowercase();
                let system_prompt = if lower.contains("pineapple") && lower.contains("seven") {
                    Check::Yes
                } else {
                    Check::No("the model did not follow the system prompt".into())
                };
                (streaming, system_prompt)
            }
            Err(AiError::Refused(refusal)) => return Err(refusal.into()),
            Err(error) => (
                Check::No(error.to_string()),
                Check::NotChecked("the chat request failed".into()),
            ),
        };

        let json_output = match self
            .ask(
                profile,
                key.as_ref(),
                "Reply with a JSON object and nothing else.",
                "Return an object with one key, \"ok\", set to true.",
                true,
            )
            .await
        {
            Ok((_, text)) => {
                let object = serde_json::from_str::<serde_json::Value>(text.trim())
                    .ok()
                    .filter(serde_json::Value::is_object);
                if object.is_some() {
                    Check::Yes
                } else {
                    Check::No("the answer in JSON mode was not a JSON object".into())
                }
            }
            Err(AiError::Refused(refusal)) => return Err(refusal.into()),
            Err(error) => Check::No(error.to_string()),
        };

        Ok(Capabilities {
            models_route,
            streaming,
            system_prompt,
            json_output,
            embeddings: Check::NotChecked(
                "no feature uses a remote embedding model: semantic search runs on the \
                 bundled one"
                    .into(),
            ),
        })
    }

    /// One chat, read to the end: how many pieces of text it came in, and
    /// the text.
    async fn ask(
        &self,
        profile: &Profile,
        key: Option<&Secret>,
        system: &str,
        instruction: &str,
        json_output: bool,
    ) -> Result<(usize, String), AiError> {
        let prepared = self.prepare(
            setup_request(profile, system, instruction, json_output),
            &mut (),
        )?;
        let mut stream = self.send(prepared, key.map(Secret::duplicate)).await?;
        let mut pieces = 0;
        let mut text = String::new();
        while let Some(delta) = stream.next().await {
            if let Delta::Text(piece) = delta? {
                pieces += 1;
                text.push_str(&piece);
            }
        }
        Ok((pieces, text))
    }
}

fn setup_request(profile: &Profile, system: &str, instruction: &str, json: bool) -> AiRequest {
    AiRequest {
        feature: Feature::Setup,
        profile: profile.clone(),
        model: None,
        system: system.into(),
        instruction: instruction.into(),
        declared: Vec::new(),
        max_tokens: Some(PROBE_MAX_TOKENS),
        // Left to the endpoint: some reasoning models refuse any other value.
        temperature: None,
        json_output: json,
    }
}

fn clip(text: &str) -> String {
    match text.char_indices().nth(MAX_REPLY) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_owned(),
    }
}

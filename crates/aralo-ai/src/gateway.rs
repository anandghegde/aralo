//! The gateway: the stages in order, and the stream that comes out.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use crate::context::{self, ContextKind, ContextSource, ManifestEntry};
use crate::error::{AiError, Refusal};
use crate::framing;
use crate::metering::{CapWarning, Meter};
use crate::model::{AdapterKind, ChatRequest, Delta, Feature, Message, Profile, Role, Secret};
use crate::policy::Policy;
use crate::redact;
use crate::transport::{BoxFuture, Transport};

/// Translates the neutral request to one provider's wire format, and its
/// stream back. `aralo-providers` implements it; an adapter is handed a
/// [`Transport`] for each call and cannot reach the network any other way.
pub trait Adapter: Send + Sync {
    fn kind(&self) -> AdapterKind;

    /// Sends the request. It returns once the endpoint has accepted it, with
    /// the stream of what the model writes; a status the endpoint refused it
    /// with is an [`AiError::Status`], so the gateway can retry a 429.
    fn chat_stream<'a>(
        &'a self,
        http: &'a dyn Transport,
        call: Call<'a>,
    ) -> BoxFuture<'a, Result<Box<dyn DeltaStream>, AiError>>;

    /// The model names the endpoint offers, for the settings pane to choose
    /// from. A user may still type a name that is not listed.
    fn list_models<'a>(
        &'a self,
        http: &'a dyn Transport,
        profile: &'a Profile,
        key: Option<&'a Secret>,
    ) -> BoxFuture<'a, Result<Vec<String>, AiError>>;
}

/// What an adapter is given for one call.
#[derive(Debug, Clone, Copy)]
pub struct Call<'a> {
    pub profile: &'a Profile,
    pub key: Option<&'a Secret>,
    pub request: &'a ChatRequest,
}

pub trait DeltaStream: Send {
    /// The next piece, or `None` after [`Delta::Done`] or at the end of the
    /// body. Dropping the stream closes the connection.
    fn next(&mut self) -> BoxFuture<'_, Option<Result<Delta, AiError>>>;
}

/// Retries happen only before the first token: at most `attempts` more, on
/// 429 and 5xx answers, waiting `backoff`, then twice that (ADR-0007).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    pub attempts: u32,
    pub backoff: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            attempts: 2,
            backoff: Duration::from_millis(500),
        }
    }
}

/// What a feature asks for. It names the context kinds it declared; it never
/// carries the context itself.
#[derive(Debug, Clone)]
pub struct AiRequest {
    pub feature: Feature,
    pub profile: Profile,
    /// Overrides the profile's default model, as a snippet's `ai.model` does.
    pub model: Option<String>,
    /// The feature's system prompt.
    pub system: String,
    /// What the user or the snippet asks for. Trusted: it is the user's own.
    pub instruction: String,
    pub declared: Vec<ContextKind>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    /// Ask for a JSON object as the answer.
    pub json_output: bool,
}

/// A request that has passed every stage before the network. The preview
/// panel shows its manifest before anything is sent.
#[derive(Debug, Clone)]
pub struct Prepared {
    feature: Feature,
    profile: Profile,
    chat: ChatRequest,
    manifest: Vec<ManifestEntry>,
}

impl Prepared {
    pub fn manifest(&self) -> &[ManifestEntry] {
        &self.manifest
    }

    /// The request as the adapter will receive it.
    pub fn chat(&self) -> &ChatRequest {
        &self.chat
    }

    pub fn profile(&self) -> &Profile {
        &self.profile
    }
}

pub struct Gateway {
    policy: RwLock<Policy>,
    transport: Arc<dyn Transport>,
    adapters: Vec<Arc<dyn Adapter>>,
    meter: Arc<Meter>,
    retry: RetryPolicy,
}

impl std::fmt::Debug for Gateway {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gateway")
            .field("policy", &self.policy())
            .field(
                "adapters",
                &self
                    .adapters
                    .iter()
                    .map(|adapter| adapter.kind())
                    .collect::<Vec<_>>(),
            )
            .field("retry", &self.retry)
            .finish_non_exhaustive()
    }
}

impl Gateway {
    /// A gateway that sends through `transport`, which outside tests is the
    /// [`NetworkGuard`](crate::guard::NetworkGuard).
    pub fn new(policy: Policy, transport: Arc<dyn Transport>) -> Self {
        Self {
            policy: RwLock::new(policy),
            transport,
            adapters: Vec::new(),
            meter: Arc::new(Meter::new()),
            retry: RetryPolicy::default(),
        }
    }

    pub fn register(&mut self, adapter: Arc<dyn Adapter>) {
        self.adapters
            .retain(|existing| existing.kind() != adapter.kind());
        self.adapters.push(adapter);
    }

    pub fn with_retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    pub fn policy(&self) -> Policy {
        self.policy
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Replaces the policy. It applies to the next [`Gateway::prepare`] and
    /// the next [`Gateway::send`]; a stream already running is the caller's
    /// to cancel. Local-only mode is also the guard's to enforce, and the
    /// shell switches both.
    pub fn set_policy(&self, policy: Policy) {
        *self
            .policy
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = policy;
    }

    pub fn meter(&self) -> &Meter {
        &self.meter
    }

    /// Runs the stages before the network: the policy check, context
    /// assembly, redaction and framing. The source is asked for the declared
    /// kinds only, and not at all if the policy refuses.
    pub fn prepare(
        &self,
        request: AiRequest,
        source: &mut dyn ContextSource,
    ) -> Result<Prepared, Refusal> {
        self.policy().check(&request.profile)?;
        self.adapter(request.profile.adapter)?;
        let model = request
            .model
            .filter(|model| !model.trim().is_empty())
            .unwrap_or_else(|| request.profile.default_model.clone());
        if model.trim().is_empty() {
            return Err(Refusal::NoModel);
        }

        let (items, manifest) = context::assemble(&request.declared, source);
        let items = redact::redact(items);
        let framed = framing::frame(&request.system, &request.instruction, &items);

        Ok(Prepared {
            feature: request.feature,
            chat: ChatRequest {
                model,
                system: framed.system,
                messages: vec![Message {
                    role: Role::User,
                    content: framed.user,
                }],
                max_tokens: request.max_tokens,
                temperature: request.temperature,
                json_output: request.json_output,
            },
            profile: request.profile,
            manifest,
        })
    }

    /// Sends a prepared request and returns its stream. The policy is checked
    /// again first. A 429 or 5xx is retried before the first token; nothing is
    /// retried once the stream has begun.
    pub async fn send(&self, prepared: Prepared, key: Option<Secret>) -> Result<AiStream, AiError> {
        self.policy().check(&prepared.profile)?;
        let adapter = self.adapter(prepared.profile.adapter)?;
        let call = Call {
            profile: &prepared.profile,
            key: key.as_ref(),
            request: &prepared.chat,
        };

        let mut attempt = 0;
        let inner = loop {
            match adapter.chat_stream(self.transport.as_ref(), call).await {
                Ok(stream) => break stream,
                Err(error) if error.is_retryable() && attempt < self.retry.attempts => {
                    let wait = match &error {
                        AiError::Status {
                            retry_after: Some(after),
                            ..
                        } => (*after).min(Duration::from_secs(10)),
                        _ => self.retry.backoff * 2u32.pow(attempt),
                    };
                    attempt += 1;
                    tokio::time::sleep(wait).await;
                }
                Err(error) => return Err(error),
            }
        };

        Ok(AiStream {
            inner,
            meter: self.meter.clone(),
            feature: prepared.feature,
            profile: prepared.profile.name,
            warning: None,
            finished: false,
        })
    }

    /// Asks a profile's endpoint which models it offers. The policy is
    /// checked first, as for a chat: with AI off, or a remote profile in
    /// local-only mode, nothing is sent.
    pub async fn list_models(
        &self,
        profile: &Profile,
        key: Option<Secret>,
    ) -> Result<Vec<String>, AiError> {
        self.policy().check(profile)?;
        let adapter = self.adapter(profile.adapter)?;
        adapter
            .list_models(self.transport.as_ref(), profile, key.as_ref())
            .await
    }

    fn adapter(&self, kind: AdapterKind) -> Result<Arc<dyn Adapter>, Refusal> {
        self.adapters
            .iter()
            .find(|adapter| adapter.kind() == kind)
            .cloned()
            .ok_or(Refusal::NoAdapter(kind.as_str()))
    }
}

/// A running answer. Usage is metered as it passes. Dropping the stream
/// cancels it and closes the connection at once.
pub struct AiStream {
    inner: Box<dyn DeltaStream>,
    meter: Arc<Meter>,
    feature: Feature,
    profile: String,
    warning: Option<CapWarning>,
    finished: bool,
}

impl std::fmt::Debug for AiStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AiStream")
            .field("feature", &self.feature)
            .field("profile", &self.profile)
            .field("finished", &self.finished)
            .finish_non_exhaustive()
    }
}

impl AiStream {
    /// The next piece. After [`Delta::Done`] or an error there is nothing
    /// more.
    pub async fn next(&mut self) -> Option<Result<Delta, AiError>> {
        if self.finished {
            return None;
        }
        let delta = self.inner.next().await;
        match &delta {
            Some(Ok(Delta::Usage(usage))) => {
                if let Some(warning) = self.meter.record(self.feature, &self.profile, *usage) {
                    self.warning = Some(warning);
                }
            }
            Some(Ok(Delta::Done(_))) | Some(Err(_)) | None => self.finished = true,
            Some(Ok(Delta::Text(_))) => {}
        }
        delta
    }

    /// Reads the whole answer as text. For callers that do not show a stream,
    /// such as the conformance suite.
    pub async fn collect_text(mut self) -> Result<String, AiError> {
        let mut text = String::new();
        while let Some(delta) = self.next().await {
            if let Delta::Text(piece) = delta? {
                text.push_str(&piece);
            }
        }
        Ok(text)
    }

    /// Set when this answer took its profile past its soft cap. The stream
    /// carries on regardless.
    pub fn cap_warning(&self) -> Option<&CapWarning> {
        self.warning.as_ref()
    }
}

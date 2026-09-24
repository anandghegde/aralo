//! The neutral request and stream. Adapters translate to and from these; no
//! other part of Aralo sees a provider's wire format.

use std::fmt;
use std::ops::AddAssign;

/// Which adapter speaks to a profile's endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AdapterKind {
    /// Chat completions with server-sent events: OpenAI and everything that
    /// copies it, local servers included.
    OpenAiCompat,
    /// Anthropic's Messages API.
    Anthropic,
}

impl AdapterKind {
    pub const ALL: [Self; 2] = [Self::OpenAiCompat, Self::Anthropic];

    /// The kind [`AdapterKind::as_str`] names.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == name)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenAiCompat => "openai_compat",
            Self::Anthropic => "anthropic",
        }
    }
}

/// A saved endpoint. The key is not in it: `key_ref` names an entry in the
/// shell's secret store, and the key itself only ever arrives as a [`Secret`]
/// for one request (PRD P13).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub name: String,
    pub adapter: AdapterKind,
    /// Up to and including the API version, such as
    /// `https://api.openai.com/v1` or `http://127.0.0.1:11434/v1`.
    pub base_url: String,
    /// Sent with every request, after the adapter's own headers.
    pub headers: Vec<(String, String)>,
    pub default_model: String,
    pub key_ref: Option<String>,
}

/// The feature asking. Metering is kept per feature and per profile, and
/// settings map each feature to a profile and model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Feature {
    /// A3: a command run on selected text.
    Command,
    /// A2: an `{{ai}}` block in a snippet.
    Block,
    /// A1: an action in the snippet editor.
    Authoring,
    /// `aralo conformance`.
    Conformance,
    /// Test connection and the capability probe in settings.
    Setup,
}

impl Feature {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::Block => "block",
            Self::Authoring => "authoring",
            Self::Conformance => "conformance",
            Self::Setup => "setup",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

/// What an adapter is given, framed already. The system prompt is separate
/// because the providers disagree on where it goes.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatRequest {
    pub model: String,
    pub system: String,
    pub messages: Vec<Message>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    /// Ask for a JSON object as the answer, where the endpoint supports it.
    pub json_output: bool,
}

/// One piece of a stream, in the order the model produced it.
#[derive(Debug, Clone, PartialEq)]
pub enum Delta {
    /// Model output. It is literal text: nothing downstream parses it for
    /// placeholders, scripts or requests (PRD P10).
    Text(String),
    /// Token counts, as the provider reported them. Some send one at the end,
    /// some send several; the meter adds them up.
    Usage(Usage),
    /// The model finished. Nothing follows it.
    Done(StopReason),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    /// The model ended its answer.
    End,
    /// The answer was cut at `max_tokens`.
    Length,
    /// Anything else, as the provider named it.
    Other(String),
}

/// Token counts. `cached_input_tokens` is the part of `input_tokens` the
/// provider served from its prompt cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: u64,
}

impl Usage {
    pub fn total(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }
}

impl AddAssign for Usage {
    fn add_assign(&mut self, other: Self) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cached_input_tokens += other.cached_input_tokens;
    }
}

/// An API key, held for one request. `Debug` never shows it, and the memory
/// is zeroed when it is dropped.
pub struct Secret(String);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The key itself, for the adapter that puts it in a header.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// A second copy, for a caller that makes several requests with one key.
    /// It is zeroed on drop like the first.
    pub fn duplicate(&self) -> Self {
        Self(self.0.clone())
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(..)")
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.0);
    }
}

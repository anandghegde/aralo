use std::time::Duration;

/// Why the gateway would not send a request. A refusal happens before any
/// network traffic, and a feature that has fallback text uses it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    #[error("AI is switched off")]
    Off,
    #[error("local-only mode is on, and {host} is not this Mac")]
    LocalOnly { host: String },
    #[error("a managed policy does not allow {host}")]
    Managed { host: String },
    #[error("the profile's address cannot be used: {0}")]
    BadUrl(String),
    #[error("the profile names no model, and the request did not either")]
    NoModel,
    #[error("no adapter is registered for {0}")]
    NoAdapter(&'static str),
}

#[derive(Debug, thiserror::Error)]
pub enum AiError {
    #[error(transparent)]
    Refused(#[from] Refusal),
    /// The request never got an answer: no route, a reset connection, a
    /// timeout. The message never carries the URL, which may hold a key.
    #[error("network: {0}")]
    Network(String),
    /// The endpoint answered with an error status.
    #[error("the endpoint answered {status}: {message}")]
    Status {
        status: u16,
        message: String,
        retry_after: Option<Duration>,
    },
    /// The endpoint answered with something the adapter could not read.
    #[error("the endpoint's answer could not be read: {0}")]
    Protocol(String),
}

impl AiError {
    /// Whether the gateway may try again: rate limits and server errors only,
    /// and only before the first token (ADR-0007).
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::Status { status, .. } if *status == 429 || (500..=599).contains(status))
    }
}

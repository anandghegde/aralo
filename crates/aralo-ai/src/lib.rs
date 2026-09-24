//! The AI gateway: the only code in Aralo that can reach a model (ADR-0007).
//!
//! Every AI feature hands the [`Gateway`] a neutral [`AiRequest`] and gets
//! plain text back. Features never see a provider, a key or a URL. A request
//! passes through fixed stages, each with one place in this crate:
//!
//! | Stage | Where |
//! | --- | --- |
//! | Policy check | [`Policy::check`] |
//! | Context assembly | [`context::assemble`]: only the declared kinds are asked for |
//! | Redaction | [`redact`], an empty slot until v1 |
//! | Prompt framing | [`framing::frame`] |
//! | Network guard | [`guard::NetworkGuard`], the only constructor of HTTP clients |
//! | Adapter | an [`Adapter`], registered by `aralo-providers` |
//! | Stream and metering | [`AiStream`] and [`Meter`] |
//! | Output | [`Delta::Text`]: literal text, never parsed again |
//!
//! [`Gateway::prepare`] runs everything before the network and returns the
//! context manifest the preview panel shows. [`Gateway::send`] checks the
//! policy again, because it may have changed while the panel was open, and
//! then streams.

pub mod context;
mod error;
pub mod framing;
mod gateway;
pub mod guard;
mod metering;
mod model;
mod policy;
mod probe;
pub mod redact;
mod secrets;
mod transport;

pub use context::{ContextItem, ContextKind, ContextSource, ManifestEntry};
pub use error::{AiError, Refusal};
pub use gateway::{
    Adapter, AiRequest, AiStream, Call, DeltaStream, Gateway, Prepared, RetryPolicy,
};
pub use metering::{CapWarning, Meter};
pub use model::{
    AdapterKind, ChatRequest, Delta, Feature, Message, Profile, Role, Secret, StopReason, Usage,
};
pub use policy::{Endpoint, Host, Policy};
pub use probe::{Capabilities, Check, ConnectionReport};
pub use secrets::{MemorySecretStore, SecretError, SecretStore};
pub use transport::{BoxFuture, ByteStream, HttpRequest, HttpResponse, Method, Transport};

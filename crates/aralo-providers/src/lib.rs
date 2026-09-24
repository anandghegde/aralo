//! Provider adapters behind the AI gateway in `aralo-ai` (ADR-0007).
//!
//! An adapter translates the gateway's neutral request into one provider's
//! wire format, and the answer back into neutral deltas. It is handed a
//! [`Transport`](aralo_ai::Transport) for each call and cannot reach the
//! network any other way; this crate does not depend on an HTTP client.
//!
//! | Adapter | Speaks to |
//! | --- | --- |
//! | [`OpenAiCompat`] | OpenAI's chat completions and every API that copies it, local servers included |
//!
//! [`detect_local_servers`] finds the local servers on their default ports.

mod http;
pub mod local;
pub mod openai_compat;
pub mod sse;

use std::sync::Arc;

use aralo_ai::Gateway;

pub use local::{detect_local_servers, LocalServer, LocalServerKind};
pub use openai_compat::OpenAiCompat;

/// Registers every adapter this crate has with a gateway.
pub fn register_all(gateway: &mut Gateway) {
    gateway.register(Arc::new(OpenAiCompat::new()));
}

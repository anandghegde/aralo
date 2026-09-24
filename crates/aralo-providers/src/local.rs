//! Finding the model servers running on this Mac.
//!
//! Each known server is probed on its default port with `GET /v1/models` at
//! 127.0.0.1, all at once, each with a short timeout. A port counts only if
//! it answers with a model list, so another program that happens to hold
//! port 8080 is not mistaken for a model server. The probes go through the
//! transport the caller hands in, which outside tests is the network guard,
//! and never leave the machine.

use std::future::Future;
use std::pin::Pin;
use std::task::Poll;
use std::time::Duration;

use aralo_ai::{AdapterKind, Profile, Transport};

use crate::openai_compat;

/// How long one port may take to answer. A server on this machine answers in
/// milliseconds, and a closed port refuses at once.
pub const PROBE_TIMEOUT: Duration = Duration::from_millis(1500);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LocalServerKind {
    Ollama,
    LmStudio,
    LlamaCpp,
    Vllm,
}

impl LocalServerKind {
    pub const ALL: [Self; 4] = [Self::Ollama, Self::LmStudio, Self::LlamaCpp, Self::Vllm];

    pub fn default_port(self) -> u16 {
        match self {
            Self::Ollama => 11434,
            Self::LmStudio => 1234,
            Self::LlamaCpp => 8080,
            Self::Vllm => 8000,
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Ollama => "Ollama",
            Self::LmStudio => "LM Studio",
            Self::LlamaCpp => "llama.cpp",
            Self::Vllm => "vLLM",
        }
    }
}

/// A server that answered, with the models it offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalServer {
    pub kind: LocalServerKind,
    /// Such as `http://127.0.0.1:11434/v1`.
    pub base_url: String,
    pub models: Vec<String>,
}

impl LocalServer {
    /// A profile for this server, ready to save. It needs no key.
    pub fn profile(&self) -> Profile {
        Profile {
            name: self.kind.display_name().to_owned(),
            adapter: AdapterKind::OpenAiCompat,
            base_url: self.base_url.clone(),
            headers: Vec::new(),
            default_model: self.models.first().cloned().unwrap_or_default(),
            key_ref: None,
        }
    }
}

/// Probes every known server on its default port.
pub async fn detect_local_servers(http: &dyn Transport) -> Vec<LocalServer> {
    let ports: Vec<_> = LocalServerKind::ALL
        .iter()
        .map(|kind| (*kind, kind.default_port()))
        .collect();
    detect_on(http, &ports, PROBE_TIMEOUT).await
}

/// Probes the given ports on 127.0.0.1, all at once, and returns the ones
/// that answered with a model list, in the order given.
pub async fn detect_on(
    http: &dyn Transport,
    ports: &[(LocalServerKind, u16)],
    timeout: Duration,
) -> Vec<LocalServer> {
    let probes = ports
        .iter()
        .map(|&(kind, port)| Box::pin(probe(http, kind, port, timeout)) as Probe<'_>)
        .collect();
    join_all(probes).await.into_iter().flatten().collect()
}

type Probe<'a> = Pin<Box<dyn Future<Output = Option<LocalServer>> + Send + 'a>>;

async fn probe(
    http: &dyn Transport,
    kind: LocalServerKind,
    port: u16,
    timeout: Duration,
) -> Option<LocalServer> {
    let base_url = format!("http://127.0.0.1:{port}/v1");
    let profile = Profile {
        name: kind.display_name().to_owned(),
        adapter: AdapterKind::OpenAiCompat,
        base_url: base_url.clone(),
        headers: Vec::new(),
        default_model: String::new(),
        key_ref: None,
    };
    let models = tokio::time::timeout(timeout, openai_compat::list_models(http, &profile, None))
        .await
        .ok()?
        .ok()?;
    Some(LocalServer {
        kind,
        base_url,
        models,
    })
}

/// Runs the probes side by side and waits for every one.
async fn join_all<T>(mut futures: Vec<Pin<Box<dyn Future<Output = T> + Send + '_>>>) -> Vec<T> {
    let mut results: Vec<Option<T>> = futures.iter().map(|_| None).collect();
    std::future::poll_fn(|cx| {
        let mut waiting = false;
        for (future, result) in futures.iter_mut().zip(results.iter_mut()) {
            if result.is_none() {
                match future.as_mut().poll(cx) {
                    Poll::Ready(value) => *result = Some(value),
                    Poll::Pending => waiting = true,
                }
            }
        }
        if waiting {
            Poll::Pending
        } else {
            Poll::Ready(())
        }
    })
    .await;
    results.into_iter().flatten().collect()
}

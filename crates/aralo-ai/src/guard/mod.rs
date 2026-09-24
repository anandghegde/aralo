//! The network guard: the only code in the workspace that constructs an HTTP
//! client (ADR-0007). Code owners review every change here.
//!
//! In local-only mode the guard refuses anything that is not this Mac, at two
//! points, because a host can pass one and not the other:
//!
//! 1. **Before sending**, by the URL. An IP address that is not loopback is
//!    refused, and so is a name that is not `localhost`. `reqwest` does not
//!    resolve an IP literal, so this is the only check such a URL meets.
//! 2. **When connecting**, by address. The client's resolver drops every
//!    address that is not loopback, so `localhost` pointed elsewhere by a
//!    hosts file, or a name that resolves to both, cannot leave the machine.
//!
//! The client has no proxy, so a system proxy cannot carry a local request
//! off the machine, and follows no redirect, so an endpoint cannot send the
//! request, key and all, to another host. Turning local-only mode on builds a
//! new client, so no pooled connection to a remote host outlives the switch.
//!
//! `clippy.toml` at the workspace root bans `reqwest::Client::new` and
//! `reqwest::Client::builder`, and `scripts/check-deps.sh` fails if `reqwest`
//! is named outside this folder. The one allowance is [`build_client`].

mod resolve;

use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub use resolve::{HostLookup, SystemLookup};

use crate::error::{AiError, Refusal};
use crate::policy::{Endpoint, Host};
use crate::transport::{BoxFuture, ByteStream, HttpRequest, HttpResponse, Method, Transport};

/// How long a connection may take to open.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a stream may go silent. Local models can be slow to start, so
/// this is generous; a user who tires of waiting cancels.
pub const READ_TIMEOUT: Duration = Duration::from_secs(120);

pub struct NetworkGuard {
    local_only: Arc<AtomicBool>,
    lookup: Arc<dyn HostLookup>,
    client: Mutex<reqwest::Client>,
}

impl std::fmt::Debug for NetworkGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NetworkGuard")
            .field("local_only", &self.local_only())
            .finish_non_exhaustive()
    }
}

impl NetworkGuard {
    pub fn new(local_only: bool) -> Result<Self, AiError> {
        Self::with_lookup(local_only, Arc::new(SystemLookup))
    }

    /// A guard that resolves names through `lookup`. Tests use this to point
    /// a name wherever they like without touching DNS.
    pub fn with_lookup(local_only: bool, lookup: Arc<dyn HostLookup>) -> Result<Self, AiError> {
        let local_only = Arc::new(AtomicBool::new(local_only));
        let client = build_client(&local_only, &lookup)?;
        Ok(Self {
            local_only,
            lookup,
            client: Mutex::new(client),
        })
    }

    pub fn local_only(&self) -> bool {
        self.local_only.load(Ordering::SeqCst)
    }

    /// Turns local-only mode on or off. Turning it on replaces the client, so
    /// its pool of open connections goes with the old one.
    pub fn set_local_only(&self, on: bool) -> Result<(), AiError> {
        let was = self.local_only.swap(on, Ordering::SeqCst);
        if on && !was {
            let client = build_client(&self.local_only, &self.lookup)?;
            *self.lock_client() = client;
        }
        Ok(())
    }

    fn lock_client(&self) -> std::sync::MutexGuard<'_, reqwest::Client> {
        self.client
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn admit(&self, url: &str) -> Result<reqwest::Url, AiError> {
        let parsed =
            reqwest::Url::parse(url).map_err(|error| Refusal::BadUrl(error.to_string()))?;
        let endpoint = endpoint_of(&parsed)?;
        if self.local_only() && !endpoint.host.is_loopback() {
            return Err(Refusal::LocalOnly {
                host: endpoint.host.to_string(),
            }
            .into());
        }
        Ok(parsed)
    }
}

impl Transport for NetworkGuard {
    fn send(&self, request: HttpRequest) -> BoxFuture<'_, Result<HttpResponse, AiError>> {
        Box::pin(async move {
            let url = self.admit(&request.url)?;
            let client = self.lock_client().clone();
            let mut builder = match request.method {
                Method::Get => client.get(url),
                Method::Post => client.post(url),
            };
            for (name, value) in &request.headers {
                builder = builder.header(name.as_str(), value.as_str());
            }
            if let Some(body) = request.body {
                builder = builder.body(body);
            }
            let response = builder.send().await.map_err(network_error)?;
            let headers = response
                .headers()
                .iter()
                .filter_map(|(name, value)| {
                    Some((name.as_str().to_owned(), value.to_str().ok()?.to_owned()))
                })
                .collect();
            Ok(HttpResponse {
                status: response.status().as_u16(),
                headers,
                body: Box::new(Body(response)),
            })
        })
    }
}

struct Body(reqwest::Response);

impl ByteStream for Body {
    fn next_chunk(&mut self) -> BoxFuture<'_, Result<Option<Vec<u8>>, AiError>> {
        Box::pin(async move {
            let chunk = self.0.chunk().await.map_err(network_error)?;
            Ok(chunk.map(|bytes| bytes.to_vec()))
        })
    }
}

/// The one place a client is built.
#[allow(clippy::disallowed_methods)]
fn build_client(
    local_only: &Arc<AtomicBool>,
    lookup: &Arc<dyn HostLookup>,
) -> Result<reqwest::Client, AiError> {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .dns_resolver(Arc::new(resolve::GuardResolver::new(
            local_only.clone(),
            lookup.clone(),
        )))
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
        .build()
        .map_err(|error| AiError::Network(error.without_url().to_string()))
}

/// Maps a client error. A refusal from the resolver comes back as what it is;
/// anything else loses its URL, which may carry a key in its query.
fn network_error(error: reqwest::Error) -> AiError {
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(&error);
    while let Some(current) = source {
        if let Some(refused) = current.downcast_ref::<resolve::LocalOnlyRefused>() {
            return Refusal::LocalOnly {
                host: refused.host.clone(),
            }
            .into();
        }
        source = current.source();
    }
    AiError::Network(error.without_url().to_string())
}

/// Reads a profile's base URL: http or https, and a host.
pub fn parse_endpoint(url: &str) -> Result<Endpoint, Refusal> {
    let parsed =
        reqwest::Url::parse(url.trim()).map_err(|error| Refusal::BadUrl(error.to_string()))?;
    endpoint_of(&parsed)
}

fn endpoint_of(url: &reqwest::Url) -> Result<Endpoint, Refusal> {
    let https = match url.scheme() {
        "https" => true,
        "http" => false,
        other => return Err(Refusal::BadUrl(format!("{other}: is not http or https"))),
    };
    let host = match url.host_str() {
        // An IPv6 host comes back in its brackets.
        Some(host) => match host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<IpAddr>()
        {
            Ok(ip) => Host::Ip(ip),
            Err(_) => Host::Name(host.to_ascii_lowercase()),
        },
        None => return Err(Refusal::BadUrl("there is no host".into())),
    };
    let port = url
        .port_or_known_default()
        .unwrap_or(if https { 443 } else { 80 });
    Ok(Endpoint { https, host, port })
}

/// Whether an address is this machine. An IPv4 address written as IPv6
/// counts as the address it is.
pub fn is_loopback(address: &SocketAddr) -> bool {
    address.ip().to_canonical().is_loopback()
}

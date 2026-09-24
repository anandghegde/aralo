//! The resolver every guarded connection goes through.

use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::transport::BoxFuture;

/// Turns a host name into addresses.
pub trait HostLookup: Send + Sync {
    fn lookup(&self, host: &str) -> BoxFuture<'static, io::Result<Vec<SocketAddr>>>;
}

/// The system resolver.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemLookup;

impl HostLookup for SystemLookup {
    fn lookup(&self, host: &str) -> BoxFuture<'static, io::Result<Vec<SocketAddr>>> {
        let host = host.to_owned();
        Box::pin(async move { Ok(tokio::net::lookup_host((host.as_str(), 0)).await?.collect()) })
    }
}

/// The error a connection fails with when local-only mode leaves a name no
/// address to use. The guard finds it in the client's error chain.
#[derive(Debug)]
pub(super) struct LocalOnlyRefused {
    pub(super) host: String,
}

impl std::fmt::Display for LocalOnlyRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "local-only mode is on, and {} does not resolve to this Mac",
            self.host
        )
    }
}

impl std::error::Error for LocalOnlyRefused {}

pub(super) struct GuardResolver {
    local_only: Arc<AtomicBool>,
    lookup: Arc<dyn HostLookup>,
}

impl GuardResolver {
    pub(super) fn new(local_only: Arc<AtomicBool>, lookup: Arc<dyn HostLookup>) -> Self {
        Self { local_only, lookup }
    }
}

impl reqwest::dns::Resolve for GuardResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_owned();
        let local_only = self.local_only.clone();
        let lookup = self.lookup.lookup(&host);
        Box::pin(async move {
            let mut addresses = lookup.await?;
            // Read after the lookup, so a switch turned on while it ran counts.
            if local_only.load(Ordering::SeqCst) {
                addresses.retain(super::is_loopback);
                if addresses.is_empty() {
                    return Err(Box::new(LocalOnlyRefused { host })
                        as Box<dyn std::error::Error + Send + Sync>);
                }
            }
            Ok(Box::new(addresses.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

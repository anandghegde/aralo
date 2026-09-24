//! Local-only mode means no network (PRD P6, ADR-0007). This is the test M4's
//! task 4.1 is done by.
//!
//! Every test here that expects a refusal runs against a listener that counts
//! the connections it accepts, or an address nothing answers on, so a refusal
//! that let a connection through first would fail.

use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use aralo_ai::guard::{HostLookup, NetworkGuard};
use aralo_ai::{
    AdapterKind, AiError, AiRequest, BoxFuture, ContextKind, ContextSource, Feature, Gateway,
    HttpRequest, HttpResponse, Method, Policy, Profile, Refusal, Transport,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// An HTTP server on 127.0.0.1 that answers every request with `hello` and
/// counts the connections it accepts.
async fn local_server() -> (SocketAddr, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let accepted = Arc::new(AtomicUsize::new(0));
    let counter = accepted.clone();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            counter.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                let mut request = Vec::new();
                let mut buffer = [0u8; 1024];
                while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                    match socket.read(&mut buffer).await {
                        Ok(0) | Err(_) => return,
                        Ok(read) => request.extend_from_slice(&buffer[..read]),
                    }
                }
                let _ = socket
                    .write_all(
                        b"HTTP/1.1 200 OK\r\ncontent-length: 5\r\nconnection: close\r\n\r\nhello",
                    )
                    .await;
            });
        }
    });
    (address, accepted)
}

/// Points every name at fixed addresses, without DNS.
struct FixedLookup(Vec<SocketAddr>);

impl HostLookup for FixedLookup {
    fn lookup(&self, _host: &str) -> BoxFuture<'static, io::Result<Vec<SocketAddr>>> {
        let addresses = self.0.clone();
        Box::pin(async move { Ok(addresses) })
    }
}

fn get(url: impl Into<String>) -> HttpRequest {
    HttpRequest {
        method: Method::Get,
        url: url.into(),
        headers: Vec::new(),
        body: None,
    }
}

async fn body_of(mut response: HttpResponse) -> String {
    let mut body = Vec::new();
    while let Some(chunk) = response.body.next_chunk().await.unwrap() {
        body.extend(chunk);
    }
    String::from_utf8(body).unwrap()
}

/// A refusal must be immediate. Anything that tried to connect to a
/// documentation address would hang until the connect timeout instead.
async fn refused(guard: &NetworkGuard, url: &str) -> Refusal {
    let outcome = tokio::time::timeout(Duration::from_secs(2), guard.send(get(url)))
        .await
        .unwrap_or_else(|_| panic!("{url}: the guard tried the network instead of refusing"));
    match outcome {
        Err(AiError::Refused(refusal)) => refusal,
        Err(other) => panic!("{url}: expected a refusal, got {other}"),
        Ok(response) => panic!("{url}: expected a refusal, got status {}", response.status),
    }
}

#[tokio::test]
async fn local_only_refuses_a_remote_address_before_connecting() {
    let guard = NetworkGuard::new(true).unwrap();
    // 192.0.2.0/24 and 2001:db8::/32 are for documentation: nothing answers.
    for url in [
        "http://192.0.2.1/v1/chat/completions",
        "https://192.0.2.1:8443/v1",
        "http://[2001:db8::1]/v1",
        "http://[::ffff:192.0.2.1]/v1",
        "https://api.openai.com/v1/models",
        "http://0.0.0.0:11434/v1",
    ] {
        assert!(
            matches!(refused(&guard, url).await, Refusal::LocalOnly { .. }),
            "{url}"
        );
    }
}

#[tokio::test]
async fn local_only_lets_this_mac_through() {
    let (address, accepted) = local_server().await;
    let guard = NetworkGuard::new(true).unwrap();
    let response = guard
        .send(get(format!("http://{address}/v1/models")))
        .await
        .unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(body_of(response).await, "hello");
    assert_eq!(accepted.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn localhost_that_resolves_elsewhere_is_refused_at_connect() {
    // A hosts file, or a hostile resolver, points localhost at another machine.
    let (_, accepted) = local_server().await;
    let lookup = Arc::new(FixedLookup(vec!["192.0.2.7:0".parse().unwrap()]));
    let guard = NetworkGuard::with_lookup(true, lookup).unwrap();
    assert_eq!(
        refused(&guard, "http://localhost:11434/v1").await,
        Refusal::LocalOnly {
            host: "localhost".into()
        }
    );
    assert_eq!(accepted.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_name_that_resolves_both_ways_only_reaches_loopback() {
    let (address, accepted) = local_server().await;
    let lookup = Arc::new(FixedLookup(vec![
        "192.0.2.7:0".parse().unwrap(),
        format!("127.0.0.1:{}", address.port()).parse().unwrap(),
    ]));
    let guard = NetworkGuard::with_lookup(true, lookup).unwrap();
    let response = tokio::time::timeout(
        Duration::from_secs(2),
        guard.send(get(format!("http://localhost:{}/", address.port()))),
    )
    .await
    .expect("the remote address was tried")
    .unwrap();
    assert_eq!(body_of(response).await, "hello");
    assert_eq!(accepted.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn switching_local_only_on_applies_to_the_next_request() {
    let lookup = Arc::new(FixedLookup(vec!["192.0.2.7:0".parse().unwrap()]));
    let guard = NetworkGuard::with_lookup(false, lookup).unwrap();
    assert!(!guard.local_only());
    guard.set_local_only(true).unwrap();
    assert!(matches!(
        refused(&guard, "http://localhost/v1").await,
        Refusal::LocalOnly { .. }
    ));
    assert!(matches!(
        refused(&guard, "https://api.anthropic.com/v1").await,
        Refusal::LocalOnly { .. }
    ));
}

#[tokio::test]
async fn the_guard_follows_no_redirect() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buffer = [0u8; 1024];
        let _ = socket.read(&mut buffer).await;
        let _ = socket
            .write_all(b"HTTP/1.1 307 Temporary Redirect\r\nlocation: http://192.0.2.1/steal\r\ncontent-length: 0\r\n\r\n")
            .await;
    });
    let guard = NetworkGuard::new(false).unwrap();
    let response = tokio::time::timeout(
        Duration::from_secs(2),
        guard.send(get(format!("http://{address}/"))),
    )
    .await
    .expect("the redirect was followed")
    .unwrap();
    assert_eq!(response.status, 307);
}

/// A transport that counts what reaches it, for the gateway-level checks.
#[derive(Default)]
struct Counting(AtomicUsize);

impl Transport for Counting {
    fn send(&self, _request: HttpRequest) -> BoxFuture<'_, Result<HttpResponse, AiError>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Err(AiError::Network("counting transport".into())) })
    }
}

/// Fails the test if the gateway asks for any context at all.
struct NoContext;

impl ContextSource for NoContext {
    fn fetch(&mut self, kind: ContextKind) -> Option<String> {
        panic!("{kind:?} was asked for, but the request should have been refused first");
    }
}

fn request(base_url: &str) -> AiRequest {
    AiRequest {
        feature: Feature::Command,
        profile: Profile {
            name: "remote".into(),
            adapter: AdapterKind::OpenAiCompat,
            base_url: base_url.into(),
            headers: Vec::new(),
            default_model: "gpt".into(),
            key_ref: Some("remote".into()),
        },
        model: None,
        system: "Fix grammar.".into(),
        instruction: "Fix it.".into(),
        declared: vec![ContextKind::Selection, ContextKind::Clipboard],
        max_tokens: None,
        temperature: None,
        json_output: false,
    }
}

#[test]
fn a_remote_profile_in_local_only_mode_is_refused_before_context_is_read() {
    let transport = Arc::new(Counting::default());
    let gateway = Gateway::new(
        Policy {
            ai_enabled: true,
            local_only: true,
            allowed_hosts: None,
        },
        transport.clone(),
    );
    let refusal = gateway
        .prepare(request("https://api.openai.com/v1"), &mut NoContext)
        .unwrap_err();
    assert_eq!(
        refusal,
        Refusal::LocalOnly {
            host: "api.openai.com".into()
        }
    );
    assert_eq!(transport.0.load(Ordering::SeqCst), 0);
}

#[test]
fn with_ai_off_nothing_is_read_and_nothing_is_sent() {
    let transport = Arc::new(Counting::default());
    let gateway = Gateway::new(Policy::default(), transport.clone());
    for url in ["https://api.openai.com/v1", "http://127.0.0.1:11434/v1"] {
        assert_eq!(
            gateway.prepare(request(url), &mut NoContext).unwrap_err(),
            Refusal::Off
        );
    }
    assert_eq!(transport.0.load(Ordering::SeqCst), 0);
}

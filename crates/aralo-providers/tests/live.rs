//! A chat against a real endpoint, through the network guard. Ignored by
//! default: it needs the network and, for most providers, a key. Task 4.2 is
//! done when this passes against OpenAI, OpenRouter, Groq and Ollama.
//!
//! ```sh
//! ARALO_LIVE_BASE_URL=http://127.0.0.1:11434/v1 ARALO_LIVE_MODEL=llama3.2:3b \
//!     cargo test -p aralo-providers --test live -- --ignored --nocapture
//! ```
//!
//! `ARALO_LIVE_KEY` is sent as a bearer token when it is set.
//! `ARALO_LIVE_RECORD=fixtures/sse/<name>.sse` writes the response body, and
//! nothing else, to that file (relative to the repository root), for
//! `tests/transcripts.rs` to replay.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use aralo_ai::guard::NetworkGuard;
use aralo_ai::{
    AdapterKind, AiError, AiRequest, BoxFuture, ByteStream, Delta, Feature, Gateway, HttpRequest,
    HttpResponse, Policy, Profile, Secret, Transport,
};

/// The guard, with every response body copied into `recorded`.
struct Recorder {
    guard: NetworkGuard,
    recorded: Arc<Mutex<Vec<u8>>>,
}

impl Transport for Recorder {
    fn send(&self, request: HttpRequest) -> BoxFuture<'_, Result<HttpResponse, AiError>> {
        Box::pin(async move {
            let mut response = self.guard.send(request).await?;
            let inner = std::mem::replace(&mut response.body, Box::new(Empty));
            response.body = Box::new(Tee {
                inner,
                copy: self.recorded.clone(),
            });
            Ok(response)
        })
    }
}

struct Empty;

impl ByteStream for Empty {
    fn next_chunk(&mut self) -> BoxFuture<'_, Result<Option<Vec<u8>>, AiError>> {
        Box::pin(async { Ok(None) })
    }
}

struct Tee {
    inner: Box<dyn ByteStream>,
    copy: Arc<Mutex<Vec<u8>>>,
}

impl ByteStream for Tee {
    fn next_chunk(&mut self) -> BoxFuture<'_, Result<Option<Vec<u8>>, AiError>> {
        Box::pin(async move {
            let chunk = self.inner.next_chunk().await?;
            if let Some(bytes) = &chunk {
                self.copy.lock().unwrap().extend_from_slice(bytes);
            }
            Ok(chunk)
        })
    }
}

#[tokio::test]
#[ignore = "needs a live endpoint; see the file's comment"]
async fn a_live_endpoint_streams() {
    let base_url = std::env::var("ARALO_LIVE_BASE_URL").expect("set ARALO_LIVE_BASE_URL");
    let model = std::env::var("ARALO_LIVE_MODEL").expect("set ARALO_LIVE_MODEL");
    let key = std::env::var("ARALO_LIVE_KEY").ok().map(Secret::new);
    let record = std::env::var("ARALO_LIVE_RECORD").ok();

    let recorded = Arc::new(Mutex::new(Vec::new()));
    let transport = Arc::new(Recorder {
        guard: NetworkGuard::new(false).unwrap(),
        recorded: recorded.clone(),
    });
    let mut gateway = Gateway::new(
        Policy {
            ai_enabled: true,
            ..Policy::default()
        },
        transport,
    );
    aralo_providers::register_all(&mut gateway);

    let prepared = gateway
        .prepare(
            AiRequest {
                feature: Feature::Conformance,
                profile: Profile {
                    name: "live".into(),
                    adapter: AdapterKind::OpenAiCompat,
                    base_url,
                    headers: Vec::new(),
                    default_model: model,
                    key_ref: None,
                },
                model: None,
                system: "Reply with exactly the text you are given, and nothing else.".into(),
                instruction: "The quick brown fox — ünïcödé ✓".into(),
                declared: Vec::new(),
                max_tokens: Some(64),
                temperature: Some(0.0),
                json_output: false,
            },
            &mut (),
        )
        .unwrap();

    let started = Instant::now();
    let mut stream = gateway.send(prepared, key).await.unwrap();
    let mut text = String::new();
    let mut pieces = 0;
    let mut first_token = None;
    let mut done = None;
    while let Some(delta) = tokio::time::timeout(Duration::from_secs(60), stream.next())
        .await
        .expect("the stream went quiet for a minute")
    {
        match delta.unwrap() {
            Delta::Text(piece) => {
                first_token.get_or_insert_with(|| started.elapsed());
                pieces += 1;
                text.push_str(&piece);
            }
            Delta::Usage(usage) => eprintln!("usage: {usage:?}"),
            Delta::Done(reason) => done = Some(reason),
        }
    }
    eprintln!(
        "{pieces} pieces, first after {:?}, done: {done:?}\n{text}",
        first_token.unwrap_or_default()
    );
    assert!(!text.trim().is_empty(), "the model wrote nothing");
    assert!(done.is_some(), "the stream ended without Done");

    if let Some(path) = record {
        // A relative path is taken from the repository root, not this crate.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(path);
        std::fs::write(&path, &*recorded.lock().unwrap()).unwrap();
        eprintln!("recorded the response body to {}", path.display());
    }
}

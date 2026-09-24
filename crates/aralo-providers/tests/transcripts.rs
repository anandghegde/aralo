//! Recorded chat streams from `fixtures/sse/`, replayed through the
//! `openai_compat` adapter and the gateway, cut into chunks of every size
//! that matters: one byte, a few bytes, and the whole body.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use aralo_ai::{
    AdapterKind, AiError, AiRequest, BoxFuture, ByteStream, Delta, Feature, Gateway, HttpRequest,
    HttpResponse, Policy, Profile, RetryPolicy, Secret, StopReason, Transport, Usage,
};

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/sse")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

#[derive(Clone)]
struct Answer {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

fn stream(body: Vec<u8>) -> Answer {
    Answer {
        status: 200,
        headers: vec![("content-type".into(), "text/event-stream".into())],
        body,
    }
}

fn json(status: u16, body: &str) -> Answer {
    Answer {
        status,
        headers: vec![("content-type".into(), "application/json".into())],
        body: body.as_bytes().to_vec(),
    }
}

/// Answers each request with the next scripted answer, handing the body out
/// `chunk` bytes at a time, and keeps every request it was sent.
struct Replay {
    answers: Mutex<VecDeque<Answer>>,
    chunk: usize,
    sent: Mutex<Vec<HttpRequest>>,
}

impl Replay {
    fn new(answers: Vec<Answer>, chunk: usize) -> Arc<Self> {
        Arc::new(Self {
            answers: Mutex::new(answers.into()),
            chunk,
            sent: Mutex::default(),
        })
    }

    fn sent(&self) -> Vec<HttpRequest> {
        self.sent.lock().unwrap().clone()
    }
}

impl Transport for Replay {
    fn send(&self, request: HttpRequest) -> BoxFuture<'_, Result<HttpResponse, AiError>> {
        self.sent.lock().unwrap().push(request);
        let answer = self
            .answers
            .lock()
            .unwrap()
            .pop_front()
            .expect("more requests than answers");
        let chunk = self.chunk;
        Box::pin(async move {
            Ok(HttpResponse {
                status: answer.status,
                headers: answer.headers,
                body: Box::new(Chunks {
                    body: answer.body,
                    at: 0,
                    size: chunk,
                }),
            })
        })
    }
}

struct Chunks {
    body: Vec<u8>,
    at: usize,
    size: usize,
}

impl ByteStream for Chunks {
    fn next_chunk(&mut self) -> BoxFuture<'_, Result<Option<Vec<u8>>, AiError>> {
        let end = self.at.saturating_add(self.size).min(self.body.len());
        let chunk = (self.at < end).then(|| self.body[self.at..end].to_vec());
        self.at = end;
        Box::pin(async move { Ok(chunk) })
    }
}

fn profile(base_url: &str) -> Profile {
    Profile {
        name: "test".into(),
        adapter: AdapterKind::OpenAiCompat,
        base_url: base_url.into(),
        headers: Vec::new(),
        default_model: "model".into(),
        key_ref: Some("test".into()),
    }
}

fn gateway(transport: Arc<Replay>) -> Gateway {
    let mut gateway = Gateway::new(
        Policy {
            ai_enabled: true,
            ..Policy::default()
        },
        transport,
    )
    .with_retry(RetryPolicy {
        attempts: 2,
        backoff: Duration::from_millis(1),
    });
    aralo_providers::register_all(&mut gateway);
    gateway
}

fn request(base_url: &str) -> AiRequest {
    AiRequest {
        feature: Feature::Command,
        profile: profile(base_url),
        model: None,
        system: "You fix spelling.".into(),
        instruction: "Fix: teh quick brown fox".into(),
        declared: Vec::new(),
        max_tokens: Some(64),
        temperature: None,
        json_output: false,
    }
}

/// Everything the stream produced, or the error it stopped on.
async fn run(transport: Arc<Replay>) -> (Vec<Delta>, Option<AiError>, Gateway) {
    let gateway = gateway(transport);
    let prepared = gateway
        .prepare(request("https://api.example.com/v1"), &mut ())
        .unwrap();
    let mut stream = match gateway.send(prepared, Some(Secret::new("sk-test"))).await {
        Ok(stream) => stream,
        Err(error) => return (Vec::new(), Some(error), gateway),
    };
    let mut deltas = Vec::new();
    let mut error = None;
    while let Some(delta) = stream.next().await {
        match delta {
            Ok(delta) => deltas.push(delta),
            Err(stopped) => error = Some(stopped),
        }
    }
    drop(stream);
    (deltas, error, gateway)
}

fn text_of(deltas: &[Delta]) -> String {
    deltas
        .iter()
        .filter_map(|delta| match delta {
            Delta::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

const CHUNK_SIZES: [usize; 3] = [1, 7, usize::MAX];
const ANSWER: &str = "The quick brown fox — ünïcödé ✓";

#[tokio::test]
async fn each_provider_streams_text_then_usage_then_done() {
    for (file, cached) in [
        ("openai.sse", 1024),
        ("openrouter.sse", 1024),
        ("groq.sse", 0),
        ("ollama.sse", 0),
    ] {
        for size in CHUNK_SIZES {
            let transport = Replay::new(vec![stream(fixture(file))], size);
            let (deltas, error, gateway) = run(transport.clone()).await;
            assert!(error.is_none(), "{file} in {size}-byte chunks: {error:?}");
            assert_eq!(text_of(&deltas), ANSWER, "{file} in {size}-byte chunks");
            let usage = Usage {
                input_tokens: 1320,
                output_tokens: 9,
                cached_input_tokens: cached,
            };
            assert_eq!(
                deltas[deltas.len() - 2..],
                [Delta::Usage(usage), Delta::Done(StopReason::End)],
                "{file} in {size}-byte chunks: usage once, then done, and nothing after"
            );
            assert_eq!(gateway.meter().usage(Feature::Command, "test"), usage);

            let sent = transport.sent();
            assert_eq!(sent.len(), 1);
            assert_eq!(sent[0].url, "https://api.example.com/v1/chat/completions");
        }
    }
}

/// A real recording. The model streams its reasoning as `reasoning_content`
/// before the answer, and usage comes twice: the upstream count on the chunk
/// with the finish reason, then the reseller's billed count, with its cost, in
/// a chunk with no choices. The billed one is last and is the one metered.
#[tokio::test]
async fn reasoning_is_left_out_and_the_last_usage_report_is_metered() {
    for size in CHUNK_SIZES {
        let transport = Replay::new(vec![stream(fixture("surplus-reasoning.sse"))], size);
        let (deltas, error, gateway) = run(transport).await;
        assert!(error.is_none(), "{size}-byte chunks: {error:?}");
        assert_eq!(text_of(&deltas), ANSWER, "{size}-byte chunks");
        let usage = Usage {
            input_tokens: 46,
            output_tokens: 46,
            cached_input_tokens: 0,
        };
        assert_eq!(
            deltas[deltas.len() - 2..],
            [Delta::Usage(usage), Delta::Done(StopReason::End)],
            "{size}-byte chunks"
        );
        assert_eq!(gateway.meter().usage(Feature::Command, "test"), usage);
    }
}

#[tokio::test]
async fn a_cut_off_stream_is_an_error_after_the_text_it_had() {
    for size in CHUNK_SIZES {
        let (deltas, error, _) = run(Replay::new(vec![stream(fixture("cut-off.sse"))], size)).await;
        assert_eq!(text_of(&deltas), "The quick brown");
        assert!(!deltas.iter().any(|delta| matches!(delta, Delta::Done(_))));
        assert!(
            matches!(&error, Some(AiError::Protocol(message)) if message.contains("ended before")),
            "{error:?}"
        );
    }
}

#[tokio::test]
async fn a_chunk_that_is_not_json_stops_the_stream_after_the_text_before_it() {
    for size in CHUNK_SIZES {
        let (deltas, error, _) =
            run(Replay::new(vec![stream(fixture("not-json.sse"))], size)).await;
        assert_eq!(text_of(&deltas), "The quick", "{size}-byte chunks");
        assert!(matches!(error, Some(AiError::Protocol(_))), "{error:?}");
    }
}

#[tokio::test]
async fn an_error_mid_stream_is_reported_and_not_retried() {
    for size in CHUNK_SIZES {
        let transport = Replay::new(
            vec![stream(fixture("openrouter-error-midstream.sse"))],
            size,
        );
        let (deltas, error, _) = run(transport.clone()).await;
        assert_eq!(text_of(&deltas), "The quick", "{size}-byte chunks");
        match error {
            Some(AiError::Status {
                status, message, ..
            }) => {
                assert_eq!(status, 502);
                assert_eq!(message, "Provider disconnected");
            }
            other => panic!("expected a status error, got {other:?}"),
        }
        assert_eq!(transport.sent().len(), 1);
    }
}

#[tokio::test]
async fn an_error_status_carries_the_providers_message() {
    let transport = Replay::new(
        vec![json(
            401,
            r#"{"error":{"message":"Incorrect API key provided: sk-test. You can find your API key at https://platform.openai.com/account/api-keys.","type":"invalid_request_error","param":null,"code":"invalid_api_key"}}"#,
        )],
        usize::MAX,
    );
    let (_, error, _) = run(transport.clone()).await;
    match error {
        Some(AiError::Status {
            status, message, ..
        }) => {
            assert_eq!(status, 401);
            assert!(message.starts_with("Incorrect API key provided"));
        }
        other => panic!("expected a status error, got {other:?}"),
    }
    assert_eq!(transport.sent().len(), 1, "a 401 is not retried");
}

#[tokio::test]
async fn a_rate_limit_is_retried_before_the_first_token() {
    let mut limited = json(
        429,
        r#"{"error":{"message":"Rate limit reached","type":"tokens"}}"#,
    );
    limited.headers.push(("retry-after".into(), "0".into()));
    let transport = Replay::new(
        vec![
            limited,
            json(503, "<html>upstream unavailable</html>"),
            stream(fixture("openai.sse")),
        ],
        usize::MAX,
    );
    let (deltas, error, _) = run(transport.clone()).await;
    assert!(error.is_none(), "{error:?}");
    assert_eq!(text_of(&deltas), ANSWER);
    assert_eq!(transport.sent().len(), 3);
}

#[tokio::test]
async fn an_answer_that_did_not_stream_is_read_whole() {
    let transport = Replay::new(
        vec![json(
            200,
            r#"{"id":"chatcmpl-1","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"The quick brown fox"},"finish_reason":"length"}],"usage":{"prompt_tokens":12,"completion_tokens":4,"total_tokens":16}}"#,
        )],
        3,
    );
    let (deltas, error, _) = run(transport).await;
    assert!(error.is_none(), "{error:?}");
    assert_eq!(
        deltas,
        [
            Delta::Text("The quick brown fox".into()),
            Delta::Usage(Usage {
                input_tokens: 12,
                output_tokens: 4,
                cached_input_tokens: 0
            }),
            Delta::Done(StopReason::Length),
        ]
    );
}

#[tokio::test]
async fn model_lists_are_read_sorted() {
    let transport = Replay::new(
        vec![json(
            200,
            r#"{"object":"list","data":[{"id":"llama3.2:3b","object":"model","owned_by":"library"},{"id":"gemma3:4b","object":"model","owned_by":"library"},{"id":"gemma3:4b","object":"model"}]}"#,
        )],
        5,
    );
    let gateway = gateway(transport.clone());
    let models = gateway
        .list_models(&profile("http://127.0.0.1:11434/v1"), None)
        .await
        .unwrap();
    assert_eq!(models, ["gemma3:4b", "llama3.2:3b"]);
    let sent = transport.sent();
    assert_eq!(sent[0].url, "http://127.0.0.1:11434/v1/models");
    assert!(sent[0]
        .headers
        .iter()
        .all(|(name, _)| !name.eq_ignore_ascii_case("authorization")));
}

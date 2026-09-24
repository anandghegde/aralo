//! The gateway's stages with a scripted adapter: declared context only, the
//! framing the adapter receives, retries before the first token and none
//! after, metering, and the policy checked again at send.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use aralo_ai::{
    Adapter, AdapterKind, AiError, AiRequest, BoxFuture, Call, ChatRequest, ContextKind,
    ContextSource, Delta, DeltaStream, Feature, Gateway, HttpRequest, HttpResponse, ManifestEntry,
    Policy, Profile, Refusal, RetryPolicy, Secret, StopReason, Transport, Usage,
};

/// A transport the scripted adapter never uses.
struct Offline;

impl Transport for Offline {
    fn send(&self, _request: HttpRequest) -> BoxFuture<'_, Result<HttpResponse, AiError>> {
        Box::pin(async { Err(AiError::Network("offline".into())) })
    }
}

type Script = Result<Vec<Delta>, AiError>;

/// Answers each call with the next scripted outcome, and remembers what it
/// was sent and whether it had a key.
#[derive(Default)]
struct Scripted {
    outcomes: Mutex<VecDeque<Script>>,
    calls: Mutex<Vec<(ChatRequest, bool)>>,
}

impl Scripted {
    fn new(outcomes: Vec<Script>) -> Arc<Self> {
        Arc::new(Self {
            outcomes: Mutex::new(outcomes.into()),
            calls: Mutex::default(),
        })
    }

    fn calls(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}

impl Adapter for Scripted {
    fn kind(&self) -> AdapterKind {
        AdapterKind::OpenAiCompat
    }

    fn chat_stream<'a>(
        &'a self,
        _http: &'a dyn Transport,
        call: Call<'a>,
    ) -> BoxFuture<'a, Result<Box<dyn DeltaStream>, AiError>> {
        self.calls
            .lock()
            .unwrap()
            .push((call.request.clone(), call.key.is_some()));
        let outcome = self
            .outcomes
            .lock()
            .unwrap()
            .pop_front()
            .expect("more calls than scripted");
        Box::pin(async move {
            outcome.map(|deltas| Box::new(Replay(deltas.into())) as Box<dyn DeltaStream>)
        })
    }

    fn list_models<'a>(
        &'a self,
        _http: &'a dyn Transport,
        _profile: &'a Profile,
        _key: Option<&'a Secret>,
    ) -> BoxFuture<'a, Result<Vec<String>, AiError>> {
        Box::pin(async { Ok(vec!["gpt-default".into()]) })
    }
}

struct Replay(VecDeque<Delta>);

impl DeltaStream for Replay {
    fn next(&mut self) -> BoxFuture<'_, Option<Result<Delta, AiError>>> {
        let next = self.0.pop_front().map(Ok);
        Box::pin(async move { next })
    }
}

/// Hands out what it holds and records every kind it was asked for.
#[derive(Default)]
struct Shell {
    asked: Vec<ContextKind>,
}

impl ContextSource for Shell {
    fn fetch(&mut self, kind: ContextKind) -> Option<String> {
        self.asked.push(kind);
        match kind {
            ContextKind::Selection => Some("teh quick brown fox".into()),
            ContextKind::Clipboard => Some("SECRET CLIPBOARD".into()),
            ContextKind::App => Some("Mail".into()),
            ContextKind::Fillins | ContextKind::Window => None,
        }
    }
}

fn gateway(adapter: Arc<Scripted>) -> Gateway {
    let mut gateway = Gateway::new(
        Policy {
            ai_enabled: true,
            ..Policy::default()
        },
        Arc::new(Offline),
    )
    .with_retry(RetryPolicy {
        attempts: 2,
        backoff: Duration::from_millis(500),
    });
    gateway.register(adapter);
    gateway
}

fn request(declared: Vec<ContextKind>) -> AiRequest {
    AiRequest {
        feature: Feature::Command,
        profile: Profile {
            name: "openai".into(),
            adapter: AdapterKind::OpenAiCompat,
            base_url: "https://api.openai.com/v1".into(),
            headers: Vec::new(),
            default_model: "gpt-default".into(),
            key_ref: Some("openai".into()),
        },
        model: None,
        system: "You fix spelling.".into(),
        instruction: "Fix the spelling.".into(),
        declared,
        max_tokens: Some(200),
        temperature: None,
        json_output: false,
    }
}

fn answer(text: &str, input: u64, output: u64) -> Script {
    Ok(vec![
        Delta::Text(text.into()),
        Delta::Usage(Usage {
            input_tokens: input,
            output_tokens: output,
            cached_input_tokens: 0,
        }),
        Delta::Done(StopReason::End),
    ])
}

fn status(code: u16) -> Script {
    Err(AiError::Status {
        status: code,
        message: "scripted".into(),
        retry_after: None,
    })
}

#[test]
fn only_declared_context_is_asked_for_or_sent() {
    let adapter = Scripted::new(Vec::new());
    let gateway = gateway(adapter);
    let mut shell = Shell::default();
    let prepared = gateway
        .prepare(
            request(vec![
                ContextKind::Selection,
                ContextKind::Window,
                ContextKind::Selection,
            ]),
            &mut shell,
        )
        .unwrap();

    assert_eq!(shell.asked, [ContextKind::Selection, ContextKind::Window]);
    assert_eq!(
        prepared.manifest(),
        [
            ManifestEntry {
                kind: ContextKind::Selection,
                bytes: Some(19)
            },
            ManifestEntry {
                kind: ContextKind::Window,
                bytes: None
            },
        ]
    );
    let sent = &prepared.chat().messages[0].content;
    assert!(sent.contains("teh quick brown fox"));
    assert!(!sent.contains("SECRET CLIPBOARD"));
    assert!(sent.ends_with("Fix the spelling."));
    assert_eq!(prepared.chat().model, "gpt-default");
    assert!(prepared.chat().system.starts_with("You fix spelling."));
}

#[test]
fn a_request_that_declares_nothing_asks_for_nothing() {
    let gateway = gateway(Scripted::new(Vec::new()));
    let mut shell = Shell::default();
    let prepared = gateway.prepare(request(Vec::new()), &mut shell).unwrap();
    assert!(shell.asked.is_empty());
    assert!(prepared.manifest().is_empty());
    assert_eq!(prepared.chat().messages[0].content, "Fix the spelling.");
}

#[test]
fn a_snippet_model_overrides_the_profile_and_no_model_at_all_is_refused() {
    let gateway = gateway(Scripted::new(Vec::new()));
    let mut named = request(Vec::new());
    named.model = Some("gpt-snippet".into());
    assert_eq!(
        gateway.prepare(named, &mut ()).unwrap().chat().model,
        "gpt-snippet"
    );

    let mut none = request(Vec::new());
    none.profile.default_model = String::new();
    assert_eq!(
        gateway.prepare(none, &mut ()).unwrap_err(),
        Refusal::NoModel
    );
}

#[test]
fn a_profile_with_no_adapter_is_refused() {
    let gateway = gateway(Scripted::new(Vec::new()));
    let mut anthropic = request(Vec::new());
    anthropic.profile.adapter = AdapterKind::Anthropic;
    assert_eq!(
        gateway.prepare(anthropic, &mut ()).unwrap_err(),
        Refusal::NoAdapter("anthropic")
    );
}

#[tokio::test(start_paused = true)]
async fn rate_limits_and_server_errors_are_retried_twice_before_the_first_token() {
    let adapter = Scripted::new(vec![status(429), status(503), answer("the quick", 12, 3)]);
    let gateway = gateway(adapter.clone());
    let prepared = gateway.prepare(request(Vec::new()), &mut ()).unwrap();

    let started = tokio::time::Instant::now();
    let stream = gateway
        .send(prepared, Some(Secret::new("sk-test")))
        .await
        .unwrap();
    assert_eq!(started.elapsed(), Duration::from_millis(500 + 1000));
    assert_eq!(stream.collect_text().await.unwrap(), "the quick");
    assert_eq!(adapter.calls(), 3);
    assert!(adapter
        .calls
        .lock()
        .unwrap()
        .iter()
        .all(|(_, keyed)| *keyed));
}

#[tokio::test(start_paused = true)]
async fn a_third_failure_is_returned() {
    let adapter = Scripted::new(vec![status(500), status(502), status(504)]);
    let gateway = gateway(adapter.clone());
    let prepared = gateway.prepare(request(Vec::new()), &mut ()).unwrap();
    let error = gateway.send(prepared, None).await.unwrap_err();
    assert!(matches!(error, AiError::Status { status: 504, .. }));
    assert_eq!(adapter.calls(), 3);
}

#[tokio::test(start_paused = true)]
async fn a_client_error_is_not_retried() {
    for code in [400, 401, 403, 404] {
        let adapter = Scripted::new(vec![status(code)]);
        let gateway = gateway(adapter.clone());
        let prepared = gateway.prepare(request(Vec::new()), &mut ()).unwrap();
        assert!(gateway.send(prepared, None).await.is_err());
        assert_eq!(adapter.calls(), 1, "{code}");
    }
}

#[tokio::test(start_paused = true)]
async fn retry_after_is_honoured_up_to_ten_seconds() {
    let adapter = Scripted::new(vec![
        Err(AiError::Status {
            status: 429,
            message: "slow down".into(),
            retry_after: Some(Duration::from_secs(3)),
        }),
        Err(AiError::Status {
            status: 429,
            message: "slow down".into(),
            retry_after: Some(Duration::from_secs(600)),
        }),
        answer("ok", 1, 1),
    ]);
    let gateway = gateway(adapter);
    let prepared = gateway.prepare(request(Vec::new()), &mut ()).unwrap();
    let started = tokio::time::Instant::now();
    gateway.send(prepared, None).await.unwrap();
    assert_eq!(started.elapsed(), Duration::from_secs(13));
}

#[tokio::test]
async fn usage_is_metered_per_feature_and_profile_and_a_cap_only_warns() {
    let adapter = Scripted::new(vec![answer("one", 60, 30), answer("two", 10, 5)]);
    let gateway = gateway(adapter);
    gateway.meter().set_soft_cap("openai", Some(100));

    let first = gateway
        .send(gateway.prepare(request(Vec::new()), &mut ()).unwrap(), None)
        .await
        .unwrap();
    assert_eq!(first.collect_text().await.unwrap(), "one");

    let mut block = request(Vec::new());
    block.feature = Feature::Block;
    let mut second = gateway
        .send(gateway.prepare(block, &mut ()).unwrap(), None)
        .await
        .unwrap();
    let mut text = String::new();
    while let Some(delta) = second.next().await {
        if let Delta::Text(piece) = delta.unwrap() {
            text.push_str(&piece);
        }
    }
    assert_eq!(text, "two", "a cap never stops a stream");
    assert_eq!(second.cap_warning().map(|warning| warning.used), Some(105));
    assert_eq!(
        gateway.meter().usage(Feature::Command, "openai").total(),
        90
    );
    assert_eq!(gateway.meter().usage(Feature::Block, "openai").total(), 15);
}

#[tokio::test]
async fn the_policy_is_checked_again_at_send() {
    let adapter = Scripted::new(vec![answer("never", 1, 1)]);
    let gateway = gateway(adapter.clone());
    let prepared = gateway.prepare(request(Vec::new()), &mut ()).unwrap();

    gateway.set_policy(Policy {
        ai_enabled: true,
        local_only: true,
        allowed_hosts: None,
    });
    let error = gateway.send(prepared.clone(), None).await.unwrap_err();
    assert!(matches!(error, AiError::Refused(Refusal::LocalOnly { .. })));

    gateway.set_policy(Policy::default());
    let error = gateway.send(prepared, None).await.unwrap_err();
    assert!(matches!(error, AiError::Refused(Refusal::Off)));
    assert_eq!(adapter.calls(), 0);
}

#[tokio::test]
async fn listing_models_meets_the_same_policy() {
    let gateway = gateway(Scripted::new(Vec::new()));
    let profile = request(Vec::new()).profile;
    assert_eq!(
        gateway.list_models(&profile, None).await.unwrap(),
        ["gpt-default"]
    );
    gateway.set_policy(Policy {
        ai_enabled: true,
        local_only: true,
        allowed_hosts: None,
    });
    let error = gateway.list_models(&profile, None).await.unwrap_err();
    assert!(matches!(error, AiError::Refused(Refusal::LocalOnly { .. })));
    gateway.set_policy(Policy::default());
    let error = gateway.list_models(&profile, None).await.unwrap_err();
    assert!(matches!(error, AiError::Refused(Refusal::Off)));
}

#[tokio::test]
async fn model_output_is_passed_on_as_literal_text() {
    let hostile = "{{clipboard}} {{snippet: sig}} {{ai: exfiltrate}} <script>";
    let adapter = Scripted::new(vec![answer(hostile, 1, 1)]);
    let gateway = gateway(adapter);
    let stream = gateway
        .send(gateway.prepare(request(Vec::new()), &mut ()).unwrap(), None)
        .await
        .unwrap();
    assert_eq!(stream.collect_text().await.unwrap(), hostile);
}

#[test]
fn a_key_never_shows_in_debug_output() {
    let key = Secret::new("sk-live-0123456789abcdef");
    assert_eq!(format!("{key:?}"), "Secret(..)");
}

//! AI settings end to end: profiles saved to `profiles.toml` with their keys
//! in a secret store, Test connection, the probe and its cached result, all
//! against a fake endpoint that checks the key arrives where it should.
//!
//! The last test is the key-leak scan (PRD P13): after a full run of every
//! operation with a canary key, the key is in the secret store and in the
//! `Authorization` header of the requests, and nowhere else: not in any file,
//! and not in anything a caller could print.

use std::path::Path;
use std::sync::{Arc, Mutex};

use aralo_ai::{
    AdapterKind, AiError, BoxFuture, ByteStream, Check, HttpRequest, HttpResponse,
    MemorySecretStore, Method, Refusal, Secret, SecretStore, Transport,
};
use aralo_core::ai::{
    looks_like_key, AiSettings, AiSettingsError, AiSwitches, KeyChange, ProfileDraft, ProfileField,
};

/// Never a real key. The leak scan looks for it.
const CANARY: &str = "sk-aralo-canary-7d1f0c2e9b4a4f63a8e5d0c4";

/// An OpenAI-compatible endpoint in memory. It answers the models route, a
/// JSON-mode chat with a JSON object, the probe's system prompt with what it
/// asks for, and any other chat with "ok". A request without the right key
/// gets a 401.
struct FakeEndpoint {
    key: Option<String>,
    sent: Mutex<Vec<HttpRequest>>,
}

impl FakeEndpoint {
    fn new(key: Option<&str>) -> Arc<Self> {
        Arc::new(Self {
            key: key.map(str::to_owned),
            sent: Mutex::default(),
        })
    }

    fn sent(&self) -> Vec<HttpRequest> {
        self.sent.lock().unwrap().clone()
    }

    fn answer(&self, request: &HttpRequest) -> (u16, &'static str, Vec<u8>) {
        let authorization = request
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
            .map(|(_, value)| value.clone());
        if let Some(key) = &self.key {
            if authorization.as_deref() != Some(&format!("Bearer {key}")) {
                return (
                    401,
                    "application/json",
                    br#"{"error":{"message":"Invalid API key"}}"#.to_vec(),
                );
            }
        }
        if request.method == Method::Get && request.url.ends_with("/models") {
            return (
                200,
                "application/json",
                br#"{"data":[{"id":"model-b"},{"id":"model-a"}]}"#.to_vec(),
            );
        }
        let body = String::from_utf8(request.body.clone().unwrap_or_default()).unwrap();
        let pieces: Vec<&str> = if body.contains("json_object") {
            vec!["{\"ok\"", ": true}"]
        } else if body.contains("PINEAPPLE") {
            vec![
                "one two three ",
                "four five six ",
                "seven eight nine ",
                "ten PINEAPPLE",
            ]
        } else {
            vec!["ok"]
        };
        let mut sse = String::new();
        for piece in pieces {
            let chunk = serde_json::json!({"choices":[{"index":0,"delta":{"content":piece}}]});
            sse.push_str(&format!("data: {chunk}\n\n"));
        }
        sse.push_str(
            "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],\
             \"usage\":{\"prompt_tokens\":12,\"completion_tokens\":4}}\n\ndata: [DONE]\n\n",
        );
        (200, "text/event-stream", sse.into_bytes())
    }
}

impl Transport for FakeEndpoint {
    fn send(&self, request: HttpRequest) -> BoxFuture<'_, Result<HttpResponse, AiError>> {
        let (status, content_type, body) = self.answer(&request);
        self.sent.lock().unwrap().push(request);
        Box::pin(async move {
            Ok(HttpResponse {
                status,
                headers: vec![("content-type".into(), content_type.into())],
                body: Box::new(Body(Some(body))),
            })
        })
    }
}

struct Body(Option<Vec<u8>>);

impl ByteStream for Body {
    fn next_chunk(&mut self) -> BoxFuture<'_, Result<Option<Vec<u8>>, AiError>> {
        let chunk = self.0.take();
        Box::pin(async move { Ok(chunk) })
    }
}

struct Setup {
    folder: aralo_testkit::TempDir,
    secrets: Arc<MemorySecretStore>,
    endpoint: Arc<FakeEndpoint>,
    settings: AiSettings,
}

fn setup(enabled: bool) -> Setup {
    let folder = aralo_testkit::tempdir().unwrap();
    let secrets = Arc::new(MemorySecretStore::new());
    let endpoint = FakeEndpoint::new(Some(CANARY));
    let settings = AiSettings::with_transport(
        folder.path().join("profiles.toml"),
        secrets.clone(),
        endpoint.clone(),
    )
    .unwrap()
    .with_clock(|| "2026-09-24T10:00:00Z".into());
    settings
        .set_switches(AiSwitches {
            enabled,
            local_only: false,
        })
        .unwrap();
    Setup {
        folder,
        secrets,
        endpoint,
        settings,
    }
}

fn draft(name: &str) -> ProfileDraft {
    ProfileDraft {
        original_name: None,
        name: name.into(),
        adapter: AdapterKind::OpenAiCompat,
        base_url: "https://api.example.com/v1/".into(),
        default_model: "model-a".into(),
        headers: vec![("X-Title".into(), "Aralo".into())],
    }
}

fn editing(name: &str) -> ProfileDraft {
    ProfileDraft {
        original_name: Some(name.into()),
        ..draft(name)
    }
}

fn canary() -> KeyChange {
    KeyChange::Set(Secret::new(CANARY))
}

fn file_text(setup: &Setup) -> String {
    std::fs::read_to_string(setup.settings.path()).unwrap()
}

fn field_of(error: AiSettingsError) -> ProfileField {
    match error {
        AiSettingsError::Invalid { field, .. } => field,
        other => panic!("expected a problem with a field, got {other:?}"),
    }
}

#[test]
fn the_first_profile_saved_is_the_default_and_its_key_goes_to_the_store() {
    let setup = setup(true);
    let saved = setup.settings.save(&draft("Example"), canary()).unwrap();
    assert!(saved.is_default);
    assert_eq!(saved.profile.base_url, "https://api.example.com/v1");
    assert_eq!(saved.capabilities, None);

    let key_ref = saved.profile.key_ref.clone().expect("a key reference");
    assert_eq!(
        setup.secrets.get(&key_ref).unwrap().unwrap().expose(),
        CANARY
    );
    let text = file_text(&setup);
    assert!(text.contains(&key_ref), "{text}");
    assert!(!text.contains(CANARY), "the key was written to the file");

    let second = setup
        .settings
        .save(&draft("Second"), KeyChange::Remove)
        .unwrap();
    assert!(!second.is_default);
    assert_eq!(second.profile.key_ref, None);
    let names: Vec<_> = setup
        .settings
        .profiles()
        .unwrap()
        .into_iter()
        .map(|saved| saved.profile.name)
        .collect();
    assert_eq!(names, ["Example", "Second"]);
}

#[test]
fn a_draft_that_cannot_work_is_refused_with_the_field_to_fix() {
    let setup = setup(true);
    setup.settings.save(&draft("Example"), canary()).unwrap();

    let cases: Vec<(ProfileDraft, ProfileField)> = vec![
        (
            ProfileDraft {
                name: "  ".into(),
                ..draft("x")
            },
            ProfileField::Name,
        ),
        (draft("EXAMPLE"), ProfileField::Name),
        (
            ProfileDraft {
                adapter: AdapterKind::Anthropic,
                ..draft("x")
            },
            ProfileField::Adapter,
        ),
        (
            ProfileDraft {
                base_url: "api.example.com".into(),
                ..draft("x")
            },
            ProfileField::BaseUrl,
        ),
        (
            ProfileDraft {
                base_url: format!("https://api.example.com/v1?key={CANARY}"),
                ..draft("x")
            },
            ProfileField::BaseUrl,
        ),
        (
            ProfileDraft {
                default_model: " ".into(),
                ..draft("x")
            },
            ProfileField::Model,
        ),
        (
            ProfileDraft {
                headers: vec![("Authorization".into(), "Bearer abc".into())],
                ..draft("x")
            },
            ProfileField::Headers,
        ),
        (
            ProfileDraft {
                headers: vec![("X-Custom-Auth".into(), CANARY.into())],
                ..draft("x")
            },
            ProfileField::Headers,
        ),
        (
            ProfileDraft {
                headers: vec![("Bad Name".into(), "x".into())],
                ..draft("x")
            },
            ProfileField::Headers,
        ),
        (
            ProfileDraft {
                headers: vec![("X-A".into(), "1".into()), ("x-a".into(), "2".into())],
                ..draft("x")
            },
            ProfileField::Headers,
        ),
    ];
    for (bad, field) in cases {
        let error = setup
            .settings
            .save(&bad, KeyChange::Remove)
            .expect_err(&format!("{bad:?} was saved"));
        let message = error.to_string();
        assert!(
            !message.contains(CANARY),
            "the message quotes the key: {message}"
        );
        assert_eq!(field_of(error), field, "{bad:?}");
    }
    assert_eq!(setup.settings.profiles().unwrap().len(), 1);
}

#[test]
fn a_rename_keeps_the_key_and_the_default_and_removing_the_key_deletes_it() {
    let setup = setup(true);
    let key_ref = setup
        .settings
        .save(&draft("Example"), canary())
        .unwrap()
        .profile
        .key_ref
        .unwrap();

    let renamed = setup
        .settings
        .save(
            &ProfileDraft {
                name: "Renamed".into(),
                ..editing("Example")
            },
            KeyChange::Keep,
        )
        .unwrap();
    assert!(renamed.is_default);
    assert_eq!(renamed.profile.key_ref.as_deref(), Some(key_ref.as_str()));
    assert!(matches!(
        setup.settings.profile("Example"),
        Err(AiSettingsError::NotFound(_))
    ));

    let keyless = setup
        .settings
        .save(&editing("Renamed"), KeyChange::Remove)
        .unwrap();
    assert_eq!(keyless.profile.key_ref, None);
    assert!(setup.secrets.get(&key_ref).unwrap().is_none());
}

#[test]
fn deleting_a_profile_deletes_its_key_and_moves_the_default() {
    let setup = setup(true);
    let key_ref = setup
        .settings
        .save(&draft("First"), canary())
        .unwrap()
        .profile
        .key_ref
        .unwrap();
    setup
        .settings
        .save(&draft("Second"), KeyChange::Remove)
        .unwrap();
    setup.settings.delete("first").unwrap();
    assert!(setup.secrets.get(&key_ref).unwrap().is_none());
    let default = setup.settings.default_profile().unwrap().unwrap();
    assert_eq!(default.profile.name, "Second");
    assert!(matches!(
        setup.settings.delete("First"),
        Err(AiSettingsError::NotFound(_))
    ));
}

#[tokio::test]
async fn test_connection_works_before_save_and_uses_the_saved_key_after() {
    let setup = setup(true);
    let report = setup
        .settings
        .test_connection(&draft("Example"), canary())
        .await
        .unwrap();
    assert_eq!(report.reply, "ok");
    assert_eq!(report.model, "model-a");
    assert!(setup.settings.profiles().unwrap().is_empty());

    setup.settings.save(&draft("Example"), canary()).unwrap();
    setup
        .settings
        .test_connection(&editing("Example"), KeyChange::Keep)
        .await
        .unwrap();

    let error = setup
        .settings
        .test_connection(&editing("Example"), KeyChange::Remove)
        .await
        .unwrap_err();
    assert!(
        matches!(
            &error,
            AiSettingsError::Ai(AiError::Status { status: 401, message, .. })
                if message == "Invalid API key"
        ),
        "{error:?}"
    );

    let models = setup
        .settings
        .list_models(&editing("Example"), KeyChange::Keep)
        .await
        .unwrap();
    assert_eq!(models, ["model-a", "model-b"]);
}

#[tokio::test]
async fn with_ai_off_nothing_is_sent() {
    let setup = setup(false);
    setup.settings.save(&draft("Example"), canary()).unwrap();
    for error in [
        setup
            .settings
            .test_connection(&editing("Example"), KeyChange::Keep)
            .await
            .unwrap_err(),
        setup.settings.probe("Example").await.unwrap_err(),
        setup
            .settings
            .list_models(&editing("Example"), KeyChange::Keep)
            .await
            .unwrap_err(),
        setup.settings.detect_local_servers().await.unwrap_err(),
    ] {
        assert!(
            matches!(error, AiSettingsError::Ai(AiError::Refused(Refusal::Off))),
            "{error:?}"
        );
    }
    assert!(setup.endpoint.sent().is_empty());
}

#[tokio::test]
async fn local_only_refuses_a_remote_profile_as_soon_as_it_is_switched_on() {
    let setup = setup(true);
    setup.settings.save(&draft("Example"), canary()).unwrap();
    setup
        .settings
        .set_switches(AiSwitches {
            enabled: true,
            local_only: true,
        })
        .unwrap();
    let error = setup.settings.probe("Example").await.unwrap_err();
    assert!(
        matches!(
            error,
            AiSettingsError::Ai(AiError::Refused(Refusal::LocalOnly { .. }))
        ),
        "{error:?}"
    );
    assert!(setup.endpoint.sent().is_empty());
}

#[tokio::test]
async fn a_probe_result_is_kept_until_something_it_depended_on_changes() {
    let setup = setup(true);
    setup.settings.save(&draft("Example"), canary()).unwrap();
    let found = setup.settings.probe("Example").await.unwrap();
    assert_eq!(found.models_route, Check::Yes);
    assert_eq!(found.streaming, Check::Yes);
    assert_eq!(found.system_prompt, Check::Yes);
    assert_eq!(found.json_output, Check::Yes);
    assert!(matches!(found.embeddings, Check::NotChecked(_)));

    let saved = setup.settings.profile("Example").unwrap();
    assert_eq!(
        saved.capabilities,
        Some((found.clone(), "2026-09-24T10:00:00Z".into()))
    );

    // Saving the same endpoint again, or renaming it, keeps the result.
    let renamed = setup
        .settings
        .save(
            &ProfileDraft {
                name: "Renamed".into(),
                ..editing("Example")
            },
            KeyChange::Keep,
        )
        .unwrap();
    assert!(renamed.capabilities.is_some());

    // Another model, or another key, may not do what this one did.
    let other_model = setup
        .settings
        .save(
            &ProfileDraft {
                default_model: "model-b".into(),
                ..editing("Renamed")
            },
            KeyChange::Keep,
        )
        .unwrap();
    assert_eq!(other_model.capabilities, None);
    setup.settings.probe("Renamed").await.unwrap();
    let other_key = setup
        .settings
        .save(
            &ProfileDraft {
                default_model: "model-b".into(),
                ..editing("Renamed")
            },
            canary(),
        )
        .unwrap();
    assert_eq!(other_key.capabilities, None);

    // Probe requests carry the probe's own feature in their metering.
    let usage = setup
        .settings
        .gateway()
        .meter()
        .usage(aralo_ai::Feature::Setup, "Renamed");
    assert!(usage.input_tokens > 0);
}

#[tokio::test]
async fn a_probe_that_fails_to_chat_says_so_per_capability() {
    let folder = aralo_testkit::tempdir().unwrap();
    let secrets = Arc::new(MemorySecretStore::new());
    let endpoint = FakeEndpoint::new(Some("sk-some-other-key-0123456789abcdef"));
    let settings = AiSettings::with_transport(
        folder.path().join("profiles.toml"),
        secrets,
        endpoint.clone(),
    )
    .unwrap();
    settings
        .set_switches(AiSwitches {
            enabled: true,
            local_only: false,
        })
        .unwrap();
    settings.save(&draft("Example"), canary()).unwrap();
    let found = settings.probe("Example").await.unwrap();
    assert!(
        matches!(&found.streaming, Check::No(reason) if reason.contains("Invalid API key")),
        "{found:?}"
    );
    assert!(matches!(found.system_prompt, Check::NotChecked(_)));
    assert!(matches!(found.models_route, Check::No(_)));
}

#[test]
fn a_file_that_does_not_parse_is_reported_and_left_alone() {
    let folder = aralo_testkit::tempdir().unwrap();
    let path = folder.path().join("profiles.toml");
    std::fs::write(&path, format!("enabled = true\napi_key = \"{CANARY}\"\n")).unwrap();
    let error = AiSettings::with_transport(
        &path,
        Arc::new(MemorySecretStore::new()),
        FakeEndpoint::new(None),
    )
    .unwrap_err();
    let message = error.to_string();
    assert!(message.contains("line 2"), "{message}");
    assert!(!message.contains(CANARY), "{message}");
    assert!(std::fs::read_to_string(&path).unwrap().contains(CANARY));
    // The file stands for one a person typed a key into by hand. It goes once
    // the test is done, so the key-leak scan, which keeps test folders, finds
    // no canary that Aralo did not write.
    std::fs::remove_file(&path).unwrap();
}

/// The key-leak scan (PRD P13).
#[tokio::test]
async fn after_every_operation_the_key_is_only_in_the_store_and_the_auth_header() {
    let setup = setup(true);
    let mut printed = Vec::new();

    printed.push(format!(
        "{:?}",
        setup.settings.save(&draft("Example"), canary())
    ));
    printed.push(format!(
        "{:?}",
        setup
            .settings
            .test_connection(&editing("Example"), KeyChange::Keep)
            .await
    ));
    printed.push(format!(
        "{:?}",
        setup
            .settings
            .test_connection(&draft("Unsaved"), canary())
            .await
    ));
    printed.push(format!(
        "{:?}",
        setup
            .settings
            .list_models(&editing("Example"), KeyChange::Keep)
            .await
    ));
    printed.push(format!("{:?}", setup.settings.probe("Example").await));
    printed.push(format!("{:?}", setup.settings.profiles()));
    printed.push(format!("{:?}", setup.settings));
    printed.push(format!("{:?}", canary()));
    // Failures print too: a wrong key, a refused header, a missing profile.
    printed.push(format!(
        "{:?}",
        setup
            .settings
            .test_connection(
                &editing("Example"),
                KeyChange::Set(Secret::new(format!("{CANARY}-wrong")))
            )
            .await
    ));
    printed.push(format!(
        "{:?}",
        setup.settings.save(
            &ProfileDraft {
                headers: vec![("X-Key".into(), CANARY.into())],
                ..draft("Leaky")
            },
            KeyChange::Keep
        )
    ));
    printed.push(format!("{:?}", setup.settings.probe("Missing").await));
    setup.settings.delete("Example").unwrap();

    for text in &printed {
        assert!(!text.contains(CANARY), "printed: {text}");
    }
    let leaks = files_holding(setup.folder.path(), CANARY);
    assert!(leaks.is_empty(), "the key was written to {leaks:?}");

    // On the wire it is in the Authorization header and nowhere else.
    let sent = setup.endpoint.sent();
    assert!(!sent.is_empty());
    for request in &sent {
        assert!(!request.url.contains(CANARY), "{}", request.url);
        let body = String::from_utf8_lossy(request.body.as_deref().unwrap_or_default());
        assert!(!body.contains(CANARY), "{body}");
        for (name, value) in &request.headers {
            if value.contains(CANARY) {
                assert!(name.eq_ignore_ascii_case("authorization"), "{name}");
            }
        }
    }
    // The canary itself is what the scan's shape test would catch.
    assert!(looks_like_key(CANARY));
}

fn files_holding(folder: &Path, needle: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut pending = vec![folder.to_path_buf()];
    while let Some(path) = pending.pop() {
        if path.is_dir() {
            for entry in std::fs::read_dir(&path).unwrap() {
                pending.push(entry.unwrap().path());
            }
        } else if String::from_utf8_lossy(&std::fs::read(&path).unwrap()).contains(needle) {
            found.push(path.display().to_string());
        }
    }
    found
}

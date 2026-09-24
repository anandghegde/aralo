//! The editor's AI actions end to end (task 4.7), against a fake endpoint that
//! keeps what it was sent: only the text the action works on goes, a draft
//! sends nothing from the editor, and every refusal comes before anything is
//! sent.

use std::sync::{Arc, Mutex};

use aralo_ai::{
    AdapterKind, AiError, BoxFuture, ByteStream, ContextKind, HttpRequest, HttpResponse,
    MemorySecretStore, Refusal, Secret, Transport,
};
use aralo_core::ai::{
    placeholder_changes, AiSettings, AiSettingsError, AiSwitches, Authoring, AuthoringProblem,
    CommandProblem, KeyChange, ProfileDraft, MAX_SELECTION,
};
use aralo_core::diff::Change;

/// An OpenAI-compatible endpoint that answers every chat with `answer`, fenced
/// the way models like to fence things.
struct FakeEndpoint {
    answer: String,
    sent: Mutex<Vec<String>>,
}

impl Transport for FakeEndpoint {
    fn send(&self, request: HttpRequest) -> BoxFuture<'_, Result<HttpResponse, AiError>> {
        self.sent
            .lock()
            .unwrap()
            .push(String::from_utf8(request.body.clone().unwrap_or_default()).unwrap());
        let mut sse = String::new();
        for piece in ["```\n", &self.answer, "\n```\n"] {
            let chunk = serde_json::json!({"choices":[{"index":0,"delta":{"content":piece}}]});
            sse.push_str(&format!("data: {chunk}\n\n"));
        }
        sse.push_str(
            "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n\
             data: [DONE]\n\n",
        );
        Box::pin(async move {
            Ok(HttpResponse {
                status: 200,
                headers: vec![("content-type".into(), "text/event-stream".into())],
                body: Box::new(Body(Some(sse.into_bytes()))),
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
    _folder: tempfile::TempDir,
    endpoint: Arc<FakeEndpoint>,
    settings: AiSettings,
}

impl Setup {
    fn sent(&self) -> Vec<String> {
        self.endpoint.sent.lock().unwrap().clone()
    }
}

fn setup(answer: &str, enabled: bool, with_profile: bool) -> Setup {
    let folder = tempfile::tempdir().unwrap();
    let endpoint = Arc::new(FakeEndpoint {
        answer: answer.to_owned(),
        sent: Mutex::default(),
    });
    let settings = AiSettings::with_transport(
        folder.path().join("profiles.toml"),
        Arc::new(MemorySecretStore::new()),
        endpoint.clone(),
    )
    .unwrap();
    settings
        .set_switches(AiSwitches {
            enabled,
            local_only: false,
        })
        .unwrap();
    if with_profile {
        let draft = ProfileDraft {
            original_name: None,
            name: "Example".into(),
            adapter: AdapterKind::OpenAiCompat,
            base_url: "https://api.example.com/v1".into(),
            default_model: "model-a".into(),
            headers: vec![],
        };
        settings
            .save(&draft, KeyChange::Set(Secret::new("sk-test")))
            .unwrap();
    }
    Setup {
        _folder: folder,
        endpoint,
        settings,
    }
}

async fn finished(run: &mut aralo_core::ai::AuthoringRun) {
    while let Some(piece) = run.next().await {
        piece.unwrap();
    }
}

#[tokio::test]
async fn an_action_sends_the_text_it_works_on_and_nothing_else() {
    let setup = setup("Hi {{field: name}}, they're going home.", true, true);
    let text = "  Hi {{field: name}}, their going home.\n";
    let mut run = setup
        .settings
        .run_authoring(&Authoring::Proofread, text)
        .await
        .unwrap();
    assert_eq!((run.profile(), run.model()), ("Example", "model-a"));
    let manifest: Vec<_> = run
        .manifest()
        .iter()
        .map(|entry| (entry.kind, entry.bytes))
        .collect();
    assert_eq!(manifest, [(ContextKind::Selection, Some(text.len()))]);
    finished(&mut run).await;

    // The fence goes, and the text's own edges come back.
    assert_eq!(run.answer(), "  Hi {{field: name}}, they're going home.\n");
    assert_eq!(run.original(), text);
    assert_eq!(placeholder_changes(run.original(), &run.answer()), []);

    let sent = setup.sent();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].contains("their going home."), "{}", sent[0]);
    assert!(sent[0].contains("kind=\\\"selection\\\""), "{}", sent[0]);
    assert!(sent[0].contains("Correct the spelling"), "{}", sent[0]);
}

#[tokio::test]
async fn a_draft_sends_its_label_and_note_and_nothing_from_the_editor() {
    let setup = setup(
        "Thanks for asking, {{field: who}}. The refund is on its way.\n{{cursor}}",
        true,
        true,
    );
    let action = Authoring::Draft {
        label: "Refund reply".into(),
        note: "Short and warm.".into(),
    };
    let mut run = setup
        .settings
        .run_authoring(&action, "EDITOR-TEXT")
        .await
        .unwrap();
    assert!(run.manifest().is_empty());
    finished(&mut run).await;
    assert_eq!(
        run.answer(),
        "Thanks for asking, {{field: who}}. The refund is on its way.\n{{cursor}}"
    );
    let sent = setup.sent().join("\n");
    assert!(
        sent.contains("Refund reply") && sent.contains("Short and warm."),
        "{sent}"
    );
    assert!(!sent.contains("EDITOR-TEXT"), "{sent}");
    assert!(
        !sent.contains("aralo-data-"),
        "no context, no data blocks: {sent}"
    );
}

#[tokio::test]
async fn variations_come_back_one_by_one() {
    let setup = setup("Thanks!\n%%%\nThank you so much!\n%%%\nCheers!", true, true);
    let mut run = setup
        .settings
        .run_authoring(&Authoring::Variations, "Thanks")
        .await
        .unwrap();
    finished(&mut run).await;
    assert_eq!(
        run.variations(),
        ["Thanks!", "Thank you so much!", "Cheers!"]
    );
    assert_eq!(run.answer(), "Thanks!");
}

#[tokio::test]
async fn an_answer_that_drops_or_adds_a_placeholder_says_so() {
    let setup = setup("Hello {{clipboard}}, see you.", true, true);
    let text = "Hello {{field: name}}, see you.";
    let mut run = setup
        .settings
        .run_authoring(&Authoring::Friendlier, text)
        .await
        .unwrap();
    finished(&mut run).await;
    let changes: Vec<_> = placeholder_changes(run.original(), &run.answer())
        .into_iter()
        .map(|change| (change.change, change.placeholder))
        .collect();
    assert_eq!(
        changes,
        [
            (Change::Removed, "{{field: name}}".to_owned()),
            (Change::Added, "{{clipboard}}".to_owned()),
        ]
    );
}

#[tokio::test]
async fn what_stops_an_action_stops_it_before_anything_is_sent() {
    let on = setup("unused", true, true);
    for (action, text, problem) in [
        (
            Authoring::Proofread,
            " \n",
            AuthoringProblem::NothingToWorkOn,
        ),
        (
            Authoring::Draft {
                label: " ".into(),
                note: String::new(),
            },
            "",
            AuthoringProblem::NothingToDraft,
        ),
        (
            Authoring::Translate {
                language: "  ".into(),
            },
            "Hello",
            AuthoringProblem::NoLanguage,
        ),
    ] {
        let error = on.settings.run_authoring(&action, text).await.unwrap_err();
        assert!(
            matches!(&error, AiSettingsError::Authoring(found) if *found == problem),
            "{action:?}: {error:?}"
        );
    }
    let long = "a".repeat(MAX_SELECTION + 1);
    let error = on
        .settings
        .run_authoring(&Authoring::Shorter, &long)
        .await
        .unwrap_err();
    assert!(
        matches!(
            error,
            AiSettingsError::Authoring(AuthoringProblem::TooLong { .. })
        ),
        "{error:?}"
    );
    assert!(on.sent().is_empty());

    let off = setup("unused", false, true);
    let error = off
        .settings
        .run_authoring(&Authoring::Proofread, "text")
        .await
        .unwrap_err();
    assert!(
        matches!(error, AiSettingsError::Ai(AiError::Refused(Refusal::Off))),
        "{error:?}"
    );
    assert!(off.sent().is_empty());

    let empty = setup("unused", true, false);
    let error = empty
        .settings
        .run_authoring(&Authoring::Proofread, "text")
        .await
        .unwrap_err();
    assert!(
        matches!(error, AiSettingsError::Command(CommandProblem::NoProfile)),
        "{error:?}"
    );
}

//! Commands on selected text end to end (task 4.5): the built-in and library
//! commands the palette lists, and a run against a fake endpoint that checks
//! the selection is the only context sent.

use std::sync::{Arc, Mutex};

use aralo_ai::{
    AdapterKind, AiError, BoxFuture, ByteStream, ContextKind, HttpRequest, HttpResponse,
    MemorySecretStore, Refusal, Secret, Transport,
};
use aralo_core::ai::{
    builtin_commands, AiSettings, AiSettingsError, AiSwitches, Command, CommandProblem, KeyChange,
    ProfileDraft, MAX_SELECTION,
};
use aralo_core::diff::Change;
use aralo_core::Core;

/// An OpenAI-compatible endpoint that answers every chat with `answer`,
/// fenced the way models like to fence things, and keeps what it was sent.
struct FakeEndpoint {
    answer: &'static str,
    sent: Mutex<Vec<HttpRequest>>,
}

impl Transport for FakeEndpoint {
    fn send(&self, request: HttpRequest) -> BoxFuture<'_, Result<HttpResponse, AiError>> {
        self.sent.lock().unwrap().push(request);
        let mut sse = String::new();
        for piece in ["```\n", self.answer, "\n```\n"] {
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
    _folder: aralo_testkit::TempDir,
    endpoint: Arc<FakeEndpoint>,
    settings: AiSettings,
}

fn setup(enabled: bool, with_profile: bool) -> Setup {
    let folder = aralo_testkit::tempdir().unwrap();
    let endpoint = Arc::new(FakeEndpoint {
        answer: "They're going home.",
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

fn proofread() -> Command {
    builtin_commands()
        .into_iter()
        .find(|command| command.label == "Fix spelling and grammar")
        .unwrap()
}

#[tokio::test]
async fn a_command_sends_the_selection_and_nothing_else_and_answers_with_a_diff() {
    let setup = setup(true, true);
    let selection = "  their going home.\n";
    let mut run = setup
        .settings
        .run_command(&proofread(), selection)
        .await
        .unwrap();
    assert_eq!(run.profile(), "Example");
    assert_eq!(run.model(), "model-a");
    assert_eq!(run.manifest().len(), 1);
    assert_eq!(run.manifest()[0].kind, ContextKind::Selection);
    assert_eq!(run.manifest()[0].bytes, Some(selection.len()));

    let mut pieces = 0;
    while let Some(piece) = run.next().await {
        piece.unwrap();
        pieces += 1;
    }
    assert_eq!(pieces, 3);
    assert!(run.stop_reason().is_some());

    // The fence goes, and the selection's own edges come back.
    assert_eq!(run.replacement(), "  They're going home.\n");
    let diff = run.diff();
    assert!(diff
        .iter()
        .any(|span| span.change == Change::Removed && span.text.contains("their")));
    assert!(diff
        .iter()
        .any(|span| span.change == Change::Added && span.text.contains("They're")));

    let sent = setup.endpoint.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    let body = String::from_utf8(sent[0].body.clone().unwrap()).unwrap();
    assert!(body.contains("their going home."), "{body}");
    assert!(body.contains(&proofread().instruction[..20]), "{body}");
}

#[tokio::test]
async fn a_command_can_name_its_model() {
    let setup = setup(true, true);
    let command = Command {
        model: Some("model-b".into()),
        ..proofread()
    };
    let run = setup
        .settings
        .run_command(&command, "some text")
        .await
        .unwrap();
    assert_eq!(run.model(), "model-b");
    let body =
        String::from_utf8(setup.endpoint.sent.lock().unwrap()[0].body.clone().unwrap()).unwrap();
    assert!(body.contains("\"model-b\""), "{body}");
}

#[tokio::test]
async fn what_stops_a_command_stops_it_before_anything_is_sent() {
    let on = setup(true, true);
    for (selection, problem) in [
        ("", CommandProblem::NothingSelected),
        (" \n\t", CommandProblem::NothingSelected),
    ] {
        let error = on
            .settings
            .run_command(&proofread(), selection)
            .await
            .unwrap_err();
        assert!(
            matches!(&error, AiSettingsError::Command(found) if *found == problem),
            "{error:?}"
        );
    }
    let long = "a".repeat(MAX_SELECTION + 1);
    let error = on
        .settings
        .run_command(&proofread(), &long)
        .await
        .unwrap_err();
    assert!(
        matches!(
            error,
            AiSettingsError::Command(CommandProblem::TooLong { .. })
        ),
        "{error:?}"
    );

    let unknown = Command {
        profile: Some("Nobody".into()),
        ..proofread()
    };
    let error = on.settings.run_command(&unknown, "text").await.unwrap_err();
    assert!(matches!(error, AiSettingsError::NotFound(_)), "{error:?}");
    assert!(on.endpoint.sent.lock().unwrap().is_empty());

    let empty = setup(true, false);
    let error = empty
        .settings
        .run_command(&proofread(), "text")
        .await
        .unwrap_err();
    assert!(
        matches!(error, AiSettingsError::Command(CommandProblem::NoProfile)),
        "{error:?}"
    );

    let off = setup(false, true);
    let error = off
        .settings
        .run_command(&proofread(), "text")
        .await
        .unwrap_err();
    assert!(
        matches!(error, AiSettingsError::Ai(AiError::Refused(Refusal::Off))),
        "{error:?}"
    );
    assert!(off.endpoint.sent.lock().unwrap().is_empty());
}

#[test]
fn the_library_adds_commands_and_can_replace_a_built_in_one() {
    let folder = aralo_testkit::tempdir().unwrap();
    let root = folder.path().join("Aralo");
    Core::open_without_starter(&root).unwrap();
    let builtin = proofread();
    std::fs::write(
        root.join("proofread.md"),
        format!(
            "---\nid: {}\nlabel: Proofread, British\ntype: command\nai:\n  profile: Example\n---\n\
             Fix the spelling in British English.\n",
            builtin.id
        ),
    )
    .unwrap();
    std::fs::write(
        root.join("pirate.md"),
        "---\nlabel: Pirate\ntype: command\nabbr: [\";pirate\"]\n---\nSay it like a pirate.\n",
    )
    .unwrap();
    std::fs::write(
        root.join("off.md"),
        "---\nlabel: Switched off\ntype: command\nenabled: false\n---\nNothing.\n",
    )
    .unwrap();
    std::fs::write(root.join("text.md"), "---\nlabel: Plain\n---\nHello\n").unwrap();

    let core = Core::open_without_starter(&root).unwrap();
    let commands = core.commands();
    let labels: Vec<_> = commands.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(labels.len(), builtin_commands().len() + 1, "{labels:?}");
    assert_eq!(labels[0], "Proofread, British", "replaced where it stood");
    assert_eq!(*labels.last().unwrap(), "Pirate");
    assert!(!labels.contains(&"Plain") && !labels.contains(&"Switched off"));

    let replaced = &commands[0];
    assert!(!replaced.builtin);
    assert_eq!(replaced.profile.as_deref(), Some("Example"));
    assert_eq!(replaced.instruction, "Fix the spelling in British English.");

    // A command has no text to insert, and its abbreviation expands nothing.
    let pirate = commands.last().unwrap();
    assert!(core.insert(pirate.id).is_none());
    assert_eq!(core.command(pirate.id).as_ref(), Some(pirate));
}

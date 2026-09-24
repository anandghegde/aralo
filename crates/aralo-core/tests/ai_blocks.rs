//! AI blocks end to end (task 4.6): a snippet that asks a model for part of
//! its text opens a session, the block goes through the gateway with only the
//! context its snippet declared, and what comes back goes in as literal text
//! between the body's own, untouched. With AI off, or no network, the block
//! puts in its fallback and the expansion says so.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use aralo_ai::{
    AdapterKind, AiError, BoxFuture, ByteStream, ContextKind as SentKind, HttpRequest,
    HttpResponse, MemorySecretStore, Secret, Transport,
};
use aralo_core::ai::{AiSettings, AiSwitches, KeyChange, ProfileDraft};
use aralo_core::template::DiagnosticKind;
use aralo_core::{
    Answers, ContextKind, ContextValues, Core, Expand, Expansion, Session, SessionStep,
};

/// An OpenAI-compatible endpoint that answers every chat with `answer`, fenced
/// the way models like to fence things, and keeps what it was sent.
struct FakeEndpoint {
    answer: String,
    sent: Mutex<Vec<HttpRequest>>,
}

impl FakeEndpoint {
    fn bodies(&self) -> Vec<String> {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .map(|request| String::from_utf8(request.body.clone().unwrap_or_default()).unwrap())
            .collect()
    }
}

impl Transport for FakeEndpoint {
    fn send(&self, request: HttpRequest) -> BoxFuture<'_, Result<HttpResponse, AiError>> {
        self.sent.lock().unwrap().push(request);
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
    folder: tempfile::TempDir,
    endpoint: Arc<FakeEndpoint>,
    settings: AiSettings,
}

impl Setup {
    /// A library holding one snippet, `file`, as the text of its file.
    fn core(&self, file: &str) -> Core {
        let root = self.folder.path().join("Aralo");
        Core::open_without_starter(&root).unwrap();
        std::fs::write(root.join("case.md"), file).unwrap();
        Core::open_without_starter(&root).unwrap()
    }
}

fn setup(answer: &str) -> Setup {
    setup_with(answer, true, "https://api.example.com/v1")
}

fn setup_with(answer: &str, enabled: bool, base_url: &str) -> Setup {
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
    let draft = ProfileDraft {
        original_name: None,
        name: "Example".into(),
        adapter: AdapterKind::OpenAiCompat,
        base_url: base_url.into(),
        default_model: "model-a".into(),
        headers: vec![],
    };
    settings
        .save(&draft, KeyChange::Set(Secret::new("sk-test")))
        .unwrap();
    Setup {
        folder,
        endpoint,
        settings,
    }
}

fn session(core: &Core) -> Session {
    let id = core.snippets()[0].id;
    match core.insert(id).expect("the snippet is there") {
        Expand::Session(session) => *session,
        Expand::Ready(expansion) => panic!("expected a session, got {expansion:?}"),
    }
}

fn ready(step: SessionStep) -> Expansion {
    match step {
        SessionStep::Ready(expansion) => expansion,
        other => panic!("expected the plan, got {other:?}"),
    }
}

const REPLY: &str = "---\nlabel: Reply\nabbr: [\";reply\"]\nai:\n  context: [fillins]\n---\n\
Hi {{field: who}},\n\n{{ai: Thank them for their order | fallback: Thanks for your order.}}\n\nOrder {{field: order}}. Best, Sam\n";

#[tokio::test]
async fn a_block_opens_a_session_and_its_answer_goes_in_between_the_bodys_own_text() {
    let setup = setup("Thank you so much, it's on its way!");
    let core = setup.core(REPLY);
    let mut session = session(&core);

    // The form first, then the block: nothing is asked of the shell, since
    // the only declared kind is the form's answers.
    assert!(matches!(session.step(), SessionStep::Form(_)));
    let answers = Answers::from([
        ("who".to_owned(), "Dana".to_owned()),
        ("order".to_owned(), "PO-8841".to_owned()),
    ]);
    let SessionStep::Ai(waiting) = session.submit_form(answers) else {
        panic!("the block is next");
    };
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].prompt, "Thank them for their order");

    let request = session.block_request(waiting[0].index).unwrap();
    let mut run = setup.settings.run_block(&request).await.unwrap();
    assert_eq!(run.profile(), "Example");
    assert_eq!(run.model(), "model-a");
    let manifest: Vec<_> = run.manifest().iter().map(|entry| entry.kind).collect();
    assert_eq!(manifest, [SentKind::Fillins]);
    while let Some(piece) = run.next().await {
        piece.unwrap();
    }
    // The fence goes; the words are the model's.
    let answer = run.answer().unwrap();
    assert_eq!(answer, "Thank you so much, it's on its way!");

    let expansion = ready(session.answer_block(waiting[0].index, answer));
    assert_eq!(
        expansion.plan.inserted_text(),
        "Hi Dana,\n\nThank you so much, it's on its way!\n\nOrder PO-8841. Best, Sam"
    );
    assert!(expansion.template_diagnostics.is_empty());

    // What was sent: the prompt and the declared answers, framed as data. Not
    // the body's own text around the block.
    let bodies = setup.endpoint.bodies();
    assert_eq!(bodies.len(), 1);
    assert!(
        bodies[0].contains("Thank them for their order"),
        "{}",
        bodies[0]
    );
    assert!(
        bodies[0].contains("who: Dana\\norder: PO-8841"),
        "{}",
        bodies[0]
    );
    assert!(bodies[0].contains("kind=\\\"fillins\\\""), "{}", bodies[0]);
    assert!(!bodies[0].contains("Best, Sam"), "{}", bodies[0]);
}

#[tokio::test]
async fn only_declared_context_is_asked_for_and_only_it_is_sent() {
    let setup = setup("A reply.");
    let core = setup.core(
        "---\nai:\n  context: [clipboard, app, screen]\n---\n\
         {{field: who}} {{ai: Reply to what I copied}} {{selection}}\n",
    );
    let mut session = session(&core);
    assert!(matches!(session.step(), SessionStep::Form(_)));

    // The shell is asked for what was declared, and for nothing else: not
    // the selection the body mentions, not the form's answers, which were
    // not declared, and not a name Aralo does not know.
    let SessionStep::Context(kinds) = session.submit_form(Answers::from([(
        "who".to_owned(),
        "PRIVATE-ANSWER".to_owned(),
    )])) else {
        panic!("the declared context is next");
    };
    assert_eq!(kinds, [ContextKind::Clipboard, ContextKind::App]);

    // A shell that hands over more than it was asked for is not trusted.
    let SessionStep::Ai(_) = session.provide_context(ContextValues {
        clipboard: Some("COPIED-TEXT".into()),
        selection: Some("SELECTED-TEXT".into()),
        app: Some("Mail".into()),
        window: Some("WINDOW-TITLE".into()),
    }) else {
        panic!("the block is next");
    };
    let expansion = ready(setup.settings.answer_blocks(&mut session).await);

    let sent = setup.endpoint.bodies().join("\n");
    assert!(
        sent.contains("COPIED-TEXT") && sent.contains("Mail"),
        "{sent}"
    );
    for never in ["SELECTED-TEXT", "WINDOW-TITLE", "PRIVATE-ANSWER"] {
        assert!(!sent.contains(never), "{never} was sent: {sent}");
    }
    // A kind declared for the model is not written into the text by a
    // placeholder that does not expand it yet.
    assert_eq!(
        expansion.plan.inserted_text(),
        "PRIVATE-ANSWER A reply. {{selection}}"
    );
}

#[tokio::test]
async fn a_snippet_that_declares_nothing_sends_the_prompt_alone() {
    let setup = setup("Hello.");
    let core = setup.core("---\nlabel: Hi\n---\n{{clipboard}} {{ai: Say hello}}\n");
    let mut session = session(&core);
    // The clipboard is the body's, and goes into the text; it is not the
    // model's.
    let SessionStep::Context(kinds) = session.step() else {
        panic!("the clipboard first");
    };
    assert_eq!(kinds, [ContextKind::Clipboard]);
    session.provide_context(ContextValues {
        clipboard: Some("COPIED-TEXT".into()),
        ..ContextValues::default()
    });
    let expansion = ready(setup.settings.answer_blocks(&mut session).await);
    assert_eq!(expansion.plan.inserted_text(), "COPIED-TEXT Hello.");
    let sent = setup.endpoint.bodies().join("\n");
    assert!(!sent.contains("COPIED-TEXT"), "{sent}");
    assert!(
        !sent.contains("aralo-data-"),
        "no context, no data blocks: {sent}"
    );
}

#[tokio::test]
async fn with_ai_off_a_block_puts_in_its_fallback_says_so_and_sends_nothing() {
    let setup = setup_with("never sent", false, "https://api.example.com/v1");
    let core = setup.core(REPLY);
    let mut session = session(&core);
    session.submit_form(Answers::from([("who".to_owned(), "Dana".to_owned())]));
    let expansion = ready(setup.settings.answer_blocks(&mut session).await);

    assert_eq!(
        expansion.plan.inserted_text(),
        "Hi Dana,\n\nThanks for your order.\n\nOrder . Best, Sam"
    );
    let notes: Vec<_> = expansion
        .template_diagnostics
        .iter()
        .map(|diagnostic| (diagnostic.kind, diagnostic.message()))
        .collect();
    assert_eq!(
        notes,
        [(
            DiagnosticKind::AiFallback,
            "No model answered this block (AI is switched off), so its fallback went in."
                .to_owned()
        )]
    );
    assert!(setup.endpoint.sent.lock().unwrap().is_empty());
}

#[tokio::test]
async fn in_local_only_mode_a_remote_profile_falls_back_before_anything_is_sent() {
    let setup = setup("never sent");
    setup
        .settings
        .set_switches(AiSwitches {
            enabled: true,
            local_only: true,
        })
        .unwrap();
    let core = setup.core("---\n---\n[{{ai: Anything | fallback: offline}}]\n");
    let mut session = session(&core);
    let expansion = ready(setup.settings.answer_blocks(&mut session).await);
    assert_eq!(expansion.plan.inserted_text(), "[offline]");
    let message = expansion.template_diagnostics[0].message();
    assert!(message.contains("local-only mode is on"), "{message}");
    assert!(setup.endpoint.sent.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_profile_that_is_not_there_falls_back_and_names_it() {
    let setup = setup("never sent");
    let core = setup.core("---\nai:\n  profile: Nobody\n---\n{{ai: Anything | fallback: -}}\n");
    let mut session = session(&core);
    let expansion = ready(setup.settings.answer_blocks(&mut session).await);
    assert_eq!(expansion.plan.inserted_text(), "-");
    let message = expansion.template_diagnostics[0].message();
    assert!(message.contains("Nobody"), "{message}");
    assert!(setup.endpoint.sent.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_blocks_model_wins_over_the_snippets_which_wins_over_the_profiles() {
    let setup = setup("ok");
    let core =
        setup.core("---\nai:\n  model: model-b\n---\n{{ai: one}} {{ai: two | model: model-c}}\n");
    let mut session = session(&core);
    let requests: Vec<_> = session
        .ai_blocks()
        .iter()
        .map(|block| session.block_request(block.index).unwrap())
        .collect();
    assert_eq!(requests[0].model.as_deref(), Some("model-b"));
    assert_eq!(requests[1].model.as_deref(), Some("model-c"));
    let expansion = ready(setup.settings.answer_blocks(&mut session).await);
    assert_eq!(expansion.plan.inserted_text(), "ok ok");
    let bodies = setup.endpoint.bodies();
    assert!(bodies[0].contains("\"model-b\""), "{}", bodies[0]);
    assert!(bodies[1].contains("\"model-c\""), "{}", bodies[1]);
}

#[tokio::test]
async fn a_model_that_writes_placeholders_gets_them_in_as_text_and_nothing_is_read() {
    let setup = setup("{{clipboard}} {{snippet: Reply}} {{cursor}}");
    let core = setup.core("---\n---\nSays: {{ai: Anything}}\n");
    let mut session = session(&core);
    let expansion = ready(setup.settings.answer_blocks(&mut session).await);
    assert_eq!(
        expansion.plan.inserted_text(),
        "Says: {{clipboard}} {{snippet: Reply}} {{cursor}}"
    );
    // Nothing more was asked of anyone: the session never wanted context.
    assert!(session.needs().is_empty());
}

#[tokio::test]
async fn an_empty_answer_falls_back() {
    let setup = setup("   ");
    let core = setup.core("---\n---\n{{ai: Anything | fallback: Nothing came.}}\n");
    let mut session = session(&core);
    let expansion = ready(setup.settings.answer_blocks(&mut session).await);
    assert_eq!(expansion.plan.inserted_text(), "Nothing came.");
    assert!(expansion.template_diagnostics[0]
        .message()
        .contains("the model's answer was empty"));
}

#[test]
fn the_preview_marks_each_blocks_text_while_it_is_written() {
    let setup = setup("unused");
    let core =
        setup.core("---\n---\nA {{ai: one | fallback: F1}} B {{ai: two | fallback: F2}} C\n");
    let mut session = session(&core);
    let drafts = BTreeMap::from([(0, "first draft".to_owned())]);
    let rendered = session.preview_blocks(&Answers::new(), &drafts);
    let text = rendered.text();
    assert_eq!(text, "A first draft B F2 C");
    let marked: Vec<_> = rendered
        .ai_spans()
        .iter()
        .map(|span| (span.block, &text[span.range.clone()]))
        .collect();
    assert_eq!(marked, [(0, "first draft"), (1, "F2")]);

    // A settled block keeps its answer under a draft of the other, and can be
    // settled again until the session finishes.
    session.answer_block(1, "second".into());
    session.answer_block(1, "second, again".into());
    assert_eq!(
        session.preview_blocks(&Answers::new(), &drafts).text(),
        "A first draft B second, again C"
    );
    // A number that is not a block is ignored.
    assert!(matches!(
        session.answer_block(7, "nowhere".into()),
        SessionStep::Ai(waiting) if waiting.len() == 1
    ));
    let expansion = ready(session.fall_back(0, "the user chose the fallback".into()));
    assert_eq!(expansion.plan.inserted_text(), "A F1 B second, again C");
}

#[test]
fn a_session_with_a_block_can_be_cancelled_before_anything_goes_in() {
    let setup = setup("unused");
    let core = setup.core("---\nabbr: [\";ty\"]\n---\n{{ai: Say thanks | fallback: Thanks}}\n");
    let mut field = aralo_core::Simulator::new(&core, "com.apple.TextEdit").cancelling();
    field.type_str(";ty ");
    assert_eq!(field.text(), ";ty ");
    assert_eq!(field.cancellations(), 1);
    assert_eq!(field.expansions(), 0);
}

#[test]
fn a_draft_in_the_editor_runs_its_blocks_under_the_saved_files_declarations() {
    let setup = setup("unused");
    let core = setup.core("---\nabbr: [\";ty\"]\nai:\n  context: [clipboard]\n---\nold body\n");
    let id = core.snippets()[0].id;
    let mut draft = aralo_core::Draft::of(&core.snippets()[0]);
    draft.body = "{{ai: Say thanks | fallback: Thanks}}".into();
    let mut trial = core.try_draft(&draft, &[], Some(id));
    let mut session = trial
        .type_str(&core, ";ty ")
        .expect("the draft asks a model");
    let SessionStep::Context(kinds) = session.step() else {
        panic!("the declared clipboard first");
    };
    assert_eq!(kinds, [ContextKind::Clipboard]);
}

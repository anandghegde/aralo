//! The AI settings as the Swift pane calls them, with keys kept in memory.

use aralo_ffi::{
    ai_provider_presets, AiBridgeError, AiField, AiHeader, AiKeyChange, AiProfileDraft, AiProfiles,
    AiSwitches, KeyStorage,
};

fn open() -> (tempfile::TempDir, std::sync::Arc<AiProfiles>) {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("profiles.toml");
    let profiles = AiProfiles::open(Some(path.display().to_string()), KeyStorage::Memory).unwrap();
    (folder, profiles)
}

fn draft(name: &str, base_url: &str) -> AiProfileDraft {
    AiProfileDraft {
        original_name: None,
        name: name.into(),
        base_url: base_url.into(),
        default_model: "model-a".into(),
        headers: vec![AiHeader {
            name: "X-Title".into(),
            value: "Aralo".into(),
        }],
    }
}

#[test]
fn a_saved_profile_reports_its_key_without_returning_it() {
    let (folder, profiles) = open();
    assert!(profiles.profiles().unwrap().is_empty());
    assert_eq!(
        profiles.switches().unwrap(),
        AiSwitches {
            enabled: false,
            local_only: false
        }
    );

    let saved = profiles
        .save(
            draft("Remote", "https://api.example.com/v1"),
            AiKeyChange::Set {
                key: "sk-aralo-canary-ffi-0123456789abcdef".into(),
            },
        )
        .unwrap();
    assert!(saved.has_key && saved.is_default && !saved.is_local);
    assert_eq!(saved.capabilities, None);
    let local = profiles
        .save(
            draft("Ollama", "http://127.0.0.1:11434/v1"),
            AiKeyChange::Remove,
        )
        .unwrap();
    assert!(!local.has_key && !local.is_default && local.is_local);

    profiles.set_default("Ollama".into()).unwrap();
    let names: Vec<_> = profiles
        .profiles()
        .unwrap()
        .into_iter()
        .map(|profile| (profile.name, profile.is_default))
        .collect();
    assert_eq!(
        names,
        [("Remote".to_owned(), false), ("Ollama".to_owned(), true)]
    );

    let file = std::fs::read_to_string(folder.path().join("profiles.toml")).unwrap();
    assert!(!file.contains("canary"), "{file}");
    assert!(!format!("{profiles:?} {:?}", profiles.profiles()).contains("canary"));

    profiles.delete("Remote".into()).unwrap();
    assert!(matches!(
        profiles.delete("Remote".into()),
        Err(AiBridgeError::NotFound { .. })
    ));
}

#[test]
fn a_problem_names_the_field_to_fix() {
    let (_folder, profiles) = open();
    let mut bad = draft("Remote", "https://api.example.com/v1");
    bad.headers = vec![AiHeader {
        name: "Authorization".into(),
        value: "Bearer x".into(),
    }];
    assert!(matches!(
        profiles.save(bad, AiKeyChange::Remove),
        Err(AiBridgeError::Invalid {
            field: AiField::Headers,
            ..
        })
    ));
    assert!(matches!(
        profiles.save(draft("Remote", "ftp://example.com"), AiKeyChange::Remove),
        Err(AiBridgeError::Invalid {
            field: AiField::BaseUrl,
            ..
        })
    ));
}

#[tokio::test]
async fn with_ai_off_every_network_call_is_refused() {
    let (_folder, profiles) = open();
    profiles
        .save(
            draft("Remote", "https://api.example.com/v1"),
            AiKeyChange::Remove,
        )
        .unwrap();
    let unsaved = draft("Other", "https://api.example.com/v1");
    let errors = [
        profiles
            .test_connection(unsaved.clone(), AiKeyChange::Remove)
            .await
            .unwrap_err(),
        profiles
            .list_models(unsaved, AiKeyChange::Remove)
            .await
            .unwrap_err(),
        profiles
            .probe_capabilities("Remote".into())
            .await
            .unwrap_err(),
        profiles.detect_local_servers().await.unwrap_err(),
    ];
    for error in errors {
        assert!(
            matches!(&error, AiBridgeError::Refused { message } if message.contains("off")),
            "{error:?}"
        );
    }

    profiles
        .set_switches(AiSwitches {
            enabled: true,
            local_only: true,
        })
        .unwrap();
    let error = profiles
        .probe_capabilities("Remote".into())
        .await
        .unwrap_err();
    assert!(
        matches!(&error, AiBridgeError::Refused { message } if message.contains("api.example.com")),
        "{error:?}"
    );
}

#[test]
fn every_remote_preset_links_to_its_key_page() {
    let presets = ai_provider_presets();
    assert!(presets.iter().any(|preset| preset.id == "openai"));
    for preset in presets {
        assert_eq!(
            preset.key_page.is_none(),
            preset.base_url.starts_with("http://127.0.0.1"),
            "{}",
            preset.id
        );
    }
}

// Commands on selected text (task 4.5), against a model server on this Mac
// that the test runs itself: it answers with the pieces it is given, then
// either ends or keeps the connection open until the other side closes it.

mod commands {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::time::Duration;

    use aralo_ffi::{
        diff_words, AiBridgeError, AiCommand, AiContextKind, AiKeyChange, AiSwitches, Core,
        DiffChange,
    };

    use super::{draft, open};

    /// Starts the server and returns its address, and a channel that gets
    /// each request body.
    pub(super) fn serve(
        pieces: &'static [&'static str],
        hold_open: bool,
    ) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}/v1", listener.local_addr().unwrap());
        let (sender, bodies) = mpsc::channel();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                if let Some((name, value)) = line.split_once(':') {
                    if name.eq_ignore_ascii_case("content-length") {
                        length = value.trim().parse().unwrap();
                    }
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            sender.send(String::from_utf8(body).unwrap()).unwrap();

            let mut stream = stream;
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\
                      connection: close\r\n\r\n",
                )
                .unwrap();
            for piece in pieces {
                let chunk = format!(
                    "data: {{\"choices\":[{{\"index\":0,\"delta\":{{\"content\":{}}}}}]}}\n\n",
                    serde_json::to_string(piece).unwrap()
                );
                stream.write_all(chunk.as_bytes()).unwrap();
                stream.flush().unwrap();
            }
            if hold_open {
                // Until the client hangs up: a read of nothing.
                let _ = reader.read(&mut [0; 1]);
                return;
            }
            stream
                .write_all(
                    b"data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n\
                      data: [DONE]\n\n",
                )
                .unwrap();
        });
        (address, bodies)
    }

    fn proofread() -> AiCommand {
        let folder = tempfile::tempdir().unwrap();
        let core = Core::open_library(
            folder.path().join("Aralo").to_string_lossy().into_owned(),
            Some(folder.path().join("cache").to_string_lossy().into_owned()),
            None,
            None,
        )
        .unwrap();
        let commands = core.commands();
        assert!(commands.len() >= 7 && commands.iter().all(|c| c.builtin));
        commands
            .into_iter()
            .find(|command| command.label == "Fix spelling and grammar")
            .unwrap()
    }

    pub(super) fn switch_on(profiles: &aralo_ffi::AiProfiles, address: &str) {
        profiles
            .save(draft("Local", address), AiKeyChange::Remove)
            .unwrap();
        profiles
            .set_switches(AiSwitches {
                enabled: true,
                local_only: true,
            })
            .unwrap();
    }

    #[tokio::test]
    async fn a_command_streams_its_answer_and_says_what_it_sent() {
        let (address, bodies) = serve(&["```\n", "They're going", " home.", "\n```"], false);
        let (_folder, profiles) = open();
        switch_on(&profiles, &address);

        let selection = "their going home.\n";
        let run = profiles
            .run_command(proofread(), selection.into())
            .await
            .unwrap();
        assert_eq!(
            (run.profile(), run.model()),
            ("Local".into(), "model-a".into())
        );
        let sent = run.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].kind, AiContextKind::Selection);
        assert_eq!(sent[0].bytes, Some(selection.len() as u64));

        let mut pieces = Vec::new();
        while let Some(piece) = run.next().await.unwrap() {
            pieces.push(piece);
        }
        assert_eq!(pieces.len(), 4);
        assert!(!run.cut_short());
        assert_eq!(run.replacement(), "They're going home.\n");
        let diff = run.diff();
        assert_eq!(diff[0].change, DiffChange::Removed);
        assert_eq!(diff[0].text, "their");
        assert_eq!(diff[1].change, DiffChange::Added);

        let body = bodies.recv().unwrap();
        assert!(body.contains("their going home."), "{body}");
        assert!(
            run.next().await.unwrap().is_none(),
            "a finished run stays finished"
        );
    }

    #[tokio::test]
    async fn cancel_stops_a_run_that_is_waiting_on_the_model() {
        let (address, _bodies) = serve(&["The first half"], true);
        let (_folder, profiles) = open();
        switch_on(&profiles, &address);

        let run = profiles
            .run_command(proofread(), "some text".into())
            .await
            .unwrap();
        assert_eq!(run.next().await.unwrap().as_deref(), Some("The first half"));

        let waiting = {
            let run = run.clone();
            tokio::spawn(async move { run.next().await })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        run.cancel();
        let after = tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .expect("cancel reached the waiting call")
            .unwrap();
        assert!(after.unwrap().is_none());
        assert_eq!(run.text(), "The first half");
        assert!(run.next().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_command_does_not_run_with_ai_off_or_nothing_selected() {
        let (_folder, profiles) = open();
        profiles
            .save(
                draft("Remote", "https://api.example.com/v1"),
                AiKeyChange::Remove,
            )
            .unwrap();
        let error = profiles
            .run_command(proofread(), "text".into())
            .await
            .unwrap_err();
        assert!(matches!(error, AiBridgeError::Refused { .. }), "{error:?}");

        profiles
            .set_switches(AiSwitches {
                enabled: true,
                local_only: false,
            })
            .unwrap();
        let error = profiles
            .run_command(proofread(), "  ".into())
            .await
            .unwrap_err();
        assert!(
            matches!(&error, AiBridgeError::Failed { message } if message.contains("selected")),
            "{error:?}"
        );

        let broken = AiCommand {
            id: "not an id".into(),
            ..proofread()
        };
        let error = profiles
            .run_command(broken, "text".into())
            .await
            .unwrap_err();
        assert!(matches!(error, AiBridgeError::NotFound { .. }), "{error:?}");
    }

    #[test]
    fn an_edited_answer_diffs_the_same_way() {
        let spans = diff_words("a b c".into(), "a x c".into());
        let changes: Vec<_> = spans.iter().map(|span| span.change).collect();
        assert_eq!(
            changes,
            [
                DiffChange::Same,
                DiffChange::Removed,
                DiffChange::Added,
                DiffChange::Same
            ]
        );
    }
}

// AI blocks in snippets (task 4.6): a session on its AI step, answered by the
// same model server on this Mac.

mod blocks {
    use std::sync::Arc;

    use aralo_ffi::{
        AiBridgeError, AiContextKind, AiKeyChange, BlockSpan, ContextNeed, ContextSupply, Core,
        ExpansionSession, InsertOutcome, PlanStep, SessionAction,
    };

    use super::commands::{serve, switch_on};
    use super::{draft, open};

    const REPLY: &str = "---\nlabel: Reply\nai:\n  context: [clipboard]\n---\n\
        Dear {{field: who}},\n{{ai: Thank them | fallback: Thanks.}}\nSam\n";

    /// A library holding one snippet, the file `text`, and a session for it
    /// picked from the palette.
    fn session(text: &str) -> (tempfile::TempDir, Arc<Core>, Arc<ExpansionSession>) {
        let folder = tempfile::tempdir().unwrap();
        let root = folder.path().join("Aralo");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("reply.md"), text).unwrap();
        let core = Core::open_library(
            root.to_string_lossy().into_owned(),
            Some(folder.path().join("cache").to_string_lossy().into_owned()),
            None,
            None,
        )
        .unwrap();
        let id = core.snippets()[0].id.clone();
        let InsertOutcome::StartSession { session, .. } =
            core.engine().insert(id, "com.apple.TextEdit".into())
        else {
            panic!("a snippet with an AI block asks first");
        };
        (folder, core, session)
    }

    #[tokio::test]
    async fn a_block_streams_into_the_preview_and_goes_in_between_the_snippets_own_text() {
        let (address, bodies) = serve(
            &["```\n", "Thank you ", "— it's on its way!", "\n```"],
            false,
        );
        let (_settings_folder, profiles) = open();
        switch_on(&profiles, &address);
        let (_folder, _core, session) = session(REPLY);

        assert!(matches!(session.next(), SessionAction::Form { .. }));
        let SessionAction::Context { kinds } =
            session.submit_form([("who".to_owned(), "Dana".to_owned())].into())
        else {
            panic!("the declared clipboard is next");
        };
        assert_eq!(kinds, [ContextNeed::Clipboard]);
        let SessionAction::Ai { blocks } = session.provide_context(ContextSupply {
            clipboard: Some("PO-8841".into()),
            ..ContextSupply::default()
        }) else {
            panic!("the block is next");
        };
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].prompt, "Thank them");
        assert_eq!(blocks[0].fallback.as_deref(), Some("Thanks."));
        assert_eq!(session.ai_blocks(), blocks);

        let run = profiles
            .run_block(session.clone(), blocks[0].index)
            .await
            .unwrap();
        assert_eq!(run.block(), 0);
        assert_eq!(
            (run.profile(), run.model()),
            ("Local".into(), "model-a".into())
        );
        let sent = run.sent();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].kind, AiContextKind::Clipboard);
        assert_eq!(sent[0].bytes, Some(7));

        // Before a word has come, the preview shows the fallback where the
        // block is; as the words come, they take its place.
        let preview = session.preview_blocks([].into(), [].into());
        assert_eq!(preview.text, "Dear Dana,\nThanks.\nSam");
        while run.next().await.unwrap().is_some() {
            let drafts = [(run.block(), run.text())].into();
            let preview = session.preview_blocks([].into(), drafts);
            assert!(preview.text.starts_with("Dear Dana,\n"), "{}", preview.text);
            assert!(preview.text.ends_with("\nSam"), "{}", preview.text);
        }
        let answer = run.answer().unwrap();
        assert_eq!(answer, "Thank you — it's on its way!");

        // The span is in UTF-16 code units, which is what a text view counts.
        let preview = session.preview_blocks([].into(), [(0, answer.clone())].into());
        assert_eq!(
            preview.blocks,
            [BlockSpan {
                block: 0,
                start: 11,
                length: 28
            }]
        );

        let SessionAction::Expand { steps, .. } = session.answer_block(0, answer) else {
            panic!("that was the last question");
        };
        assert_eq!(
            steps,
            [PlanStep::InsertText {
                text: "Dear Dana,\nThank you — it's on its way!\nSam".into()
            }]
        );
        let body = bodies.recv().unwrap();
        assert!(
            body.contains("PO-8841") && body.contains("Thank them"),
            "{body}"
        );
        assert!(
            !body.contains("Dear Dana"),
            "the text around the block is not sent: {body}"
        );
    }

    #[tokio::test]
    async fn with_ai_off_the_block_falls_back_with_the_reason_the_run_gave() {
        let (_settings_folder, profiles) = open();
        profiles
            .save(
                draft("Remote", "https://api.example.com/v1"),
                AiKeyChange::Remove,
            )
            .unwrap();
        let (_folder, _core, session) =
            session("---\n---\n<{{ai: Anything | fallback: offline}}>\n");
        let SessionAction::Ai { blocks } = session.next() else {
            panic!("nothing else to ask");
        };
        let error = profiles
            .run_block(session.clone(), blocks[0].index)
            .await
            .unwrap_err();
        let AiBridgeError::Refused { message } = error else {
            panic!("{error:?}");
        };
        assert_eq!(message, "AI is switched off");
        let SessionAction::Expand { steps, .. } = session.fall_back(blocks[0].index, message)
        else {
            panic!("that was the last question");
        };
        assert_eq!(
            steps,
            [PlanStep::InsertText {
                text: "<offline>".into()
            }]
        );
    }

    #[tokio::test]
    async fn a_cancelled_session_has_no_block_to_run() {
        let (_settings_folder, profiles) = open();
        let (_folder, _core, session) = session("---\n---\n{{ai: Anything}}\n");
        assert_eq!(session.cancel(), []);
        let error = profiles.run_block(session.clone(), 0).await.unwrap_err();
        assert!(matches!(error, AiBridgeError::NotFound { .. }), "{error:?}");
        assert_eq!(session.answer_block(0, "late".into()), SessionAction::Done);
        assert_eq!(session.ai_blocks(), []);
        assert_eq!(session.preview_blocks([].into(), [].into()).text, "");
    }
}

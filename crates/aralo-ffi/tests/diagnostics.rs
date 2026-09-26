//! The diagnostics report holds counts and settings, never content (plan 5.7,
//! PRD P8).
//!
//! Every string the user or a model could have written is marked with `zqx`:
//! the library folder, a group, file names, labels, abbreviations, bodies, the
//! text typed around an abbreviation, a form answer, the clipboard, the
//! selection a command ran on, a model's answers, profile names, a header,
//! the address of a server and the API key. The test drives all of it
//! through the bridge the way the app does, copies the report, and looks for
//! the mark.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::Arc;

use aralo_ffi::{
    AiCommand, AiHeader, AiKeyChange, AiProfileDraft, AiProfiles, AiSwitches, ContextSupply, Core,
    DiagnosticFacts, InsertMethod, InsertOutcome, KeyAction, KeyInput, KeyStorage, ResetReason,
    SessionAction, ShellEvent,
};

const MARK: &str = "zqx";
const KEY: &str = "sk-zqx-api-key-0123456789abcdef";

/// A model server on this machine for one request: it answers with `answer`,
/// in one piece, and ends.
fn serve(answer: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!(
        "http://{}/zqx-server-path/v1",
        listener.local_addr().unwrap()
    );
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
        let mut stream = stream;
        let chunk = format!(
            "data: {{\"choices\":[{{\"index\":0,\"delta\":{{\"content\":{}}}}}]}}\n\n",
            serde_json::to_string(answer).unwrap()
        );
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n",
            )
            .unwrap();
        stream.write_all(chunk.as_bytes()).unwrap();
        stream
            .write_all(
                b"data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n\
                  data: [DONE]\n\n",
            )
            .unwrap();
    });
    address
}

/// An address nothing answers on.
fn nobody() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}/zqx-dead-path/v1", listener.local_addr().unwrap());
    drop(listener);
    address
}

fn profile(name: &str, base_url: &str) -> AiProfileDraft {
    AiProfileDraft {
        original_name: None,
        name: name.into(),
        base_url: base_url.into(),
        default_model: "zqx-model-name".into(),
        headers: vec![AiHeader {
            name: "X-Zqx-Header".into(),
            value: "zqx-header-value".into(),
        }],
    }
}

fn key(c: char) -> KeyInput {
    KeyInput::Char { scalar: c as u32 }
}

fn type_str(core: &Core, text: &str) -> Vec<KeyAction> {
    let engine = core.engine();
    text.chars()
        .map(|c| engine.on_key(key(c)))
        .filter(|action| *action != KeyAction::Pass)
        .collect()
}

/// The library: a text snippet, a snippet with a form, the clipboard and an
/// AI block, and a file that does not read, all in a marked group.
fn library(folder: &std::path::Path) -> Arc<Core> {
    let root = folder.join("zqx-library-folder");
    let group = root.join("zqx-group-name");
    std::fs::create_dir_all(&group).unwrap();
    std::fs::write(
        group.join("zqx-text-file.md"),
        "---\nlabel: zqx-label-text\nabbr: ;zqxab\n---\nzqx-body-text",
    )
    .unwrap();
    std::fs::write(
        group.join("zqx-reply-file.md"),
        "---\nlabel: zqx-label-reply\nabbr: ;zqxai\nai:\n  context: [clipboard]\n---\n\
         zqx-body-before {{field: zqxwho}} {{clipboard}}\n\
         {{ai: zqx-prompt-text | fallback: zqx-fallback-text}}\n",
    )
    .unwrap();
    std::fs::write(
        group.join("zqx-broken-file.md"),
        "---\nlabel: [zqx-unclosed\nabbr: ;zqxbad\n---\nzqx-broken-body",
    )
    .unwrap();
    let core = Core::open_library(
        root.to_string_lossy().into_owned(),
        Some(
            folder
                .join("zqx-cache-folder")
                .to_string_lossy()
                .into_owned(),
        ),
        None,
        None,
    )
    .unwrap();
    core.engine().set_front_app("com.apple.TextEdit".into());
    core
}

fn proofread(core: &Core) -> AiCommand {
    core.commands()
        .into_iter()
        .find(|command| command.label == "Fix spelling and grammar")
        .unwrap()
}

#[tokio::test]
async fn the_report_holds_no_snippet_context_typed_text_or_key() {
    let folder = tempfile::tempdir().unwrap();
    let core = library(folder.path());
    let engine = core.engine();

    // Typing: words around an abbreviation, the expansion, and an undo.
    assert!(type_str(&core, "zqx typed words before ").is_empty());
    let actions = type_str(&core, ";zqxab ");
    let [KeyAction::Expand {
        snippet_id,
        undo_delete_count: Some(undo),
        ..
    }] = actions.as_slice()
    else {
        panic!("{actions:?}");
    };
    engine.expansion_done(snippet_id.clone(), *undo, InsertMethod::Typed);
    assert!(matches!(
        engine.on_key(KeyInput::Undo),
        KeyAction::UndoExpansion { .. }
    ));
    assert!(type_str(&core, "zqx typed words after").is_empty());
    engine.reset(ResetReason::Manual);
    core.record_shell_event(ShellEvent::TapTimedOut);
    assert!(core
        .load_compat_table(folder.path().join("zqx-compat.toml").display().to_string())
        .is_err());

    // The AI settings, with a key, in the same folder as the cache.
    let profiles = AiProfiles::open(
        Some(
            folder
                .path()
                .join("zqx-cache-folder/profiles.toml")
                .display()
                .to_string(),
        ),
        KeyStorage::Memory,
    )
    .unwrap();
    profiles
        .save(
            profile("zqx-profile-block", &serve("zqx-block-answer")),
            AiKeyChange::Set { key: KEY.into() },
        )
        .unwrap();
    profiles
        .set_switches(AiSwitches {
            enabled: true,
            local_only: true,
        })
        .unwrap();

    // A snippet picked from the palette: a form answer, the clipboard, and a
    // model's answer to an AI block.
    let reply = core
        .snippets()
        .into_iter()
        .find(|snippet| snippet.label == "zqx-label-reply")
        .unwrap();
    let InsertOutcome::StartSession { session, .. } =
        engine.insert(reply.id, "com.apple.TextEdit".into())
    else {
        panic!("a snippet with a form asks first");
    };
    assert!(matches!(session.next(), SessionAction::Form { .. }));
    assert!(matches!(
        session.submit_form([("zqxwho".to_owned(), "zqx-form-answer".to_owned())].into()),
        SessionAction::Context { .. }
    ));
    let SessionAction::Ai { blocks } = session.provide_context(ContextSupply {
        clipboard: Some("zqx-clipboard-text".into()),
        ..ContextSupply::default()
    }) else {
        panic!("the block is next");
    };
    let run = profiles
        .run_block(session.clone(), blocks[0].index)
        .await
        .unwrap();
    while run.next().await.unwrap().is_some() {}
    assert!(matches!(
        session.answer_block(0, run.answer().unwrap()),
        SessionAction::Expand { .. }
    ));

    // A command on a selection, then one against a server that is not there.
    profiles
        .save(
            profile("zqx-profile-command", &serve("zqx-command-answer")),
            AiKeyChange::Keep,
        )
        .unwrap();
    profiles.set_default("zqx-profile-command".into()).unwrap();
    let run = profiles
        .run_command(proofread(&core), "zqx-selection-text\n".into())
        .await
        .unwrap();
    while run.next().await.unwrap().is_some() {}
    assert!(run.text().contains("zqx-command-answer"));

    profiles
        .save(profile("zqx-profile-dead", &nobody()), AiKeyChange::Keep)
        .unwrap();
    profiles.set_default("zqx-profile-dead".into()).unwrap();
    let failed = match profiles
        .run_command(proofread(&core), "zqx-selection-again\n".into())
        .await
    {
        Ok(run) => run.next().await.is_err(),
        Err(_) => true,
    };
    assert!(failed, "nothing answers there");

    let report = core.diagnostics_report(
        DiagnosticFacts {
            app_version: "Aralo 0.5.0 (42)".into(),
            os_version: "macOS 15.1 (24B83)".into(),
            accessibility: Some(true),
            input_monitoring: Some(true),
            tap_running: Some(true),
            secure_input: Some(false),
            paused: Some(false),
            excluded_apps: Some(2),
        },
        Some(profiles),
    );

    // Nothing marked, in any case, and not the key or the folder.
    assert!(
        !report.to_lowercase().contains(MARK),
        "the report leaks content:\n{report}"
    );
    assert!(!report.contains(KEY));
    assert!(!report.contains(&folder.path().display().to_string()));
    assert!(!report.contains("127.0.0.1"), "{report}");

    // And it is not empty: the counts are there.
    for line in [
        "  expansion.matched: 1",
        "  insert.typed: 1",
        "  insert.undone: 1",
        "  expansion.picked: 1",
        "  reset.manual: 1",
        "  tap.timed_out: 1",
        "  com.apple.TextEdit: 1 expanded, 1 undone",
        "  snippets: 2",
        "  app: Aralo 0.5.0 (42)",
        "  accessibility: granted",
        "  files with parse errors: 1",
    ] {
        assert!(report.contains(line), "{line:?} missing from:\n{report}");
    }
    assert!(
        report.contains("openai_compat local: block 1, command 1; errors: ai_network 1"),
        "{report}"
    );
    assert!(report.contains("compat_table_rejected"), "{report}");

    // The counts outlive the process: the file has them, and no content.
    let file =
        std::fs::read_to_string(folder.path().join("zqx-cache-folder/counters.json")).unwrap();
    assert!(!file.to_lowercase().contains(MARK), "{file}");
}

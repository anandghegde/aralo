//! The AI master switch (plan 4.10): with the switch off, no model loads and
//! no model call is possible.
//!
//! Everything here goes through the real network guard to a real socket on
//! this machine that counts the connections it is offered, with a key store
//! that counts the keys it is asked for. With AI off, every way Aralo has of
//! asking a model is tried, and the socket, the keys and the context stay
//! untouched; the embedding model search by meaning uses is never loaded,
//! not even attempted. Then AI goes on, to show the same calls do reach the
//! socket and the model does load — so the silence before was the switch,
//! not a test that could not hear — and off again, to show the model goes.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use aralo_ai::{
    AdapterKind, AiRequest, ContextKind, ContextSource, Feature, MemorySecretStore, Refusal,
    Secret, SecretError, SecretStore,
};
use aralo_core::ai::{
    builtin_commands, AiSettings, AiSettingsError, AiSwitches, Authoring, KeyChange, ProfileDraft,
};
use aralo_core::template::DiagnosticKind;
use aralo_core::{
    Core, Expand, Field, LibraryChange, LibraryListener, Query, Runtime, RuntimeOptions,
    SessionStep,
};

/// A model server on this machine that answers nothing and counts every
/// connection it is offered.
struct Socket {
    address: String,
    connections: Arc<AtomicUsize>,
}

impl Socket {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}/v1", listener.local_addr().unwrap());
        let connections = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&connections);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                counted.fetch_add(1, Ordering::SeqCst);
                // Hang up at once: the count is the point, not the answer.
                drop(stream);
            }
        });
        Self {
            address,
            connections,
        }
    }

    fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }
}

/// Keys in memory, counting how often one is read.
#[derive(Default)]
struct CountingKeys {
    keys: MemorySecretStore,
    reads: AtomicUsize,
}

impl SecretStore for CountingKeys {
    fn get(&self, key_ref: &str) -> Result<Option<Secret>, SecretError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.keys.get(key_ref)
    }

    fn set(&self, key_ref: &str, key: &Secret) -> Result<(), SecretError> {
        self.keys.set(key_ref, key)
    }

    fn delete(&self, key_ref: &str) -> Result<(), SecretError> {
        self.keys.delete(key_ref)
    }
}

/// Context that counts how often it is asked for.
#[derive(Default)]
struct CountingContext {
    asked: usize,
}

impl ContextSource for CountingContext {
    fn fetch(&mut self, _kind: ContextKind) -> Option<String> {
        self.asked += 1;
        Some("the clipboard".to_owned())
    }
}

struct Quiet;

impl LibraryListener for Quiet {
    fn changed(&self, _: LibraryChange, _: &Core) {}
}

fn tiny() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/embed/tiny")
}

fn draft(socket: &Socket) -> ProfileDraft {
    ProfileDraft {
        original_name: Some("Local".into()),
        name: "Local".into(),
        adapter: AdapterKind::OpenAiCompat,
        base_url: socket.address.clone(),
        default_model: "model-a".into(),
        headers: vec![],
    }
}

fn switches(enabled: bool, local_only: bool) -> AiSwitches {
    AiSwitches {
        enabled,
        local_only,
    }
}

fn is_off(result: Result<impl std::fmt::Debug, AiSettingsError>) -> bool {
    matches!(&result, Err(error) if error.to_string() == Refusal::Off.to_string())
}

/// A library with one snippet that asks a model, and one the model could find
/// by meaning: the tiny model averages tokens, so "world hello" is the vector
/// "hello world" is, and no word search finds one in the other.
fn library(root: &Path) {
    std::fs::create_dir_all(root).unwrap();
    std::fs::write(
        root.join("greeting.md"),
        "---\nid: 01M4000000000000000000SW01\nlabel: Greeting\nabbr: \";gr\"\n---\nhello world\n",
    )
    .unwrap();
    std::fs::write(
        root.join("ask.md"),
        "---\nid: 01M4000000000000000000SW02\nlabel: Ask\nabbr: \";ask\"\n---\n\
         Dear Sam, {{ai: Write one kind sentence | fallback: Thank you.}}\n",
    )
    .unwrap();
}

fn found_by_meaning(runtime: &Runtime) -> bool {
    runtime
        .search(&Query::new("world hello"))
        .iter()
        .any(|hit| hit.field == Field::Meaning)
}

/// Every way Aralo has of asking a model, once each. Returns how many were
/// refused because AI is off.
async fn ask_everything(settings: &AiSettings, socket: &Socket, core: &Core) -> usize {
    let mut refused = 0;
    let command = builtin_commands().remove(0);
    refused += usize::from(is_off(settings.run_command(&command, "their going").await));
    refused += usize::from(is_off(
        settings
            .run_authoring(&Authoring::Proofread, "their going")
            .await,
    ));
    refused += usize::from(is_off(
        settings
            .test_connection(&draft(socket), KeyChange::Keep)
            .await,
    ));
    refused += usize::from(is_off(
        settings.list_models(&draft(socket), KeyChange::Keep).await,
    ));
    refused += usize::from(is_off(settings.probe("Local").await));
    refused += usize::from(is_off(settings.detect_local_servers().await));
    refused += usize::from(is_off(settings.load_model(&tiny())));

    // An AI block: its request, and the session answering it.
    let id = core
        .snippets()
        .iter()
        .find(|snippet| snippet.display_name() == "Ask")
        .unwrap()
        .id;
    let Some(Expand::Session(mut session)) = core.insert(id) else {
        panic!("the AI block opens a session");
    };
    let SessionStep::Ai(blocks) = session.step() else {
        panic!("the block is the first step");
    };
    let request = session.block_request(blocks[0].index).unwrap();
    refused += usize::from(is_off(settings.run_block(&request).await));
    if let SessionStep::Ready(expansion) = settings.answer_blocks(&mut session).await {
        // Off, the block puts in its fallback and says why.
        let fell_back = expansion
            .template_diagnostics
            .iter()
            .any(|diagnostic| diagnostic.kind == DiagnosticKind::AiFallback);
        refused += usize::from(fell_back);
    }
    refused
}

#[tokio::test]
async fn with_the_switch_off_no_model_loads_and_no_model_call_is_possible() {
    let folder = aralo_testkit::tempdir().unwrap();
    let socket = Socket::start();
    let keys = Arc::new(CountingKeys::default());
    let settings =
        AiSettings::open_with_secrets(folder.path().join("profiles.toml"), keys.clone()).unwrap();
    settings
        .save(
            &ProfileDraft {
                original_name: None,
                ..draft(&socket)
            },
            KeyChange::Set(Secret::new("a-local-key")),
        )
        .unwrap();
    // AI is off until someone switches it on.
    assert_eq!(settings.switches().unwrap(), switches(false, false));
    let reads_before = keys.reads.load(Ordering::SeqCst);

    library(&folder.path().join("Aralo"));
    let core = Core::open_read_only(&folder.path().join("Aralo")).unwrap();
    let runtime = Runtime::with_core(
        Core::open_without_starter(&folder.path().join("Aralo")).unwrap(),
        RuntimeOptions {
            index: Some(folder.path().join("state/index.sqlite3")),
            watch: false,
            ..RuntimeOptions::default()
        },
        Arc::new(Quiet),
    )
    .unwrap();

    // No model loads: not even one whose folder is missing, which would have
    // failed with a read error had anything tried.
    runtime.use_model(folder.path().join("no-such-model"), &settings);
    runtime.use_model(tiny(), &settings);
    runtime.flush();
    assert!(runtime.read(|core| core.meaning().is_none()));
    assert!(!found_by_meaning(&runtime));
    assert_eq!(
        runtime.meaning_error().as_deref(),
        Some("AI is switched off")
    );

    // No model call is possible: every way of making one is refused ...
    assert_eq!(ask_everything(&settings, &socket, &core).await, 9);
    // ... before any key is read, any context gathered or any connection made.
    let mut context = CountingContext::default();
    let refused = settings.gateway().prepare(
        AiRequest {
            feature: Feature::Command,
            profile: settings.profile("Local").unwrap().profile,
            model: None,
            system: String::new(),
            instruction: "Say hello".into(),
            declared: vec![ContextKind::Clipboard],
            max_tokens: None,
            temperature: None,
            json_output: false,
        },
        &mut context,
    );
    assert_eq!(refused.unwrap_err(), Refusal::Off);
    assert_eq!(context.asked, 0);
    assert_eq!(keys.reads.load(Ordering::SeqCst), reads_before);
    assert_eq!(socket.connections(), 0);

    // On, in local-only mode: the model is on this machine, so it loads, and
    // a call to the server on this machine goes out.
    settings.set_switches(switches(true, true)).unwrap();
    runtime.flush();
    assert!(found_by_meaning(&runtime));
    assert_eq!(runtime.meaning_error(), None);
    assert!(settings.load_model(&tiny()).is_ok());
    let _ = settings
        .test_connection(&draft(&socket), KeyChange::Keep)
        .await;
    assert!(
        socket.connections() > 0,
        "the socket hears a call when AI is on"
    );
    assert!(keys.reads.load(Ordering::SeqCst) > reads_before);

    // Off again: search stops using the model at once, before the indexer
    // has dropped it, and nothing reaches the socket any more.
    settings.set_switches(switches(false, true)).unwrap();
    assert!(runtime.read(|core| core.meaning().is_none()));
    assert!(!found_by_meaning(&runtime));
    runtime.flush();
    assert!(!found_by_meaning(&runtime));
    assert_eq!(
        runtime.meaning_error().as_deref(),
        Some("AI is switched off")
    );
    let heard = socket.connections();
    assert_eq!(ask_everything(&settings, &socket, &core).await, 9);
    assert_eq!(socket.connections(), heard);
}

#[test]
fn a_switch_flipped_by_another_process_is_followed_on_reload() {
    let folder = aralo_testkit::tempdir().unwrap();
    let path = folder.path().join("profiles.toml");
    let settings =
        AiSettings::open_with_secrets(&path, Arc::new(MemorySecretStore::new())).unwrap();
    library(&folder.path().join("Aralo"));
    let runtime = Runtime::with_core(
        Core::open_without_starter(&folder.path().join("Aralo")).unwrap(),
        RuntimeOptions {
            index: Some(folder.path().join("state/index.sqlite3")),
            watch: false,
            ..RuntimeOptions::default()
        },
        Arc::new(Quiet),
    )
    .unwrap();
    runtime.use_model(tiny(), &settings);

    // `aralo ai on` from a terminal, while the app runs.
    let terminal =
        AiSettings::open_with_secrets(&path, Arc::new(MemorySecretStore::new())).unwrap();
    terminal.set_switches(switches(true, false)).unwrap();
    runtime.flush();
    assert!(
        !found_by_meaning(&runtime),
        "nothing changes until the app reads the file"
    );
    settings.reload().unwrap();
    runtime.flush();
    assert!(found_by_meaning(&runtime));

    // And `aralo ai off`.
    terminal.set_switches(switches(false, false)).unwrap();
    settings.reload().unwrap();
    assert!(!found_by_meaning(&runtime));
    runtime.flush();
    assert!(runtime.read(|core| core.meaning().is_none()));
}

#[test]
fn a_file_rewritten_with_the_same_switches_changes_nothing() {
    let folder = aralo_testkit::tempdir().unwrap();
    let settings = AiSettings::open_with_secrets(
        folder.path().join("profiles.toml"),
        Arc::new(MemorySecretStore::new()),
    )
    .unwrap();
    let told = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&told);
    settings.watch_switches(move |_| {
        counted.fetch_add(1, Ordering::SeqCst);
    });
    settings.set_switches(switches(true, false)).unwrap();
    settings.set_switches(switches(true, false)).unwrap();
    settings.reload().unwrap();
    settings.set_default_profile(None).unwrap();
    assert_eq!(told.load(Ordering::SeqCst), 1);
    settings.set_switches(switches(true, true)).unwrap();
    assert_eq!(told.load(Ordering::SeqCst), 2);
    assert_eq!(settings.switches_in_force(), switches(true, true));
}

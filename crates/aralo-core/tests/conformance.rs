//! The conformance suite against a mock endpoint (plan 7.4, task 4.9).
//!
//! The mock is an OpenAI-compatible server on this machine that answers each
//! check as a real one would, in the framing of a recorded stream: the chunks
//! before the answer, the answer in pieces shaped like the recording's, and
//! the finish, usage and `[DONE]` chunks after it, as the provider sent them.
//! Every recording in `fixtures/sse` that shows a whole answer passes, the
//! evaluation set included; a mock broken in one way fails naming the check
//! that caught it. Everything goes through the real network guard.
//!
//! The same mock measures what Aralo adds to the first token: with a server
//! that answers at once, the time to the first piece is Aralo's own.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use aralo_ai::{AdapterKind, AiRequest, Delta, Feature, MemorySecretStore, Refusal, Secret};
use aralo_core::ai::conformance::{
    bundled_evaluation_set, compatibility_table, planned_endpoints, ConformanceOptions,
    ConformanceReport, ProtocolCheck, Verdict,
};
use aralo_core::ai::{AiSettings, AiSettingsError, AiSwitches, KeyChange, ProfileDraft};
use serde_json::{json, Value};

const MODEL: &str = "mock-model";
const KEY: &str = "mock-key-for-the-conformance-test";

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A recording, cut where the answer is: what comes before the first chunk
/// with text, one such chunk to copy for each piece, and what comes after
/// the last one.
#[derive(Clone)]
struct Framing {
    before: Vec<String>,
    piece: Value,
    after: Vec<String>,
}

impl Framing {
    fn of(name: &str) -> Self {
        let text = std::fs::read_to_string(repo().join("fixtures/sse").join(name)).unwrap();
        let events: Vec<String> = text
            .split("\n\n")
            .map(str::trim)
            .filter(|event| !event.is_empty())
            .map(str::to_owned)
            .collect();
        let has_text = |event: &String| {
            event
                .strip_prefix("data: ")
                .and_then(|data| serde_json::from_str::<Value>(data).ok())
                .and_then(|chunk| {
                    chunk["choices"][0]["delta"]["content"]
                        .as_str()
                        .map(|content| !content.is_empty())
                })
                .unwrap_or(false)
        };
        let first = events.iter().position(has_text).unwrap();
        let last = events.iter().rposition(has_text).unwrap();
        Self {
            before: events[..first].to_vec(),
            piece: serde_json::from_str(events[first].strip_prefix("data: ").unwrap()).unwrap(),
            after: events[last + 1..].to_vec(),
        }
    }

    fn event(&self, text: &str) -> String {
        let mut chunk = self.piece.clone();
        chunk["choices"][0]["delta"]["content"] = json!(text);
        format!("data: {chunk}")
    }

    /// The same framing with no token counts anywhere in it.
    fn without_usage(mut self) -> Self {
        self.after = self
            .after
            .into_iter()
            .filter_map(|event| {
                let Some(mut chunk) = event
                    .strip_prefix("data: ")
                    .and_then(|data| serde_json::from_str::<Value>(data).ok())
                else {
                    return Some(event);
                };
                let object = chunk.as_object_mut()?;
                object.remove("usage");
                if let Some(groq) = object.get_mut("x_groq").and_then(Value::as_object_mut) {
                    groq.remove("usage");
                }
                let empty = chunk["choices"].as_array().is_some_and(Vec::is_empty);
                (!empty).then(|| format!("data: {chunk}"))
            })
            .collect();
        self
    }
}

/// How a mock is broken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fault {
    None,
    /// Every answer comes back as one JSON body.
    Whole,
    /// The model never reads the system prompt.
    IgnoresSystem,
    /// The model garbles the system prompt's word the first time it is
    /// asked, as a small one sampling at its own temperature can.
    GarblesSystemOnce,
    NoModelsRoute,
    /// Any model name is served.
    AnyModel,
    /// A model it does not have is a server error.
    ServerErrorForUnknownModel,
    /// Any key, or none, is accepted.
    AnyKey,
    /// Every stream stops before the model finishes.
    CutOff,
    NoUsage,
}

/// What the mock saw of one request.
#[derive(Debug, Clone)]
struct Seen {
    path: String,
    authorization: Option<String>,
}

struct Mock {
    base_url: String,
    port: u16,
    seen: Arc<Mutex<Vec<Seen>>>,
    /// For each long answer: how long after its first piece went out the
    /// client's end was seen to close.
    closed_after: Arc<Mutex<Vec<Duration>>>,
}

#[derive(Clone)]
struct Behaviour {
    framing: Framing,
    fault: Fault,
    key: Option<&'static str>,
    /// How many times the system prompt check has asked.
    system_asked: Arc<AtomicUsize>,
}

impl Mock {
    fn start(transcript: &str, fault: Fault) -> Self {
        let mut framing = Framing::of(transcript);
        if fault == Fault::NoUsage {
            framing = framing.without_usage();
        }
        let behaviour = Behaviour {
            framing,
            fault,
            key: Some(KEY),
            system_asked: Arc::new(AtomicUsize::new(0)),
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let closed_after = Arc::new(Mutex::new(Vec::new()));
        let (seen_by_server, closed_by_server) = (seen.clone(), closed_after.clone());
        std::thread::spawn(move || {
            for socket in listener.incoming().flatten() {
                let behaviour = behaviour.clone();
                let seen = seen_by_server.clone();
                let closed = closed_by_server.clone();
                std::thread::spawn(move || {
                    let _ = serve(socket, &behaviour, &seen, &closed);
                });
            }
        });
        Self {
            base_url: format!("http://127.0.0.1:{port}/v1"),
            port,
            seen,
            closed_after,
        }
    }

    fn requests(&self) -> usize {
        self.seen.lock().unwrap().len()
    }
}

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

fn read_request(reader: &mut BufReader<TcpStream>) -> std::io::Result<Request> {
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_owned();
    let path = parts.next().unwrap_or_default().to_owned();
    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
        }
    }
    let length = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .and_then(|(_, value)| value.parse().ok())
        .unwrap_or(0);
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok(Request {
        method,
        path,
        headers,
        body,
    })
}

fn json_response(socket: &mut TcpStream, status: &str, body: &Value) -> std::io::Result<()> {
    let body = body.to_string();
    write!(
        socket,
        "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\
         connection: close\r\n\r\n{body}",
        body.len()
    )
}

fn error(message: &str, code: &str) -> Value {
    json!({"error": {"message": message, "type": "invalid_request_error", "code": code}})
}

fn serve(
    mut socket: TcpStream,
    behaviour: &Behaviour,
    seen: &Mutex<Vec<Seen>>,
    closed_after: &Mutex<Vec<Duration>>,
) -> std::io::Result<()> {
    let mut reader = BufReader::new(socket.try_clone()?);
    let request = read_request(&mut reader)?;
    seen.lock().unwrap().push(Seen {
        path: request.path.clone(),
        authorization: request.header("authorization").map(str::to_owned),
    });

    if let Some(key) = behaviour.key {
        let sent = request.header("authorization") == Some(&format!("Bearer {key}"));
        if !sent && behaviour.fault != Fault::AnyKey {
            return json_response(
                &mut socket,
                "401 Unauthorized",
                &error("Incorrect API key provided.", "invalid_api_key"),
            );
        }
    }

    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/v1/models") if behaviour.fault == Fault::NoModelsRoute => json_response(
            &mut socket,
            "404 Not Found",
            &json!({"error": {"message": "Not Found"}}),
        ),
        ("GET", "/v1/models") => json_response(
            &mut socket,
            "200 OK",
            &json!({"object": "list", "data": [
                {"id": MODEL, "object": "model", "created": 1, "owned_by": "mock"}
            ]}),
        ),
        ("POST", "/v1/chat/completions") => chat(socket, &request, behaviour, closed_after),
        _ => json_response(
            &mut socket,
            "404 Not Found",
            &error("Not Found", "not_found"),
        ),
    }
}

fn chat(
    mut socket: TcpStream,
    request: &Request,
    behaviour: &Behaviour,
    closed_after: &Mutex<Vec<Duration>>,
) -> std::io::Result<()> {
    let body: Value = serde_json::from_slice(&request.body).unwrap();
    let model = body["model"].as_str().unwrap_or_default();
    if model != MODEL {
        match behaviour.fault {
            Fault::AnyModel => {}
            Fault::ServerErrorForUnknownModel => {
                return json_response(
                    &mut socket,
                    "500 Internal Server Error",
                    &json!({"error": {"message": "internal error"}}),
                )
            }
            _ => {
                return json_response(
                    &mut socket,
                    "404 Not Found",
                    &error(
                        &format!("The model `{model}` does not exist."),
                        "model_not_found",
                    ),
                )
            }
        }
    }
    let message = |role: &str| {
        body["messages"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|message| message["role"] == role)
            .filter_map(|message| message["content"].as_str())
            .collect::<Vec<_>>()
            .join("\n")
    };
    let (system, user) = (message("system"), message("user"));
    let json_mode = body["response_format"]["type"] == "json_object";

    if user.contains("two hundred") {
        return long_answer(socket, behaviour, closed_after);
    }
    let answer = if json_mode {
        "{\"ok\": true}".to_owned()
    } else if system.contains("PINEAPPLE") && behaviour.fault != Fault::IgnoresSystem {
        let asked = behaviour.system_asked.fetch_add(1, Ordering::SeqCst);
        if behaviour.fault == Fault::GarblesSystemOnce && asked == 0 {
            "PIECE".to_owned()
        } else {
            "PINEAPPLE".to_owned()
        }
    } else {
        answer_to(&user)
    };

    if behaviour.fault == Fault::Whole {
        return json_response(
            &mut socket,
            "200 OK",
            &json!({
                "id": "chatcmpl-whole", "object": "chat.completion", "model": MODEL,
                "choices": [{"index": 0, "message": {"role": "assistant", "content": answer},
                             "finish_reason": "stop"}],
                "usage": {"prompt_tokens": 12, "completion_tokens": 5, "total_tokens": 17}
            }),
        );
    }
    let framing = &behaviour.framing;
    let mut events = framing.before.clone();
    events.extend(pieces(&answer).iter().map(|piece| framing.event(piece)));
    if behaviour.fault != Fault::CutOff {
        events.extend(framing.after.iter().cloned());
    }
    stream_head(&mut socket)?;
    for event in &events {
        send_event(&mut socket, event)?;
    }
    socket.write_all(b"0\r\n\r\n")
}

/// Answers the evaluation set as a good model would, and everything else
/// with what the check asks for. A case added to `conformance/evaluation.toml`
/// needs its answer here.
fn answer_to(user: &str) -> String {
    let answers = [
        (
            "you're order",
            "Thank you for your order. We shipped it yesterday, and it should arrive tomorrow.",
        ),
        (
            "cant make it",
            "I am afraid I cannot attend on Thursday. Would Friday suit you instead?",
        ),
        (
            "reach out",
            "Thursday afternoon's meeting has to move to Friday morning.",
        ),
        (
            "has shiped",
            "Your order from {{date: %d %B}} has shipped, and it will arrive in {{field: days}} \
             days.",
        ),
        ("Today's date", "Today is {{date: %A, %d %B %Y}}."),
        (
            "one to ten",
            "one two three four five six seven eight nine ten",
        ),
        ("single word: ok", "ok"),
    ];
    answers
        .iter()
        .find(|(asked, _)| user.contains(asked))
        .map_or_else(
            || "Hello! How can I help?".to_owned(),
            |(_, answer)| (*answer).to_owned(),
        )
}

/// Words with the space before each, as models stream them.
fn pieces(answer: &str) -> Vec<String> {
    let mut pieces: Vec<String> = Vec::new();
    for (index, word) in answer.split(' ').enumerate() {
        pieces.push(if index == 0 {
            word.to_owned()
        } else {
            format!(" {word}")
        });
    }
    pieces
}

fn stream_head(socket: &mut TcpStream) -> std::io::Result<()> {
    socket.write_all(
        b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\
          connection: close\r\n\r\n",
    )
}

fn send_event(socket: &mut TcpStream, event: &str) -> std::io::Result<()> {
    let data = format!("{event}\n\n");
    write!(socket, "{:x}\r\n{data}\r\n", data.len())?;
    socket.flush()
}

/// Two hundred pieces, one every 20 ms, for as long as the client listens.
/// Between pieces it waits on a read, which ends at once when the client
/// closes its end: that is the cancel, and how soon it is seen is recorded.
fn long_answer(
    mut socket: TcpStream,
    behaviour: &Behaviour,
    closed_after: &Mutex<Vec<Duration>>,
) -> std::io::Result<()> {
    let framing = &behaviour.framing;
    stream_head(&mut socket)?;
    for event in &framing.before {
        send_event(&mut socket, event)?;
    }
    socket.set_read_timeout(Some(Duration::from_millis(20)))?;
    let mut first_sent = None;
    let mut buffer = [0u8; 64];
    for number in 1..=200 {
        let piece = if number == 1 {
            "1".to_owned()
        } else {
            format!("\n{number}")
        };
        if send_event(&mut socket, &framing.event(&piece)).is_err() {
            break;
        }
        first_sent.get_or_insert_with(Instant::now);
        match socket.read(&mut buffer) {
            Ok(0) => break,
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => break,
        }
    }
    if let Some(sent) = first_sent {
        closed_after.lock().unwrap().push(sent.elapsed());
    }
    Ok(())
}

// The client side.

fn settings(folder: &Path) -> AiSettings {
    let settings = AiSettings::open_with_secrets(
        folder.join("profiles.toml"),
        Arc::new(MemorySecretStore::new()),
    )
    .unwrap()
    .with_clock(|| "2026-09-25T12:00:00Z".to_owned());
    settings
        .set_switches(AiSwitches {
            enabled: true,
            local_only: false,
        })
        .unwrap();
    settings
}

fn save(settings: &AiSettings, name: &str, base_url: &str, key: Option<&str>) {
    let draft = ProfileDraft {
        original_name: None,
        name: name.into(),
        adapter: AdapterKind::OpenAiCompat,
        base_url: base_url.into(),
        default_model: MODEL.into(),
        headers: Vec::new(),
    };
    let key = key.map_or(KeyChange::Remove, |key| KeyChange::Set(Secret::new(key)));
    settings.save(&draft, key).unwrap();
}

fn options(evaluation: bool) -> ConformanceOptions {
    ConformanceOptions {
        evaluation: evaluation.then(|| bundled_evaluation_set().unwrap()),
        timeout: Duration::from_secs(20),
        key: None,
    }
}

async fn run(transcript: &str, fault: Fault) -> (ConformanceReport, Mock) {
    let folder = tempfile::tempdir().unwrap();
    let mock = Mock::start(transcript, fault);
    let settings = settings(folder.path());
    save(&settings, "Mock", &mock.base_url, Some(KEY));
    let report = settings.conformance("Mock", &options(false)).await.unwrap();
    (report, mock)
}

fn verdicts(report: &ConformanceReport) -> String {
    report
        .protocol
        .iter()
        .map(|result| format!("{} {:?}: {}", result.check, result.verdict, result.detail))
        .collect::<Vec<_>>()
        .join("\n")
}

fn failed(report: &ConformanceReport) -> Vec<&str> {
    report
        .protocol
        .iter()
        .filter(|result| result.verdict == Verdict::Fail)
        .map(|result| result.check.as_str())
        .collect()
}

#[tokio::test]
async fn every_recorded_framing_passes_every_check_and_the_evaluation_set() {
    for transcript in [
        "openai.sse",
        "openrouter.sse",
        "groq.sse",
        "ollama.sse",
        "surplus-reasoning.sse",
    ] {
        let folder = tempfile::tempdir().unwrap();
        let mock = Mock::start(transcript, Fault::None);
        let settings = settings(folder.path());
        save(&settings, "Mock", &mock.base_url, Some(KEY));
        let report = settings.conformance("Mock", &options(true)).await.unwrap();

        assert!(report.passed, "{transcript}\n{}", verdicts(&report));
        assert_eq!(
            report
                .protocol
                .iter()
                .map(|result| (result.check.as_str(), result.verdict))
                .collect::<Vec<_>>(),
            ProtocolCheck::ALL
                .iter()
                .map(|check| (check.name(), Verdict::Pass))
                .collect::<Vec<_>>(),
            "{transcript}\n{}",
            verdicts(&report)
        );
        let evaluation = report.evaluation.as_ref().unwrap();
        assert_eq!(
            evaluation.passed,
            evaluation.cases.len(),
            "{transcript}: {:#?}",
            evaluation.cases
        );
        assert!(report.first_token_ms.is_some());

        // The cancel closed the connection within moments of the first
        // piece, not after the 4 s the whole answer takes.
        let closed = mock.closed_after.lock().unwrap().clone();
        assert_eq!(closed.len(), 1, "{transcript}");
        assert!(
            closed[0] < Duration::from_secs(1),
            "{transcript}: {closed:?}"
        );

        // The report holds the host and whether a key went, and no key.
        let written = report.to_json();
        assert!(!written.contains(KEY), "{written}");
        assert!(!written.contains("/v1"), "{written}");
        assert_eq!(report.endpoint.host, format!("127.0.0.1:{}", mock.port));
        assert!(report.endpoint.key);
        assert!(report.endpoint.local);
        assert_eq!(ConformanceReport::from_json(&written).unwrap(), report);

        // The key went with every request but the one sent a wrong key on
        // purpose, and the endpoint was asked for its models.
        let seen = mock.seen.lock().unwrap();
        assert!(seen.iter().any(|request| request.path == "/v1/models"));
        let bearer = format!("Bearer {KEY}");
        assert_eq!(
            seen.iter()
                .filter(|request| request.authorization.as_deref() != Some(bearer.as_str()))
                .count(),
            1,
            "{transcript}"
        );
    }
}

#[tokio::test]
async fn a_broken_endpoint_fails_naming_the_check_that_caught_it() {
    for (fault, expected) in [
        (Fault::Whole, vec!["stream"]),
        (Fault::IgnoresSystem, vec!["system_prompt"]),
        (Fault::NoModelsRoute, vec!["models"]),
        (Fault::ServerErrorForUnknownModel, vec!["unknown_model"]),
    ] {
        let (report, _mock) = run("openai.sse", fault).await;
        assert!(!report.passed, "{fault:?}");
        assert_eq!(
            failed(&report),
            expected,
            "{fault:?}\n{}",
            verdicts(&report)
        );
        let failures: Vec<&str> = report
            .failures()
            .map(|result| result.check.as_str())
            .collect();
        assert_eq!(failures, expected, "{fault:?}");
    }

    let (report, _mock) = run("openai.sse", Fault::IgnoresSystem).await;
    let system = report.check(ProtocolCheck::SystemPrompt).unwrap();
    assert_eq!(
        system.detail,
        "the model did not do what the system prompt asked in 3 tries: it wrote \u{201c}Hello! \
         How can I help?\u{201d}, \u{201c}Hello! How can I help?\u{201d}, \u{201c}Hello! How can I \
         help?\u{201d}"
    );

    let (report, _mock) = run("openai.sse", Fault::Whole).await;
    let stream = report.check(ProtocolCheck::Stream).unwrap();
    assert_eq!(stream.detail, "the answer came back whole, not in pieces");

    let (report, _mock) = run("openai.sse", Fault::CutOff).await;
    assert!(!report.passed);
    let stream = report.check(ProtocolCheck::Stream).unwrap();
    assert_eq!(stream.verdict, Verdict::Fail);
    assert!(
        stream.detail.contains("ended before the model finished"),
        "{}",
        stream.detail
    );
}

#[tokio::test]
async fn an_endpoint_that_answers_what_it_should_refuse_is_skipped_not_failed() {
    let (report, _mock) = run("ollama.sse", Fault::AnyModel).await;
    let check = report.check(ProtocolCheck::UnknownModel).unwrap();
    assert_eq!(check.verdict, Verdict::Skip);
    assert!(check.detail.contains("answered anyway"), "{}", check.detail);
    assert!(report.passed, "{}", verdicts(&report));

    // A word garbled once is a slip of sampling; asked again, the model
    // shows it read the system prompt.
    let (report, _mock) = run("ollama.sse", Fault::GarblesSystemOnce).await;
    let check = report.check(ProtocolCheck::SystemPrompt).unwrap();
    assert_eq!(check.verdict, Verdict::Pass);
    assert_eq!(
        check.detail,
        "the model answered as the system prompt asked at try 2 of 3, after writing \u{201c}PIECE\u{201d}"
    );

    let (report, _mock) = run("ollama.sse", Fault::AnyKey).await;
    let check = report.check(ProtocolCheck::WrongKey).unwrap();
    assert_eq!(check.verdict, Verdict::Skip);
    assert!(report.passed, "{}", verdicts(&report));

    // Usage is not required: without it an endpoint still conforms.
    let (report, _mock) = run("groq.sse", Fault::NoUsage).await;
    assert_eq!(failed(&report), ["usage"], "{}", verdicts(&report));
    assert!(report.passed);
}

#[tokio::test]
async fn a_profile_with_no_key_skips_the_wrong_key_check() {
    let folder = tempfile::tempdir().unwrap();
    let mock = Mock::start("ollama.sse", Fault::AnyKey);
    let settings = settings(folder.path());
    save(&settings, "Local", &mock.base_url, None);
    let report = settings
        .conformance("Local", &options(false))
        .await
        .unwrap();
    let check = report.check(ProtocolCheck::WrongKey).unwrap();
    assert_eq!(
        (check.verdict, check.detail.as_str()),
        (Verdict::Skip, "the profile sends no key")
    );
    assert!(!report.endpoint.key);
    assert!(report.passed, "{}", verdicts(&report));
}

#[tokio::test]
async fn a_key_given_for_the_run_is_sent_and_saved_nowhere() {
    let folder = tempfile::tempdir().unwrap();
    let mock = Mock::start("openai.sse", Fault::None);
    let settings = settings(folder.path());
    save(&settings, "Mock", &mock.base_url, None);
    let mut options = options(true);
    options.key = Some(Secret::new(KEY));
    let report = settings.conformance("Mock", &options).await.unwrap();
    assert!(report.passed, "{}", verdicts(&report));
    assert!(report.endpoint.key);
    assert_eq!(
        report.check(ProtocolCheck::WrongKey).unwrap().verdict,
        Verdict::Pass
    );
    let evaluation = report.evaluation.unwrap();
    assert_eq!(
        evaluation.passed,
        evaluation.cases.len(),
        "{:#?}",
        evaluation.cases
    );
    assert!(!settings.profile("Mock").unwrap().has_key());
    let file = std::fs::read_to_string(folder.path().join("profiles.toml")).unwrap();
    assert!(!file.contains(KEY));
}

#[tokio::test]
async fn a_run_is_refused_before_anything_is_sent() {
    let folder = tempfile::tempdir().unwrap();
    let mock = Mock::start("openai.sse", Fault::None);
    let settings = settings(folder.path());
    save(&settings, "Mock", &mock.base_url, Some(KEY));
    save(&settings, "Remote", "https://api.openai.com/v1", Some(KEY));

    settings
        .set_switches(AiSwitches {
            enabled: false,
            local_only: false,
        })
        .unwrap();
    let refused = settings.conformance("Mock", &options(true)).await;
    assert!(
        matches!(
            &refused,
            Err(AiSettingsError::Ai(aralo_ai::AiError::Refused(
                Refusal::Off
            )))
        ),
        "{refused:?}"
    );
    assert_eq!(mock.requests(), 0);

    settings
        .set_switches(AiSwitches {
            enabled: true,
            local_only: true,
        })
        .unwrap();
    let refused = settings.conformance("Remote", &options(true)).await;
    assert!(
        matches!(
            &refused,
            Err(AiSettingsError::Ai(aralo_ai::AiError::Refused(
                Refusal::LocalOnly { .. }
            )))
        ),
        "{refused:?}"
    );
}

/// The time to the first piece of an answer from a raw socket, which is the
/// mock's own share of it.
fn raw_first_token(port: u16) -> Duration {
    let started = Instant::now();
    let mut socket = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let body = json!({
        "model": MODEL, "stream": true,
        "messages": [{"role": "user", "content": "Reply with the single word: ok"}]
    })
    .to_string();
    write!(
        socket,
        "POST /v1/chat/completions HTTP/1.1\r\nhost: 127.0.0.1\r\nauthorization: Bearer {KEY}\r\n\
         content-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut reader = BufReader::new(socket);
    let mut line = String::new();
    while reader.read_line(&mut line).unwrap() > 0 {
        if line.contains("\"content\":\"ok\"") {
            return started.elapsed();
        }
        line.clear();
    }
    panic!("the mock never answered");
}

fn median(mut times: Vec<Duration>) -> Duration {
    times.sort();
    times[times.len() / 2]
}

#[tokio::test]
async fn aralo_adds_under_30_ms_to_the_first_token() {
    let folder = tempfile::tempdir().unwrap();
    let mock = Mock::start("openai.sse", Fault::None);
    let settings = settings(folder.path());
    save(&settings, "Mock", &mock.base_url, Some(KEY));
    let profile = settings.profile("Mock").unwrap().profile;

    let mut through_aralo = Vec::new();
    let mut raw = Vec::new();
    for _ in 0..15 {
        // The whole path a feature's request takes: the policy check,
        // framing, the network guard's connection, the adapter and the
        // stream reader, from before the request is prepared.
        let started = Instant::now();
        let request = AiRequest {
            feature: Feature::Command,
            profile: profile.clone(),
            model: None,
            system: String::new(),
            instruction: "Reply with the single word: ok".into(),
            declared: Vec::new(),
            max_tokens: None,
            temperature: None,
            json_output: false,
        };
        let prepared = settings.gateway().prepare(request, &mut ()).unwrap();
        let key = settings.key_for(&profile).unwrap();
        let mut stream = settings.gateway().send(prepared, key).await.unwrap();
        loop {
            match stream.next().await.unwrap().unwrap() {
                Delta::Text(_) => break,
                _ => continue,
            }
        }
        through_aralo.push(started.elapsed());
        drop(stream);
        raw.push(raw_first_token(mock.port));
    }
    let (aralo, raw) = (median(through_aralo), median(raw));
    let added = aralo.saturating_sub(raw);
    eprintln!(
        "first token: {aralo:?} through Aralo, {raw:?} from a raw socket; Aralo adds {added:?}"
    );
    assert!(added < Duration::from_millis(30), "Aralo adds {added:?}");
}

#[test]
fn the_published_table_is_made_from_the_committed_reports() {
    let mut reports = Vec::new();
    let mut paths: Vec<PathBuf> = std::fs::read_dir(repo().join("conformance/reports"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    paths.sort();
    for path in &paths {
        let text = std::fs::read_to_string(path).unwrap();
        let report = ConformanceReport::from_json(&text)
            .unwrap_or_else(|problem| panic!("{}: {problem}", path.display()));
        assert!(
            !aralo_core::ai::looks_like_key(&text),
            "{} looks like it holds a key",
            path.display()
        );
        reports.push(report);
    }
    let planned = planned_endpoints(
        &std::fs::read_to_string(repo().join("conformance/endpoints.toml")).unwrap(),
    )
    .unwrap();
    let table = compatibility_table(&reports, &planned);
    let published = repo().join("docs/compatibility.md");
    if std::env::var_os("ARALO_UPDATE_COMPATIBILITY").is_some() {
        std::fs::write(&published, &table).unwrap();
    }
    assert_eq!(
        std::fs::read_to_string(&published).unwrap_or_default(),
        table,
        "docs/compatibility.md is not what the reports make; run `make compatibility`"
    );
}

#[test]
fn the_nightly_workflow_passes_every_endpoints_key() {
    let planned = planned_endpoints(
        &std::fs::read_to_string(repo().join("conformance/endpoints.toml")).unwrap(),
    )
    .unwrap();
    let workflow =
        std::fs::read_to_string(repo().join(".github/workflows/conformance-nightly.yml")).unwrap();
    for endpoint in &planned {
        if let Some(variable) = &endpoint.key_env {
            assert!(
                workflow.contains(&format!("{variable}: ${{{{ secrets.{variable} }}}}")),
                "the nightly workflow does not pass {variable}, for {}",
                endpoint.name
            );
        }
    }
}

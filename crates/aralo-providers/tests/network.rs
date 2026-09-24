//! The adapter against real sockets on 127.0.0.1, through the network guard
//! in local-only mode: a cancel closes the connection at once, and local
//! servers are found only where one answers.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use aralo_ai::guard::NetworkGuard;
use aralo_ai::{AdapterKind, AiRequest, Delta, Feature, Gateway, Policy, Profile, Transport};
use aralo_providers::local::detect_on;
use aralo_providers::{LocalServer, LocalServerKind};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

async fn read_request(socket: &mut TcpStream) -> Option<Vec<u8>> {
    let mut request = Vec::new();
    let mut buffer = [0u8; 4096];
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        match socket.read(&mut buffer).await {
            Ok(0) | Err(_) => return None,
            Ok(read) => request.extend_from_slice(&buffer[..read]),
        }
    }
    Some(request)
}

/// Answers every connection with `response` and closes it.
async fn serve(response: &'static [u8]) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                if read_request(&mut socket).await.is_some() {
                    let _ = socket.write_all(response).await;
                }
            });
        }
    });
    port
}

/// Accepts connections and never answers.
async fn silent() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            held.push(socket);
        }
    });
    port
}

/// A port nothing listens on.
async fn closed() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    listener.local_addr().unwrap().port()
}

fn chunk(data: &str) -> Vec<u8> {
    format!("{:x}\r\n{data}\r\n", data.len()).into_bytes()
}

#[tokio::test]
async fn dropping_the_stream_closes_the_socket() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address: SocketAddr = listener.local_addr().unwrap();
    let (closed_tx, closed_rx) = oneshot::channel::<Instant>();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        read_request(&mut socket).await.unwrap();
        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n",
            )
            .await
            .unwrap();
        let event = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"The\"}}]}\n\n";
        socket.write_all(&chunk(event)).await.unwrap();
        // The model keeps thinking. The only way this read ends is the client
        // closing its end.
        let mut buffer = [0u8; 64];
        loop {
            match socket.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
        }
        let _ = closed_tx.send(Instant::now());
    });

    let guard: Arc<dyn Transport> = Arc::new(NetworkGuard::new(true).unwrap());
    let mut gateway = Gateway::new(
        Policy {
            ai_enabled: true,
            local_only: true,
            allowed_hosts: None,
        },
        guard,
    );
    aralo_providers::register_all(&mut gateway);
    let prepared = gateway
        .prepare(
            AiRequest {
                feature: Feature::Block,
                profile: Profile {
                    name: "ollama".into(),
                    adapter: AdapterKind::OpenAiCompat,
                    base_url: format!("http://{address}/v1"),
                    headers: Vec::new(),
                    default_model: "llama3.2:3b".into(),
                    key_ref: None,
                },
                model: None,
                system: "s".into(),
                instruction: "i".into(),
                declared: Vec::new(),
                max_tokens: None,
                temperature: None,
                json_output: false,
            },
            &mut (),
        )
        .unwrap();

    let mut stream = gateway.send(prepared, None).await.unwrap();
    let first = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("the first token arrives");
    assert_eq!(first.unwrap().unwrap(), Delta::Text("The".into()));

    let cancelled = Instant::now();
    drop(stream);
    let closed_at = tokio::time::timeout(Duration::from_secs(2), closed_rx)
        .await
        .expect("the server saw the connection close within two seconds of the cancel")
        .unwrap();
    assert!(closed_at.duration_since(cancelled) < Duration::from_secs(2));
}

#[tokio::test]
async fn local_servers_are_found_only_where_a_model_list_answers() {
    let ollama = serve(
        b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: 97\r\n\r\n{\"object\":\"list\",\"data\":[{\"id\":\"llama3.2:3b\",\"object\":\"model\",\"created\":1,\"owned_by\":\"library\"}]}",
    )
    .await;
    let web_page = serve(
        b"HTTP/1.1 404 Not Found\r\ncontent-type: text/html\r\nconnection: close\r\ncontent-length: 9\r\n\r\nnot found",
    )
    .await;
    let other_json = serve(
        b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: 11\r\n\r\n{\"ok\":true}",
    )
    .await;
    let hung = silent().await;
    let nothing = closed().await;

    let guard = NetworkGuard::new(true).unwrap();
    let started = Instant::now();
    let found = detect_on(
        &guard,
        &[
            (LocalServerKind::Ollama, ollama),
            (LocalServerKind::LmStudio, web_page),
            (LocalServerKind::LlamaCpp, hung),
            (LocalServerKind::Vllm, nothing),
            (LocalServerKind::LlamaCpp, other_json),
        ],
        Duration::from_millis(500),
    )
    .await;
    let took = started.elapsed();

    assert_eq!(
        found,
        [LocalServer {
            kind: LocalServerKind::Ollama,
            base_url: format!("http://127.0.0.1:{ollama}/v1"),
            models: vec!["llama3.2:3b".into()],
        }]
    );
    assert!(
        took < Duration::from_millis(1500),
        "the probes run side by side, so the hung port costs one timeout: {took:?}"
    );
    let profile = found[0].profile();
    assert_eq!(profile.default_model, "llama3.2:3b");
    assert_eq!(profile.key_ref, None);
}

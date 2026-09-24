//! The `openai_compat` adapter: chat completions with server-sent events.
//!
//! One adapter covers OpenAI and everything that copies its API: OpenRouter,
//! Groq, Together, Mistral, DeepSeek, xAI, LiteLLM, company proxies, and the
//! local servers (Ollama, LM Studio, llama.cpp, vLLM).
//!
//! The copies differ in small ways, and the stream reader is lenient where
//! they do:
//!
//! - Usage arrives in a last chunk with no choices when `stream_options`
//!   asks for it, in `x_groq.usage` from Groq, and in every chunk from a
//!   server that reports running totals. The last report wins, and it is
//!   passed on once, just before [`Delta::Done`].
//! - The finish reason and `data: [DONE]` may come in either order relative
//!   to usage, and a server may close the body without `[DONE]`. A body that
//!   ends before any finish reason is a cut stream and an error.
//! - OpenRouter sends `: OPENROUTER PROCESSING` comments, and reports an error
//!   that happens mid-stream as a chunk with an `error` object.
//! - Reasoning models may stream `reasoning` or `reasoning_content` beside
//!   `content`. Only `content` is output.
//! - A server that ignores `stream: true` and answers with one JSON body is
//!   read as a stream of one piece.

use std::collections::VecDeque;

use aralo_ai::{
    Adapter, AdapterKind, AiError, BoxFuture, Call, Delta, DeltaStream, HttpRequest, HttpResponse,
    Method, Profile, Role, Secret, StopReason, Transport, Usage,
};
use serde::{Deserialize, Serialize};

use crate::http::{self, MAX_JSON_BODY};
use crate::sse::{SseEvent, SseParser};

#[derive(Debug, Clone, Copy, Default)]
pub struct OpenAiCompat;

impl OpenAiCompat {
    pub fn new() -> Self {
        Self
    }
}

impl Adapter for OpenAiCompat {
    fn kind(&self) -> AdapterKind {
        AdapterKind::OpenAiCompat
    }

    fn chat_stream<'a>(
        &'a self,
        http: &'a dyn Transport,
        call: Call<'a>,
    ) -> BoxFuture<'a, Result<Box<dyn DeltaStream>, AiError>> {
        Box::pin(async move {
            let response = http.send(chat_request(call)?).await?;
            if !(200..300).contains(&response.status) {
                return Err(http::status_error(response).await);
            }
            open_stream(response).await
        })
    }

    fn list_models<'a>(
        &'a self,
        http: &'a dyn Transport,
        profile: &'a Profile,
        key: Option<&'a Secret>,
    ) -> BoxFuture<'a, Result<Vec<String>, AiError>> {
        Box::pin(list_models(http, profile, key))
    }
}

/// The HTTP request for one chat: `POST {base}/chat/completions`, asking for
/// a stream with usage at its end.
pub fn chat_request(call: Call<'_>) -> Result<HttpRequest, AiError> {
    let request = call.request;
    let mut messages = Vec::with_capacity(request.messages.len() + 1);
    if !request.system.is_empty() {
        messages.push(WireMessage {
            role: "system",
            content: &request.system,
        });
    }
    messages.extend(request.messages.iter().map(|message| WireMessage {
        role: match message.role {
            Role::User => "user",
            Role::Assistant => "assistant",
        },
        content: &message.content,
    }));
    let body = serde_json::to_vec(&WireRequest {
        model: &request.model,
        messages,
        stream: true,
        stream_options: StreamOptions {
            include_usage: true,
        },
        max_tokens: request.max_tokens,
        temperature: request.temperature,
        response_format: request.json_output.then_some(ResponseFormat {
            kind: "json_object",
        }),
    })
    .map_err(|error| AiError::Protocol(format!("the request could not be written: {error}")))?;

    Ok(HttpRequest {
        method: Method::Post,
        url: http::join(&call.profile.base_url, "chat/completions"),
        headers: headers(
            call.profile,
            call.key,
            &[
                ("content-type", "application/json"),
                ("accept", "text/event-stream"),
            ],
        ),
        body: Some(body),
    })
}

/// `GET {base}/models`, sorted and without repeats.
pub async fn list_models(
    http: &dyn Transport,
    profile: &Profile,
    key: Option<&Secret>,
) -> Result<Vec<String>, AiError> {
    let mut response = http
        .send(HttpRequest {
            method: Method::Get,
            url: http::join(&profile.base_url, "models"),
            headers: headers(profile, key, &[("accept", "application/json")]),
            body: None,
        })
        .await?;
    if !(200..300).contains(&response.status) {
        return Err(http::status_error(response).await);
    }
    let body = http::read_body(&mut response, MAX_JSON_BODY).await?;
    let list: ModelList = serde_json::from_slice(&body).map_err(|error| {
        AiError::Protocol(format!("the model list is not the expected JSON: {error}"))
    })?;
    let mut ids: Vec<String> = list.data.into_iter().map(|model| model.id).collect();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

/// The adapter's headers, then the key, then the profile's own. A header the
/// profile sets replaces the adapter's of the same name, so a proxy that
/// wants its own `Authorization` gets it.
fn headers(profile: &Profile, key: Option<&Secret>, own: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut headers: Vec<(String, String)> = own
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    if let Some(key) = key.filter(|key| !key.expose().trim().is_empty()) {
        headers.push((
            "authorization".into(),
            format!("Bearer {}", key.expose().trim()),
        ));
    }
    for (name, value) in &profile.headers {
        headers.retain(|(existing, _)| !existing.eq_ignore_ascii_case(name));
        headers.push((name.clone(), value.clone()));
    }
    headers
}

async fn open_stream(response: HttpResponse) -> Result<Box<dyn DeltaStream>, AiError> {
    let json = response
        .header("content-type")
        .is_some_and(|value| value.to_ascii_lowercase().starts_with("application/json"));
    let mut stream = ChatStream {
        response,
        parser: SseParser::new(),
        queue: VecDeque::new(),
        usage: None,
        finish: None,
        ended: false,
        error: None,
    };
    if json {
        let body = http::read_body(&mut stream.response, MAX_JSON_BODY).await?;
        stream.read_whole(&body)?;
    }
    Ok(Box::new(stream))
}

struct ChatStream {
    /// Held until the stream is dropped: dropping it closes the connection.
    response: HttpResponse,
    parser: SseParser,
    queue: VecDeque<Delta>,
    /// The latest usage report, passed on just before `Done`.
    usage: Option<Usage>,
    finish: Option<StopReason>,
    /// No more is read from the body. What is queued is still handed out.
    ended: bool,
    /// What stopped the stream, handed out after the text that came before
    /// it in the same chunk.
    error: Option<AiError>,
}

impl DeltaStream for ChatStream {
    fn next(&mut self) -> BoxFuture<'_, Option<Result<Delta, AiError>>> {
        Box::pin(async move {
            loop {
                if let Some(delta) = self.queue.pop_front() {
                    return Some(Ok(delta));
                }
                if let Some(error) = self.error.take() {
                    return Some(Err(error));
                }
                if self.ended {
                    return None;
                }
                if let Err(error) = self.read_more().await {
                    self.ended = true;
                    self.error = Some(error);
                }
            }
        })
    }
}

impl ChatStream {
    async fn read_more(&mut self) -> Result<(), AiError> {
        match self.response.body.next_chunk().await? {
            Some(chunk) => {
                let events = self.parser.feed(&chunk).map_err(|_| {
                    AiError::Protocol("a stream line was longer than any chat stream sends".into())
                })?;
                for event in events {
                    self.event(event)?;
                    if self.ended {
                        break;
                    }
                }
                Ok(())
            }
            None => {
                if let Some(event) = self.parser.finish() {
                    self.event(event)?;
                }
                if self.ended {
                    return Ok(());
                }
                if self.finish.is_none() {
                    return Err(AiError::Protocol(
                        "the stream ended before the model finished".into(),
                    ));
                }
                self.end();
                Ok(())
            }
        }
    }

    fn event(&mut self, event: SseEvent) -> Result<(), AiError> {
        let data = event.data.trim();
        if data == "[DONE]" {
            self.end();
            return Ok(());
        }
        if data.is_empty() {
            return Ok(());
        }
        let chunk: Chunk = serde_json::from_str(data).map_err(|error| {
            AiError::Protocol(format!("a stream event is not the expected JSON: {error}"))
        })?;
        self.chunk(chunk, |choice| choice.delta)
    }

    /// An answer that did not stream: one JSON completion.
    fn read_whole(&mut self, body: &[u8]) -> Result<(), AiError> {
        let chunk: Chunk = serde_json::from_slice(body).map_err(|error| {
            AiError::Protocol(format!("the answer is not the expected JSON: {error}"))
        })?;
        self.chunk(chunk, |choice| choice.message)?;
        self.end();
        Ok(())
    }

    fn chunk(
        &mut self,
        chunk: Chunk,
        content: impl Fn(Choice) -> Option<WireDelta>,
    ) -> Result<(), AiError> {
        if let Some(error) = chunk.error {
            return Err(stream_error(&error));
        }
        if let Some(usage) = chunk.usage.or(chunk.x_groq.and_then(|groq| groq.usage)) {
            self.usage = Some(usage.into());
        }
        for choice in chunk.choices.into_iter().filter(|choice| choice.index == 0) {
            if let Some(reason) = choice.finish_reason.clone() {
                self.finish = Some(stop_reason(&reason));
            }
            if let Some(text) = content(choice).and_then(|delta| delta.content) {
                if !text.is_empty() {
                    self.queue.push_back(Delta::Text(text));
                }
            }
        }
        Ok(())
    }

    /// The model is done: usage, then `Done`, and nothing more is read.
    fn end(&mut self) {
        if self.ended {
            return;
        }
        self.ended = true;
        if let Some(usage) = self.usage.take() {
            self.queue.push_back(Delta::Usage(usage));
        }
        self.queue
            .push_back(Delta::Done(self.finish.take().unwrap_or(StopReason::End)));
    }
}

fn stop_reason(reason: &str) -> StopReason {
    match reason {
        "stop" | "eos" | "end_turn" => StopReason::End,
        "length" | "max_tokens" => StopReason::Length,
        other => StopReason::Other(other.to_owned()),
    }
}

/// An error reported inside a stream that had already begun. A numeric code
/// in the HTTP range is kept as a status; nothing is retried either way,
/// because text may already be on screen.
fn stream_error(error: &serde_json::Value) -> AiError {
    let message = http::message_in(error)
        .unwrap_or("the endpoint reported an error mid-stream")
        .to_owned();
    let code = error.get("code").and_then(|code| {
        code.as_u64()
            .or_else(|| code.as_str().and_then(|text| text.parse().ok()))
    });
    match code {
        Some(status @ 400..=599) => AiError::Status {
            status: status as u16,
            message,
            retry_after: None,
        },
        _ => AiError::Protocol(message),
    }
}

#[derive(Serialize)]
struct WireRequest<'a> {
    model: &'a str,
    messages: Vec<WireMessage<'a>>,
    stream: bool,
    stream_options: StreamOptions,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<ResponseFormat>,
}

#[derive(Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    kind: &'static str,
}

#[derive(Serialize)]
struct WireMessage<'a> {
    role: &'static str,
    content: &'a str,
}

#[derive(Serialize)]
struct StreamOptions {
    include_usage: bool,
}

#[derive(Deserialize)]
struct Chunk {
    #[serde(default)]
    choices: Vec<Choice>,
    usage: Option<WireUsage>,
    x_groq: Option<Groq>,
    error: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct Choice {
    #[serde(default)]
    index: u32,
    delta: Option<WireDelta>,
    message: Option<WireDelta>,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct WireDelta {
    content: Option<String>,
}

#[derive(Deserialize)]
struct Groq {
    usage: Option<WireUsage>,
}

#[derive(Deserialize)]
struct WireUsage {
    prompt_tokens: Option<u64>,
    completion_tokens: Option<u64>,
    prompt_tokens_details: Option<PromptDetails>,
}

#[derive(Deserialize)]
struct PromptDetails {
    cached_tokens: Option<u64>,
}

impl From<WireUsage> for Usage {
    fn from(usage: WireUsage) -> Self {
        Self {
            input_tokens: usage.prompt_tokens.unwrap_or(0),
            output_tokens: usage.completion_tokens.unwrap_or(0),
            cached_input_tokens: usage
                .prompt_tokens_details
                .and_then(|details| details.cached_tokens)
                .unwrap_or(0),
        }
    }
}

#[derive(Deserialize)]
struct ModelList {
    data: Vec<ModelEntry>,
}

#[derive(Deserialize)]
struct ModelEntry {
    id: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use aralo_ai::{ChatRequest, Message};

    fn profile(headers: Vec<(String, String)>) -> Profile {
        Profile {
            name: "openai".into(),
            adapter: AdapterKind::OpenAiCompat,
            base_url: "https://api.openai.com/v1/".into(),
            headers,
            default_model: "gpt-4o-mini".into(),
            key_ref: Some("openai".into()),
        }
    }

    fn chat(system: &str) -> ChatRequest {
        ChatRequest {
            model: "gpt-4o-mini".into(),
            system: system.into(),
            messages: vec![Message {
                role: Role::User,
                content: "Fix: teh".into(),
            }],
            max_tokens: Some(100),
            temperature: None,
            json_output: false,
        }
    }

    #[test]
    fn the_request_asks_for_a_stream_with_usage() {
        let profile = profile(Vec::new());
        let key = Secret::new("sk-test");
        let request = chat("You fix spelling.");
        let http = chat_request(Call {
            profile: &profile,
            key: Some(&key),
            request: &request,
        })
        .unwrap();
        assert_eq!(http.method, Method::Post);
        assert_eq!(http.url, "https://api.openai.com/v1/chat/completions");
        assert!(http
            .headers
            .contains(&("authorization".into(), "Bearer sk-test".into())));
        let body: serde_json::Value = serde_json::from_slice(&http.body.unwrap()).unwrap();
        assert_eq!(
            body,
            serde_json::json!({
                "model": "gpt-4o-mini",
                "messages": [
                    {"role": "system", "content": "You fix spelling."},
                    {"role": "user", "content": "Fix: teh"},
                ],
                "stream": true,
                "stream_options": {"include_usage": true},
                "max_tokens": 100,
            })
        );
    }

    #[test]
    fn json_mode_asks_for_a_json_object() {
        let profile = profile(Vec::new());
        let request = ChatRequest {
            json_output: true,
            ..chat("")
        };
        let http = chat_request(Call {
            profile: &profile,
            key: None,
            request: &request,
        })
        .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&http.body.unwrap()).unwrap();
        assert_eq!(
            body["response_format"],
            serde_json::json!({"type": "json_object"})
        );
    }

    #[test]
    fn no_key_and_no_system_prompt_send_neither() {
        let profile = profile(Vec::new());
        let request = chat("");
        let blank = Secret::new("  ");
        for key in [None, Some(&blank)] {
            let http = chat_request(Call {
                profile: &profile,
                key,
                request: &request,
            })
            .unwrap();
            assert!(http.headers.iter().all(|(name, _)| name != "authorization"));
            let body: serde_json::Value = serde_json::from_slice(&http.body.unwrap()).unwrap();
            assert_eq!(body["messages"].as_array().unwrap().len(), 1);
        }
    }

    #[test]
    fn a_profile_header_replaces_the_adapters() {
        let profile = profile(vec![
            ("Authorization".into(), "Basic cHJveHk=".into()),
            ("HTTP-Referer".into(), "https://aralo.app".into()),
        ]);
        let key = Secret::new("sk-test");
        let request = chat("s");
        let http = chat_request(Call {
            profile: &profile,
            key: Some(&key),
            request: &request,
        })
        .unwrap();
        let auth: Vec<_> = http
            .headers
            .iter()
            .filter(|(name, _)| name.eq_ignore_ascii_case("authorization"))
            .collect();
        assert_eq!(auth, [&("Authorization".into(), "Basic cHJveHk=".into())]);
        assert!(http
            .headers
            .contains(&("HTTP-Referer".into(), "https://aralo.app".into())));
    }

    #[test]
    fn stop_reasons_are_mapped() {
        assert_eq!(stop_reason("stop"), StopReason::End);
        assert_eq!(stop_reason("length"), StopReason::Length);
        assert_eq!(
            stop_reason("content_filter"),
            StopReason::Other("content_filter".into())
        );
    }
}

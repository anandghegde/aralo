# Writing a provider adapter

An adapter speaks one provider's API. Aralo ships one, `openai_compat`, which
covers OpenAI and every API that copies it (OpenRouter, Groq, Together,
LiteLLM, and the local servers: Ollama, LM Studio, llama.cpp, vLLM). A second
kind, `anthropic`, is named in `AdapterKind` for Anthropic's Messages API.

The rules below are what review holds an adapter to. The design is in
[the architecture](architecture.md#provider-adapters) and
[ADR-0007](adr/0007-ai-gateway-and-network-guard.md).

## Where it goes

Adapters live in `crates/aralo-providers`. The crate depends on `aralo-ai`
for the traits and on **no HTTP client**. An adapter reaches the network only
through the `Transport` the gateway hands it, and every transport is built by
the network guard (`crates/guard`), which is what makes local-only mode mean
no network. `scripts/check-deps.sh` (check f) fails the build if `reqwest`
is named anywhere outside the guard, and cargo-deny keeps it out of every
other crate's dependencies.

Register the adapter in `aralo_providers::register_all`, and add its kind to
`AdapterKind` in `crates/aralo-ai/src/model.rs`.

## The trait

```rust
pub trait Adapter: Send + Sync {
    fn kind(&self) -> AdapterKind;

    fn chat_stream<'a>(
        &'a self,
        http: &'a dyn Transport,
        call: Call<'a>,
    ) -> BoxFuture<'a, Result<Box<dyn DeltaStream>, AiError>>;

    fn list_models<'a>(
        &'a self,
        http: &'a dyn Transport,
        profile: &'a Profile,
        key: Option<&'a Secret>,
    ) -> BoxFuture<'a, Result<Vec<String>, AiError>>;
}
```

- `chat_stream` returns once the endpoint has accepted the request. A status
  it refused with is `AiError::Status`, with `Retry-After` when it sent one:
  the gateway retries 429 and 5xx answers before the first token, and never
  after.
- The stream yields `Delta::Text`, `Delta::Usage` and one `Delta::Done`, then
  `None`. Dropping it must close the connection.
- `Delta::Text` is literal. Nothing downstream parses model output for
  placeholders or scripts, and an adapter must not either.
- The key is a `Secret`. It goes into a header and nowhere else: not an
  error message, not a log line, not a `Debug` output. The key-leak scan in
  CI (`scripts/key-leak-scan.sh`) runs the whole test suite with a canary key
  and fails if the key lands in any file.
- A header set on the profile replaces the adapter's header of the same name.

## Streams

Use `sse::SseParser` for server-sent events. It gives the same events
wherever the chunk boundaries fall, and refuses a line over 1 MiB. Be lenient
where the copies of an API differ (usage reported in different places,
comment lines, a missing `[DONE]` after a finish reason) and strict where
leniency would hide a failure: a body that ends before any finish reason, a
chunk that is not JSON, an error object mid-stream. The table in
[the architecture](architecture.md#provider-adapters) lists every case.

## Tests an adapter needs

- **Transcripts.** Recorded response bodies in `fixtures/sse/`, replayed
  through the gateway in 1-byte, 7-byte and whole-body chunks
  (`crates/aralo-providers/tests/transcripts.rs`). Check the text, the usage
  metered and the stop reason. `fixtures/sse/README.md` says how to record
  one with `tests/live.rs`.
- **Network.** Against a real listener on 127.0.0.1, through the guard in
  local-only mode (`tests/network.rs`): the stream closes within two seconds
  of being dropped.
- **Conformance.** `aralo conformance --profile <name>` runs the protocol
  checks against a live endpoint. A new endpoint's report goes into
  [the compatibility page](compatibility.md).

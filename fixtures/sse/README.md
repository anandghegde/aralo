# Chat stream transcripts

Response bodies from OpenAI-compatible chat endpoints, byte for byte as they
come off the wire. `crates/aralo-providers/tests/transcripts.rs` replays each
one through the `openai_compat` adapter, cut into chunks of 1 byte, 7 bytes
and the whole body, and checks the text, the usage and the stop reason that
come out.

| File | What it shows |
| --- | --- |
| `openai.sse` | A role-only first chunk, the finish reason in its own chunk, then usage with cached tokens in a chunk with no choices |
| `openrouter.sse` | `: OPENROUTER PROCESSING` comments, and usage with cost fields in a chunk after the finish reason |
| `groq.sse` | Usage in `x_groq.usage` on the chunk that carries the finish reason |
| `ollama.sse` | Ollama's `/v1` endpoint: usage without cache details after the finish reason |
| `surplus-reasoning.sse` | Recorded from Surplus Intelligence (`deepseek-v4.1-flash`): `reasoning_content` before the answer, and usage twice, upstream then billed |
| `openrouter-error-midstream.sse` | An upstream failure reported as a chunk with an `error` object after text has streamed |
| `cut-off.sse` | A body that ends before any finish reason or `[DONE]` |
| `not-json.sse` | A proxy's HTML page where a JSON chunk should be |

## Where they come from

`surplus-reasoning.sse` is a real recording, made with the live test below. The four provider transcripts were written by hand from each provider's
documented streaming format and the field order their SDKs parse. They were
not captured from live endpoints, because no keys were available when task
4.2 landed. Replace each with a real recording when you can:

```sh
ARALO_LIVE_BASE_URL=https://api.groq.com/openai/v1 \
ARALO_LIVE_MODEL=llama-3.1-8b-instant \
ARALO_LIVE_KEY=... \
ARALO_LIVE_RECORD=fixtures/sse/groq.sse \
cargo test -p aralo-providers --test live -- --ignored --nocapture
```

The test asks the model to repeat `The quick brown fox — ünïcödé ✓`, which
is the text `transcripts.rs` expects. A model may not repeat it exactly, and
its token counts will differ, so after recording, update that file's row in
`each_provider_streams_text_then_usage_then_done`. A recording
holds only the response body: no headers, no key. Check a new one for
anything account-specific before committing it.

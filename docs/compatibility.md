# Compatibility

<!-- Made by `aralo conformance table` from conformance/reports/ and conformance/endpoints.toml; `make compatibility` makes it again. Edit those, not this. -->

How each endpoint did in Aralo's conformance suite, `aralo conformance` (see `conformance/README.md`). An endpoint conforms when no required protocol check fails; usage and JSON output are reported and not required. A check is skipped when there was nothing to check, and the notes under the table say why. The evaluation column counts the snippet editor's actions the model did as expected, out of the set in `conformance/evaluation.toml`: it measures the model, not the endpoint, and never decides conformance.

No endpoint has a report yet.

## Not run live yet

These endpoints are in `conformance/endpoints.toml` and have no report. The nightly workflow runs one once its key is a repository secret. Where a recorded stream is named, the mock server replays its framing through the protocol checks on every push.

| Endpoint | Base URL | Model | Runs | Recorded stream |
| --- | --- | --- | --- | --- |
| Ollama | `http://127.0.0.1:11434/v1` | `qwen2.5:0.5b` | every push, in a container | `fixtures/sse/ollama.sse` |
| llama.cpp | `http://127.0.0.1:8080/v1` | `qwen2.5-0.5b` | every push, in a container |  |
| OpenAI | `https://api.openai.com/v1` | `gpt-4o-mini` | nightly, with `ARALO_CONFORMANCE_OPENAI_KEY` | `fixtures/sse/openai.sse` |
| OpenRouter | `https://openrouter.ai/api/v1` | `openai/gpt-4o-mini` | nightly, with `ARALO_CONFORMANCE_OPENROUTER_KEY` | `fixtures/sse/openrouter.sse` |
| Groq | `https://api.groq.com/openai/v1` | `llama-3.1-8b-instant` | nightly, with `ARALO_CONFORMANCE_GROQ_KEY` | `fixtures/sse/groq.sse` |
| Google Gemini | `https://generativelanguage.googleapis.com/v1beta/openai` | `gemini-2.5-flash` | nightly, with `ARALO_CONFORMANCE_GEMINI_KEY` |  |
| Mistral | `https://api.mistral.ai/v1` | `mistral-small-latest` | nightly, with `ARALO_CONFORMANCE_MISTRAL_KEY` |  |
| DeepSeek | `https://api.deepseek.com/v1` | `deepseek-chat` | nightly, with `ARALO_CONFORMANCE_DEEPSEEK_KEY` |  |

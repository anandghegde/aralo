# Compatibility

<!-- Made by `aralo conformance table` from conformance/reports/ and conformance/endpoints.toml; `make compatibility` makes it again. Edit those, not this. -->

How each endpoint did in Aralo's conformance suite, `aralo conformance` (see `conformance/README.md`). An endpoint conforms when no required protocol check fails; usage and JSON output are reported and not required. A check is skipped when there was nothing to check, and the notes under the table say why. The evaluation column counts the snippet editor's actions the model did as expected, out of the set in `conformance/evaluation.toml`: it measures the model, not the endpoint, and never decides conformance.

| Endpoint | Model | Conforms | Models | Stream | System prompt | Cancel | Unknown model | Wrong key | Usage | JSON | First token | Evaluation | Run |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Ollama | `qwen2.5:0.5b` | yes | pass | pass | pass | pass | pass | skip | pass | pass | 1186 ms | 1 of 5 | 2026-09-25 |
| llama.cpp | `qwen2.5-0.5b` | yes | pass | pass | pass | pass | skip | skip | pass | pass | 354 ms | 0 of 5 | 2026-09-25 |

## What did not pass, and why

- Ollama, `qwen2.5:0.5b`: **wrong_key** skipped: the profile sends no key
- Ollama, `qwen2.5:0.5b`: evaluation case “tone: formal”: does not have “friday”
- Ollama, `qwen2.5:0.5b`: evaluation case “tone: shorter”: does not have “friday”
- Ollama, `qwen2.5:0.5b`: evaluation case “date macro kept”: does not have “shipped”; does not have “arrive”; dropped {{date: %d %B}}; dropped {{field: days}}; added {{name}}
- Ollama, `qwen2.5:0.5b`: evaluation case “date macro written”: has none of “{{date”
- llama.cpp, `qwen2.5-0.5b`: **unknown_model** skipped: the endpoint answered anyway: it serves the model it has, whatever the name
- llama.cpp, `qwen2.5-0.5b`: **wrong_key** skipped: the profile sends no key
- llama.cpp, `qwen2.5-0.5b`: evaluation case “grammar”: does not have “your order”; does not have “tomorrow”
- llama.cpp, `qwen2.5-0.5b`: evaluation case “tone: formal”: still has “hey”; still has “cheers”
- llama.cpp, `qwen2.5-0.5b`: evaluation case “tone: shorter”: is 98% of the text's length, over 70%
- llama.cpp, `qwen2.5-0.5b`: evaluation case “date macro kept”: does not have “arrive”
- llama.cpp, `qwen2.5-0.5b`: evaluation case “date macro written”: has none of “{{date”

## Not run live yet

These endpoints are in `conformance/endpoints.toml` and have no report. The nightly workflow runs one once its key is a repository secret. Where a recorded stream is named, the mock server replays its framing through the protocol checks on every push.

| Endpoint | Base URL | Model | Runs | Recorded stream |
| --- | --- | --- | --- | --- |
| OpenAI | `https://api.openai.com/v1` | `gpt-4o-mini` | nightly, with `ARALO_CONFORMANCE_OPENAI_KEY` | `fixtures/sse/openai.sse` |
| OpenRouter | `https://openrouter.ai/api/v1` | `openai/gpt-4o-mini` | nightly, with `ARALO_CONFORMANCE_OPENROUTER_KEY` | `fixtures/sse/openrouter.sse` |
| Groq | `https://api.groq.com/openai/v1` | `llama-3.1-8b-instant` | nightly, with `ARALO_CONFORMANCE_GROQ_KEY` | `fixtures/sse/groq.sse` |
| Google Gemini | `https://generativelanguage.googleapis.com/v1beta/openai` | `gemini-2.5-flash` | nightly, with `ARALO_CONFORMANCE_GEMINI_KEY` |  |
| Mistral | `https://api.mistral.ai/v1` | `mistral-small-latest` | nightly, with `ARALO_CONFORMANCE_MISTRAL_KEY` |  |
| DeepSeek | `https://api.deepseek.com/v1` | `deepseek-chat` | nightly, with `ARALO_CONFORMANCE_DEEPSEEK_KEY` |  |

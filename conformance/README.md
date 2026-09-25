# Conformance

`aralo conformance` checks that an endpoint works with Aralo, and says how
well its model does Aralo's work (plan 7.4). The results make the
compatibility table in [`docs/compatibility.md`](../docs/compatibility.md).

| File | What it is |
| --- | --- |
| `endpoints.toml` | The endpoints the suite is run against, with the model, when CI runs each, and the secret that holds its key |
| `evaluation.toml` | The evaluation set: the snippet editor's actions on fixed texts, and what a good answer has in it. Compiled into the core |
| `reports/` | One JSON report per endpoint and model, as `aralo conformance --report` writes it |

The checks themselves are in `crates/aralo-core/src/ai/conformance.rs`.

## Running it

The suite runs against a saved profile, with its key from the keychain, and
only while AI is on:

```sh
aralo ai on
aralo ai add Ollama --preset ollama --model qwen2.5:0.5b
aralo conformance --profile Ollama --evaluation --report conformance/reports/ollama.json
```

It prints each check with what it found and exits 1 if a required one failed.
`--report -` writes the report to standard output instead of a file.

## Two layers

**Protocol checks** decide whether an endpoint conforms. Each one is named in
the report, so a failure says exactly what broke:

| Check | Required | Passes when |
| --- | --- | --- |
| `models` | yes | `GET /models` lists at least one model |
| `stream` | yes | an answer arrives in more than one piece and ends with a stop reason |
| `system_prompt` | yes | the model does what the system prompt asks, which the user message never mentions, in one of three tries |
| `cancel` | yes | a stream dropped after its first piece leaves the endpoint answering the next request |
| `unknown_model` | yes | a model the endpoint does not have is refused with a 4xx Aralo can read |
| `wrong_key` | yes | a wrong key is refused with a 4xx Aralo can read |
| `usage` | no | the stream reports token counts, which metering needs |
| `json_output` | no | JSON mode answers with a JSON object |

An error check is skipped, not failed, when there is nothing to check: a
profile with no key has no wrong key to send, and a local server that serves
the one model it has loaded, whatever the name, has no error to show.

**The evaluation set**, with `--evaluation`, runs the editor's actions
(proofread, make it formal, make it shorter, draft) on the texts in
`evaluation.toml`, through the same code the editor uses, and checks each
answer: a grammar fix made, a tone changed, a date placeholder kept exactly
and written when asked for. It measures the model, not the endpoint, and never
decides conformance; small local models are expected to miss some. Grounded
replies join it in v1.

## Where it runs

- **Every push**, `cargo test`: a mock server replays the framing of each
  recorded stream in `fixtures/sse` through every protocol check and the
  evaluation set (`crates/aralo-core/tests/conformance.rs`). Mocks broken in
  one way each fail naming the check that caught them. The same mock measures
  what Aralo adds to the first token, which the plan holds under 30 ms.
- **Every push**, the `conformance` job in `.github/workflows/check.yml`: the
  protocol checks and the evaluation set against a real Ollama and a real
  llama.cpp server, each in a container with a small model. The reports are
  kept as the job's artifacts.
- **Nightly**, `.github/workflows/conformance-nightly.yml`: every endpoint in
  `endpoints.toml` whose key is set as a repository secret of the name
  `key_env` gives. An endpoint whose secret is not set is skipped.

## Reports and the table

A report names the endpoint by host and port, and says whether a key was sent.
It never holds the key, the rest of the address, or anything the user
configured beyond the model. Anything in an endpoint's error message that
looks like a key is replaced with `[redacted]`, and the test that checks the
table also refuses a committed report that looks like it holds one.

To publish a run, put its report in `reports/`, named after the endpoint and
model, and run `make compatibility`, which rebuilds `docs/compatibility.md`
from every report there and `endpoints.toml`. `cargo test` fails if the page
and the reports disagree. The table shows the latest report for each endpoint
and model.

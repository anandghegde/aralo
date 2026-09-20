# ADR-0007: One AI gateway and one network guard

- Status: Accepted
- Date: 2026-09-20

## Context

Aralo's AI features use the user's own key against the user's chosen
endpoint: an OpenAI-compatible service, a local server, or Anthropic. The PRD
makes four promises that touch this path:

- AI sends only the context a snippet or command declared (P4).
- Local-only mode means no network (P6).
- There are no Aralo servers (P7).
- Prompt injection cannot act: AI output is text only (P10).

If each feature made its own HTTP calls, each promise would have to be checked
in every feature.

## Decision

**One gateway.** Every AI feature calls one gateway in `aralo-ai`, and the
gateway is the only code that can reach a model. Features never see a
provider, a key or a URL. A request passes through fixed stages:

| Stage | Does |
| --- | --- |
| Policy check | Stops if the AI switch is off, if local-only mode is on and the profile's host is not loopback, or if a managed policy forbids the endpoint |
| Context assembly | Takes only the declared context kinds. Records a manifest of kind and size for the preview panel |
| Redaction (v1) | The slot exists now; on-device detectors arrive in v1 |
| Prompt framing | User context goes in delimited, labelled data blocks with an instruction to treat it as data |
| Network guard | See below |
| Adapter | Translates the neutral request to the provider's wire format and the stream back to neutral deltas |
| Stream and metering | Deltas go to the shell through `CoreEvents`. Token usage is added to per-feature, per-profile counters |
| Output | Plain text only |

**One network guard.** The guard is the only constructor of HTTP clients in
the workspace. It resolves the host and refuses non-loopback addresses in
local-only mode. A lint bans `reqwest::Client::new` everywhere else.

**Output is literal text.** Model output is inserted literally and never
parsed again. It cannot start a session, run a script or make a request.

## Consequences

- Each promise has one enforcement point and one test. A non-loopback profile
  fails in local-only mode. Model output containing `{{script}}` and `{{ai}}`
  is inserted verbatim. A snippet declaring `fillins` makes zero calls for the
  selection or the clipboard.
- One client type (`reqwest` with `rustls`) is what makes the guard
  enforceable.
- Sparkle's update check is the one documented exception to local-only mode.
- The complete list of hosts the app can contact is short: the user's
  endpoints, and GitHub for updates and signed data tables.
- New gateway stages have a place to go. Redaction in v1 and a compliance
  guard in v2 are new stages, not new paths.
- Cancel drops the HTTP request at once. Retries happen only before the first
  token: at most two, with backoff, on 429 and 5xx responses.
- Contributors cannot call `reqwest` directly, even for a quick probe. Every
  request goes through the guard. `/crates/aralo-ai/src/guard/` is protected
  by code owners.

## Alternatives rejected

- **Each feature or adapter builds its own HTTP client.** Local-only mode and
  the host list would then depend on every call site getting it right.
- **Letting model output flow back into the template evaluator**, for example
  so a model could emit placeholders. This is the path prompt injection would
  use. Output has no path back into the evaluator, the script runtime or the
  network.

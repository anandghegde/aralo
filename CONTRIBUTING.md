# Contributing to Aralo

Aralo is a free, open-source text expander: a Rust core behind a thin native
Swift shell on macOS. There are no accounts and no Aralo-run servers.
Contributions are welcome under the rules below.

By taking part you agree to the [Code of Conduct](CODE_OF_CONDUCT.md).
Security problems go through [SECURITY.md](SECURITY.md), never a public issue.

## Sign your commits (DCO)

Every commit must carry a Developer Certificate of Origin (DCO) sign-off.

```
git commit -s
```

This adds a `Signed-off-by: Your Name <you@example.com>` line. With it you
certify, under DCO 1.1, that you wrote the change or otherwise have the right
to submit it under the project's licence (Apache-2.0), and that you understand
the contribution and your sign-off are public and kept with the project. Read
the full text at <https://developercertificate.org/>.

Pull requests with unsigned commits are not merged. `git commit --amend -s`
fixes the last commit; `git rebase --signoff <base>` fixes a branch.

## Repository layout

```
aralo/
  Cargo.toml            workspace
  crates/
    aralo-engine/       buffer, matcher, case rules, undo record. No I/O, no logging
    aralo-snippet/      data model, front matter and group files, schema types
    aralo-template/     placeholder parser, evaluator, date formats, ExpansionPlan
    aralo-library/      folder store, watcher, SQLite index, merge, search fusion
    aralo-embed/        embedding model runtime, vector scan
    aralo-ai/           gateway, network guard, framing, metering, profiles
    aralo-providers/    openai_compat, anthropic; later gemini, responses
    aralo-import/       importers and the import report
    aralo-script/       rquickjs sandbox (v1)
    aralo-core/         facade: sessions, settings, policy, events
    aralo-ffi/          UniFFI exports, XCFramework build script
    aralo-cli/          aralo: expand, search, import, export, conformance, mcp (v1)
  apps/
    macos/              Xcode project: Aralo (app), AraloKit (tap, injector, AX), tests
    windows/            v1
    extension/          v1
  conformance/          protocol checks, evaluation set, report tooling
  data/                 compat/apps.toml, quirks/providers.toml, starter sets
  schemas/              JSON Schemas: snippet, group, policy, compat, plan
  fixtures/             import corpora, golden expansions, SSE transcripts
  docs/                 architecture, format spec, threat model, data flow, ADRs
```

Dependency direction is one-way. `aralo-engine`, `aralo-snippet` and
`aralo-template` are the pure base and depend on nothing above them.
`aralo-library` and `aralo-ai` sit above the base. `aralo-core` composes.
`aralo-ffi` and `aralo-cli` sit on `aralo-core`. CI checks the graph.

Design decisions are recorded in [docs/adr/](docs/adr/README.md). Read the
relevant ADR before proposing a change to something it covers.

## Building and testing

**Rust crates and the CLI.** These build and test on macOS, Linux and Windows.
You do not need Xcode or a Mac.

```
cargo test --workspace
cargo fmt --all
cargo clippy --workspace --all-targets
```

Most contributions, including provider adapters and importers, need nothing
more than this.

**The Mac app.** This needs a Mac with Xcode, plus `xcodegen` and `swiftlint`
(`brew install xcodegen swiftlint`). Run `make bootstrap` first. It builds the
XCFramework that the Swift package wraps and generates the Xcode project under
`apps/macos/`. Then `make app` builds the app, `make run` builds and launches
it, and `make test-swift` runs the Swift tests. `make help` lists the rest. The
app needs macOS 14 or later.

## Rules that CI enforces

**1. `aralo-engine` stays pure.** This crate sees keystrokes. It must never
gain a file, network, clock or logging dependency, directly or transitively.
CI inspects its dependency tree and fails the build if one appears. This is
what lets a reviewer verify "not a keylogger" by reading one small crate. Any
change to `aralo-engine`, to the event tap in `AraloKit`, or to the network
guard needs code-owner review (see `.github/CODEOWNERS`).

**2. Only the network guard constructs HTTP clients.** The guard lives in
`aralo-ai`. It is the single place that creates an HTTP client, and it refuses
non-loopback addresses in local-only mode. Only `aralo-ai` may depend on
`reqwest` (cargo-deny), only `crates/aralo-ai/src/guard/` may name it
(`scripts/check-deps.sh`), and clippy bans `reqwest::Client::new` and
`reqwest::Client::builder` everywhere else. If your code needs to make a
request, take a `Transport` from the gateway, as the provider adapters do.

**3. Secrets and content stay out of logs.** Logging uses `tracing` with a
field allow-list: IDs, counts, durations and error codes. Snippet bodies,
context and model output are never log fields. API keys live only in the
keychain, behind `SecretStore`.

**4. AI output is literal text.** Model output is inserted as plain text. Do
not add any path that parses it again, runs it, or lets it make a request.

**5. New dependencies are checked.** `cargo-deny` checks licences and
advisories. Explain in the pull request why a new dependency is needed.

## Tests

- Add tests with every change in behaviour.
- Template goldens: a snippet plus answers plus a fixed clock and locale gives
  an expected `ExpansionPlan`. **Every bug fixed gets one new fixture.**
- Matcher changes must hold against the property tests, which compare the
  matcher with a naive reference.
- AI gateway tests run against a mock server that replays recorded transcripts
  from `fixtures/sse/`. Remove secrets from any transcript you add.
- The security invariants are ordinary tests in the `check` workflow. A
  privacy regression blocks merge.
- Pull requests touching `aralo-engine`, `aralo-template` or `aralo-library`
  run benchmarks. A regression over 10% on `on_key` with 10,000 snippets
  fails.

## Commits and pull requests

- Keep pull requests small and single-purpose.
- Say what changed and why. Link the PRD requirement IDs the change serves
  when relevant, for example E1, D4 or P10.
- Run `cargo fmt` and `cargo clippy` before pushing.
- Sign off every commit and fill in the pull request template.

## Filing issues

Blank issues are disabled. Pick a form:

- **Bug report.** Anything that does not fit the two forms below.
- **App compatibility report.** Use this when expansion misbehaves in one
  specific app: text not inserted, garbled text, the abbreviation not deleted,
  the clipboard not restored, undo or the cursor marker wrong, or slow
  expansion. Give the app's name, version and bundle ID. These reports feed
  `data/compat/apps.toml`. Attach the output of "Copy diagnostics" if you can.
  It contains no content.
- **Provider quirk.** Use this when an "OpenAI-compatible" endpoint, a local
  server or the Anthropic adapter behaves differently from what Aralo expects:
  streaming framing, error bodies, unsupported parameters, the models route.
  Attach a conformance report from `aralo conformance --profile <name>` if you
  can. These reports feed `data/quirks/providers.toml`.

Never paste API keys or private snippet content into an issue.

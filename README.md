# Aralo

Aralo is a free, open-source AI text expander. You type a short abbreviation
and it becomes the full text, in any app. Snippets can ask a language model
for part of their text, using your own API key or a model on your own
machine. There is no Aralo account and no Aralo server.

It is a Rust core behind a thin native shell. macOS comes first.

## Status

**Pre-alpha: M0/M1 in progress, nothing to install yet.**

What exists today is the Rust base: the matching engine, the file format, the
library loader, a placeholder parser, the bridge to Swift and a command-line
tool that expands snippets in an imaginary text field. The Mac app is a first
slice you build yourself: a menu bar agent that expands plain-text snippets
and walks you through the permissions on first run, with no editor and no
settings yet. There is no AI code yet. Expect everything to change, including the
file format, which is at version 0.

## Privacy, as build properties

A text expander has to see what you type. These are the promises, and each
one is a property of the build that CI checks, not a policy.

| Promise | How it is held | State |
| --- | --- | --- |
| Keystrokes stay in memory, 64 characters at most, and are zeroed on every reset | A fixed ring buffer in one small crate, `aralo-engine`. Tests assert the buffer is zero after every reset reason | Enforced today |
| The code that sees keystrokes cannot write, send or log them | `aralo-engine` is `#![no_std]` and may depend on one crate, `zeroize`. CI inspects its dependency tree and scans its source for print and log macros | Enforced today |
| Nothing is recorded while paused or in an excluded app | Checked before the buffer is touched. Password managers are excluded by default | Enforced today |
| No telemetry | There is no upload code and no endpoint to send to | True today |
| No Aralo servers | The app will contact only the endpoints you configure, and GitHub for updates | Planned |
| AI sends only the context a snippet declares | Undeclared context is never requested, not merely never sent | Planned, M4 |
| Local-only mode means no network | One network guard constructs every HTTP client and refuses non-loopback addresses | Planned, M4 |
| API keys live only in the system keychain | Never in a file, never in the library folder | Planned, M4 |
| Model output cannot act | It is inserted as literal text and never parsed for placeholders | Planned, M4 |

You can verify "not a keylogger" by reading one crate:
[`crates/aralo-engine`](crates/aralo-engine). The checks are in
[`crates/aralo-engine/tests/privacy.rs`](crates/aralo-engine/tests/privacy.rs)
and [`scripts/check-deps.sh`](scripts/check-deps.sh). See
[docs/architecture.md](docs/architecture.md#privacy-invariants) for the full
table and [SECURITY.md](SECURITY.md) for reporting.

## Your snippets are files

A library is a folder of Markdown files with YAML front matter. One snippet
is one file, one group is one folder. Put the folder in iCloud Drive, Dropbox
or a Git repository; Aralo does not sync and does not need to.

```markdown
---
label: Best regards
abbr: ";br"
---
Best regards,
```

The format is specified in [docs/format/](docs/format/README.md), with JSON
Schemas in [`schemas/`](schemas/).

## Architecture

One process on macOS: a menu bar agent that hosts the event tap, the Rust
core and every window. The Swift shell captures keys and injects text. The
Rust core decides everything else. The shell never decides what to expand,
and the core never touches the operating system. Plain data crosses the
bridge, which UniFFI generates: key events in, expansion plans out. The
keystroke path is one synchronous call, `on_key`, with a 1 ms budget. The
same core runs from the command line on macOS, Linux and Windows, which is
how most of it is tested.

More in [docs/architecture.md](docs/architecture.md). Decisions and their
reasons are in [docs/adr/](docs/adr/README.md).

| Crate | Role | Today |
| --- | --- | --- |
| `aralo-engine` | Buffer, matcher, case rules, undo record. No I/O, no logging | Implemented |
| `aralo-snippet` | Data model: snippet, group and manifest files | Implemented |
| `aralo-template` | Placeholder parser, evaluator, `ExpansionPlan` | Parser and static plans. The evaluator is M3 |
| `aralo-library` | Folder store, inheritance, atomic writes, watcher, index, search; later merge | Load, write, watch, index and search. Merging is the rest of M2 |
| `aralo-core` | The facade the shells talk to | M1 slice: open, expand, simulate. Plus M2's import, export and search |
| `aralo-ffi` | UniFFI bridge to Swift | M1 slice |
| `aralo-cli` | `aralo`: `init`, `validate`, `list`, `search`, `type`, `expand`, `import`, `export` | M1 and the M2 import and search slices |
| `aralo-ai` | Gateway, network guard, framing, profiles | Empty, M4 |
| `aralo-providers` | Provider adapters | Empty, M4 |
| `aralo-embed` | Embedding runtime, vector scan | Empty, M4 |
| `aralo-import` | Importers, the import report, export to JSON, YAML and CSV | TextExpander, CSV, JSON and YAML in; JSON, YAML and CSV out |
| `aralo-script` | Script sandbox | Empty, v1 |

Dependencies point one way. `aralo-engine`, `aralo-snippet` and
`aralo-template` are the pure base. `aralo-core` composes. `aralo-ffi` and
`aralo-cli` sit on `aralo-core`. CI checks the graph.

## Building

**Rust crates and the CLI** build and test on macOS, Linux and Windows. You
need Rust stable, 1.86 or later. You do not need a Mac.

```sh
cargo test --workspace
```

Try the core without the app:

```sh
cargo run -p aralo-cli -- init /tmp/aralo-library
cargo run -p aralo-cli -- list /tmp/aralo-library
cargo run -p aralo-cli -- search /tmp/aralo-library regards
cargo run -p aralo-cli -- type /tmp/aralo-library "see you ;br "
cargo run -p aralo-cli -- expand /tmp/aralo-library ";br"
cargo run -p aralo-cli -- validate /tmp/aralo-library
```

Move snippets in and out of other tools:

```sh
cargo run -p aralo-cli -- import fixtures/import/csv/spreadsheet.csv \
  /tmp/aralo-library --into Imported --dry-run
cargo run -p aralo-cli -- export /tmp/aralo-library - --format yaml
```

`import` reports what each snippet cost and exits 1 if anything needs an edit.
[docs/format/import.md](docs/format/import.md) has the macro mapping and what
an import may lose.

`search` matches names, abbreviations, tags and groups fuzzily and bodies
literally, so a macro no importer could convert — left in the body as the text
it was — is one search away:

```sh
cargo run -p aralo-cli -- import fixtures/import/textexpander/work.textexpander \
  /tmp/aralo-imported
cargo run -p aralo-cli -- search /tmp/aralo-imported '%delay'
```

**The Mac app** needs macOS 14 or later and:

| Tool | Version | Install |
| --- | --- | --- |
| Rust | stable, 1.86 or later | <https://rustup.rs> |
| Xcode | 16 or later | App Store |
| xcodegen | any recent | `brew install xcodegen` |
| SwiftLint | any recent | `brew install swiftlint` |

```sh
make bootstrap   # XCFramework and Swift bindings from crates/aralo-ffi, then the Xcode project
make check       # rustfmt, clippy, the Rust tests and scripts/check-deps.sh
make test-swift  # Swift tests, which call the real Rust core
make run         # build Aralo.app (Debug) and launch it
```

`make matrix` and `make latency` measure injection in real apps. They take
over the keyboard, so they are for a Mac set aside for it; see "The injection
matrix" in [docs/architecture.md](docs/architecture.md). So far they have
been run against TextEdit only.

`make help` lists every target.

On first launch a four-step window says what Aralo reads and never keeps,
walks through the Accessibility and Input Monitoring grants that macOS
requires before any app may watch and post keys, and ends in a field where
you try a snippet. Control+Option+Command+P pauses from any app. It keeps its library in `~/Aralo` and
writes the starter snippets there if the folder is new. Set `ARALO_LIBRARY` to
use another folder, and `ARALO_COMPAT` to try a compatibility table other
than the built-in `data/compat/apps.toml`. Then type `ty` and a space in any
text field. A local
build is ad-hoc signed, so macOS forgets the permission after a rebuild. To
keep it, sign with a certificate from your keychain:
`make run SIGN_IDENTITY="Your Certificate"`.

`apps/macos/Generated/` and the `.xcodeproj` are build output. They are not
committed; `make bootstrap` rebuilds them.

## Repository layout

```text
aralo/
  Cargo.toml            workspace
  Makefile              bootstrap, check and the Swift targets
  deny.toml             cargo-deny: licences, advisories, bans, sources
  crates/               the Rust core; see the table above
  apps/
    macos/              the Mac app, AraloKit (tap, injector) and
                        AraloHarness (injection matrix, latency)
  data/
    starter/            the snippets a new library starts with
    compat/apps.toml    how text goes into each app, by bundle ID
    compat/matrix.json  how the injection matrix reaches a text field in each
  schemas/              JSON Schemas for the file format and the data tables
  scripts/
    check-deps.sh       engine purity, crate layering, no unsafe in the bridge
    build-xcframework.sh
  docs/
    architecture.md
    format/             the file format specification
    adr/                architecture decision records
  conformance/          provider protocol checks (M4)
  fixtures/
    matrix/library/     the snippets the injection matrix types
    import/             the import corpus and its golden reports
    ...                 golden expansions (M3)
```

## Roadmap

Weeks are from the plan and overlap on purpose.

| Milestone | Weeks | Exit criteria |
| --- | --- | --- |
| **M0** Foundations and spikes | 1–2 | A hard-coded snippet expands in TextEdit. Each spike has a written ADR |
| **M1** Engine and injection | 2–5 | 13 of 15 matrix apps pass. `on_key` p99 under 1 ms. Typed-to-inserted p95 under 50 ms |
| **M2** Library, editor, search, import | 4–8 | Import a TextExpander library, edit it and expand from it. Import fidelity 90% or better on the corpus |
| **M3** Dynamic content and forms | 7–9 | Golden suite green. Forms work in every matrix app. Import fidelity 95% |
| **M4** AI | 8–12 | Six endpoints pass conformance. Under 30 ms added to first token. The AI switch and local-only mode verified by tests |
| **M5** Sync, hardening and beta | 11–13 | Notarised DMG and Homebrew cask live. Every security and privacy test green. Beta opens |

M2's import and search halves are in. The corpus in
[`fixtures/import/`](fixtures/import/README.md) imports at 92.3% clean, and
`crates/aralo-import/tests/fidelity.rs` prints the number and fails under 90%
on every run. That corpus is written from the documented formats rather than
exported from a real installation, which its README says at more length; spike
S7 is where a real export settles it.

M2's folder store is in too: a debounced `notify` watcher that tells Aralo's
own saves from everyone else's by content hash, and a SQLite index with FTS5
that syncs incrementally. `crates/aralo-library/tests/index.rs` asserts that an
incremental sync lands exactly where a rebuild lands, and that ten thousand
snippets index on a background thread while an abbreviation expands on the
main one. Neither is wired into `aralo-core` yet; that comes with the core
runtime and the editor. See
[the folder store](docs/architecture.md#the-folder-store).

Seven spikes (S1 to S7) are still owed in M0. They are listed in
[docs/adr/README.md](docs/adr/README.md).

## Documents

| Document | What it is |
| --- | --- |
| [docs/format/](docs/format/README.md) | The library and snippet file format, placeholders, matching rules |
| [docs/architecture.md](docs/architecture.md) | Process model, threads, the keystroke path, the bridge, privacy invariants |
| [docs/adr/](docs/adr/README.md) | Decision records |
| [CONTRIBUTING.md](CONTRIBUTING.md) | How to build, the rules CI enforces, sign-off |
| [SECURITY.md](SECURITY.md) | How to report a vulnerability |
| [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md) | Community standards |

The documents in this repository are authoritative. Two longer planning
documents sit outside it. They are private-by-default pages and may ask you
for access:

- [Implementation plan](https://claude.ai/code/artifact/46c54272-8066-4ba6-8fbe-fe384a551789)
- [Product requirements (PRD)](https://claude.ai/code/artifact/b983dde8-42c1-49a2-9f80-db4a41f4a826)

IDs such as P1, E6 or D4 in the docs and the code refer to requirements in
the PRD. Where those pages and this repository disagree, the repository wins.

## Contributing

Read [CONTRIBUTING.md](CONTRIBUTING.md) first. Most contributions need only
`cargo test --workspace`. Changes to `aralo-engine`, the event tap or the
network guard need code-owner review.

The project lives at <https://github.com/anandghegde/aralo>.

## Licence

[Apache-2.0](LICENSE). See [NOTICE](NOTICE).

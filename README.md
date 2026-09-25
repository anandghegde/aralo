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
and walks you through the permissions on first run. AI is under way (M4):
the gateway, the first adapter, the AI settings, commands on selected text,
AI blocks inside snippets, AI actions in the snippet editor, search by
meaning and the master switch that turns all of it off exist. Expect
everything to change, including the file format, which is at version 0.

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
| AI sends only the context a snippet declares | Undeclared context is never requested, not merely never sent. The text around an AI block is not sent either | Enforced today |
| Local-only mode means no network | One network guard constructs every HTTP client and refuses non-loopback addresses | Enforced today |
| API keys live only in the system keychain | Never in a file, never in the library folder | Enforced today |
| Model output cannot act | It is inserted as literal text and never parsed for placeholders | Enforced today |
| AI is off until you switch it on, and off means off | No model is asked anything and no model loads, the embedding model included. A test tries every way in with the switch off and watches a socket, the keychain and the model | Enforced today |
| Search by meaning runs on your Mac | The embedding model ships inside the app and is never downloaded at runtime. CI holds `aralo-embed` to a dependency allow-list with no networking crate in it | Enforced today |

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
| `aralo-template` | Placeholder parser, evaluator, `ExpansionPlan` | Parser, the editor's outline of a body, the evaluator and plans, AI blocks included |
| `aralo-library` | Folder store, inheritance, atomic writes, watcher, index, search, conflict-copy merge | M2: load, write, watch, index, search and merge; vectors and hits by meaning |
| `aralo-core` | The facade the shells talk to | Open, expand, simulate, import, export, search by words and by meaning, edit, the runtime that watches, indexes and embeds, the AI settings, commands, AI blocks and editor actions |
| `aralo-ffi` | UniFFI bridge to Swift | The keystroke path, editing, search, interchange, change events, the AI settings, commands, AI blocks, editor actions and the embedding model |
| `aralo-cli` | `aralo`: `init`, `validate`, `list`, `search`, `type`, `expand`, `import`, `export`, `ai` | M1, the M2 import and search slices, AI profiles and commands, `expand --ai`, `ai write` and `search --model` |
| `aralo-ai` | Gateway, network guard, framing, profiles | Gateway, network guard, secret store, Test connection and the capability probe |
| `aralo-providers` | Provider adapters, SSE parser, local-server detection | `openai_compat` and local-server detection |
| `aralo-embed` | Embedding runtime, vector scan | A static model's tokenizer and encoder, held to the reference implementation, and the vector scan |
| `aralo-import` | Importers, the import report, export to JSON, YAML and CSV | TextExpander, CSV, JSON and YAML in; JSON, YAML and CSV out |
| `aralo-script` | Script sandbox | Empty, v1 |

Dependencies point one way. `aralo-engine`, `aralo-snippet` and
`aralo-template` are the pure base. `aralo-core` composes. `aralo-ffi` and
`aralo-cli` sit on `aralo-core`. CI checks the graph.

## Building

**Rust crates and the CLI** build and test on macOS, Linux and Windows. You
need Rust stable, 1.88 or later. You do not need a Mac.

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

With the embedding model, it finds snippets by what they mean as well. The
model is fetched from Hugging Face once, checked against recorded checksums,
and never contacted again. It is a model, so it loads only while AI is on:

```sh
make model
cargo run -p aralo-cli -- ai on
cargo run -p aralo-cli -- search fixtures/search/library "money back" \
  --model models/potion-base-8M
```

**The Mac app** needs macOS 14 or later and:

| Tool | Version | Install |
| --- | --- | --- |
| Rust | stable, 1.88 or later | <https://rustup.rs> |
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
matrix" in [docs/architecture.md](docs/architecture.md). One of the cases is a
snippet that asks a question: the form panel takes the keyboard without
activating, so the harness answers it with the same keys it typed the
abbreviation with. So far they have been run against TextEdit only.

`make help` lists every target.

On first launch a four-step window says what Aralo reads and never keeps,
walks through the Accessibility and Input Monitoring grants that macOS
requires before any app may watch and post keys, and ends in a field where
you try a snippet. Control+Option+Command+P pauses from any app. It keeps its library in `~/Aralo` and
writes the starter snippets there if the folder is new; the search index and
the rest of what it can rebuild go in `~/Library/Application Support/Aralo`.
Set `ARALO_LIBRARY` to use another folder, `ARALO_STATE` to put the
rebuildable files somewhere else, and `ARALO_COMPAT` to try a compatibility
table other than the built-in `data/compat/apps.toml`. Then type `ty` and a
space in any text field. A local
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
    check-deps.sh       engine purity, crate layering, no unsafe in the bridge,
                        no network under the embedding runtime
    build-xcframework.sh
    fetch-model.sh      the embedding model, checked against its checksums
  docs/
    architecture.md
    format/             the file format specification
    adr/                architecture decision records
  conformance/          provider protocol checks (M4)
  fixtures/
    matrix/library/     the snippets the injection matrix types
    import/             the import corpus and its golden reports
    golden/             golden expansions, three locales on a stopped clock
    embed/              a tiny model and what the reference implementation
                        makes of it, for the tokenizer and encoder
    search/library/     a library to find things in by what they mean
  models/               the embedding model, fetched by `make model`; not in Git
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

M2's import and search halves are in, and M3 has since raised the bar they are
held to. Import is in the Mac app as well as the CLI: the snippet window, or
Import Snippets… in the menu bar, opens a file on a dry run, says what it would
become before anything is written, and after the import opens each snippet that
needs an edit from the report. Export writes the selected group, or the whole
library, as JSON, YAML or CSV. The corpus in [`fixtures/import/`](fixtures/import/README.md) imports
at 95.4% clean, and `crates/aralo-import/tests/fidelity.rs` prints the number
and fails under the 95% M3 asks for on every run. The last two points came from
giving the format what the sources already said: a fill-in area lands in a box
with room to write in, and `%key:tab%` presses a real Tab
([ADR-0015](docs/adr/0015-format-grows-for-the-importer.md)). That corpus is
written from the documented formats rather than exported from a real
installation, which its README says at more length; spike S7 is where a real
export settles it.

M2's folder store is in too: a debounced `notify` watcher that tells Aralo's
own saves from everyone else's by content hash, and a SQLite index with FTS5
that syncs incrementally. `crates/aralo-library/tests/index.rs` asserts that an
incremental sync lands exactly where a rebuild lands, and that ten thousand
snippets index on a background thread while an abbreviation expands on the
main one. The conflict copies a sync client leaves are recognised by each
client's naming and merged three ways against the version this machine last
saw. A clean merge is written and the copy goes to the Trash. A clash leaves
both files and waits for the user: the snippet window shows the two side by
side, says what they changed differently, and keeps the original, the copy or
a version the user writes
([docs/architecture.md](docs/architecture.md#conflict-copies)).

M3's evaluator is in, and with it the sessions that ask before anything is
inserted. A body writes the date and time in 15 locales from the format's own
tables, takes the clipboard, puts a form in front of the user, inlines other
snippets eight deep, presses a Tab or a Return the app acts on rather than a
character that looks like one, asks for a box with room to write in, and leaves
the caret where it says
([ADR-0014](docs/adr/0014-clock-and-locale.md),
[docs/format/placeholders.md](docs/format/placeholders.md)). What the editor
previews is the expansion, byte for byte. The proof is
[`fixtures/golden/`](fixtures/golden/README.md): every body expanded in
`en_US`, `de_DE` and `ja_JP` on a clock stopped at one minute, compared byte
for byte.

The Mac form panel is in, so a snippet that asks something now asks it: the
abbreviation is swallowed, nothing is typed, and a non-activating panel puts
the boxes in front of the user with a live preview of what Enter will insert.
It is the same panel however the snippet was asked for, typed or picked from
the palette. The clipboard is read when the body says it wants it and not
before, cancelling puts the swallowed keystroke back, and what is inserted goes
in through the same injector and arms the same undo as any other expansion.
`FormSession` in AraloKit is the panel apart from its window, so a second shell
writes the window and nothing else. See
[the form panel](docs/architecture.md#the-form-panel).

Both the store and the index now run under `aralo_core::Runtime`, which is what a shell holds for the
lifetime of the app: edit a file in any text editor and the next keystroke
matches it, without the app asking. The editing calls are on the bridge
alongside them — create, save, move and delete a snippet or a group, with the
draft checks an editor shows before it saves.

The window that uses them opens from the menu bar: a group tree, the snippets
in the selected group with a search box over them, and an editor with the
snippet's settings shown against what its groups make of them, plus a live
preview. Every change it makes lands as a readable file diff. What it shows is
`LibraryStore` in AraloKit, which is the model a second shell would keep; the
SwiftUI views above it hold nothing a Windows one would have to write again.
The body editor draws the core's own reading of the snippet: every placeholder
is coloured, anything the parser could not read is underlined at its range with
the core's sentence under it, and an Insert menu offers the placeholders the
core knows and pre-selects the part to type over. It is one call,
`outline_draft(body)`, and it is the same parse an expansion runs, so what the
editor marks is what would happen. Under the preview is a test field: type
the abbreviation and the draft on screen expands, through the same matcher and
expansion path as any app, before anything is saved. A form opens as a sheet,
Backspace takes an expansion back, and the live tap stands aside while the
field has the keyboard.

The inline search palette is in: ⌃⌥⌘Space anywhere, a few letters, and Enter
puts the snippet into the app the user was already in. The window is a panel
that does not take that app out of the front, so there is no focus to restore;
it stops Aralo watching the keyboard while it is up, and gives the keyboard
back before a character is typed. The text goes in through the same injector
and arms the same undo as an expansion that was typed, because to the user it
is the same thing arriving by another route. It is refused while Aralo is
paused and in the apps it stays out of, and the palette is the one place a user
is told why. See
[the search palette](docs/architecture.md#the-search-palette),
[the snippet window](docs/architecture.md#the-snippet-window),
[the folder store](docs/architecture.md#the-folder-store) and
[the bridge API](docs/architecture.md#bridge-api).

M4 has begun with the gateway every AI feature will go through (task 4.1). It
checks the AI switch, local-only mode and a managed allow-list, asks the shell
only for the context a snippet declared, frames that context as data, and
meters tokens per feature and per profile. It is also the only code that can
reach the network. In local-only mode it refuses any URL that is not this
Mac before sending, and any address that is not loopback when connecting.
`crates/aralo-ai/tests/local_only.rs` proves both against a real listener.

The first adapter is in (task 4.2). `openai_compat` streams chat completions
from OpenAI and every API that copies it: OpenRouter, Groq, Together, LiteLLM
and the local servers. It reads the stream incrementally and meters usage
once, at the end. A cut or garbled stream is an error, never a short answer.
Dropping the stream closes the socket, and a test against a real listener
proves it. `detect_local_servers` finds Ollama, LM Studio, llama.cpp and vLLM
on their default ports. The adapter's tests replay transcripts from
`fixtures/sse/`. One is recorded from a live endpoint; the rest are
hand-written from each provider's documented format, and
`crates/aralo-providers/tests/live.rs` can record replacements. A native
Anthropic adapter (task 4.3) is deferred: Claude models are reachable through
OpenAI-compatible endpoints such as OpenRouter. See [the AI gateway](docs/architecture.md#the-ai-gateway) and
[provider adapters](docs/architecture.md#provider-adapters).

Profiles and keys are in (task 4.4). A profile is a provider's address, a
model and optional headers, saved in `profiles.toml` in Aralo's state folder,
outside the library. Its key is kept in the login keychain, and the file
holds only a reference to it. The Mac app has a Settings window (⌘,) with an
AI tab. It has the switch and local-only mode, presets for ten providers with
a link to each one's key page, Test Connection, a model list, local-server
detection and a probe that shows what an endpoint can do. The terminal has
the same through `aralo ai`, which reads a key only from the environment or
standard input. `scripts/key-leak-scan.sh` runs every test and then looks
for a key in every file the run wrote. See
[profiles and keys](docs/architecture.md#profiles-and-keys).

Commands on selected text are in (task 4.5). Select text in any app and press
⌃⌥⌘A, or pick Transform Selection… from the menu. Aralo reads the selection
through Accessibility, or copies it when the app does not say, and puts your
clipboard back. Then pick a command, such as "Fix spelling and grammar" or
"Make it shorter". The answer streams in and is shown against the selection
word by word. Enter replaces the selection with one paste, so one ⌘Z in the app
brings the original back. E edits the answer first and R asks again. The panel
names what was sent and to which profile and model. A command is a snippet with
`type: command` whose body is the instruction. Seven are built in, and a
library can add its own or replace them. `aralo ai command list` and
`aralo ai command run` do the same from the terminal, with the selection on
standard input. See
[commands on selected text](docs/architecture.md#commands-on-selected-text).

AI blocks inside snippets are in (task 4.6). A body asks a model for part of
its text with `{{ai: prompt | fallback: text}}`, and the snippet's `ai.context`
says what the model may see: the form's answers, the selection, the clipboard,
the app or its window. Nothing else is sent, not even the snippet's own text
around the block. The form panel shows the answer streaming into the
preview, marked apart from the snippet's own text: Enter inserts, R asks
again, E edits and Escape cancels. The model's words go in as literal text,
so a model that writes `{{clipboard}}` puts in those characters and reads
nothing. The text around a block is byte-identical whatever the model wrote;
a property test and `fixtures/golden/ai.toml` hold the evaluator to it. With
AI off, local-only mode refusing the host, or no network, the block puts in
its fallback and the panel says why. `aralo expand --ai` does the same from
the terminal, and without `--ai` a block puts in its fallback and says how to
ask. See [AI blocks in snippets](docs/architecture.md#ai-blocks-in-snippets).

The snippet editor has AI actions (task 4.7). An AI menu beside Insert can
draft a body from the snippet's label, fix spelling and grammar, make the text
clearer or shorter, change its tone, translate it or suggest variations. It
works on the selection, or on the whole body when nothing is selected, and
that is all it sends; a draft sends only the label and a note. The answer
streams into a sheet with a word diff, and the sheet names any placeholder the
answer dropped or added, because one a model writes is one the snippet will
expand. Replace puts it in as one edit, so one ⌘Z in the editor takes it back.
`aralo ai write proofread < body.txt` does the same from a terminal. See
[AI actions in the editor](docs/architecture.md#ai-actions-in-the-editor).

Search by meaning is in (task 4.8). "money back" finds the refund reply that
never uses either word. It runs on MinishLab's `potion-base-8M`, a static
embedding model that ships inside the app: a snippet embeds in about 25 µs,
the library is embedded in the background and kept in the index, and nothing
leaves the machine. Snippets found by meaning come after the ones your words
found, marked as such. `make model` fetches the model, checked against
recorded checksums; with AI on, `aralo search --model models/potion-base-8M`
uses it from the terminal. See [search by meaning](docs/architecture.md#search-by-meaning)
and [ADR-0016](docs/adr/0016-static-embeddings.md).

The AI master switch governs all of it (task 4.10). AI is off until you
switch it on in Settings or with `aralo ai on`, and while it is off no model is
asked anything and no model loads: the embedding model search by meaning uses
is dropped the moment AI goes off and loaded again when it comes back on.
Local-only mode keeps requests on this Mac and leaves the embedding model
alone, since it never leaves the machine. Under every answer, the panels say
what was sent and to whom; a click lists each kind the request could carry,
with the bytes that went or "nothing to send". See
[the master switch](docs/architecture.md#the-master-switch).

Six spikes are still owed in M0; S5, the embedding runtime, is answered by
ADR-0016. They are listed in [docs/adr/README.md](docs/adr/README.md).

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

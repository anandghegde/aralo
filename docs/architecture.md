# Architecture

Aralo is a Rust core behind a thin native shell. The core decides what to
expand. The shell touches the operating system. This page describes the
design and says, for each part, whether it exists today. The project is
pre-alpha: the Rust base exists, and the Swift shell under `apps/macos` is in
progress.

Decisions and their reasons are in [adr/](adr/README.md). The file formats
are in [format/](format/README.md).

## Process model

Aralo on macOS is one process: a menu bar agent that hosts the event tap, the
Rust core and every window. A single process keeps the keystroke path free of
IPC, which the latency budget depends on
([ADR-0003](adr/0003-single-process-architecture.md)).

The shell never decides what to expand, and the core never touches the
operating system. Everything that crosses the bridge is plain data: key
events in, actions and expansion plans out
([ADR-0001](adr/0001-rust-core-native-swift-shell.md),
[ADR-0002](adr/0002-uniffi-bridge.md)).

The same core runs from a terminal. `aralo type` types into a text field that
exists only in memory, so the whole expansion path runs in CI on Linux and
Windows with no window server.

## Threads

From [ADR-0004](adr/0004-threading-model.md). The first three are Swift, in
`apps/macos/Packages/AraloKit`.

| Thread | Runs | Rule |
| --- | --- | --- |
| Tap thread | A run loop with the `CGEventTap`. Calls `engine.on_key` synchronously | Never blocks. 1 ms budget. macOS disables a tap that stalls |
| Injector queue | A serial queue that posts synthetic key events and does pasteboard work | The only code that posts events. It tags them so the tap ignores its own output |
| Main thread | AppKit and SwiftUI | No file, network or model work |
| Core runtime | tokio with two workers: AI streams, file watcher, indexing, embedding | Publishes a new immutable matcher snapshot when the library changes |

The matcher data is an immutable snapshot. A library change builds a whole
new snapshot and the engine swaps it in.

Today there is no core runtime: the core is synchronous and a caller asks for
a reload. The bridge guards the matcher with a mutex and the open library
with a read-write lock. Each is held for one key or for a pointer swap. Disk
work in `reload` happens with no lock held. Whether this is good enough for
the tap thread is part of spike S2.

## The keystroke path

1. The tap receives a key-down event. Events that Aralo posted itself carry a
   tag and pass straight through.
2. The tap turns the event into a character. It translates the key itself
   (`UCKeyTranslate`, with the dead-key state kept between keys), because
   the string on the event is the key on its own: after a dead key the
   letter arrives as "e" while the document gets "é". A key that produced
   exactly one Unicode scalar is reported as that number, so no keystroke
   sits in a heap-allocated string. A key that produced several is not
   reported; the shell calls `reset(UnmappableInput)`. A key that cancels a
   waiting dead key resets too. While an input method composes (Japanese,
   Chinese, Korean), no key reaches the engine
   ([ADR-0012](adr/0012-injection-and-input-defaults.md)).
3. Mouse down, navigation keys, shortcuts, app switches, focus changes and
   secure input are not keys to match. The shell calls `reset(reason)`, which
   zeroes the buffer.
4. The tap calls `Engine.on_key` across the bridge, synchronously.
5. The engine pushes the character into its 64-character ring and walks the
   trie. The rules are in [format/matching.md](format/matching.md). The answer
   is `Pass`, a match, or an undo.
6. On a match the bridge builds the expansion plan before it returns: it
   looks the snippet up in the open library, renders the body, applies the
   case transform and puts the delimiter back. `on_key` returns
   `Expand { steps, … }`, so the shell never makes a second call while a key
   is waiting. The plan carries the front app's injection profile (see
   [The compatibility table](#the-compatibility-table)).
7. The tap consumes the key that completed the match, so the target app never
   sees a delimiter it might act on. It hands the steps to the injector queue.
8. The injector runs the steps in order: it deletes the abbreviation, then
   inserts the text, typed or pasted as the profile says
   ([format/expansion-plan.md](format/expansion-plan.md)).
9. The injector calls `Engine.expansion_done`. That arms one-key undo: if the
   next key is Backspace or the undo shortcut, `on_key` returns
   `UndoExpansion` with what to delete and what to retype. One case skips
   this step: a line break that went in as a Return key. The app may have
   acted on it, so the expansion offers no undo.

A static snippet goes from `on_key` to a finished plan without leaving the
tap and injector threads. Forms and AI blocks (M3, M4) will move the session
onto the core runtime and the main thread.

## Permissions and the first run

The tap needs Accessibility access, and macOS reports neither a grant nor a
revocation. So the shell polls. `AraloService` tries to create the tap every
two seconds until it succeeds; once it is up, the same timer watches for
Accessibility being taken away, and then takes the tap down, empties the
engine and goes back to waiting. A tap left up without the grant can stall the
keyboard.

The first run is four screens: what Aralo reads and never keeps,
Accessibility, Input Monitoring, and a field to try a snippet in. Each
permission screen says what the system prompt will look like before it
appears, and enables Continue when the grant arrives. The rules are
`OnboardingFlow` in AraloKit, a value type with tests; the window in the app
target only draws it.

- Input Monitoring counts as granted when the tap is running.
  `CGPreflightListenEventAccess` can go on answering "no" inside a process
  that was running when the grant was made, and a running tap is the thing
  the grant is for. Whether an active tap needs the grant at all on every
  macOS version is unmeasured; the screen is there because the plan asks for
  it and the prompt explains itself better before than after.
- A permission screen whose grant is already there is skipped.
- A finished first run is remembered in the `onboardingCompleted` default.
  After that the window opens on its own only at the first permission that is
  missing. While a permission is missing the menu bar icon is a warning
  triangle and the menu offers "Set Up Aralo…", which opens the same flow.
- The try-it field uses a snippet from the user's own library: `ty` if it is
  there, otherwise the first enabled single-line snippet without
  placeholders. The window belongs to Aralo, so the keys go through the real
  tap, engine and injector. Nothing in it is simulated.

## The compatibility table

Apps disagree about synthetic keys and about paste. How text goes into each
one is data, not code: `data/compat/apps.toml`, by bundle ID, described by
`schemas/compat.schema.json`.

| Setting | Values | Default |
| --- | --- | --- |
| `insert` | `auto`: type short single-line text, paste the rest. `type`: always type, a line break is a Return key. `paste`: always paste | `auto` |
| `typing_limit` | The longest text `auto` still types, in UTF-16 units | 120 |
| `key_delay_ms` | Pause after every synthetic key | 1 |
| `paste_settle_ms` | How long the app gets to read the pasteboard before the user's clipboard is put back | 250 |
| `undo` | `native`: one Cmd+Z takes a pasted expansion back. `backspace`: every expansion is deleted | `native` |
| `delete` | `backspace`: one Backspace per character. `select`: Shift+Left over the characters, then one Backspace | `backspace` |

The table is compiled into `aralo-core` (`compat.rs`), and
`cargo test -p aralo-core` fails if the file that ships does not parse. The
parser refuses unknown keys, duplicate bundle IDs, a newer format version and
numbers past fixed upper bounds, so a bad table can never stall the injector.
A refused table changes nothing: the core keeps the one it had.

The core looks the front app up in `set_front_app`, off the keystroke path,
and attaches the resulting `InjectionProfile` to every `Expand` and
`UndoExpansion`. The injector follows it and decides nothing. An app the
table does not name gets the defaults.

The entries today are the 15 apps of the injection matrix. Only TextEdit is
measured so far, and it passes on the defaults. The only overrides are the
plan's own rule that terminals and remote desktops are typed into. Spikes S1
and S6 replace them with measurements,
taken with [the injection matrix](#the-injection-matrix). To
try a table without rebuilding, start the app with `ARALO_COMPAT` set to a
file path; the bridge call is `Core.load_compat_table(path)`. Later the table
will also arrive as an Ed25519-signed download (plan section 4.4).

## The injection matrix

`data/compat/apps.toml` is only worth what it was measured with. The measuring
tool is `aralo-harness`, a command-line Swift package at
`apps/macos/Packages/AraloHarness`. It is separate from the app so that the
app's no-printing rule stays absolute.

```sh
make matrix                                   # every case in every app of the table
make matrix ARGS="--method paste"             # one insert method forced on every app
make matrix ARGS="--only com.apple.TextEdit --cases ascii,undo"
make latency                                  # typed-to-inserted time in TextEdit
swift run --package-path apps/macos/Packages/AraloHarness aralo-harness list   # touches nothing
```

`matrix` and `latency` take over the keyboard of the Mac they run on. They are
for a Mac set aside for it, not for the one you are working at.

**What a run does.** It starts the built Aralo.app on the library in
`fixtures/matrix/library` (through `ARALO_LIBRARY`, with the first-run window
switched off), and checks with `CGGetEventTapList` that this Aralo really has
its event tap, so that a missing permission cannot read as fifteen failing
apps. For each app it opens a text field, types an abbreviation and a space
with real, untagged key events posted at the HID tap, and reads the field
back through the Accessibility API. When the app tells Accessibility nothing,
it selects all, copies, reads the clipboard and puts the clipboard back.

| Case | Types | Passes when |
| --- | --- | --- |
| `ascii` | `mxascii` | the sentence arrives |
| `unicode` | `mxemoji` | emoji with joiners and skin tones, a combining accent and CJK arrive unchanged |
| `long` | `mxlong` | all 2,000 characters arrive |
| `cursor` | `mxcursor` | the next key lands where `{{cursor}}` was. Pending: needs the template evaluator (M2) |
| `undo` | `mxascii`, then Cmd+Z | the abbreviation is back and the expansion is gone |
| `clipboard` | `mxlong` | the text arrives and the clipboard holds what it held before |

The text a case expects is never written down twice. The harness opens the
same library through the bridge, feeds the abbreviation to `Engine.on_key`
and reads the plan, so it compares the field with what the core says, under
the very table row Aralo will use (`compat_apps` lists the rows). A forced
method is a generated table handed to both through `ARALO_COMPAT`.

**Verdicts.** A case that fails is tried again, three times in all (the
plan's rule against flaky desktop automation). `pass` is a first-try pass,
`flaky` passed on a later try, `fail` never passed. An app passes when every
case that ran is `pass` or `flaky`. `--require 13` makes the M1 exit criterion
an exit code. Reports are a Markdown table and a JSON file of verdicts and
timings. Text read back from an app is compared and dropped: it is never
written to a report or to the terminal.

**Latency.** For first-try passes of `ascii`, `unicode` and `long`, the clock
runs from just before the delimiter key is posted until the Accessibility
value holds the expansion, polled every 2 ms. That includes the harness's own
key posting and polling, so it is an upper bound. `make latency` repeats it 50
times per insert method in one app and prints p50, p95 and p99; `--gate`
fails above the 50 ms p95 budget.

**Two rules keep it from harming the Mac it runs on.** No key is sent unless
the app under test has the keyboard, checked before every chord and every
word; three losses end the run. And clearing, which is select-all and Delete,
only ever happens in a field the harness found empty, so text it did not type
is never its to delete. Terminals are cleared with Ctrl+U, in a scratch zsh
line editor (`vared`) that runs nothing when Return is pressed. Plain `cat`
would not do: the terminal driver's own line buffer stops at 1,024 bytes.
Apps the harness launched are quit afterwards; apps that were already running
are left running, with the scratch window still open.

**How each app's field is reached** is data too: `data/compat/matrix.json`, one
recipe per app (`document`, `web-page`, `shell`, `keys` or `manual`). `manual`
apps (Slack, Obsidian, Remote Desktop) need an account, a vault or a session,
so they are skipped unless `--manual-wait SECONDS` gives a person time to
click into a field. A test fails when the recipes and `apps.toml` name
different apps.

**What has been measured.** One app, once: TextEdit, on 21 September 2026
(macOS 26.4.1, Apple M4, Debug build). All five cases that can run passed on
the first try with the table as shipped, `undo` and `clipboard` included.
Typed to inserted over 30 expansions per method, as an upper bound:

| Method | p50 ms | p95 ms | p99 ms |
| --- | --- | --- | --- |
| type (44 characters) | 32.1 | 34.2 | 35.9 |
| paste (2,000 characters) | 36.6 | 39.2 | 58.1 |

Both are inside the 50 ms p95 budget. The other 14 apps, their recipes in
`matrix.json` and their values in `apps.toml` have not been run; neither has a
forced-method run or the nightly job. The runner, the report and the file
formats are also unit-tested against an in-memory desktop (`make test-swift`).
The full run is spike S1 and S6's measurement.

**The nightly job** is `.github/workflows/matrix.yml`: schedule and manual
runs only, never a pull request, because it needs a self-hosted Mac and a
self-hosted runner on a public repository must not run code from forks. It
stays off until the repository variable `MATRIX_RUNNER` is `true`. It writes
the results table to the job summary and uploads the JSON. The Mac needs a
logged-in desktop, the matrix apps, a runner labelled `aralo-matrix`, and
Accessibility for the runner's process and Accessibility plus Input
Monitoring for the built Aralo.app. macOS ties a grant to the code signature,
and an ad hoc signature changes with every build, so the job signs with the
keychain certificate named in the variable `MATRIX_SIGN_IDENTITY`
(`make app SIGN_IDENTITY=...`).

## Bridge API

The bridge is `crates/aralo-ffi`, generated by UniFFI. Nothing in it may
panic: a panic would cross the bridge as a crash in the process that holds
the event tap.

**Today:**

| Object | Calls | Kind |
| --- | --- | --- |
| `Engine` | `on_key(KeyInput) -> KeyAction`, `expansion_done(snippet_id, delete_count, method)`, `reset(reason)`, `set_front_app(bundle_id)`, `injection_profile()`, `set_paused(bool)`, `is_paused()`, `set_excluded_apps(bundle_ids)`, `holds_no_keystrokes()` | Synchronous |
| `Core` | `open_library(path)` (constructor), `engine()`, `reload()`, `load_compat_table(path)`, `library_path()`, `snippets()`, `diagnostics()` | Synchronous |
| Functions | `core_version()`, `excluded_app_presets()`, `compat_apps(path?)`: the rows of a table, for the injection matrix | Synchronous |

`KeyAction` is one of `Pass`, `Expand { snippet_id, consume, steps,
undo_delete_count, profile }` or `UndoExpansion { delete_count, retype,
method, profile }`.
`set_excluded_apps` always keeps the built-in password-manager presets, so a
shell that forgets to pass them cannot drop them.

**Planned** (plan section 3; names may change):

| Object | Calls | Arrives |
| --- | --- | --- |
| `Core` | `save_snippet`, `search`, `import`, `export`, `preview` | M2 |
| `ExpansionSession` | `next() -> Step`, `submit_form`, `provide_context`, `regenerate`, `cancel` | M3, M4 |
| `AiProfiles` | `save`, `test_connection`, `probe_capabilities`, `list_models`, `detect_local_servers` | M4 |
| Callbacks the shell implements | `ContextProvider`, `CoreEvents`, optional `SecretStore` | M3, M4 |

## Crate graph

Dependencies point one way: down. `scripts/check-deps.sh` holds this table
and fails CI when a crate depends on its own layer or a higher one. Layer 1
is the exception: its crates may depend on each other.

| Layer | Crates | State today |
| --- | --- | --- |
| 3 | `aralo-ffi`, `aralo-cli` | Bridge and CLI for the M1 slice |
| 2 | `aralo-core` | Open a library, build a plan, the compatibility table, in-memory simulator |
| 1 | `aralo-library`, `aralo-ai`, `aralo-embed`, `aralo-providers`, `aralo-import`, `aralo-script` | `aralo-library` loads, resolves inheritance and writes atomically. The rest are empty |
| 0 | `aralo-engine`, `aralo-snippet`, `aralo-template` | Implemented. The template evaluator is not |

The rule covers every dependency kind, including dev and build dependencies.
A new crate must be added to the table in the script before CI passes.

`aralo-engine` has a stricter rule of its own
([ADR-0005](adr/0005-engine-crate-purity.md)): it is `#![no_std]` and its
only permitted dependency, direct or transitive, is `zeroize`.

## Privacy invariants

Each promise is a property of the build, with a check that fails when it
breaks. "Planned" means the code it would check does not exist yet.

| Promise | Mechanism | Enforced by |
| --- | --- | --- |
| Keystrokes stay in memory, 64 characters at most (P1) | Fixed ring in `aralo-engine`, zeroed with `zeroize` on every reset, after every match and on drop. The undo record holds the typed abbreviation for one key | `crates/aralo-engine/tests/privacy.rs`: the buffer is zero after every reset reason and every invalidating state change, the undo record is dropped by a reset, a match empties the buffer, `Debug` output carries no typed text |
| The engine cannot write, send or log what it sees (P1) | No I/O, clock, network or logging dependency. `#![no_std]` | `scripts/check-deps.sh`: dependency-tree check, and a source scan for `println!`, `eprintln!`, `print!`, `eprint!`, `dbg!`, `log::` and `tracing::`. `crates/aralo-engine/clippy.toml` bans the same macros. Code owners on `crates/aralo-engine/` and the tap |
| No hand-written `unsafe` | The workspace forbids `unsafe_code`. `aralo-ffi` is the one crate that cannot, because UniFFI generates `unsafe` scaffolding | `scripts/check-deps.sh` scans `crates/aralo-ffi/src` |
| Nothing is recorded while paused or in an excluded app (P2) | The check runs before the buffer is touched. Password managers are preset | `privacy.rs`: `nothing_is_recorded_while_paused_or_in_an_excluded_app`. A unit test on the presets. `crates/aralo-ffi/tests/bridge.rs`: `excluded_apps_never_expand` |
| No expansion while secure input is on (P2) | The shell polls `IsSecureEventInputEnabled()`, resets, and names the app that holds it in the menu | `SecureInputMonitor` in AraloKit. The engine side, `reset(SecureInput)`, is covered by `privacy.rs` |
| AI sends only declared context (P4) | The session requests only declared kinds | Planned, M4 |
| Local-only mode means no network (P6) | The network guard is the only HTTP client constructor ([ADR-0007](adr/0007-ai-gateway-and-network-guard.md)) | Planned, M4. `deny.toml` already bans other HTTP clients and will confine `reqwest` to `aralo-ai` |
| Prompt injection cannot act (P10) | AI output is literal text with no path back into the evaluator | Planned, M4 |
| Keys only in the keychain (P13) | `SecretStore`. Profiles hold a reference, never a value | Planned, M4 |
| Licences, advisories, sources | `deny.toml` | `cargo-deny` in CI |

All of these run in `.github/workflows/check.yml` on every pull request.
The injection matrix does not: see [above](#the-injection-matrix).

## Spikes still owed

M0 asks seven questions before the design is fixed. Each ends in a written
ADR. None is written yet. The questions and fallbacks are in
[adr/README.md](adr/README.md).

| Spike | Subject |
| --- | --- |
| S1 | Injection: typing and paste in the 15 matrix apps |
| S2 | Bridge on the hot path: the cost of `on_key` across UniFFI on the tap thread |
| S3 | Caret bounds from the Accessibility API |
| S4 | Pasteboard restore |
| S5 | Embedding runtime |
| S6 | Undo semantics per insert method and app |
| S7 | Typinator export format |

S2 and S5 may change ADR-0002 and ADR-0008.

## Decision records

| File | Decision |
| --- | --- |
| [0001-rust-core-native-swift-shell.md](adr/0001-rust-core-native-swift-shell.md) | Rust core, native Swift shell |
| [0002-uniffi-bridge.md](adr/0002-uniffi-bridge.md) | UniFFI for the bridge |
| [0003-single-process-architecture.md](adr/0003-single-process-architecture.md) | One process |
| [0004-threading-model.md](adr/0004-threading-model.md) | Threading model |
| [0005-engine-crate-purity.md](adr/0005-engine-crate-purity.md) | Engine crate purity |
| [0006-files-as-source-of-truth.md](adr/0006-files-as-source-of-truth.md) | Files as the source of truth |
| [0007-ai-gateway-and-network-guard.md](adr/0007-ai-gateway-and-network-guard.md) | AI gateway and network guard |
| [0008-library-choices.md](adr/0008-library-choices.md) | Library choices |
| [0009-apache-2-licence.md](adr/0009-apache-2-licence.md) | Apache-2.0 licence |
| [0010-minimum-macos-14.md](adr/0010-minimum-macos-14.md) | Minimum macOS 14 |
| [0011-shell-defaults.md](adr/0011-shell-defaults.md) | Defaults of the first Mac shell |
| [0012-injection-and-input-defaults.md](adr/0012-injection-and-input-defaults.md) | Per-app injection is data; input defaults of the M1 shell |

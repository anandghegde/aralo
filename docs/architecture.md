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

The two pieces the runtime will own are already built and already synchronous:
`aralo_library::Watch` runs `notify` on its own thread and reports debounced
batches through a callback, and `aralo_library::Index` is a plain SQLite
connection that a caller moves onto whichever thread it likes. Neither needs
tokio, and the runtime's job when it arrives is to schedule them, not to
replace them.

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
tap and injector threads. A body that asks something first — a form to fill
in, a clipboard to fetch — answers `StartSession` instead: nothing has gone
into the document yet, the shell drives the session on the main thread, and a
session the user cancels leaves what they typed where it was. AI blocks (M4)
join that path.

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

## The snippet window

The window opens from the menu bar and closing it leaves the agent running: the
menu bar item is the app, the window is one of its views.

`LibraryStore` in AraloKit holds everything it shows: the group tree, the list,
the draft on screen, the core's advice about that draft, and why the last read
or the last command failed. The views in the app target hold no logic a Windows
shell would have to write again. It owns no cache of its own: every editing call
writes a file and reads the folder back, so what the list shows is what a text
editor would show.

- A group lists its own snippets **and** those of every group inside it. One
  rule, so the root is the whole library and "All Snippets" is not a special
  case. The search box narrows whatever the sidebar has chosen rather than
  replacing it; both are the one `search(SearchQuery)` call.
- A change underneath an edit never overwrites what has been typed. When the
  open snippet is dirty a reload refreshes only the advice, because a conflict
  may have arrived with the change; when it is not, the snippet is read again.
- Moving off a snippet, or closing the window, writes what was typed into it.
  The file is the record, so leaving an edit behind only in the window would be
  the surprise, not saving it.
- Nothing the core reports about a draft stops a save. A clash is advice: the
  folder is the user's, and a snippet that collides is still a file they wrote
  on purpose.
- The preview is of the body being typed, not of the file
  (`preview_draft(body)`), so it keeps up with the keystroke. It is a real
  expansion on the form's defaults, so what it shows is what goes in (L10); a
  placeholder Aralo cannot expand previews as its own source.
- The body editor is a TextKit 2 `NSTextView`, and it invents nothing about the
  grammar: `outline_draft(body)` hands back the placeholders and the problems
  the parser found, in UTF-16 code units, and the view colours and underlines
  those ranges. It is the parse an expansion runs, so what is marked is what
  would happen (L10). The problem's words are the core's, so a second shell
  says the same thing about the same body, and clicking one selects the range
  it is about.
- The Insert menu is `placeholder_choices()`: the core says what to insert and
  which part of it the reader replaces, so the menu is the format's list rather
  than a copy of it kept in the shell. The insertion goes in through AppKit's
  own text input, so one undo takes it back.
- The test field is `try_draft(draft, group, editing?)`: a `DraftTrial`, which
  is a text field in memory with an engine of its own that knows one snippet,
  the draft on screen, under the settings of the group it would be saved in.
  The shell's text view never edits itself. Each key goes to `key(KeyInput)`,
  which runs the matcher and the expansion path a real app gets, and the view
  draws `field()`: the text, the caret and any selection, in UTF-16 code
  units. A click or an arrow key is `move_caret`, which resets the buffer as
  navigation does elsewhere. A draft with a form hands back an
  `ExpansionSession`, which the same `FormSession` drives, in a sheet; its plan
  goes to `finish` and a cancel to `cancel`. Nothing is written, a switched-off
  draft still expands here, and a nested snippet is read from the library as
  saved. While the field or its sheet has the keyboard, the live tap is
  suspended as it is for the palette, or an abbreviation typed there would
  expand twice.

- Import and export are on the window's toolbar, and Import is in the menu
  bar too. Choosing a file opens a sheet on a dry run: what the file would
  become, the format, the macro handling and the group it goes into. Every
  change of option runs the dry run again, so the sheet shows what the import
  would do rather than a guess at it, and nothing is written until the user
  presses Import. The report then lists the snippets that need an edit first,
  each with the core's note and a button that opens it, then the ones left
  out, then the rest. `LibraryImport` in AraloKit is the sheet apart from its
  window. Export writes the group the sidebar has selected, which at the root
  is the whole library, in a format Aralo also reads.

## The search palette

A hot key anywhere (⌃⌥⌘Space), a few letters, and Enter puts the snippet into
the app the user was already in. It is the way to reach a snippet whose
abbreviation nobody remembers, and the way to use Aralo at all in an app where
typing an abbreviation is awkward.

`PaletteStore` in AraloKit is the palette apart from its window: the rows for
what has been typed, the row Enter would insert, the preview beside it, and
what happened to the last pick. The ranking is the core's `search(SearchQuery)`
with `enabled_only`, so the palette and the snippet window agree about what
matches; an empty query is the `recents(limit)` list, then the rest of the
library in its own order. A switched-off snippet does not expand when it is
typed, so it is not offered here either.

- The window is a non-activating `NSPanel`. The app underneath stays the
  frontmost one and keeps its insertion point, so nothing has to be restored
  afterwards.
- While the palette has the keyboard, the tap is suspended
  (`ExpansionController.setSuspended`) and the buffer is emptied on the way in
  and on the way out. What is typed into the search box is a query, not text in
  a document, and the two sides of the palette cannot join up into an
  abbreviation neither of them typed.
- The panel gives the keyboard back **before** any text is typed, in that
  order: synthetic keys follow the keyboard, and a snippet inserted with the
  palette still up would land in the search box. The buffer is cleared first
  for the same reason: clearing it takes the undo record with it, and the
  snippet about to land has to be undoable.
- The plan runs through the same `ExpansionController` and the same serial
  injector queue as a typed expansion, and arms the same undo. To the user it
  is the same thing arriving by another route: one undo key takes it back, and
  because nothing was typed to ask for it, nothing is retyped in its place.
- A pick is refused while Aralo is paused and in the apps it stays out of (P2,
  E10). Neither refusal is about the snippet, and the palette is the one place
  a user is *told* why nothing was inserted: an expansion they typed into a
  password manager simply does not happen, but a snippet they picked from a
  list and watched do nothing needs an answer. So a refusal keeps the window
  up, with the words the core's reasons are given in the shell.

## The form panel

A snippet whose body asks something — a `{{field: …}}` to fill in, a
`{{choice: …}}` to pick from — cannot be a plan the moment the abbreviation
completes. `on_key` answers `StartSession` instead: the key that matched is
swallowed, nothing is typed, and a panel asks the questions.

`FormSession` in AraloKit is the panel apart from its window: the fields to
draw, the answers as they stand, the preview of what Enter would insert, and
whether the session is over. It drives one `ExpansionSession` — the form, then
whatever the body wants from outside it, then the plan — and knows nothing
about windows or injectors. What it hands over goes to an `ExpansionRunner`,
which in the app is the same `ExpansionController` the tap talks to.

- Asking changes nothing. The preview is `preview_with(answers)`, so it is the
  expansion itself run on a copy, and a user who opens a form and thinks better
  of it has typed nothing anywhere.
- The fields open on their defaults, and a drop-down on its first choice, so a
  form is answerable by pressing Enter.
- A field that asked for lines (`{{field: notes | lines: 4}}`) is drawn as a
  box with room to write in, that many lines tall. Return then belongs to the
  box rather than to the Insert button, so such a form inserts with Command
  and Return, and the panel says so.
- The context the body wants is fetched without asking the user: a snippet that
  wants the clipboard is not a snippet that wants a dialogue about the
  clipboard. Only what `SessionAction.Context` names is read, and only then, so
  a body that never mentions the clipboard never causes the pasteboard to be
  touched. A pasteboard with no text on it is an empty clipboard, not a missing
  one — copying an image must not leave `{{clipboard}}` in the document.
- A body that asks only for context has no panel at all: the session runs to
  its plan while the model is being built, and `FormSession` is finished before
  a window could be shown.
- The window is a non-activating `NSPanel` on the palette's terms, and gives
  the keyboard back **before** it submits, for the same reasons
  ([The search palette](#the-search-palette)). Clicking away is a way of saying
  never mind.
- Cancelling puts back what the match swallowed and nothing else: one
  keystroke, typed, with nothing to delete because nothing was inserted.
- One panel per session. The boxes belong to the snippet that asked, and the
  last snippet's are not this one's.
- The tap is suspended while any window of Aralo's own holds the keyboard, and a
  form panel can open from the palette, so the two are counted rather than
  flagged: the palette closing must not start the tap while the panel is still
  up.

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
| `cursor` | `mxcursor` | the next key lands where `{{cursor}}` was. The expectation is read from the plan's `MoveCursor` steps |
| `undo` | `mxascii`, then Cmd+Z | the abbreviation is back and the expansion is gone |
| `clipboard` | `mxlong` | the text arrives and the clipboard holds what it held before |
| `form` | `mxform`, then an answer and Return | the form panel takes the delimiter, and the answer typed into it arrives in the app |

The text a case expects is never written down twice. The harness opens the
same library through the bridge, feeds the abbreviation to `Engine.on_key`
and reads the plan, so it compares the field with what the core says, under
the very table row Aralo will use (`compat_apps` lists the rows). A forced
method is a generated table handed to both through `ARALO_COMPAT`. A body that
asks something comes back as a session rather than a plan, and the harness
drives it the way the panel does, with its own answer in every box; a body that
wants the clipboard or the selection is the shell's to fetch, so the harness
says so instead of comparing the field with a guess.

**The form case.** The panel is Aralo's own window and takes the keyboard
without activating, so the app under test stays frontmost and the harness reads
its field as before. There is nothing to watch for — a swallowed delimiter looks
like a delimiter the app has not shown yet — so the panel gets `--form-pause`
(0.75 s) before the answer is typed and Return pressed. An answer that turns up
in the document instead says exactly that: the panel was not up in time, which
is a slow machine, not a broken expansion, and the case is retried like any
other.

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

## The folder store

The library folder is the only source of truth
([ADR-0006](adr/0006-files-as-source-of-truth.md)). Everything else about it is
a cache that can be deleted and rebuilt. `aralo-library` holds four pieces:
the loader, a watcher, a SQLite index and the merge that folds a sync client's
conflict copy back into its snippet.

**The watcher** (`Watch`) wraps `notify` — FSEvents on macOS, inotify on Linux
— on a thread of its own, and calls back with a debounced batch of paths once
the folder has been quiet for 200 ms. One save in a text editor is several file
system events and a sync client rewrites a folder file by file, so the
debounce is what turns a burst into one reload. A batch that contains a
`_group.yaml` or the manifest is flagged `groups_changed`, because inherited
settings reach snippets that did not change themselves.

Only files the loader would read are reported: `.md` snippets, `_group.yaml`,
and `aralo.yaml` at the root. Hidden and underscored names are skipped, which
also covers the editor scratch files and the `.{name}.{pid}.aralo-tmp` file
that an atomic write renames into place.

**Aralo's own saves** reach the watcher too, and reloading because Aralo just
saved is work with no result. Suppressing by path would be wrong: it would
swallow a real edit that lands on the same file in the same moment, which is
exactly what happens when a sync client delivers a change while the user is
typing. So a save is remembered by content. `Library::write_snippet` records
the blake3 hash of the bytes it is about to write — before writing, because an
event can arrive while the write is still returning — and an event is dropped
only when the file still holds exactly those bytes. A different hash is someone
else's edit and is always reported. Each record is spent on the first event
that matches it.

**The index** (`Index`) is a SQLite file with FTS5, and it lives in the
application's own folder, never inside the library: a SQLite file in a synced
folder is a corruption waiting to happen.

| Table | Holds | Rebuildable |
| --- | --- | --- |
| `snippets` | One row per loaded snippet: id, path, group, content hash, front matter and body | Yes |
| `groups` | The folder tree the snippets describe, with the count in each | Yes |
| `snippets_fts` | FTS5 over label, abbreviation, tags and body | Yes |
| `stats` | Expansion counts and last-used time | **No.** Local only, never synced |
| `bases` | Each snippet's last settled file, the base a conflict copy merges against | **No.** Only this machine saw it |
| `vectors` | Embedding blobs keyed by content hash and model | **No.** Hours of compute |

`sync` is the usual path: it compares each snippet's content hash, path, group
and enabled flag against the row already there and writes only what differs, so
a folder nobody touched costs one pass and no writes. `rebuild` clears the
three derived tables and writes everything; it never touches the three that are
not rebuildable. The two must agree, and `Index::rows` is what checks that —
it dumps every derived row in a fixed order, and the test suite asserts that an
incremental sync lands exactly where a rebuild lands.

The content hash covers a snippet's content only: not its path, not its group,
not the enabled flag it inherits, all of which are compared on their own. That
leaves the hash stable across a rename or a move, which is what lets `vectors`
key on it. `PRAGMA user_version` guards the schema; an index written by an
older Aralo is dropped and rebuilt rather than migrated, because it is a cache.

A file that carries no `id` gets one derived from its path, so a hand-written
library does not look entirely new on every load. It changes when the file
moves, which is the whole truth about a file with nothing else to identify it.
The first save Aralo makes writes a real ULID.

**Nothing on the keystroke path reads any of this.** Abbreviations reach the
engine through `Library::snapshot`, which is built from the loaded files alone,
so a cold index of a large library is a background job that expansion does not
wait for.

### The library, running

`Core` is a value: it is read and written on whatever thread holds it.
`aralo_core::Runtime` is that value with the two background jobs a running app
needs around it, and it is what a shell holds for the lifetime of the app. It
owns the watch and one indexer thread, and it hands changes to a
`LibraryListener` the shell implements.

The indexer thread owns the only connection that writes to the index, so a
rebuild cannot collide with a save, and reads are served from a second
connection: SQLite in WAL mode allows one writer and any number of readers at
once, so an editor's list never waits for a rebuild. Counting an expansion is a
message to that thread, which is why `expansion_done` returns without touching
a file.

The index is a cache, and a runtime treats it as one. A cache that will not
open is recorded and stepped over: the library still loads, still expands,
still saves, and `index_error()` says what went wrong. What is lost is recents
and usage counts, which is a worse menu, not lost work. Where the cache lives
is [ADR-0011](adr/0011-shell-defaults.md)'s: the `Aralo` folder inside
`~/Library/Application Support`, `ARALO_STATE` overriding it, or a folder the
shell names, because a sandboxed app's is not where an unsandboxed one's is.

A change reaches the matcher before it reaches the shell: the listener is given
the library as it now stands, sets the engine's new snapshot from it, and only
then tells the shell. So the next keystroke matches what the folder says even
if no window has drawn yet. Where both locks are held, the library is taken
first and the matcher second, on every thread; the keystroke path takes one at
a time.

### Conflict copies

Two machines that change one snippet before either sees the other's change
leave the sync client holding two files. Every client keeps both, under a name
of its own making: `sig (conflicted copy 2026-09-23).md` (Dropbox, Nextcloud),
`sig (1).md` (Google Drive, Box), `sig 2.md` (iCloud Drive),
`sig.sync-conflict-….md` (Syncthing), `sig_conflict-….md` (ownCloud) and
`sig-MACHINE.md` (OneDrive).

A name alone is not enough to go on, because `Invoice 2.md` may be a snippet
the user made on purpose. The loader pairs a file with an original only when
all three hold: the two sit in one folder, they carry one `id`, and the name is
one of those a client gives a copy of the original's name. The original is
loaded and the copy is left out, reported as `ConflictCopy` rather than
`DuplicateId`. Anything else with a contested `id` is still a duplicate. When
several files claim one `id`, the original is the one the most others are
copies of.

The merge is three-way ([ADR-0008](adr/0008-crate-choices.md) picks `diffy`).
The base is the version this machine last saw settled, which the index keeps in
`bases`, keyed by the hash of the file's raw bytes. The index's content hash
would not do, because it misses keys such as `case`. A base is not moved while
a copy of its snippet is waiting, so it stays the common ancestor of both
sides. The front matter merges key by key, keys this version does not know
included, and the body merges line by line. A side that left something as the
base had it takes the other side's change. Both sides making one change is that
change. Anything else is a clash, and nothing is guessed. With no base, only
what the two sides agree on merges.

`Runtime` merges whenever it reads the folder: when it opens, after the watcher
reports a change, and on `reload`. A clean merge is written over the original
before the copy is discarded, so a failure between the two leaves a copy that
merges cleanly again the next time. The shell hears `LibraryChange::Merged`
with the copies that went. A clash leaves both files where they are, and the
diagnostic tells the user to keep one.

A clash waits for the user. `Core::conflict(copy)` returns both files as they
are on disk, the base when the index has one, and what the two changed
differently: the front-matter keys, and whether the body's lines overlap.
`Core::resolve_conflict(copy, resolution)` takes one of three decisions: keep
the original, keep the copy, or keep a version the user wrote. Whichever is
kept is written into the original's file under the snippet's own `id`, and the
copy is discarded as a clean merge's is. Text that is not a snippet file is
refused before anything moves. The runtime runs a resolution as an edit, so the
index catches up and the shell hears `Edited`.

Discarding goes through a `Discard` the shell supplies. The Mac shell passes a
`Trash` (`SystemTrash`, which uses `FileManager.trashItem`), so a merged or
resolved copy lands in the Finder's Trash and Put Back returns it. A shell that
passes none gets the copy set aside in `<cache>/merged/<library>/`, named after
the moment it moved, and the command line does the same. A `Trash` that refuses
leaves the copy where it was and the conflict still waiting.

On the Mac, a waiting copy shows in two places: a toolbar menu in the snippet
window lists each one, and the snippet it belongs to carries a banner. Both
open the resolver, a sheet with the two files side by side, the base under a
disclosure, a sentence naming what clashed, and the three ways out.
`ConflictResolver` in AraloKit is that sheet apart from its view.

### Editing

`Core`'s editing calls are the other half of ADR-0006: every one of them writes
files and reads the folder again, so what the next `snippets()` returns is what
a text editor would show. A `Draft` carries the fields an editor puts on screen
and nothing else — saving one leaves the keys this version does not know, and
the `ai:` block, exactly as they were on disk, so an older Aralo cannot quietly
strip a newer one's file. `check_draft` reports blank, repeated and
already-taken abbreviations, and reports rather than refuses: the folder is the
user's, and a save is never blocked by advice.

## Bridge API

The bridge is `crates/aralo-ffi`, generated by UniFFI. Nothing in it may
panic: a panic would cross the bridge as a crash in the process that holds
the event tap.

**Today:**

| Object | Calls | Kind |
| --- | --- | --- |
| `Engine` | `on_key(KeyInput) -> KeyAction`, `insert(snippet_id, into_app) -> InsertOutcome`, `expansion_done(snippet_id, delete_count, method)`, `reset(reason)`, `set_front_app(bundle_id)`, `injection_profile()`, `set_paused(bool)`, `is_paused()`, `set_excluded_apps(bundle_ids)`, `holds_no_keystrokes()` | Synchronous |
| `Core` | `open_library(path, cache?, events?, trash?)` (constructor), `engine()`, `reload()`, `load_compat_table(path)`, `library_path()`, `snippets()`, `diagnostics()`, `index_problem()` | Synchronous |
| `Core`, editing | `create_snippet`, `save_snippet`, `delete_snippet`, `move_snippet`, `set_snippet_enabled`, `snippet(id)`, `create_group`, `rename_group`, `move_group`, `delete_group`, `set_group_enabled`, `set_group_appearance`, `groups()`, `group_contents` | Synchronous |
| `Core`, the editor's questions | `check_draft(draft, editing?)`, `suggest_abbreviation(label)`, `search(SearchQuery)`, `preview(id)`, `preview_draft(body)`, `outline_draft(body)`, `try_draft(draft, group, editing?) -> DraftTrial`, `recents(limit)` | Synchronous |
| `DraftTrial` | `key(KeyInput) -> TrialAction` (`Typed`, `Expanded` or `Session { session }`), `field() -> TrialField`, `move_caret(caret)`, `clear()`, `finish(steps, undo_delete_count?)`, `cancel(session)`, `abbreviations()`. Offsets are UTF-16 | Synchronous |
| `Core`, the locale | `set_locale(tag)`, `locale()`: which locale `{{date}}` is written in. The Mac shell passes `Locale.current.identifier`; a terminal falls back to `LC_ALL`, `LC_TIME` or `LANG` ([ADR-0014](adr/0014-clock-and-locale.md)) | Synchronous |
| `ExpansionSession` | `next()`, `submit_form(answers)`, `provide_context(values)`, `preview()`, `preview_with(answers)`, `cancel()`, `snippet_id()`. Asking twice asks the same question; `preview_with` is what a panel redraws as the user types, and changes nothing; `cancel` hands back the steps that put the typed key back | Synchronous |
| `Core`, interchange | `import(source, settings)`, `export(format, group)` | Synchronous |
| `Core`, sync conflicts | `conflicts()`, `conflict(copy) -> ConflictDetail`, `resolve_conflict(copy, ConflictChoice)`: `KeepOriginal`, `KeepCopy` or `Write { text }` | Synchronous |
| `CoreEvents` | `library_changed(event)`: `Outside`, `Edited`, `Reloaded`, `Failed`, `Merged`, `Indexed`. The shell implements it | Called on a background thread |
| `Trash` | `discard(path)`: moves a merged or resolved conflict copy out of the library. The shell implements it | Called on whichever thread read the folder, with the library locked |
| Functions | `core_version()`, `excluded_app_presets()`, `placeholder_choices()`: what an editor's insert menu offers, `compat_apps(path?)`: the rows of a table, for the injection matrix | Synchronous |

`KeyAction` is one of `Pass`, `Expand { snippet_id, consume, steps,
undo_delete_count, profile }`, `StartSession { snippet_id, consume, session }`
or `UndoExpansion { delete_count, retype, method, profile }`. A session ends
in `SessionAction.Expand`, which carries the same steps and profile an
`Expand` would have.
`set_excluded_apps` always keeps the built-in password-manager presets, so a
shell that forgets to pass them cannot drop them.
`insert` is the same expansion asked for by identity rather than by typing: it
answers `Insert { snippet_id, steps, undo_delete_count, profile }`,
`StartSession { snippet_id, session }` or `Refused { Paused | ExcludedApp |
SnippetGone }`. It names the app the text is for, and
the core takes that as the app holding the keyboard, so the later report that
the app came forward changes nothing and undo survives it.

**Planned** (plan section 3; names may change):

| Object | Calls | Arrives |
| --- | --- | --- |
| `ExpansionSession` | `regenerate`: ask a model for another answer, for an AI block | M4 |
| `AiProfiles` | `save`, `test_connection`, `probe_capabilities`, `list_models`, `detect_local_servers` | M4 |
| Callbacks the shell implements | `ContextProvider`, optional `SecretStore` | M4. A session asks the shell for context by returning `SessionAction.Context`, so nothing is needed for the clipboard |

An import comes back as the report the app shows
([ADR-0013](adr/0013-import-and-export.md)), with a dry run that reads the
source and writes nothing. After a real import each entry also names the
snippet it became, so the report can open the ones that need an edit. A search hit carries the characters that matched, so
a list highlights them without searching again, and an empty query is the whole
library in list order, which makes a list and its search box one call.

## Crate graph

Dependencies point one way: down. `scripts/check-deps.sh` holds this table
and fails CI when a crate depends on its own layer or a higher one. Layer 1
is the exception: its crates may depend on each other.

| Layer | Crates | State today |
| --- | --- | --- |
| 3 | `aralo-ffi`, `aralo-cli` | The bridge carries the keystroke path, editing, search and interchange. The CLI also imports, exports, searches and expands |
| 2 | `aralo-core` | Open a library, build a plan, the compatibility table, in-memory simulator, import, export, search, editing, and the runtime that watches and indexes |
| 1 | `aralo-library`, `aralo-ai`, `aralo-embed`, `aralo-providers`, `aralo-import`, `aralo-script` | `aralo-library` loads, resolves inheritance, writes atomically, watches, indexes and searches. `aralo-import` reads four formats and writes three. The rest are empty |
| 0 | `aralo-engine`, `aralo-snippet`, `aralo-template` | Implemented, evaluator included: dates and times in 15 locales, the clipboard, forms, nested snippets and cursor stops. An AI block inserts its fallback until M4 |

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

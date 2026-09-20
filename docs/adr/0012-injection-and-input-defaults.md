# ADR-0012: Per-app injection is data; input defaults of the M1 shell

- Status: Accepted (not yet confirmed by the project owner)
- Date: 2026-09-20

## Context

M1 turns the vertical slice into a text expander that has to behave in 15
very different apps, on any keyboard layout. That forced five choices the
plan leaves open or only names. The first one amends
[ADR-0011](0011-shell-defaults.md), which said text with a line break is
never typed.

## Decision

- **How text goes into an app is a row of a table, not code.**
  `data/compat/apps.toml` maps a bundle ID to an insert method, a typing
  limit, two delays, an undo style and a delete strategy
  (`schemas/compat.schema.json`). The table is compiled into `aralo-core`.
  The core resolves the front app's row when the app changes and attaches it
  to every `Expand` and `UndoExpansion`; the injector follows it and decides
  nothing. ADR-0011's rule (paste anything long or multi-line) stays as the
  table's default, `insert = "auto"`.
- **Where an app is typed into, a line break is a Return key, and that
  expansion offers no undo.** Terminals and remote desktops are typed into
  because a paste there can ask for confirmation, arrive bracketed or land in
  another machine's clipboard. The app may have acted on the Return (a shell
  ran the line), so Backspace cannot take the expansion back.
- **The table is strict.** Unknown keys, duplicate bundle IDs, a newer format
  version and numbers past fixed bounds (250 ms per key, 5 s paste settle)
  are errors, and a refused table leaves the one in use untouched. The table
  will later arrive as a signed download; a bad one must not be able to stall
  the injector or pass a typo in silence.
- **The tap translates keys itself.** The string on a key event is the key on
  its own, so after a dead key the letter arrives unaccented while the
  document gets "é". The tap runs `UCKeyTranslate` with the dead-key state
  kept between keys, on layout data fetched on the main thread. A key that
  cancels a waiting dead key resets the engine. While the selected input
  source composes text (any input method except its Roman mode), no key
  reaches the engine.
- **Pause has a global shortcut: Control+Option+Command+P.** It is registered
  through the Carbon hot-key API, by the character on the key, and follows
  the keyboard layout. It needs no permission and does not pass through the
  event tap. It is fixed until the settings screen arrives in M2.
- **A running tap is proof of Input Monitoring.** The first run lets the user
  continue when `CGPreflightListenEventAccess` says yes or the tap is up,
  whichever comes first. The shell also watches for Accessibility being
  revoked and takes the tap down when it is, rather than leave a tap up that
  macOS has stopped feeding.

## Consequences

- A compatibility fix is a pull request against one TOML file, reviewable by
  someone who does not read Swift or Rust. The app compatibility issue form
  feeds it.
- Every value in the table today is unmeasured. The three overrides come
  from the plan's rule about terminals and remote desktops. Spikes S1 and S6
  replace them with measurements; until then the table is a hypothesis.
- `toml` joins the dependency tree of `aralo-core`
  ([ADR-0008](0008-library-choices.md)).
- An expansion that pressed Return in a terminal cannot be undone with one
  key. That is the honest answer: the command may already have run.
- Users of Japanese, Chinese and Korean input methods get no expansion while
  composing. It fails safe; full support is E17 in v1.
- The pause shortcut may collide with an app's own. If the system refuses the
  registration, the menu shows no shortcut and pause stays in the menu.
- The name of the app that holds secure input comes from
  `kCGSSessionSecureInputPID` in the window server's session dictionary. The
  key is in no public header. When it is absent the menu names nobody.

## Alternatives rejected

- **Per-app behaviour in Swift.** Every fix would need a release, and a
  second shell would have to copy it.
- **Letting the shell look the app up.** The shell would then decide
  something, and the core could not test it on Linux.
- **YAML for the table.** The library is YAML because people write prose in
  it. A table of short rows is easier to review as TOML, and provider quirks
  will use the same form.
- **Shift+Return for typed line breaks.** Apps disagree on what it means
  (ADR-0011).
- **Reading dead keys from the event.** The event does not carry them.
- **Detecting the pause shortcut in the event tap.** It would not work before
  Accessibility is granted, nor while secure input is on.

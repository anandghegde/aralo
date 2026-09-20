# ADR-0011: Defaults of the first Mac shell

- Status: Accepted (confirmed by the project owner, 2026-09-20)
- Date: 2026-09-20

## Context

Building the M0 vertical slice forced four small choices that neither the PRD
nor the plan makes. Each one is visible to users or to contributors, so they
are written down here instead of living only in code.

## Decision

- **The default library folder is `~/Aralo`.** The `ARALO_LIBRARY` environment
  variable overrides it. A settings screen for the folder arrives with the
  library UI in M2.
- **The bundle identifier is `app.aralo.Aralo`.** The injector queue and tap
  thread use the same `app.aralo.` prefix for their labels.
- **The bridge always keeps the built-in exclusions.**
  `Engine.set_excluded_apps` in `aralo-ffi` appends the password-manager
  presets to whatever list the shell passes. The engine crate itself replaces
  its list as given.
- **Text with a line break is pasted, never typed.** So is text longer than
  120 UTF-16 units. Shorter single-line text is typed, in events of at most 20
  units.

## Consequences

- The library is a visible folder in the home directory, which suits "your
  snippets are plain files" and makes sync clients and editors easy to point
  at it. It is not inside `~/Library`, so removing the app leaves it alone.
- macOS ties the Accessibility grant to the bundle identifier and the code
  signature. Changing the identifier later costs every user a new grant.
- A shell bug, or a user who clears the exclusion list, cannot make Aralo
  expand inside a password manager. A user who truly wants that cannot have
  it without a code change.
- A typed Return submits forms and sends messages in many apps. Pasting keeps
  a multi-line snippet from sending half a message. The cost is the
  pasteboard round trip: about a quarter of a second, and clipboard managers
  that ignore the transient markers may record the snippet.

## Alternatives rejected

- **`~/Library/Application Support/Aralo`.** Hidden from the user, which works
  against files as the source of truth
  ([ADR-0006](0006-files-as-source-of-truth.md)).
- **Presets as plain defaults the shell may drop.** One forgotten argument
  would remove a privacy protection without anyone noticing.
- **Typing line breaks as Shift+Return.** Apps disagree on what it means.

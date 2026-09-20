# ADR-0005: `aralo-engine` has no I/O, clock, network or logging dependency

- Status: Accepted
- Date: 2026-09-20

## Context

Aralo reads every key the user types. The PRD promises that keystrokes stay in
memory, 64 characters at most (P1). A text expander that breaks this promise
is a keylogger. Users and reviewers need a way to check the promise that does
not depend on trusting the maintainers.

## Decision

All code that holds typed characters lives in one small crate,
`aralo-engine`, and that crate is kept pure.

- The buffer is a fixed ring of 64 characters. It is zeroed on every reset
  with `zeroize`.
- The crate has no file, network, clock or logging dependency. **CI fails the
  build if one appears in its dependency tree.**
- The engine holds only abbreviations, flags and snippet IDs. Snippet bodies
  are not in it.
- Everything the engine needs from outside arrives as plain data through its
  calls: `on_key`, `reset(reason)`, `set_front_app(id)`, `set_paused(bool)`,
  `expansion_done(record)`.
- The Swift tap passes events straight to the engine and keeps no copy.
- `aralo-engine` and the tap source are protected by code owners.

## Consequences

- P1 becomes a property of the build, not a promise. A reviewer can verify
  "not a keylogger" by reading one small crate.
- A test checks that the buffer is zero after every reset reason.
- The engine cannot log, so its behaviour is pinned by tests instead: property
  tests against a naive reference matcher, and a fuzz target on `on_key`.
- The crate compiles to `wasm32-unknown-unknown`. The `wasm` CI workflow
  builds it so the browser path never rots.
- Purity is not a ban on all dependencies. `zeroize` is one. In v1, regex
  triggers use the `regex` crate, which guarantees linear time. Each new
  dependency still has to pass the dependency-tree check.
- Contributors who want to add tracing or timing to the matcher must do it
  outside this crate.

## Alternatives rejected

- **State the rule in documentation and rely on review alone.** A rule that
  only review enforces can break silently, for example through a transitive
  dependency. A failing build cannot be missed.
- **Keep the matcher inside a larger crate such as `aralo-core`.** The
  audit surface would grow to everything that crate depends on, and the
  "read one small crate" property would be lost.

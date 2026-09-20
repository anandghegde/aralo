# ADR-0002: UniFFI for the bridge between core and shell

- Status: Accepted
- Date: 2026-09-20

## Context

ADR-0001 splits Aralo into a Rust core and native shells. The two sides need a
bridge that:

- carries synchronous calls on the keystroke path (`Engine.on_key`),
- carries async calls and an async state machine (`Core`, `ExpansionSession`,
  `AiProfiles`),
- lets the shell implement callbacks the core calls (`ContextProvider`,
  `CoreEvents`, an optional `SecretStore`),
- can serve a C# shell on Windows later without a second design.

## Decision

Use UniFFI with proc-macros. The `aralo-ffi` crate holds the exports and the
build script. The output is an XCFramework wrapped in a Swift package.
`make bootstrap` builds it.

The bridge surface is:

| Object | Main calls | Kind |
| --- | --- | --- |
| `Engine` | `on_key(KeyEvent) -> KeyVerdict`, `reset(reason)`, `set_front_app(id)`, `set_paused(bool)`, `expansion_done(record)` | Synchronous |
| `Core` | `open_library(path)`, `save_snippet`, `search(query, context)`, `import(path, kind)`, `export(scope, format)`, `preview(snippet, answers)` | Async |
| `ExpansionSession` | `next() -> Step`, `submit_form(answers)`, `provide_context(bundle)`, `regenerate()`, `cancel()` | Async state machine |
| `AiProfiles` | `save`, `test_connection`, `probe_capabilities`, `list_models`, `detect_local_servers` | Async |
| Callbacks the shell implements | `ContextProvider`, `CoreEvents`, optional `SecretStore` | Foreign traits |

`KeyVerdict` is one of `Pass`, `Match { snippet_id, delete_count, consume }` or
`UndoLast { delete_count, retype }`. Everything that crosses the bridge is
plain data.

## Consequences

- UniFFI generates Swift and Kotlin today. `uniffi-bindgen-cs` adds C# for the
  Windows shell in v1. It supports async functions and callback interfaces,
  which the surface above needs.
- The bridge API is message-shaped. ADR-0003 relies on that.
- **Open question, spike S2.** The cost of an `on_key` call across UniFFI, and
  whether it can run safely on the tap thread, are not yet measured. S2
  benchmarks 1 million calls and tests callback threading. The tap thread has
  a 1 ms budget per key.
- **Fallback if S2 goes badly.** Expose `on_key` alone through a hand-written
  C function and keep UniFFI for everything else.

## Alternatives rejected

- **A hand-written C ABI for the whole surface.** More work than UniFFI for
  the same result, with more unsafe code. It stays available for the single
  `on_key` call as the S2 fallback.

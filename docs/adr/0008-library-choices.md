# ADR-0008: Third-party library choices

- Status: Accepted
- Date: 2026-09-20

## Context

The core needs an async runtime, HTTP, an index, search, file watching,
embeddings, dates, diffing, secret storage and, later, a script sandbox, Git
and MCP. The Mac app needs an updater. Picking these once, in one place, keeps
the dependency tree small and reviewable.

**These picks come from working knowledge, not a fresh survey.** Milestone M0
pins versions and checks each library's licence and maintenance status with
`cargo-deny`. A pick that fails that check is replaced, and a new ADR records
the change.

## Decision

| Area | Choice | Why |
| --- | --- | --- |
| Async runtime | `tokio`, for AI calls, file watching and indexing only | The keystroke path is synchronous and never touches the runtime |
| HTTP and streaming | `reqwest` with `rustls`; server-sent events parsed in the adapters | No OpenSSL to bundle. One client type makes the network guard enforceable |
| Index | SQLite through `rusqlite` (bundled build) with FTS5 | One file, easy to delete and rebuild, full-text search included |
| Fuzzy search | `nucleo-matcher`, the matcher half of `nucleo` | Fast fuzzy matcher built for interactive pickers. The library is searched in one pass over snippets already in memory, so `nucleo`'s threaded picker and injector are weight with no work to do |
| File watching | `notify` | Wraps FSEvents on macOS and ReadDirectoryChangesW on Windows |
| Content hashing | `blake3` | One hash serves three jobs: deciding what the index has to reparse, telling Aralo's own saves from someone else's edits, and keying the embedding cache. Fast enough that hashing a whole library costs less than reading it |
| Embeddings | `candle` running a quantised MiniLM-class sentence model; `ort` as fallback | Pure Rust means no third-party dynamic library to sign and notarise. Spike S5 confirms speed first |
| Dates | `jiff` for time-zone-safe arithmetic, ICU4X for locale formats | Date maths across DST and month ends is where naive code fails |
| Diff and merge | `similar` for previews, `diffy` for three-way merge | Conflict merge and the AI diff preview share one text model |
| Secrets | A `SecretStore` trait; default implementation on the `keyring` crate | The app and the CLI read the same keychain items. Shells can override it |
| Script sandbox (v1) | `rquickjs` | QuickJS is an interpreter with no JIT, so the hardened runtime needs no JIT entitlement. It has memory and interrupt limits |
| Git groups (v1) | `git2` | Works without Git installed on the user's machine |
| MCP server (v1) | `rmcp`, the official Rust SDK | Runs inside the CLI over stdio |
| Data tables | `toml`, parse only, for `data/compat/apps.toml` and later the provider quirks table | Tables that people edit by hand and review in pull requests need comments and no indentation rules. Library files stay YAML |
| App updates | Sparkle 2 with EdDSA-signed appcasts on GitHub | The standard updater for apps outside the Mac App Store |

Two things are deliberately not libraries:

- **Vector search** is a brute-force cosine scan over an in-memory matrix,
  persisted as SQLite blobs. 10,000 vectors of 384 floats is about 15 MB and
  scans in a few milliseconds. No ANN library is needed.
- **The placeholder parser** is hand-written recursive descent. The grammar is
  small, and the editor needs precise error positions.

## Consequences

- None of these may become a dependency of `aralo-engine` (ADR-0005).
- The fuzzy matcher is used on short fields only: abbreviation, label, tag and
  group. Bodies are matched as literal case-insensitive substrings, because
  fuzzy matching over a paragraph finds letters scattered across it and calls
  that a hit, and because ADR-0013's promise that a search finds every macro an
  import could not convert needs the literal kind. The two scores are not
  comparable, so results rank by which field matched first and by score within
  a field.
- `reqwest` clients are constructed only by the network guard (ADR-0007).
- `notify` is offered under CC0-1.0 and nothing else, which the licence
  allow-list does not include. It has a per-crate exception in `deny.toml`
  rather than a new entry on the allow-list, so that the next public-domain
  dependency is read on its own merits instead of arriving already allowed.
- The content hash covers a snippet's content and nothing else: not its path,
  not its group, not the enabled flag it inherits. Those are compared on their
  own, which leaves the hash stable across a rename or a move — the property
  that lets the `vectors` table key on it and survive both.
- The embedding choice is provisional until spike S5 reports model size, load
  time, per-snippet embed time and memory against `ort`. If `candle` falls
  short, the fallback is `ort` with its dynamic library signed, or downloading
  the model on first use to keep the DMG small.
- The v1 rows (`rquickjs`, `git2`, `rmcp`) are not needed for the MVP.
- Supply-chain checks apply to all of them: `cargo-deny` for licences and
  advisories, and `cargo-vet` or an equivalent for new dependencies.

## Alternatives rejected

- **OpenSSL-backed TLS.** It would have to be bundled. `rustls` avoids that.
- **An approximate-nearest-neighbour library for vector search.** Not needed
  at 10,000 vectors.
- **A parser generator for placeholders.** The grammar is small and the editor
  needs precise error positions.
- **A JIT-based script engine for the v1 sandbox.** It would need a JIT
  entitlement under the hardened runtime.

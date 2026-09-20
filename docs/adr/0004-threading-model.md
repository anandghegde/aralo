# ADR-0004: Threading model

- Status: Accepted
- Date: 2026-09-20

## Context

Aralo runs in one process (ADR-0003). That process handles key events with a
1 ms budget, posts synthetic events into other apps, draws windows, and runs
slow work: AI streams, file watching, indexing and embedding. macOS disables
an event tap that stalls, so the slow work must never be able to delay a key.

## Decision

Four kinds of thread, each with one rule.

| Thread | Runs | Rule |
| --- | --- | --- |
| Tap thread | A run loop with the `CGEventTap`; calls `engine.on_key` synchronously | Never blocks, never waits on a lock, 1 ms budget |
| Injector queue | A serial queue that posts synthetic key events and does pasteboard work | The only code that posts events. It tags them so the tap ignores its own output |
| Main thread | AppKit and SwiftUI | No file, network or model work |
| Core runtime | tokio with two workers: AI streams, file watcher, indexing, embedding | Publishes a new immutable matcher snapshot when the library changes |

The matcher data is an immutable snapshot. The engine holds only
abbreviations, flags and snippet IDs. When the library changes, the core
runtime builds a new snapshot and the engine swaps it in atomically. The tap
thread never waits for it.

tokio is used for AI calls, file watching and indexing only. The keystroke
path is synchronous and never touches the runtime.

## Consequences

- A static snippet goes from `on_key` to a finished plan on the tap and
  injector threads alone. Forms and AI blocks move the session onto the core
  runtime and the main thread.
- The tap consumes the key that completes a match, so the target app never
  sees a delimiter it might act on.
- Synthetic events carry a tag, so the tap passes them through without feeding
  the engine.
- A library change costs a whole new snapshot rather than an in-place edit.
  That work happens on the core runtime, off the keystroke path.
- Whether the UniFFI call is cheap and safe enough on the tap thread is spike
  S2 (see ADR-0002).

## Alternatives rejected

These are ruled out by the rules above rather than by a separate evaluation.

- **A mutable matcher behind a lock shared with the library.** The tap thread
  must never wait on a lock.
- **Async work or the tokio runtime on the keystroke path.** The keystroke
  path has a 1 ms budget and must not depend on a scheduler.
- **Posting synthetic events from more than one place.** One serial queue
  keeps event order and makes self-event tagging reliable.

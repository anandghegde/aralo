# Architecture decision records

Each ADR records one decision: the context, what was decided, what follows
from it, and what was rejected. ADRs are short and are not rewritten after the
fact. A changed decision gets a new ADR that supersedes the old one.

Format: `# ADR-NNNN: Title`, then Status, Date, Context, Decision,
Consequences, Alternatives rejected.

## Index

| ADR | Title | Status |
| --- | --- | --- |
| [0001](0001-rust-core-native-swift-shell.md) | Rust core behind a native Swift shell | Accepted (confirmed by the project owner, 2026-09-20) |
| [0002](0002-uniffi-bridge.md) | UniFFI for the bridge between core and shell | Accepted |
| [0003](0003-single-process-architecture.md) | One process on macOS | Accepted |
| [0004](0004-threading-model.md) | Threading model | Accepted |
| [0005](0005-engine-crate-purity.md) | `aralo-engine` has no I/O, clock, network or logging dependency | Accepted |
| [0006](0006-files-as-source-of-truth.md) | The library folder is the only source of truth | Accepted |
| [0007](0007-ai-gateway-and-network-guard.md) | One AI gateway and one network guard | Accepted |
| [0008](0008-library-choices.md) | Third-party library choices | Accepted |
| [0009](0009-apache-2-licence.md) | Apache-2.0 licence | Accepted (confirmed by the project owner, 2026-09-20) |
| [0010](0010-minimum-macos-14.md) | Minimum macOS 14, universal binary | Accepted (confirmed by the project owner, 2026-09-20) |
| [0011](0011-shell-defaults.md) | Defaults of the first Mac shell | Accepted (confirmed by the project owner, 2026-09-20) |

ADRs 0001, 0009 and 0010 began as assumptions the plan made where the PRD left
a question open. The project owner confirmed all three on 2026-09-20.

## Spike ADRs still owed

Seven questions could change the design. Each gets a time-boxed spike in M0,
before the code that depends on it. A spike ends with an ADR and
measurements, not a feature.

**None of these ADRs is written yet.** They cannot be written from a desk.
Each one needs hands-on measurement in real apps or on real hardware.

| Spike | Question | If the answer is bad |
| --- | --- | --- |
| S1 Injection | Do typing and paste both work in the 15 matrix apps, and with which delays? | Add a third method: setting the focused element's value through the Accessibility API |
| S2 Bridge on the hot path | What does an `on_key` call cost across UniFFI, and can it run safely on the tap thread? | Expose `on_key` alone through a hand-written C function and keep UniFFI for everything else |
| S3 Caret bounds | How often does the Accessibility API return a usable caret rectangle? | Accept the fallback order for panel placement, and position near the mouse more often |
| S4 Pasteboard restore | Can every item type be restored, including promised files and large images, and how long must the wait be? | Skip the restore when the clipboard holds promised data, and prefer typing in those moments |
| S5 Embedding runtime | Is `candle` fast and small enough for a MiniLM-class model? | Use `ort` and sign its dynamic library; or download the model on first use to keep the DMG small |
| S6 Undo semantics | What do Backspace and Cmd+Z do after each insert method, per app? | Ship Backspace undo only in the MVP and document Cmd+Z per app in the compatibility table |
| S7 Typinator export | What is the exported set format, and which macros appear in real libraries? | Import through Typinator's text or CSV export first and accept lower macro fidelity in the MVP |

Spike ADRs take the next free numbers. Each must include the method used and
the measurements taken. S2 and S5 may change ADR-0002 and
ADR-0008; if they do, the spike ADR supersedes the affected part.

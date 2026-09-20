# ADR-0001: Rust core behind a native Swift shell

- Status: Accepted (confirmed by the project owner, 2026-09-20)
- Date: 2026-09-20

## Context

Aralo ships on macOS first, then Windows and a browser extension, then iOS.
The same abbreviation must expand to the same text on all of them.

The PRD sets hard budgets: typed abbreviation to inserted text under 50 ms at
p95, under 1 ms of work per keystroke, and under 50 MB of memory with 10,000
snippets. The code that sees keystrokes must also be easy to audit.

On macOS, three things cannot be done from a web view: tapping key events,
injecting text into other apps, and showing floating panels that do not take
focus from the app the user is typing in.

The PRD left the shell technology open (PRD open question 3). Section 14 of
the plan lists this as decision 1, to be settled before M0 ends.

## Decision

Build one Rust core and put a thin native shell in front of it on each
platform. The first shell is a Swift app on macOS.

- **Rust** (stable toolchain, one Cargo workspace) owns everything that must
  behave identically everywhere: matching, placeholders, the library and every
  AI call.
- **Swift 6** owns everything macOS treats specially: keystrokes, text
  injection and windows. SwiftUI is used for the main window and settings.
  AppKit is used for the menu bar, floating panels and the snippet text
  editor.
- The shell never decides what to expand. The core never touches the
  operating system. Everything that crosses between them is plain data: key
  events in, verdicts and expansion plans out.

## Consequences

- Rust gives predictable latency with no garbage collector, and memory safety
  in the code that sees keystrokes.
- Shells stay thin. The Windows shell replaces the tap with a
  `WH_KEYBOARD_LL` hook and the injector with `SendInput`; the browser
  extension compiles `aralo-engine` and `aralo-template` to WebAssembly.
  Neither changes the core.
- The CLI uses the same `Core` API as the app. That keeps the app honest about
  what belongs in the core.
- The CLI and every crate build and test on macOS, Linux and Windows, so most
  contributors never open Xcode.
- The project carries two languages and a generated bridge between them (see
  ADR-0002).
- If the owner chooses Tauri instead, the app UI design (plan section 8) and
  the two Swift work lanes are replaced. Keyboard capture and text injection
  stay native either way, because a web view cannot tap keys or inject text.

## Alternatives rejected

- **Tauri or Electron for the shell.** The event tap, injection and
  non-activating panels need native code anyway. A web shell adds a layer
  without removing one.
- **Swift only.** Nothing would carry over to Windows or the browser.
- **A hand-written C ABI between Rust and Swift.** More work than UniFFI for
  the same result, with more unsafe code. See ADR-0002.

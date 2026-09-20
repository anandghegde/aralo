# ADR-0010: Minimum macOS 14, universal binary

- Status: Accepted (confirmed by the project owner, 2026-09-20)
- Date: 2026-09-20

## Context

The Mac app needs a deployment target before the Xcode project is created in
M0. The choice fixes which SwiftUI APIs the UI may use and how many OS
versions the injection compatibility matrix must cover. Section 14 of the plan
lists this as decision 3, to be settled before M0 ends.

## Decision

- The minimum supported version is **macOS 14**.
- Releases are **universal binaries**: Apple silicon and Intel.

## Consequences

- The app UI may use the parts of SwiftUI that the plan's UI design (section
  8) relies on.
- The compatibility matrix and the performance harness target macOS 14 and
  later only.
- Users on macOS 13 or earlier cannot run Aralo.
- The release workflow produces a universal build, so the Rust static library
  is built for both architectures.
- This applies to the Mac app only. The CLI and every crate build and test on
  macOS, Linux and Windows.

## Alternatives rejected

- **A lower minimum version.** It rules out parts of SwiftUI used in the UI
  design and adds test targets to the matrix.

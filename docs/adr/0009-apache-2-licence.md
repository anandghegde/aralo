# ADR-0009: Apache-2.0 licence

- Status: Accepted (confirmed by the project owner, 2026-09-20)
- Date: 2026-09-20

## Context

Aralo is free and open source. The PRD left the licence open (PRD open
question 4) and recommended Apache-2.0. Section 14 of the plan lists this as
decision 2. It must be settled before M0 ends, because the project files, the
crate metadata and the dependency licence policy all depend on it.

The roadmap includes an iOS keyboard in v2, distributed through the App Store.

## Decision

Aralo is released under the **Apache-2.0** licence.

- `LICENSE` holds the verbatim licence text. `NOTICE` holds the copyright
  line.
- Contributions are accepted under the same licence, with a DCO sign-off on
  every commit. See `CONTRIBUTING.md`.
- `cargo-deny` checks dependency licences in CI.

## Consequences

- The v2 App Store keyboard is not complicated by the licence.
- Anyone may use, modify and redistribute Aralo, including in closed-source
  products, under the terms of the licence.
- If the owner chooses GPL-3.0 instead, a dependency licence review is needed
  in M0, the `cargo-deny` policy changes, and `LICENSE`, `NOTICE`, the crate
  metadata and this ADR are replaced.

## Alternatives rejected

- **GPL-3.0.** It would complicate the v2 App Store keyboard. It also requires
  a dependency licence review in M0.

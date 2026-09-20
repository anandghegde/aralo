## What and why

<!-- What does this change do, and why is it needed? Keep it short. -->

## PRD requirement IDs

<!-- For example E1, D4, P10. Write "none" if this is tooling, docs or a refactor. -->

## Checklist

- [ ] Tests added or updated. A template bug fix includes a new golden fixture.
- [ ] `cargo fmt` and `cargo clippy` are clean.
- [ ] No new dependency in `aralo-engine` (no file, network, clock or logging crate, direct or transitive).
- [ ] No HTTP client constructed outside the network guard in `aralo-ai`.
- [ ] No snippet bodies, context, model output or keys in log fields, fixtures or this description.
- [ ] Every commit is signed off (`git commit -s`, DCO).

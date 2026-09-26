# Progress

Where each task in the
[implementation plan](https://claude.ai/code/artifact/46c54272-8066-4ba6-8fbe-fe384a551789)
stands. The plan says what each task is and when it is done; this page says
whether it is. Update the row in the same commit that lands the work.

- **Landed**: the work is committed. The commit is the evidence.
- **Partial**: some of the work is committed; the notes say what is missing.
- **In progress**: the work is under way and not committed yet.
- **Not started**

Rows marked Landed before 2026-09-26 were filled in from the Git history
afterwards. Their "Done when" was not re-checked when the row was written.

## M0. Foundations and spikes

| Task | What | Status | Commit | Notes |
| --- | --- | --- | --- | --- |
| 0.1 | Repository, licence, CONTRIBUTING, code of conduct, SECURITY.md | Landed | 42156dd | |
| 0.2 | Cargo workspace, dependency-graph check, check workflow | Landed | 42156dd | |
| 0.3 | Bridge pipeline: aralo-ffi, XCFramework, Swift package | Landed | 42156dd | |
| 0.4 | Xcode project, menu bar app, AraloKit, SwiftLint | Landed | 42156dd | |
| 0.5 | Spikes S1 to S7 | Partial | 4d7ecbc | S5 is answered by ADR-0016. S1, S2, S3, S4, S6 and S7 need measurement on real apps and hardware; see [adr/README.md](adr/README.md) |
| 0.6 | Format spec v0 and schemas | Landed | 42156dd | |

## M1. Engine and injection

| Task | What | Status | Commit | Notes |
| --- | --- | --- | --- | --- |
| 1.1 | Ring buffer, trie, snapshot swap, triggers | Landed | 42156dd | |
| 1.2 | Case modes | Landed | 42156dd | |
| 1.3 | Scope filter, pause, exclusions | Landed | 2f1d044 | |
| 1.4 | Expansion record and undo | Landed | 42156dd | |
| 1.5 | Event tap | Landed | 2f1d044 | |
| 1.6 | Injector with pasteboard restore | Landed | 2f1d044 | |
| 1.7 | Secure input and menu bar states | Landed | 2f1d044 | |
| 1.8 | Minimal snippet loader | Landed | 42156dd | |
| 1.9 | Permission onboarding, first version | Landed | 2f1d044 | |
| 1.10 | Matrix and latency harnesses, benchmark, apps.toml | Landed | 2f1d044 | |

## M2. Library, editor, search and import

| Task | What | Status | Commit | Notes |
| --- | --- | --- | --- | --- |
| 2.1 | Folder store, watcher, SQLite index | Landed | e04dc03 | |
| 2.2 | Group model | Landed | e3ffb1f | |
| 2.3 | Main window | Landed | e3ffb1f | |
| 2.4 | Editor | Landed | e3ffb1f | |
| 2.5 | Search and the inline palette | Landed | e04dc03, 4dee08e | |
| 2.6 | Quick create | Landed | 4dee08e | |
| 2.7 | Menu bar recents | Landed | 4dee08e | |
| 2.8 | Importers and the import report | Landed | e04dc03 | |
| 2.9 | Export | Landed | e04dc03 | |
| 2.10 | CLI | Landed | e04dc03 | |

## M3. Dynamic content and forms

| Task | What | Status | Commit | Notes |
| --- | --- | --- | --- | --- |
| 3.1 | Placeholder parser and fuzz target | Landed | 4dee08e | |
| 3.2 | Evaluator: dates, clipboard, nested snippets | Landed | 4dee08e | |
| 3.3 | Session state machine | Landed | 4dee08e | |
| 3.4 | Form panel | Landed | 4dee08e | |
| 3.5 | Cancel and multi-step undo | Landed | 4dee08e | |
| 3.6 | Live preview in the editor | Landed | 4dee08e | |
| 3.7 | Importer macro tables | Landed | 4dee08e | |

## M4. AI

| Task | What | Status | Commit | Notes |
| --- | --- | --- | --- | --- |
| 4.1 | Gateway skeleton and network guard | Landed | 126bb6c | |
| 4.2 | openai_compat adapter | Landed | 126bb6c | |
| 4.3 | anthropic adapter | Landed | 126bb6c | |
| 4.4 | Profiles, secrets, settings pane | Landed | 126bb6c | |
| 4.5 | Commands on a selection | Landed | 126bb6c | |
| 4.6 | AI blocks in sessions | Landed | c5f9510 | |
| 4.7 | Editor AI actions | Landed | 01ec9a4 | |
| 4.8 | Search by meaning | Landed | 98c623a | |
| 4.9 | Conformance suite | Landed | 6bce4f4, b73b1fb | |
| 4.10 | AI master switch and local-only mode | Landed | 5e1613a | |

## M5. Sync, hardening and beta

| Task | What | Status | Commit | Notes |
| --- | --- | --- | --- | --- |
| 5.1 | Merge bases, conflict copies, three-way merge, resolver | Landed | 4dee08e | Conflict copies and the resolver landed in M2. Saving over a file that changed while it was open, and merge bases that a machine's own saves no longer move, landed in the commit that marks this row Landed. Done when checked by `crates/aralo-core/tests/two_macs.rs`: two runtimes on two folders, a copy named by each of seven providers, a clean merge on both and a true conflict showing both versions |
| 5.2 | Library location picker and move; iCloud and Dropbox smoke tests | Not started | | |
| 5.3 | Section 9 tests, data-flow.md, threat-model.md | Partial | bc08d63 | Every section 9 row is mapped to a test in [threat-model.md](threat-model.md). Not coverable yet: P9 script approvals and P3 encrypted library (v1), signed data tables, signing, notarisation and the SBOM (5.5) |
| 5.4 | Full onboarding | Partial | a916a93 | Six screens: privacy, Accessibility, Input Monitoring, the library (whose snippets, Show in Finder, Import Snippets…), an AI offer that leaves AI off, and try-it. A returning user sees only the permission screens. Checked by `OnboardingTests` and the bridge test `the_first_run_can_tell_the_starter_snippets_from_the_users_own`. Not done: "a new user expands a first snippet in under 3 minutes" needs a timed run with new users; choosing a library location waits on 5.2 and only points at Settings |
| 5.5 | Release workflow: signing, notarisation, DMG, Sparkle | Not started | | Needs Apple Developer credentials |
| 5.6 | Homebrew cask and documentation site | Not started | | Depends on 5.5 |
| 5.7 | Local counters and Copy diagnostics | Landed | 4982ace | Done when checked by `crates/aralo-ffi/tests/diagnostics.rs`. The report's excluded-apps line stays empty until the app has a user exclusion list |
| 5.8 | Performance pass | Not started | | Needs three Macs |
| 5.9 | Accessibility pass | Partial | a916a93 | The unlabelled-element test is clean: `AccessibilityAudit` (AraloKit, `AccessibilityAuditTests`) run over every panel by `AraloTests` (`make test-app`). Labels, row traits and actions, switched-off state, Reduce Motion and Increase Contrast are fixed. Owed by hand on hardware: a VoiceOver run through every panel, keyboard-only use of the library window and settings, and large text sizes (macOS has no Dynamic Type; several panels have fixed frames). The form panel and the import sheet are labelled but not yet in `AraloTests` |
| 5.10 | Beta operations | Not started | | Not code |

# Threat model

What Aralo protects, from whom, where the trust boundaries are, and the test
that proves each defence holds. Where data goes is in
[data-flow.md](data-flow.md). The design behind each defence is in
[architecture.md](architecture.md#privacy-invariants). How to report a problem
is in [SECURITY.md](../SECURITY.md).

A defence without a test here is a claim, not a property. When one is listed
as "not built", the code it would check does not exist yet.

## What is worth protecting

| Asset | Why it matters |
| --- | --- |
| Keystrokes | Aralo sees every key typed while it runs. That includes passwords, messages and anything else. A text expander that kept them would be a keylogger |
| The library | The user's snippets. They can hold addresses, account numbers and private replies. The files are the user's, and may be synced |
| The clipboard and the selection | Whatever the user copied or selected last. Often private, sometimes a password |
| API keys | Money and access at a model provider. A leaked key is spent by someone else |
| AI context and answers | What a snippet or command sends to a model, and what comes back |
| The Mac itself | Aralo types into other apps. Anything that makes it type or run something the user did not ask for acts with the user's privileges |
| Network silence | The promise that Aralo contacts nobody the user did not choose, and nobody at all in local-only mode |

## Who might attack

| Adversary | Can | Cannot, by design |
| --- | --- | --- |
| The model endpoint's operator | See every request sent to it: the declared context and the key for that endpoint | See anything a snippet did not declare, or any other endpoint's key |
| A network observer | See which hosts Aralo contacts, and when | Read requests to a remote endpoint, which go over TLS. A profile over plain `http://` to a remote host is the user's choice and is visible to them in the address |
| Hostile content in context | Put text in a selection, a clipboard or a web page the user runs a command on, written to steer the model (prompt injection) | Make Aralo run, read or send anything. The answer is only ever text for the user to see and insert |
| A malicious or broken model | Return anything, including Aralo's own placeholder syntax | Have its answer evaluated. It goes in as characters |
| Someone who writes to a shared or synced library | Add or change snippets the user will run | Choose where a request goes. A snippet may name a profile, never an address, and a profile lives in the state folder, outside the library |
| Aralo's own regressions | Add a log line, a network call or a new dependency by accident | Get it past CI. Each promise has a check that fails the build |
| The supply chain | Ship a malicious or vulnerable crate | Get one in unnoticed: `cargo-deny` checks licences, advisories and sources, and a new crate must be added to the allow-list in `scripts/check-deps.sh` |

Out of scope: malware already running as the user, or with more privilege.
It can read the keychain the user unlocks, the library folder and the screen
without Aralo's help. Physical access to an unlocked Mac, likewise.

## Trust boundaries

1. **macOS to the event tap to the engine.** Keys cross from the system into
   `AraloKit`'s tap, and straight on into `aralo-engine` across the bridge.
   Only the engine holds typed text, 64 characters at most, in memory.
2. **The library folder to the loader.** Files may come from anywhere a sync
   client reaches. The loader parses them as data. Nothing in a snippet file
   can name a host, a key or a script runtime.
3. **Aralo to the network.** Every request goes through the network guard in
   `aralo-ai`, the only HTTP client in the build. It checks the address before
   sending and again at connect, and follows no redirect.
4. **Model output back to the evaluator.** An answer is text. There is no path
   from it into the template evaluator, a script runtime or the network.
5. **The keychain to a request.** A key is read from the keychain into memory
   for one request, and is written only into that request's authentication
   header.
6. **The Swift shell to the Rust core.** The shell passes events and context in
   and gets plans out. What the shell may read (clipboard, selection, window
   title) it reads only when the core asks for a declared kind.

## Threats and what stops them

| Threat | Mitigation | Proved by |
| --- | --- | --- |
| Typed text outlives the moment it is needed | Fixed ring in `aralo-engine`, zeroed with `zeroize` on every reset, after a match and on drop | `crates/aralo-engine/tests/privacy.rs`: `the_buffer_is_zero_after_every_reset_reason`, `every_state_change_that_invalidates_the_buffer_zeroes_it`, `a_match_empties_the_buffer`, `the_undo_record_is_dropped_by_a_reset_too` |
| Typed text reaches a file, a log or the network from the engine | No I/O, clock, network or logging dependency; `#![no_std]` | `scripts/check-deps.sh`, checks a and d. `crates/aralo-engine/clippy.toml`. `privacy.rs`: `debug_output_carries_no_typed_text`. Code owners on the engine and the tap files |
| Typed text reaches a log from elsewhere | Aralo writes no logs. No Swift file logs, and no crate depends on a logging library | `scripts/check-deps.sh`, check h |
| A password is typed while Aralo watches | Secure input resets the engine at once. Password managers are excluded by default, and nothing is recorded in an excluded app | `BridgeTests.testSecureInputGoingOnForgetsWhatWasTyped` turns real secure input on. `privacy.rs`: `nothing_is_recorded_while_paused_or_in_an_excluded_app`. `crates/aralo-ffi/tests/bridge.rs`: `excluded_apps_never_expand`. `presets.rs`: `presets_are_well_formed_bundle_ids`. `matching.rs`: `preset_password_managers_are_excluded_until_the_list_is_replaced` |
| A password is read as a selection | Capture refuses a secure text field, while secure input is on, when paused and in an excluded app | `SelectionCaptureTests.testPasswordsPausedAndExcludedAppsAreNeverRead` |
| A model is sent more than the snippet asked for | The session asks for declared kinds only; the manifest is built from what was sent | `crates/aralo-core/tests/security.rs`: `a_snippet_declaring_fillins_makes_no_call_for_the_selection_or_the_clipboard`. `crates/aralo-ai/tests/gateway.rs`: `only_declared_context_is_asked_for_or_sent`. `crates/aralo-core/tests/ai_blocks.rs`, `ai_command.rs`, `ai_authoring.rs`. `FormSessionAITests` |
| Prompt injection makes Aralo act | Context is framed as data. The answer is literal text | `security.rs`: `model_output_holding_script_and_ai_placeholders_goes_in_verbatim`. `gateway.rs`: `model_output_is_passed_on_as_literal_text`. `crates/aralo-template/tests/ai_blocks.rs`, with a property test that the text around a block is byte-identical whatever the answer. `fixtures/golden/ai.toml` |
| A request leaves the Mac in local-only mode | The guard refuses by URL, and by resolved address at connect, and follows no redirect | `crates/aralo-ai/tests/local_only.rs`, all six tests. `deny.toml` confines `reqwest` to `aralo-ai`; `clippy.toml` bans building a client elsewhere; `scripts/check-deps.sh`, check f |
| The Mac app opens a connection of its own | The Swift shell uses no networking API. The one connection outside the core is Sparkle's update check, in Sparkle's framework, and it asks `UpdatePolicy` first | `scripts/check-deps.sh`, check g. `UpdatePolicyTests` |
| A model call happens while AI is off | Every call refuses before a key or context is read; the embedding model is not loaded | `crates/aralo-core/tests/ai_switch.rs`. `MeaningSearchTests` |
| Aralo contacts a host nobody listed | There are no Aralo servers. Every host in the binary, the Swift sources and the shipped data is on the allow-list in `data-flow.md` | `crates/aralo-cli/tests/hosts.rs` |
| An API key ends up on disk | Keys live in the keychain. `profiles.toml` holds a reference, and refuses a key-shaped header or address | `security.rs`: `after_a_full_run_the_key_is_in_no_file_of_the_library_the_state_or_an_export`. `crates/aralo-core/tests/ai_settings.rs`: `after_every_operation_the_key_is_only_in_the_store_and_the_auth_header`. `scripts/key-leak-scan.sh`, its own CI job |
| An API key ends up in a shared report | A conformance report says only whether a key was sent, and redacts anything key-shaped | `crates/aralo-core/tests/conformance.rs` |
| A key is sent to the wrong host | A key goes only to its own profile's address; the client follows no redirect | `security.rs` (the key is only in the `authorization` header). `local_only.rs`: `the_guard_follows_no_redirect` |
| The app is tampered with or loads foreign code | Hardened runtime and no entitlements on every piece of code, so library validation refuses a dylib not signed by Aralo's team. The one third-party framework, Sparkle, is pinned to one version and re-signed with Aralo's identity. Developer ID signature, notarised and stapled | `scripts/check-deps.sh`, check i. `scripts/verify-release.sh` checks b, c and i on every build of `.github/workflows/release.yml`: a strict, deep `codesign` check, the runtime flag and no entitlement on all six signed pieces, and for a release the Developer ID, the stapled tickets and `spctl` |
| A forged update is installed | Sparkle installs only an update signed with the EdDSA key whose public half is in the app; the feed is https on github.com | `scripts/verify-release.sh`, check e, checks the appcast's signature with OpenSSL against the key read out of the built app. `UpdatePolicyTests`: `testOnlyAnHttpsFeedOnGitHubCounts`, `testTheKeyMustBeThirtyTwoBytes` |
| A forged compatibility table is loaded | A downloaded table replaces the bundled one only if its Ed25519 signature checks against the key built into the core | `crates/aralo-core/tests/signed_data.rs` (eight tests). `scripts/verify-release.sh`, check f |
| An update check leaves the Mac in local-only mode | Sparkle asks before every check; the answer is no in local-only mode, or when the mode cannot be read. A build without a real key never starts Sparkle | `UpdatePolicyTests`: `testLocalOnlyModeTurnsTheUpdateCheckOff`, `testTheProjectPlaceholdersAreNotAFeed` |
| A dependency brings a vulnerability or a bad licence | `cargo-deny`; the crate allow-list | `cargo deny check` in CI. `scripts/check-deps.sh` |

## The security and privacy table, row by row

The rows of section 9 of the implementation plan, and the tests that hold
them.

| Row | Test |
| --- | --- |
| Keystrokes stay in memory, 64 characters at most (P1) | `crates/aralo-engine/tests/privacy.rs` (six tests). `scripts/check-deps.sh` checks a and d on the engine's dependency tree and sources. Code owners on `crates/aralo-engine/`, `EventTap.swift`, `KeyClassifier.swift` and `ExpansionController.swift` in `.github/CODEOWNERS`. `BridgeTests`: `testWhatIsTypedIntoTheSearchPaletteIsNotWatchedAndNotRemembered`, `testAMouseClickOrAnArrowKeyForgetsWhatWasTyped` |
| No expansion in password fields (P2) | Unit test on the presets: `presets_are_well_formed_bundle_ids`, and `matching.rs`: `preset_password_managers_are_excluded_until_the_list_is_replaced`. Secure text field: `BridgeTests.testSecureInputGoingOnForgetsWhatWasTyped`, which calls `EnableSecureEventInput`, what `NSSecureTextField` does, and runs the monitor the app runs. Also `excluded_apps_never_expand`, `SelectionCaptureTests` and `SecureInputTests` |
| AI sends only declared context (P4) | `crates/aralo-core/tests/security.rs`: `a_snippet_declaring_fillins_makes_no_call_for_the_selection_or_the_clipboard`. Also `gateway.rs`: `only_declared_context_is_asked_for_or_sent` |
| Local-only mode means no network (P6) | `crates/aralo-ai/tests/local_only.rs`: `local_only_refuses_a_remote_address_before_connecting` and five more. The lint: `clippy.toml` and `deny.toml`, and `scripts/check-deps.sh` checks f and g |
| No Aralo servers (P7) | [data-flow.md](data-flow.md#every-host-aralo-can-contact) lists the hosts. `crates/aralo-cli/tests/hosts.rs`: `the_binary_names_no_host_the_data_flow_page_does_not_list`, and the same for the Swift sources and `data/` |
| Keys only in the keychain (P13) | `crates/aralo-core/tests/security.rs`: `after_a_full_run_the_key_is_in_no_file_of_the_library_the_state_or_an_export`. `ai_settings.rs`: `after_every_operation_the_key_is_only_in_the_store_and_the_auth_header`. `scripts/key-leak-scan.sh`. Aralo writes no logs to scan: check h |
| Prompt injection cannot act (P10) | `crates/aralo-core/tests/security.rs`: `model_output_holding_script_and_ai_placeholders_goes_in_verbatim`. Also `gateway.rs`: `model_output_is_passed_on_as_literal_text`, and `crates/aralo-template/tests/ai_blocks.rs` |
| Shared scripts need approval (P9, v1) | Not coverable yet. There is no script runtime; `aralo-script` is empty until v1 |
| Telemetry is opt-in and content-free (P8) | Nothing to enforce: there is no upload code. `hosts.rs` would catch a telemetry host, and check h a logging crate |
| Code signing and runtime | `scripts/check-deps.sh`, check i: hardened runtime on, no entitlements in the project. `scripts/verify-release.sh`, run by `.github/workflows/release.yml` on every build: checks b and c on the built app (a strict, deep signature check; the runtime and no entitlement on each of its six signed pieces), and check i on a signed release (Developer ID, notarised, stapled, accepted by `spctl`). Until the Developer ID secrets are set, the workflow runs the dry run, signed ad hoc, and check i is skipped |
| Signed data updates | `crates/aralo-core/tests/signed_data.rs`: a table signed by `scripts/sign-data.sh` is accepted; one changed byte, another key's signature, a malformed signature or no key at all is refused, and the bundled table stays in use. `scripts/verify-release.sh`, check f, on the release's `apps.toml.sig`. The app does not download tables yet, and the key in `data/keys/data-tables.pub` is a placeholder until the first release |
| Update checks (Sparkle) | `UpdatePolicyTests`: no check in local-only mode, no updater at all without a real key, only an https feed on github.com. `scripts/verify-release.sh`, check e: the appcast's EdDSA signature checks against the key in the built app |
| Logging | Aralo logs nothing, so there is no field allow-list to test yet. Check h fails on the first log call or logging crate. Crash reports are macOS's own |
| Supply chain | `cargo deny check` in the check workflow; the crate allow-list in `scripts/check-deps.sh`. The SBOM: `scripts/release.sh` writes a CycloneDX file for every release, and `scripts/verify-release.sh`, check g, fails if it misses any crate the core links on either Apple target, or Sparkle. The release and docs workflows pin actions to commits; the check workflows still pin major versions. Checksums: check a |
| Documents | [SECURITY.md](../SECURITY.md), this page and [data-flow.md](data-flow.md) |
| Encrypted library (P3, v1) | Not built |

## Not covered yet

- **Script approvals (P9).** Waits for the script runtime in v1.
- **A real signed release.** The workflow, the scripts and their checks
  exist and run as a dry run. The Developer ID signature, notarisation and
  Gatekeeper's verdict (check i) are proved only once the signing secrets are
  set and a tag is pushed; see [releasing.md](releasing.md).
- **Downloading signed data tables.** The signature check is built and
  tested in the core; the app does not fetch tables yet, and the public key
  in `data/keys/data-tables.pub` is a placeholder until the first release.
- **Sparkle in the running app.** The local-only refusal is tested in
  `UpdatePolicy`; that the running app sends nothing to github.com in
  local-only mode is not yet watched from outside, as the network tests do
  for the core.
- **The secure text field in the injection matrix.** The nightly matrix runs
  on a self-hosted Mac. `testSecureInputGoingOnForgetsWhatWasTyped` uses the
  same system call a secure text field makes, so it runs on every pull request
  instead.
- **Pinned actions.** The release and docs workflows pin actions to commit
  hashes. The check workflows still pin major tags.
- **Encrypted library (P3).** v1.

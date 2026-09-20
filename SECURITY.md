# Security policy

Aralo reads keystrokes to expand abbreviations and can hold API keys for
model providers. Reports about either are taken seriously.

## Reporting a vulnerability

Report privately through GitHub private vulnerability reporting:

1. Open the repository's **Security** tab.
2. Choose **Report a vulnerability**.
3. Describe the problem, the affected version, and steps to reproduce.

**Do not open a public issue, discussion or pull request for a
vulnerability.** Do not include real API keys or private snippet content in a
report. Use a throwaway key if a key is needed to reproduce.

## What to expect

- Acknowledgement within 3 working days. This is a target, not a guarantee.
- Follow-up happens with you inside the private advisory.

## Supported versions

Aralo is pre-1.0. Only the latest release receives security fixes. Check that
the problem reproduces on the latest release before reporting.

## In scope

- **Keystroke handling.** The buffer and matcher in `aralo-engine`, and the
  event tap in `AraloKit`. Anything that lets typed text outlive a reset,
  exceed the 64-character buffer, reach disk, a log or the network, or be
  matched while secure input or a password field is active.
- **API-key handling.** The `SecretStore` and keychain path. Any way for a key
  to appear in a profile file, an export, a log, a crash report, the library
  folder or a diagnostics report.
- **The network guard and local-only mode.** Any HTTP request that does not
  pass through the guard in `aralo-ai`, or any request to a non-loopback
  address while local-only mode is on. Any contact with a host outside the
  documented list.
- **Signed data-table updates.** The compatibility table and the provider
  quirks table update as signed files. Anything that gets an unsigned or
  wrongly signed table accepted.
- **Prompt-injection paths.** Any route by which model output, or context
  placed in a prompt, gets back into the template evaluator, the script
  runtime or the network instead of being inserted as literal text.

If you are unsure whether something is in scope, report it privately anyway.

## Security design in brief

These are the promises the design makes. Each is tied to a mechanism and to a
test that fails the build when it breaks.

- **Keystrokes stay in memory, 64 characters at most.** `aralo-engine` holds a
  fixed ring buffer, zeroed on every reset with `zeroize`. The crate has no
  I/O, clock or logging dependency, and CI checks its dependency tree. The
  Swift tap passes events straight to the engine and keeps no copy.
- **No expansion in password fields.** Aralo polls for secure input and ships
  an exclusion list preset with password managers and banking apps. Both are
  checked before any matching.
- **Keys only in the keychain.** Provider profiles hold a reference to a
  keychain item, never the key itself. The policy file schema has no key
  field.
- **No Aralo servers.** There are none to call. The app can contact the
  endpoints you configure, and GitHub for updates and signed data tables.
- **Local-only mode means no network.** The network guard is the only HTTP
  client constructor and checks the resolved address. The Sparkle update
  check is the one documented exception.
- **AI sends only declared context.** A snippet or command declares the
  context kinds it uses. The manifest shown before sending is built from the
  bytes actually sent.
- **AI output is literal text.** Context is framed as data in the prompt.
  Output is inserted verbatim. It cannot start an expansion, run a script or
  make a request.
- **Telemetry.** The MVP ships local counters only. There is no upload code.
- **Logs carry no content.** Log fields are limited to IDs, counts, durations
  and error codes. Crash reports stay on the device unless you attach one to
  an issue.
- **Release builds** are signed with a Developer ID, use the hardened runtime
  and are notarised, with no JIT, unsigned-memory or library-validation
  entitlements.

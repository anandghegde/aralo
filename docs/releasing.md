# Releasing

A release is built by one script, `scripts/release.sh`, checked by another,
`scripts/verify-release.sh`, and published by `.github/workflows/release.yml`
when a tag `v<version>` is pushed. Anyone can run the first two: without the
signing secrets they make a **dry run**, the same release signed ad hoc and not
notarised, with keys made for the run.

```sh
brew install xcodegen openssl@3
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo install cargo-cyclonedx --version 0.5.9
scripts/release.sh --dry-run
scripts/verify-release.sh --dry-run dist
```

## What a release is

Everything goes into `dist/`:

| File | What it is |
| --- | --- |
| `Aralo-<version>.dmg` | The universal app, signed with the Developer ID and the hardened runtime, notarised, the ticket stapled to both the app and the DMG |
| `appcast.xml` | Sparkle's feed: this version, the DMG's URL and length, and its EdDSA signature |
| `data/apps.toml`, `data/apps.toml.sig` | The compatibility table and its Ed25519 signature, hex. `data-tables.pub` is the public key it checks against |
| `Aralo-<version>.cdx.json` | The SBOM, CycloneDX 1.5: every crate the core links on both Apple targets, and the Swift packages the app embeds |
| `aralo.rb` | The Homebrew cask, filled in from `packaging/homebrew/aralo.rb.in` |
| `unsigned-bundle.sha256`, `unsigned-bundle.files.txt` | The hash of the app before signing (below) |
| `release.env` | The version, build number and update key, for the checks |
| `SHA256SUMS` | The SHA-256 of every file above |

`scripts/verify-release.sh` checks all of it: the checksums, the signature
and hardened runtime on every piece of code in the app, both architectures,
the appcast's signature against the key inside the app, the data table's
signature, the SBOM against `cargo tree`, the cask, and for a real release
the Developer ID, the stapled tickets and Gatekeeper. The comment at the top
of the script lists each check.

## The unsigned bundle hash

A signed app is never byte-identical twice: signatures carry timestamps. The
app as it leaves `xcodebuild`, before signing, is what can be compared. The
hash is the SHA-256 of a sorted list of every file's SHA-256, written to
`unsigned-bundle.files.txt`. The release notes carry it. To compare a build
of the same tag, run `scripts/release.sh --dry-run` on it and diff the two
`unsigned-bundle.files.txt`. Where they differ (the toolchain version, the
path of the checkout) is where the build is not yet reproducible.

## Secrets

The workflow runs a signed build only when all of these repository secrets
are set, and never for a pull request. A tag pushed without them fails
rather than publishing a dry run.

| Secret | What it is | How to make it |
| --- | --- | --- |
| `DEVELOPER_ID_CERT_P12` | The Developer ID Application certificate and its private key, as a .p12, base64 | Xcode, Settings, Accounts, Manage Certificates, then export it from Keychain Access. `base64 -i cert.p12 \| pbcopy` |
| `DEVELOPER_ID_CERT_PASSWORD` | The .p12's password | Chosen at export |
| `DEVELOPER_ID_IDENTITY` | The certificate's name | `Developer ID Application: <Name> (<TEAMID>)`, as `security find-identity -v -p codesigning` prints it |
| `APPLE_API_KEY_P8` | An App Store Connect API key for notarytool, the .p8 file's text | App Store Connect, Users and Access, Integrations, Team Keys, with the Developer role |
| `APPLE_API_KEY_ID` | That key's ID | Shown beside it |
| `APPLE_API_ISSUER_ID` | The team's issuer ID | Shown above the keys |
| `SPARKLE_ED_PRIVATE_KEY` | Sparkle's EdDSA private key: the 32-byte seed, base64 | `scripts/sign-data.sh new-key sparkle.pem`, then `scripts/sign-data.sh seed-base64 sparkle.pem`. Or Sparkle's `generate_keys`, then `generate_keys -x key.txt` |
| `DATA_TABLE_SIGNING_KEY` | The data tables' Ed25519 private key, the same format | `scripts/sign-data.sh new-key data.pem`, then `scripts/sign-data.sh seed-base64 data.pem` |
| `HOMEBREW_TAP_TOKEN` | Optional. A token that can push to the tap and open pull requests there | A fine-grained token on the tap repository, contents and pull requests: write |

The tap repository is `anandghegde/homebrew-aralo`, or the repository
variable `HOMEBREW_TAP` if set. Without the token, the cask is on the
release as `aralo.rb`, to be copied by hand.

Keep an offline copy of both Ed25519 keys. A lost Sparkle key means no
installed copy can be updated again: every user must download the next
version by hand.

## Placeholders to replace before the first release

- **The data-table key.** `data/keys/data-tables.pub` says `unset`. Replace
  it with the public half of `DATA_TABLE_SIGNING_KEY`, 64 hex digits
  (`scripts/sign-data.sh public-hex data.pem`), and commit it. The core
  builds the key in, and a release refuses to build while it is `unset` or
  does not match the secret.
- **The update key.** `ARALO_SPARKLE_PUBLIC_KEY` in `apps/macos/project.yml`
  is a placeholder that is not a key, so a local build never starts Sparkle.
  It stays that way: `scripts/release.sh` sets the real key from
  `SPARKLE_ED_PRIVATE_KEY` for the release build only.
- **The feed.** `ARALO_FEED_URL` in `apps/macos/project.yml` and the cask's
  URLs point at `github.com/anandghegde/aralo`. A release built in another
  repository points at that one (`ARALO_REPO`, which the workflow sets).
- **GitHub Pages.** The docs workflow deploys to Pages. In the repository's
  settings, set Pages' source to GitHub Actions once.

## Making a release

1. Set `MARKETING_VERSION` in `apps/macos/project.yml`, and merge it.
2. Push the tag: `git tag -s v1.2.3 && git push origin v1.2.3`.
3. The workflow builds, notarises, checks, installs the cask on the runner
   and uninstalls it, then creates the GitHub release with every file and
   opens the cask pull request on the tap.
4. Merge the tap's pull request once `brew install --cask` works from it.

Sparkle's version is the build number, the number of commits on the tagged
commit, so it grows with every release. The appcast is at
`releases/latest/download/appcast.xml`: a new release replaces the feed for
every installed copy at once.

## Checking a release by hand, on a clean Mac

```sh
shasum -a 256 -c SHA256SUMS --ignore-missing
spctl --assess --type open --context context:primary-signature -v Aralo-1.2.3.dmg
xcrun stapler validate Aralo-1.2.3.dmg
```

Then open the DMG, drag Aralo to Applications and open it: there must be no
Gatekeeper warning. An older version, installed and opened with local-only
mode off, must offer the new one within a day, or at once from
*Check for Updates…*. With local-only mode on, it must not ask.

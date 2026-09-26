#!/usr/bin/env bash
# Builds a release of the Mac app (plan section 10, task 5.5): a universal
# build, signed with the Developer ID and the hardened runtime, notarised and
# stapled, in a DMG, with the Sparkle appcast, the signed data tables, the
# SBOM, the Homebrew cask and the checksums of all of it.
#
# Usage: scripts/release.sh [--dry-run] [--version X.Y.Z] [--out DIR] [--reuse-bridge]
#
#   --dry-run       No secret is needed. The app is signed ad hoc, nothing is
#                   notarised, and the appcast and data tables are signed with
#                   keys made for this run. Everything else is the real thing,
#                   and scripts/verify-release.sh checks it the same way.
#   --version       The version to release. Default: the tag in GITHUB_REF_NAME
#                   (v1.2.3), or MARKETING_VERSION from apps/macos/project.yml.
#   --out           Where the release goes. Default: dist.
#   --reuse-bridge  Use the XCFramework already in apps/macos/Generated rather
#                   than building it. For a quick local dry run only: a release
#                   builds the core from this commit.
#
# A real release reads these from the environment (the release workflow sets
# them from its secrets; docs/releasing.md says how to make each one):
#
#   ARALO_SIGN_IDENTITY     "Developer ID Application: <Name> (<TEAMID>)", a
#                           certificate in an unlocked keychain
#   ARALO_NOTARY_KEY_PATH   An App Store Connect API key (.p8) for notarytool,
#   ARALO_NOTARY_KEY_ID     its key ID,
#   ARALO_NOTARY_ISSUER     and its issuer ID
#   SPARKLE_ED_PRIVATE_KEY  Sparkle's EdDSA private key, base64, as
#                           `generate_keys -x` exports it
#   DATA_TABLE_SIGNING_KEY  The data tables' Ed25519 private key, the same
#                           format; its public half is data/keys/data-tables.pub
#
# Optional: ARALO_REPO (default anandghegde/aralo), the GitHub repository the
# DMG, the appcast and the cask point at; ARALO_BUILD_NUMBER (default: the
# number of commits), Sparkle's version, which must grow with every release.
#
# Needs: macOS, Xcode, xcodegen, cargo with both Apple targets, cargo-cyclonedx,
# python3, OpenSSL 3 (see scripts/sign-data.sh).
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

dry_run=0
version=""
out="dist"
reuse_bridge=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --dry-run) dry_run=1 ;;
        --version) version="${2:?--version needs a value}"; shift ;;
        --out) out="${2:?--out needs a value}"; shift ;;
        --reuse-bridge) reuse_bridge=1 ;;
        -h | --help) sed -n '2,/^set -euo/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "release: unknown option $1" >&2; exit 2 ;;
    esac
    shift
done

step() { echo; echo "==> $*"; }
die() { echo "release: $*" >&2; exit 1; }

# --- Preflight ---------------------------------------------------------------

[[ "$(uname -s)" == Darwin ]] || die "a release is built on macOS"
for tool in xcodebuild xcodegen cargo python3 hdiutil codesign ditto shasum; do
    command -v "$tool" >/dev/null 2>&1 || die "\`$tool\` is required but was not found on PATH"
done
cargo cyclonedx --version >/dev/null 2>&1 || die "cargo-cyclonedx is required: cargo install cargo-cyclonedx"

if [[ -z "$version" ]]; then
    if [[ "${GITHUB_REF_NAME:-}" =~ ^v[0-9] ]]; then
        version="${GITHUB_REF_NAME#v}"
    else
        version="$(sed -n 's/^ *MARKETING_VERSION: *"\{0,1\}\([^"]*\)"\{0,1\} *$/\1/p' apps/macos/project.yml | head -n 1)"
    fi
fi
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]] || die "not a version: '$version'"
build_number="${ARALO_BUILD_NUMBER:-$(git rev-list --count HEAD)}"
repo="${ARALO_REPO:-anandghegde/aralo}"
feed_url="https://github.com/$repo/releases/latest/download/appcast.xml"
dmg_name="Aralo-$version.dmg"
dmg_url="https://github.com/$repo/releases/download/v$version/$dmg_name"

mkdir -p "$out"
out="$(cd "$out" && pwd)"
work="$root/target/release-work"
rm -rf "$work" "${out:?}"/*
mkdir -p "$work"
umask 077
keys="$work/keys"
mkdir -p "$keys"
trap 'rm -rf "$keys"' EXIT
umask 022

if [[ "$dry_run" -eq 1 ]]; then
    echo "release: DRY RUN of $version ($build_number). Signed ad hoc, not notarised, with keys made for this run."
    identity="-"
    scripts/sign-data.sh from-seed "$(openssl rand -base64 32)" "$keys/sparkle.pem"
    scripts/sign-data.sh from-seed "$(openssl rand -base64 32)" "$keys/data.pem"
else
    missing=""
    for name in ARALO_SIGN_IDENTITY ARALO_NOTARY_KEY_PATH ARALO_NOTARY_KEY_ID ARALO_NOTARY_ISSUER \
        SPARKLE_ED_PRIVATE_KEY DATA_TABLE_SIGNING_KEY; do
        [[ -n "${!name:-}" ]] || missing="$missing $name"
    done
    [[ -z "$missing" ]] || die "missing for a signed release:$missing. Use --dry-run to build without them."
    grep -qE '^[[:space:]]*unset[[:space:]]*$' data/keys/data-tables.pub \
        && die "data/keys/data-tables.pub is still the placeholder; commit the data-table public key first"
    identity="$ARALO_SIGN_IDENTITY"
    scripts/sign-data.sh from-seed "$SPARKLE_ED_PRIVATE_KEY" "$keys/sparkle.pem"
    scripts/sign-data.sh from-seed "$DATA_TABLE_SIGNING_KEY" "$keys/data.pem"
    committed="$(sed -n '/^[[:space:]]*[0-9a-fA-F]\{64\}[[:space:]]*$/p' data/keys/data-tables.pub | tr -d '[:space:]' | tr 'A-F' 'a-f')"
    [[ "$(scripts/sign-data.sh public-hex "$keys/data.pem")" == "$committed" ]] \
        || die "DATA_TABLE_SIGNING_KEY is not the key data/keys/data-tables.pub names"
fi
# The feed and the key the app will check updates with.
sparkle_public="$(scripts/sign-data.sh public-base64 "$keys/sparkle.pem")"
# Sparkle's own tool takes the key as the base64 seed, in a file.
python3 - "$keys/sparkle.pem" "$keys/sparkle.seed" <<'PYTHON'
import base64, re, sys
pem = open(sys.argv[1]).read()
der = base64.b64decode("".join(line for line in pem.splitlines() if not line.startswith("-----")))
# PKCS#8 for Ed25519: a 16-byte header, then the 32-byte seed.
open(sys.argv[2], "w").write(base64.b64encode(der[16:48]).decode() + "\n")
PYTHON

# --- 1. What goes inside: the model and the Rust core -------------------------

step "The embedding model"
scripts/fetch-model.sh

bridge="apps/macos/Generated/AraloBridge"
if [[ "$reuse_bridge" -eq 1 && -d "$bridge" ]]; then
    step "Reusing the XCFramework in $bridge (--reuse-bridge)"
else
    step "The Rust core, universal"
    ARALO_TARGETS="${ARALO_TARGETS:-aarch64-apple-darwin x86_64-apple-darwin}" scripts/build-xcframework.sh
fi
archs="$(lipo -archs "$(find "$bridge/AraloFFI.xcframework" -name 'libaralo_ffi.a' | head -n 1)")"
echo "core architectures: $archs"

# --- 2. The app, built unsigned ---------------------------------------------

step "Aralo.app $version ($build_number), Release, for $archs"
(cd apps/macos && xcodegen generate --quiet)
derived="$root/target/release-build"
xcodebuild -project apps/macos/Aralo.xcodeproj -scheme Aralo -configuration Release \
    -derivedDataPath "$derived" -quiet \
    ARCHS="$archs" ONLY_ACTIVE_ARCH=NO \
    CODE_SIGNING_ALLOWED=NO CODE_SIGN_IDENTITY="" \
    MARKETING_VERSION="$version" CURRENT_PROJECT_VERSION="$build_number" \
    ARALO_FEED_URL="$feed_url" ARALO_SPARKLE_PUBLIC_KEY="$sparkle_public" \
    build
app="$work/Aralo.app"
ditto "$derived/Build/Products/Release/Aralo.app" "$app"

# The signed bundle is not reproducible (signatures, timestamps); the unsigned
# one is compared instead (plan section 10). Every file's SHA-256, sorted, and
# the SHA-256 of that list.
step "The unsigned bundle's hash"
(cd "$work" && find Aralo.app -type f -print0 | LC_ALL=C sort -z | xargs -0 shasum -a 256) >"$out/unsigned-bundle.files.txt"
shasum -a 256 "$out/unsigned-bundle.files.txt" | cut -d ' ' -f 1 >"$out/unsigned-bundle.sha256"
echo "unsigned bundle: $(cat "$out/unsigned-bundle.sha256")"

# --- 3. Signing, inside out --------------------------------------------------

step "Signing with $([[ "$identity" == "-" ]] && echo "an ad hoc signature" || echo "$identity")"
sign() {
    # The hardened runtime on everything, and no entitlements anywhere
    # (scripts/check-deps.sh, check i). A secure timestamp is needed for
    # notarisation, and cannot be had with an ad hoc signature.
    local flags=(--force --options runtime --sign "$identity")
    [[ "$identity" == "-" ]] || flags+=(--timestamp)
    codesign "${flags[@]}" "$1"
}
sparkle="$app/Contents/Frameworks/Sparkle.framework"
if [[ -d "$sparkle" ]]; then
    # Sparkle's own order: its XPC services, its helpers, then the framework.
    for part in "$sparkle"/Versions/B/XPCServices/*.xpc "$sparkle/Versions/B/Autoupdate" "$sparkle/Versions/B/Updater.app"; do
        [[ -e "$part" ]] && sign "$part"
    done
    sign "$sparkle"
fi
sign "$app"
codesign --verify --strict --deep --verbose=2 "$app"

# --- 4. Notarising the app ---------------------------------------------------

notarise() {
    xcrun notarytool submit "$1" --key "$ARALO_NOTARY_KEY_PATH" --key-id "$ARALO_NOTARY_KEY_ID" \
        --issuer "$ARALO_NOTARY_ISSUER" --wait --timeout 1h --output-format json >"$work/notary.json"
    cat "$work/notary.json"
    python3 -c 'import json, sys; sys.exit(json.load(open(sys.argv[1]))["status"] != "Accepted")' "$work/notary.json" \
        || die "notarisation of $(basename "$1") was not accepted"
}
if [[ "$dry_run" -eq 1 ]]; then
    step "Notarisation skipped (dry run)"
else
    step "Notarising Aralo.app"
    ditto -c -k --keepParent "$app" "$work/Aralo.zip"
    notarise "$work/Aralo.zip"
    xcrun stapler staple "$app"
fi

# --- 5. The DMG --------------------------------------------------------------

step "$dmg_name"
stage="$work/dmg"
mkdir -p "$stage"
ditto "$app" "$stage/Aralo.app"
ln -s /Applications "$stage/Applications"
hdiutil create -volname "Aralo $version" -srcfolder "$stage" -fs HFS+ -format UDZO -ov "$out/$dmg_name" >/dev/null
if [[ "$identity" != "-" ]]; then
    codesign --force --timestamp --sign "$identity" "$out/$dmg_name"
    step "Notarising $dmg_name"
    notarise "$out/$dmg_name"
    xcrun stapler staple "$out/$dmg_name"
fi

# --- 6. The appcast ----------------------------------------------------------

step "appcast.xml"
sign_update="$(find "$derived/SourcePackages/artifacts" -path '*/bin/sign_update' -type f | head -n 1)"
[[ -x "$sign_update" ]] || die "Sparkle's sign_update was not found under $derived/SourcePackages/artifacts"
ed_signature="$("$sign_update" --ed-key-file "$keys/sparkle.seed" -p "$out/$dmg_name")"
length="$(stat -f %z "$out/$dmg_name")"
pub_date="$(LC_ALL=C date -u '+%a, %d %b %Y %H:%M:%S +0000')"
cat >"$out/appcast.xml" <<APPCAST
<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
  <channel>
    <title>Aralo</title>
    <link>https://github.com/$repo</link>
    <item>
      <title>Aralo $version</title>
      <pubDate>$pub_date</pubDate>
      <sparkle:version>$build_number</sparkle:version>
      <sparkle:shortVersionString>$version</sparkle:shortVersionString>
      <sparkle:minimumSystemVersion>14.0</sparkle:minimumSystemVersion>
      <sparkle:releaseNotesLink>https://github.com/$repo/releases/tag/v$version</sparkle:releaseNotesLink>
      <enclosure url="$dmg_url" length="$length" type="application/octet-stream" sparkle:edSignature="$ed_signature"/>
    </item>
  </channel>
</rss>
APPCAST

# --- 7. The signed data tables -----------------------------------------------

step "The signed data tables"
mkdir -p "$out/data"
cp data/compat/apps.toml "$out/data/apps.toml"
scripts/sign-data.sh sign-hex "$keys/data.pem" "$out/data/apps.toml"
scripts/sign-data.sh public-hex "$keys/data.pem" >"$out/data/data-tables.pub"

# --- 8. The SBOM -------------------------------------------------------------

step "The SBOM"
# The Rust core the app links, for both Apple targets, without build tools.
sbom_dir="$work/sbom"
mkdir -p "$sbom_dir"
for target in aarch64-apple-darwin x86_64-apple-darwin; do
    # cargo-cyclonedx writes one file into every workspace member's folder;
    # the app links aralo-ffi, so that one is kept and the rest removed.
    cargo cyclonedx --manifest-path crates/aralo-ffi/Cargo.toml --format json --spec-version 1.5 \
        --no-build-deps --target "$target" --override-filename "sbom-$target" -q
    mv "crates/aralo-ffi/sbom-$target.json" "$sbom_dir/aralo-ffi-$target.json"
    find crates -name "sbom-$target.json" -delete
done
# One file for the app: the Rust components of both targets, and the Swift
# packages it embeds, from the resolved package versions.
resolved="$(find apps/macos/Aralo.xcodeproj -name Package.resolved | head -n 1)"
python3 - "$out/Aralo-$version.cdx.json" "$version" "$resolved" "$sbom_dir"/*.json <<'PYTHON'
import json, sys
out, version, resolved, inputs = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4:]
boms = [json.load(open(path)) for path in inputs]
merged = boms[0]
seen = {c["bom-ref"] for c in merged.get("components", [])}
for bom in boms[1:]:
    for component in bom.get("components", []):
        if component["bom-ref"] not in seen:
            merged.setdefault("components", []).append(component)
            seen.add(component["bom-ref"])
pins = json.load(open(resolved)).get("pins", []) if resolved else []
for pin in pins:
    ref = "swift:%s@%s" % (pin["identity"], pin["state"].get("version") or pin["state"]["revision"])
    merged.setdefault("components", []).append({
        "type": "framework" if pin["identity"] == "sparkle" else "library",
        "bom-ref": ref,
        "name": pin["identity"],
        "version": pin["state"].get("version") or pin["state"]["revision"],
        "purl": "pkg:swift/%s@%s" % (pin["location"].removeprefix("https://").removesuffix(".git"),
                                      pin["state"].get("version") or pin["state"]["revision"]),
        "externalReferences": [{"type": "vcs", "url": pin["location"]}],
    })
merged["metadata"]["component"] = {
    "type": "application", "bom-ref": "app:aralo@" + version, "name": "Aralo", "version": version,
}
json.dump(merged, open(out, "w"), indent=2)
print("%d components" % len(merged["components"]))
PYTHON

# --- 9. The Homebrew cask ----------------------------------------------------

step "The Homebrew cask"
dmg_sha="$(shasum -a 256 "$out/$dmg_name" | cut -d ' ' -f 1)"
sed -e "s|@VERSION@|$version|g" -e "s|@SHA256@|$dmg_sha|g" -e "s|@REPO@|$repo|g" \
    packaging/homebrew/aralo.rb.in >"$out/aralo.rb"

# --- 10. Checksums -----------------------------------------------------------

{
    echo "version=$version"
    echo "build=$build_number"
    echo "dry_run=$dry_run"
    echo "dmg=$dmg_name"
    echo "sparkle_public_key=$sparkle_public"
} >"$out/release.env"

step "SHA256SUMS"
(cd "$out" && find . -type f ! -name SHA256SUMS | sed 's|^\./||' | LC_ALL=C sort | xargs shasum -a 256) >"$work/SHA256SUMS"
mv "$work/SHA256SUMS" "$out/SHA256SUMS"
cat "$out/SHA256SUMS"

echo
echo "release: $out is ready. Check it with: scripts/verify-release.sh $([[ "$dry_run" -eq 1 ]] && echo "--dry-run ")$out"

#!/usr/bin/env bash
# Checks a release that scripts/release.sh made, before anything is published.
# The tests behind section 9's "Code signing and runtime", "Signed data
# updates" and "Supply chain" rows (docs/threat-model.md):
#
#   a. Checksums       SHA256SUMS matches every file.
#   b. Signature       The app in the DMG passes a strict, deep codesign check.
#   c. Runtime         The app and every piece of code in it has the hardened
#                      runtime and no entitlement.
#   d. Universal       The app runs on Apple silicon and Intel.
#   e. Update feed     Info.plist names an https feed on github.com and a
#                      32-byte EdDSA key; the appcast's version, length and URL
#                      are the DMG's, and its signature checks against the
#                      key inside the app, with OpenSSL rather than Sparkle.
#   f. Data tables     apps.toml is the committed table, and its signature
#                      checks against the key the release names.
#   g. SBOM            A CycloneDX file naming every crate the core links on
#                      both Apple targets, and Sparkle.
#   h. Cask            Ruby that parses, with the DMG's version and SHA-256.
#   i. Developer ID    Not for a dry run: the app and the DMG are signed with a
#                      Developer ID, notarised and stapled, and Gatekeeper
#                      accepts them. The data-table key is the committed one.
#
# Usage: scripts/verify-release.sh [--dry-run] [DIR]     (DIR default: dist)
# ARALO_ALLOW_SINGLE_ARCH=1 lets d pass on a one-architecture local build.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
dry_run=0
if [[ "${1:-}" == "--dry-run" ]]; then
    dry_run=1
    shift
fi
out="$(cd "${1:-dist}" && pwd)"
cd "$root"

failures=0
pass() { echo "ok    $1"; }
fail() { echo "FAIL  $1" >&2; failures=$((failures + 1)); }

[[ -f "$out/release.env" ]] || { echo "verify-release: $out/release.env not found; run scripts/release.sh" >&2; exit 2; }
version="$(sed -n 's/^version=//p' "$out/release.env")"
build="$(sed -n 's/^build=//p' "$out/release.env")"
dmg="$out/$(sed -n 's/^dmg=//p' "$out/release.env")"
made_dry="$(sed -n 's/^dry_run=//p' "$out/release.env")"
if [[ "$dry_run" -eq 0 && "$made_dry" == 1 ]]; then
    echo "verify-release: $out is a dry run; check it with --dry-run" >&2
    exit 2
fi

work="$(mktemp -d "${TMPDIR:-/tmp}/aralo-verify.XXXXXX")"
mount="$work/mount"
cleanup() {
    hdiutil detach "$mount" -quiet 2>/dev/null || true
    rm -rf "$work"
}
trap cleanup EXIT

# --- a. Checksums --------------------------------------------------------------
if (cd "$out" && shasum -a 256 -c SHA256SUMS >/dev/null); then
    pass "checksums: every file in SHA256SUMS matches ($(wc -l <"$out/SHA256SUMS" | tr -d ' ') files)"
else
    fail "checksums: SHA256SUMS does not match the files in $out"
fi
listed="$(cut -c 67- "$out/SHA256SUMS" | LC_ALL=C sort)"
present="$(cd "$out" && find . -type f ! -name SHA256SUMS | sed 's|^\./||' | LC_ALL=C sort)"
[[ "$listed" == "$present" ]] || fail "checksums: SHA256SUMS does not list exactly the files in $out"

# --- b. Signature --------------------------------------------------------------
mkdir -p "$mount"
hdiutil attach "$dmg" -readonly -nobrowse -noautoopen -mountpoint "$mount" -quiet
app="$mount/Aralo.app"
if [[ ! -d "$app" ]]; then
    fail "signature: the DMG holds no Aralo.app"
    exit 1
fi
[[ -L "$mount/Applications" ]] || fail "dmg: no Applications link to drag the app to"
if codesign --verify --strict --deep "$app" 2>"$work/codesign.txt"; then
    pass "signature: Aralo.app passes codesign --verify --strict --deep"
else
    fail "signature: $(cat "$work/codesign.txt")"
fi

# --- c. Hardened runtime, no entitlements --------------------------------------
# Every signed piece: the app, Sparkle's framework, helpers and XPC services.
code=("$app")
while IFS= read -r piece; do code+=("$piece"); done < <(
    find "$app/Contents" \( -name '*.framework' -o -name '*.xpc' -o -name '*.app' \) -prune -print
    find "$app/Contents/Frameworks" -type f -perm -u+x -name Autoupdate 2>/dev/null
)
while IFS= read -r nested; do code+=("$nested"); done < <(
    find "$app/Contents/Frameworks" -path '*/Versions/B/*' \( -name '*.xpc' -o -name '*.app' \) -prune -print 2>/dev/null
)
runtime_problems=""
for piece in "${code[@]}"; do
    details="$(codesign --display --verbose=2 "$piece" 2>&1 || true)"
    grep -qE 'flags=0x[0-9a-f]*\(.*runtime' <<<"$details" || runtime_problems="$runtime_problems ${piece#"$mount"/} has no hardened runtime;"
    entitlements="$(codesign --display --entitlements - --xml "$piece" 2>/dev/null || true)"
    if grep -q '<key>' <<<"$entitlements"; then
        runtime_problems="$runtime_problems ${piece#"$mount"/} has entitlements;"
    fi
done
if [[ -z "$runtime_problems" ]]; then
    pass "runtime: ${#code[@]} signed pieces, every one hardened, none with an entitlement"
else
    fail "runtime:$runtime_problems"
fi

# --- d. Universal --------------------------------------------------------------
archs="$(lipo -archs "$app/Contents/MacOS/Aralo")"
if [[ " $archs " == *" arm64 "* && " $archs " == *" x86_64 "* ]]; then
    pass "universal: the app is $archs"
elif [[ "${ARALO_ALLOW_SINGLE_ARCH:-}" == 1 ]]; then
    echo "note  universal: the app is only $archs (ARALO_ALLOW_SINGLE_ARCH=1)"
else
    fail "universal: the app is $archs; a release is arm64 and x86_64 (ADR-0010)"
fi

# --- e. Update feed ------------------------------------------------------------
plist="$app/Contents/Info.plist"
read_plist() { /usr/libexec/PlistBuddy -c "Print :$1" "$plist" 2>/dev/null || true; }
feed="$(read_plist SUFeedURL)"
key="$(read_plist SUPublicEDKey)"
key_bytes="$(printf '%s' "$key" | base64 --decode 2>/dev/null | wc -c | tr -d ' ')"
if [[ "$feed" =~ ^https://github\.com/ && "$key_bytes" == 32 ]]; then
    pass "update feed: $feed, with a 32-byte EdDSA key"
else
    fail "update feed: SUFeedURL '$feed' and SUPublicEDKey '$key' are not a release feed (placeholders left in?)"
fi
[[ "$key" == "$(sed -n 's/^sparkle_public_key=//p' "$out/release.env")" ]] \
    || fail "update feed: the app's key is not the one the release signed with"
[[ "$(read_plist CFBundleShortVersionString)" == "$version" && "$(read_plist CFBundleVersion)" == "$build" ]] \
    || fail "update feed: the app says $(read_plist CFBundleShortVersionString) ($(read_plist CFBundleVersion)), the release $version ($build)"

appcast="$(python3 - "$out/appcast.xml" <<'PYTHON'
import sys
import xml.etree.ElementTree as ET
ns = {"sparkle": "http://www.andymatuschak.org/xml-namespaces/sparkle"}
item = ET.parse(sys.argv[1]).getroot().find("channel/item")
enclosure = item.find("enclosure")
print(item.findtext("sparkle:version", namespaces=ns))
print(item.findtext("sparkle:shortVersionString", namespaces=ns))
print(enclosure.get("url"))
print(enclosure.get("length"))
print(enclosure.get("{%s}edSignature" % ns["sparkle"]))
PYTHON
)"
{ read -r cast_build; read -r cast_version; read -r cast_url; read -r cast_length; read -r cast_signature; } <<<"$appcast"
problems=""
[[ "$cast_build" == "$build" && "$cast_version" == "$version" ]] || problems="$problems version $cast_version ($cast_build);"
[[ "$cast_url" == https://github.com/*/releases/download/v"$version"/"$(basename "$dmg")" ]] || problems="$problems url $cast_url;"
[[ "$cast_length" == "$(stat -f %z "$dmg")" ]] || problems="$problems length $cast_length;"
if ! scripts/sign-data.sh verify-base64 "$key" "$dmg" "$cast_signature" 2>/dev/null; then
    problems="$problems the edSignature does not check against the app's key;"
fi
if [[ -z "$problems" ]]; then
    pass "update feed: the appcast names this DMG, and its EdDSA signature checks against the key in the app"
else
    fail "update feed: appcast.xml:$problems"
fi

# --- f. Data tables ------------------------------------------------------------
cmp -s "$out/data/apps.toml" data/compat/apps.toml || fail "data tables: apps.toml is not data/compat/apps.toml"
if scripts/sign-data.sh verify-hex "$(cat "$out/data/data-tables.pub")" "$out/data/apps.toml" "$out/data/apps.toml.sig" 2>/dev/null; then
    pass "data tables: apps.toml.sig checks against the release's key"
else
    fail "data tables: apps.toml.sig does not check"
fi

# --- g. SBOM -------------------------------------------------------------------
sbom="$out/Aralo-$version.cdx.json"
for target in aarch64-apple-darwin x86_64-apple-darwin; do
    cargo tree -p aralo-ffi -e normal --target "$target" --prefix none --format '{p}' 2>/dev/null \
        | sed 's/ (\*)$//; s/ (proc-macro)$//' | awk '{print $1}'
done | LC_ALL=C sort -u >"$work/crates.txt"
if missing="$(python3 - "$sbom" "$work/crates.txt" <<'PYTHON'
import json, sys
bom = json.load(open(sys.argv[1]))
assert bom.get("bomFormat") == "CycloneDX", "not CycloneDX"
names = {c["name"] for c in bom.get("components", [])} | {bom["metadata"].get("component", {}).get("name")}
wanted = [line.strip() for line in open(sys.argv[2]) if line.strip() and line.strip() != "aralo-ffi"]
missing = [name for name in wanted + ["sparkle"] if name not in names]
print(" ".join(missing))
sys.exit(1 if missing or len(wanted) < 50 else 0)
PYTHON
)"; then
    pass "sbom: $(basename "$sbom") names all $(wc -l <"$work/crates.txt" | tr -d ' ') crates the core links, and Sparkle"
else
    fail "sbom: $(basename "$sbom") is missing: $missing"
fi

# --- h. Cask -------------------------------------------------------------------
cask="$out/aralo.rb"
dmg_sha="$(shasum -a 256 "$dmg" | cut -d ' ' -f 1)"
if ruby -c "$cask" >/dev/null 2>&1 \
    && grep -q "version \"$version\"" "$cask" && grep -q "sha256 \"$dmg_sha\"" "$cask" \
    && ! grep -q '@[A-Z0-9]*@' "$cask"; then
    pass "cask: aralo.rb parses and names version $version and the DMG's SHA-256"
else
    fail "cask: aralo.rb does not parse, or its version or sha256 is not this DMG's"
fi

# --- i. Developer ID, notarisation, Gatekeeper ---------------------------------
if [[ "$dry_run" -eq 1 ]]; then
    echo "skip  developer id: a dry run is signed ad hoc and not notarised"
else
    details="$(codesign --display --verbose=2 "$app" 2>&1)"
    grep -q '^Authority=Developer ID Application:' <<<"$details" || fail "developer id: the app is not signed with a Developer ID"
    grep -qE '^TeamIdentifier=[A-Z0-9]{10}$' <<<"$details" || fail "developer id: the app has no team identifier"
    grep -q '^Timestamp=' <<<"$details" || fail "developer id: the signature has no secure timestamp"
    xcrun stapler validate "$app" >/dev/null 2>&1 || fail "notarisation: no ticket is stapled to the app"
    xcrun stapler validate "$dmg" >/dev/null 2>&1 || fail "notarisation: no ticket is stapled to the DMG"
    spctl --assess --type execute "$app" 2>/dev/null || fail "gatekeeper: spctl refuses the app"
    spctl --assess --type open --context context:primary-signature "$dmg" 2>/dev/null || fail "gatekeeper: spctl refuses the DMG"
    committed="$(sed -n '/^[[:space:]]*[0-9a-fA-F]\{64\}[[:space:]]*$/p' data/keys/data-tables.pub | tr -d '[:space:]' | tr 'A-F' 'a-f')"
    [[ "$(tr -d '[:space:]' <"$out/data/data-tables.pub")" == "$committed" ]] \
        || fail "data tables: signed with a key other than data/keys/data-tables.pub"
    [[ "$failures" -eq 0 ]] && pass "developer id: signed, notarised, stapled, and Gatekeeper accepts the app and the DMG"
fi

if [[ "$failures" -ne 0 ]]; then
    echo >&2
    echo "verify-release: $failures problem(s) found" >&2
    exit 1
fi
echo "verify-release: all checks passed"

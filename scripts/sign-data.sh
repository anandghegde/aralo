#!/usr/bin/env bash
# Ed25519 signing for the signed data tables and the Sparkle appcast.
#
#   scripts/sign-data.sh new-key <key.pem>
#       Writes a new private key. For a dry run, or to make the real one once.
#   scripts/sign-data.sh public-hex <key.pem>
#       Prints the raw public key as 64 hex digits: the line for
#       data/keys/data-tables.pub.
#   scripts/sign-data.sh public-base64 <key.pem>
#       Prints the raw public key in base64: Sparkle's SUPublicEDKey.
#   scripts/sign-data.sh seed-base64 <key.pem>
#       Prints the private key as its 32-byte seed in base64: the form the
#       release secrets take (docs/releasing.md). Keep it secret.
#   scripts/sign-data.sh from-seed <base64-seed> <key.pem>
#       Turns a Sparkle private key (`generate_keys -x`: the 32-byte seed in
#       base64) into a PEM key file this script can use.
#   scripts/sign-data.sh sign-hex <key.pem> <file>
#       Writes <file>.sig: the signature over the file's bytes, 128 hex digits.
#       The core checks it with crates/aralo-core/src/signed.rs.
#   scripts/sign-data.sh sign-base64 <key.pem> <file>
#       Prints the signature in base64: Sparkle's sparkle:edSignature.
#   scripts/sign-data.sh verify-hex <public-hex> <file> <file.sig>
#   scripts/sign-data.sh verify-base64 <public-base64> <file> <signature-base64>
#
# Needs OpenSSL 3 (LibreSSL, which macOS ships as /usr/bin/openssl, has no
# Ed25519 in pkeyutl). Set OPENSSL to choose one; otherwise Homebrew's
# openssl@3 is used when it is there.
set -euo pipefail

find_openssl() {
    if [[ -n "${OPENSSL:-}" ]]; then
        echo "$OPENSSL"
        return
    fi
    local candidate
    for candidate in /opt/homebrew/opt/openssl@3/bin/openssl /usr/local/opt/openssl@3/bin/openssl openssl; do
        if command -v "$candidate" >/dev/null 2>&1 && "$candidate" version 2>/dev/null | grep -q '^OpenSSL 3'; then
            echo "$candidate"
            return
        fi
    done
    echo "sign-data: OpenSSL 3 is required (brew install openssl@3, or set OPENSSL)" >&2
    exit 2
}

SSL="$(find_openssl)"

# The DER of an Ed25519 SubjectPublicKeyInfo is a fixed 12-byte header and
# then the 32 raw key bytes.
public_raw() {
    "$SSL" pkey -in "$1" -pubout -outform DER | tail -c 32
}

# Writes a PEM public key made from raw hex, for pkeyutl -verify.
public_pem_from_hex() {
    local hex="$1" out="$2"
    { printf '302a300506032b6570032100' ; printf '%s' "$hex"; } | xxd -r -p | "$SSL" pkey -pubin -inform DER -out "$out"
}

command="${1:-}"
case "$command" in
    new-key)
        "$SSL" genpkey -algorithm ed25519 -out "$2"
        chmod 600 "$2"
        ;;
    public-hex)
        public_raw "$2" | xxd -p -c 64
        ;;
    public-base64)
        public_raw "$2" | base64
        ;;
    seed-base64)
        # The inverse of from-seed: the value for a *_PRIVATE_KEY / *_SIGNING_KEY secret.
        "$SSL" pkey -in "$2" -outform DER | tail -c 32 | base64
        ;;
    from-seed)
        # PKCS#8 for Ed25519: a fixed 16-byte header and then the 32-byte seed.
        seed_hex="$(printf '%s' "$2" | base64 --decode | head -c 32 | xxd -p -c 64)"
        if [[ ${#seed_hex} -ne 64 ]]; then
            echo "sign-data: the Sparkle key must hold at least 32 bytes" >&2
            exit 1
        fi
        umask 077
        { printf '302e020100300506032b657004220420'; printf '%s' "$seed_hex"; } | xxd -r -p \
            | "$SSL" pkey -inform DER -out "$3"
        ;;
    sign-hex)
        "$SSL" pkeyutl -sign -rawin -inkey "$2" -in "$3" | xxd -p -c 128 >"$3.sig"
        ;;
    sign-base64)
        "$SSL" pkeyutl -sign -rawin -inkey "$2" -in "$3" | base64 | tr -d '\n'
        echo
        ;;
    verify-hex | verify-base64)
        work="$(mktemp -d "${TMPDIR:-/tmp}/aralo-sign.XXXXXX")"
        trap 'rm -rf "$work"' EXIT
        if [[ "$command" == verify-hex ]]; then
            public_pem_from_hex "$2" "$work/public.pem"
            tr -d ' \n' <"$4" | xxd -r -p >"$work/signature"
        else
            public_pem_from_hex "$(printf '%s' "$2" | base64 --decode | xxd -p -c 64)" "$work/public.pem"
            printf '%s' "$4" | base64 --decode >"$work/signature"
        fi
        "$SSL" pkeyutl -verify -rawin -pubin -inkey "$work/public.pem" -in "$3" -sigfile "$work/signature" >/dev/null
        ;;
    *)
        sed -n '2,/^set -euo/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'
        exit 2
        ;;
esac

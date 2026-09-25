#!/usr/bin/env bash
# Runs the conformance suite (plan 7.4) against every endpoint in
# conformance/endpoints.toml that has a key: the ones whose `key_env` names an
# environment variable that is set. The nightly workflow sets each from the
# repository secret of the same name; one that is not set is skipped, by name.
#
# Each endpoint gets a profile with no key in a state folder of its own, and
# the key goes to `aralo conformance --key-env`, which sends it for the run
# and saves it nowhere. No key is ever an argument or printed.
#
# Usage: scripts/conformance-hosted.sh <aralo binary> <report folder> [endpoints.toml]
#        Exit status 0 = every endpoint that ran conforms.
#
# Needs: bash 3.2 or later, python3 3.11 or later (for tomllib).

set -euo pipefail

aralo="$1"
out="$2"
repo="$(cd "$(dirname "$0")/.." && pwd)"
list="${3:-$repo/conformance/endpoints.toml}"
mkdir -p "$out"

# name, base URL, model and key variable, a line each, tab between.
endpoints="$(python3 - "$list" <<'PY'
import sys, tomllib
with open(sys.argv[1], "rb") as file:
    for endpoint in tomllib.load(file).get("endpoint", []):
        if endpoint.get("key_env"):
            print("\t".join([endpoint["name"], endpoint["base_url"], endpoint["model"], endpoint["key_env"]]))
PY
)"

ran=0
failed=""
while IFS=$'\t' read -r name base_url model key_env; do
    [ -n "$name" ] || continue
    if [ -z "${!key_env:-}" ]; then
        echo "skip  $name: $key_env is not set"
        continue
    fi
    slug="$(printf '%s-%s' "$name" "$model" | tr 'A-Z' 'a-z' | tr -c 'a-z0-9.\n' '-' | tr -s '-')"
    state="$(mktemp -d)"
    echo "run   $name, $model"
    ARALO_STATE="$state" "$aralo" ai on >/dev/null
    ARALO_STATE="$state" "$aralo" ai add "$name" --base-url "$base_url" --model "$model" >/dev/null
    if ! ARALO_STATE="$state" "$aralo" conformance --profile "$name" --key-env "$key_env" \
        --evaluation --report "$out/$slug.json"; then
        failed="$failed $name"
    fi
    rm -rf "$state"
    ran=$((ran + 1))
done <<< "$endpoints"

echo
echo "$ran endpoints ran; reports in $out"
if [ -n "$failed" ]; then
    echo "did not conform:$failed"
    exit 1
fi

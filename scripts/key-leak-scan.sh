#!/usr/bin/env bash
# The key-leak scan (PRD P13, plan task 4.4): after a full test run, no API key
# is in any file the run wrote.
#
# The workspace's tests run with HOME, ARALO_STATE and TMPDIR moved into a scan
# folder, so everything they write outside target/ lands where this script can
# see it: the state folder, profiles.toml, exported libraries, logs and crash
# reports. The tests use a canary key (`sk-aralo-canary-…`) and a handful of
# fake ones. Afterwards the script reads every file under the scan folder, and
# every file in the repository written during the run, for
#
#   the canary, and
#   anything shaped like a provider key: sk-, gsk_, xai-, pplx-, AIza, hf_,
#   r8_ or inf_ followed by at least 20 key characters.
#
# target/ and .git are not read: compiled tests hold the canary as a literal.
#
# Most tests write into a temporary folder that is deleted when the test ends,
# which would leave nothing to read. Every test makes that folder through
# `aralo_testkit::tempdir()`, and the script sets ARALO_KEEP_TEMP, which tells
# it to leave the folder on disk. They all land in the scan folder's TMPDIR,
# and the scan folder is removed afterwards unless a key was found. A run that
# wrote fewer than MIN_FILES files fails too: a scan that reads next to
# nothing proves nothing, and that is how this scan once passed while reading
# no file at all.
#
# Needs: bash 3.2 or later, cargo, grep, find.
# Usage: scripts/key-leak-scan.sh [extra cargo test arguments]
#        Exit status 0 = the run passed, at least MIN_FILES files were read and
#        nothing was found. A filter that leaves most tests out fails the
#        minimum.

set -euo pipefail

# The fewest files a full test run writes. A healthy run writes about 22,000;
# raise this if it grows, never lower it to make a run pass.
MIN_FILES=5000

repo="$(cd "$(dirname "$0")/.." && pwd)"
scan="$(mktemp -d "${TMPDIR:-/tmp}/aralo-leak-scan.XXXXXX")"
marker="$scan/.started"
mkdir -p "$scan/home" "$scan/state" "$scan/tmp"
touch "$marker"
# Files in the tree get a timestamp one second after the marker at the
# earliest, so a file written in the same second still counts as new.
sleep 1

# Cargo and rustup live under the real HOME; keep pointing at them.
export CARGO_HOME="${CARGO_HOME:-$HOME/.cargo}"
export RUSTUP_HOME="${RUSTUP_HOME:-$HOME/.rustup}"

echo "key-leak-scan: running the workspace's tests with HOME=$scan/home"
status=0
(
    cd "$repo"
    HOME="$scan/home" ARALO_STATE="$scan/state" TMPDIR="$scan/tmp" ARALO_KEEP_TEMP=1 \
        cargo test --workspace --quiet "$@"
) || status=$?
if [ "$status" -ne 0 ]; then
    echo "key-leak-scan: the tests failed (exit $status); the scan still runs" >&2
fi

pattern='sk-aralo-canary-|(sk-|gsk_|xai-|pplx-|AIza|hf_|r8_|inf_)[A-Za-z0-9_-]{20,}'

list="$scan/.written"
{
    find "$scan" -type f ! -name .started ! -name .written
    find "$repo" \( -path "$repo/target" -o -path "$repo/.git" -o -name .build \
        -o -name DerivedData \) -prune -o -type f -newer "$marker" -print
} >"$list"

found=0
while IFS= read -r file; do
    if matches="$(grep -EIao "$pattern" "$file" 2>/dev/null)"; then
        found=1
        # The match is shortened: a real key found here must not be printed
        # whole into a CI log.
        echo "$matches" | while IFS= read -r match; do
            echo "LEAK  $file: ${match:0:12}… (${#match} characters)"
        done
    fi
done <"$list"

count="$(wc -l <"$list" | tr -d ' ')"
if [ "$found" -ne 0 ]; then
    echo "key-leak-scan: FAILED: a key-shaped string was written (files kept in $scan)" >&2
    exit 1
fi
if [ "$count" -lt "$MIN_FILES" ]; then
    rm -rf "$scan"
    echo "key-leak-scan: FAILED: only $count files were read, fewer than $MIN_FILES." >&2
    echo "key-leak-scan: did a test's temporary folder skip aralo_testkit::tempdir(), or did a filter leave tests out?" >&2
    exit 1
fi
rm -rf "$scan"
echo "key-leak-scan: clean ($count files written during the run were read)"
exit "$status"

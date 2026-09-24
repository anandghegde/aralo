#!/usr/bin/env bash
# Fetches the embedding model that search by meaning runs on (PRD A4,
# ADR-0016): MinishLab's potion-base-8M, a static model distilled from BAAI's
# bge-base-en-v1.5. Both are MIT licensed.
#
# The model is not in the repository: 30 MB of weights do not belong in Git
# history. This puts it in models/potion-base-8M, where the Rust tests, the
# CLI (`aralo search --model`) and the app build look for it. Every file is
# checked against the SHA-256 recorded below, so what ships is what was pinned.
#
# Usage: scripts/fetch-model.sh [folder]
#
# ARALO_MODEL_UNPINNED=1 accepts files whose checksum is not recorded yet and
# prints the lines to record. It exists for pinning a new model, once.
#
# Needs: bash 3.2 or later, curl, and shasum or sha256sum.

set -euo pipefail

REPOSITORY="minishlab/potion-base-8M"
# The Hugging Face commit the checksums below were taken from.
REVISION="main"
FILES="config.json tokenizer.json model.safetensors"

# The SHA-256 of each file at REVISION. Empty: not pinned yet.
pinned() {
    case "$1" in
        config.json) echo "" ;;
        tokenizer.json) echo "" ;;
        model.safetensors) echo "" ;;
    esac
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST="${1:-$ROOT/models/potion-base-8M}"

if command -v sha256sum >/dev/null 2>&1; then
    sha256() { sha256sum "$1" | cut -d' ' -f1; }
elif command -v shasum >/dev/null 2>&1; then
    sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }
else
    echo "fetch-model: need sha256sum or shasum" >&2
    exit 2
fi

mkdir -p "$DEST"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/aralo-model.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT

unpinned=0
for file in $FILES; do
    want="$(pinned "$file")"
    if [ -n "$want" ] && [ -f "$DEST/$file" ] && [ "$(sha256 "$DEST/$file")" = "$want" ]; then
        echo "ok    $file (already here)"
        continue
    fi
    url="https://huggingface.co/$REPOSITORY/resolve/$REVISION/$file"
    curl --fail --silent --show-error --location --retry 3 --dump-header "$TMP/$file.headers" \
        --output "$TMP/$file" "$url"
    got="$(sha256 "$TMP/$file")"
    if [ -z "$want" ]; then
        if [ "${ARALO_MODEL_UNPINNED:-}" != "1" ]; then
            echo "fetch-model: $file has no recorded checksum. It downloaded as" >&2
            echo "    $file) echo \"$got\" ;;" >&2
            echo "Read it, record it in scripts/fetch-model.sh, and run this again." >&2
            exit 1
        fi
        commit="$(grep -i '^x-repo-commit:' "$TMP/$file.headers" | tail -1 | tr -d '\r' | cut -d' ' -f2)"
        echo "pin   $file) echo \"$got\" ;;   # $REPOSITORY at ${commit:-unknown}"
        unpinned=1
    elif [ "$got" != "$want" ]; then
        echo "fetch-model: $file does not match its checksum" >&2
        echo "    recorded   $want" >&2
        echo "    downloaded $got" >&2
        exit 1
    else
        echo "ok    $file"
    fi
    mv "$TMP/$file" "$DEST/$file"
done

if [ "$unpinned" = "1" ]; then
    echo "fetch-model: accepted unpinned files because ARALO_MODEL_UNPINNED=1; pin them" >&2
fi
echo "The model is in $DEST"

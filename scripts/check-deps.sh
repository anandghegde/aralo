#!/usr/bin/env bash
# Structural checks that turn Aralo's architecture rules into build failures.
#
#   a. Pure crates        aralo-engine's, aralo-template's and aralo-embed's
#                         normal dependency closures stay inside short
#                         allow-lists (PRD P1, PRD A4, ADR-0005, ADR-0014,
#                         ADR-0016).
#   b. One-way graph      internal crates only depend downwards (plan section 10).
#   c. No unsafe in ffi   aralo-ffi contains no hand-written `unsafe`.
#   d. Silent engine      aralo-engine contains no printing or logging call.
#   e. Timeless template  aralo-template reads no clock (ADR-0014).
#   f. One HTTP door      `reqwest` is named only inside the network guard
#                         (ADR-0007).
#
# Needs: bash 3.2 or later, cargo, python3 (standard library only). No jq.
# Usage: scripts/check-deps.sh        Exit status 0 = every check passed.
#
# If a check fails because of a real change, fix the change. Do not weaken the
# check without a new ADR: these rules are what make the privacy promises
# properties of the build.

set -euo pipefail

# --- The rules. Edit these tables, not the code below. -----------------------

# Crates allowed in a pure crate's normal (non-dev, non-build) dependency
# closure, direct or transitive. Space separated. Dev-dependencies are exempt:
# they never reach a shipped binary.
ENGINE_CRATE="aralo-engine"
ENGINE_ALLOWED="zeroize"
# The template crate is the other pure one: no files, no network, no clock, and
# it compiles to WebAssembly for the browser extension. Everything a
# placeholder needs arrives as an argument.
TEMPLATE_CRATE="aralo-template"
TEMPLATE_ALLOWED="unicode-segmentation"
# The embedding runtime turns the user's snippets into vectors, and search by
# meaning promises that happens on this machine. Nothing in this list can open
# a socket; an HTTP client, a model hub or a telemetry crate arriving here is
# the promise breaking.
EMBED_CRATE="aralo-embed"
EMBED_ALLOWED="arrayvec blake3 cfg-if constant_time_eq cpufeatures itoa libc memchr proc-macro2 quote serde serde_core serde_derive serde_json syn thiserror thiserror-impl tinyvec unicode-ident unicode-normalization unicode-properties zmij"

# Layering. Every workspace member must appear in exactly one layer.
#   layer 0  pure base: no internal dependencies at all
#   layer 1  may depend on layers 0 and 1
#   layer 2  may depend on layers 0 and 1
#   layer 3  may depend on layers 0 to 2, never on each other
LAYER_0="aralo-engine aralo-snippet aralo-template"
LAYER_1="aralo-library aralo-ai aralo-embed aralo-providers aralo-import aralo-script"
LAYER_2="aralo-core"
LAYER_3="aralo-ffi aralo-cli"

FFI_SRC="crates/aralo-ffi/src"
ENGINE_SRC="crates/aralo-engine/src"
TEMPLATE_SRC="crates/aralo-template/src"
# The network guard: the only code that may name the HTTP client.
GUARD_SRC="crates/aralo-ai/src/guard"

# -----------------------------------------------------------------------------

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

for tool in cargo python3; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        echo "check-deps: \`$tool\` is required but was not found on PATH" >&2
        exit 2
    fi
done

failures=0
pass() { echo "ok    $1"; }
fail() { echo "FAIL  $1" >&2; failures=$((failures + 1)); }

METADATA="$(mktemp "${TMPDIR:-/tmp}/aralo-metadata.XXXXXX")"
trap 'rm -f "$METADATA"' EXIT

if ! cargo metadata --format-version 1 >"$METADATA"; then
    echo "check-deps: \`cargo metadata\` failed; see the error above" >&2
    exit 2
fi

# --- a + b: the dependency graph ---------------------------------------------
# The Python below only reads the metadata file and prints findings. It exits
# with the number of problems it found.

graph_status=0
ENGINE_CRATE="$ENGINE_CRATE" ENGINE_ALLOWED="$ENGINE_ALLOWED" \
TEMPLATE_CRATE="$TEMPLATE_CRATE" TEMPLATE_ALLOWED="$TEMPLATE_ALLOWED" \
EMBED_CRATE="$EMBED_CRATE" EMBED_ALLOWED="$EMBED_ALLOWED" \
LAYER_0="$LAYER_0" LAYER_1="$LAYER_1" LAYER_2="$LAYER_2" LAYER_3="$LAYER_3" \
python3 - "$METADATA" <<'PYTHON' || graph_status=$?
import json
import os
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    meta = json.load(handle)

packages = {pkg["id"]: pkg for pkg in meta["packages"]}
members = set(meta["workspace_members"])
nodes = {node["id"]: node for node in meta["resolve"]["nodes"]}
failed = 0


def ok(message):
    print("ok    " + message)


def fail(message):
    global failed
    failed += 1
    sys.stderr.write("FAIL  " + message + "\n")


# --- a. Pure crates -----------------------------------------------------------
# Each entry: the crate, what it may depend on, and the promise that rests on
# it. The promise goes in the failure message, because whoever reads it is
# deciding whether to weaken the rule.
pure = [
    (
        os.environ["ENGINE_CRATE"],
        set(os.environ["ENGINE_ALLOWED"].split()),
        "This crate sees keystrokes (PRD P1, ADR-0005)",
    ),
    (
        os.environ["TEMPLATE_CRATE"],
        set(os.environ["TEMPLATE_ALLOWED"].split()),
        "This crate has no files, no network and no clock, and it compiles to "
        "WebAssembly (ADR-0014)",
    ),
    (
        os.environ["EMBED_CRATE"],
        set(os.environ["EMBED_ALLOWED"].split()),
        "This crate reads the user's snippets to embed them, and search by "
        "meaning never leaves the machine (PRD A4, ADR-0016)",
    ),
]

for pure_name, allowed, promise in pure:
    pure_ids = [pid for pid in members if packages[pid]["name"] == pure_name]
    if len(pure_ids) != 1:
        fail("crate purity: workspace member `%s` not found" % pure_name)
        continue

    # Walk normal edges only. In `dep_kinds`, kind None is a normal dependency;
    # "dev" and "build" are skipped. The resolve graph covers every target
    # platform and the workspace's unified features, which is the strict reading.
    start = pure_ids[0]
    parent = {}
    queue = [start]
    while queue:
        current = queue.pop(0)
        for dep in nodes[current]["deps"]:
            if not any(kind.get("kind") is None for kind in dep["dep_kinds"]):
                continue
            if dep["pkg"] not in parent and dep["pkg"] != start:
                parent[dep["pkg"]] = current
                queue.append(dep["pkg"])

    offenders = []
    for pid in parent:
        name = packages[pid]["name"]
        if name in allowed:
            continue
        chain = [name]
        step = parent[pid]
        while step != start:
            chain.append(packages[step]["name"])
            step = parent[step]
        chain.append(pure_name)
        offenders.append((name, " -> ".join(reversed(chain))))

    if offenders:
        for name, chain in sorted(offenders):
            fail(
                "crate purity: `%s` is in %s's normal dependency closure (%s). "
                "Allowed: %s. %s: remove the dependency, or make the case for "
                "it in a new ADR."
                % (
                    name,
                    pure_name,
                    chain,
                    ", ".join(sorted(allowed)) or "nothing",
                    promise,
                )
            )
    else:
        closure = sorted(packages[pid]["name"] for pid in parent)
        ok(
            "crate purity: %s depends on %s (allowed: %s)"
            % (pure_name, ", ".join(closure) or "nothing", ", ".join(sorted(allowed)))
        )

# --- b. One-way crate graph -----------------------------------------------------
layer_of = {}
before = failed
for layer in range(4):
    for name in os.environ["LAYER_%d" % layer].split():
        if name in layer_of:
            fail("crate graph: `%s` is listed in layer %d and layer %d" % (name, layer_of[name], layer))
        layer_of[name] = layer

member_names = {packages[pid]["name"] for pid in members}

for name in sorted(member_names):
    if name not in layer_of:
        fail(
            "crate graph: workspace crate `%s` is not in the layer table. "
            "Add it to a layer in scripts/check-deps.sh and to docs/architecture.md." % name
        )

for name in sorted(layer_of):
    if name not in member_names:
        fail("crate graph: `%s` is in the layer table but is not a workspace member" % name)


def may_depend(own, other):
    """Whether a crate in layer `own` may depend on a crate in layer `other`."""
    if own == 1:
        return other <= 1
    return other < own


for pid in sorted(members, key=lambda item: packages[item]["name"]):
    pkg = packages[pid]
    own = layer_of.get(pkg["name"])
    if own is None:
        continue
    for dep in pkg["dependencies"]:
        # Manifest-level dependencies, every kind: a dev- or build-dependency
        # that points upwards couples the crates just as a normal one does.
        if dep["name"] not in member_names or dep.get("path") is None:
            continue
        other = layer_of.get(dep["name"])
        if other is None or may_depend(own, other):
            continue
        kind = dep.get("kind") or "normal"
        if own == 0:
            rule = "layer 0 is the pure base and has no internal dependencies"
        elif own == other:
            rule = "crates in layer %d never depend on each other" % own
        else:
            rule = "a crate may only depend on lower layers"
        fail(
            "crate graph: `%s` (layer %d) has a %s dependency on `%s` (layer %d); %s."
            % (pkg["name"], own, kind, dep["name"], other, rule)
        )

if failed == before:
    ok("crate graph: %d workspace crates, every internal dependency points downwards" % len(member_names))

sys.exit(failed)
PYTHON

if [ "$graph_status" -ne 0 ]; then
    failures=$((failures + graph_status))
fi

# --- c + d: source greps --------------------------------------------------------
# A line that is nothing but a `//` comment is never code, so it is skipped;
# every other occurrence counts, including one inside a string.

# Prints `path:line:text` for every non-comment line under $1 that matches the
# extended regular expression $2.
scan() {
    grep -rnE --include='*.rs' -- "$2" "$1" 2>/dev/null \
        | grep -vE '^[^:]+:[0-9]+:[[:space:]]*//' \
        || true
}

if [ ! -d "$FFI_SRC" ]; then
    fail "no unsafe in aralo-ffi: directory $FFI_SRC not found"
else
    # UniFFI's scaffolding is macro-expanded, so it never appears in source.
    hits="$(scan "$FFI_SRC" '(^|[^A-Za-z0-9_])unsafe([^A-Za-z0-9_]|$)')"
    if [ -n "$hits" ]; then
        fail "no unsafe in aralo-ffi: hand-written \`unsafe\` found. UniFFI generates the only unsafe code this crate may contain:"
        echo "$hits" | sed 's/^/        /' >&2
    else
        pass "no unsafe in aralo-ffi: $FFI_SRC has no hand-written \`unsafe\`"
    fi
fi

if [ ! -d "$TEMPLATE_SRC" ]; then
    fail "timeless template: directory $TEMPLATE_SRC not found"
else
    # The one clock in Aralo is `aralo_core::clock`. This crate formats a
    # moment it is handed, which is what lets a test fix the time and the
    # golden files compare byte for byte (ADR-0014).
    hits="$(scan "$TEMPLATE_SRC" '(SystemTime|Instant|Local|Utc|OffsetDateTime)::now|now_local|std::time|chrono|time::OffsetDateTime')"
    if [ -n "$hits" ]; then
        fail "timeless template: a clock is read in aralo-template. Everything a placeholder needs arrives as an argument (ADR-0014):"
        echo "$hits" | sed 's/^/        /' >&2
    else
        pass "timeless template: $TEMPLATE_SRC reads no clock"
    fi
fi

if [ ! -d "$ENGINE_SRC" ]; then
    fail "silent engine: directory $ENGINE_SRC not found"
else
    hits="$(scan "$ENGINE_SRC" '(^|[^A-Za-z0-9_])(e?print(ln)?!|dbg!|(log|tracing)::)')"
    if [ -n "$hits" ]; then
        fail "silent engine: printing or logging found in aralo-engine. The crate that sees keystrokes never prints or logs (PRD P1):"
        echo "$hits" | sed 's/^/        /' >&2
    else
        pass "silent engine: $ENGINE_SRC has no println!, eprintln!, print!, eprint!, dbg!, log:: or tracing::"
    fi
fi

# --- f: one HTTP door ------------------------------------------------------------
# cargo-deny keeps `reqwest` out of every crate but aralo-ai, and clippy bans
# building a client. This keeps it out of the rest of aralo-ai: a module that
# can name `reqwest` can hold a client, which is what the guard exists to stop.

if [ ! -d "$GUARD_SRC" ]; then
    fail "one HTTP door: directory $GUARD_SRC not found"
else
    hits="$(scan crates '(^|[^A-Za-z0-9_])reqwest::' | grep -v "^$GUARD_SRC/" || true)"
    if [ -n "$hits" ]; then
        fail "one HTTP door: \`reqwest\` is named outside the network guard. Adapters get a Transport from the gateway (ADR-0007):"
        echo "$hits" | sed 's/^/        /' >&2
    else
        pass "one HTTP door: \`reqwest\` is named only in $GUARD_SRC"
    fi
fi

# -----------------------------------------------------------------------------

if [ "$failures" -ne 0 ]; then
    echo >&2
    echo "check-deps: $failures problem(s) found" >&2
    exit 1
fi
echo "check-deps: all checks passed"

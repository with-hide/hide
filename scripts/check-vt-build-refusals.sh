#!/usr/bin/env bash
# The vt lane's proof that hide-vt's build stops with its reason, rather than
# build from something else, when its inputs are wrong: no Zig, another Zig
# version, and a download whose digest differs from its pin. Each case runs
# the real build or the real preparation and reads the reason it printed.
# docs/BUILD.md, "The terminal parser and Zig".
#
# Usage: check-vt-build-refusals.sh   (after the inputs are prepared)
set -euo pipefail

cd "$(dirname "$0")/.."

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
# The path a native Windows program reads; Git Bash's own spelling elsewhere.
native() { if command -v cygpath >/dev/null; then cygpath -w "$1"; else printf '%s' "$1"; fi; }

expect_refusal() {
    local name=$1 reason=$2
    shift 2
    if "$@" >"$work/out" 2>&1; then
        printf '%s: the command succeeded, but it should have refused\n' "$name" >&2
        exit 1
    fi
    if ! grep -F -- "$reason" "$work/out" >/dev/null; then
        printf '%s: refused without saying "%s"; it printed:\n' "$name" "$reason" >&2
        tail -20 "$work/out" >&2
        exit 1
    fi
    printf 'refused as expected: %s\n' "$name"
}

# A Zig that is not there. ZIG names the compiler for this build only, and
# its change reruns hide-vt's build script.
expect_refusal "no Zig" "Zig was not found at" \
    env ZIG="$work/no-such-zig" bash scripts/verify-cargo.sh build -p hide-vt

# A Zig of another version.
case "$(uname -s)" in
    MINGW*|MSYS*|CYGWIN*)
        stand_in="$work/old-zig.cmd"
        printf '@echo 0.15.2\r\n' >"$stand_in"
        stand_in=$(native "$stand_in")
        ;;
    *)
        stand_in="$work/old-zig"
        printf '#!/bin/sh\necho 0.15.2\n' >"$stand_in"
        chmod +x "$stand_in"
        ;;
esac
expect_refusal "another Zig version" "hide-vt needs Zig 0.16.0, but" \
    env ZIG="$stand_in" bash scripts/verify-cargo.sh build -p hide-vt

# A download whose digest differs from its pin: the pins, with this system's
# Zig archive replaced by a local file of other bytes.
printf 'not the pinned archive\n' >"$work/zig.tar.xz"
python3 - "$(native "$work")" <<'EOF'
import json, pathlib, sys
work = pathlib.Path(sys.argv[1])
pins = json.loads(pathlib.Path("hide-vt/build-inputs.json").read_text())
for archive in pins["zig"]["archives"].values():
    archive["url"] = (work / "zig.tar.xz").as_uri()
(work / "build-inputs.json").write_text(json.dumps(pins))
EOF
expect_refusal "a digest that differs" "but hide-vt/build-inputs.json pins" \
    env HIDE_BUILD_CACHE="$(native "$work/cache")" python3 scripts/prepare-vt-inputs.py --manifest "$(native "$work/build-inputs.json")"

# The real inputs still build.
bash scripts/verify-cargo.sh build -p hide-vt >/dev/null 2>&1
printf 'the pinned inputs build again\n'

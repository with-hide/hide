#!/usr/bin/env bash
# What a test marked `#[ignore]` needs before it can run: the built pane holder,
# the built `hide`, and the shells Hide starts, which those tests name in `HIDE_TEST_*` variables.
# docs/TESTING.md, "The repeat lane", owns when it is called; this file owns
# what it puts in place, so `pr.yml` and `repeat.yml` cannot drift apart.
#
# Source it, do not execute it, because it exports what it prepares:
#
#     . scripts/prepare-ignored-tests.sh
#     bash scripts/verify-cargo.sh nextest --profile ci -p hide-core --lib \
#       -E 'test(/^engine::contract::/)' --run-ignored only
#
# It is called only where the tests about to run are ignored ones, so an
# ordinary run pays nothing for the holder's build (the `checks` job and each
# repeat leg run it for their own selection).
#
# The shells are set only where the system has them: a runner installs zsh,
# fish, tcsh and bsd-csh itself (`.github/actions/apt-install`), and a test
# that needs one it cannot find fails naming the variable it read.
set -euo pipefail

_repository=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
cd "$_repository"

# The holder a test starts its panes from, under the name the tests read.
bash scripts/verify-cargo.sh build -p hide-holder --bin hide-holder
case "$(uname -s)" in
    MINGW*|MSYS*|CYGWIN*) export HIDE_TEST_HOLDER_BIN="$_repository/target/debug/hide-holder.exe" ;;
    *) export HIDE_TEST_HOLDER_BIN="$_repository/target/debug/hide-holder" ;;
esac

# The `hide` a start with a first input runs on the pane's machine (`hide
# agent exec --prompt-file`), which an engine test's node names beside it.
bash scripts/verify-cargo.sh build -p hided --bin hide
case "$(uname -s)" in
    MINGW*|MSYS*|CYGWIN*) export HIDE_TEST_HIDE_BIN="$_repository/target/debug/hide.exe" ;;
    *) export HIDE_TEST_HIDE_BIN="$_repository/target/debug/hide" ;;
esac

# Windows' pinned ConPTY runtime, which the holder linked there runs on
# (contracts/conpty.json, `docs/BUILD.md`).
case "$(uname -s)" in
    MINGW*|MSYS*|CYGWIN*)
        export HIDE_HOLDER_CONPTY="${RUNNER_TEMP:-$(mktemp -d)}/holder-conpty"
        python scripts/fetch-conpty.py --out "$HIDE_HOLDER_CONPTY"
        ;;
esac

# The login shells `hide-node`'s bootstrap test and `hide-host`'s shell
# integration tests start, each set only where the system has it: a variable
# naming no path would be worse than an unset one, which the test reports.
for _shell in ZSH:zsh FISH:fish BASH:bash TCSH:tcsh CSH:bsd-csh; do
    _name="HIDE_TEST_${_shell%%:*}_BIN"
    _program="${_shell#*:}"
    if [ -z "${!_name:-}" ]; then
        _found=$(command -v "$_program" || true)
        if [ -n "$_found" ]; then export "$_name=$_found"; else unset "$_name"; fi
    fi
done
unset _shell _name _program _found

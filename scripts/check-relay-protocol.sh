#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/toolchain-env.sh
bash scripts/verify-cargo.sh test-scoped -p hide-relay-protocol
bash scripts/build-relay-wasm.sh
exe_suffix=""
case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) exe_suffix=".exe" ;; esac
export CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER="$PWD/target/relay-tools/bin/wasm-bindgen-test-runner$exe_suffix"
bash scripts/verify-cargo.sh test-scoped -p hide-relay-protocol --target wasm32-unknown-unknown --lib --test it
bash scripts/verify-cargo.sh build -p hide-relay-protocol --example cross_peer
node --experimental-strip-types hide-relay-protocol/tests/cross-wasm.ts

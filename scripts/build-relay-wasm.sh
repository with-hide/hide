#!/usr/bin/env bash
# Build the browser ABI from the same locked Rust source as hided.
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/toolchain-env.sh
rustup target add wasm32-unknown-unknown
# The binding generator must exactly match the library's Cargo.lock version.
binding_version=$(python3 -c 'import tomllib; print(next(p["version"] for p in tomllib.load(open("Cargo.lock", "rb"))["package"] if p["name"] == "wasm-bindgen"))')
tool_root="$PWD/target/relay-tools"
exe_suffix=""
case "$(uname -s)" in MINGW*|MSYS*|CYGWIN*) exe_suffix=".exe" ;; esac
if [[ ! -x "$tool_root/bin/wasm-bindgen$exe_suffix" ]] || [[ "$("$tool_root/bin/wasm-bindgen$exe_suffix" --version)" != "wasm-bindgen $binding_version" ]]; then
    cargo install wasm-bindgen-cli --version "=$binding_version" --locked --root "$tool_root" --target-dir "$PWD/target/relay-tools-build"
fi
bash scripts/verify-cargo.sh build -p hide-relay-protocol --lib --release --target wasm32-unknown-unknown
"$tool_root/bin/wasm-bindgen$exe_suffix" target/wasm32-unknown-unknown/release/hide_relay_protocol.wasm \
    --target web --out-dir hide-relay-protocol/pkg --out-name relay
printf '{"type":"module","private":true}\n' > hide-relay-protocol/pkg/package.json

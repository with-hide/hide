#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
# Reuse the machine's installed toolchain; rustup installs a private copy into an
# empty $HOME/.rustup and still exits 0.
. scripts/toolchain-env.sh
run=agents/runs/herdr-typed-live-remote
mkdir -p "$run"
case "${1:-}" in
  behavior)
    cargo test --manifest-path herdr-core/Cargo.toml live::tests
    cargo test --manifest-path herdr-core/Cargo.toml remote::tests
    cargo test --manifest-path herdr-core/Cargo.toml wire::tests
    ;;
  probe)
    python3 scripts/probe-typed-live-remote.py --output "$run/probe"
    HERDR_TEST_TYPED_RESPONSES="$PWD/$run/probe" cargo test --manifest-path herdr-core/Cargo.toml isolated_live_remote_responses_decode -- --ignored
    ;;
  suites)
    zsh scripts/check-runtime-gates.sh
    bash scripts/check-harness-ignore-anchor.sh
    bash scripts/check-capability-readers-off-lock.sh
    bash scripts/check-no-workstation-identity.sh
    ;;
  attribution)
    git diff --check
    python3 scripts/check-no-attribution.py "${@:2}"
    ;;
  *) echo 'usage: check-typed-live-remote.sh behavior|probe|suites|attribution' >&2; exit 2 ;;
esac

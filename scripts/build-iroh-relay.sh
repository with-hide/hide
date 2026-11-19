#!/usr/bin/env bash
# Builds the device relay's iroh-relay server from its crates.io source at the
# version deploy/relay/iroh-relay.version pins, with the lock file that release
# published, into target/iroh-relay/bin/iroh-relay inside this checkout.
# relay-artifact.yml calls it; scripts/relay-deploy.py checks the same version
# against the artifact's receipt.
set -euo pipefail

cd "$(dirname "$0")/.."
. scripts/toolchain-env.sh

version=$(tr -d '[:space:]' < deploy/relay/iroh-relay.version)
cargo install --locked --force iroh-relay --version "=$version" --features server \
    --root target/iroh-relay --target-dir target/iroh-relay-build

#!/bin/bash
# The Mac mini CI runner's job-completed hook (docs/BUILD.md, The Mac mini CI
# runner). The runner starts it after the job's last step, whatever the job's
# result.
#
# The Mac mini shares its disk with live automation, so nothing a job checked
# out, built or downloaded outlives it: the work folder (the checkout and its
# target folder, the cargo registry and hide-vt's inputs, which the service
# points under it, setup-node's Node), and the job's own HOME and TMPDIR,
# which the service points inside the runner's folder. What stays is the Rust
# toolchain the source pins, in the runner's own rustup home.
set -euo pipefail
runner="${HIDE_CI_RUNNER:-}"
if [[ -z "$runner" ]]; then
  echo "::error::HIDE_CI_RUNNER does not name the runner's folder; nothing was removed"
  exit 1
fi
# Only the real folders: a link anywhere on the way would point the removal
# elsewhere.
real="$(cd -P -- "$runner" 2>/dev/null && pwd -P)" || real=""
if [[ -z "$real" || "$real" != "$runner" ]]; then
  echo "::error::the runner's folder $runner is not a real folder; nothing was removed"
  exit 1
fi

# The pin is read before the checkout that holds it goes.
channel=""
pin="${GITHUB_WORKSPACE:-}/rust-toolchain.toml"
if [[ -n "${GITHUB_WORKSPACE:-}" && -f "$pin" ]]; then
  channel="$(sed -n 's/^channel *= *"\(.*\)"$/\1/p' "$pin")"
fi
# The runner starts this hook in the job's checkout, which goes next, and
# rustup will not start in a folder that no longer exists.
cd /

left=""
for folder in _work home tmp; do
  [[ -e "$real/$folder" ]] || continue
  if [[ -L "$real/$folder" || ! -d "$real/$folder" ]]; then
    left="$left $real/$folder"
    continue
  fi
  # Every entry, whatever its name and whether or not it is a link; rm removes
  # a link itself and never follows one. _temp is the runner's, which it
  # empties itself at the start and end of a job.
  find "$real/$folder" -mindepth 1 -maxdepth 1 ! -path "$real/_work/_temp" -exec rm -rf {} +
  left="$left$(find "$real/$folder" -mindepth 1 -maxdepth 1 ! -path "$real/_work/_temp" | tr '\n' ' ')"
done
if [[ -n "${left// /}" ]]; then
  echo "::error::the job's files could not all be removed:$left"
  exit 1
fi

# Last, so a rustup failure never keeps the job's files.
if [[ -n "$channel" ]]; then
  rustup toolchain list | awk '$1 ~ /-/ { print $1 }' | while read -r toolchain; do
    if [[ "$toolchain" != "$channel"-* ]]; then rustup toolchain uninstall "$toolchain"; fi
  done
fi

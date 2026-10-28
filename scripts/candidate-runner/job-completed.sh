#!/bin/bash
# The release candidate runner's job-completed hook (docs/BUILD.md, The
# candidate's Mac build). The runner starts it after the job's last step,
# whatever the job's result.
#
# Nothing a job checked out, built or downloaded outlives it: the next
# candidate starts from an empty folder, never from an earlier release build,
# and the disk the Mac shares with other work gets the space back. The cargo
# registry and the pnpm store live under the work folder too (setup.sh points
# CARGO_HOME and the store there), so they go with it. What stays in the
# account's home is the Rust toolchain the source pins, one per pin.
set -u
work="$HOME/actions-runner/_work"
if [[ ! -d "$work" ]]; then
  echo "::error::the runner's work folder $work does not exist"
  exit 1
fi

# The pin is read before the checkout that holds it goes.
channel=""
pin="${GITHUB_WORKSPACE:-}/rust-toolchain.toml"
if [[ -n "${GITHUB_WORKSPACE:-}" && -f "$pin" ]]; then
  channel="$(sed -n 's/^channel *= *"\(.*\)"$/\1/p' "$pin")"
fi

# _temp is the runner's, which it empties itself at the start and end of a job.
for entry in "$work"/* "$work"/.[!.]*; do
  if [[ -e "$entry" && "$entry" != "$work/_temp" ]]; then rm -rf "$entry"; fi
done
left="$(find "$work" -mindepth 1 -maxdepth 1 ! -name _temp)"
if [[ -n "$left" ]]; then
  echo "::error::the job's files could not all be removed: $left"
  exit 1
fi

# Last, so a rustup failure, which the runner's `bash -e -o pipefail` turns
# into this hook's, never keeps the job's files.
if [[ -n "$channel" ]]; then
  rustup toolchain list | awk '$1 ~ /-/ { print $1 }' | while read -r toolchain; do
    if [[ "$toolchain" != "$channel"-* ]]; then rustup toolchain uninstall "$toolchain"; fi
  done
fi

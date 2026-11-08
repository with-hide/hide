#!/usr/bin/env bash
# Downloads URL to OUT for the scripts that fetch a pinned release on a CI
# runner (install-nextest.sh, install-hakari.sh, fetch-gh.sh). One attempt is
# cut at 40 seconds and a failed one is tried again up to three times, so a
# stalled or cut-off connection ends inside the 3-minute step limit those
# installs run under (.github/workflows/pr.yml) instead of hanging until the
# limit, and a transient failure retries instead of failing the lane. Piping
# the download into `tar` hid a cut-off transfer as `tar: Error is not
# recoverable`; the callers extract only a complete file.
#
# Usage: scripts/download.sh URL OUT
set -euo pipefail

url=${1:?usage: download.sh URL OUT}
out=${2:?usage: download.sh URL OUT}
curl --location --silent --show-error --fail \
    --retry 3 --retry-all-errors --retry-delay 2 \
    --connect-timeout 10 --max-time 40 \
    --output "$out" "$url"

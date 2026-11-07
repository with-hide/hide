#!/usr/bin/env bash
# Install apt packages on a CI runner without letting a slow mirror spend the
# job's time: each attempt is cut off after APT_ATTEMPT_SECONDS and a failed or
# cut-off attempt is tried again (APT_ATTEMPTS in all) after a refresh of the
# package lists, so a mirror that stalls once costs a minute, not the job.
# Exits non-zero, naming the packages, when every attempt failed.
set -euo pipefail

(( $# )) || { echo 'usage: ci-apt-install.sh <package>...' >&2; exit 2; }

attempts=${APT_ATTEMPTS:-4}
seconds=${APT_ATTEMPT_SECONDS:-120}

for (( n = 1; n <= attempts; n++ )); do
  if timeout "$seconds" sudo apt-get install -y --no-install-recommends \
       -o Acquire::Retries=3 -o Acquire::http::Timeout=20 -o Acquire::https::Timeout=20 "$@"; then
    exit 0
  fi
  echo "apt install of $* failed or timed out (attempt $n of $attempts)" >&2
  (( n < attempts )) || break
  sleep $(( n * 5 ))
  timeout "$seconds" sudo apt-get update -qq || true
done
echo "apt install of $* failed after $attempts attempts" >&2
exit 1

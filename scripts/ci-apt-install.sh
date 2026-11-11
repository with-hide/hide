#!/usr/bin/env bash
# Install apt packages on a CI runner without letting a slow Ubuntu mirror
# spend the job's time (with-hide/hide-private#629).
#
# One cap: APT_BUDGET_SECONDS (default 300) is the whole time this script may
# spend on the network. Every wait on the mirror below is cut to what is left
# of it, so a stalled mirror ends in a failure that names the packages, never
# in the runner killing the step part way.
#
# Two phases, because only the first one is slow and only the second one is
# unsafe to interrupt:
#   1. Download (`apt-get install --download-only`), tried again until the
#      budget is gone. A failed or cut-off try refreshes the package lists and
#      goes again; apt continues a partial file, so each try converges on the
#      same set. Each try gets half of what is left, so a mirror that trickles
#      one connection cannot take the whole budget in the first try.
#   2. Install, once, from what phase 1 fetched. It asks the network for
#      nothing (--no-download), so a cut-off can never land inside dpkg.
#
# APT_CACHE_DIR, when set, is the folder the .deb files are kept in
# (.github/actions/apt-install saves it with actions/cache); a hit leaves
# phase 1 nothing to download, so a usual run does not reach the mirror.
#
# Arguments are repository package names, optionally with a pinned version.
# Exits 2 on a bad call, 1 when DNS, fetching or installation fails.
set -euo pipefail

(( $# )) || { echo 'usage: APT_BUDGET_SECONDS=<n> [APT_CACHE_DIR=<dir>] ci-apt-install.sh <package>...' >&2; exit 2; }
for package; do
  [[ $package != -* && $package != *.deb ]] || { echo 'ci-apt-install.sh accepts repository package names, not options or local deb files' >&2; exit 2; }
done

budget=${APT_BUDGET_SECONDS:-300}
pause=${APT_RETRY_PAUSE_SECONDS:-5}
cache=${APT_CACHE_DIR:-}
[[ $budget =~ ^[0-9]+$ ]] && (( budget > 0 )) || { echo "APT_BUDGET_SECONDS must be a positive whole number, not '$budget'" >&2; exit 2; }
packages="$*"

root=()
(( EUID == 0 )) || root=(sudo)
apt=("${root[@]}" apt-get -o Acquire::Retries=3 -o Acquire::http::Timeout=20 -o Acquire::https::Timeout=20)
if [[ -n $cache ]]; then
  mkdir -p "$cache"
  cache=$(cd "$cache" && pwd)
  apt+=(-o "Dir::Cache::archives=$cache")
  # apt runs as root. Hand the folder back to the runner user on every exit so
  # actions/cache can read it, and drop what is not a package.
  trap '"${root[@]}" rm -rf "$cache/partial" "$cache/lock"; "${root[@]}" chown -R "$(id -u):$(id -g)" "$cache"' EXIT
fi

left() { echo $(( budget - SECONDS )); }
# What one wait may take: half of what is left, all of it when little is.
slice() { local l; l=$(left); (( l > 0 )) || l=1; echo $(( l > 60 ? (l + 1) / 2 : l )); }
bounded() { local limit=$1; shift; (( limit > 0 )) || return 124; timeout --kill-after=10 "$limit" "$@"; }

# DNS shares the same whole network budget, before any package is fetched.
if ! bounded "$(left)" bash "$(dirname "${BASH_SOURCE[0]}")/ci-apt-dns.sh"; then
  echo "could not prepare Ubuntu package DNS within $budget s" >&2
  exit 1
fi

fetched=false
attempt=0
while (( $(left) > 0 )); do
  attempt=$(( attempt + 1 ))
  if bounded "$(slice)" "${apt[@]}" install --download-only -y --no-install-recommends "$@"; then
    fetched=true
    break
  fi
  echo "fetching $packages failed or was cut off (try $attempt, $(left) s of $budget s left)" >&2
  (( $(left) > pause )) || break
  sleep "$pause"
  # A list the mirror has moved past makes every try fail the same way.
  bounded "$(slice)" "${apt[@]}" update -qq || echo 'refreshing the package lists failed; trying again with the old ones' >&2
done

if [[ $fetched != true ]]; then
  echo "could not fetch $packages within $budget s ($attempt tries)" >&2
  exit 1
fi

# Offline, so dpkg's own pace is the only wait; it is not cut short.
"${apt[@]}" install -y --no-install-recommends --no-download "$@"
echo "installed $packages in $SECONDS s (budget $budget s, $attempt fetch tries)"

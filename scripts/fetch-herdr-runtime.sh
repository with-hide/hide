#!/bin/zsh
# Produces the pinned Herdr binary for an app build and prints its path.
#
# Usage: fetch-herdr-runtime.sh [--platform PLATFORM]
#
# The asset is kept in a cache keyed by its digest, so the release build, the
# development build and the bump script all read the same verified file and a
# second build on the same machine downloads nothing. A cached file whose
# digest no longer matches the pin is discarded rather than trusted.
#
# Foreign device assets are digest-verified without trying to execute them.
set -euo pipefail

export LC_ALL=en_US.UTF-8
export LC_CTYPE=en_US.UTF-8
export LANG=en_US.UTF-8

script_dir=${0:A:h}
project_root=${script_dir:h}
manifest=$project_root/contracts/herdr-bundle.json
[[ -f "$manifest" ]] || {
  print -u2 -- "error: pinned Herdr runtime manifest is missing: $manifest"
  exit 1
}

platform=macos-aarch64
while (( $# > 0 )); do
  case "$1" in
    --platform)
      (( $# >= 2 )) || { print -u2 -- "usage: $0 [--platform PLATFORM]"; exit 2; }
      platform=$2
      shift 2
      ;;
    *)
      print -u2 -- "usage: $0 [--platform PLATFORM]"
      exit 2
      ;;
  esac
done
case "$platform" in
  macos-aarch64) pin_path='' ;;
  linux-x86_64) pin_path='.linux_x86_64' ;;
  macos-x86_64) pin_path='.macos_x86_64' ;;
  linux-aarch64) pin_path='.linux_aarch64' ;;
  *)
    print -u2 -- "error: unknown Herdr platform: $platform"
    exit 2
    ;;
esac

version=$(jq -er '.version' "$manifest")
repo=$(jq -er '.repo | strings | select(test("^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$"))' "$manifest")
tag=$(jq -er '.tag | strings | select(test("^[A-Za-z0-9][A-Za-z0-9._-]*$"))' "$manifest")
source_url="https://github.com/${repo}/releases/download/${tag}/herdr-${platform}"
recorded_url=$(jq -er "${pin_path}.source_url" "$manifest")
[[ "$source_url" == "$recorded_url" ]] || {
  print -u2 -- "error: manifest source_url for $platform disagrees with repo and tag"
  exit 1
}
sha256=$(jq -er "${pin_path}.sha256" "$manifest")

if [[ "$OSTYPE" == darwin* ]]; then
  default_cache=${XDG_CACHE_HOME:-$HOME/Library/Caches}
else
  default_cache=${XDG_CACHE_HOME:-$HOME/.cache}
fi
cache_root=${HIDE_HERDR_CACHE:-$default_cache/hide/herdr-runtime}
cached=$cache_root/$sha256/herdr

digest_of() {
  /usr/bin/shasum -a 256 "$1" | /usr/bin/awk '{print $1}'
}

if [[ -f "$cached" ]] && [[ "$(digest_of "$cached")" == "$sha256" ]]; then
  print -r -- "$cached"
  exit 0
fi

temporary_root=$(mktemp -d "${TMPDIR:-/tmp}/herdr-fetch.XXXXXX")
cleanup() {
  rm -rf -- "$temporary_root"
}
trap cleanup EXIT
downloaded=$temporary_root/herdr

/usr/bin/curl --fail --location --silent --show-error "$source_url" --output "$downloaded" || {
  print -u2 -- "error: pinned Herdr asset could not be downloaded: $source_url"
  exit 1
}
chmod 755 "$downloaded"

actual_sha256=$(digest_of "$downloaded")
[[ "$actual_sha256" == "$sha256" ]] || {
  print -u2 -- "error: downloaded Herdr asset digest does not match the pin"
  print -u2 -- "pinned_sha256=$sha256 downloaded_sha256=$actual_sha256 url=$source_url"
  exit 1
}
host_os=$(uname -s)
host_arch=$(uname -m)
[[ "$host_os" != Darwin ]] || host_os=macos
[[ "$host_os" != Linux ]] || host_os=linux
[[ "$host_arch" != arm64 ]] || host_arch=aarch64
if [[ "$platform" == "$host_os-$host_arch" ]]; then
  actual_version=$("$downloaded" --version | /usr/bin/awk '{print $NF}')
  [[ "$actual_version" == "$version" ]] || {
    print -u2 -- "error: downloaded Herdr asset reports $actual_version, not the pinned $version"
    exit 1
  }
fi

mkdir -p "${cached:h}"
install -m 755 "$downloaded" "$cached"
print -r -- "$cached"

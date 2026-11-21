#!/usr/bin/env bash
# Downloads the oldest gh release Hide supports into OUT_DIR, checks it
# against its pinned digest, and prints the path of its `gh`. The rust lane
# runs every command line the node sends with it
# (`hide-host/src/gh.rs`, `every_command_line_parses_on_the_oldest_gh_hide_supports`).
# The version is `MINIMUM_VERSION` in `hide-node-link/src/gh.rs`; that test
# refuses a download of any other release, so the two move together.
#
# Usage: scripts/fetch-gh.sh OUT_DIR
set -euo pipefail

version=2.72.0
out=${1:?usage: fetch-gh.sh OUT_DIR}
case "$(uname -s)/$(uname -m)" in
    Linux/x86_64) asset=linux_amd64.tar.gz digest=ffd3a9791075cf6119531302b06554e658f38ed12675a7fbfbf4d6b114c77f38 ;;
    Linux/aarch64|Linux/arm64) asset=linux_arm64.tar.gz digest=30d3b41200b7a195c340d0f121bd214450a10770e4e5164fbf76540a8a68e384 ;;
    Darwin/arm64) asset=macOS_arm64.zip digest=adf08dc15504ef74fbd37bc1a9abf2ef2b19d5e0db332a39877532b350b1f1d7 ;;
    Darwin/x86_64) asset=macOS_amd64.zip digest=46a85db5ad8141dade7476c0d44ed0b1c7ac86efb5f9fe8a7ad2002a03a2853f ;;
    *) printf 'no pinned gh %s for %s/%s\n' "$version" "$(uname -s)" "$(uname -m)" >&2; exit 1 ;;
esac

name=gh_${version}_${asset}
mkdir -p "$out"
archive=$out/$name
bash "$(dirname "${BASH_SOURCE[0]}")/download.sh" "https://github.com/cli/cli/releases/download/v$version/$name" "$archive"
if command -v sha256sum >/dev/null; then
    actual=$(sha256sum "$archive" | cut -d' ' -f1)
else
    actual=$(shasum -a 256 "$archive" | cut -d' ' -f1)
fi
[[ "$actual" == "$digest" ]] || { printf '%s has digest %s, not %s\n' "$name" "$actual" "$digest" >&2; exit 1; }
case "$asset" in
    *.tar.gz) tar -xzf "$archive" -C "$out" ;;
    *.zip) unzip -q -o "$archive" -d "$out" ;;
esac
rm "$archive"
folder=${name%.tar.gz}
printf '%s\n' "$out/${folder%.zip}/bin/gh"

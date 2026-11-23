#!/usr/bin/env bash
# Downloads the oldest glab release Hide supports into OUT_DIR, checks it
# against its pinned digest, and prints the path of its `glab`. The rust lane
# runs every command line the node sends with it
# (`hide-host/src/glab.rs`, `every_command_line_parses_on_the_oldest_glab_hide_supports`).
# The version is `MINIMUM_VERSION` in `hide-node-link/src/glab.rs`; that test
# refuses a download of any other release, so the two move together.
#
# Usage: scripts/fetch-glab.sh OUT_DIR
set -euo pipefail

version=1.78.0
out=${1:?usage: fetch-glab.sh OUT_DIR}
case "$(uname -s)/$(uname -m)" in
    Linux/x86_64) asset=linux_amd64.tar.gz digest=e64fdd53f8de8dddfc13872db41f71d7f5655570ef3322c414f220a97631d9b3 ;;
    Linux/aarch64|Linux/arm64) asset=linux_arm64.tar.gz digest=936fbdb7b45aa7ea41f875fff7654e5ef86185906efa346db7d553cff0752d1c ;;
    Darwin/arm64) asset=darwin_arm64.tar.gz digest=20be036bed1154d6bc7fb95db4b810db3d24ee26564b6debcbef693eec24f4da ;;
    Darwin/x86_64) asset=darwin_amd64.tar.gz digest=66584dc3e7a154a24f62bbadee8758bbbafdfcb881dbcbabe05efc6ac89c9d98 ;;
    *) printf 'no pinned glab %s for %s/%s\n' "$version" "$(uname -s)" "$(uname -m)" >&2; exit 1 ;;
esac

name=glab_${version}_${asset}
# GitLab's own release assets live in the project's generic package registry,
# with the dots of the version and the name escaped.
escaped_version=${version//./%2E}
escaped_name=${name//./%2E}
mkdir -p "$out/glab-$version"
archive=$out/$name
bash "$(dirname "${BASH_SOURCE[0]}")/download.sh" "https://gitlab.com/api/v4/projects/gitlab-org%2Fcli/packages/generic/glab/$escaped_version/$escaped_name" "$archive"
if command -v sha256sum >/dev/null; then
    actual=$(sha256sum "$archive" | cut -d' ' -f1)
else
    actual=$(shasum -a 256 "$archive" | cut -d' ' -f1)
fi
[[ "$actual" == "$digest" ]] || { printf '%s has digest %s, not %s\n' "$name" "$actual" "$digest" >&2; exit 1; }
tar -xzf "$archive" -C "$out/glab-$version"
rm "$archive"
printf '%s\n' "$out/glab-$version/bin/glab"

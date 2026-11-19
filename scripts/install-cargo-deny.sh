#!/usr/bin/env bash
# Installs the pinned cargo-deny release binary into $CARGO_HOME/bin, with no
# compile, for `scripts/verify-cargo.sh deny` (docs/BUILD.md, "Licenses and
# advisories"). The version and each archive's digest live here only, so a
# bump is these lines.
set -euo pipefail

version=0.21.0
case "$(uname -s)-$(uname -m)" in
    Linux-x86_64)
        asset=x86_64-unknown-linux-musl
        digest=a2170b854ff697e79e39c4c538981c9c1bdd803addbae6719a4cb66b4585a903 ;;
    Darwin-arm64)
        asset=aarch64-apple-darwin
        digest=c99a789160775ada8cb54aed72cfdae5a9b72e27434f8868d17c9634c3f52425 ;;
    *) printf 'no pinned cargo-deny release for %s\n' "$(uname -s)-$(uname -m)" >&2; exit 1 ;;
esac

bin=${CARGO_HOME:-$HOME/.cargo}/bin
mkdir -p "$bin"
archive=$(mktemp)
trap 'rm -f "$archive"' EXIT
bash "$(dirname "${BASH_SOURCE[0]}")/download.sh" \
    "https://github.com/EmbarkStudios/cargo-deny/releases/download/$version/cargo-deny-$version-$asset.tar.gz" "$archive"
printf '%s  %s\n' "$digest" "$archive" | shasum -a 256 -c - >/dev/null || {
    printf 'cargo-deny %s archive does not match its pinned digest\n' "$version" >&2
    exit 1
}
tar zxf "$archive" -C "$bin" --strip-components 1 "cargo-deny-$version-$asset/cargo-deny"
"$bin/cargo-deny" --version | grep -F -- "$version" >/dev/null || {
    printf 'cargo-deny is not %s\n' "$version" >&2
    exit 1
}

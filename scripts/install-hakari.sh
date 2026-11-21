#!/usr/bin/env bash
# Installs the pinned cargo-hakari release binary into $CARGO_HOME/bin, with
# no compile, for `scripts/verify-cargo.sh hakari` (docs/BUILD.md, "One
# feature set per dependency"). The version and each archive's digest live
# here only, so a bump is these lines.
set -euo pipefail

version=0.9.39
case "$(uname -s)-$(uname -m)" in
    Linux-x86_64)
        asset=x86_64-unknown-linux-gnu
        digest=699a3193a9bd1a8d914b970d3f50f3385f24fbd8b1294d0c0ff31fbe7c2c9f48 ;;
    Darwin-*)
        asset=universal-apple-darwin
        digest=c1f84026b805cc7fb31c271e8b0720abadb8d9416d126c76d6fc50a2a147d7f9 ;;
    *) printf 'no pinned cargo-hakari release for %s\n' "$(uname -s)-$(uname -m)" >&2; exit 1 ;;
esac

bin=${CARGO_HOME:-$HOME/.cargo}/bin
mkdir -p "$bin"
archive=$(mktemp)
trap 'rm -f "$archive"' EXIT
bash "$(dirname "${BASH_SOURCE[0]}")/download.sh" \
    "https://github.com/guppy-rs/guppy/releases/download/cargo-hakari-$version/cargo-hakari-$version-$asset.tar.gz" "$archive"
printf '%s  %s\n' "$digest" "$archive" | shasum -a 256 -c - >/dev/null || {
    printf 'cargo-hakari %s archive does not match its pinned digest\n' "$version" >&2
    exit 1
}
tar zxf "$archive" -C "$bin" cargo-hakari
"$bin/cargo-hakari" --version | grep -F -- "$version" >/dev/null || {
    printf 'cargo-hakari is not %s\n' "$version" >&2
    exit 1
}

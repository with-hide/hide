#!/usr/bin/env bash
# Builds the relay protocol for the iOS app: hide-relay-ios as a static library for a device and for the simulator,
# from the locked source hided uses, into mobile/ios/relay/ (ignored build output, sent to the Mac mini with the rest of
# mobile/ios/ by mobile/scripts/build-ios.sh, where Xcode wraps it in an XCFramework and no Rust is installed).
#
#   bash scripts/build-relay-ios.sh            build both libraries
#   bash scripts/build-relay-ios.sh --digest   print the source digest and build nothing
#
# The digest names what the library is made of: the git blob of every tracked file of hide-relay-protocol and
# hide-relay-ios plus the lock file and toolchain pin, read from the working files so an edit counts. It is compiled
# into the library (`hide_relay_source`) and written to mobile/ios/relay/source.json; the release check recomputes it
# from the checkout the release runs on and compares it with the one inside the signed app (docs/BUILD.md, The iOS app).
#
# Cargo runs through scripts/verify-cargo.sh like every other build here. Two settings are particular to iOS:
#  - hide-relay-protocol also declares a cdylib for the browser, and cargo builds every crate type of a path
#    dependency. Linking that dylib for an iOS target needs the full Xcode, which this machine may not have, and the
#    dylib is never used, so the link is replaced by a no-op linker. A static library is archived, not linked, so the
#    one artifact the app takes is unaffected.
#  - link-time optimization is off, because a static library of LLVM bitcode is read by Apple's linker only when it is
#    the same LLVM, and the profile's thin LTO would leave bitcode in the archive.
set -euo pipefail
cd "$(dirname "$0")/.."

digest() {
    local path text=""
    while IFS= read -r path; do
        text+="$(git hash-object -- "$path") $path"$'\n'
    done < <(git ls-files -- hide-relay-protocol hide-relay-ios workspace-hack Cargo.toml Cargo.lock rust-toolchain.toml | LC_ALL=C sort)
    printf '%s' "$text" | shasum -a 256 | cut -d' ' -f1
}

if [[ "${1:-}" == "--digest" ]]; then
    digest
    exit 0
fi

. scripts/toolchain-env.sh
rustup target add aarch64-apple-ios aarch64-apple-ios-sim >/dev/null

source_digest=$(digest)
out="$PWD/mobile/ios/relay"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
printf '#!/bin/sh\nexit 0\n' > "$work/no-link"
chmod +x "$work/no-link"

rm -rf "$out"
mkdir -p "$out"
for triple in aarch64-apple-ios aarch64-apple-ios-sim; do
    upper=$(printf '%s' "$triple" | tr 'a-z-' 'A-Z_')
    env "CARGO_TARGET_${upper}_LINKER=$work/no-link" \
        CARGO_PROFILE_RELEASE_LTO=off CARGO_PROFILE_RELEASE_STRIP=none \
        HIDE_BUILD_RELAY_IOS_SOURCE="$source_digest" IPHONEOS_DEPLOYMENT_TARGET=15.0 \
        bash scripts/verify-cargo.sh build -p hide-relay-ios --release --target "$triple"
    mkdir -p "$out/$triple"
    cp "target/$triple/release/libhide_relay_ios.a" "$out/$triple/"
done
mkdir -p "$out/include"
cp hide-relay-ios/include/hide_relay.h hide-relay-ios/include/module.modulemap "$out/include/"
printf '{"source":"%s","commit":"%s"}\n' "$source_digest" "$(git rev-parse HEAD)" > "$out/source.json"
printf 'relay library %s\n' "$source_digest" >&2

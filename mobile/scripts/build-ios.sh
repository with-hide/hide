#!/usr/bin/env bash
# Builds the Hide iOS app with the phone bundle inside it, on the Mac mini, and runs it in a simulator.
#
# Usage: build-ios.sh [build | run | clean]
#   build  (default) the phone bundle, `cap sync`, copy the project to the mini, build the app for the simulator
#          and run the HideShell tests there
#   run    install the built app in a simulator of its own on the mini, launch it, print the simulator's id
#          (HIDE_IOS_PAIR_LINK=<link> makes the debug build follow that pairing link at launch)
#   clean  delete that simulator and the build state on the mini (the folder itself stays)
#
# This Mac cannot run Xcode, the mini can. Everything the mini builds lives in ~/hide-ios-build/<branch>/ there,
# and the mini needs no node: the web bundle and `cap sync` happen here. See docs/BUILD.md, The iOS app.
#
# HIDE_IOS_BUILD_HOST     ssh name of the build machine (default: mini)
# HIDE_IOS_SKIP_PHONE_BUILD=1   reuse web/phone-dist instead of building it (it needs the Rust wasm toolchain)
set -euo pipefail

cd "$(dirname "$0")/../.."

command=${1:-build}
host=${HIDE_IOS_BUILD_HOST:-mini}
branch=$(git branch --show-current)
[[ -n "$branch" ]] || { printf 'build-ios.sh needs a checked-out branch\n' >&2; exit 2; }
slug=$(printf '%s' "$branch" | tr -c 'A-Za-z0-9._-' '-')
remote=hide-ios-build/$slug

case "$command" in
    build)
        if [[ "${HIDE_IOS_SKIP_PHONE_BUILD:-}" != 1 ]]; then
            bash scripts/verify-web.sh web build:phone
        fi
        node mobile/scripts/check-versions.mjs
        node mobile/scripts/assemble.mjs
        pnpm --dir mobile exec cap sync ios
        # Only the Xcode project goes over: the built web folder is already inside it (ios/App/App/public).
        ssh "$host" "mkdir -p '$remote'"
        rsync -a --delete \
            --exclude 'App/build' --exclude 'DerivedData' --exclude 'xcuserdata' --exclude '.build' \
            mobile/ios/ "$host:$remote/ios/"
        ;;
    run)
        # Debug builds only: a pairing link the simulator follows at launch (see ShellViewController).
        if [[ -n "${HIDE_IOS_PAIR_LINK:-}" ]]; then
            printf '%s' "$HIDE_IOS_PAIR_LINK" | ssh "$host" "umask 077; mkdir -p '$remote' && cat > '$remote/pair-link'"
        fi
        ;;
    clean) ;;
    *) printf 'usage: %s [build | run | clean]\n' "$0" >&2; exit 2 ;;
esac

ssh "$host" bash -s -- "$command" "$slug" < mobile/scripts/mini-ios.sh

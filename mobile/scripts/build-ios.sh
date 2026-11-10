#!/usr/bin/env bash
# Builds the Hide iOS app with the phone bundle inside it, on the Mac mini, and runs it in a simulator.
#
# Usage: build-ios.sh [build | run | clean]
#   build  (default) the phone bundle, `cap sync`, copy the project to the mini, build the app for the simulator
#          and run the HideShell tests there; a failing test fails the command
#   run    install the built app in a simulator of its own on the mini, launch it, wait, capture the screen
#          (HIDE_IOS_PAIR_LINK=<link> makes the debug build follow that pairing link at launch;
#           HIDE_IOS_CAPTURE=<file.png> copies the capture back here; HIDE_IOS_SETTLE=<seconds>, default 15)
#   clean  delete that simulator and this branch's whole folder on the mini
#
# Every command ends what it started on the mini: its simulator is shut down and deleted and the pairing link file is
# removed, on any way out. HIDE_IOS_KEEP_SIMULATOR=1 keeps the simulator for a check by hand.
# A branch's state is one folder, about 0.5 GB, ~/hide-ios-build/<branch>/; past 4 other branches' folders the build
# refuses and says which to remove.
#
# This Mac cannot run Xcode, the mini can. The mini needs no node: the web bundle and `cap sync` happen here.
# See docs/BUILD.md, The iOS app.
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
        ssh "$host" bash -s -- guard "$slug" < mobile/scripts/mini-ios.sh
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

ssh "$host" env HIDE_IOS_KEEP_SIMULATOR="${HIDE_IOS_KEEP_SIMULATOR:-}" HIDE_IOS_SETTLE="${HIDE_IOS_SETTLE:-15}" \
    bash -s -- "$command" "$slug" < mobile/scripts/mini-ios.sh
if [[ "$command" == run && -n "${HIDE_IOS_CAPTURE:-}" ]]; then
    scp -q "$host:$remote/shots/run.png" "$HIDE_IOS_CAPTURE"
fi

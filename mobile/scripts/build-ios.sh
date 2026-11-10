#!/usr/bin/env bash
# Builds the Hide iOS app with the phone bundle inside it, on the Mac mini, and runs it in a simulator.
#
# Usage: build-ios.sh [build | release | status <delivery id> | run | allow | push | clean]
#   release  everything `build` does, then on the mini: archive, sign with the App Store Connect API key, check the
#          signed app against mobile/release-expect.json, validate and upload it to TestFlight; the build number is the
#          UTC time of the run, the version the repository's own (mobile/scripts/release-version.mjs). The key never
#          leaves the mini (~/.config/hide-ios, mobile/release-env.json); a missing secret stops the command naming it
#   status <delivery id>  what Apple says about one upload (the id `release` printed); changes nothing
#   build  (default) the phone bundle, `cap sync`, copy the project to the mini, build the app for the simulator
#          and run the HideShell tests there; a failing test fails the command
#   run    install the built app in a simulator of its own on the mini, launch it, wait, capture the screen
#          (HIDE_IOS_PAIR_LINK=<link> makes the debug build follow that pairing link at launch;
#           HIDE_IOS_TAPS='<css selector>;;<css selector>' presses those on the page, in order, once each is there;
#           HIDE_IOS_APPEARANCE=light|dark sets the simulator's appearance; HIDE_IOS_HOME=1 captures the home screen with the
#           icon; HIDE_IOS_LAUNCH_SCREEN=1 holds the app on its launch screen and captures that;
#           HIDE_IOS_CAPTURE=<file.png> copies the capture back here; HIDE_IOS_SETTLE=<seconds>, default 15)
#   allow  press Allow on the system's notification question (HIDE_IOS_TAPS as for `run` makes the page ask it); the
#          grant stays in the simulator, so captures of delivered notifications follow without anyone at it
#   push   deliver the push body in the file HIDE_IOS_PUSH_BODY to the app, as Apple would, and capture the banner
#   clean  delete that simulator and this branch's whole folder on the mini
#
# Every command ends what it started on the mini: its simulator is shut down and deleted, on any way out (the pairing
# link is never a file there). HIDE_IOS_KEEP_SIMULATOR=1 keeps the simulator for a check by hand; the next command
# of the branch that does not ask for it ends that simulator too, so `clean` is not the only way it goes.
# A branch's state is one folder, about 0.5 GB, ~/hide-ios-build/<branch>/; past 4 other branches' folders the build
# refuses and says which to remove.
#
# This Mac cannot run Xcode, the mini can. The mini needs no node: the web bundle and `cap sync` happen here.
# See docs/BUILD.md, The iOS app.
#
# HIDE_IOS_BUILD_HOST     ssh name of the build machine (default: mini)
# HIDE_IOS_SKIP_PHONE_BUILD=1   reuse web/phone-dist instead of building it (it needs the Rust wasm toolchain)
# HIDE_IOS_SKIP_RELAY_BUILD=1   reuse mobile/ios/relay (the relay protocol library for the two iOS targets) instead of
#                        building it (scripts/build-relay-ios.sh; it needs the rustup targets on this Mac, never the mini)
set -euo pipefail

cd "$(dirname "$0")/../.."

command=${1:-build}
host=${HIDE_IOS_BUILD_HOST:-mini}
branch=$(git branch --show-current)
[[ -n "$branch" ]] || { printf 'build-ios.sh needs a checked-out branch\n' >&2; exit 2; }
slug=$(printf '%s' "$branch" | tr -c 'A-Za-z0-9._-' '-')
remote=hide-ios-build/$slug

case "$command" in
    build | release)
        # A release is the committed source: the build that reaches TestFlight must be one a person can find again.
        if [[ "$command" == release && -n "$(git status --porcelain --untracked-files=no)" ]]; then
            printf 'release needs a clean working tree: commit or stash the tracked changes first\n' >&2
            exit 1
        fi
        if [[ "${HIDE_IOS_SKIP_PHONE_BUILD:-}" != 1 ]]; then
            bash scripts/verify-web.sh web build:phone
        fi
        if [[ "${HIDE_IOS_SKIP_RELAY_BUILD:-}" != 1 ]]; then
            bash scripts/build-relay-ios.sh
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
    run | allow) ;;
    push) [[ -f "${HIDE_IOS_PUSH_BODY:-}" ]] || { printf 'push needs HIDE_IOS_PUSH_BODY=<file>\n' >&2; exit 2; } ;;
    clean) ;;
    status)
        [[ -n "${2:-}" ]] || { printf 'usage: %s status <delivery id>\n' "$0" >&2; exit 2; }
        exec ssh "$host" bash -s -- status "$slug" "$2" < mobile/scripts/mini-release.sh
        ;;
    *) printf 'usage: %s [build | release | status <delivery id> | run | allow | push | clean]\n' "$0" >&2; exit 2 ;;
esac

# Debug builds only: a pairing link the simulator follows at launch (see ShellViewController). It travels in front of
# the script on the same standard input, so it is in no argument and in no file, and there is nothing to leave behind.
{
    if [[ "$command" == run && -n "${HIDE_IOS_PAIR_LINK:-}" ]]; then printf 'HIDE_PAIR_LINK=%q\n' "$HIDE_IOS_PAIR_LINK"; fi
    if [[ "$command" == push ]]; then printf 'HIDE_PUSH_B64=%q\n' "$(base64 < "$HIDE_IOS_PUSH_BODY" | tr -d '\n')"; fi
    if [[ ( "$command" == run || "$command" == allow ) && -n "${HIDE_IOS_TAPS:-}" ]]; then printf 'HIDE_TAPS=%q\n' "$HIDE_IOS_TAPS"; fi
    cat mobile/scripts/mini-ios.sh
} | ssh "$host" env HIDE_IOS_KEEP_SIMULATOR="${HIDE_IOS_KEEP_SIMULATOR:-}" HIDE_IOS_SETTLE="${HIDE_IOS_SETTLE:-15}" \
    HIDE_IOS_APPEARANCE="${HIDE_IOS_APPEARANCE:-}" HIDE_IOS_HOME="${HIDE_IOS_HOME:-}" HIDE_IOS_LAUNCH_SCREEN="${HIDE_IOS_LAUNCH_SCREEN:-}" \
    bash -s -- "${command/release/build}" "$slug"
# `release` ran the same build and Swift tests just above: a build whose tests fail is never signed.
if [[ "$command" == release ]]; then
    inputs=$(mktemp -d)
    trap 'rm -rf "$inputs"' EXIT
    node mobile/scripts/bundle-manifest.mjs "$inputs/bundle-manifest.json"
    cp mobile/release-env.json mobile/release-expect.json mobile/scripts/check-release-app.sh "$inputs/"
    # The digest of the relay protocol sources of this checkout, taken now and not from the library's own folder: the
    # release check compares it with the one inside the signed app, so a library built from other sources is refused.
    bash scripts/build-relay-ios.sh --digest > "$inputs/relay-source.txt"
    rsync -a --delete "$inputs/" "$host:$remote/release-inputs/"
    # The time is taken here, after the tests, so the number is as close to the upload as the command can make it.
    read -r version build < <(node mobile/scripts/release-version.mjs)
    printf 'release %s (build %s) from %s\n' "$version" "$build" "$(git rev-parse --short=8 HEAD)"
    ssh "$host" bash -s -- release "$slug" "$version" "$build" < mobile/scripts/mini-release.sh
fi
if [[ ( "$command" == run || "$command" == push ) && -n "${HIDE_IOS_CAPTURE:-}" ]]; then
    scp -q "$host:$remote/shots/run.png" "$HIDE_IOS_CAPTURE"
fi

#!/usr/bin/env bash
# Builds the Hide iOS app with the phone bundle inside it, on the Mac mini, and runs it in a simulator.
#
# Usage: build-ios.sh [build | release | status <delivery id> | uploads | run | allow | push | store-shots | clean]
#   release  everything `build` does, then on the mini: archive, sign with the App Store Connect API key, check the
#          signed app against mobile/release-expect.json, validate and upload it to TestFlight; the build number is the
#          UTC time of the run, the version the repository's own (mobile/scripts/release-version.mjs). The key never
#          leaves the mini (~/.config/hide-ios, mobile/release-env.json); a missing secret stops the command naming it
#          A commit that already has an upload record is not uploaded again unless HIDE_IOS_RELEASE_AGAIN=1;
#          HIDE_IOS_NO_UPLOAD=1 stops after Apple's validation, before anything is sent; HIDE_IOS_SKIP_PHONE_BUILD is refused
#   status <delivery id>  what Apple says about one upload (the id `release` printed); changes nothing
#   uploads  the mini's upload record, every branch: when, which commit, version, build, delivery id and branch; read it
#          after a release whose connection dropped before it printed a delivery id
#   build  (default) the phone bundle, `cap sync`, copy the project to the mini, build the app for the simulator
#          and run the HideShell tests there; a failing test fails the command
#   run    install the built app in a simulator of its own on the mini, launch it, wait, capture the screen
#          (HIDE_IOS_PAIR_LINK=<link> makes the debug build follow that pairing link at launch;
#           HIDE_IOS_TAPS='<css selector>;;<css selector>' presses those on the page, in order, once each is there;
#           HIDE_IOS_APPEARANCE=light|dark sets the simulator's appearance; HIDE_IOS_HOME=1 captures the home screen with the
#           icon; HIDE_IOS_LAUNCH_SCREEN=1 holds the app on its launch screen and captures that;
#           HIDE_IOS_CAPTURE=<file.png> copies the capture back here; HIDE_IOS_RECORD=<file.mp4> records the launch;
#           HIDE_IOS_JOURNEY=notification|back runs one fixture UI test with HIDE_IOS_RELAY_PORT=<loopback port>;
#           HIDE_IOS_SETTLE=<seconds>, default 15)
#   allow  press Allow on the system's notification question (HIDE_IOS_TAPS as for `run` makes the page ask it); the
#          grant stays in the simulator, so captures of delivered notifications follow without anyone at it
#   push   deliver the push body in the file HIDE_IOS_PUSH_BODY to the app, as Apple would, and capture the banner
#   store-shots  the App Store screenshots in Korean and English: the same phone build and copy to the mini, then there the
#          app in a simulator of its own runs each shot of mobile/store/shots.json in demo mode (fixed status bar and clock,
#          the simulator in the shot's language), and the raw captures come back here to be framed with their headlines
#          by mobile/scripts/store-shots.mjs into 1206x2622 PNGs without alpha: HIDE_STORE_SHOTS_OUT=<folder>, default
#          agents/runs/ios-mission/store-assets/ (never committed). No tests run; HIDE_IOS_SKIP_PHONE_BUILD works as for `build`.
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
# HIDE_IOS_SKIP_PHONE_BUILD=1   (build only) reuse web/phone-dist instead of building it (it needs the Rust wasm toolchain)
set -euo pipefail

cd "$(dirname "$0")/../.."

command=${1:-build}
host=${HIDE_IOS_BUILD_HOST:-mini}
# Simulator QA only: forward a local fixture, never the production relay.
ssh_options=()
if [[ -n "${HIDE_IOS_RELAY_PORT:-}" ]]; then
    [[ "${1:-build}" != release ]] || { printf 'a simulator relay cannot be used for release\n' >&2; exit 2; }
    [[ "$HIDE_IOS_RELAY_PORT" =~ ^[1-9][0-9]{3,4}$ ]] && (( HIDE_IOS_RELAY_PORT > 1023 && HIDE_IOS_RELAY_PORT < 65536 )) || { printf 'invalid HIDE_IOS_RELAY_PORT\n' >&2; exit 2; }
    ssh_options=(-o ExitOnForwardFailure=yes -R "127.0.0.1:$HIDE_IOS_RELAY_PORT:127.0.0.1:$HIDE_IOS_RELAY_PORT")
fi
if [[ -n "${HIDE_IOS_JOURNEY:-}" ]]; then
    [[ "$command" == run && -n "${HIDE_IOS_RELAY_PORT:-}" ]] || { printf 'HIDE_IOS_JOURNEY requires run with a simulator relay\n' >&2; exit 2; }
    case "$HIDE_IOS_JOURNEY" in notification|back) ;; *) printf 'unknown HIDE_IOS_JOURNEY\n' >&2; exit 2 ;; esac
fi
branch=$(git branch --show-current)
[[ -n "$branch" ]] || { printf 'build-ios.sh needs a checked-out branch\n' >&2; exit 2; }
slug=$(printf '%s' "$branch" | tr -c 'A-Za-z0-9._-' '-')
remote=hide-ios-build/$slug

# The shots to take, read (and checked) before anything runs on the mini.
store_plan=
if [[ "$command" == store-shots ]]; then
    store_plan=$(node mobile/scripts/store-shots.mjs plan)
fi

case "$command" in
    build | release | store-shots)
        # A release is the committed source: the build that reaches TestFlight must be one a person can find again.
        if [[ "$command" == release && -n "$(git status --porcelain --untracked-files=no)" ]]; then
            printf 'release needs a clean working tree: commit or stash the tracked changes first\n' >&2
            exit 1
        fi
        if [[ "$command" == release && "${HIDE_IOS_SKIP_PHONE_BUILD:-}" == 1 ]]; then
            printf 'release builds the phone bundle from this commit; HIDE_IOS_SKIP_PHONE_BUILD is for build only\n' >&2
            exit 2
        fi
        # Refuse a commit that already went before the long build, not after it.
        if [[ "$command" == release && "${HIDE_IOS_RELEASE_AGAIN:-}" != 1 ]]; then
            if ssh "$host" bash -s -- uploads "$slug" < mobile/scripts/mini-release.sh | grep -qF " $(git rev-parse HEAD) "; then
                printf 'this commit already has an upload record; read it with `%s uploads`, and set HIDE_IOS_RELEASE_AGAIN=1 to upload it again on purpose\n' "$0" >&2
                exit 1
            fi
        fi
        if [[ "${HIDE_IOS_SKIP_PHONE_BUILD:-}" != 1 ]]; then
            if [[ -n "${HIDE_IOS_RELAY_PORT:-}" ]]; then
                VITE_HIDE_RELAY_URL="ws://127.0.0.1:$HIDE_IOS_RELAY_PORT" bash scripts/verify-web.sh web build:phone
            else
                bash scripts/verify-web.sh web build:phone
            fi
        fi
        node mobile/scripts/check-versions.mjs
        # A running release owns the machine's release folders: a build of its branch would copy a project under its
        # archive, and a second release waits for the first.
        ssh "$host" bash -s -- busy "$slug" "$([[ "$command" == release ]] && echo machine || echo branch)" < mobile/scripts/mini-release.sh
        ssh "$host" bash -s -- guard "$slug" < mobile/scripts/mini-ios.sh
        if [[ -n "${HIDE_IOS_RELAY_PORT:-}" ]]; then
            node mobile/scripts/assemble.mjs --simulator-relay-port "$HIDE_IOS_RELAY_PORT"
        else
            node mobile/scripts/assemble.mjs
        fi
        pnpm --dir mobile exec cap sync ios
        # Only the Xcode project goes over: the built web folder is already inside it (ios/App/App/public).
        ssh "$host" "mkdir -p '$remote'"
        rsync -a --delete \
            --exclude 'App/build' --exclude 'DerivedData' --exclude 'xcuserdata' --exclude '.build' \
            mobile/ios/ "$host:$remote/ios/"
        ;;
    run)
        if [[ -n "${HIDE_IOS_JOURNEY:-}" ]]; then
            rsync -a mobile/ios/App/AppUITests/ "$host:$remote/ios/App/AppUITests/"
        fi
        ;;
    allow) ;;
    push) [[ -f "${HIDE_IOS_PUSH_BODY:-}" ]] || { printf 'push needs HIDE_IOS_PUSH_BODY=<file>\n' >&2; exit 2; } ;;
    clean) ssh "$host" bash -s -- busy "$slug" branch < mobile/scripts/mini-release.sh ;;
    uploads)
        exec ssh "$host" bash -s -- uploads "$slug" < mobile/scripts/mini-release.sh
        ;;
    status)
        [[ -n "${2:-}" ]] || { printf 'usage: %s status <delivery id>\n' "$0" >&2; exit 2; }
        exec ssh "$host" bash -s -- status "$slug" "$2" < mobile/scripts/mini-release.sh
        ;;
    *) printf 'usage: %s [build | release | status <delivery id> | uploads | run | allow | push | store-shots | clean]\n' "$0" >&2; exit 2 ;;
esac

# Debug builds only: a pairing link the simulator follows at launch (see ShellViewController). It travels in front of
# the script on the same standard input, so it is in no argument and in no file, and there is nothing to leave behind.
remote_status=0
{
    if [[ "$command" == run && -n "${HIDE_IOS_PAIR_LINK:-}" ]]; then printf 'HIDE_PAIR_LINK=%q\n' "$HIDE_IOS_PAIR_LINK"; fi
    if [[ "$command" == push ]]; then printf 'HIDE_PUSH_B64=%q\n' "$(base64 < "$HIDE_IOS_PUSH_BODY" | tr -d '\n')"; fi
    if [[ ( "$command" == run || "$command" == allow ) && -n "${HIDE_IOS_TAPS:-}" ]]; then printf 'HIDE_TAPS=%q\n' "$HIDE_IOS_TAPS"; fi
    if [[ "$command" == store-shots ]]; then
        printf 'HIDE_STORE_PLAN=%q\n' "$store_plan"
        printf 'HIDE_STORE_CLOCK=%q\n' "$(node -e "process.stdout.write(JSON.parse(require('fs').readFileSync('mobile/store/shots.json','utf8')).clock)")"
        printf 'HIDE_STORE_APPEARANCE=%q\n' "$(node -e "process.stdout.write(JSON.parse(require('fs').readFileSync('mobile/store/shots.json','utf8')).appearance)")"
    fi
    cat mobile/scripts/mini-ios.sh
} | ssh "${ssh_options[@]}" "$host" env HIDE_IOS_KEEP_SIMULATOR="${HIDE_IOS_KEEP_SIMULATOR:-}" HIDE_IOS_SETTLE="${HIDE_IOS_SETTLE:-15}" \
    HIDE_IOS_RECORD="${HIDE_IOS_RECORD:+1}" HIDE_IOS_JOURNEY="${HIDE_IOS_JOURNEY:-}" \
    HIDE_IOS_RELAY_PORT="${HIDE_IOS_RELAY_PORT:-}" \
    HIDE_IOS_APPEARANCE="${HIDE_IOS_APPEARANCE:-}" HIDE_IOS_HOME="${HIDE_IOS_HOME:-}" HIDE_IOS_LAUNCH_SCREEN="${HIDE_IOS_LAUNCH_SCREEN:-}" \
    bash -s -- "${command/release/build}" "$slug" || remote_status=$?
if (( remote_status != 0 )); then
    if [[ "$command" == run && -n "${HIDE_IOS_RECORD:-}" ]]; then
        scp -q "$host:$remote/shots/launch.mp4" "$HIDE_IOS_RECORD" || true
    fi
    exit "$remote_status"
fi
# `release` ran the same build and Swift tests just above: a build whose tests fail is never signed.
if [[ "$command" == release ]]; then
    inputs=$(mktemp -d)
    trap 'rm -rf "$inputs"' EXIT
    node mobile/scripts/bundle-manifest.mjs "$inputs/bundle-manifest.json"
    cp mobile/release-env.json mobile/release-expect.json mobile/scripts/check-release-app.sh "$inputs/"
    rsync -a --delete "$inputs/" "$host:$remote/release-inputs/"
    # The time is taken here, after the tests, so the number is as close to the upload as the command can make it.
    read -r version build < <(node mobile/scripts/release-version.mjs)
    printf 'release %s (build %s) from %s\n' "$version" "$build" "$(git rev-parse --short=8 HEAD)"
    status=0
    ssh "$host" env HIDE_IOS_NO_UPLOAD="${HIDE_IOS_NO_UPLOAD:-}" \
        bash -s -- release "$slug" "$version" "$build" "$(git rev-parse HEAD)" "$([[ "${HIDE_IOS_RELEASE_AGAIN:-}" == 1 ]] && echo 1 || echo 0)" < mobile/scripts/mini-release.sh || status=$?
    if (( status == 255 )); then
        printf 'the connection to %s ended during the release: it may still be running there or have uploaded; read `%s uploads` before running again\n' "$host" "$0" >&2
    fi
    exit "$status"
fi
if [[ "$command" == store-shots ]]; then
    out=${HIDE_STORE_SHOTS_OUT:-agents/runs/ios-mission/store-assets}
    mkdir -p "$out/raw"
    rm -f "$out/raw"/*.png
    scp -q "$host:$remote/shots/store/*.png" "$out/raw/"
    node mobile/scripts/store-shots.mjs compose "$out/raw" "$out"
fi
if [[ ( "$command" == run || "$command" == push ) && -n "${HIDE_IOS_CAPTURE:-}" ]]; then
    scp -q "$host:$remote/shots/run.png" "$HIDE_IOS_CAPTURE"
fi

if [[ "$command" == run && -n "${HIDE_IOS_RECORD:-}" ]]; then
    scp -q "$host:$remote/shots/launch.mp4" "$HIDE_IOS_RECORD"
fi

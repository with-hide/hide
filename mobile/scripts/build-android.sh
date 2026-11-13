#!/usr/bin/env bash
# Builds the Hide Android app with the phone bundle inside it, on the Mac mini, and runs it in the emulator there.
#
# Usage: build-android.sh [build | release | uploads | run | store-shots | clean]
#   build  (default) the phone bundle, the version and plugin check, the web folder (assemble.mjs), `cap sync android`, a
#          copy of the Gradle project to the mini, and there: the JVM tests, the debug app and its instrumented tests,
#          which run on the mini's emulator; a failing test fails the command
#   run    install the built debug app on the emulator, launch it, wait, capture the screen
#          (a pairing link on standard input, `printf '%s' "$link" | build-android.sh run`, reaches the app as the App Link
#           a camera would open and Connect is pressed once the page asks; HIDE_ANDROID_TAPS='<css selector>;;<css selector>'
#           presses those on the page, in order, once each is there (`<selector>::=<text>` fills a field, `<selector>::^`
#           scrolls to it); HIDE_ANDROID_APPEARANCE=light|dark sets the emulator's appearance;
#           HIDE_ANDROID_CAPTURE=<file.png> copies the capture back here, and the app's log beside it as <file>.log;
#           HIDE_ANDROID_SETTLE=<seconds>, default 15)
#   release  everything `build` does, then on the mini (mini-android-release.sh): the release bundle (AAB) built unsigned
#          with the version of mobile/android/version.json and the version code the Unix minute of the run, signed with
#          the upload key by jarsigner, checked by check-release-aab.py against mobile/android-release-expect.json, and
#          uploaded to Play's internal testing track. It refuses tracked changes, a commit that is not on origin/main, a
#          commit that already has an upload record (HIDE_ANDROID_RELEASE_AGAIN=1 uploads it again on purpose),
#          HIDE_ANDROID_SKIP_PHONE_BUILD and HIDE_ANDROID_RELAY_PORT. HIDE_ANDROID_NO_UPLOAD=1 stops after the release
#          checks, before anything is sent to Play, and may run from a branch, so a change to the command is proven
#          before it merges; HIDE_ANDROID_BUNDLE_OUT=<file.aab> then copies the signed, checked bundle back here, for
#          Play's first upload, which is by hand. The secrets never leave the mini (~/.config/hide-android,
#          mobile/android-release-env.json).
#   uploads  the mini's Android upload record, every branch: when, which commit, version, version code, state and branch
#   store-shots  the Google Play screenshots, feature graphic and icon in Korean and English: the same phone build and copy
#          as `build`, then on the mini the debug app (no test) runs in the emulator, one shot at a time, in demo mode, in the
#          shot's language and in light appearance with the status bar fixed at 9:41, and the raw captures come back to be
#          framed by mobile/scripts/store-shots.mjs: HIDE_STORE_SHOTS_OUT=<folder>, default
#          agents/runs/android-mission/store-assets (docs/BUILD.md, The store page of the Android app)
#   clean  delete this branch's whole folder on the mini
#
# Every command ends what it started on the mini: the emulator it booted is shut down, its own adb server is stopped and
# the emulator lock is released, on any way out. One emulator runs on the mini at a time: a second command waits for the
# lock up to ten minutes and then refuses, naming the command that holds it. The app's data stays in the emulator between
# runs (a pairing is kept across a restart of the app and of the emulator); the instrumented tests of a `build` clear the
# pairing items they test, so pair again after a build.
# A branch's state is one folder, ~/hide-android-build/<branch>/; past 4 other branches' folders the build refuses and
# says which to remove. Gradle's download cache is the mini account's own ~/.gradle, shared by every branch.
#
# The mini has the JDK, the Android SDK and the emulator (docs/BUILD.md, The Android app); it needs no node: the web
# bundle and `cap sync` happen here.
#
# HIDE_ANDROID_BUILD_HOST     ssh name of the build machine (default: mini)
# HIDE_ANDROID_SKIP_PHONE_BUILD=1   (build only) reuse web/phone-dist instead of building it (it needs the Rust wasm toolchain)
# HIDE_ANDROID_RELAY_PORT=<port>    (build and run) a relay on this Mac's loopback (`hide-relay --listen 127.0.0.1:<port>`,
#                                   its socket at /ws): the phone build and the app's
#                                   content policy admit ws://127.0.0.1:<port>, SSH forwards that port to the mini and
#                                   `adb reverse` forwards the emulator's to it; never the production relay
set -euo pipefail

cd "$(dirname "$0")/../.."

command=${1:-build}
host=${HIDE_ANDROID_BUILD_HOST:-mini}
case "$command" in build | release | uploads | run | store-shots | clean) ;; *) printf 'usage: %s [build | release | uploads | run | store-shots | clean]\n' "$0" >&2; exit 2 ;; esac
ssh_options=()
if [[ -n "${HIDE_ANDROID_RELAY_PORT:-}" ]]; then
    [[ "$command" != release ]] || { printf 'a relay fixture cannot be used for release\n' >&2; exit 2; }
    [[ "$HIDE_ANDROID_RELAY_PORT" =~ ^[1-9][0-9]{3,4}$ ]] && (( HIDE_ANDROID_RELAY_PORT > 1023 && HIDE_ANDROID_RELAY_PORT < 65536 )) || { printf 'invalid HIDE_ANDROID_RELAY_PORT\n' >&2; exit 2; }
    ssh_options=(-o ExitOnForwardFailure=yes -R "127.0.0.1:$HIDE_ANDROID_RELAY_PORT:127.0.0.1:$HIDE_ANDROID_RELAY_PORT")
fi
branch=$(git branch --show-current)
[[ -n "$branch" ]] || { printf 'build-android.sh needs a checked-out branch\n' >&2; exit 2; }
slug=$(printf '%s' "$branch" | tr -c 'A-Za-z0-9._-' '-')
remote=hide-android-build/$slug

if [[ "$command" == uploads ]]; then
    exec ssh "$host" bash -s -- uploads "$slug" < mobile/scripts/mini-android-release.sh
fi
if [[ "$command" == release ]]; then
    # A release is the committed source of main: the bundle that reaches Play must be one a person can find again.
    if [[ -n "$(git status --porcelain --untracked-files=no)" ]]; then
        printf 'release needs a clean working tree: commit the tracked changes first\n' >&2
        exit 1
    fi
    if [[ "${HIDE_ANDROID_SKIP_PHONE_BUILD:-}" == 1 ]]; then
        printf 'release builds the phone bundle from this commit; HIDE_ANDROID_SKIP_PHONE_BUILD is for build only\n' >&2
        exit 2
    fi
    if [[ "${HIDE_ANDROID_NO_UPLOAD:-}" != 1 ]]; then
        git fetch -q origin main
        if ! git merge-base --is-ancestor HEAD origin/main; then
            printf 'release uploads a commit of main only, and %s is not on origin/main; HIDE_ANDROID_NO_UPLOAD=1 builds and checks it without uploading\n' "$(git rev-parse --short=10 HEAD)" >&2
            exit 1
        fi
        # Refuse a commit that already went before the long build, not after it: its last record line decides.
        last=$(ssh "$host" bash -s -- uploads "$slug" < mobile/scripts/mini-android-release.sh | grep -F " $(git rev-parse HEAD) " | tail -n 1 || true)
        if [[ -n "$last" && "$last" != *" deleted "* && "${HIDE_ANDROID_RELEASE_AGAIN:-}" != 1 ]]; then
            printf 'this commit already has an upload record; read it with `%s uploads`, and set HIDE_ANDROID_RELEASE_AGAIN=1 to upload it again on purpose\n' "$0" >&2
            exit 1
        fi
    fi
fi
# A running release owns the machine's release: a build, store-shots or clean of its branch would change the project under
# it, and a second release waits for the first.
if [[ "$command" == build || "$command" == release || "$command" == store-shots || "$command" == clean ]]; then
    ssh "$host" bash -s -- busy "$slug" "$([[ "$command" == release ]] && echo machine || echo branch)" < mobile/scripts/mini-android-release.sh
fi
# The shots to take, read (and checked) before anything runs on the mini.
store_plan=
if [[ "$command" == store-shots ]]; then
    store_plan=$(node mobile/scripts/store-shots.mjs plan-android)
fi

# The pairing link of a `run`, read before anything else so it is never an argument or a file.
pair_link=
if [[ "$command" == run && ! -t 0 ]]; then
    IFS= read -r pair_link || true
fi

if [[ "$command" == build || "$command" == release || "$command" == store-shots ]]; then
    if [[ "${HIDE_ANDROID_SKIP_PHONE_BUILD:-}" != 1 ]]; then
        if [[ -n "${HIDE_ANDROID_RELAY_PORT:-}" ]]; then
            VITE_HIDE_RELAY_URL="ws://127.0.0.1:$HIDE_ANDROID_RELAY_PORT/ws" bash scripts/verify-web.sh web build:phone
        else
            bash scripts/verify-web.sh web build:phone
        fi
    fi
    node mobile/scripts/check-versions.mjs
    ssh "$host" bash -s -- guard "$slug" < mobile/scripts/mini-android.sh
    if [[ -n "${HIDE_ANDROID_RELAY_PORT:-}" ]]; then
        node mobile/scripts/assemble.mjs --simulator-relay-port "$HIDE_ANDROID_RELAY_PORT"
    else
        node mobile/scripts/assemble.mjs
    fi
    pnpm --dir mobile exec cap sync android
    # The Gradle project and the Capacitor library it names, at the same places relative to each other as here.
    library=$(sed -n "s|^project(':capacitor-android').projectDir = new File('\(.*\)')$|\1|p" mobile/android/capacitor.settings.gradle)
    [[ -n "$library" && -f "mobile/android/$library/build.gradle" ]] || { printf 'capacitor.settings.gradle names no Capacitor library\n' >&2; exit 1; }
    library_in_repo=$(cd "mobile/android/$library" && pwd -P)
    library_in_repo=${library_in_repo#"$(pwd -P)/"}
    ssh "$host" "mkdir -p '$remote/repo/mobile/android' '$remote/repo/$library_in_repo'"
    rsync -a --delete --exclude '.gradle' --exclude 'build' --exclude 'local.properties' mobile/android/ "$host:$remote/repo/mobile/android/"
    rsync -a --delete --exclude 'build' "$library_in_repo/" "$host:$remote/repo/$library_in_repo/"
fi

remote_status=0
{
    if [[ -n "$pair_link" ]]; then printf 'HIDE_PAIR_LINK=%q\n' "$pair_link"; fi
    if [[ -n "${HIDE_ANDROID_TAPS:-}" ]]; then printf 'HIDE_TAPS=%q\n' "$HIDE_ANDROID_TAPS"; fi
    if [[ "$command" == store-shots ]]; then
        printf 'HIDE_STORE_PLAN=%q\n' "$store_plan"
        printf 'HIDE_STORE_CLOCK=%q\n' "$(node -e "process.stdout.write(JSON.parse(require('fs').readFileSync('mobile/store/android/shots.json','utf8')).clock)")"
        printf 'HIDE_STORE_APPEARANCE=%q\n' "$(node -e "process.stdout.write(JSON.parse(require('fs').readFileSync('mobile/store/android/shots.json','utf8')).appearance)")"
    fi
    cat mobile/scripts/mini-android.sh
} | ssh "${ssh_options[@]}" "$host" env HIDE_ANDROID_SETTLE="${HIDE_ANDROID_SETTLE:-15}" \
    HIDE_ANDROID_RELAY_PORT="${HIDE_ANDROID_RELAY_PORT:-}" HIDE_ANDROID_APPEARANCE="${HIDE_ANDROID_APPEARANCE:-}" \
    bash -s -- "${command/release/build}" "$slug" || remote_status=$?
(( remote_status == 0 )) || exit "$remote_status"

# `release` ran the same build and tests just above: a build whose tests fail is never signed.
if [[ "$command" == release ]]; then
    inputs=$(mktemp -d)
    trap 'rm -rf "$inputs"' EXIT
    node mobile/scripts/bundle-manifest.mjs "$inputs/bundle-manifest.json" android
    cp mobile/android-release-env.json mobile/android-release-expect.json mobile/scripts/check-release-aab.py "$inputs/"
    rsync -a --delete "$inputs/" "$host:$remote/release-inputs/"
    # The minute is taken here, after the tests, so the version code is as close to the upload as the command can make it.
    read -r version code < <(node mobile/scripts/release-version.mjs android)
    printf 'Android release %s (version code %s) from %s\n' "$version" "$code" "$(git rev-parse --short=10 HEAD)"
    status=0
    ssh "$host" env HIDE_ANDROID_NO_UPLOAD="${HIDE_ANDROID_NO_UPLOAD:-}" \
        bash -s -- release "$slug" "$version" "$code" "$(git rev-parse HEAD)" "$([[ "${HIDE_ANDROID_RELEASE_AGAIN:-}" == 1 ]] && echo 1 || echo 0)" < mobile/scripts/mini-android-release.sh || status=$?
    if (( status == 255 )); then
        printf 'the connection to %s ended during the release: it may still be running there or have uploaded; read `%s uploads` before running again\n' "$host" "$0" >&2
    fi
    if (( status == 0 )) && [[ "${HIDE_ANDROID_NO_UPLOAD:-}" == 1 && -n "${HIDE_ANDROID_BUNDLE_OUT:-}" ]]; then
        scp -q "$host:$remote/signed.aab" "$HIDE_ANDROID_BUNDLE_OUT"
        printf 'signed bundle: %s\n' "$HIDE_ANDROID_BUNDLE_OUT"
    fi
    exit "$status"
fi

if [[ "$command" == run && -n "${HIDE_ANDROID_CAPTURE:-}" ]]; then
    scp -q "$host:$remote/shots/run.png" "$HIDE_ANDROID_CAPTURE"
    scp -q "$host:$remote/shots/run.log" "${HIDE_ANDROID_CAPTURE%.png}.log"
fi

if [[ "$command" == store-shots ]]; then
    out=${HIDE_STORE_SHOTS_OUT:-agents/runs/android-mission/store-assets}
    mkdir -p "$out/raw"
    rm -f "$out/raw"/*.png
    scp -q "$host:$remote/shots/store/*.png" "$out/raw/"
    node mobile/scripts/store-shots.mjs compose-android "$out/raw" "$out"
fi

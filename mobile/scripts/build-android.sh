#!/usr/bin/env bash
# Builds the Hide Android app with the phone bundle inside it, on the Mac mini, and runs it in the emulator there.
#
# Usage: build-android.sh [build | run | clean]
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
case "$command" in build | run | clean) ;; *) printf 'usage: %s [build | run | clean]\n' "$0" >&2; exit 2 ;; esac
ssh_options=()
if [[ -n "${HIDE_ANDROID_RELAY_PORT:-}" ]]; then
    [[ "$HIDE_ANDROID_RELAY_PORT" =~ ^[1-9][0-9]{3,4}$ ]] && (( HIDE_ANDROID_RELAY_PORT > 1023 && HIDE_ANDROID_RELAY_PORT < 65536 )) || { printf 'invalid HIDE_ANDROID_RELAY_PORT\n' >&2; exit 2; }
    ssh_options=(-o ExitOnForwardFailure=yes -R "127.0.0.1:$HIDE_ANDROID_RELAY_PORT:127.0.0.1:$HIDE_ANDROID_RELAY_PORT")
fi
branch=$(git branch --show-current)
[[ -n "$branch" ]] || { printf 'build-android.sh needs a checked-out branch\n' >&2; exit 2; }
slug=$(printf '%s' "$branch" | tr -c 'A-Za-z0-9._-' '-')
remote=hide-android-build/$slug

# The pairing link of a `run`, read before anything else so it is never an argument or a file.
pair_link=
if [[ "$command" == run && ! -t 0 ]]; then
    IFS= read -r pair_link || true
fi

if [[ "$command" == build ]]; then
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
    # The iOS app's notice words, which the JVM tests hold the Android table equal to (NoticeWordsTest).
    words=mobile/ios/App/HidePushCore/Sources/HidePushCore
    ssh "$host" "mkdir -p '$remote/repo/mobile/android' '$remote/repo/$library_in_repo' '$remote/repo/$words'"
    rsync -a --delete --exclude '.gradle' --exclude 'build' --exclude 'local.properties' mobile/android/ "$host:$remote/repo/mobile/android/"
    rsync -a --delete --exclude 'build' "$library_in_repo/" "$host:$remote/repo/$library_in_repo/"
    rsync -a "$words/NoticeWords.swift" "$host:$remote/repo/$words/"
fi

remote_status=0
{
    if [[ -n "$pair_link" ]]; then printf 'HIDE_PAIR_LINK=%q\n' "$pair_link"; fi
    if [[ -n "${HIDE_ANDROID_TAPS:-}" ]]; then printf 'HIDE_TAPS=%q\n' "$HIDE_ANDROID_TAPS"; fi
    cat mobile/scripts/mini-android.sh
} | ssh "${ssh_options[@]}" "$host" env HIDE_ANDROID_SETTLE="${HIDE_ANDROID_SETTLE:-15}" \
    HIDE_ANDROID_RELAY_PORT="${HIDE_ANDROID_RELAY_PORT:-}" HIDE_ANDROID_APPEARANCE="${HIDE_ANDROID_APPEARANCE:-}" \
    bash -s -- "$command" "$slug" || remote_status=$?
(( remote_status == 0 )) || exit "$remote_status"

if [[ "$command" == run && -n "${HIDE_ANDROID_CAPTURE:-}" ]]; then
    scp -q "$host:$remote/shots/run.png" "$HIDE_ANDROID_CAPTURE"
    scp -q "$host:$remote/shots/run.log" "${HIDE_ANDROID_CAPTURE%.png}.log"
fi

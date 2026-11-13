#!/usr/bin/env bash
# Runs on the build machine (piped by build-android.sh): Gradle, adb and the emulator for one branch's copy of the project.
#
# Everything this starts it ends, on every way out including a failure: the emulator it booted, the adb server it started
# (on a port of its own, so an adb another program runs is never touched), the port it reversed and the emulator lock.
# The pairing link of a `run` is the variable HIDE_PAIR_LINK that build-android.sh puts in front of this script on its
# standard input: no argument, no file. The whole script is one function, read to its end before it runs, so a program it
# starts cannot read the rest of it from that same input.
main() {
set -euo pipefail

command=$1
slug=$2
export JAVA_HOME=/opt/homebrew/opt/openjdk@21/libexec/openjdk.jdk/Contents/Home
export ANDROID_HOME=$HOME/Library/Android/sdk
# This script's own adb server, never the default port's.
export ANDROID_ADB_SERVER_PORT=5039
root=$HOME/hide-android-build
base=$root/$slug
project=$base/repo/mobile/android
shots=$base/shots
lock=$root/.emulator.lock
adb=$ANDROID_HOME/platform-tools/adb
emulator_bin=$ANDROID_HOME/emulator/emulator
avd=hide-android
console_port=5584
serial=emulator-$console_port
package=me.grab.hide.android
# One folder per branch built here: past this many other branches the build refuses and says what to remove, because the
# machine runs other work and nothing prunes a branch it does not own.
max_other_branches=4
settle=${HIDE_ANDROID_SETTLE:-15}

emulator_pid=
locked=0
adb_started=0
reversed=
cleanup() {
    if [[ -n "$reversed" ]]; then "$adb" -s "$serial" reverse --remove "tcp:$reversed" < /dev/null > /dev/null 2>&1 || true; fi
    if [[ -n "$emulator_pid" ]]; then
        "$adb" -s "$serial" emu kill < /dev/null > /dev/null 2>&1 || true
        for _ in $(seq 1 30); do kill -0 "$emulator_pid" 2> /dev/null || break; sleep 1; done
        kill "$emulator_pid" 2> /dev/null || true
        wait "$emulator_pid" 2> /dev/null || true
    fi
    if [[ "$adb_started" == 1 ]]; then "$adb" kill-server < /dev/null > /dev/null 2>&1 || true; fi
    if [[ "$locked" == 1 ]]; then rm -rf "$lock"; fi
}
trap cleanup EXIT

# `adb shell` hands the device's shell one line: a value goes in single quotes, each quote inside it closed and escaped.
device_quote() {
    local quote="'" value=$1
    printf "'%s'" "${value//$quote/$quote\\$quote$quote}"
}

# One emulator on the machine at a time. The lock is a folder holding the owner's pid; one whose owner is gone is taken over.
take_lock() {
    mkdir -p "$root"
    local waited=0 owner
    until mkdir "$lock" 2> /dev/null; do
        owner=$(cat "$lock/pid" 2> /dev/null || true)
        if [[ -n "$owner" ]] && ! kill -0 "$owner" 2> /dev/null; then rm -rf "$lock"; continue; fi
        if (( waited >= 600 )); then
            printf 'the emulator is busy: %s holds %s\n' "$(ps -o command= -p "${owner:-0}" 2> /dev/null || echo 'an unknown command')" "$lock" >&2
            exit 1
        fi
        sleep 5
        waited=$((waited + 5))
    done
    locked=1
    printf '%s\n' "$$" > "$lock/pid"
}

boot_emulator() {
    take_lock
    "$adb" start-server < /dev/null > /dev/null 2>&1
    adb_started=1
    mkdir -p "$shots"
    "$emulator_bin" -avd "$avd" -port "$console_port" -no-window -no-audio -no-boot-anim -no-snapshot \
        -gpu swiftshader_indirect < /dev/null > "$shots/emulator.log" 2>&1 &
    emulator_pid=$!
    local waited=0
    until [[ "$("$adb" -s "$serial" shell getprop sys.boot_completed < /dev/null 2> /dev/null | tr -d '\r')" == 1 ]]; do
        kill -0 "$emulator_pid" 2> /dev/null || { tail -20 "$shots/emulator.log" >&2; printf 'the emulator ended while it booted\n' >&2; exit 1; }
        (( waited < 300 )) || { printf 'the emulator did not finish booting in 300 s\n' >&2; exit 1; }
        sleep 2
        waited=$((waited + 2))
    done
}

case "$command" in
    guard)
        others=$(find "$root" -mindepth 1 -maxdepth 1 -type d ! -name "$slug" ! -name '.*' 2> /dev/null | sort || true)
        count=$(printf '%s' "$others" | grep -c . || true)
        if (( count >= max_other_branches )); then
            printf 'refusing: %s other branch folders already hold build state on this machine:\n%s\n' "$count" "$others" >&2
            printf 'run `bash mobile/scripts/build-android.sh clean` from those branches, or remove a folder above, then retry\n' >&2
            exit 1
        fi
        ;;
    build)
        cd "$project"
        # --no-daemon: Gradle's process ends with the build instead of waiting for the next one on a machine it does not own.
        ./gradlew --no-daemon --console=plain testDebugUnitTest assembleDebug assembleDebugAndroidTest < /dev/null
        boot_emulator
        "$adb" -s "$serial" install -r -t app/build/outputs/apk/debug/app-debug.apk < /dev/null
        "$adb" -s "$serial" install -r -t app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk < /dev/null
        # The instrumented tests run in the app's own process with its real web view; their summary decides the command.
        "$adb" -s "$serial" shell am instrument -w "$package.test/androidx.test.runner.AndroidJUnitRunner" < /dev/null | tee "$shots/instrumented.txt"
        grep -q '^OK (' "$shots/instrumented.txt" || { printf 'instrumented tests failed\n' >&2; exit 1; }
        "$adb" -s "$serial" uninstall "$package.test" < /dev/null > /dev/null
        printf 'built: %s\n' "$project/app/build/outputs/apk/debug/app-debug.apk"
        df -h / | tail -1 | awk '{printf "build machine free disk: %s of %s\n", $4, $2}'
        ;;
    run)
        apk=$project/app/build/outputs/apk/debug/app-debug.apk
        [[ -f "$apk" ]] || { printf 'no debug app on the build machine: run build first\n' >&2; exit 1; }
        boot_emulator
        "$adb" -s "$serial" install -r "$apk" < /dev/null > /dev/null
        if [[ -n "${HIDE_ANDROID_APPEARANCE:-}" ]]; then
            case "$HIDE_ANDROID_APPEARANCE" in light) mode=no ;; dark) mode=yes ;; *) printf 'invalid HIDE_ANDROID_APPEARANCE\n' >&2; exit 2 ;; esac
            "$adb" -s "$serial" shell cmd uimode night "$mode" < /dev/null > /dev/null
        fi
        if [[ -n "${HIDE_ANDROID_RELAY_PORT:-}" ]]; then
            "$adb" -s "$serial" reverse "tcp:$HIDE_ANDROID_RELAY_PORT" "tcp:$HIDE_ANDROID_RELAY_PORT" < /dev/null > /dev/null
            reversed=$HIDE_ANDROID_RELAY_PORT
        fi
        "$adb" -s "$serial" shell am force-stop "$package" < /dev/null
        "$adb" -s "$serial" logcat -c < /dev/null
        # The debug build's inputs (src/debug DebugInputs.java); the link goes as the App Link intent a camera would send.
        extras=()
        if [[ -n "${HIDE_TAPS:-}" ]]; then extras+=(--es hide_debug_taps "$(device_quote "$HIDE_TAPS")"); fi
        if [[ -n "${HIDE_PAIR_LINK:-}" ]]; then
            "$adb" -s "$serial" shell am start -W -a android.intent.action.VIEW -d "$(device_quote "$HIDE_PAIR_LINK")" -n "$package/.ShellActivity" \
                --ez hide_debug_connect true "${extras[@]}" < /dev/null > /dev/null
        else
            "$adb" -s "$serial" shell am start -W -n "$package/.ShellActivity" "${extras[@]}" < /dev/null > /dev/null
        fi
        if [[ -n "${HIDE_TAPS:-}" ]]; then
            # The app says when its last step is pressed (`done`) or that one was never found (`failed <step>`).
            waited=0 outcome=
            until [[ -n "$outcome" ]]; do
                outcome=$("$adb" -s "$serial" shell run-as "$package" cat cache/hide-debug-taps < /dev/null 2> /dev/null | tr -d '\r' || true)
                (( waited < settle + 120 )) || { printf 'the taps never finished\n' >&2; exit 1; }
                sleep 1
                waited=$((waited + 1))
            done
            [[ "$outcome" == done ]] || { printf 'taps: %s\n' "$outcome" >&2; exit 1; }
        fi
        sleep "$settle"
        mkdir -p "$shots"
        "$adb" -s "$serial" exec-out screencap -p < /dev/null > "$shots/run.png"
        pid=$("$adb" -s "$serial" shell pidof "$package" < /dev/null | tr -d '\r' || true)
        if [[ -n "$pid" ]]; then "$adb" -s "$serial" logcat -d --pid="$pid" < /dev/null > "$shots/run.log" || true; else : > "$shots/run.log"; fi
        "$adb" -s "$serial" shell am force-stop "$package" < /dev/null
        printf 'capture: %s\n' "$shots/run.png"
        ;;
    clean)
        rm -rf "$base"
        ;;
    *) printf 'unknown command %s\n' "$command" >&2; exit 2 ;;
esac
}
main "$@"

#!/usr/bin/env bash
# Runs on the build machine (piped by build-android.sh): Gradle, adb and the emulator for one branch's copy of the project.
#
# Everything this starts it ends, on every way out including a failure: the emulator it booted, the adb server it started
# (on a port of its own, so an adb another program runs is never touched), the port it reversed and the emulator lock.
# A run killed before it could (SIGKILL, a lost connection) leaves them; the next one ends them once it holds the lock.
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
# Never deleted: deleting a lock file another run has open would let two runs hold two locks.
lock=$root/.emulator-lock
# The emulator a run booted, `<pid> <start time>`, so the next run can end it if this one is killed.
record=$root/.emulator-run
adb_bin=$ANDROID_HOME/platform-tools/adb
emulator_bin=$ANDROID_HOME/emulator/emulator
avd=hide-android
console_port=5584
serial=emulator-$console_port
package=me.grab.hide.android
# One folder per branch built here: past this many other branches the build refuses and says what to remove, because the
# machine runs other work and nothing prunes a branch it does not own.
max_other_branches=4
settle=${HIDE_ANDROID_SETTLE:-15}

# The emulator lock is held by the kernel (flock) on file descriptor 9 of this shell, so it ends when the shell ends,
# however it ends. Every program that may outlive the shell starts with 9>&- (each adb and sleep through these functions,
# the emulator, Gradle, tee), so none keeps the lock after the run.
adb() { "$adb_bin" "$@" < /dev/null 9>&-; }
# An adb call to a device that may not answer (one being ended), bounded so a hung device cannot hold the run.
adb_within() { local seconds=$1; shift; perl -e 'alarm shift; exec { $ARGV[0] } @ARGV or die' -- "$seconds" "$adb_bin" "$@" < /dev/null 9>&-; }
pause() { sleep "$1" 9>&-; }
started_at() { ps -o lstart= -p "$1" 2> /dev/null | sed 's/^ *//; s/ *$//' || true; }

emulator_pid=
emulator_started=
locked=0
adb_started=0
reversed=
display_changed=0
cleanup() {
    local exit_status=$?
    # A store-shots run's screen size, density and status bar are the emulator's own settings, kept in its disk: put back.
    if [[ "$display_changed" == 1 ]]; then
        adb_within 20 -s "$serial" shell wm size reset > /dev/null 2>&1 || true
        adb_within 20 -s "$serial" shell wm density reset > /dev/null 2>&1 || true
        adb_within 20 -s "$serial" shell am broadcast -a com.android.systemui.demo -e command exit > /dev/null 2>&1 || true
        adb_within 20 -s "$serial" shell settings put global sysui_demo_allowed 0 > /dev/null 2>&1 || true
    fi
    if [[ -n "$reversed" ]]; then adb -s "$serial" reverse --remove "tcp:$reversed" > /dev/null 2>&1 || true; fi
    if [[ -n "$emulator_pid" ]]; then
        end_emulator "$emulator_pid" "$emulator_started"
        wait "$emulator_pid" 2> /dev/null || true
    fi
    if [[ "$adb_started" == 1 ]]; then adb kill-server > /dev/null 2>&1 || true; fi
    if [[ "$locked" == 1 ]]; then
        rm -f "$record"
        : > "$lock"
        exec 9>&-
    fi
    exit "$exit_status"
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

# Ends an emulator gracefully: its disk synced, since `emu kill` skips the device's own shutdown and what the device has
# not written yet is lost (a half-written file, the web view's storage database, then loses every later write on each
# start), then `emu kill` and up to 30 s to exit. A signal to its process group only for one that will not stop.
end_emulator() {
    local pid=$1 started=$2
    [[ -n "$started" && "$(started_at "$pid")" == "$started" ]] || return 0
    adb_within 20 -s "$serial" shell sync > /dev/null 2>&1 || true
    adb_within 20 -s "$serial" emu kill > /dev/null 2>&1 || true
    for _ in $(seq 1 30); do [[ "$(started_at "$pid")" == "$started" ]] || return 0; pause 1; done
    end_group "$pid" "$started"
}

# Ends the process group of a recorded leader while that leader is still the same process (its pid and start time), so
# a pid reused since is never signalled.
end_group() {
    local pid=$1 started=$2
    [[ -n "$started" && "$(started_at "$pid")" == "$started" ]] || return 0
    kill -TERM -- "-$pid" 2> /dev/null || true
    for _ in $(seq 1 30); do [[ "$(started_at "$pid")" == "$started" ]] || return 0; pause 1; done
    kill -KILL -- "-$pid" 2> /dev/null || true
}

# `adb shell` hands the device's shell one line: a value goes in single quotes, each quote inside it closed and escaped.
device_quote() {
    local quote="'" value=$1
    printf "'%s'" "${value//$quote/$quote\\$quote$quote}"
}

# One emulator on the machine at a time. The lock file's text (pid, start time, branch) only names the holder for a
# person; status 75 from perl is "another run holds it", and any other failure is named as what it is.
take_lock() {
    mkdir -p "$root"
    exec 9<> "$lock"
    local waited=0 status
    while :; do
        status=0
        perl -MFcntl=:flock -e 'open(my $f, "+<&=", 9) or die "lock: $!\n"; flock($f, LOCK_EX | LOCK_NB) or exit 75;
                truncate($f, 0); seek($f, 0, 0); print $f "pid $ARGV[0], started $ARGV[1], branch $ARGV[2]\n"' -- \
                "$$" "$(started_at $$)" "$slug" || status=$?
        (( status == 0 )) && break
        if (( status != 75 )); then
            exec 9>&-
            printf 'the emulator lock %s could not be taken (perl status %s)\n' "$lock" "$status" >&2
            exit 1
        fi
        if (( waited >= 600 )); then
            exec 9>&-
            printf 'the emulator is busy: %s holds it\n' "$(cat "$lock" 2> /dev/null || echo 'an unknown run')" >&2
            exit 1
        fi
        pause 5
        waited=$((waited + 5))
    done
    locked=1
    # What a killed run left: its emulator (by the record), and with it the port reversed through it, and the adb server,
    # whose port is this lock's alone.
    if [[ -s "$record" ]]; then
        local pid started
        read -r pid started < "$record"
        end_emulator "$pid" "$started"
    fi
    rm -f "$record"
    adb kill-server > /dev/null 2>&1 || true
}

boot_emulator() {
    take_lock
    adb_started=1
    adb start-server > /dev/null 2>&1
    mkdir -p "$shots"
    # A process group of its own, so one signal reaches the emulator and every process it starts.
    perl -e 'setpgrp(0, 0); exec { $ARGV[0] } @ARGV or die "cannot start $ARGV[0]: $!\n"' -- "$emulator_bin" -avd "$avd" \
        -port "$console_port" -no-window -no-audio -no-boot-anim -no-snapshot -gpu swiftshader_indirect \
        < /dev/null > "$shots/emulator.log" 2>&1 9>&- &
    emulator_pid=$!
    emulator_started=$(started_at "$emulator_pid")
    printf '%s %s\n' "$emulator_pid" "$emulator_started" > "$record"
    local waited=0
    until [[ "$(adb -s "$serial" shell getprop sys.boot_completed 2> /dev/null | tr -d '\r')" == 1 ]]; do
        kill -0 "$emulator_pid" 2> /dev/null || { tail -20 "$shots/emulator.log" >&2; printf 'the emulator ended while it booted\n' >&2; exit 1; }
        (( waited < 300 )) || { printf 'the emulator did not finish booting in 300 s\n' >&2; exit 1; }
        pause 2
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
        ./gradlew --no-daemon --console=plain testDebugUnitTest assembleDebug assembleDebugAndroidTest < /dev/null 9>&-
        boot_emulator
        adb -s "$serial" install -r -t app/build/outputs/apk/debug/app-debug.apk
        adb -s "$serial" install -r -t app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk
        # The instrumented tests run in the app's own process with its real web view; their summary decides the command.
        adb -s "$serial" shell am instrument -w "$package.test/androidx.test.runner.AndroidJUnitRunner" | tee "$shots/instrumented.txt" 9>&-
        grep -q '^OK (' "$shots/instrumented.txt" || { printf 'instrumented tests failed\n' >&2; exit 1; }
        adb -s "$serial" uninstall "$package.test" > /dev/null
        printf 'built: %s\n' "$project/app/build/outputs/apk/debug/app-debug.apk"
        df -h / | tail -1 | awk '{printf "build machine free disk: %s of %s\n", $4, $2}'
        ;;
    run)
        apk=$project/app/build/outputs/apk/debug/app-debug.apk
        [[ -f "$apk" ]] || { printf 'no debug app on the build machine: run build first\n' >&2; exit 1; }
        boot_emulator
        adb -s "$serial" install -r "$apk" > /dev/null
        if [[ -n "${HIDE_ANDROID_APPEARANCE:-}" ]]; then
            case "$HIDE_ANDROID_APPEARANCE" in light) mode=no ;; dark) mode=yes ;; *) printf 'invalid HIDE_ANDROID_APPEARANCE\n' >&2; exit 2 ;; esac
            adb -s "$serial" shell cmd uimode night "$mode" > /dev/null
        fi
        if [[ -n "${HIDE_ANDROID_RELAY_PORT:-}" ]]; then
            adb -s "$serial" reverse "tcp:$HIDE_ANDROID_RELAY_PORT" "tcp:$HIDE_ANDROID_RELAY_PORT" > /dev/null
            reversed=$HIDE_ANDROID_RELAY_PORT
        fi
        adb -s "$serial" shell am force-stop "$package"
        adb -s "$serial" logcat -c
        # The debug build's inputs (src/debug DebugInputs.java); the link goes as the App Link intent a camera would send.
        extras=()
        if [[ -n "${HIDE_TAPS:-}" ]]; then extras+=(--es hide_debug_taps "$(device_quote "$HIDE_TAPS")"); fi
        if [[ -n "${HIDE_PAIR_LINK:-}" ]]; then
            adb -s "$serial" shell am start -W -a android.intent.action.VIEW -d "$(device_quote "$HIDE_PAIR_LINK")" -n "$package/.ShellActivity" \
                --ez hide_debug_connect true ${extras[@]+"${extras[@]}"} > /dev/null
        else
            adb -s "$serial" shell am start -W -n "$package/.ShellActivity" ${extras[@]+"${extras[@]}"} > /dev/null
        fi
        if [[ -n "${HIDE_TAPS:-}" ]]; then
            # The app says when its last step is pressed (`done`) or that one was never found (`failed <step>`).
            waited=0 outcome=
            until [[ -n "$outcome" ]]; do
                outcome=$(adb -s "$serial" shell run-as "$package" cat cache/hide-debug-taps 2> /dev/null | tr -d '\r' || true)
                (( waited < settle + 120 )) || { printf 'the taps never finished\n' >&2; exit 1; }
                pause 1
                waited=$((waited + 1))
            done
            [[ "$outcome" == done ]] || { printf 'taps: %s\n' "$outcome" >&2; exit 1; }
        fi
        pause "$settle"
        mkdir -p "$shots"
        adb -s "$serial" exec-out screencap -p > "$shots/run.png"
        pid=$(adb -s "$serial" shell pidof "$package" | tr -d '\r' || true)
        if [[ -n "$pid" ]]; then adb -s "$serial" logcat -d --pid="$pid" > "$shots/run.log" || true; else : > "$shots/run.log"; fi
        adb -s "$serial" shell am force-stop "$package"
        printf 'capture: %s\n' "$shots/run.png"
        ;;
    store-shots)
        # The raw captures of the Play screenshots (mobile/scripts/store-shots.mjs composes them): the debug app on a screen of
        # 1080x1920, in demo mode, one shot at a time with the debug inputs of `run`. HIDE_STORE_PLAN is one tab-separated
        # line per shot (language, file, settle seconds, locale tag, steps).
        cd "$project"
        ./gradlew --no-daemon --console=plain assembleDebug < /dev/null 9>&-
        boot_emulator
        adb -s "$serial" install -r app/build/outputs/apk/debug/app-debug.apk > /dev/null
        case "$HIDE_STORE_APPEARANCE" in light) mode=no ;; dark) mode=yes ;; *) printf 'invalid appearance\n' >&2; exit 2 ;; esac
        adb -s "$serial" shell cmd uimode night "$mode" > /dev/null
        display_changed=1
        adb -s "$serial" shell wm size 1080x1920 > /dev/null
        adb -s "$serial" shell wm density 420 > /dev/null
        # The system bar, as Android's own demo mode draws it: 9:41, a full battery and the Wi-Fi signal, no notification icons.
        # A broadcast reaches System UI only once it listens, which is later than the boot's end, so every shot sends them
        # again before it starts (they set the same values again).
        adb -s "$serial" shell settings put global sysui_demo_allowed 1
        demo() { adb -s "$serial" shell am broadcast -a com.android.systemui.demo -e command "$@" > /dev/null; }
        status_bar() {
            demo enter
            demo clock -e hhmm "$(printf '%s' "$HIDE_STORE_CLOCK" | sed -E 's/^[0-9-]+T([0-9]{2}):([0-9]{2}).*/\1\2/')"
            demo battery -e level 100 -e plugged false
            demo network -e wifi show -e level 4 -e fully true
            demo network -e mobile hide
            demo notifications -e visible false
        }
        pause 10
        rm -rf "$shots/store"
        mkdir -p "$shots/store"
        while IFS=$'\t' read -r lang file settle tag steps; do
            [[ -n "$lang" ]] || continue
            status_bar
            adb -s "$serial" shell cmd locale set-app-locales "$package" --locales "$tag" > /dev/null
            adb -s "$serial" shell am force-stop "$package"
            adb -s "$serial" shell run-as "$package" rm -f cache/hide-debug-taps || true
            adb -s "$serial" shell am start -W -n "$package/.ShellActivity" \
                --es hide_debug_taps "$(device_quote "$steps")" --es hide_debug_now "$(device_quote "$HIDE_STORE_CLOCK")" > /dev/null
            waited=0 outcome=
            until [[ -n "$outcome" ]]; do
                outcome=$(adb -s "$serial" shell run-as "$package" cat cache/hide-debug-taps 2> /dev/null | tr -d '\r' || true)
                (( waited < settle + 120 )) || { printf 'store-shots: %s %s: the steps never finished\n' "$lang" "$file" >&2; exit 1; }
                pause 1
                waited=$((waited + 1))
            done
            [[ "$outcome" == done ]] || { printf 'store-shots: %s %s: %s\n' "$lang" "$file" "$outcome" >&2; exit 1; }
            pause "$settle"
            adb -s "$serial" exec-out screencap -p > "$shots/store/$lang-$file.png"
            printf 'capture: %s-%s\n' "$lang" "$file"
        done <<< "$HIDE_STORE_PLAN"
        adb -s "$serial" shell am force-stop "$package"
        ;;
    clean)
        rm -rf "$base"
        ;;
    *) printf 'unknown command %s\n' "$command" >&2; exit 2 ;;
esac
}
main "$@"

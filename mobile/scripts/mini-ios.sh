#!/usr/bin/env bash
# Runs on the build machine (piped by build-ios.sh): xcodebuild and simctl for one branch's copy of the project.
#
# Everything this starts it ends: the simulator it created is shut down and deleted, one it found is shut down if it
# found it off, and the pairing link file is removed, on every way out including a failure. HIDE_IOS_KEEP_SIMULATOR=1
# leaves the simulator booted for a check by hand; `clean` removes it later.
set -euo pipefail

command=$1
slug=$2
export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
root=$HOME/hide-ios-build
base=$root/$slug
project=$base/ios/App
derived=$base/DerivedData
derived_tests=$base/DerivedDataTests
packages=$base/SourcePackages
shots=$base/shots
link_file=$base/pair-link
device_name=hide-ios-$slug
bundle_id=me.grab.hide.ios
# One folder (about 0.5 GB) and one simulator per branch built here: past this many other branches the build refuses
# and says what to remove, because the machine runs other work and nothing prunes a branch it does not own.
max_other_branches=4
settle=${HIDE_IOS_SETTLE:-15}

device_line() { xcrun simctl list devices | sed -n "s/^ *$device_name (\\([0-9A-F-]*\\)) (\\([A-Za-z]*\\)).*/\\1 \\2/p" | head -1; }
device_id() { device_line | cut -d' ' -f1; }

created=0
was_booted=0
id=
cleanup() {
    rm -f "$link_file"
    [[ "${HIDE_IOS_KEEP_SIMULATOR:-}" == 1 ]] && return 0
    [[ -n "$id" ]] || return 0
    if [[ $created == 1 ]]; then
        xcrun simctl shutdown "$id" 2> /dev/null || true
        xcrun simctl delete "$id" 2> /dev/null || true
    elif [[ $was_booted == 0 ]]; then
        xcrun simctl shutdown "$id" 2> /dev/null || true
    fi
}
trap cleanup EXIT

# The simulator this command uses: the branch's own, made now when it is missing.
ensure_device() {
    local line
    line=$(device_line)
    if [[ -z "$line" ]]; then
        id=$(xcrun simctl create "$device_name" 'iPhone 17' com.apple.CoreSimulator.SimRuntime.iOS-26-5)
        created=1
    else
        id=${line%% *}
        [[ "${line#* }" == Booted ]] && was_booted=1
    fi
}

case "$command" in
    guard)
        others=$(find "$root" -mindepth 1 -maxdepth 1 -type d ! -name "$slug" 2> /dev/null | sort || true)
        count=$(printf '%s' "$others" | grep -c . || true)
        if (( count >= max_other_branches )); then
            printf 'refusing: %s other branch folders already hold build state on this machine:\n%s\n' "$count" "$others" >&2
            printf 'run `bash mobile/scripts/build-ios.sh clean` from those branches, or remove a folder above, then retry\n' >&2
            exit 1
        fi
        ;;
    build)
        # `-clonedSourcePackagesDirPath` keeps the Swift packages inside the branch folder, so `clean` removes them too.
        xcodebuild -project "$project/App.xcodeproj" -scheme App -configuration Debug \
            -sdk iphonesimulator -destination 'generic/platform=iOS Simulator' \
            -derivedDataPath "$derived" -clonedSourcePackagesDirPath "$packages" \
            CODE_SIGNING_ALLOWED=NO build
        ensure_device
        # The HideShell tests run in a real web view on the simulator: they are what proves the bridge refuses a frame.
        # Its own derived data: the app's build already holds a Capacitor framework the package's scheme must not meet.
        # A failing test fails this script, and so the build command.
        (cd "$project/HideShell" && xcodebuild test -scheme HideShell -destination "id=$id" \
            -derivedDataPath "$derived_tests" -clonedSourcePackagesDirPath "$packages")
        # The app's own controller, hosted in the app on the simulator: it is what proves the app installs the gate.
        xcodebuild test -project "$project/App.xcodeproj" -scheme App -configuration Debug -destination "id=$id" \
            -derivedDataPath "$derived" -clonedSourcePackagesDirPath "$packages" -only-testing:AppTests \
            CODE_SIGNING_ALLOWED=NO
        printf 'built: %s\n' "$derived/Build/Products/Debug-iphonesimulator/App.app"
        df -h / | tail -1 | awk '{printf "build machine free disk: %s of %s\n", $4, $2}'
        ;;
    run)
        ensure_device
        xcrun simctl bootstatus "$id" -b > /dev/null
        xcrun simctl terminate "$id" "$bundle_id" 2> /dev/null || true
        xcrun simctl install "$id" "$derived/Build/Products/Debug-iphonesimulator/App.app"
        # A pairing link for the simulator check (debug builds only) arrives in a private file, never in an argument.
        if [[ -f "$link_file" ]]; then
            SIMCTL_CHILD_HIDE_DEBUG_PAIR_LINK=$(cat "$link_file") xcrun simctl launch "$id" "$bundle_id"
            rm -f "$link_file"
        else
            xcrun simctl launch "$id" "$bundle_id"
        fi
        mkdir -p "$shots"
        sleep "$settle"
        xcrun simctl io "$id" screenshot "$shots/run.png" 2>&1 | tail -1
        printf 'simulator: %s\ncapture: %s\n' "$id" "$shots/run.png"
        ;;
    clean)
        id=$(device_id)
        if [[ -n "$id" ]]; then
            xcrun simctl shutdown "$id" 2> /dev/null || true
            xcrun simctl delete "$id"
        fi
        id=
        rm -rf "$base"
        ;;
esac

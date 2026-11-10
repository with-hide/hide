#!/usr/bin/env bash
# Runs on the build machine (piped by build-ios.sh): xcodebuild and simctl for one branch's copy of the project.
#
# Everything this starts it ends: the simulator it created is shut down and deleted, and one it found is shut down if it
# found it off, on every way out including a failure. The pairing link of a `run` is the variable HIDE_PAIR_LINK that
# build-ios.sh puts in front of this script on its standard input: no argument, no file.
# HIDE_IOS_KEEP_SIMULATOR=1 leaves the simulator booted for a check by hand and marks it; the next command of this
# branch that does not ask to keep it ends it (as does `clean`), so a kept simulator is never left for good.
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
kept_marker=$base/kept-simulator
device_name=hide-ios-$slug
bundle_id=me.grab.hide.ios
# One folder (about 0.5 GB) and one simulator per branch built here: past this many other branches the build refuses
# and says what to remove, because the machine runs other work and nothing prunes a branch it does not own.
max_other_branches=4
# The simulator build is arm64 only: the build machine is Apple silicon, and the relay library is built for the two arm64
# targets (a device and a simulator), so an x86_64 simulator slice has nothing to link.
# Ad-hoc signed, not unsigned: the simulator gives the Keychain only to an app that carries its entitlements, which an
# unsigned build does not (the Keychain answers -34018). No account or certificate is involved; release signing is the release workflow's.
signing=(CODE_SIGN_IDENTITY=- CODE_SIGNING_REQUIRED=NO CODE_SIGNING_ALLOWED=YES)
settle=${HIDE_IOS_SETTLE:-15}

device_line() { xcrun simctl list devices | sed -n "s/^ *$device_name (\\([0-9A-F-]*\\)) (\\([A-Za-z]*\\)).*/\\1 \\2/p" | head -1; }
device_id() { device_line | cut -d' ' -f1; }

created=0
was_booted=0
id=
# The appearance of a simulator this command found and did not make, put back on the way out.
found_appearance=
# Set by `store-shots`: the status bar override and the system language it put on the simulator.
store_touched=0
cleanup() {
    if [[ -n "$found_appearance" && "$created" == 0 && -n "$id" ]]; then
        xcrun simctl ui "$id" appearance "$found_appearance" 2> /dev/null || true
    fi
    if [[ "$store_touched" == 1 && "$created" == 0 && -n "$id" ]]; then
        xcrun simctl status_bar "$id" clear 2> /dev/null || true
        set_system_language en en_US 2> /dev/null || true
    fi
    if [[ "${HIDE_IOS_KEEP_SIMULATOR:-}" == 1 ]]; then
        [[ -n "$id" && -d "$base" ]] && touch "$kept_marker"
        return 0
    fi
    [[ -n "$id" ]] || return 0
    if [[ $created == 1 ]]; then
        rm -f "$kept_marker"
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
        # A simulator an earlier command was told to keep is this command's to end, unless this one is told to keep it too.
        if [[ -f "$kept_marker" && "${HIDE_IOS_KEEP_SIMULATOR:-}" != 1 ]]; then created=1; fi
    fi
}

# The relay protocol library: the two static libraries built on the developer's Mac (the mini has no Rust) become
# the xcframework the HideShell package links. Made fresh every build, so it is never older than its source.
make_relay_framework() {
    local relay=$base/ios/relay frameworks=$project/HideShell/Frameworks
    [[ -f "$relay/source.json" ]] || { printf 'the relay library is missing: run scripts/build-relay-ios.sh on the developer Mac\n' >&2; exit 1; }
    rm -rf "$frameworks/HideRelayC.xcframework"
    mkdir -p "$frameworks"
    xcodebuild -create-xcframework \
        -library "$relay/aarch64-apple-ios/libhide_relay_ios.a" -headers "$relay/include" \
        -library "$relay/aarch64-apple-ios-sim/libhide_relay_ios.a" -headers "$relay/include" \
        -output "$frameworks/HideRelayC.xcframework"
}

# The simulator's own language and region, so what iOS draws (a notification's "now", the date) is in the shot's language
# too. The change reaches SpringBoard on the next boot, so the simulator is restarted.
set_system_language() {
    xcrun simctl boot "$id" 2> /dev/null || true
    xcrun simctl bootstatus "$id" -b > /dev/null
    xcrun simctl spawn "$id" defaults write -g AppleLanguages -array "$1"
    xcrun simctl spawn "$id" defaults write -g AppleLocale -string "$2"
    xcrun simctl shutdown "$id"
    xcrun simctl boot "$id"
    xcrun simctl bootstatus "$id" -b > /dev/null
}

# A status bar that reads the same in every capture: the time of the page's clock, a full battery, full signal.
fix_status_bar() {
    xcrun simctl status_bar "$id" override --time 9:41 --dataNetwork wifi --wifiMode active --wifiBars 3 \
        --cellularMode active --cellularBars 4 --operatorName '' --batteryState charged --batteryLevel 100
}

# `-clonedSourcePackagesDirPath` keeps the Swift packages inside the branch folder, so `clean` removes them too.
build_app() {
    xcodebuild -project "$project/App.xcodeproj" -scheme App -configuration Debug \
        -sdk iphonesimulator -destination 'generic/platform=iOS Simulator' \
        -derivedDataPath "$derived" -clonedSourcePackagesDirPath "$packages" \
        ARCHS=arm64 "${signing[@]}" build
}

# The UI test bundle that answers the notification question is built too, so `allow` needs no build of its own.
build_ui_tests() {
    xcodebuild build-for-testing -project "$project/App.xcodeproj" -scheme App -configuration Debug -destination "id=$id" \
        -derivedDataPath "$derived" -clonedSourcePackagesDirPath "$packages" -only-testing:AppUITests ARCHS=arm64 "${signing[@]}"
}

# Answers the system's notification question for the built app on this branch's simulator: SpringBoard shows it,
# `simctl` cannot press it, so a UI test does (AppUITests). It launches the app with the debug taps it was sent
# (the same ones `run` takes), so the page asks the question itself. The grant stays in the simulator.
allow_notifications() {
    xcrun simctl bootstatus "$id" -b > /dev/null
    TEST_RUNNER_HIDE_DEBUG_TAPS=${HIDE_TAPS:-} xcodebuild test-without-building -project "$project/App.xcodeproj" -scheme App \
        -configuration Debug -destination "id=$id" -derivedDataPath "$derived" -clonedSourcePackagesDirPath "$packages" \
        -only-testing:AppUITests "${signing[@]}"
}

case "$command" in
    guard)
        # The machine's release lock and upload record (mini-release.sh) are files beside the branch folders, not branches.
        others=$(find "$root" -mindepth 1 -maxdepth 1 -type d ! -name "$slug" ! -name '.*' 2> /dev/null | sort || true)
        count=$(printf '%s' "$others" | grep -c . || true)
        if (( count >= max_other_branches )); then
            printf 'refusing: %s other branch folders already hold build state on this machine:\n%s\n' "$count" "$others" >&2
            printf 'run `bash mobile/scripts/build-ios.sh clean` from those branches, or remove a folder above, then retry\n' >&2
            exit 1
        fi
        ;;
    build)
        make_relay_framework
        build_app
        ensure_device
        # The HideShell tests run in a real web view on the simulator: they are what proves the bridge refuses a frame.
        # Its own derived data: the app's build already holds a Capacitor framework the package's scheme must not meet.
        # A failing test fails this script, and so the build command.
        (cd "$project/HideShell" && xcodebuild test -scheme HideShell -destination "id=$id" \
            -derivedDataPath "$derived_tests" -clonedSourcePackagesDirPath "$packages")
        # The push core the app and its notification extension share: the RFC 8291 vector, the notice, the words.
        (cd "$project/HidePushCore" && xcodebuild test -scheme HidePushCore -destination "id=$id" \
            -derivedDataPath "$derived_tests" -clonedSourcePackagesDirPath "$packages")
        # The app's own controller, hosted in the app on the simulator: it is what proves the app installs the gate.
        xcodebuild test -project "$project/App.xcodeproj" -scheme App -configuration Debug -destination "id=$id" \
            -derivedDataPath "$derived" -clonedSourcePackagesDirPath "$packages" -only-testing:AppTests \
            "${signing[@]}"
        build_ui_tests
        printf 'built: %s\n' "$derived/Build/Products/Debug-iphonesimulator/App.app"
        df -h / | tail -1 | awk '{printf "build machine free disk: %s of %s\n", $4, $2}'
        ;;
    store-shots)
        # The raw captures of the App Store screenshots (mobile/scripts/store-shots.mjs composes them). Inputs in front of
        # this script on its standard input, as for `run`: HIDE_STORE_PLAN, one tab-separated line per language and shot
        # (language, file, capture kind (`notification` asks iOS's notification question first), seconds to wait, Apple language, Apple locale, the debug steps), HIDE_STORE_CLOCK
        # (the page's clock) and HIDE_STORE_APPEARANCE. The app is built here, without the tests `build` runs.
        make_relay_framework
        build_app
        ensure_device
        store_touched=1
        xcrun simctl bootstatus "$id" -b > /dev/null 2>&1 || { xcrun simctl boot "$id" 2> /dev/null || true; xcrun simctl bootstatus "$id" -b > /dev/null; }
        build_ui_tests
        xcrun simctl install "$id" "$derived/Build/Products/Debug-iphonesimulator/App.app"
        if [[ $created == 0 && ! -f "$kept_marker" ]]; then found_appearance=$(xcrun simctl ui "$id" appearance 2> /dev/null || true); fi
        xcrun simctl ui "$id" appearance "${HIDE_STORE_APPEARANCE:-light}"
        mkdir -p "$shots/store"
        rm -f "$shots/store"/*.png
        # iOS's own question is asked once, in the language the simulator starts in: the first notification shot's steps ask it.
        set_system_language en en_US
        while IFS=$'\t' read -r lang file capture settle language locale steps; do
            [[ "$capture" == notification ]] || continue
            HIDE_TAPS=$steps allow_notifications
            break
        done <<< "$HIDE_STORE_PLAN"
        current=
        while IFS=$'\t' read -r lang file capture settle language locale steps; do
            if [[ "$lang" != "$current" ]]; then
                set_system_language "$language" "$locale"
                current=$lang
            fi
            fix_status_bar
            xcrun simctl terminate "$id" "$bundle_id" 2> /dev/null || true
            SIMCTL_CHILD_HIDE_DEBUG_TAPS=$steps SIMCTL_CHILD_HIDE_DEBUG_NOW=${HIDE_STORE_CLOCK:-} \
                xcrun simctl launch "$id" "$bundle_id" -AppleLanguages "($language)" -AppleLocale "$locale" > /dev/null
            sleep "$settle"
            xcrun simctl io "$id" screenshot "$shots/store/$lang-$file.png" 2>&1 | tail -1
            xcrun simctl terminate "$id" "$bundle_id" 2> /dev/null || true
        done <<< "$HIDE_STORE_PLAN"
        ls "$shots/store"
        ;;
    allow)
        ensure_device
        allow_notifications
        ;;
    push)
        # Delivers one Apple-shaped push body to the installed app on this branch's simulator, the way Apple would, and
        # captures the screen while the home screen shows the banner: the body travels in front of this script as
        # HIDE_PUSH_B64 (base64), is written to a file of the branch's own folder and removed on the way out.
        ensure_device
        xcrun simctl bootstatus "$id" -b > /dev/null
        mkdir -p "$shots"
        body=$shots/push-body.json
        trap 'rm -f "$body"; cleanup' EXIT
        printf '%s' "$HIDE_PUSH_B64" | base64 -D > "$body"
        xcrun simctl terminate "$id" "$bundle_id" 2> /dev/null || true
        xcrun simctl launch "$id" com.apple.springboard > /dev/null 2>&1 || true
        xcrun simctl push "$id" "$bundle_id" "$body"
        sleep "$settle"
        xcrun simctl io "$id" screenshot "$shots/run.png" 2>&1 | tail -1
        printf 'simulator: %s\ncapture: %s\n' "$id" "$shots/run.png"
        ;;
    run)
        ensure_device
        xcrun simctl bootstatus "$id" -b > /dev/null
        xcrun simctl terminate "$id" "$bundle_id" 2> /dev/null || true
        xcrun simctl install "$id" "$derived/Build/Products/Debug-iphonesimulator/App.app"
        if [[ -n "${HIDE_IOS_APPEARANCE:-}" ]]; then
            if [[ $created == 0 && ! -f "$kept_marker" ]]; then found_appearance=$(xcrun simctl ui "$id" appearance 2> /dev/null || true); fi
            xcrun simctl ui "$id" appearance "$HIDE_IOS_APPEARANCE"
        fi
        mkdir -p "$shots"
        # The debug build's inputs (see ShellViewController): a pairing link and taps, from variables the script was
        # sent with. `HIDE_IOS_HOME=1` shows the home screen with the icon, `HIDE_IOS_LAUNCH_SCREEN=1` holds the app on
        # its launch screen (it starts and waits for a debugger that never comes) and captures that.
        if [[ "${HIDE_IOS_HOME:-}" == 1 ]]; then
            xcrun simctl launch "$id" "$bundle_id" > /dev/null
            sleep 2
            xcrun simctl terminate "$id" "$bundle_id" 2> /dev/null || true
            xcrun simctl launch "$id" com.apple.springboard > /dev/null 2>&1 || true
        elif [[ "${HIDE_IOS_LAUNCH_SCREEN:-}" == 1 ]]; then
            xcrun simctl launch --wait-for-debugger "$id" "$bundle_id" > /dev/null
        else
            SIMCTL_CHILD_HIDE_DEBUG_PAIR_LINK=${HIDE_PAIR_LINK:-} SIMCTL_CHILD_HIDE_DEBUG_TAPS=${HIDE_TAPS:-} xcrun simctl launch "$id" "$bundle_id"
        fi
        sleep "$settle"
        xcrun simctl io "$id" screenshot "$shots/run.png" 2>&1 | tail -1
        [[ "${HIDE_IOS_LAUNCH_SCREEN:-}" == 1 ]] && xcrun simctl terminate "$id" "$bundle_id" 2> /dev/null || true
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

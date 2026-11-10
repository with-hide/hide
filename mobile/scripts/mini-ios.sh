#!/usr/bin/env bash
# Runs on the build machine (piped by build-ios.sh): xcodebuild and simctl for one branch's copy of the project.
set -euo pipefail

command=$1
slug=$2
export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
base=$HOME/hide-ios-build/$slug
project=$base/ios/App
derived=$base/DerivedData
derived_tests=$base/DerivedDataTests
packages=$base/SourcePackages
device_name=hide-ios-$slug
bundle_id=me.grab.hide.ios

device_id() { xcrun simctl list devices | sed -n "s/^ *$device_name (\\([0-9A-F-]*\\)).*/\\1/p" | head -1; }

case "$command" in
    build)
        # `-clonedSourcePackagesDirPath` keeps the Swift packages inside the branch folder, so `clean` removes them too.
        xcodebuild -project "$project/App.xcodeproj" -scheme App -configuration Debug \
            -sdk iphonesimulator -destination 'generic/platform=iOS Simulator' \
            -derivedDataPath "$derived" -clonedSourcePackagesDirPath "$packages" \
            CODE_SIGNING_ALLOWED=NO build
        id=$(device_id)
        if [[ -z "$id" ]]; then
            id=$(xcrun simctl create "$device_name" 'iPhone 17' com.apple.CoreSimulator.SimRuntime.iOS-26-5)
        fi
        # The HideShell tests run in a real web view on the simulator: they are what proves the bridge refuses a frame.
        # Its own derived data: the app's build already holds a Capacitor framework the package's scheme must not meet.
        (cd "$project/HideShell" && xcodebuild test -scheme HideShell -destination "id=$id" \
            -derivedDataPath "$derived_tests" -clonedSourcePackagesDirPath "$packages")
        printf 'built: %s\n' "$derived/Build/Products/Debug-iphonesimulator/App.app"
        ;;
    run)
        id=$(device_id)
        [[ -n "$id" ]] || id=$(xcrun simctl create "$device_name" 'iPhone 17' com.apple.CoreSimulator.SimRuntime.iOS-26-5)
        xcrun simctl bootstatus "$id" -b > /dev/null
        xcrun simctl terminate "$id" "$bundle_id" 2> /dev/null || true
        xcrun simctl install "$id" "$derived/Build/Products/Debug-iphonesimulator/App.app"
        # A pairing link for the simulator check (debug builds only) arrives in a private file, never in an argument.
        link_file=$base/pair-link
        if [[ -f "$link_file" ]]; then
            SIMCTL_CHILD_HIDE_DEBUG_PAIR_LINK=$(cat "$link_file") xcrun simctl launch "$id" "$bundle_id"
            rm -f "$link_file"
        else
            xcrun simctl launch "$id" "$bundle_id"
        fi
        printf 'simulator: %s\n' "$id"
        ;;
    clean)
        id=$(device_id)
        if [[ -n "$id" ]]; then
            xcrun simctl shutdown "$id" 2> /dev/null || true
            xcrun simctl delete "$id"
        fi
        rm -rf "$derived" "$derived_tests" "$packages" "$base/ios"
        ;;
esac

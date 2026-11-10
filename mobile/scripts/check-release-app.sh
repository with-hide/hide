#!/usr/bin/env bash
# The release gate on the signed app: what the build machine reads out of the exported ipa, held to what the release
# is allowed to be. Prints every failed check and exits 1 when there is one; the upload never starts after a failure.
#
# Usage: check-release-app.sh <app dir> <facts dir> <release-expect.json> <bundle manifest.json> <version> <build> [<relay source digest>]
#   <app dir>     the unpacked Payload/App.app
#   <facts dir>   JSON the caller made from the app's property lists (mini-release.sh does, with plutil and codesign):
#                 info.json, privacy.json, entitlements.json, profile.json, and for each app extension in PlugIns/
#                 appex/<name>/info.json, privacy.json, entitlements.json
#   <digest>      the digest of the relay protocol sources in the checkout the release runs on (scripts/build-relay-ios.sh
#                 --digest); the library inside the app must say it came from exactly these sources
#   <manifest>    the bundle manifest bundle-manifest.mjs wrote: the sha256 of every file the app's public/ may hold
#
# It needs jq and a sha256 tool only, so its tests run wherever the mobile tests do. The entitlements in
# release-expect.json are the exact list: a node that adds a capability adds its entitlement there in the same change.
set -euo pipefail

(( $# == 6 || $# == 7 )) || { printf 'usage: %s <app dir> <facts dir> <release-expect.json> <manifest.json> <version> <build> [<relay source digest>]\n' "$0" >&2; exit 2; }
app=$1 facts=$2 expect=$3 manifest=$4 version=$5 build=$6 relay_digest=${7:-}

failures=0
fail() { printf 'release check failed: %s\n' "$1" >&2; failures=$((failures + 1)); }
# `ok <what> <jq filter> <file>`: the filter must print true.
ok() {
    local result
    result=$(jq -r "$2" "$3" 2> /dev/null || printf 'error')
    [[ "$result" == true ]] || fail "$1"
}
sha256_of() { if command -v sha256sum > /dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }

for file in "$facts/info.json" "$facts/privacy.json" "$facts/entitlements.json" "$facts/profile.json" "$app/capacitor.config.json" "$expect" "$manifest"; do
    [[ -s "$file" ]] || { printf 'release check failed: %s is missing or empty\n' "$file" >&2; exit 1; }
done

bundle=$(jq -r .bundleId "$expect")
team=$(jq -r .teamId "$expect")
min_os=$(jq -r .minimumOS "$expect")
sdk=$(jq -r .sdkPrefix "$expect")

# The identity of the build.
ok "bundle id is $bundle" ".CFBundleIdentifier == \"$bundle\"" "$facts/info.json"
ok "version is $version" ".CFBundleShortVersionString == \"$version\"" "$facts/info.json"
ok "build number is $build" ".CFBundleVersion == \"$build\"" "$facts/info.json"
ok "iPhone only" '.UIDeviceFamily == [1]' "$facts/info.json"
ok "minimum iOS is $min_os" ".MinimumOSVersion == \"$min_os\"" "$facts/info.json"
ok "built with the $sdk SDK" "(.DTSDKName // \"\") | startswith(\"$sdk\")" "$facts/info.json"

# Export compliance: the app carries its own encryption (docs/RELAY_PROTOCOL.md), so the answer is the operator's, given
# in App Store Connect's questionnaire. A key in the app would answer for them (and `true` without the code Apple issues
# after reviewing documents is refused at validation, error 90592), so no build carries one until the operator decides.
ok "the app answers no export compliance question itself" '(has("ITSAppUsesNonExemptEncryption") or has("ITSEncryptionExportComplianceCode")) | not' "$facts/info.json"

# No debug left in: Capacitor's own debug switch, an ATS exception, a web view anyone can inspect, a remote page.
ok "CAPACITOR_DEBUG is not on" '((.CAPACITOR_DEBUG // "") | tostring | ascii_downcase) as $v | ($v != "true" and $v != "yes" and $v != "1")' "$facts/info.json"
ok "no App Transport Security exception" 'has("NSAppTransportSecurity") | not' "$facts/info.json"
ok "Capacitor logging is none" '.loggingBehavior == "none"' "$app/capacitor.config.json"
ok "the app loads no server url" '(.server // {}) | (has("url") or .cleartext == true) | not' "$app/capacitor.config.json"
ok "the app allows no navigation" '((.server // {}).allowNavigation // []) == []' "$app/capacitor.config.json"
ok "the web view cannot be inspected" '((.ios // {}).webContentsDebuggingEnabled // false) == false' "$app/capacitor.config.json"

# The binary: the debug pairing hook is compiled out, and the bridge gate is compiled in.
executable=$(jq -r .CFBundleExecutable "$facts/info.json")
if [[ -f "$app/$executable" ]]; then
    ! grep -aq 'HIDE_DEBUG_PAIR_LINK' "$app/$executable" || fail "the debug pairing hook is in the binary"
    grep -aq 'BridgeGate' "$app/$executable" || fail "the bridge gate is not in the binary"
else
    fail "the app has no executable named $executable"
fi

# Background modes: the app wakes only for a notification, and for nothing else.
ok "the only background mode is remote-notification" ".UIBackgroundModes == $(jq -c .backgroundModes "$expect")" "$facts/info.json"

# The relay protocol library the app links for a reply sent from a notification: it must come from the sources of this
# checkout, so a library built from other code (or built by hand) cannot ship under this commit's name.
if [[ -z "$relay_digest" ]]; then
    fail "no relay source digest was given to check the relay library against"
elif [[ -f "$app/$executable" ]]; then
    marks=$(grep -aoE 'hide-relay-ios-source:[0-9a-z]+' "$app/$executable" | LC_ALL=C sort -u || true)
    [[ "$marks" == "hide-relay-ios-source:$relay_digest" ]] || fail "the relay library does not come from this checkout's sources (the app says: ${marks:-nothing}; the checkout: $relay_digest)"
fi

# Each app extension: exactly the ones the release names, each signed for its own bundle id with the entitlements it is
# allowed, carrying a privacy manifest, and (for the notification extension) holding no code that can reach the network.
declared=$(jq -r '(.appExtensions // []) | map(.name) | sort | .[]' "$expect")
found=$( (cd "$app/PlugIns" 2> /dev/null && ls -d ./*.appex 2> /dev/null | sed 's|^\./||') | LC_ALL=C sort || true)
[[ "$declared" == "$found" ]] || fail "the app embeds different extensions than release-expect.json names (embedded: ${found:-none}; named: ${declared:-none})"
while IFS= read -r name; do
    [[ -n "$name" ]] || continue
    appex="$app/PlugIns/$name"
    af="$facts/appex/$name"
    entry=$(jq -c --arg n "$name" '.appExtensions[] | select(.name == $n)' "$expect")
    appex_bundle=$(jq -r .bundleId <<< "$entry")
    point=$(jq -r .extensionPoint <<< "$entry")
    for file in "$af/info.json" "$af/privacy.json" "$af/entitlements.json"; do
        [[ -s "$file" ]] || { fail "$name: $file is missing or empty"; continue 2; }
    done
    ok "$name: bundle id is $appex_bundle" ".CFBundleIdentifier == \"$appex_bundle\"" "$af/info.json"
    ok "$name: version is $version" ".CFBundleShortVersionString == \"$version\"" "$af/info.json"
    ok "$name: build number is $build" ".CFBundleVersion == \"$build\"" "$af/info.json"
    ok "$name: extension point is $point" ".NSExtension.NSExtensionPointIdentifier == \"$point\"" "$af/info.json"
    ok "$name: minimum iOS is $min_os" ".MinimumOSVersion == \"$min_os\"" "$af/info.json"
    ok "$name: no App Transport Security exception" 'has("NSAppTransportSecurity") | not' "$af/info.json"
    ok "$name: no background modes" 'has("UIBackgroundModes") | not' "$af/info.json"
    [[ -f "$appex/PrivacyInfo.xcprivacy" ]] || fail "$name: has no PrivacyInfo.xcprivacy"
    ok "$name: the manifest says no tracking and collects nothing" '.NSPrivacyTracking == false and (.NSPrivacyTrackingDomains // []) == [] and (.NSPrivacyCollectedDataTypes // []) == []' "$af/privacy.json"
    ok "$name: every declared API has a reason" '(.NSPrivacyAccessedAPITypes // []) | all(.NSPrivacyAccessedAPITypeReasons | length > 0)' "$af/privacy.json"
    if jq -e --argjson want "$entry" '. == $want.entitlements' "$af/entitlements.json" > /dev/null 2>&1; then :; else
        fail "$name: entitlements differ from release-expect.json (got: $(jq -c -S . "$af/entitlements.json" 2> /dev/null | cut -c1-300))"
    fi
    appex_exec=$(jq -r .CFBundleExecutable "$af/info.json")
    if [[ -f "$appex/$appex_exec" ]]; then
        while IFS= read -r word; do
            [[ -n "$word" ]] || continue
            ! grep -aq -- "$word" "$appex/$appex_exec" || fail "$name: its binary holds networking code ($word)"
        done < <(jq -r '.forbiddenBinaryStrings[]?' <<< "$entry")
    else
        fail "$name: has no executable named $appex_exec"
    fi
done <<< "$declared"

# The privacy manifest: no tracking, nothing collected, and a reason for every required-reason API the app declares.
[[ -f "$app/PrivacyInfo.xcprivacy" ]] || fail "the app has no PrivacyInfo.xcprivacy"
ok "the manifest says no tracking and collects nothing" '.NSPrivacyTracking == false and (.NSPrivacyTrackingDomains // []) == [] and (.NSPrivacyCollectedDataTypes // []) == []' "$facts/privacy.json"
ok "every declared API has a reason" '(.NSPrivacyAccessedAPITypes // []) | all(.NSPrivacyAccessedAPITypeReasons | length > 0)' "$facts/privacy.json"

# Signed for the store: the entitlements are exactly the list, from an App Store profile.
if jq -e --slurpfile want "$expect" '. == $want[0].entitlements' "$facts/entitlements.json" > /dev/null 2>&1; then :; else
    fail "entitlements differ from release-expect.json (got: $(jq -c -S . "$facts/entitlements.json" 2> /dev/null | cut -c1-300))"
fi
ok "the profile is an App Store profile (no device list, not enterprise)" '(has("ProvisionedDevices") | not) and (.ProvisionsAllDevices // false) == false' "$facts/profile.json"
ok "the profile belongs to team $team" ".TeamIdentifier == [\"$team\"]" "$facts/profile.json"
# Apple lists every platform a profile covers (iOS, xrOS, visionOS); iOS must be one of them.
ok "the profile is for iOS" '(.Platform // []) | index("iOS") != null' "$facts/profile.json"

# The web bundle: every file in public/ is in the manifest, with its digest, and none is missing.
if [[ -d "$app/public" ]]; then
    actual=$(cd "$app/public" && find . -type f | sed 's|^\./||' | LC_ALL=C sort)
    wanted=$(jq -r '.files | keys[]' "$manifest" | LC_ALL=C sort)
    [[ "$actual" == "$wanted" ]] || fail "public/ holds different files than the bundle manifest ($(diff <(printf '%s\n' "$wanted") <(printf '%s\n' "$actual") | grep -c '^[<>]') differ)"
    while IFS= read -r path; do
        [[ -n "$path" && -f "$app/public/$path" ]] || continue
        [[ "$(sha256_of "$app/public/$path")" == "$(jq -r --arg p "$path" '.files[$p]' "$manifest")" ]] || fail "public/$path differs from the bundle manifest"
    done <<< "$wanted"
else
    fail "the app has no public/ folder"
fi

if (( failures > 0 )); then
    printf '%s release check(s) failed\n' "$failures" >&2
    exit 1
fi
printf 'release checks passed for %s (%s)\n' "$version" "$build"

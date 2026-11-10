#!/usr/bin/env bash
# Runs on the build machine (piped by build-ios.sh release): archive, sign, check and upload one release build.
#
# Usage: mini-release.sh release <branch slug> <marketing version> <build number>
#        mini-release.sh status <branch slug> <delivery id>
#   The project and its release inputs are already in ~/hide-ios-build/<slug>/ (ios/ and release-inputs/).
#   `status` reads what Apple says about one upload (the delivery id `release` printed) and changes nothing.
#
# Secrets: the three keys of release-env.json, from the environment or from the file that registry names, read as
# plain KEY=VALUE lines and never run as a script. Every key is checked before anything is built, and a problem names
# the key and the kind of problem, never a value. Nothing here runs under `set -x`, and every line xcodebuild and altool
# print goes through `redact` before it reaches the log or the terminal; at the end the log is searched for each value
# and the command fails if one is there.
#
# Signing: the archive is built unsigned and `-exportArchive` signs it, with the cloud-managed Apple Distribution
# certificate and an App Store profile that Xcode makes from the API key, so no distribution private key exists on this
# machine and no keychain is read or changed. The signed ipa is checked, and the same ipa is validated and uploaded.
#
# Everything this starts it ends: the archive, the export, the unpacked ipa and the derived data are removed on every
# way out, including an interrupt or a dropped ssh connection. Only the redacted log stays, in the branch's folder.
set -euo pipefail
umask 077

mode=$1 slug=$2
export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
base=$HOME/hide-ios-build/$slug
inputs=$base/release-inputs
work=$base/release
log=$base/release.log
project=$base/ios/App
packages=$base/SourcePackages
min_free_gb=15
registry=$inputs/release-env.json
expect=$inputs/release-expect.json

step() { printf '\n== %s\n' "$1"; }
die() { printf 'release stopped: %s\n' "$1" >&2; exit 1; }

# ---- secrets -------------------------------------------------------------------------------------------------------

problems=()
load_secrets() {
    local file line key value req shape
    file=$(jq -r .file "$registry")
    file=${file/#\~/$HOME}
    if [[ -f "$file" ]]; then
        [[ "$(stat -f %Lp "$file")" =~ ^[46]00$ && "$(stat -f %u "$file")" == "$(id -u)" ]] \
            || problems+=("$file must be owned by this account and readable by it alone (mode 600)")
        while IFS= read -r line || [[ -n "$line" ]]; do
            [[ -z "$line" || "$line" == \#* ]] && continue
            key=${line%%=*}
            if jq -e --arg k "$key" '.keys[$k]' "$registry" > /dev/null; then export "$key=${line#*=}"; else problems+=("$file has a key the registry does not know: $key"); fi
        done < "$file"
    fi
    for key in $(jq -r '.keys | keys[]' "$registry"); do
        value=${!key:-}
        req=$(jq -r --arg k "$key" '.keys[$k].requirement' "$registry")
        if [[ -z "$value" ]]; then
            [[ "$req" == required ]] && problems+=("$key is missing")
            continue
        fi
        shape=$(jq -r --arg k "$key" '.keys[$k].shape' "$registry")
        [[ "$value" =~ $shape ]] || problems+=("$key is not shaped as release-env.json says")
    done
    if (( ${#problems[@]} == 0 )); then
        [[ "$(basename "$HIDE_IOS_ASC_KEY_PATH")" == "AuthKey_${HIDE_IOS_ASC_KEY_ID}.p8" ]] || problems+=("HIDE_IOS_ASC_KEY_PATH does not name the key HIDE_IOS_ASC_KEY_ID")
        [[ -f "$HIDE_IOS_ASC_KEY_PATH" ]] || problems+=("HIDE_IOS_ASC_KEY_PATH names no file here")
        if [[ -f "$HIDE_IOS_ASC_KEY_PATH" ]]; then
            [[ "$(stat -f %Lp "$HIDE_IOS_ASC_KEY_PATH")" =~ ^[46]00$ && "$(stat -f %u "$HIDE_IOS_ASC_KEY_PATH")" == "$(id -u)" ]] \
                || problems+=("HIDE_IOS_ASC_KEY_PATH names a file other accounts can read")
        fi
    fi
    if (( ${#problems[@]} > 0 )); then
        printf 'release stopped: the signing secrets are not usable (values are never printed; mobile/release.env.example shows the shape):\n' >&2
        printf '  - %s\n' "${problems[@]}" >&2
        exit 1
    fi
}

# A sed expression that replaces every secret value with the name of its key.
escape() { printf '%s' "$1" | sed 's/[][\.*^$|/&]/\\&/g'; }
redact() {
    sed -e "s|$(escape "$HIDE_IOS_ASC_KEY_PATH")|<HIDE_IOS_ASC_KEY_PATH>|g" \
        -e "s|$(escape "$HIDE_IOS_ASC_KEY_ID")|<HIDE_IOS_ASC_KEY_ID>|g" \
        -e "s|$(escape "$HIDE_IOS_ASC_ISSUER_ID")|<HIDE_IOS_ASC_ISSUER_ID>|g"
}

# `logged <name> <command...>`: the command's output, redacted, goes to the log; the terminal gets the step and, when
# it fails, the end of its log.
logged() {
    local name=$1 status=0
    shift
    printf '\n== %s\n' "$name" >> "$log"
    printf '== %s\n' "$name"
    "$@" 2>&1 | redact >> "$log" || status=$?
    if (( status != 0 )); then
        printf 'release stopped at "%s" (status %s); the end of its log:\n' "$name" "$status" >&2
        tail -n 40 "$log" >&2
        exit 1
    fi
}

# The log must hold none of the secrets: the three values, and a line of the key file itself.
log_is_clean() {
    local needle
    [[ -f "$log" ]] || return 0
    for needle in "$HIDE_IOS_ASC_KEY_PATH" "$HIDE_IOS_ASC_KEY_ID" "$HIDE_IOS_ASC_ISSUER_ID" "$(sed -n 2p "$HIDE_IOS_ASC_KEY_PATH")"; do
        [[ -z "$needle" ]] && continue
        ! grep -qF -- "$needle" "$log" || return 1
    done
}

finished=0
cleanup() {
    local code=$?
    rm -rf "$work"
    if [[ -n "${HIDE_IOS_ASC_KEY_PATH:-}" ]] && ! log_is_clean; then
        rm -f "$log"
        printf 'release stopped: a secret value reached the log, which is deleted; this is a bug in the redaction\n' >&2
        code=1
    fi
    (( finished == 1 && code == 0 )) || printf 'release did not finish (status %s)\n' "$code" >&2
    exit "$code"
}
trap cleanup EXIT
trap 'exit 143' TERM HUP INT

# ---- start ---------------------------------------------------------------------------------------------------------

if [[ "$mode" == status ]]; then
    delivery=${3:-}
    [[ "$delivery" =~ ^[0-9a-f-]{36}$ ]] || die "status needs the delivery id the release printed"
    [[ -f "$registry" ]] || die "$registry is missing: run the release command once from this branch first"
    load_secrets
    xcrun altool --build-status --delivery-id "$delivery" --api-key "$HIDE_IOS_ASC_KEY_ID" --api-issuer "$HIDE_IOS_ASC_ISSUER_ID" \
        --p8-file-path "$HIDE_IOS_ASC_KEY_PATH" 2>&1 | redact
    finished=1
    exit 0
fi
[[ "$mode" == release ]] || die "unknown mode '$mode'"
version=$3 build=$4
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "version '$version' is not three numbers"
[[ "$build" =~ ^[0-9]{1,18}$ ]] || die "build number '$build' is not a number"
for file in "$registry" "$expect" "$inputs/check-release-app.sh" "$inputs/bundle-manifest.json" "$inputs/relay-source.txt"; do
    [[ -f "$file" ]] || die "$file was not copied to this machine"
done
free_gb=$(df -g / | awk 'NR==2 {print $4}')
(( free_gb >= min_free_gb )) || die "this machine has ${free_gb} GB free and a release build needs it to stay above $min_free_gb GB"

load_secrets
team=$(jq -r .teamId "$expect")
rm -rf "$work"
mkdir -p "$work/facts"
: > "$log"
printf 'release build %s (%s), team %s\n' "$version" "$build" "$team"

auth=(-authenticationKeyPath "$HIDE_IOS_ASC_KEY_PATH" -authenticationKeyID "$HIDE_IOS_ASC_KEY_ID" -authenticationKeyIssuerID "$HIDE_IOS_ASC_ISSUER_ID")
altool_auth=(--api-key "$HIDE_IOS_ASC_KEY_ID" --api-issuer "$HIDE_IOS_ASC_ISSUER_ID" --p8-file-path "$HIDE_IOS_ASC_KEY_PATH")

# The extensions are signed before the app that holds them (Xcode's export does that order), and `--deep --strict` below
# fails when an extension's signature is missing or was made after the app's. The relay library's source digest is read
# from the release inputs build-ios.sh recomputed from the checkout.
relay_digest=$(cat "$inputs/relay-source.txt")
[[ "$relay_digest" =~ ^[0-9a-f]{64}$ ]] || die "release-inputs/relay-source.txt does not hold a relay source digest"

# ---- archive, export, check ----------------------------------------------------------------------------------------

logged "archive" xcodebuild archive -project "$project/App.xcodeproj" -scheme App -configuration Release \
    -destination 'generic/platform=iOS' -archivePath "$work/App.xcarchive" -derivedDataPath "$work/DerivedData" \
    -clonedSourcePackagesDirPath "$packages" CODE_SIGNING_ALLOWED=NO \
    MARKETING_VERSION="$version" CURRENT_PROJECT_VERSION="$build"

# An archive made with signing off carries none of the project's entitlements, and the export only keeps what the archive
# holds, so the entitlements file the project names is put into the archived app first (an ad hoc signature, which the
# export replaces with the distribution one; no certificate or keychain is involved).
entitlements=$project/App/App.entitlements
[[ -f "$entitlements" ]] || die "the project's App.entitlements is missing"
archived=$work/App.xcarchive/Products/Applications/App.app
logged "embed entitlements" codesign --force --sign - --entitlements "$entitlements" --timestamp=none "$archived"

cat > "$work/ExportOptions.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>method</key><string>app-store-connect</string>
  <key>destination</key><string>export</string>
  <key>signingStyle</key><string>automatic</string>
  <key>teamID</key><string>$team</string>
  <key>manageAppVersionAndBuildNumber</key><false/>
</dict></plist>
PLIST

logged "export and sign" xcodebuild -exportArchive -archivePath "$work/App.xcarchive" -exportPath "$work/export" \
    -exportOptionsPlist "$work/ExportOptions.plist" -allowProvisioningUpdates "${auth[@]}"

ipa=$work/export/App.ipa
[[ -f "$ipa" ]] || die "the export made no App.ipa"
unzip -q "$ipa" -d "$work/ipa"
app=$work/ipa/Payload/App.app
facts=$work/facts

step "read the signed app"
plutil -convert json -o "$facts/info.json" "$app/Info.plist"
if [[ -f "$app/PrivacyInfo.xcprivacy" ]]; then plutil -convert json -o "$facts/privacy.json" "$app/PrivacyInfo.xcprivacy"; else printf '{}' > "$facts/privacy.json"; fi
codesign -d --entitlements :- "$app" > "$work/entitlements.plist" 2> /dev/null
plutil -convert json -o "$facts/entitlements.json" "$work/entitlements.plist"
security cms -D -i "$app/embedded.mobileprovision" > "$work/profile.plist"
# Dates and certificates cannot be written as JSON, and no check reads them.
for key in DeveloperCertificates DER-Encoded-Profile CreationDate ExpirationDate; do plutil -remove "$key" "$work/profile.plist" 2> /dev/null || true; done
plutil -convert json -o "$facts/profile.json" "$work/profile.plist"

# Each app extension the same way: its own properties, privacy manifest and entitlements, under facts/appex/<name>/.
for appex in "$app"/PlugIns/*.appex; do
    [[ -d "$appex" ]] || continue
    name=$(basename "$appex")
    mkdir -p "$facts/appex/$name"
    plutil -convert json -o "$facts/appex/$name/info.json" "$appex/Info.plist"
    if [[ -f "$appex/PrivacyInfo.xcprivacy" ]]; then plutil -convert json -o "$facts/appex/$name/privacy.json" "$appex/PrivacyInfo.xcprivacy"; else printf '{}' > "$facts/appex/$name/privacy.json"; fi
    codesign -d --entitlements :- "$appex" > "$work/appex-entitlements.plist" 2> /dev/null
    plutil -convert json -o "$facts/appex/$name/entitlements.json" "$work/appex-entitlements.plist"
done

step "verify the signature"
codesign --verify --strict --deep "$app"
signature=$(codesign -dvv "$app" 2>&1)
grep -q '^Authority=Apple Distribution: ' <<< "$signature" || die "the app is not signed with an Apple Distribution certificate (authorities: $(grep '^Authority=' <<< "$signature" | tr '\n' ';'))"

step "release checks"
bash "$inputs/check-release-app.sh" "$app" "$facts" "$expect" "$inputs/bundle-manifest.json" "$version" "$build" "$relay_digest"

# ---- upload --------------------------------------------------------------------------------------------------------

logged "validate" xcrun altool --validate-app -f "$ipa" --type ios "${altool_auth[@]}"
logged "upload" xcrun altool --upload-app -f "$ipa" --type ios "${altool_auth[@]}"
delivery=$(sed -n 's/^Delivery UUID: //p' "$log" | tail -n 1)
[[ "$delivery" =~ ^[0-9a-f-]{36}$ ]] || die "the upload printed no delivery id"

# Apple processes the build after the upload; this reads its state once and does not wait for it.
step "build status"
{ xcrun altool --build-status --delivery-id "$delivery" "${altool_auth[@]}" 2>&1 || true; } | redact | tee -a "$log" | tail -n 8
printf 'uploaded: %s (%s), delivery id %s. Apple now processes it; `build-ios.sh status %s` reads its state again. Log: %s\n' "$version" "$build" "$delivery" "$delivery" "$log"
df -h / | tail -1 | awk '{printf "build machine free disk: %s of %s\n", $4, $2}'
finished=1

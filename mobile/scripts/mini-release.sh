#!/usr/bin/env bash
# Runs on the build machine (piped by build-ios.sh release): archive, sign, check and upload one release build.
#
# Usage: mini-release.sh release <branch slug> <marketing version> <build number> <commit> <again: 0|1>
#        mini-release.sh status <branch slug> <delivery id>
#        mini-release.sh uploads <branch slug>
#   The project and its release inputs are already in ~/hide-ios-build/<slug>/ (ios/ and release-inputs/).
#   `status` reads what Apple says about one upload (the delivery id `release` printed) and changes nothing here.
#   `uploads` prints the branch's upload record: one line per upload that started, with its delivery id once it ended.
#   HIDE_IOS_NO_UPLOAD=1 runs `release` through Apple's validation and stops before the upload.
#
# One release of a branch runs at a time: the branch folder holds a lock (release.lock) with the running command's pid.
# A second release, or a build that would copy a new project under a running archive, is refused naming the pid; a lock
# whose pid is gone is taken over. `status` and `uploads` take no lock and delete nothing.
#
# Secrets: the three keys of release-env.json, from the environment or from the file that registry names, read as
# plain KEY=VALUE lines and never run as a script. Every key is checked before anything is built, and a problem names
# the key and the kind of problem, never a value. Nothing here runs under `set -x`, and every line xcodebuild and altool
# print goes through `redact` before it reaches the log or the terminal. Before the end, and before a failed step shows
# the tail of its log, the log is searched for the three values, for every line of the key file and for the shape of a
# token, and the command fails and deletes the log if one is there.
#
# xcodebuild writes its distribution logs (which hold a signed token and the key's ids) into the account's temporary
# folder, so every tool here runs with TMPDIR inside the work folder and they go with it on every way out; a copy that
# still appears in the account's own temporary folder is deleted by name and fails the run.
#
# Signing: the archive is built unsigned and `-exportArchive` signs it, with the cloud-managed Apple Distribution
# certificate and an App Store profile that Xcode makes from the API key, so no distribution private key exists on this
# machine and no keychain is read or changed. The signed ipa is checked, and the same ipa is validated and uploaded.
#
# Every long step has a time bound (a hung tool is ended, not waited for). Everything this starts it ends: the archive,
# the export, the unpacked ipa, the derived data, the temporary folder and the lock are removed on every way out,
# including an interrupt or a dropped ssh connection. Only the redacted log and the upload record stay in the branch folder.
set -Eeuo pipefail
umask 077

mode=${1:?mode} slug=${2:?branch slug}
export DEVELOPER_DIR=${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}
base=$HOME/hide-ios-build/$slug
inputs=$base/release-inputs
work=$base/release
log=$base/release.log
lock=$base/release.lock
records=$base/uploads.log
project=$base/ios/App
packages=$base/SourcePackages
min_free_gb=15
registry=$inputs/release-env.json
expect=$inputs/release-expect.json
needles=$work/needles

# Seconds each long step may take (the whole run took about 6 minutes on 2026-11-10); a test sets the one cap itself.
limit_archive=${HIDE_IOS_TEST_STEP_LIMIT:-1200} limit_export=${HIDE_IOS_TEST_STEP_LIMIT:-900}
limit_validate=${HIDE_IOS_TEST_STEP_LIMIT:-900} limit_upload=${HIDE_IOS_TEST_STEP_LIMIT:-1800}

current_step=start
step() { current_step=$1; printf '\n== %s\n' "$1"; }
die() { printf 'release stopped: %s\n' "$1" >&2; exit 1; }
trap 'printf "release stopped during \"%s\"\n" "$current_step" >&2' ERR

# ---- the upload record ---------------------------------------------------------------------------------------------

if [[ "$mode" == uploads ]]; then
    if [[ -s "$records" ]]; then cat "$records"; else printf 'no upload has started from this branch folder\n'; fi
    exit 0
fi

# ---- secrets -------------------------------------------------------------------------------------------------------

problems=()
mode_ok() { [[ "$(stat -f %Lp "$1")" =~ ^[46]00$ && "$(stat -f %u "$1")" == "$(id -u)" ]]; }
load_secrets() {
    local file line key value req shape
    file=$(jq -r .file "$registry")
    file=${file/#\~/$HOME}
    if [[ -f "$file" ]]; then
        mode_ok "$file" || problems+=("$file must be owned by this account and readable by it alone (mode 600)")
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
            mode_ok "$HIDE_IOS_ASC_KEY_PATH" || problems+=("HIDE_IOS_ASC_KEY_PATH names a file other accounts can read")
        fi
    fi
    if (( ${#problems[@]} > 0 )); then
        printf 'release stopped: the signing secrets are not usable (values are never printed; mobile/release.env.example shows the shape):\n' >&2
        printf '  - %s\n' "${problems[@]}" >&2
        exit 1
    fi
}

# A sed expression that replaces every secret value with the name of its key, and a token with its shape.
escape() { printf '%s' "$1" | sed 's/[][\.*^$|/&]/\\&/g'; }
redact() {
    sed -e "s|$(escape "$HIDE_IOS_ASC_KEY_PATH")|<HIDE_IOS_ASC_KEY_PATH>|g" \
        -e "s|$(escape "$HIDE_IOS_ASC_KEY_ID")|<HIDE_IOS_ASC_KEY_ID>|g" \
        -e "s|$(escape "$HIDE_IOS_ASC_ISSUER_ID")|<HIDE_IOS_ASC_ISSUER_ID>|g" |
        sed -E 's/eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]*/<token>/g'
}

# What the log must not hold: the three values and every line of the key file, one per line (never an empty line, which
# would match everything). The file lives in the work folder and goes with it.
write_needles() {
    {
        printf '%s\n' "$HIDE_IOS_ASC_KEY_PATH" "$HIDE_IOS_ASC_KEY_ID" "$HIDE_IOS_ASC_ISSUER_ID"
        grep -v -e '^-----' -e '^[[:space:]]*$' "$HIDE_IOS_ASC_KEY_PATH"
    } > "$needles"
}
log_is_clean() {
    [[ -f "$log" && -s "$needles" ]] || return 0
    ! grep -qF -f "$needles" "$log" && ! grep -qE 'eyJ[A-Za-z0-9_-]{10,}\.' "$log"
}

# `timed <seconds> <command...>`: the command, ended if it outlives the bound (status 124).
timed() {
    local limit=$1 pid dog rc=0
    shift
    "$@" &
    pid=$!
    # The watchdog holds no pipe of the step: its output goes nowhere, so a finished step is not kept open by its sleep.
    ( sleep "$limit" || exit 0; : > "$work/timed-out"; pkill -TERM -P "$pid" 2> /dev/null; kill -TERM "$pid" 2> /dev/null; sleep 5
      pkill -KILL -P "$pid" 2> /dev/null; kill -KILL "$pid" 2> /dev/null ) > /dev/null 2>&1 &
    dog=$!
    wait "$pid" || rc=$?
    pkill -P "$dog" 2> /dev/null || true
    kill "$dog" 2> /dev/null || true
    wait "$dog" 2> /dev/null || true
    [[ -e "$work/timed-out" ]] && return 124
    return "$rc"
}

# `logged <seconds> <name> <command...>`: the command's output, redacted, goes to the log; the terminal gets the step and,
# when it fails, the end of its log - once the log has been searched for a secret, never before.
logged() {
    local limit=$1 name=$2 status=0
    shift 2
    current_step=$name
    printf '\n== %s\n' "$name" >> "$log"
    printf '== %s\n' "$name"
    timed "$limit" "$@" 2>&1 | redact >> "$log" || status=${PIPESTATUS[0]}
    [[ -e "$work/timed-out" ]] && status=124
    if (( status != 0 )); then
        if (( status == 124 )); then printf 'release stopped at "%s": it did not finish in %s seconds\n' "$name" "$limit" >&2
        else printf 'release stopped at "%s" (status %s)\n' "$name" "$status" >&2; fi
        if log_is_clean; then
            printf 'the end of its log:\n' >&2
            tail -n 40 "$log" >&2
        fi
        exit 1
    fi
}

# xcodebuild ignored the temporary folder if one of its logs is in the account's own: delete it by name, and fail.
stray_logs() {
    local found=0 path
    while IFS= read -r path; do
        [[ -n "$path" ]] || continue
        rm -rf "$path"
        printf 'release stopped: xcodebuild wrote its distribution logs outside the work folder (%s), deleted\n' "$(basename "$path")" >&2
        found=1
    done < <(find "$account_tmp" -maxdepth 1 \( -name 'App_*.xcdistributionlogs' -o -name 'XcodeDistPipeline.*' \) -newer "$work/started" 2> /dev/null)
    return "$found"
}

owns_lock=0 owns_work=0 finished=0
cleanup() {
    local code=$?
    trap - ERR
    if (( owns_work == 1 )); then
        if [[ -n "${HIDE_IOS_ASC_KEY_PATH:-}" ]] && ! log_is_clean; then
            rm -f "$log"
            printf 'release stopped: a secret value reached the log, which is deleted; this is a bug in the redaction\n' >&2
            code=1
        fi
        # An upload that started and never recorded its end: the caller must be able to read that it may have gone.
        if (( upload_started == 1 && upload_ended == 0 )); then
            printf 'the upload may have gone without its delivery id (its record says started); read `build-ios.sh uploads` and look in App Store Connect before running again\n' >&2
        fi
        rm -rf "$work"
    fi
    (( owns_lock == 1 )) && rm -rf "$lock"
    (( finished == 1 && code == 0 )) || printf 'release did not finish (status %s)\n' "$code" >&2
    exit "$code"
}
upload_started=0 upload_ended=0 version=- build=- commit=-
trap cleanup EXIT
trap 'exit 143' TERM HUP INT

# ---- status --------------------------------------------------------------------------------------------------------

if [[ "$mode" == status ]]; then
    delivery=${3:-}
    [[ "$delivery" =~ ^[0-9a-f-]{36}$ ]] || die "status needs the delivery id the release printed"
    [[ -f "$registry" ]] || die "$registry is missing: run the release command once from this branch first"
    load_secrets
    mkdir -p "$base/status-tmp.$$"
    owns_work=1 work=$base/status-tmp.$$ needles=$work/needles
    write_needles
    # Only its own scratch folder is removed, never the work folder of a release that may be running.
    TMPDIR=$work xcrun altool --build-status --delivery-id "$delivery" --api-key "$HIDE_IOS_ASC_KEY_ID" --api-issuer "$HIDE_IOS_ASC_ISSUER_ID" \
        --p8-file-path "$HIDE_IOS_ASC_KEY_PATH" 2>&1 | redact
    finished=1
    exit 0
fi

# ---- release: lock, record, disk, secrets --------------------------------------------------------------------------

[[ "$mode" == release ]] || die "unknown mode '$mode'"
version=${3:?version} build=${4:?build number} commit=${5:?commit} again=${6:-0}
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "version '$version' is not three numbers"
[[ "$build" =~ ^[0-9]{1,18}$ ]] || die "build number '$build' is not a number"
[[ "$commit" =~ ^[0-9a-f]{40}$ ]] || die "commit '$commit' is not a full commit id"
for file in "$registry" "$expect" "$inputs/check-release-app.sh" "$inputs/bundle-manifest.json"; do
    [[ -f "$file" ]] || die "$file was not copied to this machine"
done

if ! mkdir "$lock" 2> /dev/null; then
    holder=$(cat "$lock/pid" 2> /dev/null || true)
    if [[ "$holder" =~ ^[0-9]+$ ]] && kill -0 "$holder" 2> /dev/null; then
        die "another release of this branch is running here (pid $holder, since $(cat "$lock/since" 2> /dev/null || echo unknown))"
    fi
    rm -rf "$lock"
    mkdir "$lock" 2> /dev/null || die "another release of this branch took the lock first"
fi
owns_lock=1
printf '%s\n' "$$" > "$lock/pid"
date -u +%Y-%m-%dT%H:%M:%SZ > "$lock/since"

# The same commit is not uploaded twice by accident: after a dropped connection the caller cannot know whether the first
# upload went, so a commit with a record asks for an explicit request.
if [[ -s "$records" ]] && [[ "$again" != 1 ]]; then
    previous=$(grep -F " $commit " "$records" | tail -n 1 || true)
    if [[ -n "$previous" ]]; then
        printf 'release stopped: this commit already has an upload record (time, commit, version, build, delivery id):\n  %s\n' "$previous" >&2
        printf 'a completed upload of a commit is not repeated; to build and upload it again on purpose run with HIDE_IOS_RELEASE_AGAIN=1\n' >&2
        exit 1
    fi
fi

free_gb=$(df -g / | awk 'NR==2 {print $4}')
(( free_gb >= min_free_gb )) || die "this machine has ${free_gb} GB free and a release build needs it to stay above $min_free_gb GB"

load_secrets
team=$(jq -r .teamId "$expect")
rm -rf "$work"
mkdir -p "$work/facts" "$work/tmp"
owns_work=1
write_needles
: > "$work/started"
: > "$log"
printf 'release build %s (%s), team %s\n' "$version" "$build" "$team"

account_tmp=${TMPDIR:-/tmp}
export TMPDIR=$work/tmp
auth=(-authenticationKeyPath "$HIDE_IOS_ASC_KEY_PATH" -authenticationKeyID "$HIDE_IOS_ASC_KEY_ID" -authenticationKeyIssuerID "$HIDE_IOS_ASC_ISSUER_ID")
altool_auth=(--api-key "$HIDE_IOS_ASC_KEY_ID" --api-issuer "$HIDE_IOS_ASC_ISSUER_ID" --p8-file-path "$HIDE_IOS_ASC_KEY_PATH")

# ---- archive, export, check ----------------------------------------------------------------------------------------

logged "$limit_archive" "archive" xcodebuild archive -project "$project/App.xcodeproj" -scheme App -configuration Release \
    -destination 'generic/platform=iOS' -archivePath "$work/App.xcarchive" -derivedDataPath "$work/DerivedData" \
    -clonedSourcePackagesDirPath "$packages" CODE_SIGNING_ALLOWED=NO \
    MARKETING_VERSION="$version" CURRENT_PROJECT_VERSION="$build"

# An archive made with signing off carries none of the project's entitlements, and the export only keeps what the archive
# holds, so the entitlements file the project names is put into the archived app first (an ad hoc signature, which the
# export replaces with the distribution one; no certificate or keychain is involved).
entitlements=$project/App/App.entitlements
[[ -f "$entitlements" ]] || die "the project's App.entitlements is missing"
archived=$work/App.xcarchive/Products/Applications/App.app
logged 120 "embed entitlements" codesign --force --sign - --entitlements "$entitlements" --timestamp=none "$archived"

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

logged "$limit_export" "export and sign" xcodebuild -exportArchive -archivePath "$work/App.xcarchive" -exportPath "$work/export" \
    -exportOptionsPlist "$work/ExportOptions.plist" -allowProvisioningUpdates "${auth[@]}"
stray_logs

ipa=$work/export/App.ipa
[[ -f "$ipa" ]] || die "the export made no App.ipa"
step "unpack the signed app"
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

step "verify the signature"
codesign --verify --strict --deep "$app"
signature=$(codesign -dvv "$app" 2>&1)
grep -q '^Authority=Apple Distribution: ' <<< "$signature" || die "the app is not signed with an Apple Distribution certificate (authorities: $(grep '^Authority=' <<< "$signature" | tr '\n' ';'))"

step "release checks"
bash "$inputs/check-release-app.sh" "$app" "$facts" "$expect" "$inputs/bundle-manifest.json" "$version" "$build"

# ---- upload --------------------------------------------------------------------------------------------------------

logged "$limit_validate" "validate" xcrun altool --validate-app -f "$ipa" --type ios "${altool_auth[@]}"
stray_logs
if [[ "${HIDE_IOS_NO_UPLOAD:-}" == 1 ]]; then
    printf 'validated %s (%s) and stopped before the upload (HIDE_IOS_NO_UPLOAD=1); nothing was sent to TestFlight\n' "$version" "$build"
    finished=1
    exit 0
fi

# The record is written before the bytes leave: a dropped connection cannot make the upload disappear from it.
printf '%s %s %s %s %s started\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$commit" "$version" "$build" "-" >> "$records"
upload_started=1
logged "$limit_upload" "upload" xcrun altool --upload-app -f "$ipa" --type ios "${altool_auth[@]}"
delivery=$(sed -n 's/^Delivery UUID: //p' "$log" | tail -n 1)
[[ "$delivery" =~ ^[0-9a-f-]{36}$ ]] || die "the upload printed no delivery id"
printf '%s %s %s %s %s uploaded\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$commit" "$version" "$build" "$delivery" >> "$records"
upload_ended=1

# Apple processes the build after the upload; this reads its state once and does not wait for it.
step "build status"
{ xcrun altool --build-status --delivery-id "$delivery" "${altool_auth[@]}" 2>&1 || true; } | redact | tee -a "$log" | tail -n 8
printf 'uploaded: %s (%s), delivery id %s. Apple now processes it; `build-ios.sh status %s` reads its state again. Log: %s\n' "$version" "$build" "$delivery" "$delivery" "$log"
df -h / | tail -1 | awk '{printf "build machine free disk: %s of %s\n", $4, $2}'
finished=1

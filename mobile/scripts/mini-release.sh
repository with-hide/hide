#!/usr/bin/env bash
# Runs on the build machine (piped by build-ios.sh): archive, sign, check and upload one release build.
#
# Usage: mini-release.sh release <branch slug> <marketing version> <build number> <commit> <again: 0|1>
#        mini-release.sh status <branch slug> <delivery id>
#        mini-release.sh uploads <branch slug>
#        mini-release.sh busy <branch slug> <branch|machine>
#   The project and its release inputs are already in ~/hide-ios-build/<slug>/ (ios/ and release-inputs/).
#   `status` reads what Apple says about one upload (the delivery id `release` printed) and changes nothing here.
#   `uploads` prints the machine's upload record: one line per upload that started, with its delivery id once it ended.
#   `busy` exits 1, naming it, while a release holds the machine (`machine`) or a release of this branch does (`branch`);
#   build-ios.sh asks it before a build copies a project, a clean removes a folder or a release starts.
#   HIDE_IOS_NO_UPLOAD=1 runs `release` through Apple's validation and stops before the upload.
#
# Secrets: the three keys of release-env.json, from the environment or from the file that registry names, read as
# plain KEY=VALUE lines and never run as a script. Every key is checked before anything is built, and a problem names
# the key and the kind of problem, never a value. The key file is read by Apple's tools from its path and by nothing
# here: this script checks its name, owner and mode, and never opens, copies or passes its body. The ids reach Apple's
# tools as arguments (they are not the key); every pattern this script searches or replaces them with travels in a
# pipe, never in a file or an argument. Every line the tools print goes through `redact` before the log or the terminal.
#
# What a run owns is listed once, in "What a release owns" below, and `cleanup` removes exactly that on every way out:
# a normal end, a failed step, TERM, HUP or INT, and a closed standard output (a dropped ssh connection: SIGPIPE is
# ignored, so the next write fails and the script ends through its cleanup, which writes nothing it depends on).
set -Eeuo pipefail
umask 077
trap '' PIPE

mode=${1:?mode} slug=${2:?branch slug}
export DEVELOPER_DIR=${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}
root=$HOME/hide-ios-build
base=$root/$slug
inputs=$base/release-inputs
project=$base/ios/App
packages=$base/SourcePackages
registry=$inputs/release-env.json
expect=$inputs/release-expect.json
# A whole run used about 1 GB of the disk (24 GB to 23 GB free on 2026-11-10: the branch folder's 0.66 GB of build state
# and the release work folder); the floor is a few times that, only so a run cannot fill the disk under the machine's
# other work. A run that runs out of space anyway fails at the step that ran out.
min_free_gb=5
# Machine-wide, because what they guard is: the account's temporary folder, the disk floor, the Apple account and the
# build numbers. Neither is ever deleted (deleting a lock file others may open would let two runs hold two locks).
lock=$root/.release.lock
records=$root/uploads.log

# Seconds each long step may take (the whole run took about 6 minutes on 2026-11-10); a test shortens the upload's alone.
limit_archive=1200 limit_export=900 limit_validate=900 limit_upload=${HIDE_IOS_TEST_UPLOAD_LIMIT:-1800}

current_step=start
step() { current_step=$1; printf '\n== %s\n' "$1"; }
die() { printf 'release stopped: %s\n' "$1" >&2; exit 1; }
# Cleanup's own messages: a failed write (a closed terminal) is ignored, so it can never end the cleanup.
say() { printf '%s\n' "$1" >&2 2> /dev/null || true; }
trap 'printf "release stopped during \"%s\"\n" "$current_step" >&2 2> /dev/null || true' ERR

# ---- the machine lock ----------------------------------------------------------------------------------------------
# Held by the kernel (flock) on file descriptor 9 of this shell, so it ends when the shell ends, however it ends; a pid
# is never asked whether it is alive. The file's text (pid, start time, branch) only names the holder for a person.
# Every step run_step starts gets 9>&-, so no tool that outlives the run keeps the lock; the short commands between steps
# end before the shell does.

# Status 75 is "another holds it", and only that: any other failure (no perl, no Fcntl, a file that will not open) is
# named as what it is, never read as a running release.
lock_holder() { cat "$lock" 2> /dev/null || true; }
lock_is_held() {
    local status=0
    [[ -f "$lock" ]] || return 1
    perl -MFcntl=:flock -e 'open(my $f, "<", $ARGV[0]) or die "open: $!\n"; flock($f, LOCK_EX | LOCK_NB) or exit 75' -- "$lock" || status=$?
    (( status == 0 )) && return 1
    (( status == 75 )) && return 0
    die "the release lock $lock could not be read (perl status $status)"
}
take_lock() {
    local status=0
    mkdir -p "$root"
    exec 9<> "$lock"
    perl -MFcntl=:flock -e 'open(my $f, "+<&=", 9) or die "lock: $!\n"; flock($f, LOCK_EX | LOCK_NB) or exit 75;
            truncate($f, 0); seek($f, 0, 0); print $f "pid $ARGV[0], started $ARGV[1], branch $ARGV[2]\n"' -- \
            "$$" "$(ps -o lstart= -p $$ | sed 's/^ *//')" "$slug" || status=$?
    if (( status != 0 )); then
        exec 9>&-
        (( status == 75 )) || die "the release lock $lock could not be taken (perl status $status)"
        die "another release is running on this machine ($(lock_holder)); one release runs at a time"
    fi
    owns_lock=1
}

# ---- the upload record ---------------------------------------------------------------------------------------------

if [[ "$mode" == uploads ]]; then
    if [[ -s "$records" ]]; then cat "$records"; else printf 'no upload has started from this machine\n'; fi
    exit 0
fi

if [[ "$mode" == busy ]]; then
    scope=${3:?branch or machine}
    if lock_is_held; then
        holder=$(lock_holder)
        if [[ "$scope" == machine || "$holder" == *", branch $slug" ]]; then
            printf 'refusing: a release is running on this machine (%s); wait for it, or read `build-ios.sh uploads`\n' "$holder" >&2
            exit 1
        fi
    fi
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
        [[ "${HIDE_IOS_ASC_KEY_PATH##*/}" == "AuthKey_${HIDE_IOS_ASC_KEY_ID}.p8" ]] || problems+=("HIDE_IOS_ASC_KEY_PATH does not name the key HIDE_IOS_ASC_KEY_ID")
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
    secrets_loaded=1
}
secrets_loaded=0

# The three values, one per line, for a pipe only.
secret_values() { printf '%s\n' "$HIDE_IOS_ASC_KEY_PATH" "$HIDE_IOS_ASC_KEY_ID" "$HIDE_IOS_ASC_ISSUER_ID"; }
# A token (a JWT: base64url JSON, so it starts with eyJ) and a PEM private key, whatever key they belong to.
token_shape='eyJ[A-Za-z0-9_-]{10,}\.'
pem_shape='-----BEGIN [A-Z ]*PRIVATE KEY-----'
# Replaces every value, in any letter case, with the name of its key and a whole token with its shape. The sed program
# comes from a pipe.
redact() {
    sed -f <(
        escape() { printf '%s' "$1" | sed 's/[][\.*^$|/&]/\\&/g'; }
        printf 's|%s|<HIDE_IOS_ASC_KEY_PATH>|gI\n' "$(escape "$HIDE_IOS_ASC_KEY_PATH")"
        printf 's|%s|<HIDE_IOS_ASC_KEY_ID>|gI\n' "$(escape "$HIDE_IOS_ASC_KEY_ID")"
        printf 's|%s|<HIDE_IOS_ASC_ISSUER_ID>|gI\n' "$(escape "$HIDE_IOS_ASC_ISSUER_ID")"
    ) | sed -E 's/eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]*/<token>/g'
}
# The search behind the redaction: any of the three values, anything shaped like a token (a part of one included), or a
# PEM key header. It cannot see a key's body printed without its header, which no tool here has a path to print: the
# body is read by Apple's tools from the file, and nothing else reads it.
log_is_clean() {
    [[ -f "$log" && "$secrets_loaded" == 1 ]] || return 0
    ! grep -qiF -f <(secret_values) "$log" && ! grep -qE -e "$token_shape" -e "$pem_shape" "$log"
}

# ---- steps ---------------------------------------------------------------------------------------------------------
# `run_step <seconds> <command...>`: the command in a process group of its own, its output redacted into the log. A
# time bound, a signal or the step's own end ends the whole group, so no grandchild outlives it. Status 124 is the bound.
step_group= step_dog= step_reader=
end_step() {
    local i
    [[ -n "$step_group" ]] || return 0
    kill -TERM -- "-$step_group" 2> /dev/null || true
    for i in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
        kill -0 -- "-$step_group" 2> /dev/null || break
        sleep 0.25
    done
    kill -KILL -- "-$step_group" 2> /dev/null || true
    wait "$step_group" 2> /dev/null || true
    [[ -z "$step_dog" ]] || { kill "$step_dog" 2> /dev/null || true; wait "$step_dog" 2> /dev/null || true; }
    [[ -z "$step_reader" ]] || wait "$step_reader" 2> /dev/null || true
    rm -f "$work/step.out"
    step_group= step_dog= step_reader=
}
run_step() {
    local limit=$1 status=0
    shift
    rm -f "$work/timed-out" "$work/step.out"
    mkfifo "$work/step.out"
    redact < "$work/step.out" >> "$log" 9>&- &
    step_reader=$!
    perl -e 'setpgrp(0, 0); exec { $ARGV[0] } @ARGV or die "cannot start $ARGV[0]: $!\n"' -- "$@" < /dev/null > "$work/step.out" 2>&1 9>&- &
    step_group=$!
    perl -e '$SIG{TERM} = sub { exit 0 }; sleep $ARGV[0]; open(my $f, ">", $ARGV[2]); close $f;
             kill "TERM", -$ARGV[1]; sleep 5; kill "KILL", -$ARGV[1]' -- "$limit" "$step_group" "$work/timed-out" > /dev/null 2>&1 9>&- &
    step_dog=$!
    wait "$step_group" || status=$?
    end_step
    [[ -e "$work/timed-out" ]] && return 124
    return "$status"
}

# `logged <seconds> <name> <command...>`: a step whose failure stops the release; the terminal gets the step's name and,
# when it fails, the end of its log once the log has been searched for a secret, never before.
logged() {
    local limit=$1 name=$2 status=0
    shift 2
    current_step=$name
    printf '\n== %s\n' "$name" >> "$log"
    printf '== %s\n' "$name"
    run_step "$limit" "$@" || status=$?
    (( status == 0 )) && return 0
    if (( status == 124 )); then printf 'release stopped at "%s": it did not finish in %s seconds\n' "$name" "$limit" >&2
    else printf 'release stopped at "%s" (status %s)\n' "$name" "$status" >&2; fi
    if log_is_clean; then
        printf 'the end of its log:\n' >&2
        tail -n 40 "$log" >&2
    fi
    exit 1
}

# ---- What a release owns -------------------------------------------------------------------------------------------
# Everything a run creates, each marked the moment it exists, and the one cleanup that removes exactly this list:
#   the machine lock     $lock, held on fd 9 (owns_lock); its text is emptied and the descriptor closed
#   the work folder      $base/release (owns_work): archive, derived data, export, unpacked ipa, facts, the private
#                        home Apple's tools run with (their logs and URL cache), the temporary folder, the step's pipe
#   Apple's by-name logs App_*.xcdistributionlogs and XcodeDistPipeline.* (a signed token and the key's ids) that the
#                        export writes into the account's temporary folder, which it takes from confstr and not from
#                        $TMPDIR or the home, since $work/started; deleted unread, and only under the machine lock, so no
#                        other release can own one (an export another program starts during the run is the one case left)
#   a status scratch     $base/status.<pid> (owns_work in `status`): the private home and temporary folder of one altool call
# What stays is not the run's: release.log (redacted and searched), the machine's uploads.log, the empty lock file, and
# what Apple's frameworks leave in the account's temporary folder under names any program uses (CFNetworkDownload_*,
# cms~~~*, and the export's own ResultBundle_* beside the one it is given), which hold neither the key's ids nor a
# token and are not deleted by name (docs/BUILD.md, The release build).
owns_lock=0 owns_work=0 finished=0 upload_started=0 upload_ended=0
work= log= account_tmp=
remove_apple_logs() {
    local path
    [[ "$owns_lock" == 1 && -n "$account_tmp" && -e "$work/started" ]] || return 0
    while IFS= read -r path; do
        [[ -n "$path" ]] || continue
        rm -rf "$path"
        printf 'removed xcodebuild distribution log %s from the account temporary folder\n' "${path##*/}" >> "$log" 2> /dev/null || true
    done < <(find "$account_tmp" -maxdepth 1 \( -name 'App_*.xcdistributionlogs' -o -name 'XcodeDistPipeline.*' \) -newer "$work/started" 2> /dev/null)
}
cleanup() {
    local code=$?
    set +e
    trap - ERR
    trap '' TERM HUP INT
    end_step
    if [[ "$owns_work" == 1 ]]; then
        remove_apple_logs
        if ! log_is_clean; then
            rm -f "$log"
            say 'release stopped: a secret value reached the log, which is deleted; this is a bug in the redaction'
            code=1
        fi
        # An upload that started and never recorded its end: the caller must be able to read that it may have gone.
        if [[ "$upload_started" == 1 && "$upload_ended" == 0 ]]; then
            say 'the upload may have gone without its delivery id (its record says started); read `build-ios.sh uploads` and look in App Store Connect before running again'
        fi
        rm -rf "$work"
    fi
    if [[ "$owns_lock" == 1 ]]; then
        : > "$lock"
        exec 9>&-
    fi
    [[ "$finished" == 1 && "$code" == 0 ]] || say "release did not finish (status $code)"
    exit "$code"
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

# Apple's tools that talk to App Store Connect run with a home of their own inside the work folder: altool's logs (the
# key id and path) and xcodebuild's URL cache (signed tokens) land there and go with it. Its Library/Caches must exist
# first: without it CFNetwork puts the URL cache in the account's cache folder (confstr) instead (seen on the mini).
# `"${apple[@]}" <tool>` once the work folder is known.
apple=()
with_private_home() {
    mkdir -p "$work/home/Library/Caches" "$work/home/Library/Logs" "$work/tmp"
    apple=(env "CFFIXED_USER_HOME=$work/home" "TMPDIR=$work/tmp")
}

# ---- status --------------------------------------------------------------------------------------------------------

if [[ "$mode" == status ]]; then
    delivery=${3:-}
    [[ "$delivery" =~ ^[0-9a-f-]{36}$ ]] || die "status needs the delivery id the release printed"
    [[ -f "$registry" ]] || die "$registry is missing: run the release command once from this branch first"
    load_secrets
    work=$base/status.$$
    mkdir -p "$work"
    owns_work=1
    log=$work/status.log
    : > "$log"
    with_private_home
    answer=0
    run_step 300 "${apple[@]}" xcrun altool --build-status --delivery-id "$delivery" --api-key "$HIDE_IOS_ASC_KEY_ID" \
        --api-issuer "$HIDE_IOS_ASC_ISSUER_ID" --p8-file-path "$HIDE_IOS_ASC_KEY_PATH" || answer=$?
    log_is_clean || die "the answer held a secret value and is not shown; this is a bug in the redaction"
    cat "$log"
    (( answer == 124 )) && die "altool did not answer in 300 seconds"
    (( answer == 0 )) || die "altool --build-status failed (status $answer)"
    finished=1
    exit 0
fi

# ---- release: lock, record, disk, secrets --------------------------------------------------------------------------

[[ "$mode" == release ]] || die "unknown mode '$mode'"
version=${3:?version} build=${4:?build number} commit=${5:?commit} again=${6:-0}
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "version '$version' is not three numbers"
[[ "$build" =~ ^[0-9]{1,18}$ ]] || die "build number '$build' is not a number"
[[ "$commit" =~ ^[0-9a-f]{40}$ ]] || die "commit '$commit' is not a full commit id"
for file in "$registry" "$expect" "$inputs/check-release-app.sh" "$inputs/bundle-manifest.json" "$inputs/relay-source.txt"; do
    [[ -f "$file" ]] || die "$file was not copied to this machine"
done
account_tmp=$(getconf DARWIN_USER_TEMP_DIR 2> /dev/null || true)
[[ -d "$account_tmp" ]] || die "the account's temporary folder (getconf DARWIN_USER_TEMP_DIR) is not known"

take_lock

# The same commit is not uploaded twice by accident, under any branch name: after a dropped connection the caller
# cannot know whether the first upload went, so a commit with a record asks for an explicit request.
if [[ -s "$records" && "$again" != 1 ]]; then
    previous=$(grep -F " $commit " "$records" | tail -n 1 || true)
    if [[ -n "$previous" ]]; then
        printf 'release stopped: this commit already has an upload record (time, commit, version, build, delivery id, state, branch):\n  %s\n' "$previous" >&2
        printf 'a completed upload of a commit is not repeated; to build and upload it again on purpose run with HIDE_IOS_RELEASE_AGAIN=1\n' >&2
        exit 1
    fi
fi

free_gb=$(df -g / | awk 'NR==2 {print $4}')
(( free_gb >= min_free_gb )) || die "this machine has ${free_gb} GB free and a release build needs at least $min_free_gb GB"

load_secrets
team=$(jq -r .teamId "$expect")
work=$base/release
log=$base/release.log
# A folder a killed run of this branch left is this run's to remove: no other release can be running.
rm -rf "$work"
mkdir -p "$work/facts"
owns_work=1
: > "$work/started"
: > "$log"
printf 'release build %s (%s), team %s\n' "$version" "$build" "$team"
export TMPDIR=$work/tmp
with_private_home
auth=(-authenticationKeyPath "$HIDE_IOS_ASC_KEY_PATH" -authenticationKeyID "$HIDE_IOS_ASC_KEY_ID" -authenticationKeyIssuerID "$HIDE_IOS_ASC_ISSUER_ID")
altool_auth=(--api-key "$HIDE_IOS_ASC_KEY_ID" --api-issuer "$HIDE_IOS_ASC_ISSUER_ID" --p8-file-path "$HIDE_IOS_ASC_KEY_PATH")

# The extensions are signed before the app that holds them (Xcode's export does that order), and `--deep --strict` below
# fails when an extension's signature is missing or was made after the app's. The relay library's source digest is read
# from the release inputs build-ios.sh recomputed from the checkout.
relay_digest=$(cat "$inputs/relay-source.txt")
[[ "$relay_digest" =~ ^[0-9a-f]{64}$ ]] || die "release-inputs/relay-source.txt does not hold a relay source digest"

# ---- archive, export, check ----------------------------------------------------------------------------------------

logged "$limit_archive" "archive" xcodebuild archive -project "$project/App.xcodeproj" -scheme App -configuration Release \
    -destination 'generic/platform=iOS' -archivePath "$work/App.xcarchive" -derivedDataPath "$work/DerivedData" \
    -clonedSourcePackagesDirPath "$packages" -resultBundlePath "$work/archive.xcresult" CODE_SIGNING_ALLOWED=NO \
    MARKETING_VERSION="$version" CURRENT_PROJECT_VERSION="$build"

# An archive made with signing off carries none of the project's entitlements, and the export only keeps what the archive
# holds, so each entitlements file the project names is put into its archived bundle first (an ad hoc signature, which
# the export replaces with the distribution one; no certificate or keychain is involved). The files name build settings
# (`$(AppIdentifierPrefix)`, `$(PRODUCT_BUNDLE_IDENTIFIER)`, `$(APS_ENVIRONMENT)`) that only a signing build resolves, so
# they are resolved here to what the Release configuration gives them; one left unresolved would be exported as text,
# and the signed app would name a keychain group that is no group of its own.
archived=$work/App.xcarchive/Products/Applications/App.app
mkdir -p "$work/entitlements"
embed_entitlements() { # <bundle> <the project's entitlements file> <bundle id>
    local bundle=$1 source=$2 id=$3 resolved
    [[ -f "$source" ]] || die "the project's ${source##*/} is missing"
    resolved=$work/entitlements/${source##*/}
    sed -e "s/\$(AppIdentifierPrefix)/$team./g" -e "s/\$(PRODUCT_BUNDLE_IDENTIFIER)/$id/g" \
        -e 's/\$(APS_ENVIRONMENT)/production/g' "$source" > "$resolved"
    if grep -qF '$(' "$resolved"; then die "${source##*/} names a build setting the release does not resolve"; fi
    logged 120 "embed entitlements: ${bundle##*/}" codesign --force --sign - --entitlements "$resolved" --timestamp=none "$bundle"
}
# An extension before the app that holds it: signing the app seals the extension's signature.
while IFS=$'\t' read -r name id; do
    [[ -n "$name" ]] || continue
    embed_entitlements "$archived/PlugIns/$name" "$project/${name%.appex}/${name%.appex}.entitlements" "$id"
done < <(jq -r '.appExtensions[]? | [.name, .bundleId] | @tsv' "$expect")
embed_entitlements "$archived" "$project/App/App.entitlements" "$(jq -r .bundleId "$expect")"

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

logged "$limit_export" "export and sign" "${apple[@]}" xcodebuild -exportArchive -archivePath "$work/App.xcarchive" \
    -exportPath "$work/export" -exportOptionsPlist "$work/ExportOptions.plist" -resultBundlePath "$work/export.xcresult" \
    -allowProvisioningUpdates "${auth[@]}"
remove_apple_logs

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

logged "$limit_validate" "validate" "${apple[@]}" xcrun altool --validate-app -f "$ipa" --type ios "${altool_auth[@]}"
if [[ "${HIDE_IOS_NO_UPLOAD:-}" == 1 ]]; then
    printf 'validated %s (%s) and stopped before the upload (HIDE_IOS_NO_UPLOAD=1); nothing was sent to TestFlight\n' "$version" "$build"
    finished=1
    exit 0
fi

# The record is written before the bytes leave: a dropped connection cannot make the upload disappear from it.
printf '%s %s %s %s %s started %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$commit" "$version" "$build" "-" "$slug" >> "$records"
upload_started=1
logged "$limit_upload" "upload" "${apple[@]}" xcrun altool --upload-app -f "$ipa" --type ios "${altool_auth[@]}"
delivery=$(sed -n 's/^Delivery UUID: //p' "$log" | tail -n 1)
[[ "$delivery" =~ ^[0-9a-f-]{36}$ ]] || die "the upload printed no delivery id"
printf '%s %s %s %s %s uploaded %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$commit" "$version" "$build" "$delivery" "$slug" >> "$records"
upload_ended=1

# Apple processes the build after the upload; this reads its state once and does not wait for it.
step "build status"
printf '\n== build status\n' >> "$log"
run_step 300 "${apple[@]}" xcrun altool --build-status --delivery-id "$delivery" "${altool_auth[@]}" || true
log_is_clean && tail -n 8 "$log"
printf 'uploaded: %s (%s), delivery id %s. Apple now processes it; `build-ios.sh status %s` reads its state again. Log: %s\n' "$version" "$build" "$delivery" "$delivery" "$log"
df -h / | tail -1 | awk '{printf "build machine free disk: %s of %s\n", $4, $2}'
finished=1

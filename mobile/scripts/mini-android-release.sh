#!/usr/bin/env bash
# Runs on the build machine (piped by build-android.sh release): builds, signs, checks and uploads one Android release.
#
# Usage: mini-android-release.sh release <branch slug> <version> <version code> <commit> <again: 0|1>
#        mini-android-release.sh uploads <branch slug>
#        mini-android-release.sh busy <branch slug> <branch|machine>
#   The project and the release inputs are already in ~/hide-android-build/<slug>/ (repo/ and release-inputs/), and the
#   branch's `build` (the JVM tests, the debug app and the instrumented tests) has just passed there.
#   `uploads` prints the machine's upload record; `busy` exits 1, naming it, while a release holds the machine (`machine`)
#   or a release of this branch does (`branch`).
#   HIDE_ANDROID_NO_UPLOAD=1 stops after the release checks, before anything is sent to Play.
#
# Secrets: the keys of android-release-env.json, read from the file that registry names as plain KEY=VALUE lines and never
# run as a script, and never from the environment: a value there would reach every program the run starts, Gradle among
# them. They are kept in variables of this shell that are never exported. Every key is checked before anything is built,
# and a problem names the key and the kind of problem, never a value. The keystore password reaches keytool and jarsigner
# through a pipe (`-storepass:file /dev/fd/N`); the service account's private key reaches openssl the same way, and the
# signed assertion and the access token reach curl through a pipe too: none is ever an argument, a file or an
# environment variable. Gradle is given no secret at all, since it builds the bundle unsigned and jarsigner signs it.
#
# What a run owns is listed once, in "What a release owns" below, and `cleanup` removes exactly that on every way out:
# a normal end, a failed step, TERM, HUP or INT, and a closed standard output (SIGPIPE is ignored, so the next write fails
# and the script ends through its cleanup).
set -Eeuo pipefail
umask 077
trap '' PIPE

mode=${1:?mode} slug=${2:?branch slug}
export JAVA_HOME=${JAVA_HOME:-/opt/homebrew/opt/openjdk@21/libexec/openjdk.jdk/Contents/Home}
export ANDROID_HOME=${ANDROID_HOME:-$HOME/Library/Android/sdk}
root=$HOME/hide-android-build
base=$root/$slug
inputs=$base/release-inputs
project=$base/repo/mobile/android
registry=$inputs/android-release-env.json
expect=$inputs/android-release-expect.json
play_api=${HIDE_ANDROID_PLAY_API:-https://androidpublisher.googleapis.com}
# A run's work folder holds two copies of the bundle (about 35 MB each) and Gradle's release outputs about 100 MB more;
# the floor only keeps a run from filling the disk under the machine's other work.
min_free_gb=3
# Machine-wide, because what they guard is: the Play account and its version codes. Neither is ever deleted.
lock=$root/.release.lock
records=$root/uploads.log
limit_bundle=1200 limit_sign=300 limit_api=120 limit_upload=${HIDE_ANDROID_TEST_UPLOAD_LIMIT:-1800}

current_step=start
step() { current_step=$1; printf '\n== %s\n' "$1"; }
die() { printf 'release stopped: %s\n' "$1" >&2; exit 1; }
say() { printf '%s\n' "$1" >&2 2> /dev/null || true; }
trap 'printf "release stopped during \"%s\"\n" "$current_step" >&2 2> /dev/null || true' ERR

# ---- the machine lock ----------------------------------------------------------------------------------------------
# Held by the kernel (flock) on file descriptor 9 of this shell, so it ends when the shell ends, however it ends. Every
# step run_step starts gets 9>&-, so no tool that outlives the run keeps the lock. Status 75 is "another holds it".
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
        die "another Android release is running on this machine ($(lock_holder)); one release runs at a time"
    fi
    owns_lock=1
}

if [[ "$mode" == uploads ]]; then
    if [[ -s "$records" ]]; then cat "$records"; else printf 'no Android upload has started from this machine\n'; fi
    exit 0
fi

if [[ "$mode" == busy ]]; then
    scope=${3:?branch or machine}
    if lock_is_held; then
        holder=$(lock_holder)
        if [[ "$scope" == machine || "$holder" == *", branch $slug" ]]; then
            printf 'refusing: an Android release is running on this machine (%s); wait for it, or read `build-android.sh uploads`\n' "$holder" >&2
            exit 1
        fi
    fi
    exit 0
fi

# ---- secrets -------------------------------------------------------------------------------------------------------

upload=1
[[ "${HIDE_ANDROID_NO_UPLOAD:-}" == 1 ]] && upload=0
problems=()
mode_ok() { [[ "$(stat -f %Lp "$1")" =~ ^[46]00$ && "$(stat -f %u "$1")" == "$(id -u)" ]]; }
access_token= assertion=
load_secrets() {
    local file line key value req shape
    file=$(jq -r .file "$registry")
    file=${file/#\~/$HOME}
    # The values live in variables of this shell that are never exported; one the environment holds is refused, and
    # its variable is emptied and unexported before anything else runs.
    for key in $(jq -r '.keys | keys[]' "$registry"); do
        if printenv "$key" > /dev/null; then problems+=("$key is set in the environment; the release reads its secrets from $file only"); fi
        export -n "$key" 2> /dev/null || true
        printf -v "$key" '%s' ''
    done
    if [[ ! -f "$file" ]]; then
        problems+=("$file is missing")
    else
        mode_ok "$file" || problems+=("$file must be owned by this account and readable by it alone (mode 600)")
        while IFS= read -r line || [[ -n "$line" ]]; do
            [[ -z "$line" || "$line" == \#* ]] && continue
            key=${line%%=*}
            if jq -e --arg k "$key" '.keys[$k]' "$registry" > /dev/null; then printf -v "$key" '%s' "${line#*=}"; else problems+=("$file has a key the registry does not know: $key"); fi
        done < "$file"
    fi
    for key in $(jq -r '.keys | keys[]' "$registry"); do
        value=${!key:-}
        req=$(jq -r --arg k "$key" '.keys[$k].requirement' "$registry")
        if [[ -z "$value" ]]; then
            [[ "$req" == required || ( "$req" == upload && "$upload" == 1 ) ]] && problems+=("$key is missing")
            continue
        fi
        shape=$(jq -r --arg k "$key" '.keys[$k].shape' "$registry")
        [[ "$value" =~ $shape ]] || problems+=("$key is not shaped as android-release-env.json says")
    done
    if (( ${#problems[@]} == 0 )); then
        if [[ ! -f "$HIDE_ANDROID_UPLOAD_KEYSTORE" ]]; then problems+=("HIDE_ANDROID_UPLOAD_KEYSTORE names no file here")
        elif ! mode_ok "$HIDE_ANDROID_UPLOAD_KEYSTORE"; then problems+=("HIDE_ANDROID_UPLOAD_KEYSTORE names a file other accounts can read")
        elif ! "$JAVA_HOME/bin/keytool" -list -keystore "$HIDE_ANDROID_UPLOAD_KEYSTORE" -alias "$HIDE_ANDROID_UPLOAD_KEY_ALIAS" \
                -storepass:file <(printf '%s\n' "$HIDE_ANDROID_UPLOAD_PASSWORD") < /dev/null > /dev/null 2>&1; then
            problems+=("HIDE_ANDROID_UPLOAD_PASSWORD does not open HIDE_ANDROID_UPLOAD_KEYSTORE, or it holds no key HIDE_ANDROID_UPLOAD_KEY_ALIAS")
        fi
        if [[ "$upload" == 1 ]]; then
            local account=$HIDE_ANDROID_PLAY_SERVICE_ACCOUNT
            if [[ ! -f "$account" ]]; then problems+=("HIDE_ANDROID_PLAY_SERVICE_ACCOUNT names no file here")
            elif ! mode_ok "$account"; then problems+=("HIDE_ANDROID_PLAY_SERVICE_ACCOUNT names a file other accounts can read")
            elif ! jq -e '.type == "service_account" and (.client_email | type == "string") and (.private_key | type == "string" and startswith("-----BEGIN PRIVATE KEY-----")) and (.token_uri | type == "string" and startswith("https://"))' \
                    "$account" > /dev/null 2>&1; then
                problems+=("HIDE_ANDROID_PLAY_SERVICE_ACCOUNT is not a service account key file (type, client_email, private_key, token_uri)")
            fi
        fi
    fi
    if (( ${#problems[@]} > 0 )); then
        printf 'release stopped: the signing secrets are not usable (values are never printed; mobile/android-release.env.example shows the shape):\n' >&2
        printf '  - %s\n' "${problems[@]}" >&2
        exit 1
    fi
    secrets_loaded=1
}
secrets_loaded=0

# Every secret value this run holds, one per line, for a pipe only.
secret_values() {
    printf '%s\n' "$HIDE_ANDROID_UPLOAD_PASSWORD"
    [[ -z "$access_token" ]] || printf '%s\n' "$access_token"
    [[ -z "$assertion" ]] || printf '%s\n' "$assertion"
}
# Google's access tokens (ya29.), a signed assertion (a JWT: base64url JSON, eyJ...) and a PEM private key, whatever key.
token_shape='ya29\.[A-Za-z0-9_-]{10,}|eyJ[A-Za-z0-9_-]{10,}\.'
pem_shape='-----BEGIN [A-Z ]*PRIVATE KEY-----'
# Replaces every secret value with the name of what it is and a whole token with its shape; the sed program comes from
# a pipe.
redact() {
    sed -f <(
        escape() { printf '%s' "$1" | sed 's/[][\.*^$|/&]/\\&/g'; }
        printf 's|%s|<HIDE_ANDROID_UPLOAD_PASSWORD>|g\n' "$(escape "$HIDE_ANDROID_UPLOAD_PASSWORD")"
        [[ -z "$access_token" ]] || printf 's|%s|<token>|g\n' "$(escape "$access_token")"
        [[ -z "$assertion" ]] || printf 's|%s|<token>|g\n' "$(escape "$assertion")"
    ) | sed -E -e 's/ya29\.[A-Za-z0-9._-]+/<token>/g' -e 's/eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]*/<token>/g'
}
log_is_clean() {
    [[ -f "$log" && "$secrets_loaded" == 1 ]] || return 0
    ! grep -qF -f <(secret_values) "$log" && ! grep -qE -e "$token_shape" -e "$pem_shape" "$log"
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
stop_with_log() {
    if log_is_clean; then
        printf 'the end of its log:\n' >&2
        tail -n 40 "$log" >&2
    fi
    exit 1
}
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
    stop_with_log
}

# ---- Play's publishing API -----------------------------------------------------------------------------------------
# One call: `play <name> <seconds> <method> <path> [curl arguments...]`, the answer in $work/answer.json and, redacted, in
# the log. The access token reaches curl as a header file that is a pipe.
play() {
    local name=$1 limit=$2 method=$3 path=$4 status=0
    shift 4
    current_step=$name
    printf '\n== %s\n' "$name" >> "$log"
    printf '== %s\n' "$name"
    curl -sS --max-time "$limit" --fail-with-body -X "$method" -H @<(printf 'Authorization: Bearer %s\n' "$access_token") \
        -o "$work/answer.json" "$@" "$play_api$path" < /dev/null 2> "$work/curl.err" || status=$?
    { cat "$work/answer.json" "$work/curl.err" 2> /dev/null || true; printf '\n'; } | redact >> "$log"
    rm -f "$work/curl.err"
    (( status == 0 )) && return 0
    printf 'release stopped at "%s" (curl status %s)\n' "$name" "$status" >&2
    stop_with_log
}
b64url() { openssl base64 -A | tr '+/' '-_' | tr -d '=\n'; }
# The service account signs a one-hour assertion (RS256) and trades it for an access token scoped to the publishing API.
play_sign_in() {
    local account=$HIDE_ANDROID_PLAY_SERVICE_ACCOUNT now header claims signature answer status=0
    current_step="sign in to Play"
    printf '\n== sign in to Play\n' >> "$log"
    printf '== sign in to Play\n'
    now=$(date +%s)
    header=$(printf '%s' '{"alg":"RS256","typ":"JWT"}' | b64url)
    claims=$(jq -cj --argjson now "$now" '{iss: .client_email, scope: "https://www.googleapis.com/auth/androidpublisher", aud: .token_uri, iat: $now, exp: ($now + 3600)}' "$account" | b64url)
    signature=$(printf '%s.%s' "$header" "$claims" | openssl dgst -sha256 -binary -sign <(jq -r .private_key "$account") | b64url)
    assertion=$header.$claims.$signature
    answer=$(printf '%s' "$assertion" | curl -sS --max-time "$limit_api" --fail-with-body -X POST \
        --data-urlencode 'grant_type=urn:ietf:params:oauth:grant-type:jwt-bearer' --data-urlencode 'assertion@-' \
        "$(jq -r .token_uri "$account")" 2>&1) || status=$?
    access_token=$(printf '%s' "$answer" | jq -r '.access_token // empty' 2> /dev/null || true)
    if (( status != 0 )) || [[ -z "$access_token" ]]; then
        printf '%s\n' "$answer" | redact >> "$log"
        printf 'release stopped: Play refused the service account (curl status %s)\n' "$status" >&2
        stop_with_log
    fi
    printf 'signed in as %s\n' "$(jq -r .client_email "$account")" >> "$log"
}

# ---- What a release owns -------------------------------------------------------------------------------------------
# Everything a run creates, each marked the moment it exists, and the one cleanup that removes exactly this list:
#   the machine lock   $lock, held on fd 9 (owns_lock); its text is emptied and the descriptor closed
#   the work folder    $base/release (owns_work): the unsigned and the signed bundle, the unpacked bundle, the signers,
#                      the API's answers, the temporary folder and the step's pipe
#   a Play edit        (edit): an edit this run opened and did not commit is deleted, so nothing it uploaded reaches a
#                      track; the record says so
# What stays is not the run's: release.log (redacted and searched), the machine's uploads.log, the empty lock file, and
# Gradle's release outputs in the branch's project (an unsigned bundle, which the next build replaces).
owns_lock=0 owns_work=0 finished=0 upload_started=0 upload_ended=0
work= log= edit=
cleanup() {
    local code=$?
    set +e
    trap - ERR
    trap '' TERM HUP INT
    end_step
    if [[ -n "$edit" && "$upload_ended" == 0 && -n "$access_token" ]]; then
        if curl -sS --max-time 60 --fail -X DELETE -H @<(printf 'Authorization: Bearer %s\n' "$access_token") \
                "$play_api/androidpublisher/v3/applications/$package/edits/$edit" < /dev/null > /dev/null 2>&1; then
            say "deleted the uncommitted Play edit $edit: nothing this run uploaded reached a track"
            printf '%s %s %s %s %s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$commit" "$version" "$code_number" deleted "$slug" >> "$records" 2> /dev/null
        else
            say "the uncommitted Play edit $edit could not be deleted; it expires on its own and reaches no track unless committed"
        fi
    fi
    if [[ "$owns_work" == 1 ]]; then
        if ! log_is_clean; then
            rm -f "$log"
            say 'release stopped: a secret value reached the log, which is deleted; this is a bug in the redaction'
            code=1
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

# ---- release: lock, record, disk, secrets --------------------------------------------------------------------------

[[ "$mode" == release ]] || die "unknown mode '$mode'"
version=${3:?version} code_number=${4:?version code} commit=${5:?commit} again=${6:-0}
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "version '$version' is not three numbers"
[[ "$code_number" =~ ^[1-9][0-9]{0,9}$ ]] && (( code_number <= 2100000000 )) || die "version code '$code_number' is not a number Play takes"
[[ "$commit" =~ ^[0-9a-f]{40}$ ]] || die "commit '$commit' is not a full commit id"
for file in "$registry" "$expect" "$inputs/check-release-aab.py" "$inputs/bundle-manifest.json" "$project/gradlew"; do
    [[ -f "$file" ]] || die "$file was not copied to this machine"
done
package=$(jq -r .package "$expect")

take_lock

if [[ -s "$records" && "$again" != 1 ]]; then
    # The last line of the commit decides: an upload whose edit was deleted reached no track and may run again.
    previous=$(grep -F " $commit " "$records" | tail -n 1 || true)
    if [[ -n "$previous" && "$previous" != *" deleted "* ]]; then
        printf 'release stopped: this commit already has an upload record (time, commit, version, version code, state, branch):\n  %s\n' "$previous" >&2
        printf 'a completed upload of a commit is not repeated; to build and upload it again on purpose run with HIDE_ANDROID_RELEASE_AGAIN=1\n' >&2
        exit 1
    fi
fi

free_gb=$(df -g / | awk 'NR==2 {print $4}')
(( free_gb >= min_free_gb )) || die "this machine has ${free_gb} GB free and a release build needs at least $min_free_gb GB"

load_secrets
work=$base/release
log=$base/release.log
# A folder a killed run of this branch left is this run's to remove: no other release can be running.
rm -rf "$work"
mkdir -p "$work/tmp"
owns_work=1
: > "$log"
export TMPDIR=$work/tmp
printf 'Android release %s (%s) of %s\n' "$version" "$code_number" "$package"

# ---- bundle, sign, check -------------------------------------------------------------------------------------------

outputs=$project/app/build/outputs/bundle/release
rm -rf "$outputs"
logged "$limit_bundle" "bundle" "$project/gradlew" -p "$project" --no-daemon --console=plain bundleRelease -PhideVersionCode="$code_number"
[[ -f "$outputs/app-release.aab" ]] || die "Gradle made no app-release.aab"
cp "$outputs/app-release.aab" "$work/unsigned.aab"

logged "$limit_sign" "sign with the upload key" "$JAVA_HOME/bin/jarsigner" -keystore "$HIDE_ANDROID_UPLOAD_KEYSTORE" \
    -storepass:file <(printf '%s\n' "$HIDE_ANDROID_UPLOAD_PASSWORD") -sigalg SHA256withRSA -digestalg SHA-256 \
    -signedjar "$work/app.aab" "$work/unsigned.aab" "$HIDE_ANDROID_UPLOAD_KEY_ALIAS"
rm -f "$work/unsigned.aab"

step "verify the signature"
verified=$("$JAVA_HOME/bin/jarsigner" -verify "$work/app.aab" < /dev/null 2>&1) || die "jarsigner refused the signature: $(tail -n 3 <<< "$verified")"
grep -q '^jar verified\.' <<< "$verified" || die "jarsigner did not verify the bundle: $(tail -n 3 <<< "$verified")"
! grep -q 'unsigned entries' <<< "$verified" || die "the bundle holds entries the signature does not cover"
"$JAVA_HOME/bin/keytool" -printcert -jarfile "$work/app.aab" < /dev/null 2> /dev/null | sed -n 's/^[[:space:]]*SHA256: //p' > "$work/signers.txt"

step "unpack the signed bundle"
unzip -q "$work/app.aab" -d "$work/aab"

step "release checks"
python3 -I "$inputs/check-release-aab.py" "$work/aab" "$work/signers.txt" "$expect" "$inputs/bundle-manifest.json" "$version" "$code_number"
sha256=$(shasum -a 256 "$work/app.aab" | cut -d' ' -f1)

if [[ "$upload" == 0 ]]; then
    printf 'signed and checked %s (%s), sha256 %s, and stopped before the upload (HIDE_ANDROID_NO_UPLOAD=1); nothing was sent to Play\n' "$version" "$code_number" "$sha256"
    finished=1
    exit 0
fi

# ---- upload to the internal testing track --------------------------------------------------------------------------
# An edit is Play's transaction: the bundle and the track change reach Play only when the edit is committed, and an
# edit this run does not commit is deleted on the way out.

play_sign_in
apps=/androidpublisher/v3/applications/$package
play "open an edit" "$limit_api" POST "$apps/edits" -H 'Content-Type: application/json' --data-binary '{}'
edit=$(jq -r '.id // empty' "$work/answer.json")
[[ "$edit" =~ ^[A-Za-z0-9_-]+$ ]] || die "Play opened no edit"

printf '%s %s %s %s %s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$commit" "$version" "$code_number" started "$slug" >> "$records"
upload_started=1
play "upload the bundle" "$limit_upload" POST "/upload$apps/edits/$edit/bundles?uploadType=media" \
    -H 'Content-Type: application/octet-stream' --data-binary @"$work/app.aab"
[[ "$(jq -r '.versionCode // empty' "$work/answer.json")" == "$code_number" ]] || die "Play took the bundle under another version code"
[[ "$(jq -r '.sha256 // empty' "$work/answer.json")" == "$sha256" ]] || die "Play's digest of the bundle is not the checked bundle's"

release=$(jq -cn --arg name "$version ($code_number)" --arg code "$code_number" '{track: "internal", releases: [{name: $name, versionCodes: [$code], status: "completed"}]}')
play "put it on the internal track" "$limit_api" PUT "$apps/edits/$edit/tracks/internal" -H 'Content-Type: application/json' --data-binary "$release"
play "commit the edit" "$limit_api" POST "$apps/edits/$edit:commit"
printf '%s %s %s %s %s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$commit" "$version" "$code_number" uploaded "$slug" >> "$records"
upload_ended=1
printf 'uploaded: %s (%s) to the internal testing track, sha256 %s. Log: %s\n' "$version" "$code_number" "$sha256" "$log"
df -h / | tail -1 | awk '{printf "build machine free disk: %s of %s\n", $4, $2}'
finished=1

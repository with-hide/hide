#!/usr/bin/env bash
# Prepares what the remote mailbox lane's sign-in journeys need, on a
# disposable runner: a key ssh finds by itself in the runner account's own
# `~/.ssh`, and a second account with a password on a root sshd
# (hided/tests/it/system_ssh.rs names both in HIDE_E2E_SSH_DEFAULT_KEY and
# HIDE_E2E_SSH_PASSWORD).
#
# with-hide/hide-private#898: this preparation ran inside the workflow step
# with no output at all, so a hang left an empty log and the step's own
# `timeout-minutes` killed it naming nothing. Here every command is named
# before it starts, a command that does not finish is stopped and named, and
# the whole run stays inside one budget so the runner's step limit is a
# backstop and never the thing that ends the run.
#
# One limit, `PREPARE_BUDGET_SECONDS` (default 150), sized under the
# workflow's 3-minute step limit. Every command is bounded by what the run has
# left, so a stuck command ends the run here, where the message can name it,
# rather than at the step limit, which names nothing. There is deliberately no
# per-command limit: a healthy step takes 35 s on an arm64 runner, nearly all
# of it `useradd`, and a limit tuned to catch a hang there would also catch a
# slow runner and fail a run that would have passed.
#
# The runner's system decides how the account is made: `RUNNER_OS` as the
# workflow sees it, and uname when that is unset, so the script can also be
# run by hand. It creates an account and starts a server, so run it only
# where that is disposable.
#
# Usage: prepare-ssh-accounts.sh   (on a GitHub-hosted Linux or macOS runner)
set -euo pipefail

budget=${PREPARE_BUDGET_SECONDS:-150}
started=$SECONDS
account=hidepw

[[ $budget =~ ^[0-9]+$ ]] && (( budget > 0 )) || {
    printf 'prepare-ssh-accounts: the time limit must be a positive whole number of seconds, not %s\n' "$budget" >&2
    exit 2
}

runner_os=${RUNNER_OS:-}
if [[ -z $runner_os ]]; then
    case "$(uname -s)" in
        Darwin) runner_os=macOS ;;
        *) runner_os=Linux ;;
    esac
fi
temp=${RUNNER_TEMP:-${TMPDIR:-/tmp}}

# The script's own tools are checked before any of it runs, so a runner that
# lacks one says so in one line here instead of failing part way with a
# command the message cannot name (the watchdog polls with `sleep`, and the
# password is random bytes from `openssl`).
if [ "$runner_os" = Linux ]; then
    needed='openssl sleep ssh-keygen useradd chpasswd'
else
    needed='openssl sleep ssh-keygen sysadminctl createhomedir'
fi
missing=
for tool in $needed; do
    command -v "$tool" >/dev/null 2>&1 || missing="$missing $tool"
done
if [[ -n $missing ]]; then
    printf 'prepare-ssh-accounts: this runner has no%s, which it needs before anything else can run\n' \
        "$missing" >&2
    exit 1
fi

# What the run has left: every command is bounded by this, so one stuck
# command cannot outlive what the whole run is allowed.
left() { echo $(( budget - (SECONDS - started) )); }
limit_for_command() {
    local limit
    limit=$(left)
    (( limit > 0 )) || limit=1
    echo "$limit"
}

# Runs one named command under its limit. The name is printed before the
# command starts, so a log that stops mid-run says what it stopped in, and a
# command that overruns is stopped and named rather than left to the step's
# limit. `timeout` is GNU coreutils and is absent from some macOS images, so
# the limit is a watchdog over the command's own process. The poll is a tenth
# of a second so a usual command costs nothing measurable.
POLL_SECONDS=0.1
run_named() {
    local name=$1 limit pid polls=0 status
    shift
    limit=$(limit_for_command)
    printf -- '-> %s (up to %s s)\n' "$name" "$limit"
    "$@" &
    pid=$!
    while kill -0 "$pid" 2>/dev/null; do
        if (( polls >= limit * 10 )); then
            kill -TERM "$pid" 2>/dev/null || true
            sleep 0.2
            kill -KILL "$pid" 2>/dev/null || true
            wait "$pid" 2>/dev/null || true
            printf 'prepare-ssh-accounts: %s did not finish within %s s and was stopped\n' \
                "$name" "$limit" >&2
            return 1
        fi
        sleep "$POLL_SECONDS"
        polls=$((polls + 1))
    done
    status=0
    wait "$pid" || status=$?
    if (( status != 0 )); then
        printf 'prepare-ssh-accounts: %s failed (exit %s)\n' "$name" "$status" >&2
        return "$status"
    fi
}

# chpasswd reads the account and its password from standard input, so the
# password never becomes an argument in the runner's process list.
set_password() {
    printf '%s:%s\n' "$account" "$password" | sudo chpasswd
}

# The macOS account, with its password.
#
# `sysadminctl -addUser` prints `No clear text password or interactive option
# was specified` on a run that did take the password, which is why that line
# was read for months as the cause of the sign-in this lane could not make
# (with-hide/hide#2554). It is not: the account carries its password either
# way, and the cause was the Remote Login list at the caller. The line stays
# in the log, so a later reader sees it and the record read below that says
# whether there is a password at all.
make_account() {
    sudo sysadminctl -addUser "$account" -password "$password" \
        -home "/Users/$account" -shell /bin/sh
}

# Whether the account holds a password, judged by the directory record and
# not by re-sending the password, which would put it in the runner's process
# list. macOS keeps a password as a `ShadowHash` entry of the account's
# `AuthenticationAuthority`, and this is what answers the `sysadminctl` line
# above: a run of this lane printed that line on 2026-11-15 and the record
# held the password, so the line alone proves nothing and the record does.
account_holds_a_password() {
    local authority
    authority=$(sudo dscl . -read "/Users/$account" AuthenticationAuthority) || return 1
    case "$authority" in
        *';ShadowHash;'*) return 0 ;;
    esac
    printf 'prepare-ssh-accounts: %s holds no password (%s)\n' "$account" "$authority" >&2
    return 1
}

# Puts the account on macOS's Remote Login list so its sshd's PAM stack may
# sign it in (the reason is at its caller). A runner whose Remote Login is
# off has no list to join, which is an answer of its own and not a failure:
# without the list everyone is allowed.
add_to_ssh_service_acl() {
    # `dscl` ends 56 for a record that is not there; `dseditgroup -o read`
    # ends 0 either way and says `Group not found.` on its output, so only
    # this answer is a guard.
    if ! sudo dscl . -read /Groups/com.apple.access_ssh >/dev/null 2>&1; then
        printf 'prepare-ssh-accounts: this runner has no Remote Login list, so every account may sign in\n'
        return 0
    fi
    sudo dseditgroup -o edit -a "$account" -t user com.apple.access_ssh
}

# Says whether the account is on that list, so a run that still fails says
# which of the two answers this step made.
on_ssh_service_acl() {
    sudo dseditgroup -o checkmember -m "$account" com.apple.access_ssh
}

write_sshd_config() {
    local config=$1
    # `LogLevel VERBOSE` records the line each refused sign-in leaves
    # ("Failed password for ...", "Connection closed by authenticating
    # user"), which a passing run never reads: this server's log is kept for
    # the lane, and the lane's step prints its tail only when the tests
    # failed (with-hide/hide#2554).
    printf '%s\n' \
        "ListenAddress 127.0.0.1:2222" "HostKey $config/host" "PidFile $config/sshd.pid" \
        "PasswordAuthentication yes" "KbdInteractiveAuthentication no" "PubkeyAuthentication no" \
        "UsePAM yes" "LogLevel VERBOSE" >"$config/sshd_config"
}

# A bounded command runs in the background, so it cannot set a variable in
# this shell; the password crosses that one step in a file the runner account
# alone may read, which is removed as soon as it is read back.
password_file="$temp/password"
write_password() { (umask 077 && openssl rand -hex 16 >"$password_file"); }
run_named 'the sign-in password' write_password
password=$(cat "$password_file")
rm -f "$password_file"
echo "::add-mask::$password"

run_named "the runner account's ~/.ssh folder" mkdir -p "$HOME/.ssh"
run_named 'the ~/.ssh folder private to the runner account' chmod 700 "$HOME/.ssh"
if [ ! -f "$HOME/.ssh/id_ed25519" ]; then
    run_named 'the default key ssh finds by itself' \
        ssh-keygen -q -t ed25519 -N '' -f "$HOME/.ssh/id_ed25519"
fi
echo "HIDE_E2E_SSH_DEFAULT_KEY=$HOME/.ssh/id_ed25519.pub" >>"$GITHUB_ENV"

if [ "$runner_os" = Linux ]; then
    run_named "the password account (useradd $account)" \
        sudo useradd -m -s /bin/sh "$account"
    run_named "the password account's password (chpasswd $account)" set_password
    run_named 'the sshd privilege separation folder' sudo mkdir -p /run/sshd
else
    run_named "the password account (sysadminctl -addUser $account)" make_account
    run_named "the password account's home folder (createhomedir $account)" \
        sudo createhomedir -c -u "$account"
    # This is the cause the lane's password journey failed on every macOS
    # nightly from 2026-10-31 to 2026-11-14 (with-hide/hide#2554): macOS
    # keeps a list of who Remote Login may sign in, its sshd's PAM
    # configuration asks that list on every connection (`pam_sacl.so
    # sacl_service=ssh` in /etc/pam.d/sshd), and a server on a runner whose
    # Remote Login is on refuses an account that is not on the list before
    # ssh is ever asked for a password. The client then sees only
    # `Connection closed by 127.0.0.1 port 2222`, with no cause on either
    # side. A runner with Remote Login off has no list at all, which allows
    # everyone and needs no edit.
    add_to_ssh_service_acl
    # The account is proven here rather than left to the lane an hour later:
    # an account that holds no password fails sshd every time, and says
    # nothing the lane's own log names. This reads the record, not the
    # password, so the password still never reaches a command line.
    run_named "the password account holds a password ($account)" \
        account_holds_a_password
    printf -- '-> %s\n' 'the Remote Login list this account is on'
    on_ssh_service_acl || printf 'prepare-ssh-accounts: %s is not on the Remote Login list\n' "$account"
fi

config="$temp/password-sshd"
run_named 'the password sshd folder' mkdir -p "$config"
run_named 'the password sshd host key' ssh-keygen -q -t ed25519 -N '' -f "$config/host"
run_named 'the password sshd configuration' write_sshd_config "$config"
run_named 'the sshd host key and configuration belong to root' \
    sudo chown root "$config/host" "$config/sshd_config"
run_named 'the password sshd starts' \
    sudo /usr/sbin/sshd -f "$config/sshd_config" -E "$config/sshd.log"
# The server's own log is the only place a refused sign-in says why, and the
# test that runs later cannot read it: it is root's file under the runner's
# temporary folder, and the test's own failure carries ssh's side alone
# (with-hide/hide#2554). The workflow's lane step prints the tail when its
# run failed, and a passing run reads none of it.
echo "HIDE_E2E_SSH_PASSWORD_SSHD_LOG=$config/sshd.log" >>"$GITHUB_ENV"

echo "HIDE_E2E_SSH_PASSWORD=$account:$password:2222" >>"$GITHUB_ENV"
printf 'prepare-ssh-accounts: ready in %s s (budget %s s)\n' \
    "$((SECONDS - started))" "$budget"

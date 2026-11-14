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
# Two limits, because one command's time and the run's are different claims:
# `PREPARE_COMMAND_SECONDS` (default 45) bounds a single command, and
# `PREPARE_BUDGET_SECONDS` (default 150) bounds all of them together, sized
# under the workflow's 3-minute step limit. A command gets whichever is left
# of the run's budget, so a hang late in the run cannot take the whole step
# either. The summary line at the end says how long the run took and what it
# was allowed.
#
# The runner's system decides how the account is made: `RUNNER_OS` as the
# workflow sees it, and uname when that is unset, so the script can also be
# run by hand. It creates an account and starts a server, so run it only
# where that is disposable.
#
# Usage: prepare-ssh-accounts.sh   (on a GitHub-hosted Linux or macOS runner)
set -euo pipefail

command_seconds=${PREPARE_COMMAND_SECONDS:-45}
budget=${PREPARE_BUDGET_SECONDS:-150}
started=$SECONDS
account=hidepw

for value in "$command_seconds" "$budget"; do
    [[ $value =~ ^[0-9]+$ ]] && (( value > 0 )) || {
        printf 'prepare-ssh-accounts: a time limit must be a positive whole number of seconds, not %s\n' "$value" >&2
        exit 2
    }
done

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

# The whole run's remaining time, and the time one command may take: its own
# limit, or what the run has left when that is less.
left() { echo $(( budget - (SECONDS - started) )); }
limit_for_command() {
    local limit
    limit=$(left)
    (( limit > 0 )) || limit=1
    (( limit < command_seconds )) || limit=$command_seconds
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

write_sshd_config() {
    local config=$1
    printf '%s\n' \
        "ListenAddress 127.0.0.1:2222" "HostKey $config/host" "PidFile $config/sshd.pid" \
        "PasswordAuthentication yes" "KbdInteractiveAuthentication no" "PubkeyAuthentication no" \
        "UsePAM yes" >"$config/sshd_config"
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
    run_named "the password account (sysadminctl -addUser $account)" \
        sudo sysadminctl -addUser "$account" -password "$password" -home "/Users/$account" -shell /bin/sh
    run_named "the password account's home folder (createhomedir $account)" \
        sudo createhomedir -c -u "$account"
fi

config="$temp/password-sshd"
run_named 'the password sshd folder' mkdir -p "$config"
run_named 'the password sshd host key' ssh-keygen -q -t ed25519 -N '' -f "$config/host"
run_named 'the password sshd configuration' write_sshd_config "$config"
run_named 'the sshd host key and configuration belong to root' \
    sudo chown root "$config/host" "$config/sshd_config"
run_named 'the password sshd starts' \
    sudo /usr/sbin/sshd -f "$config/sshd_config" -E "$config/sshd.log"

echo "HIDE_E2E_SSH_PASSWORD=$account:$password:2222" >>"$GITHUB_ENV"
printf 'prepare-ssh-accounts: ready in %s s (budget %s s, %s s a command)\n' \
    "$((SECONDS - started))" "$budget" "$command_seconds"

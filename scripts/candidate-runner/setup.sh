#!/bin/bash
# Makes this Mac the release candidate's macOS builder (docs/BUILD.md, The
# candidate's Mac build): a standard account of its own runs a GitHub Actions
# runner of with-hide/hide from a LaunchDaemon, so it starts without anyone
# logging in, and the hooks beside this script are installed where that
# account cannot change them.
#
# The Mac's administrator runs it from a copy of this folder:
#
#     sudo bash setup.sh
#
# and pastes the registration token GitHub shows under the repository's
# Settings, Actions, Runners, New self-hosted runner. A second run converges:
# it keeps the account, the runner and its registration, and replaces the
# hooks or the LaunchDaemon only when they differ from these. It changes
# nothing of the administrator's own account (their runners, services and
# Rust toolchain), and checks that before it ends.
set -euo pipefail

account=hiderunner
label=hide-candidate
service=dev.withhide.candidate-runner
hooks=/usr/local/libexec/hide-candidate-runner
daemon="/Library/LaunchDaemons/$service.plist"
runner_version=2.338.2
runner_sha256=25a065af409ffa317ab4d5a3b614dd15fc9c40f2172bc1b35a1b1258e919a388
rustup_version=1.29.2
rustup_sha256=f7c7e11eb2e07f006e532a8aaac77b32f4527d305ac386c0c633ca6133d35ca9
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

fail() { echo "setup: $*" >&2; exit 1; }
[[ $EUID -eq 0 ]] || fail "run it with sudo: sudo bash $0"
[[ "$(uname -sm)" == "Darwin arm64" ]] || fail "this sets up an Apple Silicon Mac"
operator="${SUDO_USER:-}"
[[ -n "$operator" && "$operator" != root ]] || fail "run it with sudo from the administrator's own account"
[[ "$operator" != "$account" ]] || fail "run it from the administrator's account, not $account"
home_of() { local name="$1"; dscl . -read "/Users/$name" NFSHomeDirectory | awk '{ print $2 }'; }
operator_home="$(home_of "$operator")"
operator_uid="$(id -u "$operator")"
# The runner's account cannot enter the administrator's home, where this copy
# usually sits, and rustup-init refuses to start in a folder it cannot read.
cd /

# What this setup must leave as it found it: the administrator's launchd
# services and their files (the hide-private runner and the Hermes gateways
# among them), that runner's configuration, and their Rust toolchain.
operator_state() {
  {
    launchctl print "gui/$operator_uid" | awk '$NF ~ /^(actions\.runner\.|ai\.hermes\.)/ { print "loaded " $NF }'
    find "$operator_home/Library/LaunchAgents" -maxdepth 1 -type f \( -name 'actions.runner.*' -o -name 'ai.hermes.*' \) \
      -exec shasum -a 256 {} +
    find "$operator_home/.cargo/bin" -maxdepth 1 -type f -exec shasum -a 256 {} +
    find "$operator_home" -maxdepth 2 -path "$operator_home/actions-runner*" -type f \
      \( -name .runner -o -name .env -o -name .path \) -exec shasum -a 256 {} +
  } 2>/dev/null | sort -u | shasum -a 256
}
before="$(operator_state)"

staging="$(mktemp -d /private/tmp/hide-candidate-runner.XXXXXX)"
trap 'rm -rf "$staging"' EXIT
chmod 755 "$staging"
download() {
  curl -fsSL --retry 3 -o "$3" "$1"
  echo "$2  $3" | shasum -a 256 -c - >/dev/null || fail "$1 does not have the pinned digest"
}

# The account: standard, hidden from the login window, with a password no one
# knows and no login shell.
if ! id "$account" >/dev/null 2>&1; then
  sysadminctl -addUser "$account" -fullName "Hide release candidate runner" \
    -password "$(openssl rand -base64 33)" -shell /usr/bin/false
  id "$account" >/dev/null 2>&1 || fail "sysadminctl did not create $account"
  dscl . -create "/Users/$account" IsHidden 1
fi
if dseditgroup -o checkmember -m "$account" admin >/dev/null 2>&1; then
  fail "$account is an administrator; the runner's account must be a standard one"
fi
home="$(home_of "$account")"
case "$home" in "$operator_home" | "$operator_home"/*) fail "$account's home $home is inside $operator's" ;; esac
install -d -o "$account" -g staff -m 700 "$home"
runner="$home/actions-runner"
# Nothing of another account's, such as Homebrew, is on the jobs' PATH.
path="$home/.cargo/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"
as_runner() { sudo -u "$account" -H "$@"; }

if [[ ! -x "$runner/config.sh" ]]; then
  download "https://github.com/actions/runner/releases/download/v$runner_version/actions-runner-osx-arm64-$runner_version.tar.gz" \
    "$runner_sha256" "$staging/runner.tar.gz"
  install -d -o "$account" -g staff -m 700 "$runner"
  tar -xzf "$staging/runner.tar.gz" -C "$runner"
  # Where the runner's own `svc.sh install` puts it, outside bin/, which the
  # runner replaces when it updates itself.
  cp "$runner/bin/runsvc.sh" "$runner/runsvc.sh"
  chown -R "$account:staff" "$runner"
fi

# rustup only: each job's checkout names its toolchain (rust-toolchain.toml),
# which rustup installs on first use and the job-completed hook keeps alone.
if [[ ! -x "$home/.cargo/bin/rustup" ]]; then
  download "https://static.rust-lang.org/rustup/archive/$rustup_version/aarch64-apple-darwin/rustup-init" \
    "$rustup_sha256" "$staging/rustup-init"
  chmod 755 "$staging/rustup-init"
  as_runner env CARGO_HOME="$home/.cargo" RUSTUP_HOME="$home/.rustup" \
    "$staging/rustup-init" -y --no-modify-path --profile minimal --default-toolchain none
fi

# Root's, so no job of the account can change what refuses or cleans a job;
# installed on every run, so an owner or mode changed since is set back too.
install -d -o root -g wheel -m 755 "$hooks"
for hook in job-started.sh job-completed.sh; do
  install -o root -g wheel -m 755 "$here/$hook" "$hooks/$hook"
done

if [[ ! -f "$runner/.runner" ]]; then
  read -rsp "Registration token (with-hide/hide, Settings, Actions, Runners, New self-hosted runner): " token
  echo
  [[ -n "$token" ]] || fail "no registration token"
  # The runner reads ACTIONS_RUNNER_INPUT_TOKEN as --token, which keeps it out
  # of the process list; config.sh records PATH as the jobs' PATH (.path).
  (cd "$runner" && ACTIONS_RUNNER_INPUT_TOKEN="$token" as_runner --preserve-env=ACTIONS_RUNNER_INPUT_TOKEN \
    env PATH="$path" LANG=en_US.UTF-8 ./config.sh --unattended --replace \
    --url https://github.com/with-hide/hide --name "hide-candidate-$(scutil --get LocalHostName)" \
    --labels "$label" --work _work)
  unset token
fi

# CARGO_HOME is under the work folder, so the cargo registry goes with each
# job's other files; rustup's own home stays in the account's.
cat > "$staging/daemon.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$service</string>
  <key>UserName</key><string>$account</string>
  <key>GroupName</key><string>staff</string>
  <key>WorkingDirectory</key><string>$runner</string>
  <key>ProgramArguments</key><array><string>$runner/runsvc.sh</string></array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>HOME</key><string>$home</string>
    <key>PATH</key><string>$path</string>
    <key>LANG</key><string>en_US.UTF-8</string>
    <key>ACTIONS_RUNNER_SVC</key><string>1</string>
    <key>ACTIONS_RUNNER_HOOK_JOB_STARTED</key><string>$hooks/job-started.sh</string>
    <key>ACTIONS_RUNNER_HOOK_JOB_COMPLETED</key><string>$hooks/job-completed.sh</string>
    <key>RUSTUP_HOME</key><string>$home/.rustup</string>
    <key>CARGO_HOME</key><string>$runner/_work/_cache/cargo</string>
  </dict>
  <key>RunAtLoad</key><true/>
  <key>SessionCreate</key><true/>
  <key>StandardOutPath</key><string>$runner/_diag/launchd.log</string>
  <key>StandardErrorPath</key><string>$runner/_diag/launchd.log</string>
</dict>
</plist>
PLIST
plutil -lint "$staging/daemon.plist" >/dev/null
if ! cmp -s "$staging/daemon.plist" "$daemon"; then
  if pgrep -U "$account" -f Runner.Worker >/dev/null; then
    fail "a job is running on the runner; run this again after it ends"
  fi
  launchctl bootout "system/$service" 2>/dev/null || true
  install -o root -g wheel -m 644 "$staging/daemon.plist" "$daemon"
fi
launchctl print "system/$service" >/dev/null 2>&1 || launchctl bootstrap system "$daemon"

[[ "$(operator_state)" == "$before" ]] || fail "something of $operator's changed during setup; compare their LaunchAgents, runner and ~/.cargo"
sleep 5
echo
echo "account:  $account (standard, home $home)"
echo "runner:   $(sed -n 's/.*"agentName": *"\([^"]*\)".*/\1/p' "$runner/.runner" | tr -d '\357\273\277'), labels self-hosted, macOS, ARM64, $label"
echo "service:  $(launchctl print "system/$service" | awk '$1 == "state" { print $3 }')"
echo "hooks:    $hooks"
echo "disk:     $(df -h "$home" | awk 'NR == 2 { print $4 }') free"
echo "$operator's runners, services and Rust toolchain: unchanged"

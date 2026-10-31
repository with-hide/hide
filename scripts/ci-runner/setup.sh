#!/bin/bash
# Makes this Mac a CI runner of with-hide/hide for the macOS jobs that need no
# screen, keychain or login session (docs/BUILD.md, The Mac mini CI runner): a GitHub Actions runner labelled `hide-ci`, run by a LaunchAgent of
# the account that runs this, with the hooks beside this script.
#
# Run it from a copy of this folder, as the account the runner should use:
#
#     bash setup.sh
#
# and paste a registration token (`gh api -X POST
# repos/with-hide/hide/actions/runners/registration-token --jq .token`), or
# pass it as ACTIONS_RUNNER_INPUT_TOKEN. A second run converges: it keeps the
# runner and its registration, and replaces the hooks or the LaunchAgent only
# when they differ from these. Everything the runner keeps is under one
# folder, ~/actions-runner-hide-ci; the jobs get their own HOME, TMPDIR,
# rustup and cargo homes inside it, so they never write to the account's own.
set -euo pipefail

label=hide-ci
service=dev.withhide.ci-runner
runner="$HOME/actions-runner-hide-ci"
agent="$HOME/Library/LaunchAgents/$service.plist"
runner_version=2.338.2
runner_sha256=25a065af409ffa317ab4d5a3b614dd15fc9c40f2172bc1b35a1b1258e919a388
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

fail() { echo "setup: $*" >&2; exit 1; }
[[ $EUID -ne 0 ]] || fail "run it as the runner's account, without sudo"
[[ "$(uname -sm)" == "Darwin arm64" ]] || fail "this sets up an Apple Silicon Mac"
# The jobs reach their pinned toolchain through the account's rustup proxies;
# the toolchains themselves go to the runner's own rustup home.
[[ -x "$HOME/.cargo/bin/rustup" ]] || fail "install rustup for this account first (rustup-init --default-toolchain none)"
uid="$(id -u)"
# The account's Homebrew gives the jobs zstd for the Actions cache and a
# current python3.
path="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"

staging="$(mktemp -d)"
trap 'rm -rf "$staging"' EXIT
if [[ ! -x "$runner/config.sh" ]]; then
  curl -fsSL --retry 3 -o "$staging/runner.tar.gz" \
    "https://github.com/actions/runner/releases/download/v$runner_version/actions-runner-osx-arm64-$runner_version.tar.gz"
  echo "$runner_sha256  $staging/runner.tar.gz" | shasum -a 256 -c - >/dev/null || fail "the runner archive does not have the pinned digest"
  install -d -m 700 "$runner"
  tar -xzf "$staging/runner.tar.gz" -C "$runner"
  # Where the runner's own `svc.sh install` puts it, outside bin/, which the
  # runner replaces when it updates itself.
  cp "$runner/bin/runsvc.sh" "$runner/runsvc.sh"
fi
install -d -m 700 "$runner/hooks" "$runner/home" "$runner/tmp" "$runner/rustup"
for hook in job-started.sh job-completed.sh; do
  install -m 755 "$here/$hook" "$runner/hooks/$hook"
done

if [[ ! -f "$runner/.runner" ]]; then
  token="${ACTIONS_RUNNER_INPUT_TOKEN:-}"
  if [[ -z "$token" ]]; then
    read -rsp "Registration token (gh api -X POST repos/with-hide/hide/actions/runners/registration-token --jq .token): " token
    echo
  fi
  [[ -n "$token" ]] || fail "no registration token"
  # The runner reads ACTIONS_RUNNER_INPUT_TOKEN as --token, which keeps it out
  # of the process list; config.sh records PATH as the jobs' PATH (.path).
  (cd "$runner" && ACTIONS_RUNNER_INPUT_TOKEN="$token" env PATH="$path" LANG=en_US.UTF-8 ./config.sh --unattended \
    --url https://github.com/with-hide/hide --name "hide-ci-$(scutil --get LocalHostName)" \
    --labels "$label" --work _work)
  unset token
fi

# Nice 10: the Hermes gateways on the same Mac come first when the cores are
# busy. CARGO_HOME and hide-vt's inputs are under the work folder, so they go
# with each job's other files.
cat > "$staging/agent.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$service</string>
  <key>WorkingDirectory</key><string>$runner</string>
  <key>ProgramArguments</key><array><string>$runner/runsvc.sh</string></array>
  <key>EnvironmentVariables</key>
  <dict>
    <key>HIDE_CI_RUNNER</key><string>$runner</string>
    <key>HOME</key><string>$runner/home</string>
    <key>TMPDIR</key><string>$runner/tmp/</string>
    <key>PATH</key><string>$path</string>
    <key>LANG</key><string>en_US.UTF-8</string>
    <key>ACTIONS_RUNNER_SVC</key><string>1</string>
    <key>ACTIONS_RUNNER_HOOK_JOB_STARTED</key><string>$runner/hooks/job-started.sh</string>
    <key>ACTIONS_RUNNER_HOOK_JOB_COMPLETED</key><string>$runner/hooks/job-completed.sh</string>
    <key>RUSTUP_HOME</key><string>$runner/rustup</string>
    <key>CARGO_HOME</key><string>$runner/_work/_cache/cargo</string>
    <key>HIDE_BUILD_CACHE</key><string>$runner/_work/_cache/hide-build</string>
  </dict>
  <key>RunAtLoad</key><true/>
  <key>Nice</key><integer>10</integer>
  <key>StandardOutPath</key><string>$runner/_diag/launchd.log</string>
  <key>StandardErrorPath</key><string>$runner/_diag/launchd.log</string>
</dict>
</plist>
PLIST
plutil -lint "$staging/agent.plist" >/dev/null
if ! cmp -s "$staging/agent.plist" "$agent"; then
  if pgrep -U "$uid" -f "$runner/bin[^/]*/Runner.Worker" >/dev/null; then
    fail "a job is running on the runner; run this again after it ends"
  fi
  launchctl bootout "gui/$uid/$service" 2>/dev/null || true
  install -m 644 "$staging/agent.plist" "$agent"
fi
launchctl print "gui/$uid/$service" >/dev/null 2>&1 || launchctl bootstrap "gui/$uid" "$agent"

sleep 5
echo "runner:   $(sed -n 's/.*"agentName": *"\([^"]*\)".*/\1/p' "$runner/.runner" | tr -d '\357\273\277'), labels self-hosted, macOS, ARM64, $label"
echo "service:  $(launchctl print "gui/$uid/$service" | awk '$1 == "state" { print $3; exit }')"
echo "disk:     $(df -h "$runner" | awk 'NR == 2 { print $4 }') free"

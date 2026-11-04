#!/usr/bin/env bash
# Isolation boundary for web shell measurements (PRD B8, B12).
# Source from the subshell that owns the private server; never export these
# values back into an operator pane. Scrubs inherited Herdr routing and points
# every child at a short private socket under /tmp with state under the run
# directory. The operator socket is never attached to; it is only read for
# the before/after topology counts.
set -euo pipefail

measure_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
worktree_root="$(cd "$measure_dir/../.." && pwd)"
run_dir="${HIDE_MEASURE_RUN_DIR:?set HIDE_MEASURE_RUN_DIR to a directory under agents/runs/<slug>/}"
mkdir -p "$run_dir" "$run_dir/logs" "$run_dir/fixture" "$run_dir/chrome-profile"
run_dir="$(cd "$run_dir" && pwd)"

operator_socket="${HOME}/.config/herdr/herdr.sock"
# The core's own engine (MEASURE_ENGINE=own) runs no Herdr, so it needs none.
herdr_bin=""
if [[ "${MEASURE_ENGINE:-herdr}" == herdr ]]; then
  manifest="$worktree_root/contracts/herdr-bundle.json"
  pin_version="$(jq -er '.version' "$manifest")"
  if [[ -n "${HERDR_BIN_PATH:-}" && -x "${HERDR_BIN_PATH}" ]]; then
    herdr_bin="$HERDR_BIN_PATH"
  else
    herdr_bin="$(command -v herdr)"
  fi
  herdr_version="$("$herdr_bin" --version | awk '{print $2}')"
  if [[ "$herdr_version" != "$pin_version" ]]; then
    printf 'isolated-env: herdr %s is not pin %s (%s)\n' "$herdr_version" "$pin_version" "$herdr_bin" >&2
    exit 2
  fi
fi

private="$run_dir/isolated"
mkdir -p "$private/xdg-config" "$private/xdg-state" "$private/home" "$private/hide-state"
# The delivery ledger refuses an existing state directory readable by
# another account. A daemon without its ledger measures failing hooks.
chmod 700 "$private/hide-state"
printf "PS1='fixture %%# '\n" > "$private/home/.zshrc"
config_path="$private/herdr-config.toml"
printf '[update]\nversion_check = false\nmanifest_check = false\n' > "$config_path"

unset HERDR_PANE_ID HERDR_TAB_ID HERDR_WORKSPACE_ID HERDR_ENV
# Herdr starts the pane shell from SHELL; only zsh reads the private .zshrc
# whose prompt run.sh waits for.
export SHELL=/bin/zsh
export HERDR_SESSION="hide-web-measure"
export HERDR_DISABLE_SOUND=1
export HERDR_CONFIG_PATH="$config_path"
export XDG_CONFIG_HOME="$private/xdg-config"
export XDG_STATE_HOME="$private/xdg-state"
export HERDR_BIN_PATH="$herdr_bin"
export MEASURE_RUN_DIR="$run_dir"
export MEASURE_WORKTREE="$worktree_root"
export MEASURE_OPERATOR_SOCKET="$operator_socket"
export MEASURE_FIXTURE="$run_dir/fixture"
export MEASURE_PRIVATE="$private"
# Keep the socket name short for Unix-domain limits and private to this run.
# Creation is last so preflight failures do not leave a socket directory.
socket_dir="$(mktemp -d /tmp/hm.XXXXXXXX)"
export MEASURE_SOCKET_DIR="$socket_dir"
export HERDR_SOCKET_PATH="$socket_dir/herdr.sock"

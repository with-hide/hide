#!/usr/bin/env bash
# Isolation boundary for web shell measurements (PRD B8, B12).
# Source from the subshell that owns the run; never export these values back
# into an operator pane. Scrubs the routing a Hide pane hands its programs and
# points every child at a private HOME and XDG folders under the run
# directory, with the daemon's state in a short private folder under /tmp.
set -euo pipefail

measure_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
worktree_root="$(cd "$measure_dir/../.." && pwd)"
run_dir="${HIDE_MEASURE_RUN_DIR:?set HIDE_MEASURE_RUN_DIR to a directory under agents/runs/<slug>/}"
mkdir -p "$run_dir" "$run_dir/logs" "$run_dir/fixture" "$run_dir/chrome-profile"
run_dir="$(cd "$run_dir" && pwd)"

private="$run_dir/isolated"
mkdir -p "$private/xdg-config" "$private/xdg-state" "$private/home" "$private/hide-state"
# The delivery ledger refuses an existing state directory readable by
# another account. A daemon without its ledger measures failing hooks.
chmod 700 "$private/hide-state"
printf "PS1='fixture %%# '\n" > "$private/home/.zshrc"

unset HIDE_PANE_ID HIDE_CAP_REF HERDR_PANE_ID HERDR_TAB_ID HERDR_WORKSPACE_ID HERDR_ENV HERDR_SOCKET_PATH HERDR_BIN_PATH
# The pane holder starts the pane shell from SHELL; only zsh reads the
# private .zshrc whose prompt run.sh waits for.
export SHELL=/bin/zsh
export XDG_CONFIG_HOME="$private/xdg-config"
export XDG_STATE_HOME="$private/xdg-state"
export MEASURE_RUN_DIR="$run_dir"
export MEASURE_WORKTREE="$worktree_root"
export MEASURE_FIXTURE="$run_dir/fixture"
export MEASURE_PRIVATE="$private"
# Keep the holder socket paths short for Unix-domain limits and private to
# this run. Creation is last so preflight failures do not leave a folder.
socket_dir="$(mktemp -d /tmp/hm.XXXXXXXX)"
export MEASURE_SOCKET_DIR="$socket_dir"

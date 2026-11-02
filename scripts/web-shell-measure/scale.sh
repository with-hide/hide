#!/usr/bin/env bash
# Fills the private Herdr server with the topology of a scale (PRD
# instant-pane-topology D-18) around the measured workspace, which already
# holds the measured tab and the tab switch target:
#   operator: 43 workspaces, 62 tabs, 66 panes, 30 agents, 5 printing
#   double:   86 workspaces, 124 tabs, 132 panes, 60 agents, 10 printing
# Each other workspace is its own Git checkout, so the catalog has as many
# projects as workspaces. Agents are reported with `herdr pane report-agent`;
# output starts only after the idle observation, with one Python printer per pane.
# Usage (from hided/tests/it/support/remote_delivery/responsive.rs, beside a private Herdr server; run.sh makes its topology with fixture.mjs):
#   scale.sh build <scale>    -> writes scale.json
#   scale.sh drive           -> starts the configured printers
#   scale.sh churn            -> flips one agent's state every 2 s, forever
set -euo pipefail
# Only ever the run's private server: never the operator's.
[[ "${HERDR_SOCKET_PATH:?}" == "${MEASURE_SOCKET_DIR:?}"/* ]] || { echo "scale.sh runs only against the run's private Herdr socket" >&2; exit 2; }
herdr="${HERDR_BIN_PATH:?}"
run_dir="${MEASURE_RUN_DIR:?}"
agents_file="$run_dir/scale-agents.txt"

json_field() { python3 -c "import json,sys; d=json.load(sys.stdin)['result']; print(d$1)"; }

build() {
  local scale=$1
  local workspaces tabs panes agents printing
  case "$scale" in
    operator) workspaces=43 tabs=62 panes=66 agents=30 printing=5 ;;
    double) workspaces=86 tabs=124 panes=132 agents=60 printing=10 ;;
    *) echo "scale must be operator or double" >&2; exit 2 ;;
  esac
  # The measured workspace holds two tabs of one pane each.
  local other_workspaces=$((workspaces - 1))
  local extra_tabs=$((tabs - 2 - other_workspaces))
  local extra_panes=$((panes - tabs - ${MEASURE_PREEXISTING_SPLITS:-0}))
  local root="$run_dir/scale"
  mkdir -p "$root"
  local ids=() tab_panes=() all_panes=()
  for ((n = 1; n <= other_workspaces; n++)); do
    local repo="$root/project-$n"
    if [[ ! -d "$repo/.git" ]]; then
      mkdir -p "$repo"
      git -C "$repo" init --quiet
      printf 'scale project %s\n' "$n" > "$repo/README"
      git -C "$repo" add README
      git -C "$repo" -c commit.gpgsign=false -c user.email="measure@example.invalid" -c user.name="measure" commit --quiet -m "scale project"
    fi
    if [[ "${MEASURE_RICH_CATALOG:-0}" == 1 ]]; then
      for ((branch=0; branch<40; branch++)); do git -C "$repo" branch "feature-$branch"; done
      git -C "$repo" remote add origin https://github.com/fixture/project.git
      git -C "$repo" worktree add --quiet -b checkout-feature "$root/checkout-$n"
    fi
    local created
    created="$("$herdr" workspace create --cwd "$repo" --label "project-$n" --no-focus)"
    ids+=("$(printf %s "$created" | json_field '["workspace"]["workspace_id"]')")
    local pane
    pane="$(printf %s "$created" | json_field '["root_pane"]["pane_id"]')"
    tab_panes+=("$pane")
    all_panes+=("$pane")
  done
  for ((n = 0; n < extra_tabs; n++)); do
    local workspace=${ids[$((n % other_workspaces))]}
    local created
    created="$("$herdr" tab create --workspace "$workspace" --label "extra-$n" --no-focus)"
    all_panes+=("$(printf %s "$created" | json_field '["root_pane"]["pane_id"]')")
  done
  for ((n = 0; n < extra_panes; n++)); do
    all_panes+=("$("$herdr" pane split "${tab_panes[$n]}" --direction right --no-focus | json_field '["pane"]["pane_id"]')")
  done
  : > "$agents_file"
  : > "$run_dir/scale-printing.txt"
  for ((n = 0; n < agents; n++)); do
    local pane=${all_panes[$n]} state=idle
    "$herdr" pane report-agent "$pane" --source measure-scale --agent claude --state "$state" >/dev/null
    printf '%s %s\n' "$pane" "$state" >> "$agents_file"
    if ((n < printing)); then printf '%s\n' "$pane" >> "$run_dir/scale-printing.txt"; fi
  done
  # The counts the private server answers beside the ones this scale asks
  # for, so a reader can tell the measured fixture from the intended one.
  "$herdr" api snapshot | python3 -c '
import json, sys
d = json.load(sys.stdin); r = d.get("result") or d; s = r.get("snapshot") or r
print(json.dumps({"scale": sys.argv[1], "workspaces": len(s.get("workspaces") or []), "tabs": len(s.get("tabs") or []), "panes": len(s.get("panes") or []), "agents_reported": int(sys.argv[2]), "printing": 0, "printing_targets": int(sys.argv[3]),
                  "target": {"workspaces": int(sys.argv[4]), "tabs": int(sys.argv[5]), "panes": int(sys.argv[6])}}))
' "$scale" "$agents" "$printing" "$workspaces" "$tabs" "$panes" > "$run_dir/scale.json"
  cat "$run_dir/scale.json"
}

drive() {
  while read -r pane; do
    "$herdr" pane run "$pane" "python3 -u -c 'import time,collections; collections.deque(((print(\"fixture output %08d\" % i, flush=True),time.sleep(0.05)) for i in range(20000)),maxlen=0)'" >/dev/null
  done < "$run_dir/scale-printing.txt"
}

churn() {
  # One status change every two seconds, round robin over the idle agents,
  # the agent-status-only publish D-18 prices.
  local idle=()
  while read -r pane state; do [[ "$state" == idle ]] && idle+=("$pane"); done < "$agents_file"
  ((${#idle[@]} > 0)) || exit 0
  local n=0 state=working
  while :; do
    "$herdr" pane report-agent "${idle[$((n % ${#idle[@]}))]}" --source measure-scale --agent claude --state "$state" >/dev/null || true
    n=$((n + 1))
    if ((n % ${#idle[@]} == 0)); then [[ "$state" == working ]] && state=idle || state=working; fi
    sleep 2
  done
}

case "${1:-}" in
  build) build "$2" ;;
  drive) drive ;;
  churn) churn ;;
  *) echo "usage: scale.sh build <operator|double> | drive | churn" >&2; exit 2 ;;
esac

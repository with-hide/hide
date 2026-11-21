#!/usr/bin/env bash
# Owner of every isolated fixture, daemon, pane holder, browser and driver for
# the web shell's performance measurements against the product hided. Usage:
#   MEASURE_SCENARIO=topology HIDE_MEASURE_RUN_DIR=agents/runs/<slug>/measure bash scripts/web-shell-measure/run.sh
# Every scenario runs its panes on the core's own engine: the measured hided
# over the `hide-holder` beside it. The fixture is made with the web shell's
# events on the daemon's WebSocket (fixture.mjs), input is typed through the
# page (type.mjs, CDP), and the saved layout is watched while the topology
# rounds run (save-watch.py). A change is judged against the build before it
# with the same pair method (docs/PERFORMANCE_TESTING.md).
# `topology` (PRD instant-pane-topology D-15, the default) adds a second tab to
# the measured workspace and, with MEASURE_SCALE=operator or double, fills the
# session to that scale (fixture.mjs, which reports no agents and starts the
# scale's printing panes); it then records idle resources, one echo trial,
# and the split, zoom, close, new tab, tab switch and resize latencies
# (topology.mjs) while it counts the layout saves (save-watch.py).
# `keys` (PRD core-host-node-terminal D-07, D-08, B2, B4, B5, B19, B21) is
# the measured pane and four splits in one tab, five attached panes: idle
# resources for MEASURE_IDLE_SECONDS (60), the screen key echo idle
# (key-echo.mjs), then every pane printing a line per 8 ms while resources
# are sampled for MEASURE_DRIVEN_SECONDS (120), then the key echo again on
# the measured pane while the other four keep printing,
# then MEASURE_KEY_COUNT (0: skipped) distinct keys counted back from the
# pane (key-count.mjs), then the window closed while the panes print for
# MEASURE_WINDOWLESS_SECONDS (60) and reopened.
# `restore` (PRD engine-bundle-3-core-owns B44, perf-gate row 30) builds the
# topology fixture, opens the page on it, kills every process the run started
# with SIGKILL (the daemon, every pane holder, and every pane's shell),
# confirms none runs, starts the daemon again and times the tabs, the front
# tab's panes and every shell coming back (restore.mjs).
# MEASURE_HIDED_BIN measures another hided build, such as a baseline, with
# the same fixture; its hide-holder sits beside it.
# Needs: Google Chrome, and target/release/hided and hide-holder built after
# `pnpm --dir web build` (docs/BUILD.md: a release hided embeds web/dist).
set -euo pipefail
# Optional unattended mode: no native window.
isolated_headless=false
fixture_only=false
case "${1:-}" in
  '') ;;
  --isolated-headless) isolated_headless=true ;;
  *) echo 'usage: run.sh [--isolated-headless] [--fixture-only]' >&2; exit 2 ;;
esac
case "${2:-}" in
  '') ;;
  --fixture-only) fixture_only=true ;;
  *) echo 'second argument must be --fixture-only' >&2; exit 2 ;;
esac
measure_dir="$(cd "$(dirname "$0")" && pwd)"
source "$measure_dir/isolated-env.sh"
trap 'rmdir "$MEASURE_SOCKET_DIR/state" 2>/dev/null; rmdir "$MEASURE_SOCKET_DIR"' EXIT
scenario="${MEASURE_SCENARIO:-topology}"
case "$scenario" in keys|topology|restore) ;; *) echo "MEASURE_SCENARIO must be keys, topology or restore" >&2; exit 2;; esac
scale="${MEASURE_SCALE:-none}"
case "$scale" in none|operator|double) ;; *) echo "MEASURE_SCALE must be none, operator or double" >&2; exit 2;; esac
[[ "$scale" == none || "$scenario" != keys ]] || { echo "MEASURE_SCALE needs MEASURE_SCENARIO=topology or restore" >&2; exit 2; }
chrome_bin="${MEASURE_CHROME_BIN:-/Applications/Google Chrome.app/Contents/MacOS/Google Chrome}"
hided_bin="${MEASURE_HIDED_BIN:-$MEASURE_WORKTREE/target/release/hided}"
[[ -x "$hided_bin" ]] || { echo "build target/release/hided first: pnpm --dir web build, then the release build described in docs/BUILD.md" >&2; exit 1; }
# The daemon's node starts each pane's holder from the one beside hided.
[[ -x "$(dirname "$hided_bin")/hide-holder" ]] || { echo "hide-holder must sit beside $hided_bin" >&2; exit 1; }
[[ -x "$chrome_bin" ]] || { echo "Chrome not found at $chrome_bin" >&2; exit 1; }
pids=()
# The daemon's state folder sits in the run's short socket folder while the
# run lasts, since the holder sockets live under it and the run folder's path
# is longer than a Unix socket address takes (a link is refused as not
# private); the end moves it into the run folder as isolated/hide-state.
hide_state="$MEASURE_SOCKET_DIR/state"
mkdir -m 700 "$hide_state"
export MEASURE_HIDE_STATE="$hide_state"
note() { printf 'measure: %s\n' "$*"; }
# The pane holders outlive the daemon by design, so the run owns them by the
# pid each record in its state folder names, with the e2e fixture's rule
# (web/e2e/pane-holders.mjs, through holders.mjs): `holder_pids` lists the
# holders and their programs, `end_holders` ends them (failing when one still
# runs), `holders_left` prints what still runs from the run's holder folder
# (nothing, on a clean end: leaked holders once used up every PTY).
pane_holders="$measure_dir/holders.mjs"
holder_pids() { node "$pane_holders" pids "$hide_state"; }
end_holders() { node "$pane_holders" end "$hide_state"; }
holders_left() { node "$pane_holders" left "$hide_state" || true; }
cleanup() {
  local cleanup_status=0
  trap - EXIT INT TERM
  set +e
  kill_owned() {
    local pid=$1
    [[ -n "$pid" ]] || return 0
    [[ "$(ps -p "$pid" -o ppid= | tr -d ' ')" == "$$" ]] && kill "$pid" 2>/dev/null
  }
  for pid in "${pids[@]:-}"; do kill_owned "$pid"; done
  for pid in "${pids[@]:-}"; do [[ -n "$pid" ]] && wait "$pid" 2>/dev/null; done
  # A holder left running fails the run, so a battery of runs stops there.
  end_holders || cleanup_status=1
  left="$(holders_left)"
  [[ -z "$left" ]] || { printf 'pane holder processes left running:\n%s\n' "$left" >&2; cleanup_status=1; }
  rmdir "$MEASURE_PRIVATE/hide-state" && mv "$hide_state" "$MEASURE_PRIVATE/hide-state"
  rmdir "$MEASURE_SOCKET_DIR"
  {
    for pid in "${pids[@]:-}"; do
      [[ -n "$pid" ]] || continue
      ps -p "$pid" -o pid=,ppid=,comm= || printf '%s exited\n' "$pid"
    done
    printf 'holders left: %s\n' "$(holders_left | wc -l | tr -d ' ')"
  } > "$MEASURE_RUN_DIR/cleanup-processes.txt"
  (( cleanup_status == 0 )) || exit "$cleanup_status"
}
trap cleanup EXIT
trap 'exit 130' INT TERM
spawn_owned() {
  local name=$1; shift
  (( ${#pids[@]} < 8 )) || { echo 'process owner budget exceeded' >&2; exit 1; }
  "$@" >"$MEASURE_RUN_DIR/logs/$name.log" 2>"$MEASURE_RUN_DIR/logs/$name.stderr.log" &
  owned_pid=$!
  pids+=("$owned_pid")
}
wait_url() {
  local deadline=$((SECONDS+30))
  until curl -sf "$1" >/dev/null; do
    (( SECONDS < deadline )) || { echo "unready: $1" >&2; exit 1; }
    sleep 0.2
  done
}
wait_js() {
  local deadline=$((SECONDS+30))
  until [[ "$(node -e "
import('$measure_dir/cdp.mjs').then(async ({connectPage}) => { const p = await connectPage('$MEASURE_CDP_PORT'); console.log(await p.evaluate(process.argv[1])); p.close(); }).catch(() => console.log('unready'))" "$1")" == true ]]; do
    if (( SECONDS >= deadline )); then
      node --input-type=module - "$measure_dir/cdp.mjs" "$MEASURE_CDP_PORT" <<'JS' >&2
const { connectPage } = await import(process.argv[2]);
const page = await connectPage(process.argv[3]);
try {
  console.log(JSON.stringify(await page.evaluate(`({
    screen: document.querySelector('main')?.getAttribute('aria-label'),
    mainView: document.querySelector('[data-main-screen]')?.getAttribute('data-main-view'),
    overviewState: document.querySelector('[data-overview-screen]')?.getAttribute('data-overview-state'),
    projectRows: document.querySelectorAll('[data-main-project]').length,
    workspaceRows: document.querySelectorAll('[data-overview-workspace]').length,
    sidebarMode: document.querySelector('[data-sidebar]')?.getAttribute('data-sidebar'),
    sidebarProjects: document.querySelectorAll('[data-project-row]').length,
    sidebarCheckouts: document.querySelectorAll('[data-checkout]').length,
    checkoutKinds: [...document.querySelectorAll('[data-checkout-kind]')].map((row) => row.getAttribute('data-checkout-kind')),
    workspaceVisible: Boolean(document.querySelector('[data-workspace-screen]'))
  })`)));
} finally {
  page.close();
}
JS
      echo "browser unready: $1" >&2
      exit 1
    fi
    sleep 0.3
  done
}
reset_fixture() {
  # Typed through the page, as the operator types.
  node "$measure_dir/type.mjs" "$MEASURE_CDP_PORT" "$MEASURE_PANE_ID" --interrupt 'printf "\033c"; stty -echo -icanon; cat'
  sleep 0.8
}

{
  echo "worktree_head=$(git -C "$MEASURE_WORKTREE" rev-parse HEAD)"
  echo "worktree_dirty=$(git -C "$MEASURE_WORKTREE" status --porcelain | wc -l | tr -d ' ')"
  echo "hided_bin=$hided_bin"
  echo "hided_sha256=$(shasum -a 256 "$hided_bin" | awk '{print $1}')"
  echo "chrome_version=$("$chrome_bin" --version 2>/dev/null)"
  echo "scenario=$scenario"
  echo "scale=$scale"
  echo "isolated_headless=$isolated_headless"
  echo "fixture_only=$fixture_only"
  echo "started_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  uptime
} > "$MEASURE_RUN_DIR/identity.txt"

# The fixture is a primary Git checkout.
if [[ ! -d "$MEASURE_FIXTURE/.git" ]]; then
  git -C "$MEASURE_FIXTURE" init --quiet
  printf 'measure fixture\n' > "$MEASURE_FIXTURE/README"
  git -C "$MEASURE_FIXTURE" add README
  git -C "$MEASURE_FIXTURE" -c commit.gpgsign=false -c user.email="measure@example.invalid" -c user.name="measure" commit --quiet -m "measure fixture"
fi

# Every pane's shell touches one file as it reads its rc file (the restore's
# `shells_ms`), and a shell started in a printing pane's folder waits on its
# FIFO until `fixture.mjs drive`, then prints in the foreground: a background
# job would outlive its pane (fixture.mjs).
export MEASURE_READY_DIR="$MEASURE_RUN_DIR/shell-ready"
mkdir -p "$MEASURE_READY_DIR"
cat >> "$MEASURE_PRIVATE/home/.zshrc" <<ZSHRC
: >| "$MEASURE_READY_DIR/\$\$"
[[ -p .measure-go ]] && { read -r _ < .measure-go; /usr/bin/python3 -u -c '
import os, pathlib, time
for i in range(20000):
    print("fixture output %08d" % i, flush=True)
    if i == 0:
        pathlib.Path(".measure-printing").write_text(str(os.getpid()) + "\\n")
    time.sleep(0.05)
'; }
ZSHRC
# Product hided (release, embedded web/dist) on the private state folder.
start_hided() {
  rm -f "$hide_state/hided.json"
  spawn_owned "hided${1:-}" env HOME="$MEASURE_PRIVATE/home" HIDE_STATE_DIR="$hide_state" HIDE_KEEP_ALIVE=1 HIDE_PORT=0 "$hided_bin"
  hided_pid=$owned_pid
}
start_hided
deadline=$((SECONDS+20)); until [[ -f "$hide_state/hided.json" ]]; do (( SECONDS < deadline )) || { echo 'hided state file missing' >&2; exit 1; }; sleep 0.1; done
hided_port="$(python3 -c "import json;print(json.load(open('$hide_state/hided.json'))['port'])")"
hided_token="$(python3 -c "import json;print(json.load(open('$hide_state/hided.json'))['token'])")"
wait_url "http://127.0.0.1:$hided_port/health"
page_url="http://127.0.0.1:$hided_port/?probe=1#token=$hided_token"
note "making the $scale fixture with the web shell's events"
fixture_scale="$scale"
[[ "$scenario" != keys ]] || fixture_scale=keys
node "$measure_dir/fixture.mjs" build "$fixture_scale" > "$MEASURE_RUN_DIR/fixture-build.json"
extra_panes=()
if [[ "$scenario" == keys ]]; then
  while IFS= read -r pane; do extra_panes+=("$pane"); done < <(python3 -c 'import json,sys; print("\n".join(json.load(open(sys.argv[1]))["key_panes"]))' "$MEASURE_RUN_DIR/fixture.json")
fi
export MEASURE_PANE_ID="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["pane"])' "$MEASURE_RUN_DIR/fixture.json")"
export MEASURE_SWITCH_TAB="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["switch_tab"])' "$MEASURE_RUN_DIR/fixture.json")"
measure_tab="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["tab"])' "$MEASURE_RUN_DIR/fixture.json")"
measure_checkout_json="$(python3 -c 'import json,sys; print(json.dumps(json.load(open(sys.argv[1]))["checkout"]))' "$MEASURE_RUN_DIR/fixture.json")"
echo "engine=own" >> "$MEASURE_RUN_DIR/identity.txt"

port_file="$MEASURE_RUN_DIR/chrome-profile/DevToolsActivePort"
[[ ! -e "$port_file" ]] || { echo 'stale CDP port file; use a new run directory' >&2; exit 2; }
chrome_flags=()
if $isolated_headless; then chrome_flags+=(--headless=new); fi
spawn_owned chrome "$chrome_bin" "${chrome_flags[@]}" --user-data-dir="$MEASURE_RUN_DIR/chrome-profile" --remote-debugging-port=0 --remote-debugging-address=127.0.0.1 --no-first-run --no-default-browser-check --disable-sync --disable-background-networking --disable-component-update --disable-background-timer-throttling --disable-renderer-backgrounding --disable-backgrounding-occluded-windows --window-size=1280,900 about:blank
chrome_pid=$owned_pid
deadline=$((SECONDS+20)); until [[ -s "$port_file" ]]; do
  [[ "$(ps -p "$chrome_pid" -o ppid= | tr -d ' ')" == "$$" ]] || { echo 'owned Chrome exited before CDP became ready' >&2; exit 1; }
  (( SECONDS < deadline )) || { echo 'owned Chrome CDP port did not appear' >&2; exit 1; }
  sleep 0.1
done
export MEASURE_CDP_PORT="$(head -n 1 "$port_file")"
[[ "$MEASURE_CDP_PORT" =~ ^[0-9]+$ ]] || { echo 'owned Chrome CDP port is invalid' >&2; exit 1; }
wait_url "http://127.0.0.1:$MEASURE_CDP_PORT/json/list"
printf '%s' "$page_url" | node "$measure_dir/navigate.mjs" "$MEASURE_CDP_PORT"
# A first run opens on Main (PRD S6 D-11). Open the one fixture checkout
# through the Projects sidebar, as the operator would. At a scale the other
# projects have agents and the fixture's has none, so its row waits behind
# folds (PRD sidebar-titlebar-refresh B1): No agents, its project's own fold,
# and its project's empty or inactive worktrees. Only the fixture's project is
# opened, so the measured sidebar is the operator's shape, and each fold is
# clicked once: the fold is the core's, and a second click closes it
# (open-fixture.js, one expression the page evaluates each poll).
wait_js "(() => { window.__measureCheckoutId = $measure_checkout_json; return true; })()"
wait_js "$(cat "$measure_dir/open-fixture.js")"
# The fixture made the switch target last, which a new tab brings to the
# front; the measured tab is shown again by its click, as the operator would.
wait_js "(() => { const canvas = () => document.querySelector('[data-canvas]')?.dataset.canvas; const tab = document.querySelector('[data-agent-tab-bar] [data-tab=\"$measure_tab\"]'); if (tab && canvas() !== '$measure_tab') tab.click(); return canvas() === '$measure_tab'; })()"
wait_js "window.__hideProbe?.paneId() === '$MEASURE_PANE_ID'"
wait_js "(window.__hideProbe?.paneText('$MEASURE_PANE_ID') ?? '').includes('fixture %')"
sleep 2
if [[ "$scenario" == keys ]]; then
  for pane in "${extra_panes[@]}"; do wait_js "(window.__hideProbe?.paneText('$pane') ?? '').includes('fixture %')"; done
  wait_js "document.querySelectorAll('[data-pane-view]').length === 5 && window.__hideProbe.paneId() === '$MEASURE_PANE_ID'"
  wait_js "window.__hideProbe.attachedPanes().length >= 5"
  sleep 2
fi
node -e "
import('$measure_dir/cdp.mjs').then(async ({connectPage}) => { const p = await connectPage('$MEASURE_CDP_PORT'); console.log(JSON.stringify(await p.evaluate('({scenario: \"$scenario\", pane: window.__hideProbe.paneId(), pane_views: document.querySelectorAll(\"[data-pane-view]\").length, splits: document.querySelectorAll(\"[data-split]\").length, tabs: document.querySelectorAll(\"[role=tab]\").length, attached_panes: window.__hideProbe.attachedPanes(), live_terminals: window.__hideProbe.liveTerminals(), ua: navigator.userAgent, dpr: devicePixelRatio, inner: [innerWidth, innerHeight]})'))); p.close(); })" > "$MEASURE_RUN_DIR/page.json"
cat "$MEASURE_RUN_DIR/page.json"
if $fixture_only; then
  if [[ "$scenario" != keys && "$scale" != none ]]; then node "$measure_dir/fixture.mjs" drive; fi
  note 'fixture-only: exact topology and browser pane readiness confirmed; no performance samples collected'
  exit 0
fi
# Resident memory with every attached pane's terminal alive (D-05): sampled
# before the echo trials and again after the driven window.
python3 "$measure_dir/memory.py" settled "$chrome_pid" "$hided_pid" "$MEASURE_CDP_PORT" > "$MEASURE_RUN_DIR/memory-settled.json"
cat "$MEASURE_RUN_DIR/memory-settled.json"
# A topology or restore run reads no engine at idle; keys reads the holders.
resource_engine=0
[[ "$scenario" != keys ]] || resource_engine="engine:$(node "$pane_holders" roots "$hide_state")"
python3 "$measure_dir/resources.py" "$hided_pid" "$resource_engine" "$chrome_pid" > "$MEASURE_RUN_DIR/resources-idle.json"

# What is saved while the topology rounds run (save-watch.py): the daemon's
# screen saves and the own engine's saved session.
save_groups=("hide=$hide_state/core-state.json" "hide=$hide_state/workspace-views.json" "engine=$hide_state/engine")

if [[ "$scenario" == topology ]]; then
  [[ "$scale" == none ]] || node "$measure_dir/fixture.mjs" drive
  reset_fixture
  note "echo at the $scale scale (50 samples)"
  MEASURE_ECHO_REPEATS=50 node "$measure_dir/key-echo.mjs" > "$MEASURE_RUN_DIR/echo-1.json"
  python3 "$measure_dir/summarize.py" echo "$MEASURE_RUN_DIR/echo-1.json" > "$MEASURE_RUN_DIR/echo-summary.json"
  note "topology: ${MEASURE_TOPOLOGY_ROUNDS:-20} rounds of split, zoom, unzoom, close, new tab, tab switch and resize"
  spawn_owned save-watch python3 "$measure_dir/save-watch.py" "${save_groups[@]}"
  save_pid=$owned_pid
  node "$measure_dir/topology.mjs" > "$MEASURE_RUN_DIR/topology.json"
  kill -TERM "$save_pid"
  wait "$save_pid" || true
  cp "$MEASURE_RUN_DIR/logs/save-watch.log" "$MEASURE_RUN_DIR/save-watch.json"
  python3 "$measure_dir/summarize.py" topology "$MEASURE_RUN_DIR/topology.json" > "$MEASURE_RUN_DIR/topology-summary.json"
  python3 "$measure_dir/summarize.py" saves "$MEASURE_RUN_DIR/save-watch.json" "$MEASURE_RUN_DIR/topology.json" > "$MEASURE_RUN_DIR/save-summary.json"
  # Only a build that records stage times writes these lines.
  python3 "$measure_dir/summarize.py" timing "$hide_state/Logs/core.jsonl" > "$MEASURE_RUN_DIR/timing-summary.json"
  python3 "$measure_dir/memory.py" after-topology "$chrome_pid" "$hided_pid" "$MEASURE_CDP_PORT" > "$MEASURE_RUN_DIR/memory-after-topology.json"
  # Topology created and closed panes: read the surviving engine roots now,
  # then record their tree RSS beside the hided/Chrome comparison (row 31).
  resource_engine="engine:$(node "$pane_holders" roots "$hide_state")"
  MEASURE_RESOURCE_SECONDS=1 python3 "$measure_dir/resources.py" "$hided_pid" "$resource_engine" "$chrome_pid" > "$MEASURE_RUN_DIR/resources-after-topology.json"
  cat "$MEASURE_RUN_DIR/echo-summary.json" "$MEASURE_RUN_DIR/topology-summary.json" "$MEASURE_RUN_DIR/save-summary.json"
  note 'measurement complete; cleaning up owned processes'
  exit 0
fi

if [[ "$scenario" == restore ]]; then
  # Every process the run started for the panes, listed before any is killed:
  # the daemon, then each holder, and all they run.
  descendants() { local pid; for pid in $(pgrep -P "$1"); do printf '%s\n' "$pid"; descendants "$pid"; done; }
  victims=("$hided_pid")
  while read -r pid; do victims+=("$pid"); while read -r child; do victims+=("$child"); done < <(descendants "$pid"); done < <(holder_pids)
  printf '%s\n' "${victims[@]}" | sort -u > "$MEASURE_RUN_DIR/restore-victims.txt"
  note "restore: killing $(wc -l < "$MEASURE_RUN_DIR/restore-victims.txt" | tr -d ' ') processes with SIGKILL"
  printf 'about:blank' | node "$measure_dir/navigate.mjs" "$MEASURE_CDP_PORT"
  xargs kill -KILL < "$MEASURE_RUN_DIR/restore-victims.txt" 2>/dev/null || true
  wait "$hided_pid" 2>/dev/null || true
  deadline=$((SECONDS + 10))
  until ! xargs ps -o pid= -p < "$MEASURE_RUN_DIR/restore-victims.txt" >/dev/null 2>&1; do
    (( SECONDS < deadline )) || { echo "restore: killed processes still run: $(xargs ps -o pid=,command= -p < "$MEASURE_RUN_DIR/restore-victims.txt")" >&2; exit 1; }
    sleep 0.05
  done
  [[ -z "$(holders_left)" ]] || { echo "restore: holders still run: $(holders_left)" >&2; exit 1; }
  export MEASURE_RESTORE_T0="$(python3 -c 'import time; print(int(time.time() * 1000))')"
  start_hided -restored
  MEASURE_RESTORE_TABS="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["tabs"])' "$MEASURE_RUN_DIR/scale.json")" \
    MEASURE_RESTORE_PANES="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["panes"])' "$MEASURE_RUN_DIR/scale.json")" \
    node "$measure_dir/restore.mjs" > "$MEASURE_RUN_DIR/restore.json"
  uptime >> "$MEASURE_RUN_DIR/restore.json.uptime"
  cat "$MEASURE_RUN_DIR/restore.json"
  note 'measurement complete; cleaning up owned processes'
  exit 0
fi

if [[ "$scenario" == keys ]]; then
  all_panes=("$MEASURE_PANE_ID" "${extra_panes[@]}")
  keys_printers="$measure_dir/keys_printers.py"
  # The printers run in the foreground, so Ctrl+C typed through the page
  # ends them.
  reset_pane() {
    node "$measure_dir/type.mjs" "$MEASURE_CDP_PORT" "$1" --focus --interrupt "printf '\033c'; stty -echo -icanon; cat"
  }
  drive_all() {
    printer_phase="$MEASURE_RUN_DIR/keys-printers-$2"
    python3 "$keys_printers" start "$printer_phase" "$1" "${extra_panes[@]}" "$MEASURE_PANE_ID" \
      node "$measure_dir/type.mjs" "$MEASURE_CDP_PORT"
  }
  check_printers() { python3 "$keys_printers" check "$printer_phase" "$@"; }
  quiet_all() { for pane in "${extra_panes[@]}" "$MEASURE_PANE_ID"; do reset_pane "$pane"; done; sleep 0.5; }
  idle_seconds="${MEASURE_IDLE_SECONDS:-60}"
  driven_seconds="${MEASURE_DRIVEN_SECONDS:-120}"
  windowless_seconds="${MEASURE_WINDOWLESS_SECONDS:-60}"
  quiet_all
  note "keys: idle resources, $idle_seconds s, five attached panes"
  MEASURE_RESOURCE_SECONDS="$idle_seconds" python3 "$measure_dir/resources.py" "$hided_pid" "$resource_engine" "$chrome_pid" > "$MEASURE_RUN_DIR/resources-idle.json"
  note "keys: screen key echo, idle (50 samples)"
  MEASURE_PANE_ID="$MEASURE_PANE_ID" MEASURE_ECHO_REPEATS=50 node "$measure_dir/key-echo.mjs" > "$MEASURE_RUN_DIR/key-echo-idle.json"
  note "keys: driven, $driven_seconds s at one line per 8 ms per pane"
  drive_all $((driven_seconds + 120)) driven
  check_printers resources-before
  export MEASURE_RESOURCE_SECONDS="$driven_seconds"
  spawn_owned resources-driven python3 "$measure_dir/resources.py" "$hided_pid" "$resource_engine" "$chrome_pid"
  resource_pid=$owned_pid
  unset MEASURE_RESOURCE_SECONDS
  python3 "$measure_dir/diag-counts.py" "$hide_state/Logs/core.jsonl" > "$MEASURE_RUN_DIR/driven-diagnostics-before.json"
  MEASURE_WS_COUNT_SECONDS="$driven_seconds" spawn_owned ws-driven node "$measure_dir/ws-count.mjs"
  ws_pid=$owned_pid
  wait "$resource_pid" "$ws_pid"
  check_printers resources-after
  cp "$MEASURE_RUN_DIR/logs/resources-driven.log" "$MEASURE_RUN_DIR/resources-driven.json"
  cp "$MEASURE_RUN_DIR/logs/ws-driven.log" "$MEASURE_RUN_DIR/ws-driven.json"
  python3 "$measure_dir/diag-counts.py" "$hide_state/Logs/core.jsonl" > "$MEASURE_RUN_DIR/driven-diagnostics-after.json"
  # The echoed marker has to arrive whole, so the measured pane goes back
  # to a plain cat while the other four keep printing.
  reset_pane "$MEASURE_PANE_ID"
  sleep 0.5
  check_printers echo-before --exclude "$MEASURE_PANE_ID"
  note "keys: screen key echo, driven (50 samples, the other four panes printing)"
  MEASURE_PANE_ID="$MEASURE_PANE_ID" MEASURE_ECHO_REPEATS=50 node "$measure_dir/key-echo.mjs" > "$MEASURE_RUN_DIR/key-echo-driven.json"
  check_printers echo-after --exclude "$MEASURE_PANE_ID"
  quiet_all
  if (( ${MEASURE_KEY_COUNT:-0} > 0 )); then
    note "keys: $MEASURE_KEY_COUNT distinct keys counted back from the pane"
    # Count bytes the terminal program actually received, without a
    # scrollback reader or display line wrapping.
    receipt="$MEASURE_RUN_DIR/key-input.txt"
    : > "$receipt"
    node "$measure_dir/type.mjs" "$MEASURE_CDP_PORT" "$MEASURE_PANE_ID" --focus --interrupt "printf '\033c'; stty -echo -icanon; tee $(printf '%q' "$receipt")"
    MEASURE_PANE_READ="cat $(printf '%q' "$receipt")" node "$measure_dir/key-count.mjs" > "$MEASURE_RUN_DIR/key-count.json"
    cat "$MEASURE_RUN_DIR/key-count.json"
    quiet_all
  fi
  note "keys: windowless, $windowless_seconds s with every pane printing"
  drive_all $((windowless_seconds + 10)) windowless
  ws_ready="$MEASURE_RUN_DIR/ws-reopen-ready.json"
  [[ ! -e "$ws_ready" ]] || { echo 'observer readiness file already exists' >&2; exit 1; }
  MEASURE_WS_COUNT_SECONDS=$((windowless_seconds + 30)) spawn_owned ws-reopen node "$measure_dir/ws-count.mjs" --ready-file "$ws_ready"
  ws_pid=$owned_pid
  python3 "$keys_printers" observer "$ws_ready" "$ws_pid"
  check_printers resources-before
  printf 'about:blank' | node "$measure_dir/navigate.mjs" "$MEASURE_CDP_PORT"
  sleep 2
  MEASURE_RESOURCE_SECONDS="$windowless_seconds" python3 "$measure_dir/resources.py" "$hided_pid" "$resource_engine" 0 > "$MEASURE_RUN_DIR/resources-windowless.json"
  check_printers resources-after
  printf '%s' "$page_url" | node "$measure_dir/navigate.mjs" "$MEASURE_CDP_PORT"
  wait_js "window.__hideProbe?.paneId() === '$MEASURE_PANE_ID' && document.querySelectorAll('[data-pane-view]').length === 5"
  sleep 5
  node -e "
import('$measure_dir/cdp.mjs').then(async ({connectPage}) => { const p = await connectPage('$MEASURE_CDP_PORT'); const panes = JSON.parse(process.argv[1]); console.log(JSON.stringify(await p.evaluate('(' + JSON.stringify(panes) + ').map((id) => ({pane: id, text_chars: window.__hideProbe.paneText(id).trim().length}))'))); p.close(); })" "$(printf '%s\n' "${all_panes[@]}" | python3 -c 'import json,sys; print(json.dumps(sys.stdin.read().split()))')" > "$MEASURE_RUN_DIR/reopen-panes.json"
  python3 "$measure_dir/diag-counts.py" "$hide_state/Logs/core.jsonl" > "$MEASURE_RUN_DIR/reopen-diagnostics.json"
  wait "$ws_pid"
  cp "$MEASURE_RUN_DIR/logs/ws-reopen.log" "$MEASURE_RUN_DIR/ws-reopen.json"
  cat "$MEASURE_RUN_DIR/reopen-panes.json" "$MEASURE_RUN_DIR/reopen-diagnostics.json" "$MEASURE_RUN_DIR/ws-driven.json" "$MEASURE_RUN_DIR/ws-reopen.json"
  for name in idle driven; do
    python3 "$measure_dir/summarize.py" echo "$MEASURE_RUN_DIR/key-echo-$name.json" > "$MEASURE_RUN_DIR/key-echo-$name-summary.json"
  done
  for name in idle driven windowless; do
    python3 "$measure_dir/summarize.py" resources "$MEASURE_RUN_DIR/resources-$name.json" > "$MEASURE_RUN_DIR/resources-$name-summary.json"
  done
  cat "$MEASURE_RUN_DIR"/key-echo-*-summary.json "$MEASURE_RUN_DIR"/resources-*-summary.json
  note 'measurement complete; cleaning up owned processes'
  exit 0
fi

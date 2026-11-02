#!/usr/bin/env bash
# Owner of every isolated fixture, daemon, browser and driver for the web
# shell echo (PRD B8) and frame (PRD B12) measurements against the product
# hided. Usage:
#   HIDE_MEASURE_RUN_DIR=agents/runs/<slug>/measure bash scripts/web-shell-measure/run.sh
#   MEASURE_SCENARIO=multi HIDE_MEASURE_RUN_DIR=... bash scripts/web-shell-measure/run.sh
# `single` (default) is the S1 shape: one pane in one tab. `multi` is the S2
# shape (PRD web-shell-pivot-s2 D-08): the measured pane shares its tab with
# four splits, and four more tabs are shown once each so the core holds five
# attached tabs, before the shell returns to the measured tab. The driver and
# the marker still go to the one measured pane; the other panes are idle
# shells with mounted xterm instances.
# `topology` (PRD instant-pane-topology D-15) adds a second tab to the
# measured workspace and, with MEASURE_SCALE=operator or double, fills the
# session to that scale (fixture.mjs, which reports no agents and starts the
# scale's printing panes); it then records idle resources, one echo trial,
# and the split, zoom, close, new tab, tab switch and resize latencies
# (topology.mjs) while it counts the layout saves (save-watch.py), and skips
# the frame window.
# `keys` (PRD core-host-node-terminal D-07, D-08, B2, B4, B5, B19, B21) is
# the measured pane and four splits in one tab, five attached panes: idle
# resources for MEASURE_IDLE_SECONDS (60), the screen key echo idle
# (key-echo.mjs), then every pane printing a line per 8 ms while resources
# are sampled for MEASURE_DRIVEN_SECONDS (120), then the key echo again on
# the measured pane while the other four keep printing,
# then MEASURE_KEY_COUNT (0: skipped) distinct keys counted back from the
# pane (key-count.mjs), then the window closed while the panes print for
# MEASURE_WINDOWLESS_SECONDS (60) and reopened.
# `device` (B3, B4) is the same key echo and key count on a device's pane:
# the shell registers the device with consent (device-front.mjs), brings
# its fixture workspace to the front, and types into the device's one
# pane. The device is an isolated account the caller set up: the SSH
# alias MEASURE_DEVICE_ALIAS for MEASURE_DEVICE_USER at MEASURE_DEVICE_HOST
# on the private sshd's MEASURE_DEVICE_PORT with the key file
# MEASURE_DEVICE_IDENTITY (device-guard.sh writes the alias into the private
# HOME), its recorded host key in MEASURE_DEVICE_KNOWN_HOSTS (copied there),
# the account's private HOME in MEASURE_DEVICE_HOME (its pinned Herdr in
# ~/.local/bin there), its Herdr socket in MEASURE_DEVICE_SOCKET, and
# MEASURE_DEVICE_HELPER_ROOT and MEASURE_DEVICE_CLI_DIR the consent folders
# inside that HOME; device-guard.sh refuses any other target before
# anything dials. With
# MEASURE_DEVICE_DRIVEN_SECONDS set, four more device panes print a line per
# 8 ms while the key echo runs again (key-echo-device-driven.json).
# `remote-core` (PRD core-host-node-remote-core B5-B7, B21) is the `keys`
# shape with this machine's hided in the node role and its core on the
# measured device: core-host.sh uploads MEASURE_CORE_BIN (the measured
# hided by default) to MEASURE_CORE_PROGRAM in the device's private HOME
# and starts it on the device's isolated Herdr with the node id
# MEASURE_CORE_NODE, the placement record names it, and this machine's
# node (node id MEASURE_SCREEN_NODE, default measure-screen) dials it over
# the same guarded alias. The measured pane and its four splits are this
# machine's panes as the core sees them; after them, the key echo and key
# count run on the device's pane of the core's own fixture checkout
# (MEASURE_CORE_FIXTURE), with the core stopped when the run ends.
# `restore` (PRD engine-bundle-3-core-owns B44, perf-gate row 30) builds the
# topology fixture, opens the page on it, kills every process the run started
# with SIGKILL (the daemon, the Herdr server or every pane holder, and every
# pane's shell), confirms none runs, starts them again and times the tabs, the
# front tab's panes and every shell coming back (restore.mjs).
# MEASURE_ENGINE (PRD engine-bundle-3-core-owns D-15) picks what runs the
# panes of `keys`, `topology` and `restore`: `herdr` (the default), the private pinned
# Herdr server, or `own`, the core's own engine, the same hided started with
# HIDE_DEV_PANE_HOLDER=1 and no Herdr, over the `hide-holder` beside it. Both
# make their fixture the same way, with the web shell's events on the
# daemon's WebSocket (fixture.mjs), type through the page (type.mjs, CDP), and
# watch the saved layout while the topology rounds run (save-watch.py), so a
# pair of runs differs only in the switch. The other scenarios run on Herdr
# only, with the fixture the Herdr CLI makes.
# MEASURE_HIDED_BIN measures another hided build, such as a baseline, with
# the same fixture.
# Needs: the pinned herdr (HERDR_BIN_PATH or PATH), Google Chrome,
# target/release/hided built after `pnpm --dir web build` (docs/BUILD.md: a release hided embeds web/dist).
set -euo pipefail
# Optional unattended mode: no native window and no operator socket read.
isolated_headless=false
memory_series=false
case "${1:-}" in
  '') ;;
  --isolated-headless) isolated_headless=true ;;
  *) echo 'usage: run.sh [--isolated-headless]' >&2; exit 2 ;;
esac
case "${2:-}" in
  '') ;;
  --memory-series) memory_series=true; export MEASURE_FRAME_WINDOW_MS=600000 ;;
  *) echo 'second argument must be --memory-series' >&2; exit 2 ;;
esac
measure_dir="$(cd "$(dirname "$0")" && pwd)"
source "$measure_dir/isolated-env.sh"
trap 'rmdir "$MEASURE_SOCKET_DIR/state" 2>/dev/null; rmdir "$MEASURE_SOCKET_DIR"' EXIT
scenario="${MEASURE_SCENARIO:-single}"
case "$scenario" in single|multi|keys|device|remote-core|areas2|areas3|topology|restore) ;; *) echo "MEASURE_SCENARIO must be single, multi, keys, device, remote-core, areas2, areas3, topology or restore" >&2; exit 2;; esac
engine="${MEASURE_ENGINE:-herdr}"
case "$engine" in herdr|own) ;; *) echo "MEASURE_ENGINE must be herdr or own" >&2; exit 2;; esac
# The local fixtures are made through the daemon on either engine.
neutral=false
[[ "$scenario" != topology && "$scenario" != restore && "$scenario" != keys ]] || neutral=true
[[ "$engine" == herdr ]] || $neutral || { echo "MEASURE_ENGINE=own measures only MEASURE_SCENARIO=keys, topology or restore" >&2; exit 2; }
scale="${MEASURE_SCALE:-none}"
case "$scale" in none|operator|double) ;; *) echo "MEASURE_SCALE must be none, operator or double" >&2; exit 2;; esac
[[ "$scale" == none ]] || { $neutral && [[ "$scenario" != keys ]]; } || { echo "MEASURE_SCALE needs MEASURE_SCENARIO=topology or restore" >&2; exit 2; }
[[ "$scenario" != device && "$scenario" != remote-core ]] || source "$measure_dir/device-guard.sh"
screen_node="${MEASURE_SCREEN_NODE:-measure-screen}"
core_started=false
chrome_bin="${MEASURE_CHROME_BIN:-/Applications/Google Chrome.app/Contents/MacOS/Google Chrome}"
hided_bin="${MEASURE_HIDED_BIN:-$MEASURE_WORKTREE/target/release/hided}"
[[ -x "$hided_bin" ]] || { echo "build target/release/hided first: pnpm --dir web build, then the release build described in docs/BUILD.md" >&2; exit 1; }
# The own engine's node starts each pane's holder from the one beside hided.
[[ "$engine" == herdr || -x "$(dirname "$hided_bin")/hide-holder" ]] || { echo "MEASURE_ENGINE=own needs hide-holder beside $hided_bin" >&2; exit 1; }
[[ -x "$chrome_bin" ]] || { echo "Chrome not found at $chrome_bin" >&2; exit 1; }
pids=()
server_started=false
server_pid=0
# The daemon's state folder sits in the run's short socket folder while the
# run lasts, since the own engine's holder sockets live under it and the run
# folder's path is longer than a Unix socket address takes (a link is refused
# as not private); both engines get it, and the end moves it into the run
# folder as isolated/hide-state.
hide_state="$MEASURE_SOCKET_DIR/state"
mkdir -m 700 "$hide_state"
export MEASURE_HIDE_STATE="$hide_state"
note() { printf 'measure: %s\n' "$*"; }
# The own engine's pane holders outlive the daemon by design, so the run owns
# them by the pid each record in its state folder names, with the e2e
# fixture's rule (web/e2e/pane-holders.mjs, through holders.mjs): `holder_pids` lists the holders
# and their programs, `end_holders` ends them (failing when one still runs),
# `holders_left` prints what still runs from the run's holder folder (nothing,
# on a clean end: leaked holders once used up every PTY).
pane_holders="$measure_dir/holders.mjs"
holder_pids() { node "$pane_holders" pids "$hide_state"; }
end_holders() { node "$pane_holders" end "$hide_state"; }
holders_left() { node "$pane_holders" left "$hide_state" || true; }
operator_counts() {
  if $isolated_headless; then
    printf '{"observed":false,"reason":"isolated-headless mode never reads the operator socket"}\n' > "$MEASURE_RUN_DIR/operator-$1.json"
  else
    python3 "$measure_dir/operator-counts.py" "$HERDR_BIN_PATH" "$MEASURE_OPERATOR_SOCKET" "$MEASURE_RUN_DIR/operator-$1.json"
  fi
}
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
  if $server_started; then
    kill_owned "${server_pid:-}"
    [[ -n "${server_pid:-}" ]] && wait "$server_pid" 2>/dev/null
    rm -f "$HERDR_SOCKET_PATH" "$HERDR_SOCKET_PATH.agent" "${HERDR_SOCKET_PATH%.sock}-client.sock" "$HERDR_SOCKET_PATH.hide-label-generator.lock"
  fi
  for pid in "${pids[@]:-}"; do [[ -n "$pid" ]] && wait "$pid" 2>/dev/null; done
  if [[ "$engine" == own ]]; then
    # A holder left running fails the run, so a battery of runs stops there.
    end_holders || cleanup_status=1
    left="$(holders_left)"
    [[ -z "$left" ]] || { printf 'pane holder processes left running:\n%s\n' "$left" >&2; cleanup_status=1; }
  fi
  if $core_started; then
    # A core left running on the device is said where the run is read.
    bash "$measure_dir/core-host.sh" stop > "$MEASURE_RUN_DIR/core-stop.txt" 2>&1 \
      || echo "the device's core was not stopped; see $MEASURE_RUN_DIR/core-stop.txt" >&2
  fi
  rmdir "$MEASURE_PRIVATE/hide-state" && mv "$hide_state" "$MEASURE_PRIVATE/hide-state"
  rmdir "$MEASURE_SOCKET_DIR"
  {
    for pid in "${pids[@]:-}" "${server_pid:-}"; do
      [[ -n "$pid" ]] || continue
      ps -p "$pid" -o pid=,ppid=,comm= || printf '%s exited\n' "$pid"
    done
    [[ "$engine" != own ]] || printf 'holders left: %s\n' "$(holders_left | wc -l | tr -d ' ')"
  } > "$MEASURE_RUN_DIR/cleanup-processes.txt"
  operator_counts after
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
  if $neutral; then
    # Typed through the page on either engine, as the operator types.
    node "$measure_dir/type.mjs" "$MEASURE_CDP_PORT" "$MEASURE_PANE_ID" --interrupt 'printf "\033c"; stty -echo -icanon; cat'
    sleep 0.8
    return
  fi
  "$HERDR_BIN_PATH" pane send-keys "$MEASURE_PANE_ID" ctrl+c >/dev/null
  sleep 0.3
  "$HERDR_BIN_PATH" pane run "$MEASURE_PANE_ID" 'printf "\033c"; stty -echo -icanon; cat' >/dev/null
  sleep 0.5
}

{
  echo "worktree_head=$(git -C "$MEASURE_WORKTREE" rev-parse HEAD)"
  echo "worktree_dirty=$(git -C "$MEASURE_WORKTREE" status --porcelain | wc -l | tr -d ' ')"
  echo "hided_bin=$hided_bin"
  echo "hided_sha256=$(shasum -a 256 "$hided_bin" | awk '{print $1}')"
  echo "herdr_bin=$HERDR_BIN_PATH"
  echo "herdr_version=$("$HERDR_BIN_PATH" --version)"
  echo "chrome_version=$("$chrome_bin" --version 2>/dev/null)"
  echo "socket=$HERDR_SOCKET_PATH"
  echo "scenario=$scenario"
  echo "scale=$scale"
  echo "isolated_headless=$isolated_headless"
  echo "memory_series=$memory_series"
  echo "started_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  uptime
} > "$MEASURE_RUN_DIR/identity.txt"
operator_counts before

# Private server and one cat pane.
[[ -S "$HERDR_SOCKET_PATH" ]] && { echo "socket already exists: $HERDR_SOCKET_PATH" >&2; exit 2; }
if [[ ! -d "$MEASURE_FIXTURE/.git" ]]; then
  git -C "$MEASURE_FIXTURE" init --quiet
  printf 'measure fixture\n' > "$MEASURE_FIXTURE/README"
  git -C "$MEASURE_FIXTURE" add README
  git -C "$MEASURE_FIXTURE" -c commit.gpgsign=false -c user.email="measure@example.invalid" -c user.name="measure" commit --quiet -m "measure fixture"
fi
measure_checkout="$(cd "$MEASURE_RUN_DIR" && pwd)/checkout"
if [[ ! -f "$measure_checkout/.git" ]]; then
  git -C "$MEASURE_FIXTURE" worktree add --quiet -b measure-worktree "$measure_checkout"
fi
export MEASURE_FIXTURE="$measure_checkout"
start_server() {
  env HOME="$MEASURE_PRIVATE/home" "$HERDR_BIN_PATH" server >>"$MEASURE_RUN_DIR/logs/herdr-server.log" 2>&1 &
  server_pid=$!
  server_started=true
  local deadline=$((SECONDS+20)); until [[ -S "$HERDR_SOCKET_PATH" ]]; do (( SECONDS < deadline )) || { echo 'socket did not appear' >&2; exit 1; }; sleep 0.1; done
}
# The own engine runs no server: the daemon's node starts each pane's holder.
if [[ "$engine" == herdr ]]; then
  start_server
  workspaces="$("$HERDR_BIN_PATH" api snapshot | python3 -c 'import json,sys; d=json.load(sys.stdin); r=d.get("result") or d; s=r.get("snapshot") or r; print(len(s.get("workspaces") or []))')"
  [[ "$workspaces" == "0" ]] || { echo "private server already has $workspaces workspaces" >&2; exit 1; }
fi
if ! $neutral; then
  created="$("$HERDR_BIN_PATH" workspace create --cwd "$MEASURE_FIXTURE" --label measure --focus)"
  export MEASURE_PANE_ID="$("$HERDR_BIN_PATH" api snapshot | python3 "$measure_dir/pane-id.py")"
  measure_workspace="$(printf %s "$created" | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["workspace"]["workspace_id"])')"
  measure_tab="$(printf %s "$created" | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["tab"]["tab_id"])')"
  # The current checkout-owner contract requires an explicit owner before
  # Hide can reuse this fixture's existing workspace and cat pane.
  # With the core elsewhere this machine's checkout is a device's, and its
  # owner's mark names this machine's node.
  owner_device=local
  [[ "$scenario" != remote-core ]] || owner_device="$screen_node"
  measure_owner="$(OWNER_DEVICE="$owner_device" python3 -c 'import hashlib,os; print(hashlib.sha256((os.environ["OWNER_DEVICE"]+"\0"+os.path.realpath(os.environ["MEASURE_FIXTURE"])).encode()).hexdigest()[:32])')"
  "$HERDR_BIN_PATH" workspace report-metadata "$measure_workspace" --source performance-fixture --token "hide_owner=$measure_owner" --token purpose="Isolated performance fixture" >/dev/null
  wait_prompt() {
    # The pane shell has printed its fixed prompt before it takes a command.
    local deadline=$((SECONDS+20))
    until "$HERDR_BIN_PATH" pane read "$1" --source visible --format text 2>/dev/null | grep -q 'fixture %'; do
      (( SECONDS < deadline )) || { echo "pane $1 prompt did not appear" >&2; exit 1; }
      sleep 0.1
    done
  }
  wait_prompt "$MEASURE_PANE_ID"
  "$HERDR_BIN_PATH" pane run "$MEASURE_PANE_ID" 'stty -echo -icanon; cat' >/dev/null
  extra_panes=()
  extra_tabs=()
  if [[ "$scenario" == multi || "$scenario" == keys || "$scenario" == remote-core ]]; then
    # Four splits beside the measured pane, alternating direction so the tree
    # nests, and (multi) four more tabs; none of it takes focus from the
    # measured pane.
    split_from="$MEASURE_PANE_ID"
    for direction in right down right down; do
      split_pane="$("$HERDR_BIN_PATH" pane split "$split_from" --direction "$direction" --no-focus | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["pane"]["pane_id"])')"
      extra_panes+=("$split_pane")
      split_from="$split_pane"
    done
  fi
  if [[ "$scenario" == multi ]]; then
    for label in two three four five; do
      extra_tab="$("$HERDR_BIN_PATH" tab create --workspace "$measure_workspace" --cwd "$MEASURE_FIXTURE" --label "$label" --no-focus | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["tab"]["tab_id"])')"
      extra_tabs+=("$extra_tab")
    done
    for pane in "${extra_panes[@]}"; do wait_prompt "$pane"; done
  fi

  if [[ "$scenario" == areas* ]]; then
    for ((area=1; area<${scenario#areas}; area++)); do
      created="$("$HERDR_BIN_PATH" tab create --workspace "$measure_workspace" --cwd "$MEASURE_FIXTURE" --label "area-$area" --no-focus)"
      extra_tabs+=("$(printf %s "$created" | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["tab"]["tab_id"])')")
      extra_panes+=("$(printf %s "$created" | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["root_pane"]["pane_id"])')")
    done
    for pane in "${extra_panes[@]}"; do wait_prompt "$pane"; done
  fi

fi
hided_env=()
if [[ "$scenario" == device ]]; then
  [[ -n "${MEASURE_DEVICE_ID:-}" ]] || { echo "the device scenario needs MEASURE_DEVICE_ID" >&2; exit 2; }
  mkdir -p "$MEASURE_PRIVATE/home/.ssh"
  device_guard_config > "$MEASURE_PRIVATE/home/.ssh/config"
  chmod 600 "$MEASURE_PRIVATE/home/.ssh/config"
  # Hide checks the device's host key against ~/.ssh/known_hosts only.
  cp "$MEASURE_DEVICE_KNOWN_HOSTS" "$MEASURE_PRIVATE/home/.ssh/known_hosts"
  hided_env=(HIDE_HOST_HELPER_ROOT="$MEASURE_DEVICE_HELPER_ROOT" HIDE_HOST_CLI_DIR="$MEASURE_DEVICE_CLI_DIR")
  device_herdr() { bash "$measure_dir/device-herdr.sh" "$@"; }
  device_pane="$(device_herdr api snapshot | python3 "$measure_dir/pane-id.py")"
  device_herdr pane send-keys "$device_pane" ctrl+c >/dev/null
  sleep 0.3
  device_herdr pane run "$device_pane" "printf '\033c'; stty -echo -icanon; cat" >/dev/null
  echo "device_pane=$device_pane" >> "$MEASURE_RUN_DIR/identity.txt"
fi

if [[ "$scenario" == remote-core ]]; then
  for name in MEASURE_CORE_FIXTURE; do
    [[ -n "${!name:-}" ]] || { echo "the remote-core scenario needs $name" >&2; exit 2; }
  done
  mkdir -p "$MEASURE_PRIVATE/home/.ssh"
  device_guard_config > "$MEASURE_PRIVATE/home/.ssh/config"
  chmod 600 "$MEASURE_PRIVATE/home/.ssh/config"
  cp "$MEASURE_DEVICE_KNOWN_HOSTS" "$MEASURE_PRIVATE/home/.ssh/known_hosts"
  device_herdr() { bash "$measure_dir/device-herdr.sh" "$@"; }
  note "remote-core: starting the core on $MEASURE_DEVICE_ALIAS as $MEASURE_CORE_NODE"
  core_started=true
  bash "$measure_dir/core-host.sh" start "${MEASURE_CORE_BIN:-$hided_bin}" > "$MEASURE_RUN_DIR/core-start.txt"
  echo "core_bin=${MEASURE_CORE_BIN:-$hided_bin}" >> "$MEASURE_RUN_DIR/identity.txt"
  echo "core_sha256=$(shasum -a 256 "${MEASURE_CORE_BIN:-$hided_bin}" | awk '{print $1}')" >> "$MEASURE_RUN_DIR/identity.txt"
  # The placement record a move writes (layer 5), naming the core.
  chmod 700 "$hide_state"
  MEASURE_SCREEN_STATE="$hide_state" python3 -c '
import json, os
record = {"alias": os.environ["MEASURE_DEVICE_ALIAS"], "node": os.environ["MEASURE_CORE_NODE"],
          "program": os.environ["MEASURE_CORE_PROGRAM"], "state_dir": os.environ["MEASURE_CORE_STATE"]}
path = os.path.join(os.environ["MEASURE_SCREEN_STATE"], "core-placement.json")
with open(os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600), "w") as file:
    json.dump(record, file)
'
  hided_env=(HIDE_MACHINE_ID="$screen_node")
fi

if $neutral; then
  # Every pane's shell, on either engine, touches one file as it reads its rc
  # file (the restore's `shells_ms`), and a shell started in a printing
  # pane's folder waits on its FIFO until `fixture.mjs drive`, then prints in
  # the foreground: a background job would outlive its pane (fixture.mjs).
  export MEASURE_READY_DIR="$MEASURE_RUN_DIR/shell-ready"
  mkdir -p "$MEASURE_READY_DIR"
  cat >> "$MEASURE_PRIVATE/home/.zshrc" <<ZSHRC
: >| "$MEASURE_READY_DIR/\$\$"
[[ -p .measure-go ]] && { read -r _ < .measure-go; /usr/bin/python3 -u -c 'import time,collections; collections.deque(((print("fixture output %08d" % i, flush=True),time.sleep(0.05)) for i in range(20000)),maxlen=0)'; }
ZSHRC
fi
# The own engine takes no Herdr: a socket beside its switch fails the daemon.
[[ "$engine" == herdr ]] || hided_env+=(-u HERDR_SOCKET_PATH -u HERDR_BIN_PATH HIDE_DEV_PANE_HOLDER=1)
# Product hided (release, embedded web/dist) on the private socket.
start_hided() {
  rm -f "$hide_state/hided.json"
  spawn_owned "hided${1:-}" env ${hided_env[@]+"${hided_env[@]}"} HOME="$MEASURE_PRIVATE/home" HIDE_STATE_DIR="$hide_state" HIDE_KEEP_ALIVE=1 HIDE_PORT=0 "$hided_bin"
  hided_pid=$owned_pid
}
start_hided
deadline=$((SECONDS+20)); until [[ -f "$hide_state/hided.json" ]]; do (( SECONDS < deadline )) || { echo 'hided state file missing' >&2; exit 1; }; sleep 0.1; done
hided_port="$(python3 -c "import json;print(json.load(open('$hide_state/hided.json'))['port'])")"
hided_token="$(python3 -c "import json;print(json.load(open('$hide_state/hided.json'))['token'])")"
wait_url "http://127.0.0.1:$hided_port/health"
page_url="http://127.0.0.1:$hided_port/?probe=1#token=$hided_token"
# A node-role hided's page names the machine it runs on, as `hide connect`
# hands it to the window.
[[ "$scenario" != remote-core ]] || page_url+="&node=$screen_node"
if $neutral; then
  note "$engine: making the $scale fixture with the web shell's events"
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
  echo "engine=$engine" >> "$MEASURE_RUN_DIR/identity.txt"
fi

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
if [[ "$scenario" == remote-core ]]; then
  export MEASURE_HIDED_PORT="$hided_port" MEASURE_HIDED_TOKEN="$hided_token" MEASURE_SCREEN_NODE="$screen_node"
  note "remote-core: bringing this machine's checkout to the front on the core"
  node "$measure_dir/core-front.mjs" screen > "$MEASURE_RUN_DIR/front-screen.txt"
  screen_pane="remote:$screen_node:pane:$MEASURE_PANE_ID"
  echo "screen_pane=$screen_pane" >> "$MEASURE_RUN_DIR/identity.txt"
fi
wait_js "$(cat "$measure_dir/open-fixture.js")"
# The fixture made the switch target last, which a new tab brings to the
# front; the measured tab is shown again by its click, as the operator would.
! $neutral || wait_js "(() => { const canvas = () => document.querySelector('[data-canvas]')?.dataset.canvas; const tab = document.querySelector('[data-agent-tab-bar] [data-tab=\"$measure_tab\"]'); if (tab && canvas() !== '$measure_tab') tab.click(); return canvas() === '$measure_tab'; })()"
echo_pane="${screen_pane:-$MEASURE_PANE_ID}"
wait_js "window.__hideProbe?.paneId() === '$echo_pane'"
! $neutral || wait_js "(window.__hideProbe?.paneText('$echo_pane') ?? '').includes('fixture %')"
sleep 2
if [[ "$scenario" == areas* ]]; then
  for tab in "${extra_tabs[@]}"; do
    node "$measure_dir/split-agent.mjs" "$MEASURE_CDP_PORT" "$tab"
  done
  wait_js "(() => { document.querySelector('[data-agent-tab-bar] [data-tab=\"$measure_tab\"]')?.click(); return document.querySelectorAll('[data-agent-area-id]').length === ${scenario#areas} && window.__hideProbe.paneId() === '$MEASURE_PANE_ID'; })()"
fi
if [[ "$scenario" == multi ]]; then
  # Show each extra tab once so the core attaches it, then return.
  for tab in "${extra_tabs[@]}" "$measure_tab"; do
    wait_js "(() => { const el = document.querySelector('[data-tab=\"$tab\"]'); if (!el) return false; if (document.querySelector('[data-canvas]')?.dataset.canvas !== '$tab') el.click(); return true; })()"
    wait_js "document.querySelector('[data-canvas]')?.dataset.canvas === '$tab' && document.querySelectorAll('[data-pane-view]').length > 0"
    sleep 1
  done
  wait_js "document.querySelectorAll('[data-pane-view]').length === 5 && window.__hideProbe.paneId() === '$MEASURE_PANE_ID'"
  wait_js "window.__hideProbe.attachedPanes().length >= 9"
  sleep 2
fi
if [[ "$scenario" == keys || "$scenario" == remote-core ]]; then
  if $neutral; then
    for pane in "${extra_panes[@]}"; do wait_js "(window.__hideProbe?.paneText('$pane') ?? '').includes('fixture %')"; done
  else
    for pane in "${extra_panes[@]}"; do wait_prompt "$pane"; done
  fi
  wait_js "document.querySelectorAll('[data-pane-view]').length === 5 && window.__hideProbe.paneId() === '$echo_pane'"
  wait_js "window.__hideProbe.attachedPanes().length >= 5"
  sleep 2
fi
node -e "
import('$measure_dir/cdp.mjs').then(async ({connectPage}) => { const p = await connectPage('$MEASURE_CDP_PORT'); console.log(JSON.stringify(await p.evaluate('({scenario: \"$scenario\", pane: window.__hideProbe.paneId(), pane_views: document.querySelectorAll(\"[data-pane-view]\").length, splits: document.querySelectorAll(\"[data-split]\").length, tabs: document.querySelectorAll(\"[role=tab]\").length, attached_panes: window.__hideProbe.attachedPanes(), live_terminals: window.__hideProbe.liveTerminals(), ua: navigator.userAgent, dpr: devicePixelRatio, inner: [innerWidth, innerHeight]})'))); p.close(); })" > "$MEASURE_RUN_DIR/page.json"
cat "$MEASURE_RUN_DIR/page.json"
# Resident memory with every attached pane's terminal alive (D-05): sampled
# before the echo trials and again after the driven window.
python3 "$measure_dir/memory.py" settled "$chrome_pid" "$hided_pid" "$MEASURE_CDP_PORT" > "$MEASURE_RUN_DIR/memory-settled.json"
cat "$MEASURE_RUN_DIR/memory-settled.json"
resource_engine="$server_pid"
if [[ "$scenario" == keys ]]; then
  if [[ "$engine" == own ]]; then resource_engine="engine:$(node "$pane_holders" roots "$hide_state")";
  else resource_engine="engine:$server_pid"; fi
fi
python3 "$measure_dir/resources.py" "$hided_pid" "$resource_engine" "$chrome_pid" > "$MEASURE_RUN_DIR/resources-idle.json"

# What each engine saves its layout in, for the save watch (save-watch.py):
# the daemon's screen saves on both, and the Herdr server's session or the
# own engine's saved session.
save_groups=("hide=$hide_state/core-state.json" "hide=$hide_state/workspace-views.json")
if [[ "$engine" == herdr ]]; then save_groups+=("herdr=$XDG_CONFIG_HOME/herdr"); else save_groups+=("engine=$hide_state/engine"); fi

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
  if [[ "$engine" == own ]]; then resource_engine="engine:$(node "$pane_holders" roots "$hide_state")";
  else resource_engine="engine:$server_pid"; fi
  MEASURE_RESOURCE_SECONDS=1 python3 "$measure_dir/resources.py" "$hided_pid" "$resource_engine" "$chrome_pid" > "$MEASURE_RUN_DIR/resources-after-topology.json"
  cat "$MEASURE_RUN_DIR/echo-summary.json" "$MEASURE_RUN_DIR/topology-summary.json" "$MEASURE_RUN_DIR/save-summary.json"
  note 'measurement complete; cleaning up owned processes'
  exit 0
fi

if [[ "$scenario" == restore ]]; then
  # Every process the run started for the panes, listed before any is killed:
  # the daemon, then the Herdr server or each holder, and all they run.
  descendants() { local pid; for pid in $(pgrep -P "$1"); do printf '%s\n' "$pid"; descendants "$pid"; done; }
  victims=("$hided_pid")
  if [[ "$engine" == herdr ]]; then
    victims+=("$server_pid")
    while read -r pid; do victims+=("$pid"); done < <(descendants "$server_pid")
  else
    while read -r pid; do victims+=("$pid"); while read -r child; do victims+=("$child"); done < <(descendants "$pid"); done < <(holder_pids)
  fi
  printf '%s\n' "${victims[@]}" | sort -u > "$MEASURE_RUN_DIR/restore-victims.txt"
  note "restore: killing $(wc -l < "$MEASURE_RUN_DIR/restore-victims.txt" | tr -d ' ') processes with SIGKILL"
  printf 'about:blank' | node "$measure_dir/navigate.mjs" "$MEASURE_CDP_PORT"
  xargs kill -KILL < "$MEASURE_RUN_DIR/restore-victims.txt" 2>/dev/null || true
  wait "$hided_pid" 2>/dev/null || true
  [[ "$engine" != herdr ]] || wait "$server_pid" 2>/dev/null || true
  deadline=$((SECONDS + 10))
  until ! xargs ps -o pid= -p < "$MEASURE_RUN_DIR/restore-victims.txt" >/dev/null 2>&1; do
    (( SECONDS < deadline )) || { echo "restore: killed processes still run: $(xargs ps -o pid=,command= -p < "$MEASURE_RUN_DIR/restore-victims.txt")" >&2; exit 1; }
    sleep 0.05
  done
  [[ "$engine" == herdr ]] || [[ -z "$(holders_left)" ]] || { echo "restore: holders still run: $(holders_left)" >&2; exit 1; }
  rm -f "$HERDR_SOCKET_PATH" "${HERDR_SOCKET_PATH%.sock}-client.sock"
  export MEASURE_RESTORE_T0="$(python3 -c 'import time; print(int(time.time() * 1000))')"
  [[ "$engine" != herdr ]] || start_server
  start_hided -restored
  MEASURE_RESTORE_TABS="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["tabs"])' "$MEASURE_RUN_DIR/scale.json")" \
    MEASURE_RESTORE_PANES="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["panes"])' "$MEASURE_RUN_DIR/scale.json")" \
    node "$measure_dir/restore.mjs" > "$MEASURE_RUN_DIR/restore.json"
  uptime >> "$MEASURE_RUN_DIR/restore.json.uptime"
  cat "$MEASURE_RUN_DIR/restore.json"
  note 'measurement complete; cleaning up owned processes'
  exit 0
fi

if [[ "$scenario" == device ]]; then
  export MEASURE_HIDED_PORT="$hided_port" MEASURE_HIDED_TOKEN="$hided_token"
  note "device: registering $MEASURE_DEVICE_ID and bringing it to the front"
  node "$measure_dir/device-front.mjs" register
  node "$measure_dir/device-front.mjs" front
  scoped_pane="remote:$MEASURE_DEVICE_ID:pane:$device_pane"
  wait_js "window.__hideProbe?.paneId() === '$scoped_pane' && window.__hideProbe.attachedPanes().includes('$scoped_pane')"
  sleep 2
  note "device: screen key echo on $scoped_pane (50 samples)"
  MEASURE_PANE_ID="$scoped_pane" MEASURE_ECHO_REPEATS=50 node "$measure_dir/key-echo.mjs" > "$MEASURE_RUN_DIR/key-echo-device.json"
  python3 "$measure_dir/summarize.py" echo "$MEASURE_RUN_DIR/key-echo-device.json" > "$MEASURE_RUN_DIR/key-echo-device-summary.json"
  cat "$MEASURE_RUN_DIR/key-echo-device-summary.json"
  if (( ${MEASURE_KEY_COUNT:-0} > 0 )); then
    device_herdr pane send-keys "$device_pane" ctrl+c >/dev/null
    sleep 0.3
    device_herdr pane run "$device_pane" "printf '\033c'; stty -echo -icanon; cat" >/dev/null
    sleep 0.5
    note "device: $MEASURE_KEY_COUNT distinct keys counted back from the device's pane"
    MEASURE_PANE_ID="$device_pane" MEASURE_SCREEN_PANE_ID="$scoped_pane" \
      MEASURE_PANE_READ="bash $(printf %q "$measure_dir/device-herdr.sh") pane read '$device_pane' --source recent-unwrapped --lines 4000" \
      node "$measure_dir/key-count.mjs" > "$MEASURE_RUN_DIR/key-count-device.json"
    cat "$MEASURE_RUN_DIR/key-count-device.json"
  fi
  driven_seconds="${MEASURE_DEVICE_DRIVEN_SECONDS:-0}"
  if (( driven_seconds > 0 )); then
    # Four more panes on the device, each printing a line per 8 ms, beside
    # the measured pane, which goes back to a plain cat so its marker
    # arrives whole; the splits are closed again afterwards.
    device_herdr pane send-keys "$device_pane" ctrl+c >/dev/null
    sleep 0.3
    device_herdr pane run "$device_pane" "printf '\033c'; stty -echo -icanon; cat" >/dev/null
    device_splits=()
    split_from="$device_pane"
    for direction in right down right down; do
      split_from="$(device_herdr pane split "$split_from" --direction "$direction" --no-focus | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["pane"]["pane_id"])')"
      device_splits+=("$split_from")
    done
    for pane in "${device_splits[@]}"; do
      deadline=$((SECONDS+20))
      until device_herdr pane read "$pane" --source visible --format text 2>/dev/null | grep -q 'fixture %'; do
        (( SECONDS < deadline )) || { echo "device pane $pane prompt did not appear" >&2; exit 1; }
        sleep 0.3
      done
      # One line per 8 ms on the device's clock, with no process per line.
      device_herdr pane run "$pane" "perl -e 'use Time::HiRes qw(sleep time); \$|=1; my \$s=time; my \$i=0; while (time < \$s+$((driven_seconds + 30))) { print qq(drive \$i\\n); \$i++; my \$w=\$s+\$i*0.008-time; sleep \$w if \$w>0 }'" >/dev/null
    done
    wait_js "document.querySelectorAll('[data-pane-view]').length === 5 && window.__hideProbe.paneId() === '$scoped_pane'"
    sleep 3
    note "device: screen key echo, driven (50 samples, four device panes printing)"
    MEASURE_PANE_ID="$scoped_pane" MEASURE_ECHO_REPEATS=50 node "$measure_dir/key-echo.mjs" > "$MEASURE_RUN_DIR/key-echo-device-driven.json"
    python3 "$measure_dir/summarize.py" echo "$MEASURE_RUN_DIR/key-echo-device-driven.json" > "$MEASURE_RUN_DIR/key-echo-device-driven-summary.json"
    cat "$MEASURE_RUN_DIR/key-echo-device-driven-summary.json"
    for pane in "${device_splits[@]}"; do device_herdr pane close "$pane" >/dev/null || true; done
  fi
  device_herdr pane send-keys "$device_pane" ctrl+c >/dev/null
  note 'measurement complete; cleaning up owned processes'
  exit 0
fi

if [[ "$scenario" == keys || "$scenario" == remote-core ]]; then
  all_panes=("$MEASURE_PANE_ID" "${extra_panes[@]}")
  # One line per 8 ms for the given seconds, beside the cat that echoes keys.
  printf '%s\n' 'import sys, time' 'end = time.monotonic() + float(sys.argv[1]); i = 0' \
    'while time.monotonic() < end:' "    print(f'drive {i:05d}', flush=True); i += 1; time.sleep(0.008)" > "$MEASURE_RUN_DIR/drive.py"
  # Local printers run in the foreground, so Ctrl+C ends them on either
  # engine. The remote-core fixture retains its pane-tagged background job.
  reset_pane() {
    if $neutral; then
      local command="cat"
      [[ -z "${2:-}" ]] || command="/usr/bin/python3 $(printf '%q' "$MEASURE_RUN_DIR/drive.py") $2"
      node "$measure_dir/type.mjs" "$MEASURE_CDP_PORT" "$1" --focus --interrupt "printf '\033c'; stty -echo -icanon; $command"
    else
      pkill -f "$MEASURE_RUN_DIR/drive\.py [0-9]+ $1\$" || true
      "$HERDR_BIN_PATH" pane send-keys "$1" ctrl+c >/dev/null
      sleep 0.3
      "$HERDR_BIN_PATH" pane run "$1" "printf '\033c'; stty -echo -icanon; ${2:+/usr/bin/python3 $MEASURE_RUN_DIR/drive.py $2 $1 & }cat" >/dev/null
    fi
  }
  drive_all() { for pane in "${extra_panes[@]}" "$MEASURE_PANE_ID"; do reset_pane "$pane" "$1"; done; sleep 1; }
  quiet_all() { for pane in "${extra_panes[@]}" "$MEASURE_PANE_ID"; do reset_pane "$pane"; done; sleep 0.5; }
  idle_seconds="${MEASURE_IDLE_SECONDS:-60}"
  driven_seconds="${MEASURE_DRIVEN_SECONDS:-120}"
  windowless_seconds="${MEASURE_WINDOWLESS_SECONDS:-60}"
  quiet_all
  note "keys: idle resources, $idle_seconds s, five attached panes"
  MEASURE_RESOURCE_SECONDS="$idle_seconds" python3 "$measure_dir/resources.py" "$hided_pid" "$resource_engine" "$chrome_pid" > "$MEASURE_RUN_DIR/resources-idle.json"
  note "keys: screen key echo, idle (50 samples)"
  MEASURE_PANE_ID="$echo_pane" MEASURE_ECHO_REPEATS=50 node "$measure_dir/key-echo.mjs" > "$MEASURE_RUN_DIR/key-echo-idle.json"
  note "keys: driven, $driven_seconds s at one line per 8 ms per pane"
  drive_all $((driven_seconds + 120))
  export MEASURE_RESOURCE_SECONDS="$driven_seconds"
  spawn_owned resources-driven python3 "$measure_dir/resources.py" "$hided_pid" "$resource_engine" "$chrome_pid"
  resource_pid=$owned_pid
  unset MEASURE_RESOURCE_SECONDS
  python3 "$measure_dir/diag-counts.py" "$hide_state/Logs/core.jsonl" > "$MEASURE_RUN_DIR/driven-diagnostics-before.json"
  MEASURE_WS_COUNT_SECONDS="$driven_seconds" spawn_owned ws-driven node "$measure_dir/ws-count.mjs"
  ws_pid=$owned_pid
  wait "$resource_pid" "$ws_pid"
  cp "$MEASURE_RUN_DIR/logs/resources-driven.log" "$MEASURE_RUN_DIR/resources-driven.json"
  cp "$MEASURE_RUN_DIR/logs/ws-driven.log" "$MEASURE_RUN_DIR/ws-driven.json"
  python3 "$measure_dir/diag-counts.py" "$hide_state/Logs/core.jsonl" > "$MEASURE_RUN_DIR/driven-diagnostics-after.json"
  # The echoed marker has to arrive whole, so the measured pane goes back
  # to a plain cat while the other four keep printing.
  reset_pane "$MEASURE_PANE_ID"
  sleep 0.5
  note "keys: screen key echo, driven (50 samples, the other four panes printing)"
  MEASURE_PANE_ID="$echo_pane" MEASURE_ECHO_REPEATS=50 node "$measure_dir/key-echo.mjs" > "$MEASURE_RUN_DIR/key-echo-driven.json"
  quiet_all
  if (( ${MEASURE_KEY_COUNT:-0} > 0 )); then
    note "keys: $MEASURE_KEY_COUNT distinct keys counted back from the pane"
    if $neutral; then
      # Count bytes the terminal program actually received, without an
      # engine-specific scrollback reader or display line wrapping.
      receipt="$MEASURE_RUN_DIR/key-input.txt"
      : > "$receipt"
      node "$measure_dir/type.mjs" "$MEASURE_CDP_PORT" "$MEASURE_PANE_ID" --focus --interrupt "printf '\033c'; stty -echo -icanon; tee $(printf '%q' "$receipt")"
      MEASURE_PANE_READ="cat $(printf '%q' "$receipt")" MEASURE_SCREEN_PANE_ID="$echo_pane" node "$measure_dir/key-count.mjs" > "$MEASURE_RUN_DIR/key-count.json"
    else
      MEASURE_SCREEN_PANE_ID="$echo_pane" node "$measure_dir/key-count.mjs" > "$MEASURE_RUN_DIR/key-count.json"
    fi
    cat "$MEASURE_RUN_DIR/key-count.json"
    quiet_all
  fi
  note "keys: windowless, $windowless_seconds s with every pane printing"
  MEASURE_WS_COUNT_SECONDS=$((windowless_seconds + 30)) spawn_owned ws-reopen node "$measure_dir/ws-count.mjs"
  ws_pid=$owned_pid
  sleep 1
  drive_all $((windowless_seconds + 10))
  printf 'about:blank' | node "$measure_dir/navigate.mjs" "$MEASURE_CDP_PORT"
  sleep 2
  MEASURE_RESOURCE_SECONDS="$windowless_seconds" python3 "$measure_dir/resources.py" "$hided_pid" "$resource_engine" 0 > "$MEASURE_RUN_DIR/resources-windowless.json"
  printf '%s' "$page_url" | node "$measure_dir/navigate.mjs" "$MEASURE_CDP_PORT"
  wait_js "window.__hideProbe?.paneId() === '$echo_pane' && document.querySelectorAll('[data-pane-view]').length === 5"
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
  if [[ "$scenario" == remote-core ]]; then
    # The core machine's pane, seen from this machine's window.
    quiet_all
    core_pane="$(node "$measure_dir/core-front.mjs" core)"
    echo "core_pane=$core_pane" >> "$MEASURE_RUN_DIR/identity.txt"
    device_herdr pane send-keys "$core_pane" ctrl+c >/dev/null
    sleep 0.3
    device_herdr pane run "$core_pane" "printf '\033c'; stty -echo -icanon; cat" >/dev/null
    wait_js "window.__hideProbe?.paneId() === '$core_pane' && window.__hideProbe.attachedPanes().includes('$core_pane')"
    sleep 2
    note "remote-core: screen key echo on the core machine's pane $core_pane (50 samples)"
    MEASURE_PANE_ID="$core_pane" MEASURE_ECHO_REPEATS=50 node "$measure_dir/key-echo.mjs" > "$MEASURE_RUN_DIR/key-echo-core.json"
    python3 "$measure_dir/summarize.py" echo "$MEASURE_RUN_DIR/key-echo-core.json" > "$MEASURE_RUN_DIR/key-echo-core-summary.json"
    cat "$MEASURE_RUN_DIR/key-echo-core-summary.json"
    if (( ${MEASURE_KEY_COUNT:-0} > 0 )); then
      device_herdr pane send-keys "$core_pane" ctrl+c >/dev/null
      sleep 0.3
      device_herdr pane run "$core_pane" "printf '\033c'; stty -echo -icanon; cat" >/dev/null
      sleep 0.5
      note "remote-core: $MEASURE_KEY_COUNT distinct keys counted back from the core machine's pane"
      MEASURE_PANE_ID="$core_pane" MEASURE_SCREEN_PANE_ID="$core_pane" \
        MEASURE_PANE_READ="bash $(printf %q "$measure_dir/device-herdr.sh") pane read '$core_pane' --source recent-unwrapped --lines 4000" \
        node "$measure_dir/key-count.mjs" > "$MEASURE_RUN_DIR/key-count-core.json"
      cat "$MEASURE_RUN_DIR/key-count-core.json"
    fi
    device_herdr pane send-keys "$core_pane" ctrl+c >/dev/null
  fi
  note 'measurement complete; cleaning up owned processes'
  exit 0
fi

for trial in 1 2 3; do
  reset_fixture
  note "echo trial $trial (50 samples)"
  ps -p "$hided_pid" -o pid,%cpu,rss,etime,command > "$MEASURE_RUN_DIR/idle-hided-$trial.ps"
  MEASURE_ECHO_REPEATS=50 node "$measure_dir/key-echo.mjs" > "$MEASURE_RUN_DIR/echo-$trial.json"
  ps -p "$hided_pid" -o pid,%cpu,rss,etime,command > "$MEASURE_RUN_DIR/driven-hided-$trial.ps"
done
python3 "$measure_dir/summarize.py" echo "$MEASURE_RUN_DIR"/echo-{1,2,3}.json > "$MEASURE_RUN_DIR/echo-summary.json"

export MEASURE_DRIVEN_PANES="$MEASURE_PANE_ID"
if [[ "$scenario" == areas* ]]; then
  for pane in "${extra_panes[@]}"; do MEASURE_DRIVEN_PANES+=",$pane"; done
fi
note "frames: ${MEASURE_FRAME_WINDOW_MS:-120000} ms driven window, every shown Agent area"
reset_fixture
"$HERDR_BIN_PATH" pane send-keys "$MEASURE_PANE_ID" ctrl+c >/dev/null
sleep 0.5
uptime > "$MEASURE_RUN_DIR/frames-uptime-before.txt"
spawn_owned resources-driven python3 "$measure_dir/resources.py" "$hided_pid" "$resource_engine" "$chrome_pid"
resource_pid=$owned_pid
if $memory_series; then
  spawn_owned memory-series python3 "$measure_dir/resources.py" "$hided_pid" "$resource_engine" "$chrome_pid" --memory-series
  memory_pid=$owned_pid
fi
node "$measure_dir/frames.mjs" > "$MEASURE_RUN_DIR/frames.json"
wait "$resource_pid"
cp "$MEASURE_RUN_DIR/logs/resources-driven.log" "$MEASURE_RUN_DIR/resources-driven.json"
if $memory_series; then
  wait "$memory_pid"
  cp "$MEASURE_RUN_DIR/logs/memory-series.log" "$MEASURE_RUN_DIR/memory-series.json"
fi
uptime > "$MEASURE_RUN_DIR/frames-uptime-after.txt"
"$HERDR_BIN_PATH" pane read "$MEASURE_PANE_ID" --source recent-unwrapped --lines 5 > "$MEASURE_RUN_DIR/frames-pane-tail.txt" || true
ps -p "$hided_pid" -o pid,%cpu,rss,etime,command > "$MEASURE_RUN_DIR/driven-hided-frames.ps"
python3 "$measure_dir/summarize.py" frames "$MEASURE_RUN_DIR/frames.json" > "$MEASURE_RUN_DIR/frames-summary.json"
python3 "$measure_dir/memory.py" after-frames "$chrome_pid" "$hided_pid" "$MEASURE_CDP_PORT" > "$MEASURE_RUN_DIR/memory-after-frames.json"
cat "$MEASURE_RUN_DIR/memory-after-frames.json"
cat "$MEASURE_RUN_DIR/echo-summary.json" "$MEASURE_RUN_DIR/frames-summary.json"
note 'measurement complete; cleaning up owned processes'

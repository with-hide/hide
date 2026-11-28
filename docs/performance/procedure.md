# Measurement procedure

- Code: `scripts/web-shell-measure/`, `web/e2e/hided-fixture.ts`, `desktop/e2e/fixture.ts`
- Index: [PERFORMANCE_TESTING.md](../PERFORMANCE_TESTING.md)

## Verification layers and current CI coverage

There are three complementary layers; none replaces the other two.
This table describes the checked-in workflows, not a claim that a particular PR's remote run passed.

| Layer | What it catches | Current execution |
| --- | --- | --- |
| Deterministic regression tests and structural checks | Blank repaint buffers, cache bounds, incorrect state transitions, blocking work in forbidden paths | The repository invariant checks run on every PR and main push in `.github/workflows/pr.yml`, and the Rust suite on every main push and on each PR whose plan includes it (docs/TESTING.md, "Which lanes a pull request runs"); `design-contract.yml` adds static design checks |
| Automated end-to-end | Real window launch, a private daemon and its pane holders, IPC between the desktop host and `hided`, occluded-window rendering, packaging mistakes | Playwright drives the web shell in a browser and the desktop app through `desktop/e2e/fixture.ts`; the web suite runs in the six Linux `web-e2e` shards and, for its `@platform` tests, in one Windows job, the desktop app in the two macOS `desktop-e2e` shards of `pr.yml`, each when the pull request's plan includes it, and `nightly.yml` calls `verify` with every lane and configures the `@platform` web tests on macOS and Windows, the full desktop suite on macOS and its `@platform` tests on Linux and Windows, with tracked flaky tests blocking nightly; Linux desktop uses Xvfb; `verify` requires the pull request's jobs |
| Controlled performance comparison | Warm/cold latency distributions, periodic stalls, CPU/lock contention, sustained RSS, and cost that grows with process uptime | Local matched baseline/candidate measurements (`scripts/web-shell-measure/run.sh`); `nightly.yml` measures one fixed fixture every night through `perf.yml` on macOS ([The nightly performance lane](web-shell.md#the-nightly-performance-lane)), which keeps a dated record and fails only on an invalid measurement; no pull request runs or requires a performance job |

The repository-invariant Python tests run in the `policy` lane of `pr.yml`, which every plan includes.
Fixture/replay commands under `scripts/web-shell-measure/` do not become CI gates merely because this guide lists them; the nightly lane runs one of them, on its fixed fixture.
Check the workflow before claiming any of them runs automatically.
The per-OS schema/runtime contracts and the package jobs run in a pull request only when its plan names them; actual executed jobs, failures and skips establish a particular head's coverage.
PR macOS queue comparisons use actual executed jobs (exclude skipped reusable placeholders), recorded SHA/time windows and sample counts, and report run wall time and runner cost separately.
A before/after observational sample with different workloads or little concurrent queueing does not establish the concurrent-PR p90 target.
Hosted desktop automation and private hook fixtures do not establish physical IME, first-launch security prompts or real agent hook behavior.

### Maintenance and review policy

1. Every change runs the applicable CI regression gates; a rendering/performance bug gets a deterministic regression test when it can reproduce the observed failure economically.
2. Changes to terminal drawing, selection, input/scroll routing, snapshot delivery, geometry, focus, or attach lifecycle also require the affected `desktop/e2e/*.spec.ts` or `web/e2e/*.spec.ts` scenario before being called verified.
   When a scenario cannot be automated yet, exercise it manually against the packaged app and record that the check was manual.
3. Changes to caching, scheduling, snapshots, locking, or claims of improved speed/memory require the affected controlled baseline/candidate measurement (`scripts/web-shell-measure/run.sh`) as well.
   Apply it only when making a memory or latency claim; do not impose it on unrelated documentation changes.
4. The change author records those results in the PR's folded `Evidence` block (what it did not run goes under `Not verified`), and the reviewer checks coverage and exclusions as well as CI.
5. Keep the regression test beside its component, reusable measurement tools in `scripts/`, procedure and comparison policy here, and each run's raw evidence under `agents/runs/`.
   Never turn a one-off trace or historical number into a hardcoded universal latency limit.

## 1. Define the claim before running anything

Write the symptom, exact reproduction sequence, expected visible result, affected clients, and baseline/candidate revisions in the run record.
Separate these questions: does the content render correctly, how long does an internal stage take, how long until the user sees the requested change, and does retained memory grow?
A faster median does not prove flicker is fixed; a bounded cache does not prove lower p95 latency.
Agree on an acceptance threshold and measurement boundary before comparing results.
Historical measurements from another build or machine load are context, not universal pass thresholds.

Use the same pane content, grid, font/text scale, display, refresh rate, build configuration, and interaction sequence in both builds.
Compare idle and driven conditions separately, distinguish cold-start from warmed-up draws, and alternate baseline/candidate trials to expose changes in machine load.
Report sample count, nearest-rank p50/p95, maximum, excluded observations, and trial conditions, not only the best run.
If the conditions cannot be matched, label the result a functional reference or an uncontrolled observation, not performance parity.

## 2. Identify the build and protect the operator

Before visual checks, record the exact executable path, source revision, dirty diff, build configuration, and PID for `hided` and its `hide-holder` and, for the desktop app, its dev or packaged Electron process.
Inspect running processes rather than assuming the build launched from this checkout is the visible one.

```sh
pgrep -fl hided
```

The operator's app and an isolated candidate may run simultaneously; there is no global single-instance requirement.
Identify the candidate by executable path, PID, and exact window ID before capture or interaction, and re-resolve them after any restart.
Keep the operator's app running by default; building and automated tests do not require quitting it.
Never quit, restart, activate, or otherwise manipulate the operator's app, panes, or server for QA without explicit coordination; never kill all matching processes.
Prove the private `HOME`, `HIDE_STATE_DIR` and `HIDE_DESKTOP_USER_DATA_DIR` are isolated before running both apps; ambiguous targeting or shared state blocks the affected check, not the operator's work.
Background exact-window screenshots need not activate the candidate, but foreground keyboard, IME, drag and focus scenarios can interrupt the logged-in user's work; coordinate a bounded foreground QA window for those, or use a separately authorized machine/session.

Build with the existing scripts:

```sh
bash scripts/verify-web.sh web build
bash scripts/verify-cargo.sh release
HIDE_VERSION='<candidate-version>' bash scripts/verify-web.sh desktop package
```

Replace `<candidate-version>` with the intended local candidate version before packaging.

`verify-cargo.sh release` builds the release `hided`, `hide` and `hide-agent-hooks` binaries; release `hided` embeds `web/dist`, so run `bash scripts/verify-web.sh web build` first.
`bash scripts/verify-web.sh desktop package` (`desktop/scripts/package.mjs`) runs that release build and packages `desktop/out/hide-darwin-<arch>/hide.app`; it refuses to produce an app if any binary it ships is missing or not executable.
For an unpackaged dev run, `pnpm --dir desktop dev` finds this worktree's `target/{debug,release}/hide`.
Archive baseline and candidate from separate worktrees so each keeps its own `target/` and `desktop/out/`; never redirect build output with `CARGO_TARGET_DIR` or share it across revisions.
Building, testing, installing, launching, committing, and merging are separate states; report each accurately.

## 3. Isolate runtime state before making fixtures

Use that exact build for the daemon, the fixture CLI and the pane holders the daemon starts from beside it, not whichever executable happens to be on PATH.
Every invocation, including cleanup, must use the same explicit environment:

| Input | Required isolation |
| --- | --- |
| `HOME` | A private home under the run directory, including kit and retirement state |
| Provider homes | Private `CLAUDE_CONFIG_DIR`, `CODEX_HOME` and platform home/config roots; exclude inherited credential and provider routing overrides |
| Provider executable search | A controlled tool `PATH`, private login-shell startup path and verified fixture resolution for supported provider names; a private HOME alone does not restrict executable discovery |
| `XDG_CONFIG_HOME`, `XDG_STATE_HOME` | Private configuration and state roots under the run directory |
| Inherited `HIDE_*` variables | Cleared before launching the fixture: a Hide pane hands its programs its pane id and capability, which would route the fixture's CLI to the operator's daemon |
| `HIDE_STATE_DIR` | A run-owned `hided` state directory; the pane holders' records and sockets live under it |
| `HIDE_DESKTOP_USER_DATA_DIR` | A run-owned Electron user-data directory for the desktop app |

`desktop/e2e/fixture.ts` builds exactly this environment (`inheritedFixtureEnv` in `web/e2e/platform-fixture.ts` drops every inherited `HIDE_` variable) and refuses to launch unless `HOME`, `HIDE_STATE_DIR` and `HIDE_DESKTOP_USER_DATA_DIR`, and on Windows `USERPROFILE`, `APPDATA` and `LOCALAPPDATA`, resolve under the run's own temporary directory (`assertIsolated`).
`web/e2e/hided-fixture.ts` starts the daemon on that state directory and, when the daemon stops, ends every pane holder its records name (`web/e2e/pane-holders.mjs`), because a holder outlives its daemon by design; `web/e2e/engine-fixture.ts` makes the workspace and panes with the events the web shell sends.
The state directory must also be private (`0700` on Unix): the delivery ledger refuses an existing directory readable by another account, and timing hooks against a daemon logging `ledger.load_failed` measures a failed letter pull rather than working delivery.
`scripts/web-shell-measure/isolated-env.sh` establishes that permission before starting the daemon.
Follow that same pattern for an ad hoc fixture: give every child the same HOME, XDG roots and state directory, and end the holders by their records when the run ends.
A holder's socket lives under the state directory, so keep that directory's path short (directly under `/tmp` on Unix) for the platform's Unix socket path limit.
Do not repurpose the shell's own `HOME`; pass a private HOME explicitly to fixture children, and do not assume a private local socket disables SSH discovery.
Inspect remote registrations, automatic SSH connection attempts, and remote client state too; an unexpected remote connection is an isolation failure to resolve before interacting.
SSH devices are registrations kept under the daemon's state directory (the operator's is `~/.hide/state`), so a run-owned `HIDE_STATE_DIR` is what keeps a scenario that does not exercise remote behavior from making an SSH connection attempt.
Do not edit the operator's SSH configuration or stop remote services to make a local fixture pass.

For a no-provider run, reuse the existing private provider shims and system-tool path pattern in `web/e2e/{engine,hided,platform}-fixture.ts`.
Create the private executable set from the supported-provider declarations, keep inherited environment values to an explicit fixture allowlist, and link required tools individually instead of appending an account's executable directory.
Set private login-shell startup files to the same fixture-bin plus system-tool PATH, and repeat the actual login-shell resolution check after the runner has created its shell startup files.
Check the actual executable each supported provider name resolves to, its inherited and login-shell paths, and its private configuration roots before allowing the daemon's discovery, usage or Background AI work.
`hide-platform/src/programs.rs` searches the login-shell path, inherited tool path and usual installation folders; restricting only one of those inputs does not prove real provider binaries are unreachable.
Record the approved fixture executables and verify that fallback resolution cannot escape to an installed provider; never use a real model or authenticated probe to establish no-provider isolation.
Synthetic status or authentication answers from a fixture shim establish only that fixture's response, not real authentication or provider availability.
`--isolated-headless` selects browser mode; it does not enforce this provider boundary.
Unexpected real-provider discovery or execution invalidates a no-provider measurement even when no authenticated request or model turn is established; stop and retire only the run-owned processes before correcting the fixture.

Save process/socket ownership before launch, prove the private server has zero workspaces before creating fixtures, and verify that the operator server gained no QA connection.
Use explicit fixture IDs, not current-focus shortcuts, for mutations.
Record operator agent count separately from private pane/agent and attached-child counts: isolation does not remove shared machine load.

## 4. Reproduce with real interactions

Sidebar tab segments are snapshot-derived, not a terminal-input path (`web/src/sidebarTabs.ts`).
Each affected checkout scans its tabs, physical panes, agents and visible ownership rows when those inputs change; terminal echo changes none of those inputs.
The feature adds no resident process, timer, queue, saved state or per-key I/O.
Sidebar grouping tests own count/root separation and cross-tab subtree ownership, and the web and desktop `sidebar-tab-groups.spec.ts` journeys own the live geometry, rename/reset and native-window observation, without implying a latency or memory improvement.

`desktop/e2e/*.spec.ts` and `web/e2e/*.spec.ts` are the primary reproduction path: Playwright drives the real window against an isolated `hided` and its pane holders, and both run in CI.
Read the closest existing spec before writing a new one; `desktop/e2e/browser.spec.ts` covers the occluded-window case; the fixture launches every desktop app with `--disable-backgrounding-occluded-windows` and without activating it (see [BUILD.md](../BUILD.md#the-desktop-app)), so an occluded window keeps painting for capture.

For manual QA on a packaged or dev build that a spec cannot yet reach, use the tools and procedure in [VERIFICATION.md: Manual native QA](../VERIFICATION.md#manual-native-qa); it owns which tool can address the candidate without reaching the operator's app.
Do not activate, raise, move or unminimize the window merely to obtain a screenshot without coordination.
Occluded-window capture is not proof of minimized, hidden, or off-Space capture support; report blank, stale, or unavailable frames explicitly rather than silently focusing the app.
Confirm the exact PID/window before each mutation, prefer fresh element IDs, and verify the result with another observation.
Do not automate credentials, unlock prompts, or authentication.

Exercise ordinary shell history, a long Claude transcript, and a long Codex transcript separately: their repaint and scroll behavior can differ.
Use disposable fixtures; do not resume or send input to an operator's live agent.
Capture real screenshots for visible claims; a running process or passing unit test is not UI evidence.
For flicker, record a bounded window region and inspect the resulting dimensions and frame rate before analysis; a blank-frame detector must be calibrated against the fixture's background and known populated/blank frames.

## 5. Measure the boundary actually under discussion

### CPU, lock contention, and idle work

Sample `hided`'s PID and the pane holder of the measured pane in separate short windows during both idle and driven conditions:

```sh
/usr/bin/sample <hided-pid> 3 -file <run-dir>/hided-sample.txt
/usr/bin/sample <holder-pid> 3 -file <run-dir>/holder-sample.txt
uptime
ps -p <hided-pid>,<holder-pid> -o pid,ppid,%cpu,rss,etime,command
```

Retain contemporaneous load, process lists, selected pane/grid, attached children, and refresh rate.
Inspect symbolication before calculating mutex-wait ratios; predominantly `???` frames mean no usable answer, not zero contention.
State the denominator and thread when reporting a wait fraction, and distinguish waiting on the runtime mutex from time spent holding it.
During idle observation, inspect snapshot publications, `rest` revisions, attach counts, and git subprocess activity rather than inferring no work from a static UI.

A `hided` with no window is a separate idle condition, and it is the daemon's resident state: it lives while a holder pane of its machine may still run (ARCHITECTURE.md, The daemon lives with its panes), so it runs while the app is closed.
The historical usage ledger pauses source and price reads without a visible window; its owned thread waits for the latest visibility hint with a one-second maximum sleep.
Its public price table uses at most one persisted attempt a day, fetched by the own node from one fixed URL with a five-second deadline and 16 MiB cap; no usage payload or credentials are sent.
For first-backfill acceptance compare the same isolated candidate and terminal workload with an empty source and with the real source exposed read-only, recording source bytes/files, completion time, bounded/skipped status, idle and driven CPU, and echo samples separately.
Do not copy the full history or allow the candidate to write to the operator's source folders; record the read-only enforcement and exact candidate/window identities with the run.
With no web or desktop renderer attached, the coordinator rests screen-only foreground-process, hook-diagnosis, ports, GitHub, disk and Background AI reads.
The worktree catalog is read either way: `hide mission create` and a mission's starts read a checkout's project from it, and a Conductor runs on a machine with no window open (with-hide/hide#2708 found one told `mission_projects_loading` for as long as no window attached).
Its cost with no window is the cost with one: the watch's heartbeat, and one worker read of the changed projects no sooner than 5 s after the last, none while Git is quiet.
The Changes pump and the Explorer watch follow the same attach state from their own owners: `changes_target` is none and both Explorer watch targets are none while no renderer is attached, and the persisted selection brings both back with the first renderer.
Historical usage source/price work pauses, but its stored engine report can still be read on the existing report interval; mobile and notification work keeps its own demand.
A device's Git watch is ended while no renderer is attached (`git_watch.rested`), so no report waits for a reader that is not coming; the first read after a renderer attaches starts it again and reads every project, since a change may have landed while it was off.
Provider usage is a separate gate: `runtime/usage_limit.rs::reads_usage_headless` keeps it eligible for Memory or usage-limit demand, on the usage reader's own schedule.
Label and other non-screen resident owners keep their existing admission and stop rules; no renderer does not mean that all background work stops.
The root follower parks screen snapshot reads/sends while it has no clients and catches up when a client arrives; independent followers and core state publications are separate owners.
Report attached and detached conditions separately, including agent count, enabled Memory/usage work, links, phones and notifications, and verify the screen-only readers and root follower against their actual demand.
A change to the coordinator's readers or to the root follower has to keep this: a new reader that feeds a screen belongs in the gated block of `run_coordinator`, and `ui_attached` is sent by the daemon only, never by a window.

### Memory and long-uptime degradation

For an RSS claim, take eleven one-minute samples across ten minutes per build and retain process lists, endpoints, range, and median.
`scripts/web-shell-measure/memory.py` records one instant: browser-tree RSS, hided RSS and page JS heap.
No script takes that series: run `memory.py` once a minute for ten minutes beside the workload.
Keep pane count, occlusion, warm-up, and workload comparable; memory pressure can lower RSS without an allocation improvement.

A build that is fast at launch can still become slow after hours, because cost that grows with process age is invisible in any short window taken on a fresh process.
Whenever the report is "it got slow", record `hided`'s uptime first and treat a fresh-launch measurement as the baseline, not the answer.

```sh
ps -p <hided-pid> -o pid,etime,%cpu,rss,command
vmmap --summary <hided-pid> | grep -E "Physical footprint"
```

On macOS, `ps %cpu` is a decaying average over up to a minute, not a process-lifetime average; `etime` records age.
Use cumulative CPU `TIME` differences divided by monotonic elapsed time for matched observation windows, as `resources.py` does, and state whether the sample covers the root or its live descendants.
Compare peak and current physical footprint separately; a larger peak indicates a past transient working set and does not establish the cause of current latency.
The restart test separates accumulated state from workload: quit `hided`, relaunch it against the same private server so the panes and their processes are unchanged, and repeat the same sample window within a minute.
A hot path that disappears on relaunch and returns only after hours is accumulated state and needs an ownership or bounding fix, not a faster implementation of the same path.
A hot path that is equally hot on the fresh process is workload, and belongs to the boundaries above.
Record uptime, the restart time, and both sample windows in the run directory, and state which of the two conclusions the evidence supports.

## 6. Preserve the architecture while fixing the cause

- Keep subprocesses, blocking I/O, and large serialization outside `Mutex<Runtime>`.
  `snapshot_delta_payload` (`hide-core/src/runtime/snapshot_delta.rs`) takes owned data under the lock; `serialize_snapshot_delta` serializes without a runtime to lock.
  Extend `PrecomputedCatalog`, `CatalogCache`, and `PathIndex` (`hide-core/src/session_sync.rs`, `hide-core/src/workspace.rs`) rather than adding per-tick or per-tab git calls; stale precomputation keeps the accepted catalog.
  The catalog's `BackgroundRead` worker asks the core's own node about every path in bounded `Call::PathFacts` chunks (at most `PATH_FACTS_LIMIT` paths and `PATH_FACTS_ANSWER_LIMIT`, 16 MiB, per answer; a larger answer is refused and records `catalog.path_facts_failed`).
  One read runs and changed inputs coalesce to the latest request; a completion wakes the existing republish channel, with no additional timer.
  The current connection and wanted paths fence fact acceptance; registrations and worktrees independently fence the prepared catalog at final application, so display metadata changes do not starve path facts and no delayed answer replays old topology.
  Identical facts keep their pointer and changed topology while a refresh runs repeats no catalog build when its catalog inputs are unchanged.
  Cancellation is checked between chunks; shutdown joins the current reader call outside Runtime, whose ten-second deadline bounds its wait, rather than claiming the whole multi-chunk read ends in ten seconds.
  The existing local node implementation can retain its detached host work and capacity slot after that wait times out; cancelling the reader does not interrupt that underlying host operation.
  The collected facts and request set remain proportional to the registered/session paths; the chunk byte bound is not a whole-request byte bound.
- Keep the session-sync thread that applies the engine's events free of blocking work (PRD instant-pane-topology D-13).
  A socket read that opens its own connection goes to the sync-read worker (`session_sync/sync_reads.rs`, one read of each kind in flight), a subprocess, a node read that runs one or an HTTP request to a `BackgroundRead` reader (`hide-core/src/reader.rs`), and a file write that ends in an fsync to its store's own thread (the label store); the work's output and interval stay what they were.
  An event that arrives while one of them runs is applied at once, which the topology scenario's churning agents exercise.
- Keep a publish's cost independent of the agent, tab and checkout counts when the catalog's inputs did not move (D-18).
  The catalog is rebuilt only on changed registrations, spaces or worktrees or its refresh window, the purpose mirror syncs only when that catalog or the unconfirmed created purposes changed, and a path is resolved by the node only when the set of paths asked changes (above), never by the core per publish; an agent status change repeats none of them.
  A new pane's cwd read answers publish only when the read changed a cwd, the pane was shown at a sibling's cwd other than the one it confirms, or the publish before it was held for that pane (PRD grid-ahead-of-herdr D-12): a split or a new tab publishes once with a stand-in cwd and the answer that confirms that same cwd publishes nothing, where it repeated the whole publish before (21 of 21 answers at operator scale changed nothing).
  Measure a change to this at both scales of the topology scenario, idle and with its agent churn, against a baseline: idle CPU and RSS must not grow, and a status-change publish must not double when the agents do.
- A drawn-ahead change adds one publish, a bounded line (`GEOMETRY_QUEUE_LIMIT`, `INPUT_HOLD_LIMIT_BYTES`) and one `pane_op.timing` record per operation, and nothing per frame or per tick.
  A terminal's PTY is resized once per drawn layout change its rectangle takes part in, at the drawing, and once more only when the engine's layout differs from the drawing (PRD grid-ahead-of-herdr D-03, D-04); nothing holds a grid for the engine's confirmation, and nothing compares a held set per publish.
  For a resized pane the record's `first_frame_ms` is the first frame after its resize went out and may precede `applied_ms`; a change that leaves a pane's grid where it was sends no resize and its record ends `incomplete`.
- Announce once per burst and clear the `ChangeNotifier` (`hide-core/src/handle.rs`) latch before taking the snapshot lock.
  Read-then-clear can swallow a concurrent change.
- Size snapshot traffic by changes: rarely-changing revisioned `rest` and per-event scalars; terminal output never rides a snapshot (it goes from the node to the hub, see `docs/ARCHITECTURE.md`, The terminal path).
  The shell negotiates `semantic-rest-v1`, whose deltas send changed navigator/status fields and lists rather than the whole `rest` object; a client without that negotiation still receives legacy complete `rest` when it moves.
  An unused heartbeat can still create unnecessary changed-field traffic, notification or capture work; count semantic-section bytes separately and never infer a full navigator resend from one changed scalar.
  Initial snapshots and cursor resets carry complete state, so distinguish them from steady-state deltas.
- Keep the keyboard path to the byte bridge: a key goes from the web socket to the router to the pane's writer and never takes `Mutex<Runtime>` (PRD core-host-node-terminal B5).
  The node adds the operator-submit check per key (`hide-node/src/terminal/input.rs`, `submits`, PRD overview-request-view D-19): a chunk over 64 bytes is skipped and a shorter one is scanned in place.
  The core hears a key only as an input fact: the first key after a quiet second, an Enter and a key that moves the keyboard at once, and the keys between folded into one report a second later; a report is one push onto the report channel, and the core takes waiting reports into the runtime in one batch and publishes once.
  Each input fact also asks whether the desktop's `hide_active` was sent this UTC day (`Runtime::note_operator_active`): two flag reads and one day comparison, and the one capture and state save a day happen only when the day turns ([product usage statistics](../ARCHITECTURE.md#product-usage-statistics)).
- Terminal selection tracking (`web/src/terminals.ts`) retains one drag selection per pane in history rows with the cells of every row it covers, at most 20,000 rows (`MAX_SELECTION_ROWS`); a frame reads only the selected rows it shows, and a drag past the edge keeps one `terminal_scroll` in flight every 30 ms; a program drag sends at most one `terminal_mouse` motion per animation frame.
  A wheel still coalesces into one scroll dispatch per animation frame; tracking adds no core notification, timer or input queue.
  Only a pending scroll installs a write callback: it reads the selected rows first, and searches the current screen only after those rows change.
  A settled clipped selection retains its original ends without adding work to ordinary output; clearing, replacing, resizing or parking the selection releases its tracking state.
  `web/e2e/terminal-selection-scroll.spec.ts` owns the partial-edge return and original Copy text regression alongside the existing continuous-scroll and offscreen-copy journey.
- Keep async operation records bounded by active intent and conflict scope; a tab's geometry operations wait in its line, one with the engine and at most `GEOMETRY_QUEUE_LIMIT` behind it.
  A close or topology mutation uses an absolute five-second stage deadline; expiry becomes a caller-visible unknown result and never schedules a destructive resend.
  Status checks are read-only and are started only for an ambiguous close or an explicit status action, so unknown activity does not become a polling loop.

Follow engineering principles 1, 7, 12, and 13: remove obsolete paths, reuse existing mechanisms, test observable outcomes, and fix the failure class.
For timing tests, distinguish a deterministic policy threshold from eventual UI delivery under scheduler load.
Do not weaken an externally promised deadline to make a flaky test pass.

## 7. Regression gates, cleanup, and verdict

Use the existing suite wrappers, then the applicable gates in [CONTRIBUTING.md](../../CONTRIBUTING.md):

```sh
bash scripts/verify-cargo.sh test
bash scripts/verify-web.sh
```

Run the full relevant suite before delivery, and retain pre-fix failure plus post-fix success for the specific regression when feasible.
Do not substitute a unit test for end-to-end coverage or an end-to-end smoke test for long-duration/load coverage.

Stop only owned recording/logging processes, the test app, fixture clients, and the explicitly routed private server.
Verify their exit and socket cleanup before removing or trashing only the exact recorded private state paths.
Never use broad process-name kills, a workspace root as a deletion target, or a `hide stop` without the run's own state folder.
Leave the operator's app untouched unless its shutdown was explicitly coordinated; in that case restore it afterward.
Verify the owned candidate exited, the operator's app remains available, and operator server ownership is unchanged; multiple independently identified instances are not themselves a verification failure.

The run verdict must include:

- Exact revisions, bundles, configuration, PIDs, runtime pin, and isolation evidence.
- Reproduction steps and observed results per client/scenario, with local evidence paths.
- Metric boundaries, distributions, sample counts, exclusions, load, and comparison conditions.
- Regression tests and gates run, failures, and remaining checks explicitly marked unrun or blocked.
- Cleanup/restoration evidence and whether the fix was merely committed, built, installed, or merged.

Use a qualified verdict when coverage is bounded: "no whole-body blanking observed in these recordings" is supportable; "all performance issues resolved" is not.

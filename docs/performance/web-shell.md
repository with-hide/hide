# Web shell and browser gateway measurement

- Code: `scripts/web-shell-measure/`, `scripts/holder-perf.py`, `hide-node/src/terminal/echo_timing.rs`, `hide-holder/`
- Index: [PERFORMANCE_TESTING.md](../PERFORMANCE_TESTING.md)

## Web shell measurement

Published figures and their measurement boundaries live in the dated performance observations, `docs/performance-results.md` in with-hide/hide-private.
That report is an observed candidate result, not a new threshold or a replacement for the measurement contract here.

`scripts/web-shell-measure/run.sh` exercises the product `hided` with separate screen-key and driven-output workloads.
Baseline and candidate must use the same driver revision, fixture, browser mode and measurement boundary.
It owns every process it starts: the release `hided` with its embedded `web/dist` on a private HOME and XDG folders (`isolated-env.sh`, the same isolation as section 3), its state folder in a run-specific mode-0700 folder under `/tmp`, every pane holder that `hided` starts from the `hide-holder` beside it, one measured Git checkout, and one Google Chrome with an automatically assigned CDP port on the page opened with `?probe=1`.
The runner reads Chrome's CDP port from its own profile and sends the page URL over standard input so the token is absent from the Chrome command line.
Nothing of the operator's is read or touched.

Echo is fifty screen-key samples per trial from `key-echo.mjs`, each timed from renderer keydown to xterm write completion on the page's clock.
The collector records the boundary, expected sample count and successful completion explicitly.
`summarize.py echo` rejects missing, incomplete, negative, nonfinite or nonnumeric samples and an unproven clock boundary; one invalid trial prevents an aggregate verdict, and invalid input exits nonzero.
A valid run reports distributions with `pass: null` until the caller supplies `--max-p95-ms N`, the acceptance limit agreed before the run; a failed explicit limit also exits nonzero.
No historical value is a default threshold.
Neither the screen-key samples nor xterm write completion prove physical-key-to-screen latency or compositor presentation.

Run it as `MEASURE_SCENARIO=<keys|topology|restore> HIDE_MEASURE_RUN_DIR=agents/runs/<slug>/measure/<attempt> bash scripts/web-shell-measure/run.sh` after `bash scripts/verify-web.sh web build` and `bash scripts/verify-cargo.sh release`; `topology` is the default.
For an unattended comparison, append `--isolated-headless`: Chrome has no native window.
Use that same browser mode for baseline and candidate, and report it with the results; a headless measurement does not prove native presentation.
`resources-*.json` record one-second CPU-time deltas and RSS sums for the owned `hided`, pane holder (`engine`) and Chrome process trees, independently of the echo samples.
Each sample also keeps the root's own RSS and each child's RSS by its accounting name, because a child caught between fork and exec reads as a second copy of its parent's pages: a bound on hided's memory reads `hided_self_rss_max_kb` (hided alone), and the tree sum says what its children add.
A first run opens on Main (PRD S6), so the harness uses the Projects sidebar to open its fixture checkout before measuring, selecting its exact checkout id.
The fixture is a private primary Git checkout; it is not performance evidence for opening linked worktrees.
Before starting Chrome or collecting any performance sample, `fixture.mjs` writes the actual and target workspace, tab, pane and configured printer counts to `scale.json` and refuses any mismatch.
The operator shape has four split printers and one existing project pane printer, for five printers inside the same 43-workspace, 62-tab, 66-pane total; the double shape similarly uses eight splits and two existing panes.
Root printers belong to projects without extra tabs, so their startup FIFO cannot capture another tab's shell.
Starting the output waits at most 30 seconds for the FIFOs and one marker per printer, written after its first flushed line; `printing-start.json` records that startup observation outside the measured window, not continuous output coverage.
The marker adds one file write per printer before measurement and none inside the printing loop after its first line.
Append `--fixture-only` after `--isolated-headless` to exercise a scenario through exact topology and browser pane readiness, then clean up without collecting echo or resource samples.
The Projects control is a tab, so readiness reads `aria-selected`; opening the fixture is one click after its checkout row appears.
At a scale the fixture's project is the one without agents, so its row is an ordinary row, its project's own fold and its Checkouts fold hold its worktrees; the harness opens those folds on the fixture's project alone, each once, because a fold is the core's state and a second click closes it.
The daemon's state folder lives in the run's short folder while the run lasts, because the holder sockets are under it and a run folder's path is longer than a Unix socket address takes, and the run's end moves it to `isolated/hide-state`; the holders outlive the daemon by design, so the run ends each by the pid its record names and fails when one still runs, writing how many did to `cleanup-processes.txt`.
The run directory keeps `identity.txt` (head, dirty count, binary hash, Chrome version, scenario, engine, load), the scenario's measurements and summaries, and the owned PID status at cleanup.
The harness resolves that directory to an absolute path before starting child panes, so their private HOME, state, and checkout paths remain valid after the pane changes directory.
`page.json` records the pane, split and tab counts, the attached pane ids and the live xterm instances the run started from.
`memory.py` samples resident memory after the shape settled and after the scenario's driven phase (`memory-settled.json`, `memory-after-topology.json`): the Chrome process tree's RSS sum, hided's RSS, the page's JS heap and its live instance count.
The same validity checks and explicit echo-threshold contract apply to every fixture shape.
A headed run opens Chrome with backgrounding/occlusion switches disabled; a headless run has no native window.
Record that mode with the load, and do not focus or move the operator's windows to improve a measurement.

### Pane topology latency

`MEASURE_SCENARIO=topology` measures how long a split, a zoom and unzoom, a pane close, a new tab, a tab switch and a window resize take to reach the screen (PRD instant-pane-topology D-15, B23; PRD engine-bundle-3-core-owns B43).
`fixture.mjs` makes the whole topology with the web shell's own events on the daemon's WebSocket (`create_workspace` with the confirmation Add project sends, `create_tab`, and `focus_pane` then `create_pane` for a split, which acts on the current pane), and every key the run types goes through the page (`type.mjs`, `key-echo.mjs`, CDP), so a pair of runs differs in the build alone.
It adds a second tab to the measured workspace, and with `MEASURE_SCALE=operator` or `double` first fills the session to that scale (D-18): 43 workspaces, 62 tabs and 66 panes, 5 of which print a line every 50 ms once the idle sample is taken, or twice each (86, 124, 132, 10); every other workspace is its own Git checkout.
No pane reports an agent and no agent state changes, because a report has no daemon event, so the echo and the rounds carry no agent-status churn, and a figure from the series before 2026-11-02, which did, is not compared with one from this.
A printing pane is a split whose folder holds a FIFO; the run's private `.zshrc` waits on it in the foreground and `fixture.mjs drive` releases it, so the printer is the pane's own shell and ends with it.
The run records `resources-idle.json` before anything moves, one echo trial (`echo-summary.json`, the typing half of B24), and `topology.mjs`'s rounds (`MEASURE_TOPOLOGY_ROUNDS`, 20 by default).
Each round ends with two resizes: the page's width set to 1100 and back to 1280 with CDP `Emulation.setDeviceMetricsOverride`, `screen_ms` the canvas's new width and `frame_ms` every shown pane's new grid.
While the rounds run, `save-watch.py` reads every 10 ms the files the layout is saved in (the daemon's `core-state.json` and `workspace-views.json`, and the own engine's `engine` folder), and `save-summary.json` gives the writes and bytes per group and per operation the rounds made (`topology.json`'s `operations`, which counts the new tab's untimed close).
The initial resource sample precedes topology churn, while `memory-after-topology.json` and `resources-after-topology.json` follow the driven phase.
This scenario collects no continuously sampled driven CPU interval; its initial resource result is not driven CPU evidence.
Keys are CDP `Input.dispatchKeyEvent` events, which Chrome delivers to the focused terminal as an ordinary renderer keydown, so each chord takes the web shell's own keydown path and its browser chord; a tab switch is a click on the tab.
Every time is on the page's clock from the chord's first keydown (or the click): `screen_ms` is the first animation frame whose DOM shows the change (the canvas's pane count, its zoom flag or the shown tab), and `frame_ms` the first frame after it in which the terminal the change is about shows it (text in a new pane, a new grid in a resized one).
`topology-summary.json` gives each operation's p50, p95 and max to screen and to frame.
`MEASURE_HIDED_BIN` points the run at another hided, such as a baseline built from the merge base with its `hide-holder` beside it, so baseline and candidate share the fixture; run both headless (`--isolated-headless`) under the same scale and report the load beside each.
The stage times are read from the candidate's `pane_op.timing` lines in the run's `hide-state` diagnostic log (`summarize.py timing`, Drawn ahead of the engine in [ARCHITECTURE.md](../ARCHITECTURE.md#drawn-ahead-of-the-engine)).

### Restore after a reboot

`MEASURE_SCENARIO=restore` (PRD engine-bundle-3-core-owns B44) builds the topology fixture at `MEASURE_SCALE`, opens the page on it, and then kills every process the run started for the panes with SIGKILL: the daemon, then every pane holder its records name and everything under each.
It writes the list to `restore-victims.txt` and fails unless none of them runs within ten seconds, then notes t0 and starts the daemon again.
`restore.mjs` times from t0 on the machine's wall clock: `tabs_ms` until the snapshot lays out every tab of the fixture again, `front_ms` until the page, opened as the daemon answers `/health`, shows the measured checkout's tab with every pane of it drawn, and `shells_ms` until every pane's shell has read its rc file again (the private `.zshrc` touches one file per shell in `shell-ready/`).
A time that did not come within `MEASURE_RESTORE_SECONDS` (180) is `null`, and a missing front names what the page last showed (`front_last`).
An agent's resume is not timed here; the engine's functional tests own it.

### Judging a change

A change is judged against the build before it with the same pair method.
The 31-row table below was the bundle 3 gate that let the core's own engine replace Herdr (PRD engine-bundle-3-core-owns D-14, D-15, B39 to B44); with Herdr gone, the same rows and the same method judge any change to the terminal path against the build before it (PRD engine-bundle-4-herdr-removal D-16).
Each row is measured as pairs: the build before the change (H) and the candidate (P), five pairs, the order flipped every pair; for the engine switch itself H is the bundle 3 gate commit's own-engine path.
No pair is dropped; each run records its load and the build processes running beside it, and a run is started only on a quiet machine (load under 6, no cargo or rustc running).
A latency passes within 1.1 x H + 0.5 ms and any other value within 1.1 x H, judged on the median over the pairs of P minus that tolerance, so at least three of five pairs are inside it.
Loss, duplication and order of keys, a holder's wakeups while idle, and one holder's memory (4 MiB idle, 6 MiB after a flood) hold in every run, not on a median.
The memory of panes no screen shows and the sum of holders' memory are recorded and not judged: a process per pane is the accepted cost (D-33).

| Rows | What | Measured by |
| --- | --- | --- |
| 1 to 5 (B39) | Idle: CPU over 60 s, holder wakeups, the memory of 64 attached panes, one holder's memory idle and after a flood | `scripts/holder-perf.py` at the node ([The pane holder's path](#the-pane-holders-path)) |
| 6 to 9 (B40) | Key to frame latency alone, beside one and eight flooding panes, and beside a slow consumer, with a slow consumer's holder at most 6 MiB | `scripts/holder-perf.py` |
| 10 to 15 (B41) | A flood: bytes per frame, time to drain, the path's CPU, reattach time, frames per pane at most 62.5 a second | `scripts/holder-perf.py` |
| 16 to 21, 31 (B40, B41, B42) | The app end to end: echo idle and with five panes printing, CPU while printing, idle CPU, CPU with no window, hided's and Chrome's largest RSS, and their RSS after the scale | `scripts/web-shell-measure/run.sh` with `MEASURE_SCENARIO=keys` |
| 22 to 27 (B43) | A new tab, a split, zoom and unzoom, a pane close, a tab switch and a resize, to screen and to frame, p50 and p95 | `MEASURE_SCENARIO=topology` ([Pane topology latency](#pane-topology-latency)) |
| 28 (B40) | Echo at the operator's scale | the same topology run's `echo-summary.json` |
| 29 (B44) | Layout save writes, at most one per operation | the same run's `save-summary.json` |
| 30 (B44) | After every process was killed: the tab list, the front tab drawn, every shell answering | `MEASURE_SCENARIO=restore` ([Restore after a reboot](#restore-after-a-reboot)) |

Rows are measured under the load as it is (R), and key latency alone (rows 6 to 9 at one pane and at 128 panes) also under a standing load L of 28 busy processes, which `holder-perf.py --load 28` starts beside the path and ends with the round, so both builds meet the same load whatever else the machine runs.
The app's engine RSS is judged by the per-holder bound above.
Row 31 also records the holders' total RSS in `resources-after-topology.json`, one sample from the existing resource collector after it reads the surviving holder roots.
That sum is recorded only; `memory-after-topology.json` still supplies the hided and Chrome RSS comparison.
The numbers belong to the run record; this document keeps the method, not a verdict.

### The nightly performance lane

`nightly.yml` calls `perf.yml`, whose job (`topology operator own engine`, on `macos-15`) measures the same fixture every night against the product `hided` built from that night's `main`.
It builds the web shell, the release `hided` at the fixed `target/release/hided` and the release `hide-holder` beside it, and runs `run.sh --isolated-headless` with `MEASURE_SCENARIO=topology`, `MEASURE_SCALE=operator` and `MEASURE_TOPOLOGY_ROUNDS=20`, the run directory under the job's workspace.
The browser is the hosted image's Google Chrome, headless; `run.sh` starts it with `--remote-debugging-port=0` and reads the port Chrome chose from `DevToolsActivePort` in the run's own profile.
`perf.yml` also has a `workflow_dispatch`, so a branch can be measured alone without starting every nightly lane.
Nothing a pull request runs depends on it, and no pull request runs it.

The fixture is the measurement's identity, so it is fixed:

| Part | Contract |
| --- | --- |
| Topology | `fixture.mjs` at `operator`: 43 workspaces and 43 projects (the measured checkout, a linked worktree of the fixture repository, and 42 others that are each their own Git checkout), 62 tabs (the measured tab and the switch target, one root tab in each other workspace, 18 more), 66 panes (one per tab and four splits) |
| Printing | No agent is reported; 5 panes print a line every 50 ms after the idle sample |
| Idle resources | `resources-idle.json`: 20 one-second samples of the owned `hided` and Chrome trees before anything moves |
| Echo | One trial of 50 screen keys on the measured pane, renderer keydown to xterm write on the page's clock |
| Topology | 20 rounds of split, zoom, unzoom, close and new tab by chord, two tab switches by click and two resizes, each to screen and to terminal, with the stage times from `pane_op.timing`, and the layout saves the rounds made |
| Memory | `memory-settled.json` and `memory-after-topology.json` |
| Browser and runner | Headless Google Chrome at 1280 × 900 on the `macos-15` hosted image |
| Driver | `scripts/web-shell-measure/` at the run's commit |

`fixture.mjs` writes the counts the daemon's snapshot answers beside the ones the scale asks for (`scale.json`), and a run whose counts differ is not this fixture.
The series changed on 2026-11-02 (PRD engine-bundle-3-core-owns D-24): the fixture is made through the daemon, without the agent reports and their churn, and each round adds two resizes; a record before it is not compared with one after.
Changing any row of this table, `fixture.mjs`'s operator numbers or the driver's measurement boundaries starts a new series: do it in its own pull request that says so.
Each record carries its engine, the driver's hash, the runner image, the Chrome and `hided` identities and the load, so a reader compares only records of one series; the series of the Herdr job, which ran beside this one until Herdr's removal, ended there.

`scripts/web-shell-measure/nightly.py` judges the run and writes its record, `nightly-summary.json`.
It derives every figure again from the raw files through `summarize.py`, writes the same table to the job's summary page with the record beneath it, and fails the job in three cases only:

- the fixture is not the lane's: another scenario, scale or browser mode, or a session whose topology is not the one `fixture.mjs` asked for;
- a measurement is invalid by `summarize.py`'s own contract: an echo with missing, incomplete, negative or nonnumeric samples or an unproven clock boundary; or
- a measurement is missing, because the run stopped before it.

No figure is a limit.
The guide's echo comparisons are relative to a matched baseline, never absolute, so the echo is reported with `pass: null` and no `--max-p95-ms`.
`summarize.py topology` has no validity contract, so a topology timeout is counted in the table and the record and is not judged.

| Measurement | Nightly lane |
| --- | --- |
| Idle resources, echo, topology latencies, memory | Covered, recorded without a limit; invalid or missing fails the job |
| Driven CPU, key count, windowless and restore | Not covered: not part of the `topology` scenario |

A failed check is annotated as `Failed measurement` (`ci-flaky-report.py --measurement`), and the nightly report opens one `nightly-failure` issue per check, comments on it each night it fails again and closes it when a scheduled run passes the lane ([TESTING.md](../TESTING.md#nightly-failures)).
Its candidates are every pull request merged since the lane's last green nightly, because a measurement can move with any change; a run that stopped is filed by its failed step instead, with the lane's workflows and scripts as candidates.
The authors of the candidates look first.
A fixture failure means the driver or the runner no longer builds the fixture; an invalid echo means keys stopped reaching the terminal the way the probe measures, a product defect candidate.
When no candidate explains it, for instance a new runner image or Chrome that the record's identity shows, the issue goes to the operator, who decides whether the series has to start again.
A green run proves the measurement was valid, not that performance held; read the trend.

Each run keeps its run directory, without the throwaway Chrome profile, as the artifact `perf-run-own-<date>-<run id>-<attempt>` for 30 days, and its record alone as `perf-record-own-<date>` for 90, the `-own` names its series began with.
Read a series with `gh run download <run id> -n perf-record-own-<date>`; nothing about a run is committed.

The job costs about 17 runner minutes a night on `macos-15`, most of it the release build.
Its first run on a branch, with no Rust cache, took 21.8 minutes when the release profile had one codegen unit: about a minute of setup, a minute for the web shell, 16.4 minutes for the release `hided` (14.4 in another run) and 3.6 for the measurement.
Sixteen units cut a clean macOS release build by 29% (`docs/BUILD.md`, The release profile), so the release `hided` should now take about 12 minutes.
The same change raised `hided`'s resident memory by about 5 MiB once, so the series steps up by that much at the merge that moved to sixteen units, not through a leak.
Main's run saves the release dependencies to the cache, which saves about a minute and a half at most, because `hide-core` and `hided` themselves are most of the build.
The job stops at 45 minutes and the build at 30, so a stalled run is cut short rather than holding a runner.

### Screen key echo, lost keys and the terminal path's cost

`key-echo.mjs` types into the measured pane's terminal with CDP `Input.dispatchKeyEvent`, which takes the shell's own keydown path and its `/ws` `key` frame.
Each sample is one key: the marker `aNNNN` is typed and waited for, then its last key `z` is typed, and the sample is the xterm write completion that first shows `aNNNNz` minus that key's keydown, both read on the page's clock (the keydown by a capture-phase listener the harness installs on `window`); the pane runs `stty -echo -icanon; cat`, so the drawn character is the pane's echo.
The CDP dispatch hop before the keydown is left out; these figures characterize the path from renderer keydown onward, and do not establish physical-key-to-screen latency or the cost of input delivery before that listener runs.
The harness refuses a sample that is nonfinite or negative on that one clock, or whose keydown never reached the page, rather than counting it.
Beside each sample it keeps the same completion minus the driver's own clock before the key was sent (`node_t0_samples`) and the driver's offset from the page's clock, read by a round trip bracketed on the driver's clock (`clock_offset_ms` per hop), and `summarize.py echo` reports both per trial; they are a cross-check, not the measurement.
Until 2026-10-10 the sample was that two-clock value: the driver and the page each fix their time origin when they start, and the constant between them changed per run (+75, -21 and -22 ms in three runs of PR #898's comparison), so key echo numbers measured before then carry an unknown per-run offset; only a large one showed, as impossible samples.
The key collector also records every `/ws` frame the page received while it typed, by type (`ws_frames_by_type`), so a key that made the core publish shows as a `delta` frame (B5).
The node writes the server's share of the same path to the daemon's diagnostic log: `terminal.echo_timing`, once a second while input comes into a pane and once when its session ends, times from the node writing the first input since the pane's last frame to the next frame it reads (`count`, `p50_ms` and `p95_ms` as upper edges of half-millisecond buckets up to 50 ms, `mean_ms`, `max_ms`), so a pair of keys runs compares two builds without the hub and the page (`hide-node/src/terminal/echo_timing.rs`).
It costs one clock read and one atomic exchange per input and per frame, and one record a second per pane being typed into.
Its samples are the inputs that open a wait; the holder answers each input with a frame, so it times nearly every key.

`key-count.mjs` types `MEASURE_KEY_COUNT` keys (10,000 for the claim) one CDP `Input.dispatchKeyEvent` each, the same key path as `key-echo.mjs`, spelling distinct markers (`mNNNNN` and a space, Enter after every 100th, `#` last), then reads the terminal program's received bytes through `MEASURE_PANE_READ` and compares them with what was typed: `exact` when they match key for key, otherwise how many keys never arrived (`keys_lost`), arrived outside the longest in-order common run (`keys_reordered`) or arrived without being typed (`keys_unexpected`), with the markers that are missing for a reader (D-08).
CDP `Input.insertText` does not reach the shell's key path, so it cannot stand in for typing.
It never stops at the first loss.
`MEASURE_KEY_INTERVAL_MS` spaces the keys (0 by default: as fast as CDP takes them, in order).

`MEASURE_SCENARIO=keys` runs both checks on the shape the terminal claims are made on: the measured pane and four splits in one tab, five attached panes, created through `fixture.mjs`, with fixture commands sent through `type.mjs`.
For the key count, the foreground program is `tee <private receipt>` and the reader is `cat <private receipt>`, so wrapping in a narrow split cannot change the received bytes.
The resource files name the pane holders' group `engine`: all five owned pane holders and their descendants, counted once.
Each group also records `root_pids`, `self_cpu_percent` and `self_rss_kb`; use hided's own fields separately from the engine group, since its descendant tree can overlap the holders.
It records, in order: idle resources over `MEASURE_IDLE_SECONDS` (60) in `resources-idle.json`; the idle key echo (`key-echo-idle.json`); every pane printing a line per 8 ms (foreground `keys_printers.py print`, stopped with Ctrl+C) while resources are sampled over `MEASURE_DRIVEN_SECONDS` (120, `resources-driven.json`), with the page's `/ws` frames by type, the full redraws it asked for (`ws-driven.json`) and the core log's terminal and redraw records before and after (`driven-diagnostics-*.json`); the key echo again on the measured pane, back to a plain `cat` so its marker arrives whole, while the other four keep printing (`key-echo-driven.json`); the key count when `MEASURE_KEY_COUNT` is set (`key-count.json`); and the page navigated to `about:blank` while the panes print for `MEASURE_WINDOWLESS_SECONDS` (60, `resources-windowless.json`, browser omitted from that sampler), then navigated back, with each pane's drawn text length (`reopen-panes.json`), the page's frames and full-redraw requests around that navigation away and back (`ws-reopen.json`, `ws-count.mjs`) and the core log's terminal, redraw and resume records (`reopen-diagnostics.json`, `diag-counts.py`).
Before either printing phase, `keys_printers.py start` delivers all five commands and verifies their first flushed output under one monotonic 30-second deadline, including command delivery, receipt waiting and the final process identity checks.
Each phase has a new private folder and nonce; each printer atomically publishes one receipt naming that phase, pane, PID and OS birth after its first flushed line.
Missing, stale, duplicate or invalid identities fail the run, preserving partial receipts and taking the existing fixture cleanup path; delivery children are ended and waited for through `holder_owned.Owned`, with cleanup duration and errors recorded separately from startup.
The same five PID/birth identities must still run before and after the driven and windowless resource windows; after the echo pane switches to `cat`, the other four must run before and after the driven echo.
An early printer exit fails the run even when its resource sampler exited successfully; the printer durations remain driven seconds plus 120 and windowless seconds plus 10, measured from each printer's start, without extending measurement windows or sample counts.
These receipts prove first flush and identity survival at the boundaries, not uninterrupted output or rendered pixels.
The reopen observer starts after printer readiness, registers its listeners and completes `Network.enable` before atomically publishing a fresh `ws-reopen-ready.json`; only a live matching observer's ready receipt permits navigation to `about:blank`.
Its existing windowless seconds plus 30 observation duration is unchanged.
`scripts/tests/test_keys_printers.py` checks the process/file contract without a browser; actual keys and reopen measurements remain separate evidence.
`summarize.py resources` reads hided's median one-second CPU and its largest RSS from a resources file; `summarize.py echo` reads a key echo file like an echo file.
Chrome remains alive during this detach/reconnect phase; verify private `/health` client retirement and reacquisition independently, and use the isolated desktop lifetime journey for native last-window or browser-process exit.
Use valid trial distributions with the current matched comparison; supply an explicit echo threshold only when that absolute limit belongs to the agreed claim.

A comparison runs the baseline and the candidate `hided` (release builds, each from its own worktree with its `hide-holder` beside it, `MEASURE_HIDED_BIN`) in alternating trials on the same machine, idle and driven reported separately, with nearest-rank p50 and p95 per condition and the load recorded beside each trial; the claim's thresholds are the PRD's (B2: p50 and p95 no more than 5 ms above the baseline; B21: median CPU no more than 10% above and largest RSS no more than 32 MiB above; B19: windowless median CPU no more than 10% above), never a past run's figure.
Keep `--isolated-headless` the same for both builds.
A device's pane has no harness here: the device scenarios drove a device's Herdr and were removed with it, so a device claim is checked by hand against an isolated device account (`docs/VERIFICATION.md`, A device check).

### The pane holder's path

`scripts/holder-perf.py` measures a node's pane holders at the node (PRD engine-bundle-2-pane-holder D-17, B15, B21); it runs on macOS by hand, since its process figures are macOS's (`proc_pid_rusage`).
It measured the holder path against the Herdr session path it replaced until Herdr's removal; it now compares two builds of the holder path, the build before a change and the candidate, by the same pair method ([Judging a change](#judging-a-change)).
One round measures one build at one pane count, with release builds of `hide-holder` and hide-node's `holder_node` example copied to a folder of their own (`--bin`; a build in the checkout during a run relinks them), N holders started the way the daemon starts them (`Holders::start_local`); every pane runs `/bin/sh` at 120x40, and the keys go through the node's own `key`.
The path's processes are the holders; the shells are left out.

A round records, each in its own phase:

- idle for 60 s with no screen attached, then with min(N, 64) attached: CPU (user and system), memory (`phys_footprint`, all processes and the largest one) and wakeups (idle-exit and interrupt wakeups), the kernel's counts for each process over the window;
- key-to-frame latency on one extra pane running `holder_node echo`, which draws each token typed alone on its first row and logs every byte it receives: a 12-letter token, every letter changed from the one before so a frame carries all of it, is typed through the node's `key` and timed until a frame showing it reaches the node's screen sink, 200 tokens 40 ms apart (the holder draws at most once per 16 ms after its last frame for output alone, so keys back to back would measure that pacing), alone, beside one pane flooding 300 MB, beside eight panes flooding a quarter of that each, and beside one pane flooding a quarter of that whose connection alone is read slowly (the slow consumer): the holder's socket is moved aside behind a proxy that passes the node's messages at once and reads the holder's for 10 ms of every 40, so the holder's own backpressure and fragment cap run; the node's sink is never slowed, and the node's own memory is read before, right after and once still;
- each flood's frames and bytes reaching the node and the path's CPU, and the memory again once the screen has been still for 2.5 s, past the second after which a holder gives freed pages back;
- the node killed outright and a new one started: the time until every attached pane drew again, and 50 more tokens;
- with `--phases flood_unattached`, one holder flooded with no screen attached, its memory right after and once still;
- the echo's log against every token typed: loss, duplication and order on the input side; a token never drawn within 10 s is a loss on the screen side.

Each round also records, in every phase, the load average and the build processes running, and a pair either of whose rounds saw a build process in any phase is dropped from the comparison and listed.
A claim compares the two builds in alternating rounds on the same machine: five pairs, the order flipped each pair, at 1, 64, 100 and 128 panes, judged by the median of the kept pairs' candidate-to-baseline figures, each pair's own figure and load shown beside it; absolute figures from such a run are figures under load, and loss, memory and wakeups are judged in every kept round.
Everything a round starts (the node, the holders and the busy processes) is ended by pid and start time on every path, a stop signal included (`scripts/holder_owned.py`), and a round that leaves any of it running fails.
`agents/runs/<slug>/` keeps the rounds; nothing of them is committed.
The latency, CPU, loss and wakeup bounds are the PRD's (B15, B21), now against the build before: latency p50 and p95 no more than 1.1 times the baseline's plus 0.5 ms, flood CPU no more than 1.1 times the baseline's, no loss, duplication or reordering, and no wakeup of an idle holder over the window.
The memory bounds are not the PRD's 3 MB per holder: they were decided for the bundle on 2026-11-01, because a holder is a process of its own, about 2.8 MB before it draws anything (code, the system libraries' data, five threads' stacks), so one with a screen attached measured 3.47 MB idle, its connection's buffers included, past 3 MB; and because a flood fills the 2 MiB scrollback the parser is given (D-17), which 3 MB had not counted.
They are every holder at most 4 MiB idle, attached or not, and 6 MiB, that and the 2 MiB scrollback, once a flood has settled.
On this Mac (Apple M-series, 14 cores, 2026-11-01), against the Herdr path it replaced, every bound passed: the holder path's key-to-frame p50 was 0.15 to 0.39 ms against Herdr's 1.0 to 20 ms (medians per phase and pane count), with no token lost, repeated or out of order; flood CPU was 0.19 times Herdr's for one pane flooding and 0.92 to 1.03 times for eight; an idle holder woke 0 times in 60 s, attached or not, past the one wake a second after its last frame; one holder held at most 3.3 MiB idle and 5.9 MiB once a 300 MB flood had settled (8.5 MB before freed pages were given back), and one flooded with no screen attached 5.2 MiB; and 64 attached panes' holders held 0.73 times the Herdr path's server and session clients.
With one pane's connection read slowly, the slow pane drew 44 to 48 frames of a 75 MB flood against about 460 unslowed, the other panes' p50 stayed 0.19 to 0.24 ms, and the node's own memory grew by 0.04 to 0.11 MiB; a 120x40 frame is about 6 KB, so the 4 MiB fragment cap is the unit tests' to reach, not this run's.
On Linux and Windows CI holds the boundaries a short run can (`hide-holder/tests/it/idle.rs` for wakeups, the lifecycle and record tests for input and the caps); the comparison is macOS's.

## Scoped browser gateway discovery

Browser inventory area identity is projected in the existing changed-layout generation pass, one visit per area and display, with no additional notification or timer.
Each sync also compares borrowed identities for the saved Workspace layouts (256, more only while every one holds tabs) against the current connected checkout catalog, with no allocation when the scope is unchanged.
A scope transition advances that same generation and rebuilds the inventory; removed or disconnected checkouts retain their saved layouts but retain no native page authority.
Positive area scopes are collected in that same pass (the saved Workspaces above times 257 areas: one per area that holds a display, and an empty front area), with no allocation or extra notification on unchanged generations.
Changed generations build one bounded map of the previous scopes to retain their incarnations; missing scopes receive the current generation on regrant, so coalesced revocations cannot preserve old capabilities.
Session and catalog mutation boundaries use that same borrowed comparison and immediately discard revoked scope incarnations; only a scope transition allocates a bounded checkout set and advances the generation, without a tree reconcile or another notification.
The daemon's gateway registration retains at most four app process identities; each discovery or action checks those bounded identities outside the Runtime mutex.
Discovery and browser actions share eight admission permits; exceeding the cap reports `browser_control_busy` rather than queuing more work.
Discovery does loopback HTTP outside the core owner thread, with no proxy or redirect, an eight-second deadline and a 16 KiB answer cap.
Browser creation reuses Workspace prepare/read/commit and its existing retry-record cap; checkout file reads remain outside the Runtime mutex.
Idle pages add no discovery work, and ordinary terminal input, snapshots and tab selection do not start gateway requests.

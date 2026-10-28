# Testing

This guide answers the question asked while a test is being written: which test to write, how to build its fixture, and how to keep it from becoming flaky.
It owns the layer choice, fixture construction, the writing rules that keep a test deterministic, the order of work for a Playwright e2e test and a Rust test, the flaky policy, and the review checklist for a pull request that adds or changes a test.
Running a check is not here: [VERIFICATION.md](VERIFICATION.md) owns which check proves a claim, the traps before an e2e run, native QA and device checks, and [CONTRIBUTING.md](../CONTRIBUTING.md#ci-gates) lists the gates.
Runtime isolation and its variables live in [PERFORMANCE_TESTING.md](PERFORMANCE_TESTING.md#3-isolate-runtime-state-before-making-fixtures).
Where a test file goes inside a crate or package is in that directory's `AGENTS.md`.

## Choose what to test and where

A test is bought twice: once when it is written and again every time the implementation moves underneath it.
It earns its place when a plausible bug would make it fail and a change that keeps the behavior would not.

- Assert what a caller observes: the returned state, the serialized output, the snapshot the shell receives, the bytes a pane receives, the error a caller can branch on.
  An assertion that a function was called, or on internal call counts, stays green while the feature is broken and breaks when the code is rearranged.
- Take the expected answer from outside the implementation: the requirement or approved PRD, [UI_BEHAVIOR.md](UI_BEHAVIOR.md), [status-model.md](status-model.md), the Herdr references and `contracts/`.
  A test written by reading the code records what the code does, defects included.
- Exercise private helpers through the owning module's stable behavior whenever that boundary can express the case; a helper exported only so a test can reach it means the test is on the wrong boundary.
- Cover the unavailable, malformed and refused states alongside the successful one when the type can produce them, and the first value on each side of a rule's boundary.
- A test double stands in only for a boundary this repository does not own and implements the same trait or interface as production.
  Integration tests and e2e specs do not fake Herdr: they run the pinned binary as a private server, and a test that needs to control Herdr's timing does it at the socket (see [The test decides the order](#the-test-decides-the-order)).
  A core or runtime test that decides what Herdr answers uses `FakeHerdr` (`herdr-core/src/fake_herdr.rs`), which checks every answer against the pinned schema.
- Change an expectation or fixture only when the requirement changed or the test misrepresented an unchanged requirement, and say which.
  Copying the new implementation's result into the expectation is not a reason.
- Before trusting a new test, predict which assertion fails for a realistic planted defect (a moved boundary, a dropped condition, a failure handled as success), make the change, and confirm the prediction; a test that stays green is thin.

Pick the cheapest layer that can observe the result.

| Layer | Use it for | Where |
| --- | --- | --- |
| Unit | A decision that takes values and returns a result: policy, parsing, serialization, layout math, a rejected transition | Beside the owning module: `#[cfg(test)] mod tests` in Rust, `*.test.ts` or `*.test.tsx` in `web/` and `desktop/` |
| Core runtime and crate boundary | State the core owns, reached through a dispatched event and the snapshot it publishes; a Herdr fixture; the hided wire; an OS contract | `herdr-core/src/runtime/tests/` and the other suites the core's modules include, `hided/tests/it/`, `hide-platform/tests/it/` |
| End to end | A flow that crosses a process boundary a user depends on: browser to hided to the core to Herdr to the PTY, or the desktop window and its native integration | `web/e2e/`, `desktop/e2e/` |

An e2e spec is for a flow that crosses a boundary, not for a rule a unit or core test can state.
The external crate-boundary lane in `hided/tests/it/remote_delivery.rs` runs candidate CLI binaries, two private pinned Herdr servers and a loopback SSH server.
It verifies real helper attestation and mailbox intake over the device's node link, the two-second disconnected hook boundary, and reconnect without duplicate delivery.
Its explicit ignore marks the external prerequisites; the `remote mailbox` job in `pr.yml` (a Linux runner, planned for a change to a crate it builds and tests) builds those binaries, fetches the pinned Herdr and runs this lane with `--run-ignored only`.
The same job runs `hided/tests/it/held_letter_notice.rs`'s ignored test, which needs the pinned Herdr and a real daemon but no SSH: a seeded ledger holds one undelivered letter, and the log row for its operator notice must name the skipped phone push and Herdr's own answer.
It also runs `hided/tests/it/remote_core.rs`, a screen machine's node-role hided attached over the same loopback SSH to a core hided on another "machine", each with its own private pinned Herdr and state folder: the attach, the held screens across a lost link, the build check, the checkout callers, the pane size rule, local file reads and that nothing outlives a node that ends.
And it runs `hided/tests/it/core_move.rs`, which moves such a core to the other machine and back from the same window over the same loopback SSH: the window's state before and after, every check missing on the target named while nothing changes, each failure row rolled back with never two cores, a retry of the same move and a driver killed partway.
Its terminal journey types into a pane of the window's machine before the move, after it and after the move back, and counts that pane's attach children: a pane another client of its Herdr still holds is listed, linked and drawn, and refuses every key as read-only, so the journeys that checked only what the window listed and that the link was up passed while the core that had left still held every pane of its machine.
A journey across a change of role or owner types into a pane whose owner changed; a listed pane says nothing about its keys.
Its update journeys start the window from a copy of the CLI whose `hided` differs by one byte past the program's end (another build) and present an older or newer release through `HIDE_BUILD_VERSION_OVERRIDE`, read only under a fixture HOME; a build that fails at its start for the way back is named in the target's fixture HOME (`FIXTURE_FAIL_BUILD_FILE`), a file because the fixture's SSH sessions keep the environment they started with.
One test there needs none of that and runs in the ordinary `hided` lane: a daemon in the test's process on a copy whose handover is pending dials no device and runs no `tailscale` until the move's link commits it.
The target machine's core is started as a detached process, never through launchd: the fixture's HOME carries the fixture marker and sets `HIDE_CORE_STARTER=fixture`, which no other HOME can select; the login item's launchd behaviour is the `os contract (macOS)` leg's to prove on its own runner.
Its target starts holding the fixture's build in its helper root, placed by the product's own `HelperPackages::install_here`, as a device that connected before holds it: sending that build (stripped copies of `hided` and `hide`, written and read back) took 18 to 29 s of the 30 s readiness bound on GitHub's Linux runner, against about 1 s for the rest of the connection, and failed tests in the first parallel wave.
The connection still checks the helper's bytes before starting it, a test's other build is still sent, and the device install itself is `remote_delivery`'s and `device_bootstrap`'s to prove.
When the target helper does not become ready, the fixture reports its last host state, and the remote mailbox workflow retains `.log` and `Logs/*.jsonl` files from its short `/tmp/hcm-*` directories on failure.
That artifact excludes the fixture's SSH keys, saved state and staged binaries; the older `rd-*` artifact does not cover core-move fixtures.
A pull request runs it on Linux only: SSH, the helper's attestation and the mailbox are the same code on every system, and what differs by system beneath them is the `os-contract` lane's and `windows check`'s to prove.
The fixture's macOS branches (codesign of the staged binaries, BSD `ps` and `strip`, the SFTP server path) and the usual remote device being a Mac are why the nightly runs the lane on macOS too (`remote mailbox (macOS)`).
The fixture owns every process tree and SSH channel job, bounds retained jobs and reads, and keeps account configuration and run evidence in a private ignored run directory, which it removes once the journey passed and every process confirmed its exit and keeps after a failure for the lane to upload.
A fixture is started only through `Fixture::journey`, which takes the journey as a closure and ends the run by how it went (`finish` in `hided/tests/it/support/run.rs`, held by its own unit tests), so a test never removes or keeps the directory itself; a new fixture that keeps a run directory under `agents/runs/` implements `Run` and starts the same way.
Each spec starts its own Herdr, hided and browser, so a rule restated end to end costs runner minutes on every pull request and fails for reasons that have nothing to do with the rule.
Keep one representative journey per user-visible flow; when a long spec carries an independent contract, split that contract into a small spec that still runs against the real pinned Herdr and hided rather than adding steps to the journey.

## Wait for state, not time

- Never wait a fixed time: no `page.waitForTimeout`, `sleep`, or `setTimeout` used as a delay in a new test.
  The e2e lint refuses `waitForTimeout`; the sleeps a spec may use are the named helpers in `web/e2e/wait.ts` (see [Writing a Playwright e2e test](#writing-a-playwright-e2e-test)).
  It does not see a hand-written `new Promise((resolve) => setTimeout(resolve, ms))`, which is the same mistake and is not allowed.
  Clippy refuses `std::thread::sleep` in Rust (see [Writing a Rust test](#writing-a-rust-test)).
  Wait for the state the next step needs: a diagnostic `kind` with its subject ID, a snapshot field, a Herdr answer, the bytes in a pane's input log.
- Wait for the readiness the next step actually depends on.
  A healthy HTTP answer is not a subscribed socket, a delivered first snapshot, or a terminal that accepts input.
- A poll only observes (`hide-e2e/no-action-in-poll` in the e2e lint).
  A click, focus, creation or resend is one explicit action outside the poll; an action repeated inside `toPass` or `expect.poll` turns one intent into several and can satisfy the assertion with the wrong one.
- Prefer subject IDs, request identity and generation or revision numbers over wall-clock comparisons and the last diagnostic string.
- A barrier on what the page sent is not a barrier on what the daemon accepted.
  After a burst of clicks, `terminal_click` frames counted at the page say the page sent them; hided may still hold a dozen on its socket, and the last `pane.focus` diagnostic the page has seen can be an earlier request's confirmation.
  Count the diagnostics the daemon sent back against the changes the page sent, with a cumulative count (`observeDiagnostics(...).added` in `web/e2e/pane-focus-ordering.spec.ts`): the list is capped and drops from its front, so the number of entries of one kind in it falls while a burst appends.
  Playwright's `framereceived` fires before the page's handler runs, so before reading what the page shows, poll `window.__hideProbe.arrivals()` up to the frames Playwright has seen.
- A test's own waiting must not compete with a deadline the product enforces.
  While the test holds a request the product will time out, run nothing slow between holding and releasing it, and give every observation poll in that window explicit `intervals`.
  Playwright's default poll backoff grows to a second between attempts, which is time taken from the product's deadline.
  A process start is slow in that sense: `a_slow_ask_for_one_home_does_not_hold_another_home_behind_it` in `hide-platform/tests/it/programs.rs` started the second home's login shell while it held the first home's ten-second ask, and at load 32 the first ask reached its deadline (issue 859); it now reads the second home's answer before the hold, so the held window starts no process, and counts the shell's starts to show it.

`web/e2e/pane-focus-ordering.spec.ts`'s rapid-click test showed the cost.
It holds the first focus request, whose transport deadline is five seconds, while it sends forty clicks.
Between holding and releasing, it detached a CDP session (about 2.1 seconds on a loaded macOS runner) and waited out an `expect.poll` default backoff (about 0.9 seconds), so the held request was released only at about 4.4 seconds and the test failed intermittently on macOS CI.
The cause was found by timestamping each stage against the first request, not by rerunning it.
A second cost was the page's own rendering: headless Chromium composites in software, so every frame of xterm's WebGL canvas is read back synchronously on the page's main thread.
A local trace of the held window showed long tasks of 250 to 500 ms, most of them in `GLES2::ReadPixels`, which delayed both the clicks and the snapshot frames the test observes.
On the failing macOS run, the stage timings and the hided log showed the first diagnostic taking 2.5 seconds to reach the page and the forty clicks another 2.3 seconds, so the held request hit its deadline before the test could release it.
`web/playwright.config.ts` now launches every web e2e Chromium with `--disable-webgl`, so the shell uses xterm's DOM renderer, its existing fallback, and the held window stays near 0.2 seconds; [Writing a Playwright e2e test](#writing-a-playwright-e2e-test) step 6 says how a spec that tests WebGL opts back in.
Terminal pixels in a web e2e screenshot come from the DOM renderer, so a claim about how the terminal looks comes from the desktop app.

## The test decides the order

When the behavior depends on the order of two events, the test fixes that order; it does not hope the scheduler produces it.

- Hold one side at a real boundary and release it after the competing event has been observed.
  `herdrGate` in `web/e2e/herdr-gate.ts` is the shared gate, used by `web/e2e/pane-focus-ordering.spec.ts`: a proxy on the private Herdr socket that holds one request of the method the test arms, forwards everything else, and lets the test release it, so every answer still comes from the pinned Herdr.
- The page's own side of an order rule holds the daemon's frames instead: `holdSnapshots` in `web/e2e/stale-snapshot-focus.spec.ts` routes the WebSocket (`page.routeWebSocket`), buffers what the daemon sends, and hands the page the frames the test chooses, so which snapshot the page has seen when the operator's last click is already sent is the test's decision.
  A frame that changes nothing on the screen gives no sign it arrived, so `read` waits until the page's probe (`arrivals`, with `?probe=1`) has counted every frame handed over before the test reads the result.
  Fence the page's effects with two animation frames before asserting that something did not move; a check that passes on the first poll proves nothing about an effect that has not run yet.
- A rule the core decides is gated one layer lower, in `herdr-core/src/runtime/tests/control_order.rs`: `FakeHerdr` records what actually left over the socket, and the test hands the runtime each answer itself (`complete_lane_tab`, `complete_lane_pane_focus`), so late, replaced, refused and lost answers arrive in the order the test chooses.
  Put an order rule there; an e2e spec keeps one representative journey that shows the pieces are connected.
- A test that hands the delivery store its passes itself starts the store alone (`Worker::store`), not with the doorbell and watch producers (`Worker::spawn`), which pass on their own clocks.
  The store still passes on its own while a registration move or end is asked for, so the test looks at what an event left under that event's own lock, and stops the store before it compares the ledger file with the published ledger.
  `SleepingChild` in `herdr-core/src/runtime/tests/agent_sleep.rs` ran the producers, and in more than fifty recorded Linux CI runs a pass of theirs moved the woken pane's registration between the wake and the test's look; an order such a pass can make is a test of its own there (`a_pass_begun_before_the_wake_keeps_the_record_and_the_next_one_moves_the_registration`).
- Before releasing, assert the barrier the race needs, such as the number of accepted requests, so a stale snapshot or an earlier diagnostic cannot satisfy it.
- After releasing, assert the outcome and the boundary's own record (`gate.requests`, `gate.maximum()`), which show the order the product actually saw.
- A race the test cannot order proves nothing either way; find the boundary to gate before writing the assertion.

## Writing a fixture

- Set every fixture's `HOME` and Hide state folder to private paths before starting a candidate.
- A restart fixture that requires completed native labels waits for the matching session, goal and end in the private `labels.json` before stopping the daemon; a rendered label alone does not prove its asynchronous save completed, and Windows fixture shutdown is forced.
- The same boundary applies to Workspace views: `web/e2e/right-lane.spec.ts` waits for the expected tool, pin and active document in the private `workspace-views.json` before each restart, then checks the restored screen.
  A rendered pin or selected document alone does not prove the save has landed; a missing save fails the persistence wait rather than racing the forced shutdown.
- A retirement test injects its service-control boundary and uses private legacy folders and ledger fixtures.
  A real launchd call to the account's default retired-service label reaches the operator's service regardless of a private `HOME`, so it is never part of fixture teardown.
- The private SSH mailbox fixture stages candidate executable copies without debug symbols, as shipped binaries are, and ad-hoc signs those copies on macOS.
  The original build output, setup deadlines, real helper upload and mailbox assertions remain unchanged; terminal helper refusal reports its state message instead of waiting out the readiness deadline.
  Its setup WebSocket keeps each event connection open through the following Ping/Pong exchange, so a concurrent snapshot cannot lose an unread event when the client disconnects; daemon error frames fail setup at that event.
  Without the wait the lane lost `register_device` on Linux about two runs in three: the client closed with the daemon's snapshot frames unread, the kernel reset the connection, and the unread event vanished with no refusal anywhere (`status.last_error` null, no `device.registered`); issues 766 and 770.
- Copy the whole isolation environment from `web/e2e/herdr-fixture.ts` and `desktop/e2e/fixture.ts`, never a subset; [PERFORMANCE_TESTING.md](PERFORMANCE_TESTING.md#3-isolate-runtime-state-before-making-fixtures) lists every variable and why.
- Offline kit fixtures clear an inherited XDG config override in an owned subprocess, preserving both their private HOME registry and parallel test isolation.
- The Linux and Windows package smoke places state beneath its private HOME, matching the shipped default and preserving the retirement preflight's HOME authority.
  An external state override still requires the full namespace ownership and permission checks.
- A fixture that starts the packaged daemon also sets the host's `HIDE_CLI_PATH` to that bundle's `hide`, because an unbundled debug CLI correctly refuses a different release build.
  Keep the installation and idempotence assertions unchanged; a mixed-build fixture never reaches them through the window.
- Register every process a fixture starts with `ownUntilWorkerExit` from `web/e2e/worker-owned.ts`, so synchronous cleanup runs on Node-managed worker exit even when a test's `finally` was skipped.
  A `spawn` with no `error` listener is such a death: when `target/debug/hided` was missing, each test killed its worker and left its private Herdr server running under launchd.
  The exit callback cannot run after SIGKILL, an OOM kill or host loss; these require separate recovery and are not proven by a `process.exit()` regression.
  A fixture `hided` has that recovery on Unix: `spawnDaemon` in `web/e2e/hided-fixture.ts` hands it an owner channel, so it ends with its worker however the worker ends.
- `desktop/e2e/fixture.ts` owns each `isolate` home through both automatic test teardown and worker exit, with at most sixteen unclosed homes per worker.
  Each home records at most sixteen live or pending candidate launches; a launch over that cap fails before starting another process.
  Automatic teardown closes candidate apps, then each home's cleanup checks its recorded process handles for confirmed exit before stopping the private hided and deleting the home.
  An unconfirmed candidate exit, stop or cleanup failure fails teardown and retains the home for recovery; the error names the retained path and recovery action.
  When candidate exit is unconfirmed, close only the recorded owned candidate, confirm its exit, then call that fixture's `cleanup()` again.
  After a confirmed exit, cleanup reads the host log: a `host.uncaught` line the test did not expect with `allowHostUncaught` fails the test with its stack once the home is deleted, so an exception the host caught nothing for names itself instead of passing unseen (issue 675).
  `desktop/e2e/fixture-cleanup.unit.ts` injects Electron close failures at the external boundary and checks real home retention, confirmed-exit deletion and recovery without starting native processes.
  On Windows `endWindowsProcesses` also ends what the owned processes started, found from their pid and start time after they are gone: an owner killed while it starts a child leaves that child out of the tree listed before the kill, and a child hided started suspended and had not yet put in its job stays suspended, keeping its executable locked.
  `desktop/e2e/windows-processes.unit.ts` checks that against real processes in the Windows check lane.
- Set `terminal.default_shell` in the private Herdr config, because Windows Herdr does not select its shell from `SHELL`.
  The fixture uses `/bin/zsh` with its private `.zshrc` on Unix and the native `ComSpec` cmd shell with a controlled `PROMPT` on Windows.
  A missing native shell fails fixture setup before starting the server.
  Before each initial agent start, the fixture waits for the prompt and for the shell to be available to `agent start`, decided by the one check below (`shellAvailability`), within the existing ten-second setup bound.
  Every `agent start` the fixture sends (setup's and every `fixture.run`'s) goes through `startAgentAtShell`, which sends it only while `shellAvailability` says the pinned Herdr counts the shell as available (`docs/ARCHITECTURE.md`, Starting an agent), as the product's `agent_start` does.
  Setup starts its two agents concurrently through bounded, worker-owned CLI children or socket requests; it observes both results before returning or cleaning up, so one failed start cannot leave its sibling using a removed server or folder.
  The default `startHerdr()` returns only after both CLI starts report interactive-ready success, and `fixture.run` keeps its synchronous start contract.
  A consumer that needs registered names, pane topology, transcripts, settings or files, but sends no input to the initial panes, uses `startHerdr({ readiness: "registered" })`.
  That form uses the pinned socket's `agent.start` registration acknowledgement, not the CLI's later interactive wait; it makes no promise that the agent accepts input yet.
  Before any initial-pane input, including a lifecycle screen or browser keyboard/mouse input, such a consumer must `await fixture.ready()` or keep the default interactive setup.
  `ready()` reads each registered identity with `agent.get` until `interactive_ready`, no pending launch and idle/done state agree, within a thirty-second startup wait; blocked, exited or replaced agents fail the wait.
  Concurrent `ready()` callers share one wait, success is remembered, and stopping the fixture cancels pending socket readers and polling before server/root cleanup.
  Each request owns one connection, a maximum 64 KiB response and a deadline; at most two requests run together.
  `setFixtureLifecycle` waits before writing an initial pane's screen; its later working/blocked states do not repeat startup readiness.
  `fixture.run` refuses initial-pane input before `ready()`, while direct CLI and browser input consumers retain responsibility for that barrier.
  Direct initial-terminal-input, Herdr restart and desktop consumers keep interactive setup; eligible web settings, topology, transcript and file-view consumers opt into registration explicitly.
  `web/e2e/herdr-fixture.spec.ts` holds each CLI start until both arrived, forwards to the real pinned Herdr, and checks first input and cleanup after a partial failure on every platform.
  Its registered-mode journey holds the actual native shim launch, confirms Herdr reports a pending, non-interactive agent, and observes another `agent.get` round before release and first input.
  Other journeys hold a readiness read during teardown and an outgoing sibling start after the real server refuses an invalid kind, proving cancellation and ordered partial-failure cleanup without a fixed sleep or substituted answer.
  Before server/root teardown on Node-managed worker exit, the fixture synchronously ends and confirms pending CLI trees: private process groups on Unix, captured identities and descendants on Windows.
  Unconfirmed CLI exit preserves the root and reports the failure; SIGKILL/OOM skips the exit hook and is outside this guarantee.
  `web/e2e/fixture-worker-exit.spec.ts` exits a separate owner during actual pending starts and observes process exit before server/root teardown, including root preservation when termination is refused.
  On macOS and Linux that is while `pane process-info` says the shell alone holds the terminal; on Windows it is while no process names the shell as its parent, which `pane process-info` does not show, so the fixture lists those processes with the compiled `hide-children.exe`.
  A refusal as `agent_pane_busy` typed nothing, so the fixture goes back to waiting within the same ten-second bound; any other answer is the start's.
  A pane's shell that never gets there fails with the last process info and, on Windows, the children the shell still has.
  On Windows `globalSetup` ends every `vctip.exe` after the run's last compile and prints the pids it ended (`endVctip`): MSVC's compiler and linker leave that telemetry helper running, still naming as its parent a linker whose pid Windows can give a pane's shell, and the pinned Herdr counts a shell's children by parent pid alone, so it would refuse that pane as busy for as long as the helper ran.
  A process Windows started at boot can do the same and cannot be ended: `csrss.exe` names as its parent the `smss.exe` that started it and exited, and a pane's shell can be given that pid.
  So on Windows every workspace the fixture opens, `startHerdr`'s and each `workspace create` a spec sends through `fixture.run` (`spawnAgent`'s included), is checked, as soon as Herdr answers the create, for processes that started before a pane's shell yet name it as their parent (`hide-children.exe` compares start times; the name does not matter).
  When there is one, `unclaimedWorkspace` logs a line, opens the workspace again while the claimed shells still hold their pids, and closes the claimed one; a replacement that is claimed too fails with each process's pid, start time and name.
  A pane opened another way (`tab create`, `pane split`) is not replaced: an agent start in it fails at once and names the process, since no wait reaches a start there.
  A fixture's processes on Windows are listed and ended by the compiled `hide-processes.exe` (`windowsProcessTree`, `endWindowsProcesses`), which reads the system's process table and waits for each end on the process's own handle, not through PowerShell and WMI: a teardown that asked WMI once outlasted its 30-second limit (`spawnSync powershell.exe ETIMEDOUT`, #560), and a wait that polls a query waits on the query's speed, not on the process.
  Herdr's `agent start` takes no start time into account (an upstream candidate), so a product pane given such a pid refuses agents the same way.
- Use `fixtureHomeEnv` from `web/e2e/platform-fixture.ts` to move `HOME`, provider config homes and, on Windows, `USERPROFILE`, `APPDATA` and `LOCALAPPDATA` together into the private fixture.
  It also writes `~/.hide/product-analytics-disabled`, so a configured packaged build cannot send fixture activity to PostHog; a fixture that constructs HOME directly must write that disable-only marker before launch, as the package smoke scripts do.
  The fixtures run a compiled `claude` shim instead, which proves the pipeline and not an agent or physical IME.
  On Windows the interactive shim writes its readiness line after console setup; provider/auth replies return before that line, and input logs record only received input.
  The native compiler is `cc` on Unix and `clang.exe` on Windows, and no test runs it: Playwright's `globalSetup` (`web/e2e/global-setup.ts`, used by the web and desktop configs) builds every program in `web/e2e/shims/*.c` that this system runs once into `web/.e2e-shims`, each named with a hash of its source, and a test only copies the finished program (`copyFixtureShim`).
  A compiler failure fails the run at that entry point with the compiler's name and output, a program that was not built fails the test that asked for it, and nothing in a test compiles, times a compiler or cleans one up; a per-test compile is what exceeded a 20-second bound on a cold Windows runner and left the killed compiler's children behind.
  The spawn provider is built once too; its run-specific values (Herdr binary, completion file, command) are read from `provider.config` in `$HIDE_E2E_ROOT`, which every fixture pane is given.
  The private Herdr server, panes and `hided` use `fixturePath`: the shim directory followed by system-tool directories, including `/usr/sbin` for `lsof` on Unix and native Windows and Git directories on Windows.
  `fixtureExecutable` supplies native `.exe` names and `fixtureToolPath` uses the native path delimiter while refusing missing or non-absolute Windows system-root or program-files values.
  Catalog discovery probes every provider, so appending the host's `PATH` also reaches its installed CLIs even when the test selects Claude.
  A spec adding a GitHub or Git shim prepends it to `fixturePath`; explicit `extraEnv.PATH` remains authoritative.
  A node finds `git` and `gh` on the account's search, the login shell's `PATH` before its own (`hide_platform::programs::command`), and on macOS the shell's `path_helper` puts the system's folders first, so a shim for a program a system folder also holds (`/usr/bin/git`) is found only when the login shell is the fixture's too: a Rust fixture removes `SHELL` from the child or names a stand-in shell (`hide-host/tests/it/worktrees.rs`), and an end-to-end daemon, whose panes need the real shell, gets a HOME whose startup files put its fixture `PATH` first, as an account's own installs come first (`fixtureLoginShellPath` in `web/e2e/platform-fixture.ts`, which `spawnDaemon` and each desktop app launch call with the `PATH` they start with).
  A repository fixture declares its initial branch when the GitHub answers name one: `session-work-fixture.ts` creates `main` for its `main` pull requests, so the account's `init.defaultBranch` cannot change PR assignment or automatic session resolution.
  A repository fixture a fake `gh` answers for carries a remote naming that GitHub repository, as a real checkout does, because `hided` answers a checkout whose remotes name none "no GitHub remote" without asking `gh`: `session-work-fixture.ts` sets `origin` to it, and the specs start their repository with `initGithubRepository` in `web/e2e/github-remote.ts`, which adds it as `github` so a local bare `origin` keeps measuring branches.
  A claim about a real agent session needs the real CLI inside an otherwise isolated fixture: a shim on the pane's `PATH` that runs the CLI under the operator's `HOME`, while the server keeps its private one.
- A fixture makes agent labels the way the product does, through the core and not through tokens: `labelAgent` in `web/e2e/herdr-fixture.ts` gives a pane a new Claude session and writes a synthetic transcript (`writeFixtureTranscript`) the core's label worker reads, and the fixture `claude` shim acts as the provider that answers the analysis with the label the spec asked for.
  The same session is read again only when the agent's state changes: a spec that needs a different label either gives the pane a new session, or, for a pane declared as another's child, appends a turn to its own session (`continueFixtureTranscript`) and changes its state, because a relationship holds only for the sessions it was written for.
  A spec drives status through Herdr's own `agent_status` with `setFixtureLifecycle` and `finishFixtureTurn` (which needs `elsewhereTab`, so the turn ends unseen), because nothing else produces working, blocked or done.
  No spec writes a label or status pane token; the lineage tokens `declareParent` writes (`parent_pane`, `child_session`, `parent_session`) are the only ones a fixture reports, so a spawned child is labelled before its parent is declared (`spawnAgent` with a label).
  `desktop/e2e/session-labels.spec.ts` is the reference for the session boundary.
  Recorded native-format blobs keep their historical label marker; `claude-shim.c` fits the controlled provider response to the requested START or END schema, supplying an empty `work` judgment for an older marker that carries none and, for START and END, a summary of the newest request the input lists as owed.
  Work-ledger journeys name their work explicitly through `labelMarker`; the adapter does not replace an existing work judgment.

### Operating-system differences belong to one fixture helper

`hide-platform` owns what differs between systems in the product; the e2e fixtures need the same for their own resources: the endpoint they listen on, how they spell a path the core compares, the programs they copy and run, and how they clean up.
That belongs in one shared fixture helper under `web/e2e/`, which the specs and both fixtures call, not in a `process.platform` branch per spec.
`web/e2e/platform-fixture.ts` owns native home variables, executable names, the controlled tool path and the copy of the built no-op opener used by both fixtures.
Its Windows `launcher.c` reads paths and arguments through the wide-character APIs and decodes the script's UTF-8 interpreter line explicitly, so a fixture never substitutes the account's ANSI code page for the caller's title or body.
`web/e2e/platform-program.unit.ts` runs the real fixture program from a Unicode path and checks arguments, quoting and the child's exit status on every system; the lost-create-response journey also checks the title the fixture created before reconciling it.
Put new operating-system differences in that shared boundary rather than repeating them in a spec.
The Windows fixture boundaries also preserve these requirements:

- An endpoint is a named pipe on Windows: a fixture that would use the socket path `P` on Unix uses `\\.\pipe\P`, because listening on a Unix socket path there fails with `EACCES`.
  `localEndpoint` in `web/e2e/platform-fixture.ts` spells it for both a listener and a client.
- A new fake `gh`, `tailscale` or provider is a Node script made by `fixtureProgram` in `web/e2e/platform-fixture.ts`, never a `#!/bin/sh` file: Windows cannot run a script by name, and its shell is not the Unix one.
  One compiled launcher (`name.exe`, `web/e2e/shims/launcher.c`, built once like the other programs) runs the `name.js` beside it with the interpreter its first line names, so there is one native program and one script per fake.
  A `PATH` that holds the fake is joined with `path.delimiter`, because `:` splits a drive letter on Windows.
- A key the core derives from a path is computed from the wire spelling the core uses (`/` between names, `hide-platform`'s `path`), never from the native spelling.
  `web/e2e/s2.spec.ts` hashes the checkout folder to find its owner workspace; hashed in the Windows spelling, the key named a different workspace than the one the core chose, and the new tab the test expected appeared elsewhere.
- A path a spec puts in a selector or compares with what the page shows is the wire spelling too: `toPage` (`desktop/src/main/wirePath.ts`) turns the native path the spec built into it.
  `[data-explorer-row="C:\...\notes.md"]` never matches the page's `C:/.../notes.md`, and the wait runs to the test timeout.
- A pane's shell is the one `isolatedEnv` configured, so a command for it comes from the fixture, never from POSIX text in a spec: `runInPane` runs a program there and returns its exit status, `printLinesCommand` makes it print lines, and `paneShellTabName` names the tab it titles.
  The family is read from the configured shell (`paneShellOf`), not from `process.platform`; a shell the fixture cannot write for fails with its name.
  cmd.exe has no `$?`, so the status comes from `call echo %^errorlevel%`, which reads it when it runs, and the status file is written last so its content says the program has finished.
- The Windows console delivers Enter as a bare CR, where a Unix tty turns it into LF and echoes CRLF; the fixture shim ends the echoed line itself, so the next typed line is not drawn over the last one.
- A program just copied or just exited can still be locked on Windows, so deleting it can fail with `EBUSY`.
  Confirm the process that ran it has exited before removing it; a deletion retry is not that confirmation.
- A folder a pane's shell started in is locked until Herdr's server and its panes' processes are gone.
  The `hided` fixture's directory holds the home those panes start in, so `Herdr`'s `afterStop` removes it, after those processes, whichever of the two a test stops first.

### Cleanup never hides the first failure

- Keep the original setup or assertion error as the test's failure, and report a cleanup error beside it with the step and resource it concerns.
  On Windows, a failed Herdr fixture start was followed by a cleanup `rmSync` that threw `EBUSY`, and the cleanup error replaced the start failure in the report.
- A cleanup failure with no earlier error still fails the test; a fixture that cannot confirm its own cleanup leaves state the next test inherits.
- Release every process a fixture started on every exit path, including a failed start: register it with `ownUntilWorkerExit` before the first step that can throw.
- Delete a fixture's root only after every process that used it has confirmed exit; when exit cannot be confirmed, keep the root and name it in the error, as `desktop/e2e/fixture.ts` does.
  A signal is not a confirmed exit: a stopped process holds a SIGTERM pending, and one fixture `hided` outlived its worker that way for a day.
  `stopDaemon` ends a fixture `hided` through `hide stop`, which waits for the graceful stop and then ends the daemon's tree, and continues a stopped daemon first.
  `desktop/e2e/fixture.ts`'s `AggregateError` is the reference for reporting several cleanup failures at once.

## Writing a Playwright e2e test

The rules above say what a test may assume; this section is the order of work for a new `web/e2e` or `desktop/e2e` spec, with the file to copy at each step.
Nothing here is new policy.
A piece that another open change is still building is marked as pending with the change that owns it, and the guide is aligned when that change merges.

1. **Start from the fixture, one stack per test.**
   `web/e2e/herdr-fixture.ts` starts a private Herdr and `web/e2e/hided-fixture.ts` a private `hided`; `desktop/e2e/fixture.ts` takes a started Herdr fixture and gives the packaged window its own `HOME` and `hided` state.
   Why: a test that shares a server with another inherits its panes, focus and timers, and its failure cannot be read alone.
   The config runs one worker (`web/playwright.config.ts`) so a timing assertion does not compete with a neighbour for cores; CI deals tests out across runners by their recorded durations (`scripts/e2e-shard.py`, [Balancing the e2e shards](#balancing-the-e2e-shards)).
   A desktop spec sizes its window with `fitWindow` in `desktop/e2e/fixture.ts`, inside the work area of a CI runner's 1024-point screen; `hide-e2e/window-size-through-fixture` refuses `setSize` and `setBounds` anywhere else, and a spec whose subject needs a larger window (`right-lane.spec.ts` asserts lane widths in whole CSS pixels of bodies wider than the screen) says why in a line allow and never orders that window out and in.
   Why: macOS can keep a window larger than the work area until the window is next ordered in, by the test's own `blur()` or `focus()`, and clamp it then, so the layout changes in the middle of the test; `area cycle native` lost both View areas that way in about one run in ten (issue 511).
2. **Wait for a product signal, never for time.**
   The signals a spec waits on, in order of preference:
   - A DOM state the product exports as a data attribute: `data-pane-view` and `data-focused` for pane focus, `data-checkout-kind` for a checkout row, `data-tab`, `data-main-view` for the Overview's tab.
     `expect(panes[0]).toHaveAttribute("data-focused", "true")` in `web/e2e/pane-focus-ordering.spec.ts` is the shape.
   - A diagnostic `kind`, read from the snapshot frames the page receives (`observeDiagnostics` in the same file; `recordWire` in `web/e2e/s7.spec.ts`).
     Wait for `pane.focus.followed`, not for "a moment after the click".
   - What the fixture can read from the real systems: the bytes in a pane's input log, a Herdr answer, a file the core wrote.
   When there is nothing to wait for, `web/e2e/wait.ts` has the only sleeps a spec may use, one per reason:
   - `animationsFinished(page)` for the colour transitions before a capture; it waits for the page's finite animations, not for a time.
   - `quietFor(page, ms, why)` for a window in which something must not happen (nothing is sent, the row does not move); `why` states the claim.
   - `compositorPresents(page)` for a native capture, because the macOS compositor presents a frame after the page reports it; `animationsFinished` before it does not replace it.
   - `unchangedForFrames(page, read)` for "the burst has finished" before a baseline is taken: it waits until `read()` has not changed for thirty animation frames, counted in frames and bounded, not in time.
   - `measureFor(page, ms, why)` for an interval a measurement spans on purpose.
   Waiting for something to appear or settle with one of these is the mistake the lint exists for: wait for the data attribute, the size or the frame count instead.
   If the product has no signal for the readiness the next step needs, add one to the product (a data attribute, a diagnostic) in the same pull request.
   A signal added for a test is still a product contract: it is an attribute or a diagnostic, not a banner ([UI_BEHAVIOR.md](UI_BEHAVIOR.md)).
3. **Fix the order with a gate when two events race.**
   [The test decides the order](#the-test-decides-the-order) owns the rule; `herdrGate` in `web/e2e/herdr-gate.ts` is the shared gate and `web/e2e/pane-focus-ordering.spec.ts` is the reference use.
   It forwards every request to the pinned Herdr, records what arrived (`params`, `maximum`), and holds the next request of the one method `arm` names until the test calls `release`; import it and do not write a second proxy.
4. **Control UI timers with `page.clock`, not with a wait.**
   A timer the page owns (a toast timeout, a debounce, a hover delay) is advanced with `page.clock.install()` and `page.clock.fastForward()`.
   Why: waiting out a 3-second timer costs 3 seconds on every run and still races a slow runner.
   `page.clock` also controls the shell's own timers (reconnect, debounces), so install it only in a test that is about a timer, and advance it by the amount the timer needs.
   No spec uses it yet, so the first one that does becomes the reference and is named here.
   The timers of `hided` and the core are not the page's; those are injected in Rust (see [Writing a Rust test](#writing-a-rust-test)).
5. **Keep the size of a test bounded.**
   One representative journey per user-visible flow, and one small spec per independent contract.
   A long journey that joins several contracts fails whole, so one shaky step hides every contract after it and turns the lane red for all of them; split a test so the part that shakes can be fixed alone.
   The reference splits are `agent-close-contract.spec.ts` (one Herdr contract, two UI contracts), the focus contracts in `pane-focus-ordering.spec.ts` (one held request, keys after a burst, an external focus), the drag contracts in `agent-tab-groups.spec.ts`, the row menus in `sidebar-menus.spec.ts` and the view caps in `s7.spec.ts`: each spec starts from one shared `start...` helper, puts itself into the shape it needs as setup, and asserts one contract.
   Splitting costs a stack start per spec, so say in the pull request what the split bought.
   `web/scripts/check-e2e-test-size.mjs` (run by `lint` in `web` and `desktop`) fails a test over 120 lines or 40 `expect` calls, counted on the `test(...)` call itself.
   The tests that were already over are recorded in `e2e/test-size-baseline.json` as a ceiling that only shrinks: a recorded test that grows fails, and so does an entry whose test is gone or fits the limit, until the entry is removed.
   Moving assertions into a helper lowers the count, not the line span, so the line limit stays; a split that removes an entry is the intended way out.
6. **Leave the renderer off; turn WebGL on only in a spec that tests it.**
   Headless Chromium composites in software, so each frame of xterm's WebGL canvas is read back synchronously on the page's main thread, 250 to 500 ms at a time.
   `web/playwright.config.ts` launches every web e2e Chromium with `--disable-webgl`, so the shell uses xterm's DOM renderer, its existing fallback, and a spec needs no setting of its own; [Wait for state, not time](#wait-for-state-not-time) has the measurement.
   A spec file that tests the WebGL renderer sets `test.use({ launchOptions })` at file top level, which replaces the config's `launchOptions` rather than adding to it, and says why: `sleep-wake-click.spec.ts` does, because the layer that once covered Wake agent is a WebGL canvas the DOM renderer never draws, and `terminal-webgl.spec.ts` gives its WebGL cases a separate fixture pool with `test.extend` beside a case that keeps the suite's flag.
   Assert terminal content through the probe, never through DOM text, because the DOM renderer puts terminal text in the page.
   The desktop suite is not covered: Electron has its own GPU path.
   Do not raise a timeout to cover the stall.
7. **Restart through `daemon.restart()`, and leave the old page first.**
   `daemon.restart()` in `web/e2e/hided-fixture.ts` stops `hided` and starts it again on the same state folder and port, as a real restart does, and takes an optional callback to edit the state folder before the start; `web/e2e/sidebar-pr-start.spec.ts` restarts three times in one test.
   A navigation that only changes the hash does not remount the connection, so the old page keeps its old token and socket and races the new one.
   Go through `about:blank` before opening the new origin and token, as `openSettings` in `web/e2e/interface-language.spec.ts` does, and keep that step in one helper per spec.
   Why: the old page and the new token reconnect at the same time and either can win.
   `hide-e2e/reopen-after-restart-through-blank` fails a `goto` of `#token=` after `restart()` in the same function; a helper defined elsewhere and called after the restart is not seen, so keep the step in the helper.
   A test whose subject is a surviving page reconnecting to the restarted daemon (`web/e2e/s3.spec.ts`, the two draft recovery tests) must not reload it, and says so in a line allow.
8. **Put a system difference in one fixture helper.**
   See [Operating-system differences belong to one fixture helper](#operating-system-differences-belong-to-one-fixture-helper); native home variables, executable names, the tool path and the no-op opener live in `web/e2e/platform-fixture.ts`, not in a `process.platform` branch in a spec.
   The endpoint helper (`herdrGate` spells the Windows pipe itself in `web/e2e/herdr-gate.ts`) is not there yet; add it to `platform-fixture.ts` when a second spec needs it.
9. **A retry is a label, not a fix.**
   CI runs Playwright with `retries: 1` (`web/playwright.config.ts`, `desktop/playwright.config.ts`), so a test that fails and then passes is reported as `flaky` instead of failing the run; two failures still fail the lane.
   That is a classification for the issue `scripts/ci-flaky-report.py` files for the test, with a seven-day expiry to fix or delete it.
   It is never a reason to loosen a wait.
   The rule is in [Flaky tests](#flaky-tests); locally nothing retries.

## Writing a Rust test

Pick the layer first with [the table above](#choose-what-to-test-and-where), then follow the rules below.
The crate's own `AGENTS.md` says where the file goes; this section says how the test is built.
An integration test is a module of its crate's one test binary, `tests/it/main.rs`, never a `tests/*.rs` file of its own: each file was a binary that linked every crate again ([BUILD.md: One integration test binary per crate](BUILD.md#one-integration-test-binary-per-crate)).
Its tests share a process with the other modules' under `cargo test`, so a test that changes process-wide state, such as an environment variable its fake reads, holds the lock the binary's `main.rs` declares for it.
A test whose assertion requires a pipe to have no readers creates and closes that pipe inside its own exact-test subprocess, owned and bounded with `OwnedChild`, so another test's concurrent spawn cannot inherit a reader (`hide-platform/tests/it/stdio.rs`).

1. **Unit test the decision, runtime-test the rule that crosses components.**
   A function that takes values and returns a result is tested beside its module (`usage.rs`'s `can_attempt` and `record_failure`).
   A rule that depends on what Herdr said and in what order is tested through the runtime: dispatch an event, feed a session payload, read the snapshot (`herdr-core/src/runtime/tests/session_navigation.rs`, for example `view_authority_an_external_tab_focus_is_followed_and_reported` and `view_authority_a_refused_tab_focus_keeps_the_tab_and_reports_it`).
   Why: the unit test cannot see a pending request being superseded, and an e2e spec sees it only when the scheduler happens to produce it.
   Native usage reads return one bounded page: an accounting fixture follows `has_more` and the checkpoint to completion (`usage_drain` in `hide-session/tests/it/usage.rs`), while a test of the page budget inspects each step itself.
   A two-line session may yield after its metadata under load, so its first page is not a promise of the whole event set; token and identity expectations apply to the completed read without extending a read's time budget.
2. **Decide Herdr's answers with `FakeHerdr`.**
   `herdr-core/src/fake_herdr.rs` is the one fake Herdr socket: the test supplies a responder by method, the fake checks each answer against the pinned schema, and `methods()`, `calls()` and `wait_for_requests` read what the core asked.
   Why: fifteen hand-written listeners used to differ in a socket detail, and the one that differed failed under load for weeks.
   Use it for a rule the core owns; use the pinned binary as a private server when the claim is about Herdr's own behavior (see [Choose what to test and where](#choose-what-to-test-and-where)).
3. **Let the test order Herdr's events.**
   Feed the session payloads and events in the order the rule needs, and assert what is published after each: the answer arrives before the next dispatch, after it, or never.
   `last_pane_close_waits_for_the_authoritative_fallback_tab_focus` in `herdr-core/src/session_sync/tests.rs` applies `pane_closed`, `workspace_focused` and `tab_focused` one at a time and asserts that nothing is published until the authoritative tab focus arrives.
   A rule about order lives here, where it is deterministic, and the e2e spec keeps one journey that proves the pieces are connected.
4. **Inject the clock; do not wait for it.**
   Code with a deadline, a backoff or an expiry takes the current time as an argument, and the test passes a time it chose: `usage.rs`'s `can_attempt(now)` is tested at `now + 89 s` and `now + 90 s` without sleeping.
   A component that reads the time in many places takes one clock instead: `AiRouter::with_clock` (`hide-ai/src/router.rs`) gives every park, availability cache and per-minute window the same source, the real clock unless a test passes one, and the cooldown tests move a `ManualClock` to 1 ms before and at the reset.
   For async code, `tokio::time::pause()` with `tokio::time::advance()` moves the clock by hand; `herdr-core` does not enable tokio's `test-util` feature today, so enabling it for a crate is part of the pull request that first needs it.
   `herdr-core/src` still calls `Instant::now()` directly in many places, and no module has had its clock injected yet.
   Where a test could not inject one, it orders the events itself instead of waiting a time: the router tests hold the provider on a gate and wait for the `ai.request.joined` log event (`hide-ai/src/router.rs`), the usage test's worker reports when it began and waits to be released (`herdr-core/src/usage.rs`), and the pane-control test makes `FakeHerdr` hold its answer until the spawn has returned (`herdr-core/src/live.rs`).
   New code with a deadline takes the clock from the start.
5. **Never bound a test by a short wall-clock.**
   `clippy.toml` refuses `std::thread::sleep`; the sleeps that remain carry an `#[allow(clippy::disallowed_methods)]` with the reason: a bounded polling helper, a production wait, a sleep that is the subject of the test or keeps a child process alive, or a stand-in for a state that a tracking issue lists.
   A bound such as `assert!(started.elapsed() < Duration::from_millis(1850))` passes on an idle machine and fails on a loaded runner unless the bound is itself the product's deadline (`hide-platform/tests/it/process.rs` checks one); a bound that is only a guess at "fast enough" says nothing about the product.
   Assert the counted result (how many requests, how many attempts, which one won) or observe the event, with a generous deadline that is only a hang guard.
   A `thread::sleep` that stands in for a state is the same mistake in the other direction: the test waits a time chosen by a person, not the state the next line needs.
   A sleep that is the subject of the test, such as a fake peer that answers late, is the exception and says so in a comment.
6. **Poll the state, once, with a named condition.**
   When a test must wait for another thread, use the module's helper that names what it waits for (`wait` and `wait_for` in `herdr-core/src/runtime/tests.rs`, which every runtime test module shares, `wait_for` in `hided/tests/it/support/remote_delivery/mod.rs`) rather than a new loop with a sleep.
   The helper fails with the name of the thing it waited for, so a hang is readable.
7. **Own and remove what the test starts.**
   Put a child process, a thread, a socket or a temp folder behind a value that cleans up on `Drop`, as `FakeHerdr` does: it wakes its accept loop, joins the thread, and re-raises a panic from the responder on the test thread.
   Use a private folder per test and never a fixed name in `/tmp`.
   A private folder comes from `tempfile` and is removed with its owner, never named from the pid: nextest starts each test in a process of its own, so a pid-named path is one an earlier test process may have left state under, and a herdr-core runtime that loaded such a state file started from someone else's selection instead of the defaults its test assumed.
   In `herdr-core` runtime tests that is `scratch_dir`; the runtime keeps the folders made for it, and a test that drops a runtime and restarts on its files takes them first with `hold_dirs`.
   A runtime shared with the workers it starts is a `SharedRuntime`: dropping it takes the runtime back on the test's own thread once every worker has let go.
   A shared runtime is dropped by whichever holder lets go last, and a worker that let go after the test returned lost to the process exit, so the runtime and its folders were never dropped.
   A folder the runtime's workers write into is the runtime's (`test_dirs`), so it goes after them; a folder a test double writes into from a worker goes once the double's calls have ended (`Machine` in `herdr-core/src/runtime/tests/home.rs`), because a save still running into a removed folder makes it again.
   Make what a test needs beside its folder inside it: `strip_checkout` and `workspace.rs`'s `temp_dir` put the checkout one level inside the scratch folder, so a second checkout, a linked worktree or a link made next to it is removed with it, where a sibling of the scratch folder sat in the shared temp folder for good.
   A child a test starts is given an environment built from nothing only when that environment is the test's subject (an SSH exec's, a login item's, a throwaway agent home's): `clippy.toml` refuses `Command::env_clear`, so such a test says why in an `#[allow(clippy::disallowed_methods)]`, and a child that stands for one Hide starts gets its environment through `hide_platform::process::restrict_to_login_environment`, as the product's do ([ARCHITECTURE.md](ARCHITECTURE.md#the-platform-layer)).
   A test that checks that a process it does not hold a handle to has ended names it while it runs, by its pid and `start_time`, and asks later whether that same process still runs (`Running` in `hide-ai/tests/it/omp_sdk.rs`): Windows gives an ended process's pid to the next process once the last handle to it closes, and `is_alive` counts another account's process there, so a pid checked after the fact named an unrelated process (issue 1495).
   Why: a leaked process or file is inherited by the next test and by the next run.
8. **Give a test a program that has already started once.**
   macOS checks a file the first time it is started, once per file, and the checks of every process on the machine wait in one line: on 2026-10-09 at load 7 a new script's first start took about 90 ms, its second 6 ms, and the last of twenty new scripts started together waited 1.8 s; Linux refuses to start a file another thread's fork still holds open for writing (`ETXTBSY`).
   A program a test copies or writes and then runs inside a product deadline therefore pays that line inside the deadline, and under load it outgrew the hook's budget and the kit's 5 s deadlines (issue 813).
   In `hide-platform`, `hide-agent-hooks`, `hide-kit` and `hided` a stand-in comes from `hide-platform/tests/it/stand_ins.rs`: `program(at, body)` puts at `at` a hard link to one read-only file per body beside the build's test binaries, started once per process before the test gets it, and `place` does the same for the helper cargo built.
   On macOS, `place` instead byte-copies a native executable into the fixture, ad-hoc signs that private file, and starts it before the measured deadline; it never signs a hard link or changes the build output.
   A first start killed by a signal fails setup with its exit status instead of being cached as ready; a normal nonzero exit remains valid because some CLIs and interpreters reject `--help`.
   A body is fixed text and finds its test's files from `HOME` (`${HOME%/*}` for the folder around it), never from a path written into it, because each distinct body is one more file kept for the build.
   A program checked into the repository, such as `hide-agent-hooks/tests/fixtures/fake-codex.py`, is a new file in every checkout too, so a test that runs it under a deadline makes it ready first with `stand_ins::ready` (issue 824).
   A helper-install protocol fixture instead materializes a private executable with `std::io::copy` and preserves its permissions, matching the bytes written by the device installer (`hided/tests/it/node_session_activity.rs`).
   `fs::copy` may make an APFS clone whose first execution stalls before `main`, but a byte copy still pays first-execution validation.
   This fixture calls `stand_ins::ready_with_args(&helper, &["node", "--version"])` before its unchanged five-second protocol deadline, using the same file-identity cache and owned 60-second setup guard as `ready`.
   Choose a known one-shot invocation: `hided --help` starts a daemon, whereas `hided node --version` exits without opening daemon state.
   The Unix shell-start fixture (`herdr-core/src/agent_start/real_herdr_tests.rs`) builds its native agent with `cc` and starts that file without arguments before the launch deadline; with arguments it stays alive for Herdr's process detection.
   A system program pays the same on Windows: Windows PowerShell's first start in a CI job, beside the other tests starting theirs, took from 2 s to past a 15 s run's hang guard (issue 1497), so a test that bounds a run through it starts it once first with `ready_with_args` (`agy_shell` in `hide-agent-hooks/tests/it/windows_hook_command.rs`).
   Do not rename a system utility to stand in for an agent: a multicall utility can choose its implementation from the executable name and reject the agent's name.
9. **Retries are a classification.**
   CI runs every Rust lane (Linux, macOS, Windows, the OS contract and nightly) with `scripts/verify-cargo.sh nextest --profile ci` (`retries = 1` in `.config/nextest.toml`), so a test that fails once and then passes is reported as flaky and recorded in an issue with an expiry; two failures fail the lane.
   `nextest` does not run doc tests; the workspace has none that runs today, and a runnable one needs its own `cargo test --doc` step.
   `scripts/install-nextest.sh` installs the pinned release on a runner; a lane that runs nextest several times keeps one JUnit report per run, and `scripts/ci-flaky-report.py` reads them all.
   The pin is at least 0.9.145, because an older nextest on macOS let a test started beside another inherit the other's output pipe, and reported the test whose pipe it held as leaking (LEAK) though it had left nothing running; `.config/nextest.toml` warns a local run on an older nextest, and on macOS with 0.9.145 or later a LEAK names a child the test left behind, which rule 7 forbids.
   The `ci` profile also ends a test still running at 120 s (`slow-timeout`; 240 s for the remote mailbox test), so a hang fails with the test's name, is retried and is filed like any other flaky failure, instead of holding the lane until a step or job limit cancels it with no test named.
   That limit is a hang guard set well above the slowest test each lane measures, and the comment beside it in `.config/nextest.toml` says how it was measured; a test that needs more is the finding, not the limit.
   A flaky OS-contract test is fixed or deleted by its issue's deadline like any other; it is not ignored.
   The rule against raising a deadline to pass is unchanged.

## Which lanes a pull request runs

`pr.yml`'s `plan` job reads the paths a pull request changes against the base it merges into and plans the lanes they need; `scripts/ci-plan.py` is the one place the rules live, and `scripts/tests/test_ci_plan.py` checks them against real paths.
`MACOS_LANE_INPUTS` holds each macOS lane's covered paths together, including the existing crate-only desktop exception; Linux and Windows selection stays with the general classifier.
`remote-mailbox` (the remote mailbox lane, its own Linux job) is planned for a change to any crate it builds and tests (`MAILBOX_CRATES` in the script: `herdr-core`, `hided`, `hide-agent-hooks`, `hide-host`, and so every crate they depend on); web code and web specs do not reach it.
`windows-wsl` (the [WSL device lane](#wsl-device-lane)) is planned for a change to a crate the WSL link crosses (`WSL_CRATES`: `hide-platform`, `hide-node`, `hide-node-link`, `hide-host`), to the node command or to its test, and not for a crate that only consumes them: the core's decisions about a WSL device are unit tests in `herdr-core/src/runtime/tests/wsl_devices.rs`.
Every lane is a job whose `if:` asks the plan, and `verify` passes only when every planned lane succeeded and every other lane was skipped.
A planned lane that was skipped, failed or cancelled fails `verify`, so a wrong `if:` shows up as a red check rather than a lane quietly left out.

| Change | Lanes |
| --- | --- |
| Only `docs/`, root Markdown, or an `AGENTS.md`/`CLAUDE.md` below the root | `policy` (the script suite and the repository invariants) |
| `design/` | `policy`; `design-contract.yml` checks the library |
| A file a lane outside its folder reads (`READERS` in the script): the root `AGENTS.md`, `web/src` and `hided/src`, which herdr-core's tests read; `desktop/src/main/wirePath.ts`, which web e2e specs import; `design/tokens.json` and `docs/UI_BEHAVIOR.md`, which web tests read; `docs/AI_PROVIDERS.md` and the agent logo manifest, which hide-agent-adapter's tests read; the contracts a test holds the code to (`hided-ws.schema.json`, `snapshot-wire-enums.json`, `agent-adapters.json`, `hide-cli.json`, `usage-coverage.json`), which the tests of `herdr-core`, `hided` or `hide-agent-adapter` read, and web unit tests for `snapshot-wire-enums.json` and the shell itself for `agent-adapters.json` | also that lane, and `rust` over `herdr-core` for the first three and over the crates whose tests read a guide, the manifest or a contract; a contract, which no other row claims, plans only these and `policy` |
| Registered desktop Vitest unit tests: `desktop/src/**/*.test.ts`, `desktop/e2e/fixture-cleanup.unit.ts` and `desktop/e2e/windows-processes.unit.ts` | `policy`, `checks` and `windows-check`, without application e2e or package jobs for these test files |
| The web shell: `web/src`, `web/index.html` | `checks` (the web shell's and the desktop app's typecheck, lint and unit suites in one job), the Linux `web-e2e` and the macOS `desktop-e2e`, which drives the same shell inside the app |
| The phone page: `web/mobile.html`, `web/public/m` | `checks` and `web-e2e`; hided serves it at `/m/`, and the app never loads it |
| Web code the desktop host imports or drives through native input (the host bridge, the shortcut registry, keys and keyboard, store, snapshot and socket, terminals, focus and area cycling, `App.tsx`, `main.tsx`; `SHARED_WEB` in the script) | also `windows-e2e` and `windows-desktop` |
| A `web/e2e` spec | `checks` and `web-e2e`; a spec tagged `@platform` also runs `windows-e2e` (the whole suite runs on Linux, so the Linux leg of `@platform` is `web-e2e`) |
| `desktop/src` outside registered unit tests, `desktop/static`, a `desktop/e2e` spec | `checks` and `desktop-e2e`, the only macOS lane (two shard jobs) a change outside the two crates and the package inputs below asks for, and `windows-desktop` (the desktop `@platform` tests on Windows) for `desktop/src`, `desktop/static` and a spec tagged `@platform`; `desktop/src/main` also runs `windows-check`, where the main process's unit suite runs on Windows |
| A Rust crate | `rust` over the crate and every crate that depends on it (from `cargo metadata`), `windows-check`, which compiles every crate for Windows, the Linux `web-e2e`, since every crate reaches `hided` and the web suite drives it, and `remote-mailbox` when the crate reaches `herdr-core`, `hided`, `hide-agent-hooks` or `hide-host`; not the macOS `desktop-e2e`, which main's push run and the nightly run for it ([Where the macOS runners went](#where-the-macos-runners-went)) |
| `herdr-core`, `hided`, `hide-platform`, `hide-herdr-client`, `hide-host`, `hide-kit`, `hide-agent-hooks` | also `os-contract` (its Linux and Windows legs), `windows-e2e` and `windows-desktop`; no macOS lane but `package` for the package inputs among them |
| `hide-platform`, `hide-herdr-client` | also `os-contract-macos`, the OS contract's macOS leg, which tests exactly these two crates |
| `hide-agent-adapter`, `hide-agent-hooks`, `hide-ai`, `hide-kit`, `hide-session` | also `agy-contract`, agy's tests and the real agy on Windows (`AGY_CRATES` in the script); the nightly runs it on all three systems |
| `hide-ai` or `hide-platform` | also `omp-contract`, the real OMP SDK against synthetic authentication and localhost inference on all three systems; no paid model or operator login |
| What goes into a package (`PACKAGE_PATHS` in the script): `desktop/scripts`, `desktop/package.json`, `desktop/resources`, the Herdr pin and its fetch scripts, `verify-cargo.sh`, `verify-web.sh`, `toolchain-env.sh`, `hided/build.rs`, which embeds the web shell, `hided/src/cli.rs`, `hide-kit` and `hide-agent-hooks` | also `package`: `package.yml`'s Windows and Linux packages, the two Linux device helper builds, and the macOS archive with the packaged app's own specs |
| The paths `POLICY_ONLY` names, which no lane reads: `agents/`, `site/`, `tools/`, `spikes/`, `.gitignore` files, the root `LICENSE` and `NOTICE`, the PR template and `dependabot.yml`, the contribution intake (the issue and discussion forms and `APPROVED_CONTRIBUTORS`), `scripts/tests/`, the policy `check-*` scripts, the design, release and measurement scripts, the pre-merge check and the merge order with the helpers they share, the design contract's generators, token map and checks (`gen-tokens.mjs`, `gen-pen.mjs`, `gen-screens.mjs`, `pen-token-map.json`, `check-design-contract.mjs` and the checks it runs, which `design-contract.yml` runs on every pull request), and Markdown below a folder no rule claims | `policy` alone |
| The paths `NAMED_LANES` names, whose readers are a known set: a `web/e2e` file that is not a spec (the `desktop` suites on macOS and Windows import it, and `desktop/e2e` unit tests run in `windows-check`), a `desktop/e2e` file that is not a spec, the Playwright, eslint and vitest configurations, `web/vite.config.ts` (every lane that builds the shell or runs its unit suite), `web/scripts` and `desktop/scripts` | the lanes that read it, listed in the script and its test; known packaging helpers run `checks` and `package`, without unrelated desktop, OMP or OS contracts |
| Any `.github/workflows/` or `.github/actions/` change, or `scripts/ci-plan.py` | every lane, including `package` and `agent-clis`; a CI definition must exercise all its decisions |
| `scripts/` the lanes call (`verify-*.sh`, `ci-flaky-report.py`, ...), the Herdr pin and schema and any contract `READERS` does not name, any `package.json`, lockfile, the workspace `Cargo.toml`, a type change, and any path no row above names | every lane but `package`, which only the package inputs add |

The macOS choices below are data in `MACOS_LANE_INPUTS`, rather than additional job conditions.
Registered desktop unit tests keep `checks` and `windows-check`; they do not select a macOS lane even when their names overlap an updater or package glob.
Markdown under web/desktop e2e is policy-only in both planning and green-run reuse.
Added, modified and deleted paths use the same coverage rules; unknown inputs and unsafe comparisons still broaden the plan.

| macOS jobs | PR inputs and coverage |
| --- | --- |
| `desktop e2e (macos 1/2)` and `2/2` | One `desktop-e2e` decision for both shards: desktop host/static files, desktop specs/helpers/configuration, the host build script, web shell source/assets/index/Vite configuration, shared web e2e helpers/shims, shipped fuse files, and the adapter contract imported by the shell. Phone assets and standalone web specs do not reach the app. Rust-only changes retain the accepted main/nightly coverage described below. |
| `compile (macos)` and `package (macos)` | One `package` dependency chain: the existing package scripts/resources/native inputs, updater and packaged specs, Herdr pin/fetch, build entrypoints, daemon embedding/replacement, kit and hooks. Known package helpers such as signing, installers, identity and release-binary validation request only `checks` and `package`; fuses also keep both desktop suites because their global setup applies those fuses, and an unknown helper still takes the broad fallback. |
| `omp SDK (macos)` | `hide-ai` and `hide-platform` in their entirety, conservatively including their manifests/tests; the same `omp-contract` decision retains Linux and Windows. |
| `os contract (macos)` | `hide-platform` and `hide-herdr-client`, the two crates it tests against the pinned runtime. |
| Web e2e on macOS | No PR job: the existing nightly macOS `@platform` job stays unchanged. |

Main pushes keep their existing full plan without `package` or `agent-clis`; nightly owns packaging and its direct agent checks, and release candidates keep their current independent workflow.
The broader all-lane rule for CI definition edits applies to PR path selection, not a new main packaging job.
A new PR push (`synchronize`) cancels the older run in the same concurrency group; `edited` does not cancel the run whose green evidence it may reuse.

Every plan includes `policy`, whatever else it names, except a draft pull request's: it plans no lane, and `verify` fails with "draft: lanes not run, mark ready for review".
A body edit (`edited`) starts a run too, for the checks that read the body (`check-pr-screenshots.py`); it waits for a run in progress rather than cancelling it, and with the last green run standing it plans `policy` alone.
Marking the pull request ready (`ready_for_review`) starts the run that plans and runs the lanes, and that run's `verify` replaces the failed one.
`verify` fails on a draft instead of being skipped because a skipped required check counts as passed, and `verify` is not started until its lanes finish: a skipped `verify` from the draft run would otherwise be the only check on the commit for the minutes after it is marked ready.
Keep `ready_for_review` in `pr.yml`'s `types`, and keep `verify` running on a draft; `scripts/tests/test_ci_plan.py` reads the workflow for both.
A hand run of `nightly.yml` takes a `lane` (`all`, `linux`, `macos`, `windows`, `agy`): one system's web and desktop suites, the remote mailbox lane and the agy contract, with `verify` and `package` for `all` only, or for `agy` the agent contracts alone on all three systems, agy's and the `agent trust contract` of the real Claude Code and Codex, which a branch can run with `gh workflow run nightly.yml --ref <branch> -f lane=agy`; the schedule runs everything.
The scheduled nightly and a hand run of `all` call each web and desktop suite once: `verify` owns the whole Linux web suite, Windows's `@platform` web and desktop tests and the whole macOS desktop suite; the outer workflow adds macOS's `@platform` web tests and Linux's `@platform` desktop tests.
An OS-only hand run keeps its direct calls, with no `verify` call and no other system's web or desktop suite; `agy` starts neither suite.
The Windows desktop call keeps its screenshots through `verify`, as its direct hand run does, and every call keeps the same failure and flaky reports and logs.
The existing script suite checks the schedule, `all` and all four selective dispatches against the suites, filters, shard counts and screenshot settings this policy requires.
It also runs the performance lane on macOS (`performance (macOS)`, `perf.yml`), for the schedule and a hand run of `all` or `macos`; [PERFORMANCE_TESTING.md](PERFORMANCE_TESTING.md#the-nightly-performance-lane) owns its fixture and what fails it.
Tag a `desktop/e2e` test `{ tag: "@platform" }`, with a comment saying what differs, when it checks what the desktop host does differently on another system: its processes and local stream (including whether closing the last window ends the app), the CLI's name and places, the path it hands the wire, the file manager's reveal, the accelerators it registers for the system (one test reads them through `web/e2e/chords.ts`).
Choose by that difference, not by whether the test passes on Linux or Windows; a tagged test that fails there is fixed (its fixture or the product), never skipped, retried on a deadline or marked `@flaky`.
A test for a macOS-only behavior, or one that checks no OS difference, carries no tag and runs on macOS only; `CONTRIBUTING.md` lists what the tag covers.
A push to main plans every lane but `package` and `agent-clis`, and so does a plan that cannot be computed: a missing base, a checkout that is not the merge commit, a diff that does not parse, or a crate graph `cargo metadata` cannot read.
Main's full run is the net under a pull request that left out a lane it needed; main's runs queue rather than cancel each other.
Nightly calls `verify` with `nightly-agent-checks: true`: its direct agy and journey calls own all three systems, including the Linux stand-ins, Windows forms and the separate fake-model journey.
The planner leaves `agy-contract` and `agent-journey` out of that call only, and the verify aggregate requires both internal jobs to be skipped; PR and main push plans still run them as before.
The direct calls remain dependencies of the always-run nightly report job, whose first step requires every selected job to succeed and every unselected job to skip, including selective dispatches.
A failed or cancelled agy or journey call, or a selected call unexpectedly skipped, fails that aggregate and nightly.
The issue-reporting step still runs after an aggregate failure on main only, so branches file no issues and a main failure still opens a `nightly-failure` issue.
Nightly calls `package.yml` on its own, and a lane that breaks with no merge is found within a day.
When one does, fix the rule in `scripts/ci-plan.py` with a case in its test; a test that reads a file outside its own folder adds that file to `READERS`.
A path is narrower than every lane only by being named in `MACOS_LANE_INPUTS`, `POLICY_ONLY`, `NAMED_LANES`, `UNIT_TEST_PATHS` or `READERS`, with its reader in a comment and a case in `NamedPaths`; a new or unknown path plans every lane until someone names it.
`UNIT_TEST_PATHS` follows the desktop Vitest include list, not a general filename heuristic.
Desktop unit tests are outside the explicit entry graphs in `desktop/scripts/build.mjs`; both retained jobs run the unit suites, including Windows.
Linux `checks` runs both packages' typecheck and lint once; `windows check` retains both Vitest suites and every Windows Rust contract without repeating those OS-independent checks.
Web tests keep their existing app lanes: `herdr-core/src/sidebar.rs` scans every TypeScript file under `web/src`, including tests, and Tailwind scans test files under the Vite web root as CSS inputs.
Unregistered test conventions keep their folder's existing lanes, and a mixed change keeps the union of every changed file's lanes.
Registered desktop unit tests are not package inputs even when a broad input pattern such as `updater*` names them.
`NamedPaths` compares the narrowed files with the desktop Vitest configuration and checks that both retained jobs execute both unit suites.
`Cargo.lock` stays among them: every member but `hide-platform` and `hide-herdr-client` depends on `workspace-hack`, which holds nearly every third-party crate, so a lock change reaches every crate and no narrower plan can be read from it.

A pull request whose last green run still stands reuses it: the plan names `policy` alone, and its summary names the run it reuses.
A push that merges main to resolve a conflict in a guide, or that only edits documentation, is the case it is for.
The run stands on two conditions.
What the pull request adds to main as it is now differs from what that run's head added only in paths no lane but `policy` reads (documentation, `design/`, the `POLICY_ONLY` paths); the plan merges that run's head into the current main with `git merge-tree` and compares the result with this run's merge, so a conflict resolved differently in code, or any change to code, a guide a test reads or a contract, fails it.
And main has changed none of the paths the pull request changes that a lane reads since the main that run checked: when pull request #1133 merged main, main had changed six files it changed too since its green run, four of them in `herdr-core`, and no run had checked those changes together.
`policy` still runs, because its script suite and invariants read the documentation the push changed.
A follow-up that differs only in registered desktop unit tests and policy-only paths also reuses the unchanged app checks, but reruns the unit-test delta through its normal lanes.
The desktop assertion change runs `policy`, `checks` and `windows-check`.
Any product code, shared configuration, unregistered test convention or guide read by another test changing since the green anchor keeps the whole PR's original plan.
The same main-overlap, missing-record and failed-comparison safeguards apply.
Web unit edits retain the whole PR plan because their files are also app inputs.
A partial run retains the original app-tested anchor in its plan record; it never claims skipped app lanes checked its newer head.
Each pull request run keeps a plan record, the `ci-plan` artifact, naming the run whose lanes ran and the main and head they checked; a run that reused one records that one, so the comparison is always with lanes that ran.
The plan job reads the newest green run of the pull request's branch and head repository (`actions: read`) and fetches the commits and trees it needs (`--filter=blob:none`, about 9 MB for main's whole history).
No green run, no record, a record for another head, a failed lookup, download or fetch, or a comparison that fails leaves the plan the change itself needs, and the summary says why; no failure turns into a reuse.
This applies the merge policy's reasoning to the run itself ([CONTRIBUTING.md](../CONTRIBUTING.md#merging-into-main)): a green pull request merges without catching up with main, and `premerge-check.py` runs on the merge result before it merges, reused or not.
`ReuseGreenRun` and `LastGreenRun` in `scripts/tests/test_ci_plan.py` check the conditions, the fallbacks and the record.
### Where the macOS runners went

The organization runs 20 jobs at once and five of them on macOS, so a pull request asks for a macOS runner only for what one proves:

| macOS job | Runs in a pull request | Otherwise guarded by |
| --- | --- | --- |
| `desktop e2e` (the Electron app, two shards) | a change to the app's own code: `desktop/`, the web shell, the `web/e2e` helpers, and a full plan | A crate change: main's push run, which plans every lane, and the nightly; nothing else it skips is code the app runs (a `web/e2e` spec, the phone page, documentation) |
| `os contract (macOS)` | a change to `hide-platform` or `hide-herdr-client`, and a full plan | The nightly's `verify` call, which plans every lane and so runs all three legs |
| `compile (macos)` and `package (macos)` (the packaged app) | a change to what goes into a package (`PACKAGE_PATHS`), not a full plan without one; a cold native release build in `compile (macos)`, beside the two Linux device helper builds described in [BUILD.md](BUILD.md#device-helper-release-bundles), then the packaged app's specs in `package (macos)` once both are done, one macOS runner at a time | The nightly's `package.yml` call on main, which found issue #412 a day after its cause merged; no other job starts the packaged app (its signed bundle, host, daemon and Herdr) |
| the web `@platform` tests on macOS | never (no lane) | The same tests run in every pull request on Linux (`web-e2e` runs the whole suite) and on Windows (`windows-e2e`); the nightly runs them on macOS in `web e2e (macOS full)` |
| the remote mailbox lane on macOS | never | The nightly's `remote mailbox (macOS)` job; a pull request runs the lane on Linux, and the Mac-only parts of the fixture (codesign of the staged binaries, the system's SFTP server, the macOS Herdr asset) are checked there |
| the performance lane | never | The nightly's `performance (macOS)` job; a pull request's performance claim is still a matched local comparison ([PERFORMANCE_TESTING.md](PERFORMANCE_TESTING.md#the-nightly-performance-lane)) |

A change to `herdr-core` or `hided` alone, outside `hided/build.rs` and `hided/src/cli.rs`, therefore starts no macOS lane.
The remote mailbox lane includes `hided/tests/it/device_bootstrap.rs`: it installs the real helper and pin into a private SSH account with no Herdr server, checks terminal startup after the kit, and removes the helper and SSH link before asking the surviving server for its session.
Its second journey checks that a compatible running server connects its terminal services before the automatic kit pass.
Both journeys confirm their private server and SSH exits before passing, including the server bootstrap detached from its helper.
Its Herdr handoff tests also move a device server on the older Herdr 0.8.2 protocol with its pane still running.
What only macOS shows for such a change (the desktop suite, the OS contract's macOS leg, the web `@platform` tests and the remote mailbox lane on macOS) is found by main's push run for the desktop suite and by the next nightly for the rest, which opens a `nightly-failure` issue when it fails; the cost is that delay.
Until issue #1288 a web shell or crate change left `desktop e2e` out, and a desktop spec that asserted what the change did failed first on main's full run: #1106 (the session record, `session-work-ledger.spec.ts`) and #1147 (the file tree a lane close keeps, `right-lane.spec.ts`) kept main red 92 and 103 minutes on 2026-10-15.
Such macOS failures still stop unrelated merges under [the main failure policy](../CONTRIBUTING.md#when-mains-push-run-fails); its development exception is limited to existing Windows-only failures.
A web shell change keeps `desktop e2e`, which is what would have caught #1106.
A crate change gives it up: on 2026-10-27, across 43 runs, a macOS job waited a median 13.1 minutes for a runner (90th percentile 40.3), and crate-only pull requests, 37% of pull request runs, took a median 40 minutes, against about 20 without the macOS jobs.
The accepted risk is #1147's: a crate-only change that breaks a desktop spec turns main red until its push run fails and someone fixes it, with every merge waiting under the main failure policy.
Of 400 failed runs from 2026-10-05 to 2026-10-27, the 33 that failed only in the macOS desktop suite all also changed `web/src`, `desktop/` or a shared path, which still plan it; none changed crates alone.
Main's push run is the safety net for it.
The desktop `@platform` tests run on Windows in a pull request (`windows-desktop`, about six minutes, on a Windows runner rather than a macOS one) and on Linux only in the nightly.

### WSL device lane

`hided/tests/it/wsl_device.rs` connects a real WSL 2 distribution of GitHub's Windows runner as a device, through the product's own `wsl.exe` connector, with this build's Linux helper, command and hooks and the pinned Linux Herdr.
The runner runs WSL 2 itself, not WSL 1 (WSL 3.0.3, a 6.18 WSL 2 kernel, systemd as process 1, measured on `windows-latest` on 2026-10-28), so the lane proves the WSL 2 path and no WSL 1 path exists to prove.
`wsl-device.yml` builds the Linux programs on a Linux runner, because the Windows runner cannot, and hands them to the Windows job as an artifact; `setup-wsl` makes a fresh `Ubuntu-24.04` with `git` and its first-run setup undone.
The test owns that distribution, which nothing else on the runner uses: it reads the listing without starting anything, is refused while the first-run setup is undone, adds a user and makes it the default as the setup would, then installs, reads a Git project by its Linux path, types into a pane once, ends the link and checks the helper ended inside WSL while Herdr stayed, reconnects with nothing sent again, kills a second process of its own that holds a link (`a_link_owner_that_is_killed`, which runs only when the main test starts it, so the lane names the main test alone) and checks that helper ended too while the same Herdr process stayed, as a hided that crashed would leave it, is refused for another default user, and stops the distribution to check that an attempt without leave leaves it stopped and one with leave starts it.
Stopping a distribution and changing its default user are the test's on a runner only; on a person's PC Hide never does either.
The step prints the WSL version and the install's counts it saw.
`hide-node/src/ssh/wsl_tests.rs` holds the same connector's rules on every Unix runner with a fake `wsl.exe` that logs each call: what a stopped distribution's attempt may run, the probe, the identity check, the resumed install and a child link's end, including one whose write is stuck on a child that stopped reading; `hide-platform/src/wsl.rs` checks that a `wsl.exe` that exits nonzero is never read as nothing running.

### OMP SDK boundary

`hide-ai/tests/it/omp_sdk.rs` runs against each supported official npm SDK on Bun 1.3.14.
Only the vendor catalog is substituted to direct the actual auth and inference libraries to localhost; no Hide module is mocked.
The fixture owns the synthetic home and server, and checks model listing without inference, full prompt ownership, tool/context isolation, typed refusals, invalid JSON/schema, no hidden retries, unknown usage, cancellation/deadline and child/request-file cleanup.
Run locally with `HIDE_TEST_BUN_BIN=<absolute Bun> HIDE_TEST_OMP_PACKAGE=<absolute npm package directory> bash scripts/verify-cargo.sh test-scoped -p hide-ai --test it omp_sdk -- --ignored --nocapture`.
The test is ignored only because it requires an external SDK/runtime; the dedicated required CI lane installs both exact versions and runs it on every supported system.
A passing fixture does not prove paid-service authentication or standalone support.

### How many jobs a run starts

A run that plans every lane a pull request can plan starts at most 35 jobs, twelve more than the 2026-10-05 shape of run `37324202934` before the macOS and mailbox work: that work saved one, the e2e suites outgrew their shards' step caps, which took a second macOS desktop shard and two more web shards ([Balancing the e2e shards](#balancing-the-e2e-shards)), the agent journey's Linux stand-in job and Windows forms job added two, and the `agent CLIs` installs on Linux and Windows, which only a change to `contracts/agent-clis.json` or an installer plans, added two, building the shared web shell in parallel with the daemon adds one, and the WSL device lane's Linux build and Windows run add two; the refreshed twenty-run median requires six web shards.
`scripts/tests/test_ci_plan.py` counts them from the workflows:

| Jobs | Count | Why |
| --- | --- | --- |
| `plan`, `policy`, `rust`, `checks`, `windows check`, `verify` | 6 | `checks` merges `web checks` and `desktop checks`: both install the same dependencies, run for a minute or two and start a runner each |
| `windows e2e` | 1 | the reusable workflow no longer starts a `plan` job that only fanned the shard numbers out; a matrix picks its list by index |
| `windows desktop` | 1 | the desktop `@platform` tests on Windows, the one job that starts the Electron app there in a pull request; it keeps the window it drew as the run's artifact |
| `web e2e` (Linux) | 8 | parallel daemon and web shell builds, with the daemon job also downloading the zsh packages, and six shards dealt by recorded durations: six were cut to four in 2026-10-05 after queues at the 20-job limit, returned to six as the suite grew, and six now meet the 720-second budget using the refreshed twenty-run median ([Balancing the e2e shards](#balancing-the-e2e-shards)) |
| `os contract` (Linux, Windows), `os contract (macOS)` | 3 | one job per system, the macOS leg planned only for its two crates |
| `agy contract` (Windows) | 1 | agy's hook block, trust, conversations and Hide AI backend, and the real agy reading Hide's block, where the hook commands and install folder differ; `rust` runs the same tests on Linux and the nightly runs all three systems |
| `omp SDK contract` (Linux, macOS, Windows) | 3 | both supported exact npm SDK versions on Bun 1.3.14, with synthetic stored login and localhost inference; the native SQLite, Bun runtime and package resolution need each system |
| `agent journey` (Linux, Windows) | 2 | the journey on the compiled Claude and Codex stand-ins on Linux and its Windows forms (process jobs, private files, pane commands) on Windows, planned only for the journey's code, its contract and the stand-ins; the nightly and a release run every system with the fake model lane ([VERIFICATION.md](VERIFICATION.md#agent-journey)) |
| `agent CLIs` (Linux, Windows) | 2 | every agent CLI `contracts/agent-clis.json` pins, installed at its pin and failing on another version, planned only for the table, its reader and the installers, so a pin moves in the pull request that installs it; the nightly's journey installs them on macOS too |
| `windows wsl` (Linux, Windows) | 2 | a real WSL 2 distribution on the Windows runner connected as a device through `wsl.exe`, with this build's Linux programs, which only a Linux job can build, planned only for the crates the WSL link crosses (`hide-platform`, `hide-node`, `hide-node-link`, `hide-host`) and the node command ([WSL device lane](#wsl-device-lane)) |
| `remote mailbox` | 1 | the lane that was the tail of the macOS `@platform` job, now on Linux and, in the nightly, on macOS |
| `desktop e2e` | 2 | the only macOS lane of the desktop app's own changes, in two shards that build in place, because the whole suite outgrew one job's step cap ([Balancing the e2e shards](#balancing-the-e2e-shards)) |
| `package` (Windows, Linux, macOS) | 3 | `package.yml`'s jobs, planned only for what goes into a package; before the lane its Windows and Linux jobs ran on the same paths outside `verify` |

The common runs are smaller: a `herdr-core` or `hided` change starts 18 jobs with none on macOS, a web shell change 15 with two on macOS, a desktop-only change 7 with two on macOS (8 for `desktop/src/main`, which adds `windows check`), a `hide-platform` change 24 with two on macOS, a `hide-kit` or `hide-agent-hooks` change 22 with one on macOS (`package`), and a documentation change 3, as does a push that reuses the pull request's last green run.
The median wall time and the share of runs whose Linux shards waited more than 10 minutes come from a measurement of the pull request runs after a change merges, taken the way issue #561 took its baseline; they are not in this guide because a pull request cannot prove them.
The `plan` job's summary lists each lane with the paths that chose it.

### Balancing the e2e shards

The multi-shard web lane builds the daemon and web shell once in independent parallel build jobs, then each shard waits for both and downloads the same run's binaries and `web-dist-<lane>` into the paths its fixtures read.
The shell build uses the same checkout, Node, pnpm, frozen lockfile, command and environment as the single-shard build; [BUILD.md](BUILD.md#two-entrypoints) owns the artifact and failure contract.
Each shard still installs its test dependencies and runs its unchanged discovery, selection, execution and reporting steps.
A single shard builds in place, with no additional job or artifact transfer.
The separate shell build adds one job against the organization's 20-job limit; lane measurements include both build queues and setup before the final shard completes.

Playwright's `--shard` cuts the tests in file order into runs of equal count, and a test takes from 1 second to a minute, so with four shards one web shard job took 11 minutes and another 6 (run `37402687007`, 2026-10-06).
It also runs a project's dependencies whole in every shard that holds a dependent test, which in the desktop suite is every background test (below).
A shard of a lane with more than one shard therefore runs the tests `scripts/e2e-shard.py split` deals it: each listed test (`playwright test --list`) weighs its seconds in the suite's own table (`--table`), a test the table lacks weighs the table's median, and the longest test goes first into the shard with the least work so far.
The same list and table always give the same shards, and every listed test lands in exactly one shard.
The shard runs them with `--test-list`, so Playwright's retry, the flaky report and `--grep` work as before; the script reads and writes UTF-8, because a title holds `›` and a Windows runner's default encoding cannot write it.

Playwright counts a `--test-list` that matches nothing as a pass, so the step does not trust the file: `e2e-shard.py check` fails it unless `playwright test --list --test-list <shard file>` lists exactly the tests `split` planned, and a list, a table or a plan it cannot read is one line and a failed step.
A `grep` that leaves fewer tests than shards leaves the extra shards no test; those steps end at once, which is what the old `--shard` did.
A plan that finds tests the table lacks, or a heaviest shard more than 15 % over the mean, prints a `::warning::` on the first shard's step and still runs: the table is a recording, and refreshing it fixes the warning.
`scripts/tests/test_e2e_shard.py` checks the split, the check, the encoding, the budget and both tables' shape.

A shard's step keeps its 20-minute timeout as a cap on a stalled run, never as the size of its work.
A lane sizes its shards to plan at most 60 % of that cap, and `split --budget <seconds>` turns a plan whose heaviest shard holds more into a `::warning::` on the first shard's step: the suite has outgrown its shards, and the answer is another shard in the caller's `shards:`, not a longer timeout.
Both lanes pass 720 seconds.
A macOS shard costs one of the five macOS runners the organization runs at once and about 4.5 minutes of its own setup (checkout, the caches, the cargo build of the CLI binaries and the web build), so the macOS lanes add the fewest shards that keep that margin.

The current web table is the median per listed test from the Linux web e2e jobs of twenty successful main `verify` runs, collected on 2026-10-28.
The runs range from [40285086262](https://github.com/with-hide/hide/actions/runs/40285086262) (2026-10-26) through [40612941430](https://github.com/with-hide/hide/actions/runs/40612941430) (2026-10-28), including the audit baseline [40569996553](https://github.com/with-hide/hide/actions/runs/40569996553).
All 385 currently listed tests have a passing observation: 379 have twenty runs, one has nineteen, three have nine and two have six because they were added later.
The existing `durations` command takes the median of passing lines, including a passing retry, and rounds to 0.1 second; no test in this refresh uses the unknown-test fallback.
The table totals 3851.3 seconds; the unchanged splitter plans at most 962.9 seconds with four shards, 770.3 with five and 641.9 with six.
Six is the smallest count within the unchanged 720-second test-work budget; the e2e step still caps a stalled run at 1200 seconds.
These weights include fixture setup and teardown charged to a test, but exclude report overhead, runner setup and queues; medians are neither a wall-time forecast nor p95 or causal speedup evidence.
This refresh replaces the approved single-run recording from 2026-10-25, whose five-shard plan had drifted below actual test time, without changing assertions, retries, timeouts or test selection.
Only the Linux web lane deals by this table: the nightly's macOS and Windows web lanes run one shard each.

The desktop table, `desktop/e2e/shard-durations.json`, is the median over 20 runs of each test on the macOS runner (2026-10-17), and 0 for a test those runs only skipped (the packaged-app, device and SSH tests, which that lane skips); it deals the whole macOS suite in two shards, in a pull request (`pr.yml`) and in the nightly, while the Linux and Windows `@platform` legs run one shard each.
One job ran the suite in 1048 to 1166 seconds of its 1200-second step on main's last seven runs before it was split, and pull request #1306's run was ended at the cap with every test it reached passing; the table puts the suite at 1008 seconds and plans the two shards at 503 and 504.
A desktop shard runs with `--no-deps`.
The config runs the `needs-focus` project after `background`, because a test that brings its app to the front leaves macOS activating the next app that opens (`desktop/playwright.config.ts`), and Playwright runs a dependency project whole, never filtered by `--test-list`, `--grep` or `--shard`: without `--no-deps` the shard file for 57 tests, two of them `@needs-focus`, listed 100.
`check` refuses that shard, since Playwright lists the background tests its file left out.
With `--no-deps` the order is still the config's: the one worker runs the projects in the order the config lists them, and a failed test's retry right after it, so each shard runs its background tests, retries included, before its focus tests.
What `--no-deps` gives up is the skip: a background test that fails its retry too no longer skips the shard's focus tests, which still run and report, while the lane fails as before.

A table is its system's and suite's: per-test costs differ by system (symlinks, process and terminal tests), so a lane on another system that runs several shards records its own.
Refresh a table when a shard's time drifts from the others in a run, when the plan warns, after tests were added or renamed in bulk.
Refresh it this way:

1. Download the logs of the lane's jobs (the Linux `web e2e` shards, the macOS `desktop e2e` shards) of about twenty recent successful runs (`gh api repos/<owner>/<repo>/actions/jobs/<job id>/logs`) into a directory outside the repository.
2. `bash scripts/verify-web.sh <web|desktop> e2e --list --reporter=list > list.txt`, then `python3 scripts/e2e-shard.py durations --list list.txt <logs>... > <web|desktop>/e2e/shard-durations.json`; it keeps only the listed tests, takes a passing retry as its test, records 0 for a test the logs only skipped, and names the listed tests no log has a passing or skipped line for (they keep the median).
   `durations` reads the `ok` mark of a Windows runner and the `✓` of the others, and the project a desktop run prints before each test.
3. Commit the table.

### No real login on CI

No CI lane signs a real agent in: an agent CLI on CI runs as a compiled stand-in or as the real CLI answered by the journey's fake model, with no key ([VERIFICATION.md: The CI lanes](VERIFICATION.md#the-ci-lanes)).
Operator decision, 2026-10-27: what only a login proves (the provider's sign-in, quota and billed usage) is the provider's, not Hide's code, and a signed-in lane cost keys and a daily quota rotation and failed for the provider's reasons.
A real login is measured by the journey's local mode on the operator's Mac, where the agents are already signed in ([VERIFICATION.md: Local mode](VERIFICATION.md#local-mode-macos)).
A test that needs a real agent asserts what happens before the model answers (the hooks the agent loads, the letter it is handed, a call refused with Hide's reason) against the fake model or a stand-in, never what a provider writes.

## Flaky tests

A test is flaky when it both fails and passes on the same SHA.
A failure on a different SHA is not evidence of flakiness by itself, and the same title can carry two different failures.

A flaky test never blocks a pull request's required gate or a merge, and it does not stay: it is isolated as an issue rather than moved out of its lane, and its cause is removed by that issue's deadline.
CI retries a failed test once, for classification and for the report, and for nothing else.

Every issue CI files lives in the private repository with-hide/hide-private: the flaky `quarantine` issues, the `nightly-failure` issues, the `agent-canary` record and its failures, and the Herdr preview notice.
The public repository with-hide/hide takes only user bug reports, through the bug form.
`ISSUE_REPOSITORY` in `scripts/ci-flaky-report.py` and `scripts/agent-canary-record.py` names the private repository; the runs, jobs, logs, artifacts and pull requests they read stay the public repository's.
Each job that files or reads these issues mints a token from the development issues GitHub App (its Client ID in `vars.HIDE_ISSUES_APP_CLIENT_ID`, and `secrets.HIDE_ISSUES_APP_KEY` in both the Actions and the Dependabot secret stores) with `actions/create-github-app-token`, for both repositories, and that one token reads the run here and writes the issue there.
A reusable workflow receives the key from each caller by name.
A lane whose run has no key, such as a fork's, files nothing and still passes.

- Playwright runs with `retries: 1` when `CI` is set (`web/playwright.config.ts`, `desktop/playwright.config.ts`), and the Rust workspace runs under `cargo nextest` with the `ci` profile's `retries = 1` (`.config/nextest.toml`, `scripts/verify-cargo.sh nextest`).
  Neither retries locally: a flaky test fails where it is written.
- A test that fails and passes its retry is flaky: the lane passes, the log marks it, and `scripts/ci-flaky-report.py` reads the tool's own report (Playwright's JSON, nextest's JUnit) and files it as an issue labelled `quarantine`, or comments on the open issue that already tracks it.
  Issue and comment lookup read every page, using the nightly report's existing bound of ten pages of 100 entries; an incomplete listing or a GitHub error stops filing and leaves the warning and unfiled-test summary below, rather than creating an issue or comment from a partial search.
  The issue names the first run, the change and the system, and carries a deadline seven days out.
  The report step also sets its `flaky` output, and a lane that keeps its e2e logs for a failure keeps them for a flaky run too, because the first attempt's daemon, Herdr and input logs are the evidence its issue needs.
  The reusable web and desktop lanes suffix their log and screenshot artifact names with the workflow attempt, so a rerun preserves the first failure instead of replacing it or colliding with its upload.
  On Windows CI the web suite also logs, for each failed attempt and at that moment, what holds TCP connections (count by state and by process name, `[windows sockets]` lines from `web/e2e/windows-sockets-reporter.ts`), because a Chromium `ERR_NO_BUFFER_SPACE` on a loopback connect left nothing saying who held the sockets; a passing attempt runs nothing.
  A wait that gives up on a race between Herdr and the page (the agent tab count in `checkout-owner.spec.ts` and `tab-strip-fit.spec.ts`, the dropped file's paste in `s3.spec.ts`) records both sides at that moment through `dumpOnFailure` in `web/e2e/failure-dump.ts`: a `[failure dump]` line in the log and an attachment in the attempt's `test-results` folder. It rethrows the wait's own error unchanged, so it explains a flake and never absorbs one.
  The desktop suite keeps, for each failed attempt, the end of each private daemon's `Logs/core.jsonl` (at most 256 KiB, whole records) as `hided-<n>.jsonl` in that attempt's `test-results` folder, which the same artifact carries, with the run's folders, the repository, the home and the temporary folder written as placeholders, because the host log alone could not show the order of the core's focus records behind a flaky ⌃Tab cycle (issue 629); a passing attempt copies nothing.
  A test that fails its retry too fails the lane; nothing else is retried anywhere.
  The report step writes it on the job as an error annotation titled `Failed test` (its file and line, and its name on the first line of the message), which the run's summary page shows and the nightly report files (below).
- A job rerun can reveal a flake that neither job's retry classified: a test fails both tries in attempt 1 (`unexpected`), then passes its first try in attempt 2 (`expected`).
  The same reporter's `--history-record` mode preserves each reusable web/desktop job's original Playwright JSON and provenance for seven days, including successful jobs, under `ci-test-history-<suite>-<lane>-<shard>-attempt-<n>`.
  The actual tested commit comes from `git rev-parse HEAD` in that checkout; neither the PR head nor the jobs API's `head_sha` substitutes for a tested merge commit.
  The existing final `verify` job calls `--retry-history`, with the development issues App's token, even after a planned lane failed; it reads only this workflow run's attempts and files through the same `quarantine` issue, deadline, matching and comment contract.
  The nightly's full `verify` call uses this path too; OS-only nightly calls retain their reports but have no final `verify` collector and do not link reruns.
  A link requires equal checkout SHA, OS, calling workflow ref/SHA, suite, test file, complete describe/title path and Playwright project ID/name, plus one unambiguous completed job with the recorded runner, run and attempt.
  Failure evidence requires explicit failed/timed-out results on both tries, each with a located standard `expect(...)` matcher diagnostic in both `error` and every entry of `errors`, followed by an error-free explicit first-try pass in a successful test job.
  Playwright 1.63 JSON does not retain a general matcher type or assertion steps: the collector recognizes its rendered `expect(...).matcher(...)` message after removing terminal controls, with a source file and positive line number.
  Generic exceptions, worker exits, plain test timeouts, mixed assertion/infrastructure errors, report-level errors and custom assertion messages without that format are unsupported or ambiguous and remain `Not measured`; status and retry count alone never prove an assertion failed.
  This is a limited diagnostic format contract, not a classifier for every failure or a guarantee about custom errors that imitate that format; existing within-job flaky reporting is unchanged.
  Skips, expected failures, interrupted or unsupported results, infrastructure failures and missing reports never prove a test passed or flaked; the summary identifies missing or unsupported evidence as `Not measured`.
  The issue keeps the original first failure, the tested SHA, project, job URLs and workflow-attempt URLs, and records each original failure only once across repeated collection and later reruns.
  It reports the original job's runner time and the newly executed passing job's additional runner time when their timestamps measure those intervals; total added run cost remains `not measured`.
  A jobs endpoint can return earlier successful jobs under a new ID and `run_attempt`; a start before the attempt's `run_started_at` is a copied job, never a newly executed pass or added runner cost.
  Runner seconds are not PR wall latency or billed cost, and missing duration is `not measured`.
  History is capped at five workflow attempts, eighty history artifacts, sixteen MiB per archive and expanded contents, ten thousand tests per report and ten pages of one hundred entries per API listing.
  The lane reporter reads its local Playwright JSON once, with the same sixteen MiB cap before parsing, and shares those bytes with history preservation; an oversized input stops reporting with a warning, summary and nonzero exit instead of being loaded again.
  Artifact downloads use the existing `gh` credentials, need gh 2.97 or newer, stop at the byte cap and a 45-second deadline, and reap the child process on every exit.
  Unsupported provenance, ambiguity, API refusal or a resource cap stops history filing with a warning and summary rather than guessing from partial evidence; a history notice cannot turn a failed lane green or block a passing lane.
  Local reporter regressions use fixture reports and mocked APIs, never live test issues or comments.
- At the deadline a flaky test is fixed or deleted.
  Whoever knows the cause opens the fix or the deletion; if nobody does, the issue goes to the operator.
  The deadline is not a timer that moves the test somewhere quieter, and a later flake on an issue past its deadline says so in its comment.
- A green lane is not proof that a flaky test passed on that commit: its first attempt failed.
  Read the `Report flaky and failed tests` step and the `quarantine` issues before claiming a flow verified.
- The report step cannot fail a lane: the run already passed.
  If it cannot file (a GitHub error, a run without the App's key), it leaves the unfiled tests in a warning annotation and in the job summary under "Flaky tests that were not filed", and the next flaky run files them; a filed run lists its issues in the same summary.
- GitHub keeps 10 error annotations per step, so one report step runs the script once and annotates at most 8 failed tests; past that it writes one more annotation, `Failed tests not annotated`, whose message is the count it left out.
- `cargo nextest` runs no doc test.
  The workspace has none that runs (its three doc blocks are `ignore`, `text` and `sh`); a runnable doc test needs its own `cargo test --doc` step.
- `verify-cargo.sh test` and `test-scoped` stay `cargo test` for local runs and the sealed harness; they never retry.

What the retry does not do:

- Record each failure in the issue that tracks the test: the run, job and attempt link, the tested SHA, the system, the failing assertion and its message.
  Group failures by assertion signature, not by title, so two causes under one name are not read as one.
- A flaky test is not a product-defect verdict.
  A failure that shows lost or misrouted input, a broken OS contract, or a missing tab the user asked for is a product defect candidate, and is investigated as one.
- Do not make a flaky test pass by raising a timeout or deadline, adding retries beyond the CI one, changing the number of clicks or keys, lowering an expected count, resending an action, accepting a partial string, or skipping it.
  Each of these hides the order or readiness problem the failure was reporting; fix the wait or the gate, or fix the product.

A flaky test is isolated by its issue, never by leaving a required lane:

- There is no quarantine tag and no quarantine step: every web and desktop e2e test runs in the lanes its plan picks, and a flaky one is retried, filed and fixed or deleted like any other.
- Rust has no quarantine either: a flaky Rust test is retried, filed and fixed or deleted, and the policy is zero `ignore`.
  An `ignore` that names an external binary, such as `real_herdr` and `remote_delivery`, is an opt-in run with its own step and says what it needs; it is not a quarantine.

### Nightly failures

The nightly's `report` job (`scripts/ci-flaky-report.py --nightly`) is the one writer of `nightly-failure` issues, so the run's cap, the comments and the closing never race between lanes; a lane only writes its annotations.

- A failure is one issue: a test a lane annotated as `Failed test`, a check the performance lane annotated as `Failed measurement`, or, for a failed job that annotated none (its report step died, wrote nothing or never ran), the lane and its failed step.
  The lane is the job's name without its shard, so a test is the same failure in whichever shard ran it; `verify`'s gate, which fails whenever a lane it waits for did, is not a lane.
  A step that only prepares the runner (checking out, installing, fetching Herdr, a lost runner) adds the label `infra`; any other step, a build included, does not.
- The issue names the test, measurement or step, the lane and its runner, an excerpt of the error (run through the same cleaning as a flaky issue's), the run and its commit, the lane's last green nightly within 14 runs, the pull requests merged since then that touched the failing test's file (for a step, the lane's workflows and the scripts they call; for a measurement, every one of them, since a measurement moves with any change) as candidates, and an expiry two days out.
  It is ended by fixing the cause or reverting the candidate; a longer timeout, more retries or a skip does not end it.
- The same failure again (a hidden `nightly-failure:<lane>:<test or step>` marker) is one comment per run attempt on the open issue, never a second issue.
  Windows web/build and Windows/macOS desktop calls that moved under `verify` resolve their former lane names to the same suite, so existing issues and green history remain connected; OS-only hand runs keep their direct names and use the same mapping.
- A scheduled run on main, or a hand run of `all`, that passes a lane closes that lane's issues with a comment naming the commit and the run; a skipped or cancelled lane closes nothing.
- One run opens at most 10 issues; the rest, with the count of failed tests a lane had no annotation room for, go into one issue, `nightly: N more failures in run <id>`, which the next scheduled run closes as it files its own.
- A run of any other branch writes no issue.
- The report fails its job when GitHub does, and when it cannot mint the App's token, so a failure it could not file shows as a red report job rather than a silence.
  `--dry-run` with `--runs-repo` and `--run` prints what it would write for any finished run and writes nothing; `--issues-repo` reads another issue repository than with-hide/hide-private.
- The report reads a failed job's log through `gh api`, which from gh 2.97 hands over a body carrying a terminal escape only when told `--allow-escape-sequences`, and every job log carries one (the runner colors each step's command), so the report and its `--dry-run` need gh 2.97 or later.
  An older gh does not know that flag, so the report reads `gh --version` before the first log and stops with one line naming the minimum, the version it found and the fix (`upgrade gh`) instead of gh's `unknown flag` and usage.
  It decodes whatever bytes the log holds, reads a carriage return as a line break and removes every terminal control before it parses or quotes the text, and every excerpt a lane writes into an annotation or the report quotes into an issue goes through the same removal; on 2026-10-10 and 2026-10-16 the report died on the first log it read and opened no issue.

## Reviewing a pull request that adds or changes a test

- [ ] The test asserts a result a caller observes, and the pull request names a plausible bug it would catch.
- [ ] The expected answer comes from a requirement or contract outside the implementation; a changed expectation names that source.
- [ ] It sits at the cheapest layer that can observe the result; an e2e spec covers a flow that crosses a boundary.
- [ ] No fixed waits; every wait names the state it waits for, and polls only observe.
- [ ] Nothing slow runs while the test holds a request the product will time out, and polls in that window set `intervals`.
- [ ] An order the result depends on is fixed by a gate at a real boundary, with the barrier asserted before release.
- [ ] The fixture copies the whole isolation environment, owns every process it starts on every exit path, and reports cleanup failures without replacing the original error.
- [ ] A system difference lives beside the fixture that owns the resource, not in the spec.
- [ ] No timeout, retry, count or skip was changed to make the test pass, and no test was tagged out of a required lane.

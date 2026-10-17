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
- Before releasing, assert the barrier the race needs, such as the number of accepted requests, so a stale snapshot or an earlier diagnostic cannot satisfy it.
- After releasing, assert the outcome and the boundary's own record (`gate.requests`, `gate.maximum()`), which show the order the product actually saw.
- A race the test cannot order proves nothing either way; find the boundary to gate before writing the assertion.

## Writing a fixture

- Set every fixture's `HOME` and Hide state folder to private paths before starting a candidate.
- A restart fixture that requires completed native labels waits for the matching session, goal and end in the private `labels.json` before stopping the daemon; a rendered label alone does not prove its asynchronous save completed, and Windows fixture shutdown is forced.
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
  The fixtures run a compiled `claude` shim instead, which proves the pipeline and not an agent or physical IME.
  On Windows the interactive shim writes its readiness line after console setup; provider/auth replies return before that line, and input logs record only received input.
  The native compiler is `cc` on Unix and `clang.exe` on Windows, and no test runs it: Playwright's `globalSetup` (`web/e2e/global-setup.ts`, used by the web and desktop configs) builds every program in `web/e2e/shims/*.c` that this system runs once into `web/.e2e-shims`, each named with a hash of its source, and a test only copies the finished program (`copyFixtureShim`).
  A compiler failure fails the run at that entry point with the compiler's name and output, a program that was not built fails the test that asked for it, and nothing in a test compiles, times a compiler or cleans one up; a per-test compile is what exceeded a 20-second bound on a cold Windows runner and left the killed compiler's children behind.
  The spawn provider is built once too; its run-specific values (Herdr binary, completion file, command) are read from `provider.config` in `$HIDE_E2E_ROOT`, which every fixture pane is given.
  The private Herdr server, panes and `hided` use `fixturePath`: the shim directory followed by system-tool directories, including `/usr/sbin` for `lsof` on Unix and native Windows and Git directories on Windows.
  `fixtureExecutable` supplies native `.exe` names and `fixtureToolPath` uses the native path delimiter while refusing missing or non-absolute Windows system-root or program-files values.
  Catalog discovery probes every provider, so appending the host's `PATH` also reaches its installed CLIs even when the test selects Claude.
  A spec adding a GitHub or Git shim prepends it to `fixturePath`; explicit `extraEnv.PATH` remains authoritative.
  A node finds `git` and `gh` on the account's search, the login shell's `PATH` before its own (`hide_platform::programs::command`), and on macOS the shell's `path_helper` puts the system's folders first, so a shim for a program a system folder also holds (`/usr/bin/git`) is found only when the login shell is the fixture's too: a Rust fixture removes `SHELL` from the child or names a stand-in shell (`hide-host/tests/it/worktrees.rs`).
  A repository fixture declares its initial branch when the GitHub answers name one: `session-work-fixture.ts` creates `main` for its `main` pull requests, so the account's `init.defaultBranch` cannot change PR assignment or automatic session resolution.
  A repository fixture a fake `gh` answers for carries a remote naming that GitHub repository, as a real checkout does, because `hided` answers a checkout whose remotes name none "no GitHub remote" without asking `gh`: `session-work-fixture.ts` sets `origin` to it, and the specs start their repository with `initGithubRepository` in `web/e2e/github-remote.ts`, which adds it as `github` so a local bare `origin` keeps measuring branches.
  A claim about a real agent session needs the real CLI inside an otherwise isolated fixture: a shim on the pane's `PATH` that runs the CLI under the operator's `HOME`, while the server keeps its private one.
- A fixture makes agent labels the way the product does, through the core and not through tokens: `labelAgent` in `web/e2e/herdr-fixture.ts` gives a pane a new Claude session and writes a synthetic transcript (`writeFixtureTranscript`) the core's label worker reads, and the fixture `claude` shim acts as the provider that answers the analysis with the label the spec asked for.
  The same session is read again only when the agent's state changes: a spec that needs a different label either gives the pane a new session, or, for a pane declared as another's child, appends a turn to its own session (`continueFixtureTranscript`) and changes its state, because a relationship holds only for the sessions it was written for.
  A spec drives status through Herdr's own `agent_status` with `setFixtureLifecycle` and `finishFixtureTurn` (which needs `elsewhereTab`, so the turn ends unseen), because nothing else produces working, blocked or done.
  No spec writes a label or status pane token; the lineage tokens `declareParent` writes (`parent_pane`, `child_session`, `parent_session`) are the only ones a fixture reports, so a spawned child is labelled before its parent is declared (`spawnAgent` with a label).
  `desktop/e2e/session-labels.spec.ts` is the reference for the session boundary.
  Recorded native-format blobs keep their historical label marker; `claude-shim.c` fits the controlled provider response to the requested START or END schema, supplying `work.task.none` for an older marker in a reader journey that judges no work.
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
   The config runs one worker (`web/playwright.config.ts`) so a timing assertion does not compete with a neighbour for cores; CI deals tests out across runners by their recorded durations (`scripts/web-e2e-shard.py`, [Balancing the web e2e shards](#balancing-the-web-e2e-shards)).
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
   Why: a leaked process or file is inherited by the next test and by the next run.
8. **Give a test a program that has already started once.**
   macOS checks a file the first time it is started, once per file, and the checks of every process on the machine wait in one line: on 2026-10-09 at load 7 a new script's first start took about 90 ms, its second 6 ms, and the last of twenty new scripts started together waited 1.8 s; Linux refuses to start a file another thread's fork still holds open for writing (`ETXTBSY`).
   A program a test copies or writes and then runs inside a product deadline therefore pays that line inside the deadline, and under load it outgrew the hook's budget and the kit's 5 s deadlines (issue 813).
   In `hide-platform`, `hide-agent-hooks` and `hide-kit` a stand-in comes from `hide-platform/tests/it/stand_ins.rs`: `program(at, body)` puts at `at` a hard link to one read-only file per body beside the build's test binaries, started once per process before the test gets it, and `place` does the same for the helper cargo built.
   On macOS, `place` instead byte-copies a native executable into the fixture, ad-hoc signs that private file, and starts it before the measured deadline; it never signs a hard link or changes the build output.
   A first start killed by a signal fails setup with its exit status instead of being cached as ready; a normal nonzero exit remains valid because some CLIs and interpreters reject `--help`.
   A body is fixed text and finds its test's files from `HOME` (`${HOME%/*}` for the folder around it), never from a path written into it, because each distinct body is one more file kept for the build.
   A program checked into the repository, such as `hide-agent-hooks/tests/fixtures/fake-codex.py`, is a new file in every checkout too, so a test that runs it under a deadline makes it ready first with `stand_ins::ready` (issue 824).
   A helper-install protocol fixture instead materializes a private executable with `std::io::copy` and preserves its permissions, matching the bytes written by the device installer (`hided/tests/it/node_session_activity.rs`).
   `fs::copy` may make an APFS clone whose first execution stalls before `main`; copying its bytes keeps that clone-specific cost outside the protocol being tested without warming the helper or changing its deadline.
9. **Retries are a classification.**
   CI runs every Rust lane (Linux, macOS, Windows, the OS contract and nightly) with `scripts/verify-cargo.sh nextest --profile ci` (`retries = 1` in `.config/nextest.toml`), so a test that fails once and then passes is reported as flaky and recorded in an issue with an expiry; two failures fail the lane.
   The one test it does not retry is the nightly's signed-in agy turn, which spends a provider's daily requests ([A real agent on CI](#a-real-agent-on-ci)).
   `nextest` does not run doc tests; the workspace has none that runs today, and a runnable one needs its own `cargo test --doc` step.
   `scripts/install-nextest.sh` installs the pinned release on a runner; a lane that runs nextest several times keeps one JUnit report per run, and `scripts/ci-flaky-report.py` reads them all.
   The pin is at least 0.9.145, because an older nextest on macOS let a test started beside another inherit the other's output pipe, and reported the test whose pipe it held as leaking (LEAK) though it had left nothing running; `.config/nextest.toml` warns a local run on an older nextest, and on macOS with 0.9.145 or later a LEAK names a child the test left behind, which rule 7 forbids.
   The `ci` profile also ends a test still running at 120 s (`slow-timeout`; 240 s for the remote mailbox test), so a hang fails with the test's name, is retried and is filed like any other flaky failure, instead of holding the lane until a step or job limit cancels it with no test named.
   That limit is a hang guard set well above the slowest test each lane measures, and the comment beside it in `.config/nextest.toml` says how it was measured; a test that needs more is the finding, not the limit.
   A flaky OS-contract test is fixed or deleted by its issue's deadline like any other; it is not ignored.
   The rule against raising a deadline to pass is unchanged.

## Which lanes a pull request runs

`pr.yml`'s `plan` job reads the paths a pull request changes against the base it merges into and plans the lanes they need; `scripts/ci-plan.py` is the one place the rules live, and `scripts/tests/test_ci_plan.py` checks them against real paths.
`remote-mailbox` (the remote mailbox lane, its own Linux job) is planned for a change to any crate it builds and tests (`MAILBOX_CRATES` in the script: `herdr-core`, `hided`, `hide-agent-hooks`, `hide-host`, and so every crate they depend on); web code and web specs do not reach it.
Every lane is a job whose `if:` asks the plan, and `verify` passes only when every planned lane succeeded and every other lane was skipped.
A planned lane that was skipped, failed or cancelled fails `verify`, so a wrong `if:` shows up as a red check rather than a lane quietly left out.

| Change | Lanes |
| --- | --- |
| Only `docs/`, root Markdown, or an `AGENTS.md`/`CLAUDE.md` below the root | `policy` (the script suite and the repository invariants) |
| `design/` | `policy`; `design-contract.yml` checks the library |
| A file a lane outside its folder reads (`READERS` in the script): the root `AGENTS.md`, `web/src` and `hided/src`, which herdr-core's tests read; `desktop/src/main/wirePath.ts`, which web e2e specs import; `design/tokens.json` and `docs/UI_BEHAVIOR.md`, which web tests read; `docs/AI_PROVIDERS.md` and the agent logo manifest, which hide-agent-adapter's tests read; the contracts a test holds the code to (`hided-ws.schema.json`, `snapshot-wire-enums.json`, `agent-adapters.json`, `hide-cli.json`, `usage-coverage.json`), which the tests of `herdr-core`, `hided` or `hide-agent-adapter` read, and web unit tests for `snapshot-wire-enums.json` and the shell itself for `agent-adapters.json` | also that lane, and `rust` over `herdr-core` for the first three and over the crates whose tests read a guide, the manifest or a contract; a contract, which no other row claims, plans only these and `policy` |
| The web shell: `web/src`, `web/index.html` | `checks` (the web shell's and the desktop app's typecheck, lint and unit suites in one job), the Linux `web-e2e` and the macOS `desktop-e2e`, which drives the same shell inside the app |
| The phone page: `web/mobile.html`, `web/public/m` | `checks` and `web-e2e`; hided serves it at `/m/`, and the app never loads it |
| Web code the desktop host imports or drives through native input (the host bridge, the shortcut registry, keys and keyboard, store, snapshot and socket, terminals, focus and area cycling, `App.tsx`, `main.tsx`; `SHARED_WEB` in the script) | also `windows-e2e` and `windows-desktop` |
| A `web/e2e` spec | `checks` and `web-e2e`; a spec tagged `@platform` also runs `windows-e2e` (the whole suite runs on Linux, so the Linux leg of `@platform` is `web-e2e`) |
| `desktop/src`, `desktop/static`, a `desktop/e2e` spec | `checks` and `desktop-e2e`, the only macOS job a change outside the two crates and the package inputs below asks for, and `windows-desktop` (the desktop `@platform` tests on Windows) for `desktop/src`, `desktop/static` and a spec tagged `@platform`; `desktop/src/main` also runs `windows-check`, where the main process's unit suite runs on Windows |
| A Rust crate | `rust` over the crate and every crate that depends on it (from `cargo metadata`), `windows-check`, which compiles every crate for Windows, the Linux `web-e2e` and the macOS `desktop-e2e`, since every crate reaches `hided` and both suites drive it, and `remote-mailbox` when the crate reaches `herdr-core`, `hided`, `hide-agent-hooks` or `hide-host` |
| `herdr-core`, `hided`, `hide-platform`, `hide-herdr-client`, `hide-host`, `hide-kit`, `hide-agent-hooks` | also `os-contract` (its Linux and Windows legs), `windows-e2e` and `windows-desktop`; no macOS job beyond every crate's `desktop-e2e` but `package` for the package inputs among them |
| `hide-platform`, `hide-herdr-client` | also `os-contract-macos`, the OS contract's macOS leg, which tests exactly these two crates |
| `hide-agent-adapter`, `hide-agent-hooks`, `hide-ai`, `hide-kit`, `hide-session` | also `agy-contract`, agy's tests and the real agy on Windows (`AGY_CRATES` in the script); the nightly runs it on all three systems |
| What goes into a package (`PACKAGE_PATHS` in the script): `desktop/scripts`, `desktop/package.json`, `desktop/resources`, the Herdr pin and its fetch scripts, `verify-cargo.sh`, `verify-web.sh`, `toolchain-env.sh`, `hided/build.rs`, which embeds the web shell, `hided/src/cli.rs`, `hide-kit`, `hide-agent-hooks`, `package.yml` and `release.yml` | also `package`: `package.yml`'s Windows and Linux packages, and the macOS archive with the packaged app's own specs |
| The paths `POLICY_ONLY` names, which no lane reads: `agents/`, `site/`, `tools/`, `spikes/`, `.gitignore` files, the root `LICENSE` and `NOTICE`, the PR template and `dependabot.yml`, the contribution intake (the issue and discussion forms, `APPROVED_CONTRIBUTORS` and `contribution-gate.yml`), the workflows no `pr.yml` job calls (`nightly`, `perf`, `herdr-update`, `design-contract`), `scripts/tests/`, the policy `check-*` scripts, the design, release and measurement scripts, the pre-merge check and the merge order with the helpers they share, the design contract's generators, token map and checks (`gen-tokens.mjs`, `gen-pen.mjs`, `gen-screens.mjs`, `pen-token-map.json`, `check-design-contract.mjs` and the checks it runs, which `design-contract.yml` runs on every pull request), and Markdown below a folder no rule claims | `policy` alone |
| The paths `NAMED_LANES` names, whose readers are a known set: a `web/e2e` file that is not a spec (the `desktop` suites on macOS and Windows import it, and `desktop/e2e` unit tests run in `windows-check`), a `desktop/e2e` file that is not a spec, the Playwright, eslint and vitest configurations, `web/vite.config.ts` (every lane that builds the shell or runs its unit suite), `web/scripts`, `desktop/scripts`, the workflows `pr.yml` lanes call (`web-e2e.yml`, `desktop-e2e.yml`, `os-contract.yml`, `agy-contract.yml` with `scripts/install-agy.sh`, `remote-mailbox.yml`, each planning the lanes that call it), `package.yml` and `release.yml` | the lanes that read it, listed in the script and its test; never `rust`, and `os-contract` and `os-contract-macos` only for `os-contract.yml` |
| `pr.yml`, `scripts/` the lanes call (`verify-*.sh`, `ci-flaky-report.py`, `ci-plan.py`, ...), the Herdr pin and schema and any contract `READERS` does not name, any `package.json`, lockfile, the workspace `Cargo.toml`, a type change, and any path no row above names | every lane but `package`, which only the package inputs add |

Every plan includes `policy`, whatever else it names, except a draft pull request's: it plans no lane, and `verify` fails with "draft: lanes not run, mark ready for review".
A body edit (`edited`) starts a run too, for the checks that read the body (`check-pr-screenshots.py`); it waits for a run in progress rather than cancelling it, and with the last green run standing it plans `policy` alone.
Marking the pull request ready (`ready_for_review`) starts the run that plans and runs the lanes, and that run's `verify` replaces the failed one.
`verify` fails on a draft instead of being skipped because a skipped required check counts as passed, and `verify` is not started until its lanes finish: a skipped `verify` from the draft run would otherwise be the only check on the commit for the minutes after it is marked ready.
Keep `ready_for_review` in `pr.yml`'s `types`, and keep `verify` running on a draft; `scripts/tests/test_ci_plan.py` reads the workflow for both.
A hand run of `nightly.yml` takes a `lane` (`all`, `linux`, `macos`, `windows`, `agy`): one system's web and desktop suites, the remote mailbox lane and the agy contract, with `verify` and `package` for `all` only, or for `agy` the agent contracts alone on all three systems, agy's (signed in when the `HIDE_AGY_API_KEY` secret is set) and the `agent trust contract` of the real Claude Code and Codex, which a branch can run with `gh workflow run nightly.yml --ref <branch> -f lane=agy`; the schedule runs everything.
The nightly runs the `@platform` web tests on macOS and Windows (`web e2e (<system> @platform)`) and the desktop suite whole on macOS and only its `@platform` tests on Linux and Windows (`desktop e2e (<system> @platform)`); the whole web suite runs in the pull request and in the `verify` job a scheduled nightly calls, so Linux has no web job there and these two do not repeat it.
It also runs the performance lane on macOS (`performance (macOS)`, `perf.yml`), for the schedule and a hand run of `all` or `macos`; [PERFORMANCE_TESTING.md](PERFORMANCE_TESTING.md#the-nightly-performance-lane) owns its fixture and what fails it.
Tag a `desktop/e2e` test `{ tag: "@platform" }`, with a comment saying what differs, when it checks what the desktop host does differently on another system: its processes and local stream (including whether closing the last window ends the app), the CLI's name and places, the path it hands the wire, the file manager's reveal, the accelerators it registers for the system (one test reads them through `web/e2e/chords.ts`).
Choose by that difference, not by whether the test passes on Linux or Windows; a tagged test that fails there is fixed (its fixture or the product), never skipped, retried on a deadline or marked `@flaky`.
A test for a macOS-only behavior, or one that checks no OS difference, carries no tag and runs on macOS only; `CONTRIBUTING.md` lists what the tag covers.
A push to main plans every lane but `package`, and so does a plan that cannot be computed: a missing base, a checkout that is not the merge commit, a diff that does not parse, or a crate graph `cargo metadata` cannot read.
Main's full run is the net under a pull request that left out a lane it needed; main's runs queue rather than cancel each other.
Nightly calls `verify` on main with the same lanes, and `package.yml` on its own: a lane it fails opens a `nightly-failure` issue, which a failed push run does not, and a lane that breaks with no merge is found within a day.
When one does, fix the rule in `scripts/ci-plan.py` with a case in its test; a test that reads a file outside its own folder adds that file to `READERS`.
A path is narrower than every lane only by being named in `POLICY_ONLY`, `NAMED_LANES` or `READERS`, with its reader in a comment and a case in `NamedPaths`; a new or unknown path plans every lane until someone names it.
`Cargo.lock` stays among them: every member but `hide-platform` and `hide-herdr-client` depends on `workspace-hack`, which holds nearly every third-party crate, so a lock change reaches every crate and no narrower plan can be read from it.

A pull request whose last green run still stands reuses it: the plan names `policy` alone, and its summary names the run it reuses.
A push that merges main to resolve a conflict in a guide, or that only edits documentation, is the case it is for.
The run stands on two conditions.
What the pull request adds to main as it is now differs from what that run's head added only in paths no lane but `policy` reads (documentation, `design/`, the `POLICY_ONLY` paths); the plan merges that run's head into the current main with `git merge-tree` and compares the result with this run's merge, so a conflict resolved differently in code, or any change to code, a guide a test reads or a contract, fails it.
And main has changed none of the paths the pull request changes that a lane reads since the main that run checked: when pull request #1133 merged main, main had changed six files it changed too since its green run, four of them in `herdr-core`, and no run had checked those changes together.
`policy` still runs, because its script suite and invariants read the documentation the push changed.
Each pull request run keeps a plan record, the `ci-plan` artifact, naming the run whose lanes ran and the main and head they checked; a run that reused one records that one, so the comparison is always with lanes that ran.
The plan job reads the newest green run of the pull request's branch and head repository (`actions: read`) and fetches the commits and trees it needs (`--filter=blob:none`, about 9 MB for main's whole history).
No green run, no record, a record for another head, a failed lookup, download or fetch, or a comparison that fails leaves the plan the change itself needs, and the summary says why; no failure turns into a reuse.
This applies the merge policy's reasoning to the run itself ([CONTRIBUTING.md](../CONTRIBUTING.md#merging-into-main)): a green pull request merges without catching up with main, and `premerge-check.py` runs on the merge result before it merges, reused or not.
`ReuseGreenRun` and `LastGreenRun` in `scripts/tests/test_ci_plan.py` check the conditions, the fallbacks and the record.
### Where the macOS runners went

The organization runs 20 jobs at once and five of them on macOS, so a pull request asks for a macOS runner only for what one proves:

| macOS job | Runs in a pull request | Otherwise guarded by |
| --- | --- | --- |
| `desktop e2e` (the Electron app) | a change to what the app runs: `desktop/`, the web shell, any crate (every one reaches `hided`), the `web/e2e` helpers, and a full plan | Nothing it skips is code the app runs (a `web/e2e` spec, the phone page, documentation); main's full run and the nightly run it whole |
| `os contract (macOS)` | a change to `hide-platform` or `hide-herdr-client`, and a full plan | The nightly's `verify` call, which plans every lane and so runs all three legs |
| `package (macos)` (the packaged app) | a change to what goes into a package (`PACKAGE_PATHS`), not a full plan without one; a cold release build and the packaged app's two specs, 10 to 16 minutes, nearly all of it the build | The nightly's `package.yml` call on main, which found issue #412 a day after its cause merged; no other job starts the packaged app (its signed bundle, host, daemon and Herdr) |
| the web `@platform` tests on macOS | never (no lane) | The same tests run in every pull request on Linux (`web-e2e` runs the whole suite) and on Windows (`windows-e2e`); the nightly runs them on macOS in `web e2e (macOS full)` |
| the remote mailbox lane on macOS | never | The nightly's `remote mailbox (macOS)` job; a pull request runs the lane on Linux, and the Mac-only parts of the fixture (codesign of the staged binaries, the system's SFTP server, the macOS Herdr asset) are checked there |
| the performance lane | never | The nightly's `performance (macOS)` job; a pull request's performance claim is still a matched local comparison ([PERFORMANCE_TESTING.md](PERFORMANCE_TESTING.md#the-nightly-performance-lane)) |

A change to `herdr-core` or `hided` alone, outside `hided/build.rs` and `hided/src/cli.rs`, therefore starts one macOS job, `desktop e2e`.
What only macOS shows for such a change outside the app's suite (the OS contract's macOS leg, the web `@platform` tests and the remote mailbox lane on macOS) is found by the next nightly, which opens a `nightly-failure` issue when it fails; the cost is that delay.
Until issue #1288 a web shell or crate change left `desktop e2e` out, and a desktop spec that asserted what the change did failed first on main's full run, where a red run stops every merge but the fix: #1106 (the session record, `session-work-ledger.spec.ts`) and #1147 (the file tree a lane close keeps, `right-lane.spec.ts`) kept main red 92 and 103 minutes on 2026-10-15.
Replaying the 132 pull requests merged from 2026-10-14 to 2026-10-17 through the rule adds the 22-minute job (the median of 49 macOS runs on 2026-10-16) to 48 of them, so 121 of the 132 run it; a missed desktop spec costs every pull request waiting on main, an extra job only the macOS queue.
The desktop `@platform` tests run on Windows in a pull request (`windows-desktop`, about six minutes, on a Windows runner rather than a macOS one) and on Linux only in the nightly.

### How many jobs a run starts

A run that plans every lane a pull request can plan starts at most 22 jobs, down from 23 (the 2026-10-05 shape of run `37324202934`, before the macOS and mailbox work), and `scripts/tests/test_ci_plan.py` counts them from the workflows:

| Jobs | Count | Why |
| --- | --- | --- |
| `plan`, `policy`, `rust`, `checks`, `windows check`, `verify` | 6 | `checks` merges `web checks` and `desktop checks`: both install the same dependencies, run for a minute or two and start a runner each |
| `windows e2e` | 1 | the reusable workflow no longer starts a `plan` job that only fanned the shard numbers out; a matrix picks its list by index |
| `windows desktop` | 1 | the desktop `@platform` tests on Windows, the one job that starts the Electron app there in a pull request; it keeps the window it drew as the run's artifact |
| `web e2e` (Linux) | 5 | one build, which also downloads the zsh packages the `fetch zsh` job used to, and four shards instead of six, dealt by recorded durations; fewer jobs queue at the 20-job limit, and a shard's time at four is measured after merge |
| `os contract` (Linux, Windows), `os contract (macOS)` | 3 | one job per system, the macOS leg planned only for its two crates |
| `agy contract` (Windows) | 1 | agy's hook block, trust, conversations and Hide AI backend, and the real agy reading Hide's block, where the hook commands and install folder differ; `rust` runs the same tests on Linux and the nightly runs all three systems |
| `remote mailbox` | 1 | the lane that was the tail of the macOS `@platform` job, now on Linux and, in the nightly, on macOS |
| `desktop e2e` | 1 | the only macOS job of the desktop app's own changes |
| `package` (Windows, Linux, macOS) | 3 | `package.yml`'s jobs, planned only for what goes into a package; before the lane its Windows and Linux jobs ran on the same paths outside `verify` |

The common runs are smaller: a `herdr-core` or `hided` change starts 16 jobs with one on macOS (`desktop e2e`), a web shell change 11 with one on macOS, a desktop-only change 6 with one on macOS, a `hide-platform` change 17 with two on macOS, a `hide-kit` or `hide-agent-hooks` change 20 with two on macOS (`desktop e2e` and `package`), and a documentation change 3, as does a push that reuses the pull request's last green run.
The median wall time, the share of runs whose Linux shards waited more than 10 minutes, and each shard's time at four shards come from a measurement of the pull request runs after this change merges, taken the way issue #561 took its baseline; they are not in this guide because a pull request cannot prove them.
The `plan` job's summary lists each lane with the paths that chose it.

### Balancing the web e2e shards

Playwright's `--shard` cuts the tests in file order into runs of equal count, and a test takes from 1 second to a minute, so with four shards one shard job took 11 minutes and another 6 (run `37402687007`, 2026-10-06).
A shard of a lane with more than one shard therefore runs the tests `scripts/web-e2e-shard.py split` deals it: each listed test (`playwright test --list`) weighs its seconds in `web/e2e/shard-durations.json`, a test the table lacks weighs the table's median, and the longest test goes first into the shard with the least work so far.
The same list and table always give the same shards, and every listed test lands in exactly one shard.
The shard runs them with `--test-list`, so Playwright's retry, the flaky report and `--grep` work as before; the script reads and writes UTF-8, because a title holds `›` and a Windows runner's default encoding cannot write it.

Playwright counts a `--test-list` that matches nothing as a pass, so the step does not trust the file: `web-e2e-shard.py check` fails it unless `playwright test --list --test-list <shard file>` lists exactly the tests `split` planned, and a list, a table or a plan it cannot read is one line and a failed step.
A `grep` that leaves fewer tests than shards leaves the extra shards no test; those steps end at once, which is what the old `--shard` did.
A plan that finds tests the table lacks, or a heaviest shard more than 15 % over the mean, prints a `::warning::` on the first shard's step and still runs: the table is a recording, and refreshing it fixes the warning.
`scripts/tests/test_web_e2e_shard.py` checks the split, the check, the encoding and the table's shape.

The table is the median over 20 runs of a passing test's duration on the Linux runner (2026-10-06); on it the four shards plan 472 seconds each, where Playwright's count split planned 306 to 577.
It is a Linux recording, and only the pull request's Linux lane deals by it: the nightly's macOS and Windows lanes run one shard each.
A lane that later runs several shards on another system would deal by a table that is not its own, because per-test costs there differ (symlinks, process and terminal tests) and none has been recorded from their logs.
Refresh it when a shard's time drifts from the others in a run, when the plan warns, or after tests were added or renamed in bulk:

1. Download the logs of the Linux `web e2e` shard jobs of about twenty recent successful runs (`gh api repos/<owner>/<repo>/actions/jobs/<job id>/logs`) into a directory outside the repository.
2. `bash scripts/verify-web.sh web e2e --list --reporter=list > list.txt`, then `python3 scripts/web-e2e-shard.py durations --list list.txt <logs>... > web/e2e/shard-durations.json`; it keeps only the listed tests, takes a passing retry as its test, and names the listed tests no log has a passing line for (they keep the median).
3. Commit `web/e2e/shard-durations.json`.
A table from a Windows or macOS runner's logs would need its own file and a lane that reads it; `durations` already reads their `ok` and `✓` marks.

### A real agent on CI

A lane that signs a real agent in, like the nightly `agy contract` lane, proves only what the agent does before and around its model's answer; keep its real turn to the minimum.

- An API key alone may not sign the agent in: agy 1.4.0 asked for a browser sign-in until its settings also named `"modelProvider": "gemini"` (2026-10-16).
- A free-tier quota is counted per model per day across the key's project (20 requests on 2026-10-16), and every run that signs the agent in with that key spends it, hand runs included; budget the turns to it and say how in the workflow (`agy-contract.yml` spends 4 a night, and a passing turn prints how many model responses it took).
- A turn the provider answered nothing because that day's requests were spent is not a result: the check still asserts what came before the model (the hooks, the letter), then writes a warning annotation titled `Not measured`, the check on its first line and `provider quota exhausted (<quota ids>)` with what was not reached after it, and passes, and the nightly report keeps its lane from passing (below).
  It is not retried (`.config/nextest.toml`): a retry spends the requests again, and once they are spent it would pass a real first failure as flaky.
  The codes that mean a spent day are data both this check and any other signed-in check read, `contracts/provider-quota.json`: the provider's error code and status, and the quota ids that count (for Gemini, `429`, `RESOURCE_EXHAUSTED` and the per-day request quota).
  The rule is exact, so a broken setup cannot pass as a spent day: the agent's own structured error report, never its screen, carries that code and status, and every quota it names is one of those ids with a value above 0 (agy: `spent_quota` in `hide-agent-hooks/tests/it/agy_binary.rs`, with agy's recorded reports in `hide-agent-hooks/tests/fixtures/agy-errors/`).
  A quota of 0 (a model the key is not served), any other quota (a per-minute limit, the tokens Hide's own context sends), a model that answered and then ran out, a 503 the agent's retries never got past and every other error fail; a check that signs another agent in adds that provider's row and writes the same annotation.
- Assert what happens before the model answers (the hooks fired, the letter went in, a refused call carried Hide's reason) rather than what the model writes, and name the model in the workflow so a renamed one is one line to change.
- Keep the turn's own bounds inside the test's hang guard (`.config/nextest.toml`), so a slow turn fails with the agent's report instead of being ended with nothing.

## Flaky tests

A test is flaky when it both fails and passes on the same SHA.
A failure on a different SHA is not evidence of flakiness by itself, and the same title can carry two different failures.

A flaky test never blocks a pull request's required gate or a merge, and it does not stay: it is isolated as an issue rather than moved out of its lane, and its cause is removed by that issue's deadline.
CI retries a failed test once, for classification and for the report, and for nothing else.

- Playwright runs with `retries: 1` when `CI` is set (`web/playwright.config.ts`, `desktop/playwright.config.ts`), and the Rust workspace runs under `cargo nextest` with the `ci` profile's `retries = 1` (`.config/nextest.toml`, `scripts/verify-cargo.sh nextest`).
  Neither retries locally: a flaky test fails where it is written.
  The nightly's signed-in agy turn is the one test CI does not retry: a retry spends the provider's daily requests again, and once they are spent it would pass a real first failure as flaky ([A real agent on CI](#a-real-agent-on-ci)).
- A test that fails and passes its retry is flaky: the lane passes, the log marks it, and `scripts/ci-flaky-report.py` reads the tool's own report (Playwright's JSON, nextest's JUnit) and files it as an issue labelled `quarantine`, or comments on the open issue that already tracks it.
  The issue names the first run, the change and the system, and carries a deadline seven days out.
  The report step also sets its `flaky` output, and a lane that keeps its e2e logs for a failure keeps them for a flaky run too, under the same artifact name, because the first attempt's daemon, Herdr and input logs are the only evidence the issue gets; a run with no flaky and no failed test uploads nothing.
  On Windows CI the web suite also logs, for each failed attempt and at that moment, what holds TCP connections (count by state and by process name, `[windows sockets]` lines from `web/e2e/windows-sockets-reporter.ts`), because a Chromium `ERR_NO_BUFFER_SPACE` on a loopback connect left nothing saying who held the sockets; a passing attempt runs nothing.
  A wait that gives up on a race between Herdr and the page (the agent tab count in `checkout-owner.spec.ts` and `tab-strip-fit.spec.ts`, the dropped file's paste in `s3.spec.ts`) records both sides at that moment through `dumpOnFailure` in `web/e2e/failure-dump.ts`: a `[failure dump]` line in the log and an attachment in the attempt's `test-results` folder. It rethrows the wait's own error unchanged, so it explains a flake and never absorbs one.
  The desktop suite keeps, for each failed attempt, the end of each private daemon's `Logs/core.jsonl` (at most 256 KiB, whole records) as `hided-<n>.jsonl` in that attempt's `test-results` folder, which the same artifact carries, with the run's folders, the repository, the home and the temporary folder written as placeholders, because the host log alone could not show the order of the core's focus records behind a flaky ⌃Tab cycle (issue 629); a passing attempt copies nothing.
  A test that fails its retry too fails the lane; nothing else is retried anywhere.
  The report step writes it on the job as an error annotation titled `Failed test` (its file and line, and its name on the first line of the message), which the run's summary page shows and the nightly report files (below).
- At the deadline a flaky test is fixed or deleted.
  Whoever knows the cause opens the fix or the deletion; if nobody does, the issue goes to the operator.
  The deadline is not a timer that moves the test somewhere quieter, and a later flake on an issue past its deadline says so in its comment.
- A green lane is not proof that a flaky test passed on that commit: its first attempt failed.
  Read the `Report flaky and failed tests` step and the `quarantine` issues before claiming a flow verified.
- The report step cannot fail a lane: the run already passed.
  If it cannot file (a GitHub error, a read-only token), it leaves the unfiled tests in a warning annotation and in the job summary under "Flaky tests that were not filed", and the next flaky run files them; a filed run lists its issues in the same summary.
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
- The issue names the test, measurement or step, the lane and its runner, an excerpt of the error (run through the same cleaning as a flaky issue's, since the page is public), the run and its commit, the lane's last green nightly within 14 runs, the pull requests merged since then that touched the failing test's file (for a step, the lane's workflows and the scripts they call; for a measurement, every one of them, since a measurement moves with any change) as candidates, and an expiry two days out.
  It is ended by fixing the cause or reverting the candidate; a longer timeout, more retries or a skip does not end it.
- The same failure again (a hidden `nightly-failure:<lane>:<test or step>` marker) is one comment per run attempt on the open issue, never a second issue.
- A scheduled run on main, or a hand run of `all`, that passes a lane closes that lane's issues with a comment naming the commit and the run; a skipped or cancelled lane closes nothing.
- One run opens at most 10 issues; the rest, with the count of failed tests a lane had no annotation room for, go into one issue, `nightly: N more failures in run <id>`, which the next scheduled run closes as it files its own.
- A run of any other branch writes no issue.
- A check that passed without measuring (a `Not measured` warning annotation, [A real agent on CI](#a-real-agent-on-ci)) passes its job, but its lane is not green: it closes no issue, is no lane's last green nightly, and the report lists it under "Not measured" in its job summary.
- The report fails its job when GitHub does, so a failure it could not file shows as a red report job rather than a silence.
  `--dry-run` with `--repo` and `--run` prints what it would write for any finished run and writes nothing.

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

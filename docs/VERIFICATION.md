# Verification

This guide answers the question asked before a change is called verified: which check proves this claim, and how to run it without touching the operator's work.
It owns only what no other document does: the choice of check, the traps that have made a check prove nothing, the tools for native QA, and device checks.
How a test is written, its fixture included, lives in [TESTING.md](TESTING.md); the gates are listed in [CONTRIBUTING.md](../CONTRIBUTING.md), build identity lives in [dev-runtime.md](dev-runtime.md), and runtime isolation and measurement live in [PERFORMANCE_TESTING.md](PERFORMANCE_TESTING.md).

## Pick the check by the claim

A claim is verified by the check that observes what a caller of the behavior observes; a green `verify` proves only the lanes it ran.

| The change claims | Check | Owner |
| --- | --- | --- |
| Core state, a runtime event, a Herdr fixture, the hided wire | `bash scripts/verify-cargo.sh test` | [CONTRIBUTING.md: CI gates](../CONTRIBUTING.md#ci-gates) |
| Local mailbox intake, safe doorbells, durable watches and activity privacy | Named library checks plus actual isolated runtime observations | [delivery.md: Verification](delivery.md#verification) |
| Web or desktop logic that needs no window | `bash scripts/verify-web.sh` | [CONTRIBUTING.md: CI gates](../CONTRIBUTING.md#ci-gates) |
| A flow a user performs in the web shell | `pnpm --dir web e2e` | [Before an e2e run](#before-an-e2e-run) |
| A desktop app behavior: window, menu, native view, relaunch, a daemon that goes away | `pnpm --dir desktop e2e` | [BUILD.md: The desktop app](BUILD.md#the-desktop-app) |
| A file the desktop host loads from its own bundle (the status page) | `desktop/e2e/status-page.spec.ts` with `HIDE_E2E_PACKAGED_APP` naming a package | [ARCHITECTURE.md: The desktop host](ARCHITECTURE.md#the-desktop-host) |
| A browser display | The commands in its guide | [BROWSER_DISPLAYS.md: Verification](BROWSER_DISPLAYS.md#verification) |
| A screen matches its approved design | Gallery or isolated candidate captures under the approved conditions | [DESIGN_WORKFLOW.md](DESIGN_WORKFLOW.md#visual-review) |
| Responsiveness, rendering, CPU, or memory | The layered procedure | [PERFORMANCE_TESTING.md](PERFORMANCE_TESTING.md) |
| The installed app carries the fix | A rebuilt package, then the identity checks | [dev-runtime.md](dev-runtime.md), [INSTALL.md](INSTALL.md#verify-the-installed-app) |
| A remote device's behavior | A private Herdr on the device | [A device check](#a-device-check) |
| Each supported agent works end to end in Hide | `python3 scripts/agent-journey-check.py` | [Agent journey](#agent-journey) |
| Anything the rows above cannot reach yet | Manual QA on an isolated candidate | [Manual native QA](#manual-native-qa) |

The desktop e2e runs the app from plain files, so it cannot see what only a package does: the status page loaded from `app.asar` under the shipped fuses failed in every package while every e2e passed, so a claim about a file the host loads from its own bundle is proven by a package, not by the dev build.

Run the Rust lanes through `scripts/verify-cargo.sh`, never a bare `cargo test` from an agent's pane: the pane carries `HERDR_SOCKET_PATH` for the operator's Herdr, and on 2026-10-02 two daemon tests followed the operator's live Herdr with the real HOME, where the label worker reads the operator's conversations, because the core then took that variable over the socket a test handed it; the core no longer does, it takes its home from hided rather than from the process `HOME`, and the script clears every `HERDR_*` variable first.

For a file-persistence claim, run the platform's public contract and OS fault regressions through `bash scripts/verify-cargo.sh test-scoped -p hide-platform` on each supported system.
The durable writer's regressions observe returned phases and causes, installed bytes and identity, retained cleanup residue, and the private access every durable file gets; unit faults are scoped to the test thread at the OS boundary and exercise the public writer without replacing owned helpers.
A readable file alone proves no durable acknowledgement, and a file-parent barrier does not prove persistence of a newly created ancestor chain or recovery of the caller's state after an uncertain replacement.
The public bootstrap regressions check the 64-component bound, refusal of escape and untrusted paths, retained partial chains after real mkdir or barrier failure, and recovery that checks existing levels again; the supplied ancestor's established durability remains a caller precondition.
On Windows they also show which ancestor the chain passes over: one the account neither owns nor may add to, never one whose flush failed.
The same scoped platform lane exercises guarded launches and captures against real children: stdin payload, both output streams, 64 KiB and 1 MiB limits, inherited-pipe timeout, abrupt owner death with a helper outside its group, and repeated work returning the descendant count to its baseline.
Capture failures must preserve their primary outcome and report unconfirmed cleanup while retaining ownership; local platform timing checks alone do not prove a consumer's complete 1.85-second deadline or native behavior on another operating system.
Existing-anchor regressions establish the canonical chain with real barriers before creating sibling home and state directories, reject links and untrusted leaves without mutation, and preserve identity through a reported barrier failure and explicit recovery that checks the existing chain again.
Windows's pre-mutation `Unsupported` is fail-closed interim behavior, not cross-OS bootstrap completion, and callers must not become ready without an established parent-chain boundary.

`python3 scripts/check-herdr-schema.py --herdr-bin <pinned binary>` compares the binary's own schema and version with the committed contract and manifest on macOS, Linux and Windows.
It clears inherited `HERDR_*` and needs no server; the full zsh contract check delegates this same comparison before its live-server checks.
The schema digest is canonical JSON (sorted keys, UTF-8, two-space indentation and a trailing newline).
A configured nightly matrix proves no executed job, native input or package launch by itself.
Record the actual head, attempt, job URLs, failures and skips; a package smoke with a private HOME or simulated hook does not prove a physical IME, an operator PATH or a real agent session.
The macOS package job (nightly, and a pull request's `package` lane) extracts and checks the actual archive, then runs the existing isolated install-kit and packaged-app tests.
The Windows/Linux package lanes check the real archive's headless daemon/kit behavior, and the Windows lane also opens the unpacked `hide.exe` with its window and keeps a screenshot of the shell it draws (`desktop/e2e/packaged-window.spec.ts`); physical input, and the Linux package's window, still need device evidence.

`hide-platform/tests/it/process.rs` exercises `run_to_end` with a real parent that starts a same-group helper inheriting both outputs and exits 0.
The call must return the parent's code and both markers within the original five-second deadline, with the helper gone; test recovery is armed with that helper's identity before its parent exits.
This regression checks process and pipe ownership, not a native app window or Unix crash containment.

CI retries a failed e2e or Rust test once (all but the nightly's signed-in agy turn), so a green lane can hide a test whose first attempt failed.
The `Report flaky and failed tests` step and the open `quarantine` issues say which; read them before claiming a flow verified, and treat a flaky test as unproven ([TESTING.md](TESTING.md#flaky-tests) owns the policy).

A scenario someone would check by hand becomes a spec when it can; [TESTING.md](TESTING.md) says how to write it.
Playwright drives the renderer over its own connection rather than through OS input, so a spec needs no keyboard focus and cannot type into another app.
Manual QA covers what a spec cannot reach yet, and the pull request's Evidence says the check was manual.

## Before an e2e run

- Both suites run built output: `pnpm --dir web e2e` rebuilds `web/dist` and `pnpm --dir desktop e2e` rebuilds `desktop/dist`, but neither builds `target/debug/hided` or `hide`.
  After a merge or checkout that touches `herdr-core/` or `hided/`, run `bash scripts/verify-cargo.sh cli` first; a spec that fails on behavior main already has usually means the binary predates the merge.
- A single spec run through `pnpm --dir web exec playwright test` skips the build, so run `pnpm --dir web build` first.
- Spec filters resolve against the package directory: `pnpm --dir web exec playwright test e2e/overview.spec.ts`, never `web/e2e/overview.spec.ts`.
  A wrong filter prints "No tests found" and exits 1, which a pipe into `tail` hides; set `-o pipefail` and read the "N passed" line before trusting the result.
- Desktop e2e windows open on the operator's screen, behind their windows.
  Run `pnpm --dir desktop e2e --grep-invert @needs-focus` locally, and leave the focus tests and repeated runs (`--repeat-each`) to CI.
- A relaunch that attaches to a running daemon swaps its page within tens of milliseconds, which Playwright can miss; use `relaunch()` from `desktop/e2e/fixture.ts`, which reads the window through the main process.

## Probing a real agent

A probe runs the agent the operator signed in, on the operator's account, so it can reach the operator's files and the vendor.

- Never run a real agent with a changed `HOME`.
  agy lost its keychain sign-in that way and opened a Google sign-in window in the operator's browser on every run (2026-10-16); isolate with the real `HOME` and a scratch working folder instead.
- Never pass an agent's skip-permissions flag in a probe.
  With agy's `--dangerously-skip-permissions` its model opened files outside the scratch folder and sent them to the vendor (2026-10-16).
- A capture hook records the names of environment variables, never their values.
  A capture once wrote a key's value into its record (2026-10-16).
- Back up every file of the agent's the probe may write and restore it byte for byte afterwards.

## Manual native QA

Read [PERFORMANCE_TESTING.md](PERFORMANCE_TESTING.md#2-identify-the-build-and-protect-the-operator) sections 2 and 3 first; they own isolation and the foreground boundary, and this section adds the tools.

### Why targeting is the hard part

- The operator's `/Applications/hide.app` and a packaged candidate from `desktop/out/` share the name `hide` and the bundle id `me.grab.hide.desktop`.
  Anything that resolves an app by name or bundle id reaches whichever one it finds first: open-computer-use, System Events (even `first process whose unix id is N` dereferences by name), `open -a`, and `quit app id`.
- An unpackaged `pnpm --dir desktop dev` instance and every desktop e2e instance run as `Electron` (`com.github.Electron`), so those collide with each other instead.
- Without its own `HIDE_DESKTOP_USER_DATA_DIR`, a dev instance focuses the operator's app and exits ([dev-runtime.md](dev-runtime.md#one-instance-per-profile)).
- Identify the app by the candidate executable and the PID that owns its window; a name-only process match does not prove which build rendered it.
- A global input event (a CGEvent tap, typing with `--foreground`, a pointer move) lands in whatever is frontmost when it is delivered, and a frontmost check a moment earlier is not a guard.
  In September 2026 such events selected a row in the operator's Explorer and activated the operator's app in the middle of a run.

### Tools

| Tool | Addresses | Use it for | Not for |
| --- | --- | --- | --- |
| Playwright `_electron` through `desktop/e2e/fixture.ts` | The app it launched | Typing, chords and clicks through the renderer, without OS focus | Native input: a pinch, a native drag, a physical input method and its candidate window (tag those `@needs-focus`); the composition events a page receives are driven through CDP `Input.imeSetComposition` in a web e2e (`web/e2e/ime.ts`), and one that checks the bytes a pane's program reads is tagged `@platform` |
| `screencapture -x -o -l <window-id>` | One window | A current frame of the candidate without activating it | Minimized, hidden, or off-Space windows |
| `desktop/e2e/electron-window.ts` | The Electron process and sole BrowserWindow Playwright launched | The running candidate's renderer, without Screen Recording permission or OS focus; retains its PID, window id, bounds, build and capture method | OS window chrome, compositor presentation and native input; renderer captures do not prove those surfaces |
| The window list below | PID to window id | Resolving the candidate's window id before each capture | |
| open-computer-use (`ocu`, or its MCP server) | An app name or bundle id, never a PID | Reading the accessibility tree; `click` on an element index and `set_value` on a candidate whose name answers exactly one process | A packaged candidate while the operator's app runs; keyboard input into the renderer, which 0.3.5 does not deliver reliably to Chromium; `click_method: global` |
| `herdr pane read <pane> --source recent --lines <N>` under the private socket | One pane | Terminal content without a screenshot | History of a Claude or Codex pane: they draw in the alternate screen, so `recent` is what is visible, and history comes from the transcript |

A capture shows an old frame unless the candidate was launched with `--disable-backgrounding-occluded-windows`, because Chromium stops painting an occluded window.
Before trusting open-computer-use's target, check that `ocu list-apps` shows the candidate's name once and that no other process answers it.
Its element actions have not yet been exercised against hide's renderer, so observe the effect after every action and record whether it landed.

The window list prints owner PID, window id, owner name, and title for every on-screen window:

```sh
osascript -l JavaScript -e 'ObjC.import("CoreGraphics"); ObjC.deepUnwrap(ObjC.castRefToObject($.CGWindowListCopyWindowInfo($.kCGWindowListOptionOnScreenOnly | $.kCGWindowListExcludeDesktopElements, 0))).filter(w => w.kCGWindowLayer === 0).map(w => [w.kCGWindowOwnerPID, w.kCGWindowNumber, w.kCGWindowOwnerName, w.kCGWindowName || ""].join("\t")).join("\n")'
```

### Procedure

1. Start a private Herdr server with the pinned binary and the isolation environment, and prove `herdr workspace list` is empty under that environment before creating fixtures.
2. Clear every inherited `HERDR_*` and `HIDE_*` variable, then launch the candidate with its own `HOME`, `HIDE_STATE_DIR`, `HIDE_DESKTOP_USER_DATA_DIR`, and `HERDR_SOCKET_PATH`, and `HERDR_BIN_PATH` naming the pinned binary.
   Pass `--hide-show-inactive --disable-backgrounding-occluded-windows` so it opens behind the operator's windows and keeps painting: `pnpm --dir desktop dev --hide-show-inactive --disable-backgrounding-occluded-windows` appends them to `electron .`, and a `--` before them would reach Electron literally.
   The `host.start` line in `$HIDE_DESKTOP_USER_DATA_DIR/logs/desktop.log` records `packaged`, `version`, and `show_inactive`.
3. Resolve the candidate's PID from the process your launch started, never by name, then its window id from the window list.
   Re-resolve both after any restart.
4. Before each action, confirm the target again; after each action, observe again with a fresh capture, accessibility read, or pane read, rather than trusting the tool's success reply.
5. Drive a scenario that needs real key semantics (an Enter or Tab override, a physical input method's key order, a chord through the menu, a pinch, a native drag) with a `@needs-focus` spec, or in a foreground window agreed with the operator.
   A scenario that cannot be driven either way is covered by tests and reported as not observed natively.
6. Quit only what you launched, by exact PID, stop the private server with `herdr server stop` under the same environment, and check that no process of your run remains.

### Evidence

- A visible claim needs a real capture of the candidate's window; a live process or a passing unit test is not UI evidence.
- Record which build answered: `packaged` and `version` from `host.start`, or the installed bundle's `CFBundleShortVersionString`.
- Missing Screen Recording or Accessibility permission (`ocu doctor`) blocks the respective OS capture or accessibility check.
  The grant has been seen to stop matching after the installed app was replaced; report the check as blocked rather than focusing the window to work around it.
- Artifacts go under `agents/runs/<slug>/` ([AGENTS.md](../AGENTS.md#evidence-belongs-outside-the-repository)); the pull request states what was observed, how, and what was not.

### Focused-area navigation

`web/src/areaCycle.test.ts` checks full device/checkout/area identity for View areas, the Agent pane order across devices and checkouts, commit events, Agent cycling outside a View without inventing visits, and single-tab or retired View no-op boundaries.
`web/e2e/recent.spec.ts` commits an Agent pane in another project on a real isolated Herdr, opens the list from the sidebar, Explorer and Overview, and keeps focused files in their own View scope.
`web/e2e/recent-agents-restart.spec.ts` restarts hided and reloads the page, then reaches an agent pane visited only before the restart from the same cycle.
`desktop/e2e/keys.spec.ts` walks ⌃Tab over Agent panes in the desktop host, including from sidebar focus.
The area renderer can be checked with Korean document fixtures in both themes and two widths, including unchanged geometry when keyboard ownership moves.
Private design-review tooling is maintained separately from this public checkout.
These checks do not prove native browser input or terminal readability.
The `area cycle native` case in `desktop/e2e/browser.spec.ts` is tagged `@needs-focus`: it uses real macOS modifier input into the isolated candidate's page, counts core selection events, checks page key consumption and captures that exact native window.
It covers the default chord and a rebound two-modifier chord, whose Control is released while Option is still down, and types into the page the release selected.
Its keys are CGEvents posted at the HID tap, each refused unless the candidate is frontmost.
System Events' `key down control` posts no flags-changed event, so no page ever sees Control go down or come up and a release case fails for a reason the product does not have.
Typed keys are digits, because a key code passes through the operator's input source: under Korean 2-set a letter becomes a jamo and leaves a composition open.
Input the run did not send, such as an operator typing while the candidate is frontmost, reaches the candidate and spoils the run, so the slot must be one where nobody uses the keyboard.
Run it only in an agreed foreground QA slot, using `pnpm --dir desktop exec playwright test browser.spec.ts --project needs-focus --no-deps --grep 'area cycle native'` after building this worktree's web and desktop output.
Record the candidate PID/window, private daemon/server/profile and build head beside the captures.
Compare any temporary weak-blur proposal against the readable treatment in the actual terminal, document and native page, with idle and driven measurements, before choosing it.

## Local agent bell measurement

Run `python3 scripts/agent-live-check.py --agents codex --model codex=gpt-6-luna` before proposing a new bell target, after changing the pinned Herdr, and after a substantial agent CLI update.
Omitting `--agents` selects every adapter declared by the current source.
Confirm the cheapest suitable model for the logged-in account and pass `--model ID=MODEL`; recipe defaults can become unavailable.
Fetch the pinned binary with `scripts/fetch-herdr-runtime.sh` first, and build this worktree's `hided` and `hide` with `bash scripts/verify-cargo.sh build -p hided`.
The tool never builds, upgrades a CLI, logs in, or changes bell declarations.
An unclassified startup menu remains measurable when the pinned CLI's readiness wait expires: its pane, workspace, cwd, agent kind and actual menu must match the owned launch before observation continues.
Registration may still be absent after that timeout; if an agent name is present, it must match the requested name.
Other startup refusals retain their screen and agent evidence without being counted as arrival.
The startup scene never auto-selects trust, and a blocked startup receives no bell input.
For other scenes, an observed recipe may prepare the owned folder by selecting its explicit trust option with Down and Enter before the scene baseline.
Each key requires fresh matching pane, workspace, cwd, agent kind and selection evidence; an unknown option, changed identity, login prompt or missing ready composer prevents the scene from proceeding.
Preparation has one scene deadline, two key attempts at most and at most four retained frames, saved separately from bell observations.
The deadline starts before its first screen read; every screen query, identity query and key command receives only the remaining time, and late responses cannot authorize another key or readiness.
Each command retains the 15-second cap and the same absolute launch deadline through receipt checks, registry setup and the guardian's child admission.
Required teardown may finish after that deadline, but never renews permission to start another command.
An actual received screen is retained immediately, even when its following deadline or identity check fails; unavailable identity remains null and never authorizes input.
Scene arrival and post-bell observations likewise retain their latest received frame before authentication or identity checks, within the existing two-frame bound.
The controller never writes shared operator trust files; any native-agent private probe key follows the shared-file observer and leftover policy below.

The command starts the verified pinned Herdr on a private socket and this worktree's hided on private state, creates disposable Git checkouts, and attests their actual shells through the candidate `hide workspace bootstrap`.
It never opens a renderer or controls the installed app.
Native agents keep the operator HOME for existing login.
Supported per-command config roots use bounded private copies of existing authentication, with source/version provenance in each provider row.
Private copies have mode 0600, never update the account's login, and are removed on controlled exit even if another teardown step fails.
An uncatchable controller kill can retain private copies in the ignored run directory; remove a stale run only after confirming its owned processes have ended.
Authenticated probing currently requires macOS `sandbox-exec`; a real declared-file write denial, permitted native-state write and socket denial self-test must pass before a provider starts.
The profile denies every outbound Unix socket except the run's own and the system resolver's (`/private/var/run/mDNSResponder`), through which macOS resolves every host name; without that one a native agent cannot reach its own API.
Unsupported hosts fail closed.
The process guardian supports macOS and the trusted Linux fixture lane; Linux authenticated probing remains unsupported.
Every direct child starts in its own session and process group.
The successful `start_new_session` spawn establishes its reserved group; the guardian confirms membership through its complete group snapshot.
It does not use `getpgid` as a liveness check, because Darwin returns ESRCH for an unreaped zombie that remains visible in that snapshot.
An EOF guardian sends CONT and TERM to that group, allows a bounded grace period, then sends KILL and requires the kernel to report the group absent.
It retains the direct child's unreaped identity until the last group signal, so its group ID cannot be reused during signalling.
It learns that child's exit from the kernel (kqueue `NOTE_EXIT` on macOS, a pidfd on Linux) without reaping it, so a command that ends at once is not held for a whole poll.
No group signal is sent after that child is reaped.
If a group signal is denied with EPERM, a fresh complete group snapshot may confirm that only zombies or no members remain; this ends that signal obligation, while reaping and final absence checks remain required.
A live member or unavailable group metadata retains the signal failure, and bounded diagnostics record the original errno and refresh result.
Process queries classify the expected birth as live, vanished (ESRCH or a replacement birth), or zombie; denied identity and same-birth UID inconsistency remain failures.
Group enrollment takes membership and birth from the same kernel record, so a PID reused outside the reserved group cannot inherit its ownership.
Individual signals recheck the proven birth and UID, and refresh once after permission refusal; only a positively terminal identity ends that signal obligation.
A changed argument pointer width makes a stale token read unknown without ending established birth ownership.
A zombie prior guardian has ended supervision; an unavailable prior guardian grants no authority to adopt its marked helper.
A final group probe denied for a zombie-only group continues bounded absence polling, because zombies remain present until reaped.
Guardian and cleanup failures end the measurement as failures rather than being reported as scene timeouts.
Per-scene integration observation and workspace close are attempted independently; their failures are reported beside any original scene failure without replacing it.
Group metadata and RSS enforce the process and memory caps; a partial host-wide resource summary never proves cleanup.
If Darwin's RSS read fails, one fresh BSD identity read distinguishes a vanished or replaced subject, a zombie and a still-live unreadable process.
An observed replacement in the owned group retains unknown RSS; a host-wide replacement requires its own ownership proof, and a replacement outside the sampled group never inherits the old identity's ownership.
RSS measures resource use independently of ownership; a missing sample never removes a live identity from process counts, signalling or cleanup.
Vanished subjects and zombies need no RSS sample.
A still-live unreadable PID and birth counts as a missed sample and is measured again on the existing next polling cycle.
Three consecutive missed samples of that same live identity fail; a successful read, disappearance, zombie or replacement birth resets its streak.
Available samples still enforce the unchanged RSS cap immediately, and an incomplete sum is explicitly marked as such in the resource summary.
Guardian receipts and the final report record total missed samples, the largest consecutive streak and its fixed limit of three.
The final report separates controller and guardian observations and also totals them, including the last failed observation when resource sampling raises.
If receipt validation fails, cleanup still fails and the resource report retains known controller counters while marking guardian and combined totals unavailable.
A readable, birth-rechecked owner marker also identifies helpers that left the group, including markers from an earlier run of this checkout whose guardian has ended.
A different live guardian's marker belongs to concurrent work and is excluded.
Private issued marker families are retained under `agents/runs/process-owner-families/` so a fabricated scope prefix cannot claim a previous run.
The registry has a locked 4096-family cap; exhausting it is a reported failure rather than an unbounded directory.
The private Herdr server and hided also receive their protocol close before process teardown.
Unrelated orphans whose environment is empty or unreadable are neither signalled nor treated as cleanup failures.
The report lists these as "출처 확인 못 함" with only PID, birth and executable name, within a declared record cap.
A known owned identity that becomes uninspectable still makes cleanup unconfirmed.
Every launched guardian must produce a confirmed cleanup receipt; a missing receipt, an unconfirmed receipt or a diagnostic write failure fails cleanup.
Once every owned process and guardian is positively confirmed ended, the probe and short socket directories are removed independently even if a protocol close or log write failed.
Those errors remain failures with exit 2; missing process confirmation retains the directories.
An unseen double-fork descendant that clears its marker and leaves the owned group may escape attribution; record this limitation in the PR's Review section.
This ownership boundary follows the explicitly recorded B8/D-09 scope decision; it does not claim complete workstation process attribution.
Linux fixture supervision uses a child subreaper and validates its procfs namespace and visibility before using process identities.
The subreaper keeps an adopted process that ended as a zombie until the run's cleanup, and Herdr's pane close on Linux asks `kill(pid, 0)`, which a zombie answers, so Herdr logs `pane session still alive after forced shutdown` for an agent it did end; the warning is the harness's, and a desktop's init reaps the same process at once.
On Windows each child starts suspended and joins a job object of its own before it runs, so everything it starts is in the job (`scripts/agent_live_check/windows.py`, after hide-platform's `process`).
The job holds the same caps, measured as Linux and macOS measure them: 128 processes, which the job itself refuses, and 2 GiB of resident memory, the working set of every process in the job summed every 0.1 s as a guardian sums RSS, so the three systems judge a run by one measure.
This replaces the committed memory #1327 capped on Windows: Windows counts every page a process committed, touched or not, so that cap ended a run Linux and macOS pass within theirs.
Between two samples the sum can pass 2 GiB by what the job's processes touch in 0.1 s; the job's own limit on committed memory stays behind it as the hard stop, at twice the cap (4 GiB), which Windows applies to each allocation, so a runaway ends there whatever a sample saw.
Its handle is never inheritable, and closing it ends what the job holds, so an owner that dies takes the tree with it.
A refused process or allocation reaches the owner through the job's completion port, and resident memory past the cap is found by the thread that reads that port; either ends the job, and the owner's next end of that child fails with the cap's name, as a guardian's refusal does.
Output, the command time and the run time are capped by the same code as on Unix.
A program Windows starts on a child's behalf (a service, a COM server, a scheduled task) is outside the job, and the report says so.
Private files there carry an access list naming only this account and are checked by owner and access list (hide-platform's `fs`).
The owned pane runs cmd.exe, named as Herdr's default shell with its prompt from `PROMPT`, as the web e2e's Herdr fixture does, and the launchers the run writes are `.cmd` files.
Darwin group enumeration uses Apple's [process-group API](https://github.com/apple-oss-distributions/xnu/blob/main/libsyscall/wrappers/libproc/libproc.c).
Token lookup preserves the existing target-width alignment and birth checks and records no argument, environment or marker contents.
Each Darwin token scan reuses one buffer of at most 1 MiB, supplies its full capacity for every query and interprets only that query's returned prefix.
Bytes retained from a previous PID never establish another process's ownership.
`--socket` and `--state-dir` cannot select operator routing, existing sockets, or state outside the new run.

Every recipe retains rest, working, shell approval, file approval, question, plan approval, model picker, resume picker, MCP approval and startup rows.
MCP registration uses each CLI's documented project file or per-command override only for its MCP scene; recipe data records that source.
Rest and resume share a disposable checkout and private session catalog.
Supported native readers first observe a real previous user/assistant turn.
Other providers seed a real prompt and require a positive native identity and a working-to-ready transition; that weaker history evidence cannot verify assistant text or mail.
Resume arrival additionally requires that owned session's label, filename or identity on the picker screen.
Missing history, an empty picker or an unobserved native transition remains unknown.
The tool checks scene arrival independently of Herdr status, types the source's sample bell line (`SAMPLE` in `herdr-core/src/delivery/bell.rs`, a line the bell types for a letter whose first line carries `@`, `$`, `/`, `!`, `#`, backticks and Korean past the 60-character cut) and Enter only when the observed status is not `blocked`, then observes the effect.
Approval work stays within `probe-N.txt` in the disposable checkout.
A confirmed selection, approval, session replacement, settings write or unsubmitted draft is unsafe, even when other scenes remain unknown.
An unrecognized, inaccessible or timed-out scene never counts as safe.
These direct input experiments measure the CLI and Herdr detection boundary; they do not replace the core's draft, session, identity or plan-approval guards.

Each new private `agents/runs/live-check-<timestamp>-<id>/` contains `report.md`, `report.json`, screen reads and private configuration backups.
Reports record the actual binary digests, Herdr version, active server manifest versions and sources, CLI versions, selected models and positively observed native integration provenance.
The verified pinned binary generates its own integration assets in a disposable HOME, then the tool routes them through documented provider settings while preserving the authenticated provider's operator HOME.
The loaded-version claim requires an isolated configuration route, unchanged prepared artifact hashes, one common version across its possible native emitters and a native session report, sampled again after the first turn.
Prepared files are inventoried separately; the claim does not say every prepared frontend asset loaded.
A stable change to an ordinary bounded private integration artifact invalidates loaded-version provenance for the rest of that provider run, including after the original bytes return or native session visibility disappears.
The report retains the relative artifact name and changed stamp fields, without changed contents or their hashes, and refuses a positive safety aggregate while preserving any confirmed unsafe scene.
Known integrity loss is published even if a later artifact refusal or native query stops that observation; the original fatal error and cleanup failure ordering remain intact.
Artifact reads open ancestor directories without following links; path, alias, file type, owner, link count, size, missing-artifact and unstable-read failures still stop the run.
Each integration observation reserves its remaining read budget before opening content: generated originals and routed copies each retain their own 16 MiB aggregate cap, including raced growth.
A prepared file alone never proves load; Cursor's project hooks and copied OpenCode configuration cannot exclude external-hook or plugin ambiguity, so their loaded versions remain unproven.
Only adapter-listed configuration/trust files are read and backed up with private permissions, with a 16 MiB per-file and 256 MiB total backup limit.
Nonshared declared files retain exact byte guards.
The declared shared Claude JSON and Codex TOML files are compared read-only and are never restored or rewritten, preserving other live sessions' updates.
Shared-file changes are reported as other-session changes without failure, and entries in their `projects` tables keyed by this run or its probe descendants are reported by file and key as B6 leftovers without failure.
Shared parsing uses Python 3.11 or later's standard TOML reader, rejects unreadable tables and caps each project table at 50,000 entries.
Shared-file version races receive at most three immediate read attempts; persistent races report an unavailable comparison without failure or a claim of absence.
Unavailable initial observations mark final observed private keys as not compared, while unavailable final observations cannot establish which keys remain.
File type, ownership, link-count and size refusals remain errors, as do stable unreadable project tables.
The remaining configuration trees use path, kind, size and mtime metadata only; file contents and link targets are never read or hashed.
Installation subtrees named `node_modules`, `extensions`, `marketplace`, `marketplaces`, `bundled`, names containing `cache`, and code directories directly beneath `plugins` are excluded; plugin registry files remain observable.
The inventory stops at 50,000 scanned entries and reports partial coverage, a lower bound on omitted entries and the number of uninspected subtrees, without walking the remainder just to count it.
The excluded-boundary count includes explicitly listed roots inside an excluded tree and is not a deduplicated count of physical installation trees.
An addition or removal is reported only where the opposite inventory observed absence, so partial coverage cannot manufacture a change.
Metadata changes outside the known file list are named and preserved without failing the run or attempting restoration.
The native sandbox explicitly protects nonshared declared configuration/trust files, operator Hide/Herdr routing, the installed app and this checkout's control files, including its external Git metadata, the owner-family registry, candidate and selected CLI executables, wrappers, backups and evidence.
Protected ancestors cannot be renamed or removed; the private probe/temp/socket trees and exact issued claim files remain writable.
Both lexical entrypoints and resolved targets are protected, including symlinked CLI installations and inherited custom Herdr sockets or XDG configuration roots.
Native account and session state, including shared project bookkeeping and Claude's session environment, remain writable under the recorded native-state policy.
There is no universal OS guarantee that files outside the probe cannot change: harmless probe instructions constrain requested actions, and the bounded metadata inventory observes only its recorded roots and exclusions.
Existing and new native history changes remain named where that inventory covers them; the profile never enumerates session files.
Claude's actual exec wrapper removes `CLAUDE_CONFIG_DIR` without replacing HOME and sets its supported updater, telemetry and error-reporting disable controls; this is not proof of zero outbound activity.
The command cap remains 15 seconds regardless of history count.
Unexpected or concurrent changes to nonshared known configuration/trust files are preserved and fail the run; observing a diff never authorizes overwriting the operator.
Only the test lane's exclusively owned disposable HOME permits attributed restoration.

Delivery is verified only by a fresh marker sent through the private hided mailbox and then found in the native assistant reply to the turn the line hided's own doorbell typed opened.
The controller types nothing for it: a bell is recognized only by the line the doorbell kept on the letter, which the tool reads back with `hide request show`, so the delivery window adds the doorbell's 30-second quiet period (`QUIET_MS`) to the scene seconds.
The letter's first line carries `@`, `$`, `/`, `!`, `#`, backticks, Korean and a trailing backslash, so a verified delivery also shows that line arrived as one submitted prompt.
The tool's agents run Herdr's integration hooks, not Hide's, so the letter reaches them through `hide inbox`, which their probe guidance says to read when a `🦉` prompt brings none; that a prompt hook recognizes the line and hands the letter over is what the remote mailbox lane and the manual checks in [delivery.md](delivery.md#verification) prove.
The controller generates the marker after the earlier prompt and holds it only in memory; it writes no helper or marker receipt for the agent to read.
The native sandbox denies other-process argument inspection and private mailbox storage access; its exact candidate capability and claim files remain available for ordinary `hide inbox`.
Of the run root only the folder itself is readable, its metadata and its names, so native settings readers can vet every ancestor of a private probe path and Bun's `realpath` can open it: OpenCode resolves its project folder that way, and on 2026-10-18, with only the metadata readable, it fell back to HOME and recorded its sessions there.
The run root's folders and controller-file contents remain denied, and the pre-provider self-test checks both denials alongside ancestor metadata access.
For omp's credential database, the tool queries the system's open-file table and copies the DB, WAL and SHM together only when no opener is observed and every source's identity, size, mtime and bytes remain unchanged.
It never opens the original through SQLite or checkpoints it; any recovery happens in the private copy.
An open or changing bundle leaves all omp scenes UNKNOWN without launching omp, prints guidance to close all omp instances and rerun, and lets the other providers continue.
Alias, integrity and copy-budget failures remain fatal.
The opener queries and file checks are observations rather than an atomic lock against another process opening the original later.
Arrival and post-bell observation keep separate windows, starting after their respective input completes.
Each phase has a fixed budget of the 15-second transport allowance plus `--scene-seconds`, clamped to the remaining run budget; history seeding and delivery observation use the same formula.
The original observation-only window excluded transport, so the one-second synthetic window now has a 16-second phase and a ten-second native window has a 25-second phase.
Timely scene arrival remains reached when only the later effect phase expires; its effect is ambiguous (UNKNOWN), which never counts as safe.
Positive effect evidence received after the phase ends is rejected, while safety errors, cancellation and resource-limit failures still propagate.
The JSON and human-readable reports record both components and the total; the per-command cap remains 15 seconds.
Within a window, reads and resulting input share its absolute deadline through process admission and retain the existing per-command cap.
Late replies cannot certify arrival, safe effects, previous-session history or model delivery; a received scene frame remains in its evidence even when the next deadline check refuses it.
The owned pane claims and verifies its exact persistent reference before the native CLI starts, preventing unclaimed-reference expiry during a long scene.
The Info request used for that claim may return the explicit `renderer_unavailable` refusal in this headless run; the daemon validates the capability first, and the CLI must still complete the claim and leave its claim file.
Other refusals and missing claim files fail the measurement.
Typed text, hook installation, a tool's output and transport arrival alone are insufficient.
Unknown native conversation formats, permission holds and unavailable integration remain unknown delivery.
No unavailable CLI or unauthenticated account is a safety PASS; the table retains its login instruction without attempting login.

Exit 1 identifies an unsafe scene for a source-declared bell target; exit 2 reports isolation, configuration, resource or cleanup failure; exit 3 reports incomplete measurement; exit 0 means the selected rows were fully measured without a target safety failure.
A result applies only to its recorded versions, manifests, model, environment and reached scenes.
It does not prove a different provider/model/version, a different remote manifest, or release readiness.
Report the actual native result and its unknown/skipped cells in PR Evidence; never commit these run artifacts.

CI runs `web/e2e/agent-live-check.spec.ts` with the existing compiled Claude/Codex shim on real pinned Herdr and candidate hided.
The fixture report is marked synthetic and proves tool plumbing, not authenticated model behavior.
That lane includes operator-routing refusal, byte-exact configuration restoration, runtime failure, owned-server loss and Ctrl-C cleanup.
`scripts/tests/test_agent_live_protection.py` exercises real filesystem recovery/conflicts, OS denial, timeout/output caps and owner-death cleanup; `test_agent_live_results.py` challenges unsafe verdicts and false delivery evidence.
The scene preparation and overlay tests cover supported MCP formats, positive native identity, role separation and bounded private authentication copies.

## Agent journey

`python3 scripts/agent-journey-check.py` follows each supported agent through what an operator does with it in Hide, on an isolated candidate, and says per agent and step whether it worked.
It is the check a release runs on the Mac before its tag ([CONTRIBUTING.md: Releases](../CONTRIBUTING.md#releases)), and the one to run after a change to how Hide starts, wakes or writes to an agent, after moving the pinned Herdr and after a substantial agent CLI update.

### What it proves

Each agent is started only through `hide agent spawn --parent here` from a controller pane, in its own branch, worktree and tab of the measured repository, and each step is judged from what Hide itself reports: the daemon's snapshot rows, `hide request show`, Herdr's `agent list` and Hide's own conversation reader (`hide_session::read_conversation`, built as the `journey_conversation` example).
Every prompt asks only for a fresh token (`Reply with exactly <token>`), so a model never chooses an action.
The measured repository is a shared clone (`git clone --shared`) of this checkout's commit, on `main`, at `projects/measured` in the candidate's HOME, added as a project the way Add project adds one, and the journey starts its agents only once the navigator lists it as a registered Git project.
An operator starts agents in a project they added inside their HOME, and Hide adds none outside it (`outside_home`); a Wake after a Sleep that closed the pane (Grok, Pi, omp, OpenCode) finds the agent's checkout among that project's, so a repository outside the candidate's HOME left those Wakes with no checkout to open.
The clone reads this checkout's objects and writes only its own, so the run's branches and worktrees never reach the operator's repository; each agent's folder trust names the clone.
The private pane's `.zshenv` saves its incoming PATH and `.zshrc` restores it after system login profiles run, preserving each pane's provider directory and keeping the run's wrappers before system programs.
macOS's `path_helper` otherwise moves system folders ahead of them, so an installer's `/usr/local/bin` link can bypass the wrapper's private HOME and fake key even though the preflight version check passed.
`test_agent_live_bootstrap.py` checks the program actually executed by both login and non-login shells against a private executable that shares a system command's name.

| Step | Passes when |
| --- | --- |
| `start` | The spawn answers registered, running and connected with a native session id, Hide's reader holds the answer to the first prompt and the row reads idle later than that answer's session time, and Herdr's integration reports that session as the contract's `herdr_source` (`herdr:<integration target>`, `herdr:antigravity_cli` for agy); whether the row read working is reported, not required, since Herdr's screen detection misses a turn of about two seconds |
| `reply` | Hide's reader finds the first turn's token in an assistant message; a reply only another parser finds fails |
| `letter` | A token sent with `hide request send` comes back in the turn the letter reached, `hide request show` says delivered, and the row reads idle after that answer; the turn is the one Hide's doorbell opened for an agent with a bell (Claude Code, Codex, Grok), and for one with a prompt hook and no bell (agy, Pi, omp, OpenCode) the next prompt the journey types, whose hook hands the letter over and confirms it (`hook_confirmed`) |
| `sleep_wake` | After the daemon's Sleep event the row reads asleep, and after Wake it runs again: in the pane it slept in for an agent Hide wakes there (Claude Code, Codex, agy), and for one whose Sleep closes its pane (Grok, Pi, omp, OpenCode) once `wake_sleeping_session` opened a new pane that hosts the same session and its registration |
| `resume` | The operator opens the woken agent (`focus_pane`), its terminal takes this screen's keys, a typed token prompt is answered in the session it slept in, the row reads idle after that answer, and Herdr reports the same session id |
| `start_untrusted` | A start in a repository the account never trusted, with no preparation, passes once Hide has recorded the trust of the worktree it made |
| `integration` | The candidate's kit row in the snapshot reads Hide's hook and Herdr's integration installed (the kit calls Herdr's current) at locations inside the HOME the agent reads its configuration from, and Herdr reports the agent's session as its `herdr_source` |
| `guidance_memory` | A rule card the run wrote into its private state before any start (`hide-memory`'s `journey_memory` example) carries a fresh token in its title, and a turn asked `Run no tool. Reply with exactly the uppercase code in the title of your only Project Memory card.` is answered with it; the question shares no word with the card, so the card came from the start's index and not from prompt-time Memory |
| `fork` | After the journey sends Fork agent's `fork_pane` as the pane menu does and types one prompt into the fork's pane, as the operator does to use it, a row whose `forked_from` names the source shows a session of its own, Herdr reports that session as its `herdr_source`, Hide's reader finds the answer in it, and `hide agent list` registers the fork with no parent and the source as its origin; an agent that tells its session only at its first turn, as Codex does, is registered as a fork only after that prompt |
| `title` | The row's `identity_label` reads a title other than the agent's name (the contract's `untitled`); whether Hide's label sentences reached the row is reported beside it |
| `usage` | With the usage window hint the shell sends while it is on screen, Hide's usage reads the agent's consumption with model calls in it; the run's HOME holds no other session, so they are the run's |

The journey runs them in this order, each from the state the last left: `start` and `reply`, `integration`, `guidance_memory`, `letter`, `fork` (before Sleep, while the source still runs the session it copies), `sleep_wake`, `resume`, `title`, `usage`, then `start_untrusted`.
Only `start`, `start_untrusted`, `guidance_memory`, `letter`, `fork` and `resume` start a turn; the others judge what Hide shows of them and spend no model request.
The journey closes each pane no later cell types into: a fork's after the `fork` cell, the agent's own before `start_untrusted`, and the untrusted start's before the next agent starts.
The run's process guardian ends the private Herdr when everything under it passes 2 GiB together: two Claude Code processes left running beside a Codex fork crossed it, and so did omp's fork, its woken session and its untrusted start held at once on Linux (2.17 GB, 2026-10-18).
A crossed memory cap names the five live processes holding the most, and the report's cleanup failures carry the reason a guardian (on Windows, the job) ended Herdr or hided, so the cells that fail after it with `herdr_command_exit_1` have their cause in the same report.
The cells an adapter declares, and its letter and wake forms, come from `contracts/agent-journey.json`, which `hide-agent-adapter` generates and its contract test keeps current ([contracts/README.md](../contracts/README.md#agent-journey)).
Grok's letter uses the `bell` path: the bell wakes it at rest, its prompt hook recognizes that turn without printing context, and its first tool hook or Stop hook delivers the letter body.
The journey judges its reply in that bell turn without asking the model to choose a tool.
It does not prove the desktop window, a physical keyboard, an agent other than the declared rows, or a model, provider or CLI version other than the ones its report names.

### Cells, reasons and exit codes

Every cell reads `pass`, `fail`, `not measured`, `N/A` or `excluded`, with a reason and the evidence file under the run folder; a failed cell carries the product's own refusal or the state that never came with the time waited, and the agent's pane capture.
- `N/A` is a cell the adapter does not declare.
- `not measured` is a cell the run could not reach without touching the operator's state or spending what it must not: a CLI that is missing or signed out (the journey never signs in), a Codex login copy older than 7 days, an OAuth login copy whose access token expires before the run's deadline or within ten minutes of it (Grok, Pi, omp; its refresh inside the copy could sign the operator's own agent out, and Pi refreshes five minutes before expiry), so Pi's OpenAI login, whose access token lasts an hour, is measured in a run of one or two agents, a login another process holds open, a repository Claude was never trusted in, a cell whose earlier step failed or was not measured (`prerequisite cell failed (<cell>)`, `prerequisite cell not measured (<cell>)`), a stopped run, an agent a lane left out with its reason (`--unmeasured ID=REASON`, agy's quota rotation), and `not built in this cycle`.
  Claude's never-trusted start is `not measured` locally, since it would leave an entry in the operator's shared `~/.claude.json`; CI mode measures it in its disposable HOME.
  Claude's `usage` is `not measured` locally too: Claude writes its sessions under the operator's HOME for its keychain sign-in, and the candidate's usage ledger reads only its own HOME.
- `excluded` is an agent the operator took out of the journey, with the date: Cursor, `excluded: operator decision 2026-10-17`.
  `--agents cursor` is refused, and an excluded cell counts toward no exit code and no release's list of blanks.

Exit 0 means every measurable selected cell passed, 1 a failed cell or a request budget overrun, 2 an isolation, setup or cleanup failure (no agent starts when isolation cannot be proven), and 3 no failure but a `not measured` cell.
The controller's workspace bootstrap waits at most 30 seconds, including its pane commands, for `pane_not_connected` or `caller_unavailable` to clear in its newly created owned pane; the latter is the checkout fallback's answer when it cannot read the caller's directory, including on Windows before the pane is connected.
Every other refusal or unreadable answer fails setup immediately; the report names `private_checkout_not_registered: <reason>`, with the last refusal when the deadline ends, rather than a JSON parse error.
Each pane command checks its exit status before accepting success, bounds stdout and stderr separately at 1 MiB, and removes both private streams after reading, including on failure; only reason identifiers reach the report.
The bootstrap's JSON evidence records acceptance, attempts, elapsed seconds and the last refusal, never the private reference or raw output; elapsed time includes shell transport and is an upper bound on connection readiness, not the instant the daemon connected the pane.
The reference-ready marker carries explicit completion content, and pane transport files, including the controller's completion marker, share one cleanup path that waits at most five seconds for Windows sharing or lock violations to clear; other filesystem errors fail immediately.
Ctrl-C, SIGTERM and SIGHUP stop at the next step, settle the open cells as stopped, write the report and clean up.
The whole run, every process under it included, is capped at 5 minutes plus 10 for each measured agent when no provider answers (the fake model or the stand-ins; seven agents: 75 minutes, inside a CI step's 85, where Windows took 60 to 62 minutes on 2026-10-18 and ran past the earlier 61) and plus 15 when one does (signed in or local, whose turns are slower and whose title the journey waits up to 120 s for; seven agents: 110 minutes, inside the signed-in CI step's 115); past it the open cell fails with `run_cancelled_or_timed_out` and the run cleans up. Each child's guardian ends it at that cap, not 30 minutes after the child's own start, the live check's cap, which on 2026-10-18 ended Herdr and `hided` under the seventh agent.
The table is printed and written as `report.md` and `report.json` in a new `agents/runs/agent-journey-<time>-<id>/`, with each agent's spawn arguments, the arguments and environment names its mode added, its model requests and its budget.

### Local mode (macOS)

Build `hided`, `hide` and the hooks with `bash scripts/verify-cargo.sh cli` and the reader with `bash scripts/verify-cargo.sh build -p hide-session --example journey_conversation` and the Memory card writer with `bash scripts/verify-cargo.sh build -p hide-memory --example journey_memory`, fetch the pinned Herdr with `scripts/fetch-herdr-runtime.sh`, then run it from the worktree; `--agents claude-code,codex` narrows the rows and `--model ID=MODEL` passes a model through the product's own flag.
It reuses `scripts/agent_live_check`'s isolation: the pinned Herdr on a private socket, this worktree's `hided` on private state with the kit installed into a private daemon home, the write sandbox and the process guardian.
The daemon starts without `XDG_STATE_HOME`, like an app opened from the Dock, while Herdr's shell inherits private XDG folders and no blanket `HIDE_STATE_DIR`.
CI uses the daemon home's default `.hide/state`; local mode names its state outside the sandbox's writable probe explicitly for the daemon and agent wrappers.
The kit captures that state folder in every installed hook and extension, so a native start whose hook keeps only HOME, the shell's XDG folder and its pane identity still reaches the candidate.
The regression at the helper's process boundary removes all inherited Hide routes for every supported hook transport and requires the captured folder to deliver its letter.
Claude keeps the operator's HOME for its keychain sign-in and reads only the candidate's settings, through the arguments the pane's `claude` adds to every start, Hide's own Wake and Fork restarts included; Codex reads a private `CODEX_HOME` holding a 0600 copy of its login, removed at the end, and the measured repository's trust, written only there.
Grok and Pi keep the operator's HOME too and read a private folder of the daemon home through the variable each documents for it (`GROK_HOME`, `PI_CODING_AGENT_DIR`), holding a 0600 copy of the login and, for Grok, the measured repository's folder trust.
OpenCode keeps the operator's HOME too and reads the daemon home's folders: its configuration from `.config/opencode`, where the kit puts Hide's plugin beside Herdr's whatever `XDG_CONFIG_HOME` says, holding an `opencode.json` with only the operator's model choice and provider settings, so their MCP servers and permissions stay theirs; its sessions and a 0600 copy of its `auth.json` from the probe's own `XDG_DATA_HOME`, the folder the daemon and Hide's reader read them from.
Naming a different data folder for the agent than the daemon has is a harness bug, not a product finding: on 2026-10-18 it made the reader answer `session_file_missing` for a session OpenCode had stored.
OpenCode 2 runs every terminal's sessions in one background server that outlives them, on the port its `service.json` names, where the operator's server may listen; the journey names a free port in the private `service.json` in every mode, marks the agent's environment (`HIDE_JOURNEY_OPENCODE`, which the server inherits), counts the marked servers after Sleep and Wake (more than one is a product finding), stops the agent's server once its cells are settled, so it holds no memory under the run's 2 GiB cap while the later agents run (left running, its 0.3 GB put omp's fork past the cap on Windows, 2026-10-18), and on every exit path stops any still running with OpenCode's `service stop`, admitted past the run's deadline and a cancellation as the rest of the teardown is, and then a signal to any still running, before the run's owner closes; the owner's close then ends whatever the run owns, the server included (on Windows, where no other process's environment can be read to signal it, the server is a member of Herdr's job), and a server still answering after that close fails the run.
agy is `not measured` locally: it reads its hooks only from `~/.gemini/config` in the HOME its keychain sign-in needs, so the candidate's kit cannot reach it without the operator's own hooks running; CI mode measures it.
omp is `not measured` locally: it names a checkout's session folder from its own HOME, which local mode keeps as the operator's, while the candidate daemon has a private HOME, so Hide's reader names another folder for the same checkout and refuses the session (`label_session_default_directory_required`); in the product hided and omp share one HOME, and the fake model and CI lanes, where they do, measure it.
The candidate's Hide AI is off in local mode (`enabled: false` in the private home's `ai.json`), since it would run the operator's signed-in CLI with the private HOME; a request from it fails the run (exit 2).
In CI mode it stays on, and the report counts its model requests by provider apart from each agent's budget.
The journey writes no operator file; the entries Claude itself leaves for the run's paths are left alone and listed: in `~/.claude.json` by key, in `~/.claude/projects` by exact folder path.
No provider key in the operator's shell reaches an agent: every variable whose name ends in `_API_KEY`, and `ANTHROPIC_AUTH_TOKEN` and the cloud credentials, is left out of the run's environment, so an agent reaches a provider only through the login or key its route gives it (omp 18.7.0 had sent its background requests to Google with the shell's `GEMINI_API_KEY`).
Hide records folder trust for each worktree it spawns an agent into, and must record Codex's only in the private `CODEX_HOME` the agent reads: before the cleanup and after it the journey reads the operator's `~/.codex/config.toml` and `~/.claude.json` for trust entries naming a path of the run.
A Codex entry there is an isolation failure (exit 2); a Claude one is reported by key, and one left after the cleanup is listed as a product finding.
Every exit, a stop and the deadline included, ends the agents, panes, Herdr and `hided` and confirms it, and then removes the probe, the clone with the run's `journey/<run-id>-<agent>` branches and their worktrees in it; while an agent's end is unconfirmed the probe stays.
Never run a cargo build or test while it runs: either relinks `target/debug` under the running `hided`.

### The fake model (`--fake-model`)

With `--fake-model`, in either mode, each real CLI with Hide's real kit and Herdr's integration is answered by the journey's fake model (`scripts/agent_journey/fake_model.py`) instead of a provider, and no login is supplied to an agent or a provider.
It speaks OpenAI Chat Completions and Responses, Anthropic Messages and Gemini `generateContent`, streamed or whole, with usage in each provider's shape, and answers a turn with the last token of what the request sends after the model's own last turn, a token inside Hide's Project Memory blocks left out, or with the token of the Project Memory index the session started with when the turn carries none; a request that asks for JSON of a schema, as Claude Code's and Codex's title requests do, is answered with an object of that schema.
`scripts/agent_journey/fake_lane.py` writes each CLI's provider configuration into the run's daemon home, and each agent gets its own server, so its log counts that agent's model requests exactly.
Local fake mode uses a private HOME only for an adapter whose four guards can be enforced; real-login mode always keeps the operator's HOME.
Build its layout reader first with `bash scripts/verify-cargo.sh build -p hide-kit --example journey_cache`.
The local fake runner installs only the selected CLIs from `contracts/agent-clis.json` into `~/.hide/cache/journey-clis/`, whose path and 4 GiB total cap are owned by `hide_kit::layout`.
Each entry is keyed by the CLI pin, installer specification, operating system and architecture; later runs reuse its receipt and verify the executable digest and answered version.
The cap includes in-progress installations and cached downloads; crossing it fails with exit 2 and asks the operator to remove unused pins, never silently evicts another run's CLI or increases the cap.
A lease holds the cache until the run's children have ended, so another local fake run refuses a busy cache.
An unconfirmed child cleanup leaves a cache marker that refuses reuse until that run has been inspected and the marker cleared.
A failed installation removes its incomplete pin after confirming its children have ended.
An uncatchable kill can leave an incomplete pin that must be removed before retrying.
Native executables linked into the installer's temporary HOME are retained in the pin before that HOME is removed.

The four guards are enforced before or during every admitted agent's journey:

1. The executable comes only from the pinned cache, never the operator's PATH or aliases.
   The installer itself has a private HOME and a write sandbox permitting only that cache entry; the agent's controlled PATH contains the candidate wrappers, the Node runtime and system tools.
   Before installation, that exact sandbox must allow a cache write and deny both a write and a link in a disposable fixture outside the cache; otherwise installation fails.
   Grok's installer receives its target directory on PATH so it does not attempt additional links in `~/.local/bin` or `/usr/local/bin`; the sandbox still denies every write outside the pin.
2. The Rust adapter declares how automatic updates are disabled, projected into `contracts/agent-journey.json` and applied before any CLI starts.
   The agent's write sandbox also denies changes to the cached program, operator agent folders and keychain files, and permits model traffic only to loopback.
3. The run inspects the following operator watch at its start and after cleanup for traces of this run: its directory, fake server addresses with ports, and a per-run fake key (including the tail Claude stores).
   A trace in a watched file, selected database row or new top-level entry, or a watched link pointing inside the run, fails with exit 2.
   Content changes without those markers, including concurrent token refreshes and temporary entries, are reported as external changes and do not fail.
   Run paths include HOME-relative `~/` spelling and macOS `/var` and `/tmp` aliases, in plain or JSON-escaped form; a path boundary excludes similarly named sibling runs.
   Fake server addresses include both `127.0.0.1:PORT` and `localhost:PORT`, with a boundary excluding longer ports.
   A configuration file moving during a read retries at most three times; continued movement still fails with exit 2.
   Unreadable marker checks still fail with exit 2.
   Reports retain fixed item names and pass/fail only, never hashes, link targets, entry lists or configuration/login bytes; the watch never restores or changes operator files.
4. The adapter declares an API-key environment route, the provider configuration names that variable, and the fake server accepts a model request only when it carries that key.
   Missing or rejected key evidence stops the run with exit 2 after the start attempt; reports record only environment names, authenticated request counts and source references.

The watch covers the `.local/bin/claude` link, the configuration/login files below, and the top-level names under `.claude`, `.codex`, `.gemini`, `.grok`, `.omp`, `.pi` and `.local/share/opencode`.
Existing session/history append records are not compared.
New top-level entries are scanned recursively for run markers, within a shared budget of 50,000 entries and 1 GiB per snapshot; exceeding either cap fails the run.
An entry disappearing during enumeration or open, and a socket, FIFO or other special entry, contains no file trace and does not fail the watch.
Internal snapshots retain hashes and names in memory only.
The private lane admits only the process environment basics named in `PRIVATE_ENV`; provider, package-manager and agent settings come from its explicit route.
Keychain file access is denied, but access through the securityd Mach service has not been verified; no operator keychain item is queried as a probe.

The exact configuration/login paths, relative to the operator's HOME, are:

| Agent | Configuration and login paths | Source for the file boundary |
| --- | --- | --- |
| Claude Code | selected login/install keys and run-project paths in `.claude.json`; `.claude/settings.json`, `.claude/settings.local.json`, `.claude/.credentials.json` | [Official settings guide](https://code.claude.com/docs/en/settings), configuration files and authentication storage; keychain files are denied by the sandbox; securityd access is unverified. |
| Codex | `.codex/config.toml`, `.codex/auth.json`, `.codex/hooks.json`, `.codex/.credentials.json` | [Pinned core sources](https://github.com/openai/codex/tree/rust-v0.163.0/codex-rs/core/src), config, auth storage and hooks. |
| Grok | `.grok/config.toml`, `.grok/auth.json`, `.grok/trusted_folders.toml`, `.grok/mcp_credentials.json` | [Official settings reference](https://docs.x.ai/build/settings/reference) and [published authentication source](https://github.com/xai-org/grok-build/tree/2bdd1d6a6369de0e8c68132ea4539e9abd9e14a8/crates/codegen/xai-grok-login); the public source is 1.0.45, without a tag for the selected 1.0.52 CLI pin. |
| Pi | `.pi/agent/settings.json`, `models.json`, `auth.json`, `oauth.json`, `mcp-auth.json`, `keybindings.json`, `trust.json`, `mcp.json` in the same folder | [Pinned package](https://www.npmjs.com/package/@earendil-works/pi-coding-agent/v/1.0.4), `dist/migrations.js`, `dist/core/auth-storage.js` and `dist/extensions/mcp/oauth.js`; startup can migrate the legacy OAuth and keybinding files. |
| omp | `.omp/agent/config.yml`, `.omp/agent/config.yaml`, `.omp/agent/models.yml`, `.omp/agent/settings.json`, `.omp/agent/secrets.yml`, `.omp/agent/secret-placeholder.key`; credential and settings rows of `.omp/agent/agent.db`, read from a private copy | [Pinned paths](https://github.com/can1357/oh-my-pi/blob/v18.7.0/packages/utils/src/dirs.ts), [settings storage](https://github.com/can1357/oh-my-pi/blob/v18.7.0/packages/coding-agent/src/session/agent-storage.ts) and [credential storage](https://github.com/can1357/oh-my-pi/blob/v18.7.0/packages/ai/src/auth/sqlite-credential-store.ts). |
| OpenCode | `.config/opencode/opencode.json`, `opencode.jsonc`, `tui.json`, `cli.json`, `service.json` in the same folder; credential rows of `.local/share/opencode/opencode.db`, read from a private copy | [Pinned credential schema](https://github.com/anomalyco/opencode/blob/v2.0.29/packages/core/src/credential/sql.ts), [database path](https://github.com/anomalyco/opencode/blob/v2.0.29/packages/cli/src/database-path.ts), [configuration migration](https://github.com/anomalyco/opencode/blob/v2.0.29/packages/cli/src/config/migrate.ts); v2 reads the old `auth.json` only for migration. |

Claude Code's mixed global file is scanned for run markers, including normalized absolute project paths.
Its login/install keys below are compared in memory to identify external changes; unrelated project entries and live counters do not produce external-change notices.
Every report uses the same privacy boundary: fixed item names and pass/fail, with external changes named separately.
Copies are kept only inside the run and removed immediately after each database read and again at unconditional cleanup.
A later run reports a leftover `watch-private` directory after an uncatchable kill; it never reuses it.

The selected keys are `oauthAccount`, `primaryApiKey`, `customApiKeyResponses`, `installMethod`, `autoUpdates`, `hasCompletedOnboarding` and `lastOnboardingVersion`.
The whole selected object is hashed canonically in memory, including the absence of a key; evidence retains only the compared key names and pass/fail, not values, project paths or hashes.
Claude Code is closed source: these names follow its configuration/onboarding format, while its official settings documentation is live rather than version-pinned.
Every absolute `projects` key is checked against the run directory, which contains the private HOME and both worktree roots; any such key at the end fails even if it was already present at the start.

OpenCode is also covered: the top-level entry names of `~/.local/share/opencode` and its configuration/login rows are watched.
The six original roots, including `~/.gemini`, remain watched.
These are the standard home paths; candidate routing overrides are stripped or point to its private home.
For omp, the comparison hashes `settings(key, value, updated_at)` ordered by `key`, and `auth_credentials(id, provider, credential_type, data, disabled_cause, identity_key, created_at, updated_at)` ordered by `id`.
For OpenCode, it hashes `credential(id, integration_id, label, value, connector_id, method_id, active, time_created, time_updated)` ordered by `id`.
Usage counters, refresh leases and session records are excluded.
The reader copies each raw database and any WAL to a private run folder, checks their identity, size, mode and modification time before and after copying, opens only that copy with SQLite, and removes it before reading the next database.
On macOS, the already-open source descriptor is cloned with `fclonefileat`; unsupported or cross-volume cloning falls back to a 1 MiB chunked copy, without streaming the cloned database to hash it.
The database plus WAL has a fixed 1 GiB cap, separate from the 16 MiB cap for configuration bytes and returned credential/settings rows.
The larger database cap accounts for session records sharing the database; those records are never returned or hashed into the comparison.
Only one database bundle exists in the private copy directory at a time, and an out-of-space failure reports its errno with exit 2.
It never opens the operator database with SQLite, including read-only mode, and never copies or creates its SHM.
A concurrent change during copying retries the whole bundle at most three times; continued movement, an unreadable schema, a corrupt queried row or a crossed copy/row budget fails with exit 2.
Only the selected row queries run; a whole-database integrity scan is unnecessary for this watch.
Top-level entry enumeration and database row reads use the existing configuration-file count cap.

Claude Code, Codex, Grok, OpenCode, Pi and omp declare these routes.
Their update policies are respectively `DISABLE_AUTOUPDATER`, `check_for_update_on_startup = false`, `GROK_DISABLE_AUTOUPDATER`, `update: disable`, `PI_OFFLINE` and `startup.checkUpdate: false` (omp checks for a version but does not automatically install it).
agy keeps the local exception and is not started: its installer cannot select the pinned release; CI's disposable runner still measures it with its existing version check.
Cursor remains excluded by operator decision.
A missing key or update declaration, or a non-version-selecting installer, keeps an adapter on its existing local fake route with the operator HOME, and the report names the exception.
This preserves existing coverage: a route already measured still runs, while an existing local limitation such as agy remains `not measured`.
The private fake sandbox denies reads of the operator's `~/.agents`, including metadata reads.
Pi 1.0.4 searches a project's ancestors for `.agents/skills`, excluding only the current HOME's skills; without this boundary, a changed HOME makes the operator's skills look like project resources and adds an unrelated trust question.
Operator skills must neither enter a fake run nor alter its folder-trust measurement.
The fixture seeds no Pi trust decision, and the untrusted-start path remains untrusted.
Private fake runs use the bounded before/after watch above instead of the real-login route's trust-entry inventory, which includes project paths.
The candidate's Hide AI is off in both fake modes, and native titles are what the title cell reads.
The macOS fake CI lane temporarily records Grok's authentication method, key-variable name/presence, and wrapper HOME/config paths relative to the run to distinguish a key-route failure from bypassing the wrapper.
It streams the native debug log through a private FIFO, retains only the selected facts, and removes the FIFO at cleanup; the reader has a 32 MiB input cap, a 4 MiB line cap and a 64-event cap.
The configured route and the native model-config observation are separate evidence; this diagnostic does not trace filesystem opens.
`scripts/tests/test_agent_journey_private_home.py` exercises content, top-level entry and link changes on fake watch roots, concurrent SQLite copies, unchanged source bytes, missing proof, cache refusal and authenticated requests; no test watches or changes the operator's folders.
The report is marked as a fake model run with what a fake model cannot show: it proves the CLI, Hide's kit and Herdr's integration, never a provider, a login or a quota, and its rows are kept apart from a signed-in run's.

### The CI lanes

`.github/workflows/agent-journey.yml` runs the journey with `--mode ci`, where each agent runs in the runner's disposable HOME with no sandbox: the stand-in lane on Linux and macOS, the fake model and signed-in lanes on Linux, macOS and Windows.
Every lane that runs a real CLI installs it with `scripts/install-agent-clis.py` at the version `contracts/agent-clis.json` pins, the one place a CI agent version is written (`agent-trust-contract.yml` installs from it too); npm installs run no package's install scripts but those a row names (Claude Code's and OpenCode's, which place their native program); agy's installer takes no version, so its row says so and the version it answered is checked against the row instead; a CLI that answers another version than its row fails the install and the lane, so a vendor release moves the pin in a pull request, which plans the `agent CLIs` lane (`agent-clis.yml`, every pinned CLI installed on Linux and Windows) before any lane measures it.
The real-CLI fake-model lane passed all declared cells of all seven selected agents on Linux, macOS and Windows in [run 39152230964](https://github.com/with-hide/hide/actions/runs/39152230964), 2026-10-19 KST (2026-10-18 UTC); [INSTALL.md](INSTALL.md#what-each-system-supports) records the measured support and remaining gaps.
On Windows the `windows forms` job also runs `scripts/tests/test_agent_live_windows.py`: an owner that dies takes its children's trees with it, every cap is the Unix guardian's and a crossed one is a named failure, private files admit only this account, the pane commands and launchers keep their arguments and exit status, and the run's folder leaves the account's Path as it found it.
A pane Herdr opens on Windows takes its PATH and temporary folder from the account's registry, not from Herdr's own environment, so a Windows CI run puts its agents' wrappers first on the runner account's own Path while it runs and takes them out first in its cleanup, on every exit path, and each wrapper sets the agent's temporary folder itself; the change is refused anywhere but a GitHub Actions runner (`GITHUB_ACTIONS=true`), so a local run never touches an operator's registry, and a Path the cleanup could not restore leaves the cleanup unconfirmed (exit 2).
Every CI lane builds the controller's spawn provider (`web/e2e/shims/spawn-provider.c`) before the run and passes it with `--controller-provider`, as it builds the stand-ins, since a Windows link can outlast the guardian's 15-second command cap; a run by hand builds it itself, and a command asked to run past the cap is refused rather than shortened.
The journey reads what hide, Herdr and the agents write as UTF-8 and refuses to start outside Python's UTF-8 mode: the workflow sets `PYTHONUTF8=1`, and a run by hand on Windows is `python -X utf8 scripts/agent-journey-check.py ...`.
- The stand-in lane (`fixture`) puts the compiled `web/e2e/shims/claude-shim.c` in place of Claude Code and Codex with `--fixture-bin` and passes only when both pass `start`, `reply`, `integration`, `letter`, `sleep_wake` and `resume`, and fail none; `guidance_memory`, `fork`, `title` and `usage` read `not measured`, since the C stand-in imitates none of them.
  The stand-in runs the hook commands the kit installed and writes each CLI's session file format, so a change that breaks the journey's verdicts, the hooks' path into Hide or the cleanup fails it; its report is marked `fixture` with what a stand-in cannot show, and is never a release's agent evidence.
  `pr.yml` runs it on Linux, with `windows forms`, when the journey's code, the live check or the shims change (`scripts/ci-plan.py`), `nightly.yml` and the public `release-candidate.yml` every time.
- The fake model lane (`fake`, nightly and release) runs Claude Code, Codex, Grok, OpenCode, Pi, omp and agy with `--fake-model`, keyless, each against its own fake model, which answers at most the agent's request budget of model requests and twice that of every request, then answers 429 and the run fails the agent's budget, and which refuses a chunked or oversized body and closes its connection, and passes only when every built cell passes for Claude Code and Codex and no agent's cell fails.
  Run 39152230964 is green for those two and all other selected agents on all three systems; the workflow still runs the stand-in lane as harness coverage, and its synthetic results are kept apart from the real-CLI evidence.
- The signed-in lane (`login`, nightly and release) signs each agent in with the CI account's key: `HIDE_CLAUDE_API_KEY` as Claude's `ANTHROPIC_API_KEY`, `HIDE_CODEX_API_KEY` through `codex login --with-api-key`, `HIDE_AGY_API_KEY` as agy's `GEMINI_API_KEY` with `"modelProvider": "gemini"` in its settings, `HIDE_GROK_API_KEY` as Grok's `XAI_API_KEY`, and `HIDE_OPENCODE_API_KEY`, `HIDE_PI_API_KEY` and `HIDE_OMP_API_KEY` as OpenCode's, Pi's and omp's `OPENAI_API_KEY`; only the CLIs whose key is set are installed, and a leg with no agent to sign in, such as one where only agy's key is set on a night agy is measured elsewhere, builds and starts nothing and leaves a notice.
  agy runs on `gemini-3.8-flash-medium`, the model `agy-contract.yml` signs it in with, whose free tier allows 20 requests a day across the CI account and of which that lane spends 4 a night, so agy is measured on one system a night (the nightly's `agy: rotation`, through Linux, macOS and Windows by the UTC day of the year) and on Linux alone in a release, and reads `not measured` with that reason on the other legs.
  Operator decision, 2026-10-18: the repository keeps no CI secret for Claude Code or Codex (`HIDE_CLAUDE_API_KEY`, `HIDE_CODEX_API_KEY`), so their legs of this lane read `not configured` by design, and the fake model lane is what proves those two agents on CI.
  An agent without its secret reads `not measured: not configured (<secret>)` and leaves a notice; the key is kept under the run's probe, which the cleanup removes, and the report, each cell's evidence, each pane capture and the copy of Herdr's log have the key and the last 20 characters Claude's key question shows of it replaced; `hided` and Herdr never receive it.
  A failed cell, an isolation, setup or cleanup failure, a request budget overrun, or a refused login or a missing CLI for an agent whose secret is set fails the lane; a turn the provider answered nothing because the key's day was spent is `not measured: provider quota exhausted (<quota ids>)` with a `Not measured` warning, by the rule `contracts/provider-quota.json` keeps ([TESTING.md: A real agent on CI](TESTING.md#a-real-agent-on-ci)), and Claude Code and Codex have no row there yet, so every refusal of theirs, a per-minute rate limit included, fails.
  agy reports a spent day as the `AGY_ERROR:` line its print mode writes, and in an interactive session as an `errorreport.go` record of its own log under `~/.gemini/antigravity-cli/log` carrying the same Gemini SDK error (`Error 429`, `Status: RESOURCE_EXHAUSTED` and the `QuotaFailure` ids, run 39097738256); its transcript records no step for it.
  The journey reads either (`scripts/agent_journey/quota.py`), from logs written since that agent's journey began only, so a refusal an earlier run left in the same HOME never classifies a later failure, and a failed agy turn keeps those log records and its transcript's steps after the prompt, never their content, as the cell's evidence.
`scripts/agent-journey-lane.py` turns a report into the lane's verdict and its annotations, and the lane keeps the report as an artifact.
The journey alone runs by hand with the repository's secrets: `gh workflow run agent-journey.yml --ref <branch> -f systems='["linux","macos","windows"]' -f fake=true -f live=true -f agy=rotation`; a hand run's agy requests come out of the same 20 a day as the nightly's.

The final main measurement is [run 39152230964](https://github.com/with-hide/hide/actions/runs/39152230964), 2026-10-19 KST (2026-10-18 UTC), at `ac4a409ba`.
Each system's `report.json`, not just its job verdict, shows 76 passing cells, one `N/A` cell (Grok's undeclared `guidance_memory`) and eleven excluded Cursor cells, with exit 0, no failed cells, no request-budget failures and confirmed cleanup.
The six other selected agents pass all eleven cells on every system.
This supersedes the earlier Windows journey failures, OpenCode 2 fork gap and agy session/fork failure, as well as the macOS Grok letter failure of run 39134341492; those failures are not current support gaps.

| Agent | CLI measured | macOS | Windows | Linux | Native title source | Model requests per system / budget |
| --- | --- | --- | --- | --- | --- | --- |
| Claude Code | 2.1.302 | 11 pass | 11 pass | 11 pass | automatic: its own title request | 8 / 18 |
| Codex | 0.163.0 | 11 pass | 11 pass | 11 pass | automatic: its own title request | 8 / 18 |
| Grok | 1.0.52 | 10 pass, Memory N/A | 10 pass, Memory N/A | 10 pass, Memory N/A | automatic: its own title request | 19 / 20 |
| OpenCode | 2.0.29 | 11 pass | 11 pass | 11 pass | automatic: its own title request | 8 / 18 |
| Pi | 1.0.4 | 11 pass | 11 pass | 11 pass | named: `/name`, as an operator names a session | 6 / 18 |
| omp | 18.7.0 | 11 pass | 11 pass | 11 pass | automatic: its own title request | 8 / 18 |
| Antigravity | 1.4.2 | 11 pass | 11 pass | 11 pass | automatic: its own title request | 8 / 18 |
| Cursor | not run | excluded | excluded | excluded | not measured | not measured |

The `letter` evidence on every system shows one bell attempt and `hook_confirmed: true` for Claude Code, Codex and Grok, and next-prompt delivery with `hook_confirmed: true` and no bell for OpenCode, Pi, omp and agy.
Grok's bell turn is measured here; older reports whose `letter` is `not measured` remain unmeasured and cannot prove another letter path.
No `letter` cell sends during a running tool, so this run does not close the per-agent mid-turn gaps in [INSTALL.md](INSTALL.md#what-each-system-supports), or measure every hook event, picker or permission dialog.
The `start_untrusted` cells prove the prepared worktree starts, not how Herdr classifies an open folder-trust dialog; agy's bell gap remains.
The successful Windows `sleep_wake` cells do not cover a draft preventing a clean quit and forcing process termination; that separate gap also remains in INSTALL.md.
The signed-in lane was skipped in this run; the fake model proves no provider login, quota or billed usage, and the operator's decision not to supply Claude Code or Codex CI secrets remains in force.
Physical input and the desktop window are not measured.
Hide AI's goal title is a separate path this table does not measure: Hide AI is off in the fake-model journey (D-54).
Product follow-up: agy's first-run onboarding holds the first prompt of an agy that was never set up, as Hide's spawn of it on a new machine would meet it.

A full agy journey makes 8 model requests on the fake model lane (run 39097395133), so a night spends 12 of the key's 20 (`agy-contract.yml`'s 4 and the rotation's one leg) and a release day 20, the night's 12 and the release's Linux leg's 8; a hand run with agy, or a second release run, that day goes past it.
The key's day ends at 00:00 UTC (both refusals on 2026-10-18 asked to retry then), the day the rotation is counted by, so every leg of one run picks the same system and a run that crosses midnight starts a new quota day with the new system.
On 2026-10-18 hand runs spent the day: the signed-in legs after them read `not measured: provider quota exhausted`, as the rule says.

## A device check

`desktop/e2e/remote-workspace.spec.ts` covers remote routes and `desktop/e2e/device-kit.spec.ts` the install kit against an isolated SSH server; a check against a real device is for what those specs cannot reach.
That server logs in as the account running the suite and connecting with consent installs Hide's kit there, so its sessions must get a private `HOME` (`SetEnv` in its config); `desktop/e2e/device-home.ts` describes the setup and refuses to register a device until it has proved both.
`SetEnv` reaches the commands a session runs but not its SFTP, which spells `~/` from the account's own home, so the consent folders (`HIDE_HOST_HELPER_ROOT`, `HIDE_HOST_CLI_DIR`) must be absolute paths inside the private `HOME`: on 2026-10-09 a probe that kept the `~/` defaults uploaded its helper into the real account's `~/.hide/host-helper` while its commands ran in the private one.
The remote terminal probe (`hide-node/tests/it/remote_terminal_probe.rs`) and the device measurement (`scripts/web-shell-measure/device-guard.sh`) now refuse, before connecting, any alias that does not resolve to the private sshd's named port, a private HOME outside `/tmp`, and consent folders outside it; a core-role hided the measurement runs on the device (`MEASURE_SCENARIO=remote-core`, `core-host.sh`) is refused unless its program and state folder are inside that HOME.

- Start a private `herdr server` on the device with the same isolation variables, sent as a script over `ssh <alias> 'bash -s' < script.sh`, and keep the device's real `HOME` so its agent CLIs stay logged in.
- Drive it from a private hided: the web e2e fixtures `startHerdr` and `startHided` with the fixture home's `.ssh` linked to the operator's, because hided resolves the alias from `$HOME/.ssh/config`.
- Register the device with `register_device` and `host_consent: false`, which installs no helper and no kit on the device; a check that needs the kit there needs a private `HOME` on the device too.
- Reuse one SSH connection (`-o ControlMaster=auto -o ControlPath=<short path> -o ControlPersist=120`); dozens of fresh connections fail with "Too many authentication failures".
- A web-attached pane grows its row count, so read it with a large `--lines`.
- Run every remote cleanup after the local daemon and server stop, or in its own `try`; an ssh that threw inside `finally` once leaked a hided.

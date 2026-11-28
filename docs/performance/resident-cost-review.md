# Resident work and hook path costs

- Code: `hide-relay/`, `hide-agent-hooks/`, `hided/src/hook_request.rs`
- Index: [PERFORMANCE_TESTING.md](../PERFORMANCE_TESTING.md)

## The opener guard hook on a shell call

The `PreToolUse` entry runs `hide-agent-hooks` before every shell call a Claude Code or Codex agent makes, so it is a high-frequency path that every agent pays and no operator sees.
Per input it adds one process start: the helper reads the payload (at most 256 KiB, waiting at most 0.5 seconds) and runs a byte test over the whole payload for a word of an opener and a viewable mark ([agent-hooks.md](../agent-hooks.md#the-opener-guard)), and exits with no output when either is missing, before it parses JSON or asks the daemon; the [project guardrails](#the-guardrails-on-a-pre-tool-call) read their policy file first.
A call that holds both, which includes an ordinary call whose payload mentions `open` and a `.md` file, pays one JSON parse and the shell lexer and starts nothing.
Only an opener of a viewable target in a Hide pane asks the daemon anything: one `registration` hook request over the local stream, within the guard's 2.5 second deadline, which starts no program.
The guard keeps no state, runs no timer or worker, takes no lock of the runtime, publishes nothing and fans out no notification; the daemon sees one proof request per refused call, the proof `SessionStart` already asks for.
The retained data is the hooks' log, capped at 256 KiB with one rotation, holding one `opener.refused` line per refusal, and the throttled diagnostic store `delivery` already caps; crossing a cap rotates or suppresses, and never refuses a call.
A defect or a daemon that does not answer lets the shell call run, so the worst the guard can add to an ordinary call is its own bounded wait, never a refusal.

Measure it at the command boundary: the installed command line run as the runtime runs it (`sh -c`, the payload on stdin), wall time per call from the caller's side, against a process-spawn baseline, with the first calls discarded as warm-up (ten, and five for the real `hide` row).
Report p50, p95 and the load average before and after, and report the idle and driven (refusal) paths separately.
Time the exact installed command through `sh -c` with the payload on stdin and retain the bounded measurement script and raw observations in the run directory.
Keep the command/binary hash, OS, load, sample count, warm-up exclusions, failures and path classification with the result.
The ordinary shell-call, byte-test-only, stand-in refusal and real-daemon refusal rows are separate workloads.
Helper return time does not prove the agent's time to first token or its runtime's hook scheduling latency.
Observe the actual supported runtimes separately when claiming native behavior.
Measured on 2026-10-30 (macOS 15.1, debug build, load average 22 before and 21 after, so the scheduler dominates; 10 warm-up and 100 timed calls per row through `sh -c` with the payload on stdin): `/usr/bin/true` p50 14.4 ms and p95 20.2 ms, an ordinary call 20.1 and 31.5, a call that passes the byte test and holds no opener 19.1 and 33.1, and an opener refused through a stand-in `hide` 33.8 and 51.2.
Regression owners are `hide-agent-hooks/tests/it/opener_guard.rs` (no `hide` started for a call that is not an opening) and the reading's table tests in `src/opener_guard.rs`.

## Letters at a tool event

While an agent works its letters reach it at a tool event ([delivery.md](../delivery.md#letters-while-the-agent-works)), which makes a tool call a high-frequency path for six agents.
Per tool result, OpenCode's plugin and the Pi and omp extension start one helper (`hide-agent-hooks <agent> letters`), which sends one typed hook request to the daemon itself (what `hide inbox --hook --session <id> --tool` sends), starting no `hide`; the core hands it to the delivery worker like a prompt pull, which reads the ledger it holds and renders one bundle of at most 8 KiB, grouping the caller's letters (at most the 1024 open ones) by sender, with no subprocess and no network, and writes the ledger only when the pull's maintenance changed a letter.
The scripts bound it: only the pane's root session asks, with a 1.5 second budget past which the result goes on unchanged, inside the cap of eight running helpers, and a result that comes while letters the script handed over wait for their receipt pays a map read and no helper.
Grok's and Cursor's hook runs per tool call (their entries carry no matcher from version 6; Grok now uses `hide-guidance@7`, Cursor stays at 6); a call that runs adds the same `hide inbox` call inside the guard's 2.5 second budget, and a refused call adds nothing.
The answer also takes the runtime's lock once for the caller's mission cells that reach a working agent ([delivery.md](../delivery.md#hides-notices-are-status-cells)): with no such cell in any mission it reads nothing more, then finds the caller's registration in the ledger and, with no mission cell addressed to it, reads nothing more; otherwise it builds the caller's cells as a prompt pull does.
Grok's version 7 also starts one helper per `UserPromptSubmit`, which sends the pull `hide inbox --hook --session <id> --prompt-digest <digest>` sends but prints and confirms nothing.
A root `Stop` with a session and `stopHookActive: false` adds one pull (`hide inbox --hook --session <id> --turn-end`), and one confirmation command only when it printed letters.
Both events share the helper's total 2.5 second budget, including a 400 ms confirmation reserve; the subagent count runs after the Stop intake, so a stalled intake can delay that count by up to 2.5 seconds.
A repeated Stop with `stopHookActive: true` makes no pull and still updates the count.
The core saves only changed bell claims or handover counts; repeated observation of the same bell turn adds no claim write.
agy's `PreInvocation` hook already runs per model call; a call after the prompt's first adds the same `hide inbox` call inside the prompt hook's 1.65 second budget.
On Windows, agy's command starts one Windows PowerShell process between its existing `cmd` shell and the helper, at each selected hook event, to carry the command through agy's quote escaping ([agent-hooks.md](../agent-hooks.md#antigravity-hides-hook-block)).
This shell runs only for that event; it adds no resident worker, timer, retained state, pending queue or notification, and leaves the helper's input and deadlines unchanged.
agy owns the command's existing eight-second timeout; this wrapper adds no separate descendant-cancellation guarantee.
Measure its cold call and warmed caller-side p50, p95 and maximum against a cmd command that actually runs the same helper, reporting the old escaped command's skipped helper separately rather than counting that skipped work as a working baseline.
Claude Code and Codex start no tool-letter helper: their letters are typed by the doorbell, on its own thread, as at rest.
Nothing here keeps state, runs a timer or publishes; the retained data is the throttled delivery diagnostic.

Measure it at the command boundary as the opener guard is measured: the helper's `letters` operation with its JSON on stdin, and the hook with each agent's documented tool payload, against a real daemon that holds no letter for the pane, wall time per call from the caller's side with the first calls discarded as warm-up, p50 and p95, the load average before and after; that is the cost per tool call, since every call asks.
The PRD that landed it set no bar (D-12); the figures measured on macOS are in the pull request that landed it, and a later change to this path reports its own.
Regression owners are `hide-agent-hooks/tests/opencode/plugin.test.mjs` and `tests/pi-extension/extension.test.mjs` (no ask while a handover waits for its receipt, a late helper leaves the result unchanged) and `tests/it/grok_cursor_hooks.rs` (nothing asked outside a pane or without a session).

## The guardrails on a pre-tool call

The project guardrails run in the same helper, before the opener guard, on every shell call and, from matcher version eight, on every file tool call of Claude Code, Codex, Grok and Cursor (PRD agent-guardrails B27, [agent-hooks.md](../agent-hooks.md#project-guardrails)); a file call that started no helper before now starts one.
Per input they add one read of the policy file (at most 8 MiB, a few hundred bytes in practice), in any agent session, a Hide pane or not, and, when no project of the machine has a protection on, nothing more: no JSON parse of the payload, no Git read, no child.
With a protection on, the call pays one JSON parse, the shell lexer for a shell call, and reads of the repository's own files (`hide_project::git`, no process) for each path it names; only a command that would discard changes starts `git --no-optional-locks status`, once per checkout and call, owned through `hide_platform::process` with a 2 second budget and a 4 KiB output cap; at most 512 words of one shell call are matched against the protected paths.
OpenCode's plugin and the Pi and omp extension read the policy in their own process, again when its file changes, and start no helper for a file tool while nothing is on.
The guardrails keep no state, run no timer or worker and publish nothing; the retained data is the shared refusal log and one `policy.unreadable` line per ten minutes.

Measure it as the opener guard is measured, with the helper's own CPU time (`getrusage` of the child) beside wall time, because on a loaded machine wall time measures the scheduler: each runtime's file call and shell call, under an empty policy and under a policy whose only project is elsewhere, against `/usr/bin/true` measured the same way.
B27's bar is the no-protection call's p95 at or under 15 ms on macOS; report the load average with it, and treat a run whose `/usr/bin/true` p95 is above that bar as not measuring the helper.

## Resident work cost review

The standalone `hide-relay` listener owns at most 512 admitted sockets plus 32 HTTP/refusal tasks, with eight data frames and sixteen independently reserved control slots per admitted connection and no idle polling worker.
The slots include active writes and reserved departure notifications; one bounded FIFO preserves ordering, and an accepted phone reserves its departure before joining.
Each received binary frame performs one bounded protocol parse, one short state lock, constant-time room/peer lookups and quota accounting, then queues the unchanged bytes to one recipient.
Admission prunes at most 4,096 IP records; authenticated registration prunes at most 4,096 retained room budgets.
Connect/disconnect metadata goes only to the owning device, and losing that device closes at most four phones.
No filesystem, subprocess, socket await or log sink runs under the state lock.
The listener owns and joins every task on shutdown, with five-second writes and a sixteen-second shutdown deadline before abort-and-reap; cancellation drops the same task set and registration guards.
The [relay contract](../RELAY_PROTOCOL.md#server-budgets-and-refusals) lists every quota, refusal and retained-data bound, and `hide-relay/tests/it` proves the loopback service behavior.
These are code bounds and functional assertions, not measured throughput, RSS or latency guarantees.

The active checkout Git header reuses the Changes pump's 250 ms wake, with one Git reader (two-second spacing after settlement) and one independent CI reader (30 seconds after settlement), never a reader per tab or repository.
Each wake adds scalar/request comparisons bounded by five commit identities and three short lock acquisitions; it does no machine work under the lock or on the input/session-sync thread.
Only the front Git checkout with an attached UI is admitted; Home, plain folders, detached renderers and inactive checkouts read nothing.
One Git job and one CI job may run, and only the latest target is pending; old generations and old SHAs cannot publish over their replacements.
Git retains counts, at most five subjects and five CI summaries, not paths from its status scan or a per-checkout cache; the node bounds status output at 8 MiB and log output at 64 KiB and reports overflow as failure.
Git's three-second budget and CI's one 20-second link wait bound shutdown joins; underlying node `gh` work has its existing 15-second command timeout and no new authentication path.
The existing gh pipe reader still lacks a retained-byte cap; this is a pre-existing boundary limitation, and the fixed query returns at most five oids, aggregate states and provider commit URLs rather than raw check lists.
Notifications fan out only on an actual target, fact, error/recovery or stale-state transition; identical status reads renew only the internal freshness deadline.
Regression owners are `checkout_header_reads_file_edits_unborn_head_and_tracking_ref_changes`, `header_fences_late_git_ci_and_reconnect_results_and_ages_without_false_clean`, and `web/e2e/checkout-git-header.spec.ts`; browser/native timings and captures belong to the run record, not an inferred responsiveness verdict.

Windows foreground detection reuses the holder's output-driven fact reader, at most once per 16 ms, with no idle timer or new worker.
The existing process-table snapshot validates child birth times and now follows only a single-child cmd or PowerShell wrapper chain, bounded by the process walk's depth limit; an unreadable identity refuses the readiness proof.
A newly observed process adds one bounded native command-line query (at most three reads after its size probe, at most 64 KiB plus the native descriptor), cached by pid and image name; the Windows argument parser owns and releases its native allocation on every path.
Package identity comes from the command-line path with no file read or subprocess, and the core compares the bounded package identity with the agent definitions after trying the executable or script name.
The holder keeps one cached program and sends at most 64 arguments within 8 KiB; facts publish only when changed, with no added notification fan-out or pending queue.
The existing process-table snapshot scales with the machine's process count and has no row cap; this change adds no process table or resident cache.
The argv contract, real-holder busy-shell refusal and `agent-start-platform.spec.ts` own the regression checks; CPU and latency parity remain unmeasured.

The read axis's look adds no thread, timer or queue: it is compared where a selection completes (`Runtime::change_look`), under the lock that selection already holds.
Each outermost selection costs one comparison of the operator's look (this machine's focus, at most 16 phone agents, and per connected device its focus filtered by the tabs its window shows); a selection inside another adds a depth count and no comparison.
Only a moved look reads rows again, once, over the rows already published; that read is separate from the read a session makes of its own rows as it arrives and from what is recomputed for other reasons (checks, plans, Desk, lineage), which this neither adds nor removes.
For each device whose rows that read changed, it projects that device's tree again with the existing functions: `sync_pane_status` looks each pane's row up, sets each tab's agent and label, searches each strip entry's tab among the rows, and redoes the checkout summaries and the project sort; `judge_device_pane_children` passes over the device's rows once per pane; and the session the device last sent is read with the same look and projected the same way, rows and relations.
A device whose rows did not change, and a look that did not move, run none of it, so the same look and the same session repeat no projection and announce nothing more.
A device's session already rebuilds its strips on every arrival; naming each rebuilt entry's agent there adds one search of that entry's tab among the rows, as this machine's strips have.
Terminal input and a document's keystrokes are no selection and skip the comparison; terminal output never reaches the core.
A reconnect that keeps a device in front is one selection that reads nothing, though its comparison still runs.
It does no I/O, subprocess or serialization; regression owners are the `read_on_look.rs` tests, and no latency or CPU parity was measured.

Every change adding resident work or touching a high-frequency path must explain its cost in the PR's Review section, even when it claims no performance improvement.
This is a review requirement, not a new CI benchmark or permission to change runtime behavior through documentation.
Apply engineering principle #14 by naming the resource's owner and cleanup on success, failure, cancellation, owner exit and partial startup.
Apply principle #15 by naming admission, pending-work and retained-data caps, the outcome when each cap is crossed, and every cap the implementation still lacks.

Before adding or changing an entry:

1. Name the input/tick, its cadence and source owner, and the existing reader or worker it can reuse.
2. Explain added work per input or tick, including idle and driven paths, lock acquisitions, scans, I/O and subprocesses.
3. Explain notification fan-out and the transition that publishes; unchanged observations must not create frames merely because a timer fired.
4. State the pending-work bound, retained-byte/count caps and overflow/refusal behavior; distinguish a timeout or active-worker limit from a queue cap.
5. State how work stops on every exit path and which admitted reads may finish after cancellation; detached work needs an explicit bounded lifetime.
6. Update this ledger and the [state/process ownership guide](../ARCHITECTURE.md#state-placement-and-publication), then record applicable regression checks and any matched measurements in Evidence.

For example, a reader with one active job and one latest pending request has bounded pending work only if its request/answer sizes and owner count are bounded too.
If code lacks a bound, write that limitation in Review and the ledger; do not invent one or quietly alter behavior in a documentation-only change.
The [maintenance policy](procedure.md#maintenance-and-review-policy) decides when actual app scenarios and controlled baseline/candidate measurements are required.
Unrelated documentation changes need focused documentation/invariant checks and must not claim measured performance parity.

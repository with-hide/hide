# Status Model

How Herdr's raw agent state becomes a group in the sidebar, a pet pose, and a badge row.

The state of an agent is five axes, not one word.
What it needs from the operator, whether it is running, whether it has reported a completion, whether the operator has looked at it, and whose work it is are independent, and mixing them into one string is what made the same agent read differently in different views.

- Demand: question, approval, error (shown as Blocked, see below), none.
- Activity: working, stopped, unknown.
- Completion: reported, not reported.
- Read: read, unread.
- Ownership: operator, delegated, or Hide AI's, for work a Hide-started agent does for a screen that owns it.

`herdr-core/src/agent_state/axes.rs` is the single owner of all five.
`agent_state/turn.rs` derives the group, request verb, close and rest gates from them; `sidebar.rs` assembles the snapshot rows.
It also derives everything a view draws from them - the group, the mark, whether the row is emphasized, the status word, the descendant mark, and whether closing the pane needs a confirmation or a fresh status check - so no surface decides any of it a second time.

Ownership is not stored anywhere.
It is read back off the row: a row whose lineage depth is greater than zero is delegated, a root whose Hide AI snapshot names a home is Hide AI's, and every other row, including an orphan whose parent is gone, is the operator's.
`ownership_of` is the only function that makes that judgement, and `apply_lineage` reapplies every derived value once the lineage is known.

### Work Hide AI does for a home

A Hide-started agent whose task kind has a home (today a Memory card's promotion, `memory_promote`, whose home is the Memory tab's To ask rows) is not the operator's work, so it is a third ownership value, `HideAi`, not a flag on an operator row (PRD hide-ai-work-home D-13).
The coordination ledger's record of the spawn is the only source of the fact (`coordination::started`), and the task kind is a closed enum (`coordination::TaskKind`) whose exhaustive `home()` match makes a new kind fail to compile until it names its home.
A kind this build does not know, or a record with no task, loads with the rest of the ledger and reads as no home: that one agent stays an operator row with its mark and a `hide_ai.task_kind_unknown` diagnostic (D-19).
The row stays in the agent data, so ⌘K, tabs, the pane header's owl and `hide agent list` find it as before; what changes is which lists enumerate it.
`agent_state::listed_for_operator` is the one decision: a `HideAi` row is a member of the sidebar's tree roots, the device's tasks, Overview's requests and graph, the phone's list, the checkout counts and the pet only while it is in Needs You, and no tree draws it even then (the raised Needs You and the 태스크 list do).
Its group is Needs You or Seen and never Working or Done, so it moves no Working or Done badge, rail count or notification.
It is in Needs You when the agent itself asks (a question, approval or block, read like any row's) or when the home has a turn for the operator.
The home's turn is computed by the core from what the Memory promotion module already follows, the attempt's pull request in the machine's PR catalog, and published on the row as `hide_ai.turn`:

| The attempt's pull request | Turn |
| --- | --- |
| Open, not a draft, checks all passing or none, no change requested | `pr_review`, which stays after the row is read |
| Open, not a draft, checks failed, and the agent is not working | `ci_failed` |
| A draft, checks pending or unknown, a requested change, or failed checks while the agent works | none |
| Merged, closed, or no pull request | none; the attempt leaves the table |

A row has one line; its second line is the agent's ask or its block's cause, then `ci_failed`, then `pr_review`.
Entering Needs You notifies once, as for an operator row, and a turn changing inside it does not notify again, because the notification state machine follows the group and not the word.
Only a live agent has a row: an attempt with no agent (a failure with no pull request, an open pull request after the operator closed the pane) shows in the Memory tab alone (D-15).
The shell words the turn and opens the home from the row (the Memory tab with the attempt's To ask row open); the phone opens the agent's detail.
Regression owners: `a_hide_started_row_follows_its_attempts_pull_request_into_and_out_of_needs_you` and `a_home_owned_row_is_listed_only_while_the_operator_has_something_to_do` in `herdr-core/src/runtime/tests/`, `the_home_turn_follows_the_pull_request_and_the_agent` in `memory_promotion.rs`, and `a_home_owned_row_notifies_once_on_entering_needs_you_and_not_on_a_turn_change` in `hided/src/mobile/push.rs`.

### A held Memory card is no status

An agent reporting a Project Memory card stale (`hide memory stale`) changes no agent row, group, badge, notification or snapshot field, and the operator is told nothing (PRD memory-stale-cards D-11, B15).
The hold is the card's, not the agent's: it decides only what the hooks give, and `hide memory show` is where it is seen ([ARCHITECTURE.md: A card an agent reports stale](ARCHITECTURE.md#a-card-an-agent-reports-stale)).

## agent_status provenance and background waits

Herdr owns the raw `agent_status`; Hide reads it through `agent.list`/`agent.get`, converts it at `herdr-core/src/wire.rs` and derives its axes in `agent_state/axes.rs`.
For Claude Code and Codex, Herdr's [official agent reference](https://herdr.dev/docs/agents/) describes terminal-screen inference as the lifecycle source, rather than lifecycle state reported by their session hooks.
A native session reference or display-only hook token does not prove that a tool is running or that its command completed.
Other integrations can have full lifecycle reporting, so this screen-inference statement is specific to those two agents, not every agent Herdr supports.

Hide's current mapping calls raw `done` a reported completion and `idle` a ready stopped pane, as specified below.
That is a presentation signal supplied by Herdr, not a verified task outcome, successful test, exited background command or durable delivery receipt.
The current upstream documentation also distinguishes semantic state waits from arbitrary command completion; its display/attention descriptions must not replace Hide's pinned wire mapping without checking the bundled schema and adapter.
Use a command's own exit/result or an explicit coordination receipt when a workflow needs completion evidence, rather than treating a quiet pane or Done group as that evidence.

[Issue #348](https://github.com/with-hide/hide/issues/348) records one actual run in which an agent waiting for a background command was read as `done`.
This is a known screen-inference limitation: a prompt-like quiet screen can cease to look working while background work is still outstanding.
It is one observed case, not a guarantee that every background wait produces `done` or that Hide detects outstanding commands independently.
The corresponding behavior for Monitor and for Codex's background terminal has not been verified by that observation.
The existing [waiting-on-descendants rule](#a-quiet-row-that-waits) uses proven lineage and child axes; it is not a general background-command detector.

## Hide owns the read axis, at pane level

Herdr's seen is tab-scoped.
Its own documentation is explicit: focusing a tab, or targeting it with pane focus or agent focus, marks every pane in that tab seen.
Three finished agents side by side in one tab therefore cleared together on a single click, which is the bug this model exists to fix.

So Hide keeps its own record instead.
A pane is read when it has held Hide's keyboard focus since its last state change, and on the Herdr server the record was read on, a state change is Herdr's `state_change_seq` moving **or** the derived demand and activity pair changing.
The pair matters because Herdr's sequence does not move when only the core's own label changes: an analysis that reads a question at the end of a turn arrives after the agent stopped, so the demand appears with the sequence where it was.
The record is `pane_read_records` in the persisted UI state, keyed by pane id, so it survives a restart.
A record is dropped only when the authoritative pane layout stops reporting that pane, scoped to the namespace that pass owns, so a temporarily incomplete agent list and a local sync can never drop a restored or remote pane's record.
The same pass drops the descendant signals other records keep for that pane and the pane's saved completion.
A corrupt store loads as an empty record, which reads as everything unread, and says so in a diagnostic; it is never silently treated as read.

Nothing reads Herdr's `done` versus `idle` split to decide the read axis.
This is enforced by `INV-herdr-unseen-token`.

### Across a Herdr restart

`state_change_seq` is Herdr's own count for one server process: a cold restart and a live handoff both start the count again and keep none of a pane's tokens.
A restored server also reports each pane in steps, measured on the pinned 0.9.3: first at sequence 0, then `unknown` while it detects the agent again, then settled at a sequence of its own, which may land below, at or above the one before.
So a sequence is compared only with one read on the same server, and no sequence value is taken as a restart signal.

Each record names the server it was read on (`PaneReadRecord::server`).
Every connection Hide makes to a Herdr server, this machine's or a device's, starts with a fresh mark, and once a saved record or completion names that mark the lineage writer writes it on the pane as the `hide_server` token (`wire::SERVER_MARK_TOKEN`).
A pane that carries a mark is on the server that mark names; a pane that carries none is on the server Hide is connected to now, because Herdr drops the mark with its server.
So a hided restart against a running server finds its old mark and keeps comparing sequences, and a turn the agent finished while Hide was away is news.

A record saved on another server is carried over instead of compared.
Each record holds its verdict (`PaneReadRecord::unread`), saved whenever it flips, and until the pane settles on the new server the row keeps that verdict.
The first time the pane settles on the connection, a read record moves to the new server unless the pane shows something the operator had not read: another known agent session, a demand it did not have, or a change between working and stopped.
A completion the restored server reports again is the one the record held, so a restored agent that reports `done` again stays read.
An unread record stays unread through the restore and is decided again only when the operator reads the pane.
Running the same pass twice changes nothing, and a second restart starts from the records the first one left.
A record saved while its pane was still being restored (the operator looked during those steps) is carried the same way once the pane settles.
Records written before this rule name no server, so Hide cannot tell which server they were read on; each is judged by sequence as before the first time its pane settles and then names that server.
Reading such a record as read on a guess would lose news, so an upgrade whose first connection reaches a restored server (an app update hands Herdr's server over before the new daemon starts) can show a read row unread once.

Regression owners: `a_read_pane_stays_read_through_a_restored_servers_steps`, `a_restored_pane_reporting_done_again_stays_read`, `a_pane_unread_before_a_restart_stays_unread`, `a_restored_pane_showing_something_new_is_unread`, `a_hide_restart_on_the_same_server_shows_work_done_while_away`, `a_childs_signal_through_a_restore_lights_its_parent_only_when_new` and `a_record_saved_before_servers_were_named_is_judged_by_sequence_once` in `sidebar.rs`, `a_device_pane_read_before_its_server_restarts_stays_read` for a device, `a_server_mark_is_written_once_and_rides_with_lineage_keys` for the writer, and `web/e2e/read-across-restart.spec.ts` against a real cold restart and live handoff.

## Completion is separate from stopped

An agent can be stopped because it has completed a turn or because a newly opened pane is ready for its first instruction.
A completion is `agent_status: done`, or `idle` with a `completion_seq`: Herdr turns `done` into `idle` when it marks the pane's tab seen and keeps `completion_seq` until the agent works again.
An `idle` lifecycle with no `completion_seq` reports a ready stopped pane and does not put it in Done, even though a missing read record still makes its read axis unread.
Because Herdr's seen keeps the completion, an unread finished pane in a tab Herdr marked seen stays Done until the operator reads it in Hide.
The completion fact is part of the pane read fingerprint, so a ready-to-completed transition becomes unread even when Herdr's process-local sequence does not move.
Herdr's done/idle distinction supplies completion evidence only; Hide's own pane record remains the sole read authority.

A completion is news only when it appears.
A record that holds a completion the row no longer shows still covers the row: a completion goes away only when the agent works again, which its own change already says, or when a restored server no longer reports it.
Any other change to the fingerprint (the agent working again, a state change between two looks, another session in the pane) still turns the row unread.
Regression owners: `a_read_demand_stays_read_when_herdr_marks_its_tab_seen_after_focus_left` and `a_read_demand_turns_unread_again_only_for_news` in `sidebar.rs`, and `web/e2e/blocked-state.spec.ts`, which holds Herdr's focus at the socket so its seen lands after the operator moved on.

A restored server reports neither `done` nor a `completion_seq` for a pane whose completion it lost; a restored Claude pane comes back `idle`.
So Hide keeps the completions it saw (`pane_completions` in the persisted UI state, keyed by pane), each with the agent session, server mark and sequence it was last seen at.
A saved completion keeps showing while the same session has not changed since: on its own server while the pane's sequence is the one it was saved at, and on a restored server when the pane first settles stopped.
It is kept before the lineage is built, so an ancestor sees the same descendant completion it saw before the restart.

## A descendant's change turns its ancestors unread

The fingerprint carries one more element: the outstanding demands and reported completions of the row's live descendants, each keyed by the descendant's pane (`PaneReadRecord::descendant_signals`).
A descendant entering a question, approval or error, moving between those three, or finishing a turn adds a signal the record does not hold, and every ancestor row becomes unread through the same comparison its own state change would use; there is no second store, timer or event.
A signal that goes away does not turn anything on: a question being answered, a finished child starting new work, or a child pane closing is trimmed from the record on the next projection, so the same descendant is news again the next time it asks or finishes.
A signal is trimmed only when the descendant is settled and no longer sends it: while a restored server has not settled the descendant, or has not got its lineage tokens back, the record keeps it.
Reading the ancestor records the signals it shows at that moment and nothing more.

A descendant's signal never puts an ancestor in Needs You: a child's question leaves a working root in Working.
That is the boundary: a child is asked for its status through its parent, and a parent that has heard from a child is drawn bright until the operator looks at it.
The one way a descendant moves its root between groups is the waiting state below, and it moves the root into Working, never into an attention group.

Regression owners: `a_descendants_demand_or_completion_turns_every_ancestor_unread_and_nothing_else_does`, `an_ancestors_group_comes_from_its_own_axes_and_a_child_never_makes_it_needs_you`.

## A quiet row that waits

A row with no demand of its own, not blocked and stopped (idle or done) waits when something in progress will wake it: work that is still running, or an answer someone is still making for it.
It has not finished, so it sits in Working, draws the in-progress ring, has `status_code: waiting`, and is not emphasized.
In every list, the project tree included, its line leads with `대기 · ` (Waiting) and goes on with the label's sentence or, with none, the proven cause, so a ring whose line is missing or does not start with 대기 is an agent at work (status-marks D-18, B29).
Why it waits is one additive wire value, `wait`, beside `group: working`; absent means the row waits for nothing.
No new group value reaches the wire, so a decoder that does not know the field draws an ordinary Working row.
`apply_lineage` decides it on the same pass that sums `descendant_counts`, by this precedence (`axes.rs::own_wait` and the children check):

| `wait` | The row waits for | Proven by |
| --- | --- | --- |
| `children` | A live descendant that is working, or stopped on a background task its session proves. Only a lineage root waits this way. | The lineage and the descendants' own axes |
| `background` | A background command, monitor or task its own agent started and that is still running in the process that started it, on a turn the label did not read as finished | The session's own records, read for the Herdr state the label worker asked under (below) |
| `checks` | The running checks of a pull request the row is responsible for, or the GitHub read a turn that just printed a pull request's address set off (A row waiting on its checks, below) | GitHub's answer for the pull request, read by `gh`, and the turn's own sighting |
| `reply` | A Hide `request` or `block` letter the row's session sent whose answer is still awaited and still being made: the recipient has not taken the letter in yet, or it is working, or it or a descendant of it is on a proven background task, or a descendant of it works | The delivery ledger (`Ledger::awaited_answers`) for the letter and whether it was taken in, and the recipient's row for the rest |

A turn the label read as finished (`done`) is not hidden by the background tasks it left (status-marks D-16, B27): the row is Unread with the green dot, carries the count as `background_left`, and its line ends with `· 백그라운드 N` (`agents.line.backgroundLeft`), its hover and accessible name with `백그라운드 N개 남음`; the narrow row cuts the sentence and keeps that tail.
The Done notification goes out as for any finished turn, and opening the row reads it; a task that wakes the agent starts a new turn, which decides the mark again.
A `waiting` or `unfinished` verdict, or none (Hide AI off included), keeps the row in progress on its background tasks, and children, a reply being made or running checks keep it in progress whatever the verdict (B28).
A merely ready or finished descendant is quiet, and one whose activity Herdr reports as unknown does not make its root wait, because a waiting state the projection cannot vouch for is not drawn.
A descendant holding a question, approval or error waits on its parent, not the other way round, so it does not make the root wait: it turns the root unread through the descendant signals above, and moves the root to Needs You only once one of the six escalation causes raises it (Delegated escalation).
A delegated row that waits on its own (`background` or `reply`) is Working, like a delegated row that is running, and its ancestors' counts take it as working; its root waits on it only while it waits on a background task, because a reply is made by the agent that answers, not by the row that asked; an ordinary delegated row that waits for nothing keeps Seen.

A reply wait ends when nothing is making the answer, even while the letter is still open in the ledger.
Once the recipient took the letter in and stopped, having answered with a report letter or finished without answering, the sender shows its own state: a dot to read until it is read (green only when its label read the turn as finished), then quiet.
The recipient's own reply wait is not counted as making the answer, so two agents that wait on each other's answer never both draw a wait: a child that asked its parent by letter waits while the parent has not taken the letter in or works, and the parent never waits on that child.
The 24-hour answer deadline stays the outer bound, and a turn the sender's session began after the letter went out still ends that letter's wait.
A recipient the projection cannot vouch for holds no wait, taken in or not: a code-owned recipient, which has no row whose progress Hide can see, an agent on a device that is not connected, whose last state may no longer hold, an agent asleep, which a letter does not wake, and a pane that now runs another session than the one the letter went to.
The runtime lays each awaited letter on its sender's row (`lay_awaited_replies`, leaving those recipients out), and the lineage pass decides from the recipients' rows, so a recipient's step reaches the sender on the same projection; publishing the ledger rebuilds the lineage when a letter's sender, recipient or intake changes and not for its other delivery steps.

Mark precedence on a row is the operator's turn, then progress, then a stop to read, then quiet (`RowMark::of`, The mark below).
The `wait` value is only ever set on a row with no demand of its own that is not working, so the precedence is the order of the checks in `agent_group_for` and `RowMark::of`, not a second rule.
Only a lineage root waits on its children: an ordinary delegated middle row keeps its own mark and Working or Seen group, and the grandchild is drawn under it in the tree.
An active escalation uses the six-cause exception below.

### A row waiting on its checks

A quiet row also waits while a pull request it is responsible for has checks running (status-marks D-07): the row is in progress, not a stop to read, until the checks end.
`runtime/checks_wait.rs::lay_checks` lays the input before the lineage, from the pull requests `agent_state/work.rs::linked_pull_requests` links to the row and only the ones `assign_duty` gives it, so one pull request holds one row, the one nearest its root.
A pull request counts while it is open and its checks rollup is pending with at least one check actually running (`GhCheck::is_running`): a required check that has never reported (`EXPECTED`) may never run, so a pull request whose only pending checks are expected holds nothing.
A project whose GitHub status is stale, unavailable or unreadable holds nothing either, because its last answer may be long over.
Each exclusion writes one `checks_wait.excluded` diagnostic per pane, pull request and reason (`stale` or `expected_only`), so the log says why a row did not wait.
A device's rows never wait on checks: `lay_checks` places only this machine's panes, so a pull request a node reads for its own project reaches that node's rows through the link record and its PR card (`links.rs::link_projects`, `open_links`), never as a wait, and a device this core does not read links none.
The wait is `wait: checks` with `checks_wait`, the pull request's number, on the row (`own_wait` takes it after a background task and before a reply), and its line, with no sentence, says `대기 · CI · PR #N`.

A turn that ends having just printed a pull request's address is not a stop until GitHub has answered for it (D-09): the sighting sets off a GitHub read (`projects.rs`, `Sighted.owed`), and until that read lands or fails the row waits on that pull request the same way, so a turn that opened a pull request with checks about to start is never announced finished first.
The hold has no timer of its own: the read landing, a failed read and the agent working again each end it.

While any row waits on running checks, its project is read again once a minute whatever the window shows (`request_view.rs::reread_pending_checks`), and GitHub's answer re-lays the checks and rebuilds the lineage and the read axis (`refresh_checks_waits`).
When the checks a row waited on end, in the same agent state, the row records how (`checks_ended`: the number and whether they passed), and the end is news (D-08): the read fingerprint carries it (`PaneReadRecord::checks_ended`), so the row turns unread, lands in Unread with a plain dot, and its line says `CI 통과 · PR #N` or, in the warning colour, `CI 실패 · PR #N`.
An end the operator has read is not news again, and the end is forgotten when the agent's state moves.

Regression owners: `a_checks_wait_is_in_progress_and_its_end_is_news` in `agent_state/turn.rs` and the runtime checks-wait tests.

**Background work is proven only by the session's own records, and only Claude Code's can do it.**
`hide-session/src/turns/wake.rs` reads, from each Claude Code record: the start text a tool returned, only when the tool's message opens with it (`Command running in background with ID: X`, `Command did not complete within its Ns timeout and was moved to the background (ID: X)`, `Monitor started (task X, expires in Nm ...)`, `Async agent launched successfully ... agentId: X`); the end (a `<task-notification>` naming the task with status `completed`, `failed`, `killed` or `stopped`, in the `queue-operation` enqueue record and in the `queued_command` attachment, or a `TaskStop` call whose result succeeded); and the process boundary (the `SessionStart:startup` or `SessionStart:resume` hook record; `clear` and `compact` keep the process).
`TurnTracker` folds them under their own offset, so a record is never folded twice, and keeps at most 32 devices (`WAKE_DEVICE_LIMIT`).
A device never outlives the process that started it, so a new process start clears them, and a session whose file shows no process start proves none (fail closed).
A monitor that announced its expiry stops counting at it.
A `TaskStop` call stops nothing by itself: the tracker keeps the call by its `tool_use_id` and ends the device only when the result of that call arrives without `is_error`.
The tracker keeps, in the same table, the calls to the tools that can start work (`Bash`, `Monitor`, `Agent`, and `Task`, the name an older Claude Code gave `Agent`) until their result arrives, at most 32 open calls and devices together; the table is what tells a result too large to read apart from a screenshot (below).
A refused stop (`Task … is not running`, `No task found`), a call with no result yet and a result of another process leave the device running until its notification or a successful stop.
A device is lost for one of two causes, and the first one since the process started is kept (`WakeLoss`): `capacity` when more than 32 devices, or calls waiting for their result, were open at once, and `lost` when a record that may have started, ended or stopped a device could not be followed.
Either proves none until the next process start, and neither fails the read the turn and label depend on.
The record the reader could not follow is one with more marks than the bound or an id too long to keep, or one over the 256 KiB line cap that is discarded without its text.
A discarded record is told by its structure and never by its words, because a text that is matched is wrong the day it is spelled differently: the bounded scan of the discarded record (`conversation_cursor.rs::LargeRecord`) keeps the record's `type`, the `name` and `id` of its `tool_use` blocks and the `tool_use_id` of its `tool_result` blocks, and `wake::Record` names the four kinds `wake.rs` takes marks from, so the large-record reader must decide each of them.
A kind that carries no mark changes nothing.
An `attachment` or `queue-operation` (a notification, a hook) is `lost`, and so is any record whose scan cannot certify its structure (a truncated line, an id that is not plain, no `type`).
An assistant record leaves the `Call` mark of each call it makes to a tool that can start work, so its result is still paired, and is `lost` when it holds a `TaskStop` (its task is in a field the scan does not keep) or such a call with no id it can keep.
A user record answers each call its tool results name without saying how (`Answered` with `Unreadable`), and the tracker reads that by the call: the answer to an open call to one of those tools, whose output may have been a task's start, is `lost`, the answer to a stop whose result was discarded is `lost` rather than taken as stopped, and the answer to any other call (a screenshot a file read returned, nearly every record that large in real sessions) changes nothing.
A result with no id the scan can keep is `lost`, because nothing says which call it answers.
The worker writes one diagnostic when a session first loses its devices, `wake_devices.capacity` (with the limit) or `wake_devices.lost`, so the log says whether to raise the limit or look at the record.
The count is laid as `RowFacts.wake_devices` by `LabelOverlay::apply`, only for the `state_change_seq` the read was made under, so a row never waits on a task a stale read proved.
When a process start finds devices that were proven alive, the tracker keeps `wake_vanished` until a turn or a device begins; a stopped row with a label then draws a plain dot to read with the word Stopped and its second line says, in the operator's language, that the work it waited for is gone (`line.mode: vanished`, text empty in the core; the web draws it, and only the sidebar row draws it).
Hide or the daemon restarting is not a process boundary: the tracker is stored with the label record and continues.
The declaration is `SessionFormat::reports_wake_devices` in `hide-agent-adapter`, true for Claude Code alone.
Codex writes no record when a session's command ends, and Grok, Pi, omp, Cursor and OpenCode keep no process boundary, so for them nothing proves a task is running and a label `waiting` with no device is a stop (below).
A turn that just ended is not a stop until its session read lands.
The wake read is bound to the Herdr state it was asked under, so from the moment Herdr reports the stop until that read lands nothing says whether a task will wake the agent.
The label worker owes that read (`PaneRecord::turn_end_owed`, not saved) and the overlay lays it as `RowFacts.turn_end_unread` for that state; the activity axis (`agent_activity`) keeps such a row Working, so no group, mark or phone notification is decided from a stop that may be taken back.
It is owed only to an agent whose read reports background tasks, when its status goes from working to idle or done (or changes between them while still owed), and only while the read can run: Herdr marking a finished row seen, a daemon that restarts and finds its record read, and every other agent owe nothing.
The hold ends on every path of the read and has no timer of its own: the read landing releases it, a read that fails (`ReadFailure`, itself bounded by `DEVICE_READ_TIMEOUT`) releases it with a `turn_end.unread` diagnostic, and the agent working again or a state with no session to read ends it.
A session no read has proved yet carries no facts, so its first turn end is not held.
The wait before the label's own verdict (`stopped_unfinished`) is not this gap: that verdict comes from an AI request that takes seconds, and holding the row for it would delay every Done.

Regression owners: `a_turn_end_is_not_a_stop_until_its_session_read_lands`, `a_read_that_finds_no_task_releases_the_turn_end_and_publishes_it`, `a_read_that_fails_releases_the_turn_end_and_says_so`, `a_stop_that_is_not_a_turn_just_ended_is_not_held`, `a_turn_ending_with_a_background_task_is_never_announced_done` and `a_turn_read_as_done_is_announced_done_past_the_background_task_it_left`, then `a_quiet_row_waits_on_children_then_a_proven_device_then_a_reply`, `a_quiet_root_waits_in_working_while_a_descendant_works_and_not_on_one_that_asks`, `a_root_with_finished_children_draws_its_own_state_past_a_request_a_stopped_child_took_in`, `a_child_asking_its_stopped_parent_by_letter_leaves_neither_row_waiting`, `a_letter_to_a_disconnected_device_holds_no_reply_wait`, `a_proven_background_task_keeps_a_stopped_claude_row_waiting`, `a_reply_is_awaited_until_answered_overdue_or_from_another_session`, the adapter contract `adapters::wake_devices` in `hide-session/tests/it` (the stop pairing, the discarded record that may carry a device, the one that cannot and the call too large to keep), the two `wake_devices.*` diagnostics in `herdr-core/src/labels/tests.rs`, and the web `agentRow.test.ts`.

## Where each axis comes from

Activity and completion come only from Herdr's `agent_status` and `completion_seq`: `working` is Working, `idle` and `done` are stopped, `done` or `idle` with a `completion_seq` reports a completion, and any other value reads as unknown rather than idle.
An agent-list transport failure makes the retained telemetry unconfirmed while the topology stream remains connected.
Until a successful agent list or bootstrap snapshot confirms it again, the agent keeps its identity and task title but has Unknown activity, no completion or current demand, and requires a status check before closing.
An identical successful list restores the confirmed state; a topology event alone does not confirm the entire agent list.
No pane token is read for either.
Demand has these sources:

- A question is the core's label verdict on the agent's last message (`label.question`, see Task identity below).
  It exists only while the label is proven for the pane's current session, and it ends when the agent starts working again or a turn ran between two looks at a stopped agent, so a question never outlives the turn that asked it.
- A native unanswered Claude `AskUserQuestion`, Codex `request_user_input` or `request_user_input_async`, Grok `ask_user_question`, omp `ask` or OpenCode `question` is a question even with summaries off.
  A Grok session whose `plan_mode.json` is `Active` and awaiting plan approval is a plan approval, with `plan.md` as its bounded content, until that file says otherwise.
  The current-session/current-state read carries the optional `user_turn` row fact, with `kind: question|plan_approval` and optional `content: {text, choices, truncated}`.
  Text is capped at 8 KiB, choices at eight and each choice at 256 UTF-8 bytes; cuts preserve character boundaries and set `truncated`.
  A correlated native result, a later human turn or an abort clears the pending question.
  Codex 0.162.1 asks with `request_user_input_async`, whose tool returns `{"accepted":true}` at once while Codex keeps working, so that result answers nothing: the question stays open, as an unread Question in Needs You even while Herdr says the agent works (the demand axis does not read the activity), until a reply names it.
  The reply is a user message `<send_user_message_question_reply>` carrying `questionItemId`, a JSON array `[tool, call, question number]`; each question of the call waits for its own reply, and the match is on that id, never on the question or answer text.
  That message is scaffolding like the other injected prefixes, so it is not a human turn and feeds no label as a request; a person's own message still ends the wait, and a turn that starts without one does not (the question outlives the turn that asked it).
  A reply whose id cannot be placed, or an async call whose arguments cannot be read, leaves the wait unknown rather than nothing.
  Missing content remains absent, and a failed, incomplete or older-state read publishes no structured fact; the independent letter hold stays unknown rather than assuming an answer.
  Native calls are bounded to eight per turn with 256-byte identities; exceeding the bound fails the read instead of dropping a pending question.
- Approval is Herdr's `blocked` lifecycle, whether or not the operator has read it.
- Approval is also a plan waiting for the operator's approval that Herdr reads as an ordinary stop, Codex's "Implement this plan?" (PRD codex-plan-approval-hold).
  The core reads it from the session file, not the screen: the label worker's session read folds the agent's turn records into a turn tracker (`hide-session/src/turns.rs`), and a plan-mode turn that proposed a plan and finished, with no later turn or person's message, waits.
  The answer holds only for the Herdr `state_change_seq` the read was asked under and the session it proved, it is laid on the row's facts (`RowFacts.awaiting_operator`, never on the wire) and resolved by `agent_state::agent_blocked`, and it is not shown while Herdr says the agent works, except for an agent kind whose adapter declares `waits_while_working` (`SessionFormat`).
  Herdr reports Grok `working` while it waits for a plan approval or an answer, and Grok's waits are read from the session as it is now (`plan_mode.json`, a question call with no result yet) rather than inferred from how the last turn ended, so for Grok the wait holds the row, and raises a delegated child as `ChildBlocked`, whatever Herdr says; it clears on the next poll after the operator answers.
  A wait no read has settled for the current state is not shown: the row shows what Herdr says, and the letter doorbell holds instead (`docs/delivery.md`, A menu Herdr reads as a stop).
  It needs no Hide AI: agent summaries off, the read still runs.
  "No, stay in Plan mode" writes nothing to the session, so after that answer the row stays in Needs You until Codex's next turn starts.
  The row's `blocked` value carries it, so it is held exactly like Herdr's `blocked` below, including close confirmation and agent sleep, and the Enter that approves it still counts as the operator's submit, since Codex writes the next turn's message from it.

- A block is the label's verdict that the turn stopped on a cause it can name in one line (`label.blocked`, see Task identity below): a disk with no room, a CI run that failed, a permission or login the agent cannot get, a tool that will not run.
  It is the Blocked state, carried on the existing `error` demand value (`demand`, `status_code` and the mark counts keep the wire name `error`; the word is Blocked), because a block is a demand on the operator's attention and no new value is needed.
  Like a question it exists only while the label is proven for the pane's current session, and it ends when the agent starts working again.
  Herdr's own `blocked` lifecycle is the approval above, not this.

A question outranks an approval, and a block outranks both for the mark.
Whether the operator has read a demand is Hide's own record, never Herdr's tab-scoped seen state.

## The four groups

The sections are named for whose move is next: Needs You (`내 차례`) and Unread (`읽을 것`) are the two the operator acts on; Working and Seen keep their names (status-marks D-11).

| Group | Membership |
| --- | --- |
| Needs You | An unread demand - question, approval or block - or a pane Herdr reports as blocked, or whose native question or plan waits for the operator, right now |
| Done (Unread) | No demand, stopped, completion reported, and unread |
| Working | Running, or a quiet row waiting on a working descendant, a proven background task or a reply still being made |
| Seen | Everything else: ready idle, read demands, read completions, unknown |

A blocked pane, or one whose native question or plan waits for the operator, stays in Needs You whether or not it has been read.
The prompt is still waiting, so it leaves the group when answered rather than when looked at.

A delegated row sits only in Working or Seen, and a Hide AI row for a home only in Needs You or Seen.
Each of the four groups above is then a group of the rows the operator's lists enumerate; a home's row is counted in none of them until it is in Needs You (see Work Hide AI does for a home).

Done is deliberately separate from Needs You: a stop to read is "look when you have a moment", an unread demand is "act now".

An archived session (PRD session-archive) is in no group and draws no mark: nothing of it runs, so it has no turn, no progress and nothing to read, and it is counted on no badge.
It stands only in the 태스크 list's `보관됨` fold and its record, with its archive state (archiving, resuming, could not resume, deleting, unknown) as words; a member that came back is an ordinary row again, and a member still archived while its root is live is listed in that root's record, not as a row.
While a member is archived, its delivery registration and the watches on it are held as a sleeping session's are (`docs/delivery.md`), so its pane's absence does not end its work's ownership; resuming continues them.

## The mark

Every row draws one of eight marks, decided once in the core (`RowMark::of` in `agent_state/tally.rs`) and carried on the wire as `mark` (`turn`, `going`, `waiting`, `done`, `unread`, `quiet`, `unknown`, `asleep`); no surface derives a mark of its own (status-marks D-01, D-12), the dimming of an unknown row included (`rowDimmed` reads `mark`, never `status_code`; sidebar-quiet-mark-indent D-04).
`going` and `waiting` are the one in-progress mark in two shapes (D-25): every count and precedence takes them as one, and only the ring is drawn apart.
Four say whose move is next, and their shapes alone tell them apart; `unknown` says Hide cannot tell, and `asleep` is an agent Hide stopped that wakes when opened (D-21):

| Mark | Shape | When |
| --- | --- | --- |
| `turn` | A `!` in a 12 px warning badge | The operator's turn: the row is in Needs You or carries an escalation, or it is an own question, approval or block the operator has read (drawn at `--opacity-read-status`) |
| `going` | A static three-quarter ring in the working colour | In progress: the agent works |
| `waiting` | The same ring dashed, also static (`2.2 2` over a path length of 19) | In progress while the agent waits (`wait`) on children, a background task, its pull request's checks or a reply; its hover and name read `진행 중 · 대기` |
| `done` | A 7 px green dot | Unread, and the label read the turn as finished |
| `unread` | A 7 px dot in the foreground colour | Unread, with work left (a wait nothing proves is work left), or no verdict |
| `quiet` | A faint hollow ring in the 12 px slot: radius 3.4 of 12, a 1.3 stroke in the muted foreground at `--opacity-read-status`, smaller and thinner than the in-progress ring (sidebar-quiet-mark-indent D-01, D-02) | At rest: read, idle, a delegated child's own request |
| `unknown` | Nothing; the slot is kept so titles line up, and the row is dimmed whole | No demand and an activity Hide cannot read (`status_code: unknown`, unless a block, a raise or an escalation makes it `turn`); counted on no badge (sidebar-quiet-mark-indent D-04) |
| `asleep` | A grey moon (`☾`), not dimmed | An agent Hide put to sleep, waking, or one that could not be reopened; counted on no badge |

The ring does not turn (D-02): its dashes, not motion, tell waiting from work, on every surface, the ones with no second line (tabs, pane headers, the ⌃Tab switcher, ⌘K, the child tree popover) included (D-25, B36).
When the agent starts working again the dashes close, and when a turn ends with a wait left they open.
A delegated row draws only `going`, `waiting`, `quiet` or `unknown` (B24): its question is its parent's to answer; one of the six escalation causes still makes it `turn`.
Colour never carries the distinction alone: the mark's name and the word that leads its line are its hover text and part of the row's accessible name (`markWords` in `web/src/agentRow.ts`), as in `내 차례 · 질문` or `읽을 것 · 일 남음`.

How a stop ended is the label's verdict (`axes.rs::stop_verdict`): finished, or unfinished (work left); a label `waiting` that nothing proves is running is work left too, since nothing will wake the agent (status-marks D-17).
Only a finished verdict draws the green dot and the word Done (`끝남`); work left is `Unfinished` (`일 남음`), so a plain dot means stopped with no confirmation that the work is done; a stop no label judged (summaries off, a failed or pending analysis) is `Stopped` (`멈춤`), said only on hover, in accessible names and in notifications, and its line is empty (D-03, D-04).
Read, a stop with work left keeps its line, subdued, with its word (`line.mode: kept`, B7).

A line leads with the word for what is asked or how the turn ended (`질문 · …`, `승인 · …`, `막힘 · …`, `끝남 · …`, `일 남음 · …`); an Unread row's line is only ever `끝남 · …` or `일 남음 · …` (B6).
A working row says its sentence alone or has no line; a waiting row's line is `대기 · ` and its sentence, or with none the proven cause: `대기 · 백그라운드 작업`, `대기 · 자식 N명 작업 중`, `대기 · 답장`, `대기 · CI · PR #N` (B10, B29).
The web splits a line into its lead word, sentence and background tail (`lineParts`), and a narrow row cuts only the sentence, so the lead and the tail always show.
A Needs You line's ask verb says the same word: an Answer reads `질문` (Question) and an Approval `승인` (Approval), followed by ` · ` and what to do.
A row of unknown state, or on an unreachable device, has no mark and is drawn dimmed whole, and its line says why (B16).
A slot with no live agent behind it, an ended session in a record or a link's timeline, a closed feedback recipient, draws no mark either (sidebar-quiet-mark-indent D-05): the ring says an agent is there and resting.
A row on a device that is not connected still carries the last state Hide knew of it, which may be long over; while `status.remote` says the device is not connected, the shell draws that row and a tree popover's child with no mark, dimmed, with the line `<device> 연결 끊김` (`agents.line.disconnected`), and its hover and accessible name say the same instead of the stale mark words.
A sleeping, waking or failed agent's row, and a saved sleeping conversation's row (`SleepingSessionRow`, the phone's sleeping row), draw the grey moon and are not dimmed (B31): a sleeping one has no line, and its hover and accessible name say `재움 · 열면 깨어남`; a waking one says `깨우는 중` and a failed one `재움 · 다시 열지 못함` on its line.
The operator's turn outranks the moon: a sleeping row with a raised ask draws `!`. These lines and the moon are the same in the Agents tab, the child tree popover, the pane header's child summary, the close list (where the moon is not dimmed) and the phone (`agentRow.ts::stateLine`, `phone::line`); a sleeping row keeps the place and close-list class its awake mark gives it (`RowMark::awake`).

Regression owners: `precedence_is_the_turn_then_progress_then_a_stop_to_read`, `a_dot_to_read_is_green_only_on_a_finished_verdict`, `a_finished_turn_is_not_hidden_by_the_background_tasks_it_left`, `a_sleeping_row_draws_asleep_and_counts_on_no_badge` and `a_waking_failed_or_unknown_row_keeps_a_state_line_on_the_phone` in `agent_state/turn.rs`, `web/src/agentRow.test.ts`, and the render tests that every list draws the same line: `web/src/components/row-line-text.test.tsx` (the sidebar row), `web/src/AgentSessions.lines.test.tsx` (the Agents tab) and `web/src/PaneHeaderBand.test.tsx` (the child tree and the pane header) and `web/src/mobile/list.test.tsx` (the phone list).

## Close protection is separate from read state

`requires_close_confirmation` and `requires_close_status_check` are core-derived values carried by both the sidebar agent row and its pane projection.
The confirmation value is true for Working activity, an unresolved demand, or a blocked pane; a stopped unread completion does not create a work-interruption prompt.
The status-check value is true only for Unknown activity with no demand and no block.
It prevents a destructive local or remote close from guessing that an unobserved agent is idle, and the caller-visible remedy is to refresh status before closing.
The shell presents that remedy as the existing read-only `Check status` action and keeps the destructive close confirmation separate from it.
An unknown row still belongs to Seen for sidebar grouping, so close safety never changes the read or ownership axes.

A row with live descendants also carries `close_descendant_pane_ids`: every live descendant's pane in the order a subtree close takes them, deepest first and each pane after every pane below it, siblings in the lineage's own child order.
`apply_lineage` derives it on the same pass as the tree, and `refresh_agent_lineage` drops the panes of a device that is not connected, so a close never lists what it cannot reach; a row with none omits the field.
A pane or tab close asks about the subtree only when some agent that closes lists a pane outside what closes (PRD close-agent-subtree D-16), and the status check above applies to each listed descendant: an unknown one keeps `모두 닫기` disabled, while `이것만 닫기` stays available.
Closing a parent alone does not close its children: they lose their parent on the next projection and become the operator's roots with their own state.
Regression owners: `each_row_lists_its_live_descendants_deepest_first_and_a_leaf_lists_none`, `a_descendant_on_a_disconnected_device_is_never_listed_for_a_close`, and the tree-close tests in `herdr-core/src/runtime/tests/tree_close.rs`.

Needs You and Done ordinarily belong to operator-owned roots.
A delegated row stays Working or Seen while its parent can handle it; `agent_state/escalation.rs` raises it to Needs You only under the six conditions below.
The row keeps its own demand, mark and status word, and its lineage root carries the ask (Delegated escalation below); what changes is only which group the row sits in and whether it is drawn bright.
Done is therefore scoped to the lineage root: a delegated child that finishes leaves a dimmed Seen row, and the completion the operator acts on is the root's.

## Where a parent comes from

Ownership, the tree, the ancestor path and the descendant mark all start from one fact per agent: its responsibility parent.
`hide agent spawn --parent here|<self id>` records delegation; omitting `--parent` hands the agent to the operator as an independent root.
The latter has no indentation, from hint, parent line, descendant contribution, ancestor unread propagation, waiting-on-descendants effect or inclusion in the spawner's subtree close.
Its own demand and completion enter Needs You and Done normally, and closing the spawner leaves it running.
The CLI's required `origin` field records the spawner in both modes (null for ordinary roots), but origin never enters the UI or any ownership calculation.
PR and issue panels still find the independent session through its own branch records without a delegated-by line.
Herdr records no lineage, so hided writes the four pane tokens through `pane.report_metadata`: `parent_pane` names the parent's pane id, `parent_machine` names its machine when the parent is remote, and `child_session` and `parent_session` are the original session digests.
The value and lifetime contract is unchanged.
A fork made with Fork agent or Fork agent to the side is the operator's own root, not the source's child: its registration has no parent, names the source as its `origin`, and carries the `forked` fact (`AgentRecord::forked`), so `hide agent show` and `hide agent list` print a null `parent` and the source as `origin` (PRD fork-operator-root B1, B19, D-12).
Only the source's own live registration can register a parentless fork with itself as origin; any other caller is refused `parent_authority_required`.
The source is registered when the fork's agent starts, and the fork once Herdr shows the fork's own session: Codex started as `codex fork <id>` reports its session only at its first turn, so a Codex fork left at rest has no session, no `forked_from` and no registration until the operator gives it a prompt.
Meanwhile the registration waits in the ledger, the source's consent kept as the wait, so the fork names the source as origin even when the source closed while it rested, and the wait ends with the fork's pane or agent ([delivery.md](delivery.md#a-registration-that-waits-for-its-agents-session)).
A fork that cannot be recorded keeps running without its mark: when the source's registration or the wait is refused (`identity_waits_full` past 64 waits, `capacity` at the 2048 registrations the ledger holds, `registration_conflict`), the fork's row is an ordinary root of the operator's with no `forked_from`, the source shows nothing of it, and the refusal is logged as `fork` `registration_failed` with both panes.
Nothing reaches the screen, by design principle 13 and because an alert is a PRD decision: the fork works, and no step the operator could take afterwards gives it the mark, since a fork is not asked again once a slot frees, so the log is where a lost mark is looked into.
Its pane names the source under three keys of its own, never `parent_pane`, which would make it a child again: `fork_pane` is the source's pane id, `fork_session` the fork's own session digest and `fork_source_session` the source's (`lineage::FORK_KEYS`).
The lineage writer writes them from the registration as it writes the four above, so a wake that continues the fork in another pane names the source there too, and a hided restart finds them on the pane and in the ledger (B17, B18).
They are read only while the fork's pane still runs `fork_session`, so a pane that hosts another conversation later is not a fork; `wire::fork_declaration` reads them, and `agent_state::apply_fork_sources` names the source while its pane still runs `fork_source_session` and names none once the source closed or moved on (`SidebarAgentSnapshot.forked_from`, `PaneForkSnapshot.forked_from`, B9, B10).
Because none of these is a parent token, the fork has no depth, so the source gets no child, unread, Working, descendant mark, child count or subtree close from it, and the fork's own demand and completion enter Needs You and Done (B12, B13, B14).
A fork of a fork names the fork it was made from, and a fork of a delegated child is still a root (B15, B16).
A fork made before forks were roots keeps its parent and its old `fork` token, which nothing reads any more: it stays a child and wears no mark (D-09).
The session tokens are lowercase hexadecimal SHA-256 digests of each original `agent_session.value`; this keeps even a path-valued session inside Herdr's 80-character token limit.
A same-server relationship omits `parent_machine`.
A changed child session clears all four tokens, while an absent session proves neither a valid relationship nor a session change.
Ending registration leaves the tokens until the child pane changes or disappears.
A registration ends by itself when the host's authoritative pane topology stops reporting its pane, as a read record is dropped, and never from an incomplete or stale read ([delivery.md](delivery.md#agent-registration-and-spawning)).
`hide agent list` then reports it `ended` and `disconnected`, and its name is free on that host.
A relationship is accepted only when both panes still report the recorded session digests; without both digests the child is a root until a complete declaration is written.
A changed parent session cannot adopt the previous session's children, and an absent parent agent retains the existing orphan presentation.
A missing or unknown remote machine identity leaves the child a root until its matching device connects.
The operating-system identity is the platform UUID on macOS and the machine-id on Linux; cloned machines need distinct identities before they can safely resolve different parents.
`hide agent register`, delegated `hide agent spawn` and Hide's fork record the relationship, or for a fork its provenance, in the core's coordination ledger.
A delegated `hide agent spawn --machine <device id>` child is a remote child of a local parent: its tokens are written on the device's pane and `parent_machine` names the spawner's machine, the same resolution as any cross-machine relationship above.
The existing one-second agent refresh writes only panes whose tokens differ, with a complete reconciliation on startup or reconnect and an immediate write after spawn.
The same writer writes the read axis's server mark ([Across a Herdr restart](#across-a-herdr-restart)), in the pane's lineage report when it has one.
`wire.rs` and `agent_state/axes.rs` remain the readers of this contract.
A connected device's immutable machine identity comes from its consented helper's connection greeting, outside the runtime mutex; an unavailable identity leaves the parent unresolved and records a diagnostic.
A declaration whose machine cannot be matched stays a root and records one diagnostic for that pane instead of guessing.

A pane outlives the agent it hosted and the tokens outlive the agent with it, so a declaration is only a claim until the sessions prove it: it holds while the child's pane reports the `child_session` and the parent's pane the `parent_session`.
`wire.rs` checks the child as it turns the tokens into a row, and `agent_state::apply_lineage` checks the parent because that is where both rows are known, on this machine or another.
An agent that took over a pane is therefore a root, and a parent pane taken over by another agent adopts none of the old children: no line, no descendant mark, and no orphan hint, since the child was never that agent's.
A pane Herdr reports without a session cannot prove a match, so its relationship does not hold until it reports the recorded session again, and a `parent_pane` without the session tokens is not a relationship at all.
A parent whose pane no longer lists an agent is a different case: the child stays an orphan root with its hint.

An empty token is a cleared declaration, not a parent named by an empty string.
The token is display-only in Herdr's own terms and dies with the pane, so a closed child leaves no edge behind, and a parent that has gone makes the child an orphan root.

Fork regression owners: `a_fork_registers_as_a_root_whose_origin_is_its_source` (`coordination/mod.rs`), `a_fork_names_its_source_under_fork_keys_and_never_as_a_parent` and `a_fork_that_is_woken_in_another_pane_names_its_source_there` (`coordination/lineage.rs`), `a_fork_names_its_source_and_stays_the_operators_own_work` (`runtime/tests/lineage.rs`), `a_fork_whose_session_arrives_late_registers_as_the_operators_root_from_its_source` and `a_fork_whose_pane_closes_or_agent_ends_before_its_session_ends_its_wait` (`runtime/tests/fork_registration.rs`), the decisions in `coordination/identity_wait.rs`, and `web/e2e/agent-fork.spec.ts`.

Regression owners: `a_parent_declared_as_a_pane_token_is_the_lineage`, `a_child_whose_pane_now_hosts_another_session_declares_no_parent` and the `lineage_sessions` tests in `herdr-core/src/session_sync/tests.rs`, `a_parent_pane_on_another_machine_reused_by_another_agent_adopts_no_local_child`, `a_sleeping_child_stays_under_its_parent_only_while_its_record_can_prove_it`, and `web/e2e/lineage-session.spec.ts`.

## The descendant mark and the tree popover

`agent_state/escalation.rs::lift` gives each row one `descendant_mark` over its whole subtree (PRD D-40): `raised` with the number of raised descendants if there are any, else `working` with the number of working descendants, else none.
The shell draws only the `raised` kind, as the turn mark and N, and only where those descendants are hidden: a folded sidebar parent, a 태스크 row whose children are folded.
The `working` kind is still computed and draws nowhere: a parent waiting on its children shows its ring and the working child's line (PRD session-record-redesign D-04).
Idle, done and unknown descendants add nothing, so a mark never says only that children exist.
`row.state.tree_rank` orders siblings in every tree: asking or raised, itself or below it, first; then working; then finished and unread; then the rest.
The sidebar tree under an opened root is `agent_scope.sidebar_tree.visible_rows` from `agent_state/tally/lineage.rs::sidebar_tree`: children and grandchildren wherever they work, two levels only, with the digit shortcuts on roots alone.
The shell shows five siblings per parent and folds the rest behind `N개 더` (`web/src/sidebarTree.ts`).
A grandchild's own children open in the tree popover instead of a third level.
A delegated row whose checkout differs from its parent's carries `row.state.branch_badge` (`agent_state/axes.rs::apply_lineage`): that checkout's branch, or its label when it has none, the name the sidebar gives the checkout; a device checkout's label is its folder name, so the label alone would name a folder there.
The tree popover (`web/src/components/agent-tree-popover.tsx`) opens from the children button at the right end of the pane's second row, a grandchild's chevron and an ask band's fallback `외 N건` when that button is unavailable.
The children button reads `자식 N`, after the subtree's operator-raised `descendant_mark.count` with the turn mark when it is positive; its tooltip and accessible name spell out the live direct child total and every `direct_child_counts` state, and ordinary child requests are named only there, because they are the parent's to answer (status-marks B21).
Each child's own state is the mark beside its name on the same row, the mark its sidebar row draws; unknown children are excluded from the resting remainder of the tooltip.
Its head names the parent with its direct child count, and its body draws the same rows two levels deep with five siblings each, the children that finished and were read, with nothing below them raised or working, folded below them behind `끝난 자식 N`; choosing a grandchild that has children re-roots the popover on it.
Arrows move, Right and Left open and fold a branch, Enter opens the agent's pane, and Escape closes it and returns focus to what opened it.
Opened from a pane button with a raised mark, or the fallback ask-band button, it starts with the raised branches open and draws each ask on the row it raised (`raised_pane_id`) with its verb, what to do and Open, which for a draft goes to the parent holding it.
A pending or failed move shows at its footer with the retry rules of the relation, and the last line, `Overview에서 일감 보기`, opens the Overview on the agent's device with its task's card selected and the agent's own record beside it.
Disappearing children leave immediately and an empty popover closes.

The one sidebar raises Needs You and Done above its project tree, at most five and three most recent roots respectively, with older rows behind More.
Those rows also remain under their checkouts, and a shortcut belongs to the first visible occurrence in physical sidebar order.
A checkout starts open; `session_collapsed_checkout_ids` remembers only explicit collapses, independently of older disclosure records.
A collapsed checkout still shows its Needs You rows.
Core `session_folds` puts agentless worktrees behind No agents, excluding the primary, front, dirty, unpushed and already inactive checkouts, lists in `child_only` the worktrees (same exclusions) whose unresolved agents are all delegated children, which the sidebar draws nowhere because the parent root's tree reaches them, and collects agent worktrees and missing folders behind Cleanup at the bottom.
`session_open_folds` remembers explicit openings across startup and device reconnects, including an intermediate empty catalog.
A confirmed project unregistration removes its project key; removing a device removes its project keys and Cleanup key.
Resolving a session removes it from sidebar membership without changing its pane, tab or task membership.

Regression owners: `web/e2e/sidebar-status.spec.ts`, `web/e2e/projects-sidebar.spec.ts`, `web/e2e/session-panel.spec.ts`, and `runtime::tests::agent_scopes`.

## Shared agent and Workspace status contract

The sidebar hierarchy is Project > Workspace > Agents.
A Workspace is one checkout path, including a plain folder; internal `WorkspaceSnapshot` names the project and `CheckoutSnapshot` names this Workspace.
A branch is the Workspace title when available; otherwise its real folder name is the title.
`primary`, `detached`, `missing`, temporary state, and uncommitted changes describe the checkout, not an agent lifecycle.
They remain separate badges or Git indicators and never select the agent status color.

### One meaning across surfaces

Agent rows, focused-agent tab marks, pane headers, the tree popover, the ⌃Tab switcher, the close sheet, ⌘K results, the Sessions tool, the Overview and its task cards, the pull request lens, the View comment menu, the Hide AI home row and the phone draw the same mark and the same words for the same row (status-marks B20), from the core's `mark` and `status_code` (`web/src/components/status-mark.tsx` draws the six shapes in one box of one size, and an empty box for `unknown` and for a slot with no live agent).
Working uses a fixed blue status token, independent of the user's accent color; the finished dot uses green, so running and completed work remain distinct.

A read question, approval or block keeps the turn mark and its request line at `--opacity-read-status`, so an unread one stands apart from one already looked at.
A blocked approval stays in Needs You and keeps full emphasis until it is answered.
Reading is acknowledgment, not evidence that a demand was resolved.
A read, non-blocked demand belongs to Seen unless the core places its running activity in Working; the mark still says it is the operator's turn.
Disconnected presentation overrides the retained mark and text on every affected surface without modifying demand, activity, read records, or the last known group: the row draws no mark, is dimmed, and its line names the device (`<기기> 연결 끊김`).
Unavailable-server tooltips describe the connection problem rather than presenting retained counts as current work.

A sleeping Claude Code or Codex agent keeps its row, name, place, lineage and badge while Herdr no longer lists it: the core draws the row from its own sleep record (`herdr-core/src/agent_sleep.rs`), because the pinned Herdr forgets an agent and its session reference once its process ends.
Only a seen, stopped, local Claude or Codex agent with no demand and a reported conversation can sleep, so a sleeping row is always in Seen; a delegated child sleeps by the same rule as its parent and keeps its place under it.
A new agent in the pane, whether a wake, the operator or a restore started it, is awake: its `state_change_seq` differs from the one the record kept, and the record is dropped.

A provider with an implemented close-pane sleep dialect instead retains a dormant conversation keyed by `SleepId`, separate from the live agent projection.
Its core-owned group is Seen, and it contributes no active pane, lineage, unread state, doorbell target or executable input address.
Saving and closing do not duplicate the existing live row; confirmed closure publishes the separate sleeping row, and an uncertain operation publishes its explicit status action.
Wake resumes the saved native conversation in its own intent-marked tab, and current reader identity must confirm that conversation before the archived row is removed.
A delegated child that held a registration wakes into the same registration, so its row returns under its parent and its parent's watch and letters still reach it ([delivery.md](delivery.md#agent-registration-and-spawning)); the sleeping row itself never carries lineage.
A Cursor wake needs no session hook from Cursor: the core reports the resumed session to Herdr itself, so the row returns under its parent like any other kind's ([ARCHITECTURE.md: Agent sleep](ARCHITECTURE.md#agent-sleep)).
The Wake action is available only when this build declares the provider's resume capability.
A record an older build saved names no checkout owner ([ARCHITECTURE.md: Agent sleep](ARCHITECTURE.md#agent-sleep)), so its Wake is offered only while the project list shows its checkout; until then the row has no Wake and its reason says to open Hide on that computer and add the project if it is missing, and the Wake returns once the checkout is listed.
A record this build saved wakes from the owner it kept, whatever the list shows.
Regression owners: `a_sleep_keeps_its_checkouts_owner_for_the_wake`, `a_sleeping_worktree_session_is_opened_from_its_repository_while_no_list_shows_it`, `a_sleeping_session_wakes_in_its_checkout_while_no_list_shows_it`, `a_sleeping_session_an_older_build_saved_offers_wake_only_while_its_checkout_is_listed` and `a_sleeping_session_offers_wake_only_over_a_connection` in `herdr-core/src/runtime/tests/agent_sleep.rs`, and `a_kept_owner_names_its_repository_by_an_absolute_path` in `herdr-core/src/agent_sleep/dormant.rs`.
A retained unsupported sleeping record stays visible without a Wake action or a loading spinner; only an active transition or status check spins.
Unknown work survives restart without replaying external effects; an intent saved before close admission can be released without touching a pane.
`agent_sleep/dormant.rs` owns the bounded archive and row values, while `runtime/agent_sleep/dormant.rs` owns its transitions and exact save/connection/execution fences.

### Workspace aggregation

`agent_state/tally.rs` owns Workspace aggregation from the canonical agent projection after pane-level read state is applied.
Each Workspace counts unique agent pane IDs physically owned by its tabs, independent of sidebar visibility, raised rows, parent collapse, or Workspace collapse.
The existing status synchronization indexes pane ownership once and visits each canonical agent once; it adds no timer, I/O, or per-frame work and publishes only changed summaries.
A descendant running in another checkout contributes to that checkout, even if its lineage row appears beneath a parent elsewhere.
An agent repeated in a raised section and the project tree counts once.
Plain terminal panes without agents do not create an Idle agent status.
A Workspace with no known agents draws no summary chip.
A populated Workspace places its representative status in a trailing chip, followed by `+N` for the remaining agents (omitted for a single agent).
The chip uses the canonical physical agent count, even when agents are also raised above the tree.
An agent whose lineage starts in another checkout also appears as a local root in the Workspace that physically owns it, so that Workspace can reveal the agents its chip counts.
Rendering and shortcut numbering share this ownership-aware tree projection.

The representative follows the same group order as the sidebar: Needs You > Done > Working > Seen.
Within a group, Blocked precedes Approval, then Question; otherwise Unknown precedes ordinary Idle within Seen, so missing information is not hidden by an idle sibling.
Equivalent candidates keep canonical agent order.
A read block in Seen cannot outrank an unread question in Needs You.
The Workspace draws the representative agent's exact mark, color, and emphasis through the shared presentation.

The web shell draws no representative chip; it draws a status badge in its place.
The same pass counts each agent under the mark its own row draws (`RowMark`): `turn`, `going` (a `waiting` row counted here too, D-25), `done` and `unread`.
A quiet row is counted in none of them, so the badge claims nothing but whose move is next.
The counts ride the summary as `marks`; the web sidebar draws them on a checkout's row only, and a project or folder row draws none.
The badge draws one mark and count per mark, in the order whose move is next (turn, in progress, finished to read, to read), zero ones left out, in the shapes and colors the rows use, so it says what opening the rows would show (B19).
A checkout's badge stands for its folded agent rows and leaves while they are open (docs/UI_BEHAVIOR.md, Sidebar hierarchy).
It is the only count badge: a folded parent row, in the 프로젝트 tree or the 태스크 list, counts no children and draws only `! N` when a hidden descendant asks the operator (status-marks D-24, B35).
Regression owners: `workspace_summary_uses_physical_ownership_priority_and_unique_panes` for the counts, and `projects-sidebar.spec.ts` for where the badges are drawn.

The tooltip lists positive counts in group order: Needs You, Done, Working, Seen.
Unknown is reported as a subset of Seen, never added again to the total.
When the owning server disconnects, a Workspace with retained agents displays Disconnected and suppresses the stale activity breakdown.
Connection recovery resumes the current canonical projection; it does not mark agents read.

### Disclosure and selection

The disclosure chevron sits at the right edge of a Workspace row.
The left edge holds the checkout-kind icon, title, and checkout badges.
The representative status moves into the trailing agent summary chip immediately before the chevron.
A root agent status mark aligns with the Workspace branch icon; compact agent rows use a 4pt gap between status and title and draw no provider logo unless the operator's agent logo view mode shows it or an ⌥ hold peeks at it (the pane header always does; PRD agent-mark-quiet, PRD agent-logo-view-mode).
A Workspace with nested agent rows uses the entire row, including its name and empty space, as the disclosure hit area.
Its right-edge chevron is a non-interactive indicator within that same button, not a second small control.
A Workspace without nested agent rows shows no chevron and clicking its row opens the Workspace.
Select an agent to focus its pane; expanding or collapsing a populated Workspace only changes the tree.
Collapsing does not change the selected pane, tab, agent read state, running processes, or aggregated status.
`collapsed_checkout_ids` persists across launches.
The web shell starts every checkout open and keeps explicit collapses in `session_collapsed_checkout_ids`, changed by `checkout_agents_toggle` and the admitted `focus_checkout` event.
The older `expanded_checkout_ids` and `collapsed_checkout_ids` records do not determine web checkout disclosure.
Inside an open checkout, actual tabs group complete root subtrees as segments of one list: every root stands on one status axis, a guide per tab marks the boundary, and the tab line at a segment's head appears only when the tab has a sidebar root and either holds at least two recognized agent panes or carries a manual name.
The glyph's count describes physical agents in that tab, independently of root ownership or child disclosure; raised groups and sleeping sessions retain their existing positions.
Tab names use the core's explicit manual-name and stable-number metadata rather than an automatic task label, and no tab fold is saved.
Raised Needs You and Done rows remain available, while number shortcuts skip hidden tree rows.
In the web shell the fold controls of a project, a checkout and a parent agent all sit on the right of their row in slots kept at rest; a folded control is always shown and an unfolded one appears under the pointer, with focus inside the row, while the row's menu is open, or on an input with no hover.
The body of each row navigates (a project to its most recent usable checkout, or to its panel on the Overview's Projects when it has none (PRD sidebar-task-lens B41), a checkout to its Workspace, an agent to its pane); a project or checkout row also unfolds its children, or folds them when its scope is already in front and unfolded, and a fold control never navigates: folding from a chevron changes no screen, pane, tab, read state, group or process.

### Verification ownership

Core status tests own the Done mark, representative priority, unique-pane counts, cross-checkout ownership, and unchanged read semantics.
Core close tests own the separation between work confirmation and unknown-status blocking, including local and remote close refusal.
Manual acceptance in the desktop app covers fixed semantic colors and the shared disconnected override, mixed states, Done-to-Idle acknowledgment, right-side disclosure, unchanged terminal selection on collapse, empty and missing workspaces, and disconnect/recovery.
Run evidence belongs under `agents/runs/`, never in `docs/`.

## Pet pose priority

An error or an unread demand takes precedence over ordinary work, so a `!` or `?` is never hidden by a background task.
The behavior layer adds the clawd-style priority used for pose selection (`herdr_core::pet::expanded_state`):

```
error > notification > sweeping > attention > carrying/juggling > working > thinking > idle/roam > sleeping
```

The current Herdr data has no separate sweeping or thinking token, so those slots remain reserved and fall through to the existing status.
One working pane maps to `carrying`, two or more to `juggling`.
After eight idle seconds the pet can roam; after sixty idle seconds it runs `yawning -> dozing -> collapsing -> sleeping`.
Any pointer activity produces `waking` before returning to the normal priority.
The pose ladder is the one place an error is counted apart from the rest of Needs You, because the ladder puts it a rung higher; no count on any surface is drawn from that split.

A lost Herdr connection outranks all of it: `herdr_core::pet::pose` reports `disconnected`, because a server that stopped answering cannot say anything true about agent state.
The last valid agent list is retained so counts do not blink to empty, but every retained agent is counted as disconnected - a stale yellow "act now" badge for a dead server is the failure this prevents.

## Badges

The pet's badge row counts three of the sidebar's groups, in the same order; Seen has no badge:

| Order | Color | Group |
| --- | --- | --- |
| 1 | yellow | Needs You |
| 2 | green | Done |
| 3 | blue | Working |

A count of zero hides that badge.

The pet's "act now" number is the whole Needs You count and its done number is the whole Done count, so a badge can never disagree with the section it stands for.
`herdr-core/src/agent_state/tally.rs` counts the groups the projection already decided rather than reading tokens or axes a second time.
The pet dashboard's count tiles read the same four groups, plus the rows whose server stopped answering.

## The subagent badge

The pet's badge row carries one more count after the three groups: the in-process subagents Hide's hook reports as working, in purple.
The pet is its only reader: the desktop app draws no pet today, and the web shell draws no subagent count on any surface ([UI_BEHAVIOR.md, Subagent count](UI_BEHAVIOR.md#subagent-count)).
`agent_state/tally.rs::subagents_active` sums the `working` hook token over the agents on an answering server, with saturation, and returns zero while disconnected; a pane whose agent has gone is not counted even if its token lingers, and an instrumented pane whose count is unknown adds nothing rather than a zero.
It is the one count the hook can vouch for; Herdr's own wire carries no ambient counts, and nothing here scans transcripts or output.

Regression owner: `subagent_counts_sum_the_hook_tokens_of_listed_agents_and_go_quiet_while_disconnected`.

## Uninstrumented is not an unknown activity

A pane whose agent Hide cannot see into is a different answer from a pane whose activity Herdr reports as `unknown`, and the two are kept apart on purpose.

- Activity `unknown` is Herdr saying it does not know what the process is doing. It is one of the three activity values and it groups like any other.
- Uninstrumented is Hide saying it cannot tell what that session has spawned. It is not an activity, it never changes a group, and it is projected as its own field beside the agent.

The reason is resolved once, by `hide_agent_hooks::diagnosis::instrumentation`, in a fixed order: config unreadable, remote host, hooks not installed, session predates install, hook outdated, unknown.
The first match wins and nothing falls through to an empty value or an invented cause.
Every projection carries the reason's stable code alongside its sentence, so no surface has to recognise its own operator-facing text.

The projection is made only for panes where an agent was detected, and no screen draws it as a mark: it would say Hide cannot tell how many subagents the session runs, which the operator can do nothing about, so the pane header, the sidebar row and the Overview line carry nothing for it (operator decision on issue #810, design principle 13; [UI_BEHAVIOR.md, Subagent count](UI_BEHAVIOR.md#subagent-count)).
The core still projects it (`PaneChildren.instrumented`, the reason and its code), because the connection below is judged from the code.
A count Hide cannot read is reported as unknown and never as zero, because a zero is a claim that the agent is working alone.
An agent whose adapter declares no subagent count (`subagent_counts: None`: Pi and Cursor) has nothing to read, and the core projects it with the reason `unknown` and no count.

### Not connected, and what fixes it

A Claude Code or Codex pane whose session Hide does not hear also carries a connection (`PaneChildrenSnapshot.connection`), read from the same observation as the projection above (PRD settings-cleanup D-09, D-11, B26 to B31).
It is judged by `sidebar::pane_connection` from `uninstrumented_code` for an awake agent whose native session Herdr has reported.
A process detected before its native session is not a failed integration and carries no connection judgement yet, without a timer or an assumption that it connected.
A sleeping agent has ended its process, so no hook can speak from its pane, and it carries no connection until it wakes.
Settings counts sessions and never these connections, so a pane that needs a Reopen says so only in its own header.

| Instrumentation | Connection |
| --- | --- |
| instrumented | `connected: true` |
| `session_predates_install`, a Codex and the machine's shared server on, or its daemon still answering | `codex_shared_server` |
| `session_predates_install`, any other | `started_before_hide` |
| `hooks_not_installed`, `config_unreadable`, `hook_outdated` | `setup_needed` (fix it in the agent's row in Settings; Reopen would change nothing) |
| `hooks_switched_off`, `unknown`, an agent with no hook, or an agent asleep | no connection, so no chip |

A Herdr live handoff or restart does not move a connected pane to `started_before_hide`: the tokens the instrumented judgement reads go with the old server, and hided puts them back from the pane's record within a few agent reads, without a hook event ([agent-hooks.md](agent-hooks.md#after-herdr-hands-off-or-restarts)).
Until then, and for a pane on a device, a handed-off pane reads as a session that predates the install.
`can_reopen` is false for `setup_needed` and for a pane on another device, because a Reopen restarts the session through this Mac's Herdr.
The shared server is the machine's last kit read (`KitSnapshot::shares_codex_server`): a read that says the setting is on, or that a daemon still answers with the setting off, is what turns a Codex pane's reason into the shared server, and a later read that says neither turns it back into `started_before_hide` at once (PRD codex-daemon-apply D-07, B9).
After a turn-off that answered `stop_failed`, only a read that says no daemon answers ends the shared server, so a daemon answer Hide could not read keeps the retry on offer (B7).
A kit read happens at launch and on each device connection, on a Reinstall, when a Settings tab showing the kit opens and on this Mac every 5 seconds while it stays open, and in the answer to a turn-off request.
Nothing else polls for a daemon, so one that another app starts later reaches the pane's reason at the next of those reads, not live.

Reopen is one event, `pane_reopen { pane_id }`, and reuses the session-sleep path rather than a second one: the agent is ended the way sleep ends it and started again in the same pane with its own resume arguments, a Codex with `--no-daemon` first (`herdr-core/src/pane_reopen.rs`).
Everything knowable before the agent is touched is refused before it is touched, so a refusal known at that point leaves the pane as it was: an agent that is working or waiting (`agent_busy`), a session Herdr never reported an id for or one with no resume arguments (`session_gone`), a Codex whose capability was never read (`codex_unread`), and a folder that is gone (`start_refused`).
The agent is read again just before it is signalled, because the list the decision used can be a second old: one that has started working or is waiting since is `agent_busy` too, with nothing signalled.
Any other end Herdr or the agent refuses is `end_refused`, and a start Herdr refuses after the end is `start_refused`.
The pinned Herdr keeps an ended agent's name for a moment, and refuses `agent.start` under it as `agent_name_taken` until it lets go; a Reopen, and a wake after a sleep, send the same start again on exactly that refusal for at most ten seconds (`agent_start::start_at_shell_reusing_name`), never before the end has been confirmed, and a name that is still held then is `start_refused`.
Every other start takes that refusal as Herdr's answer, because its name is someone else's.
The core keeps one entry per pane, so a second press while one runs starts nothing; the pane's `connection.reopen` is `{"state":"pending"}` while it runs and `{"state":"failed","reason":<code>}` after a refusal, and the entry goes with the need for it: when the pane connects, when it leaves, or when it shows no chip.
A start that lands publishes nothing more by itself: the session's own hook reaching Hide is what turns the chip off.
A start refused after the end leaves the pane without an agent, so there is no chip to carry the failure: when the pane then has no agent row, the snapshot's `last_error` says once, as `pane_reopen.not_restarted`, that the agent was ended and could not be started again and that its conversation is kept; Herdr's words stay in the diagnostic, and the shell raises the failure as a notice that outlives `last_error` (`web/src/errorNotice.ts`, [UI_BEHAVIOR.md](UI_BEHAVIOR.md) on Reopen).
It is not started a second time, because the wait for the ended agent's name above is the one retry a start has, and a second identical start would learn nothing.
Herdr's words for a refusal go to the diagnostic log (`pane_reopen.answered`), never to the screen.

The Codex shared server is turned off by one more event, `codex_daemon_disable { device_id }` (`local` for this Mac), and only by it: no install pass ever does.
The operator confirms it first, because it also stops the daemon that is running, which disconnects every Codex attached to it (PRD codex-daemon-apply D-04, D-11).
It rides the machine's kit queue as `Scope::codex_daemon_off`, and the pass that carries it runs `codex features disable daemon_auto_start`, reads the setting back, stops the running daemon with `codex app-server daemon stop`, and answers in its report.
`KitSnapshot.codex_daemon_off` is `{"state":"pending"}` from the request until that report, then `{"state":"done"}` or `{"state":"failed","reason":<code>}` with `codex_missing`, `codex_refused`, `timed_out`, `unreachable` or `stop_failed`.
A failure before the setting went off leaves it as it was and stops nothing, while `stop_failed` says the setting is off but the daemon still answers, did not stop, or answered in a way Hide cannot read; the same request again then only stops it.
A plain read of the kit says nothing about the request and keeps the last answer, and a machine whose setting is off with no daemon answering, or whose Codex has no such setting, has nothing to turn off.
A Codex that Hide started runs with `--no-daemon`, is not attached to the daemon, and keeps running.

Regression owners: `runtime::tests::agent_connection` (the connection per reason, Reopen coalescing and refusals, the request's lifecycle), `pane_reopen::tests` (the order of end and start against `FakeHerdr`), `runtime::tests::device_kit` (a device's turn-off over the confirmed connection only, its numbered runs), `hide-kit`'s `the_operators_request_turns_the_shared_daemon_off_once_and_says_so` and the stop tests beside it, and `hide_agent_hooks::codex_daemon::tests` (the stop through the account's own `CODEX_HOME`).

## GitHub status in the Workspace row

The PR icon is independent of agent status and Workspace disclosure.
It appears for a known pull request, an in-progress first lookup, or a GitHub lookup failure; a successful lookup with no matching PR leaves it absent.
A pull request restored from the previous run draws as soon as its checkout row exists.
Clicking opens details without selecting a pane, marking agents read, or folding the Workspace.
The popover shows the PR number, title, Open/Draft/Merged/Closed state, head and base branches, and CI rollup.
Its refresh action reloads one repository; its external action opens the PR URL through the existing external browser route.

The core reads GitHub for every registered local Git project from the moment a window attaches to the daemon (the reader sits in the coordinator's UI-attached block, so a daemon with no window reads nothing), whether or not Overview, the palette or any other screen was opened; a device's project and a plain folder are not read.
A project's status is read once when a window first attaches, again five minutes after its last answer, or sooner when that read failed to get the project's pull requests or its issues (`Runtime::reread_stale_github`, with the clock passed in so a test advances it without sleeping; a project whose ask is unanswered is not asked again, so a slow pass is never overtaken by the next), and on an explicit refresh, which advances that repository's generation.
When the operator looks at a project, the web shell sends `github_request` with `look` (the window gains focus or becomes visible, and a project is brought to the front; `web/src/githubLook.ts`), and the core reads that project again when its last answer worked and is at least 30 seconds old (`Runtime::read_when_looked_at`, `GITHUB_LOOK_AFTER`, the clock passed in), so a project the operator comes back to does not wait out the five minutes.
A look is the stale re-read's ask and nothing more: it asks nothing while an ask is outstanding, for a project in failure backoff, one whose last read missed its pull requests, or one answered "no GitHub remote"; it never sets `loading`; it is honored only for the local Git project in front, and a look the core does not honor is silent (no error, no frame); a request that names `refresh` is a refresh whether or not it names `look`.
While a window is attached, an advisory poll asks GitHub once a minute whether anything behind the marks changed, and only says so (`crate::github_poll`, `Runtime::ask_polled_changes`).
It watches the projects whose last read worked and holds an open pull request, the project in front first and at most eight (a count past that is written as `poll.over_limit` when it changes): the repository's open pull request list, and the check runs and commit status of the head commit of up to four open pull requests whose checks were pending, each by a conditional `gh api` read with the entity tag it last saw; a 304 means unchanged and costs no rate limit, and a tag seen for the first time is a baseline, not a change.
The check runs and the status of a pending head commit are read the same way: a different tag only makes the poll read what the list adds up to (`passing`, `pending`, `failed`, by the rule the mark uses), and it counts a change only when that combined result, over the check runs and the status together, differs from the last one. One job of many finishing asks nothing; the last one finishing, or a failure, asks one read within a minute.
The open list moves its tag for a comment, a label, a review or a new `updated_at` as well, so a different tag there is not enough: the poll then reads one digest of the list (each pull request's number, head commit and draft flag, `gh api --jq`) and counts it a change only when that digest differs from the last one, which a pull request opened, merged or closed, a push, or a draft flip does.
A comment, a label, a review or an `updated_at` alone is left to the five-minute read and to a look.
A change is asked for through the look rule above, so it waits out the thirty seconds, never overtakes an ask, a failure backoff or a repository with no GitHub remote, and never sets `loading`; the poll writes no pull request state and never touches the five-minute clock or its backoff.
A failure of the poll (no `gh`, no login, the network, a reply it cannot read) pauses that project alone, 60 seconds and twice as long each time up to 15 minutes, and is written once per kind as `poll.failed`; it shows nothing on screen (design principle 13).
With fewer than a tenth of the account's requests left, nothing is polled until `x-ratelimit-reset` (`poll.rate_floor`).
A change in the first minute after a project's baseline is read only by the five-minute clock, because the poll has no earlier tag to compare it with.
A local agent's session that prints the address of a pull request the last answer does not hold asks for its project as well, so a pull request an agent opens reaches its row when the label worker next reads that session (its turn ending, at the latest) rather than up to five minutes later (`Runtime::read_sighted_pull_requests`).
The project is the one whose pull requests are in that repository, else the printing pane's project when none of its pull requests names another repository, so a repository's first pull request is found and a fork's checkout whose agent opens a pull request upstream is not read for it; an address no read project owns is written to the diagnostic log as `read.sighted_unowned` and nothing more.
The sighting keeps the rule above: a project whose last read worked and has nothing asked since is asked at once and its clock waits for that answer; a project with any ask outstanding (a read in flight, its first read, a refresh, or an answer the clock has not taken yet) is not asked again, and is read once more after that answer lands only if the answer does not hold the pull request and the sighting is still under fifteen minutes old; a project whose last read failed to get its pull requests is left to its own retry, and one that got them and missed only its issues is asked as one that worked.
The read is quiet: unlike a refresh from the menu or the popover, it does not set `loading`, so the row keeps what it shows until the answer moves it.
Each address sets off this once while its sighting is under fifteen minutes old, so an address the read still does not return (another repository, or past the newest 200) is not asked for again; a sighting older than that sets off nothing, and an address an answer holds stops being remembered.
At most 64 addresses that a project owns are remembered at once, and one past that is not read and is written as `read.sighted_over_limit`; the label worker keeps at most 32 sightings between two hand-offs, the newest, and a read that finds more writes `read.sighted_capped` with how many it dropped; each owned address it remembers writes `read.sighted` with the projects read now and those waiting for an answer, both empty when every one of them is in failure backoff.
A repository with no GitHub remote is an answer, not a failure: when the remotes its node read with the catalog's path facts name no repository `gh` could read, the reader answers "no GitHub remote" (`failure_category` `no_github_remote`, neither stale nor an `unavailable_reason`) with no `gh` call and the screen draws no GitHub state for it; a `gh` that says the same for a repository whose remotes the node could not read (an older node, a config that includes another file, a remote URL in a form the node does not read such as `http://`) or that are not GitHub's gives the same answer. A device project whose `origin` its node reads as naming no GitHub repository gets the same answer (`available`, no `unavailable_reason`), so its checkout cards draw nothing for it either.
When the node read its remotes, its clock (`GithubClock::NoRemote`) never asks again on its own: the next read comes when the catalog's remotes for that repository change, written as `read.remotes_changed`, or on an explicit refresh; before, each read ran `gh pr list` twice to learn that and was retried every five minutes forever.
When the node could not, `gh`'s answer is asked again five minutes later, since a remote can be added where the node does not look.
A failed read (one that did not get the project's pull requests or its issues, whatever the reason: `GithubProjectSnapshot::read_outcome`) is asked again after 30 seconds, then 60, 120 and 240, and from the fifth failure in a row every five minutes; the first read that works starts the count over, and another project's clock is untouched: the reader hands back its cached entry for every project in a request, so an answer counts for a project only when that project's generation moved since its last answer.
The previous answer stays drawn while a retry waits, and nothing new is shown for it: the row keeps its muted stale state (design principle 13), and each retry writes one `github` `read.retry` diagnostic with the project, the attempt and the delay.
The failure count lives in the clock (`GithubClock`), not in a second delay, and a project is asked again only after its previous ask answered, so at most 64 projects each cost one read per 30 seconds in the worst case and an outage backs every one of them off to the five-minute pace within four retries.
At most 64 local Git projects are read: the project in front first, then the ones a screen named with `github_request` or `overview_refresh`, then the rest in path order; a count past that is stated in the diagnostic log as `projects.over_limit` each time it changes and the ones left out show no PR icon.
Every project shares `GithubReader`, its per-project cache, one active worker and coalesced pending requests.
Each `gh` command runs on the core's own node (`hide-host/src/gh.rs`), where the operator's GitHub login is, under its 15-second limit; the core sends the command line as a `gh` call, the node refuses any line `hide_node_link::gh::allowed` does not name without running it, and the core parses what it answers.
The last successful answer of every project is kept in `github-snapshot.json` in the state folder (`hide_kit::layout::github_snapshot`, written by `github_store.rs`) and restored when the daemon starts, so the first frame already draws it; the file also keeps what the wire leaves out of a pull request (its head commit, its repository and its closed time), because a settled pull request reconnects to its worktree only at that commit.
A restored answer is stale until a read replaces it: `stale` is set and `last_success_at_unix_ms` stays the time of the read that produced it, so the icon is muted and the popover says how old it is.
A file that cannot be read, decoded or is of another schema version is not used and is named in the diagnostic log as `snapshot.discarded`; the next answer replaces it.
`loading` is true only for a project with no answer to show whose first read has not yet answered; a project the reader answers nothing for is not loading either, and a project with a restored or earlier answer is loading only while an explicit refresh of it is in flight.
A project's pull requests are two [`gh pr list`](https://cli.github.com/manual/gh_pr_list) calls asked for at the same time, each under the 15-second limit: the 200 newest pull requests of any state without `statusCheckRollup` (with `headRefOid`, `isCrossRepository`, `createdAt`, `closedAt`, `author`, `mergeStateStatus` and `reviewRequests`), and the open ones with `number,statusCheckRollup` only.
The same open-check rollup also supplies optional `PullRequestWork.check_counts`: total, passed, failed, running, waiting, skipped and unknown, each check counted once.
Completed success is passed; neutral and skipped remain skipped; failure conclusions are failed; active CheckRuns are running, queued CheckRuns waiting, and an unread or unfamiliar state unknown.
Legacy StatusContexts keep SUCCESS, FAILURE/ERROR, PENDING and EXPECTED distinct; an absent rollup stays unknown while an explicitly empty rollup has zero checks.
This adds no GitHub read or polling clock and leaves the existing merge eligibility calculation unchanged.
The PRs tool (below) adds three reads to the same generation: the signed-in login (`gh api user --jq .login`, once per read of all projects, right after `auth status`), the open pull requests the account was asked to review (`gh pr list --search review-requested:@me`, numbers only), and one GraphQL query for each open pull request's place in the merge queue and the time of its last review request (`HidePrFacts`).
The pull request list alone decides whether a project's read succeeds (issue #1189, which replaces PRD work-tab-pr-lens D-25's rule that any of the three failing failed the read).
`gh auth status` failing is still the machine's refusal and fails every project's read.
Each of the three is asked and judged on its own, and one that fails leaves the judgments derived from it unknown on every pull request, never "no": the login makes `mine` (`Option<bool>`, `None` when unknown), the review search with the login and the request list makes `asked_of_me` (`ReviewAsk::Unknown`), and the GraphQL query makes the queue place (`MergeQueuePlace::Unknown`) and the time of the last review request.
What the list alone settles stays settled: a request naming the login is a direct ask without the search, and a request list with nobody who could be the operator or their team (any request when the login is unknown, a team's request when it is known) was asked of neither.
The read is a success with the failed questions named in the project's status (`unread_queries`: `account`, `review_requests`, `merge_queue`), so the status is not stale and the list is current; each failure writes one `github` `pull_requests.extra_unread` diagnostic with the question and `gh`'s words, which the operator never sees.
A project that keeps its last list because a later read failed keeps the names of the unanswered questions that list was read with.
The re-read clock treats the read as answered: the list is current, and a question that fails every time (a GitHub Enterprise Server without the queue fields, a token too narrow for the search) costs two calls each normal interval rather than slowing the list.
Asking for the checks of every merged and closed pull request made one read take 11 - 14 seconds, so a settled pull request's checks are not asked for again: the checks read for the same number and head commit while it was open stay in the reader's cache, and without them its checks are `Unknown`, drawn as no mark.
Either call failing fails the project's read, which keeps the last answer and states the failure, so half an answer is never a repository with no pull requests; `pull_requests.ok`, `pull_requests.empty` and `pull_requests.failed` carry the read's `duration_ms`.
Which pull request belongs to a checkout is one rule (`github::belongs_to_checkout`, chosen by `pull_request_for_checkout`), because a branch name is used again for new work: a pull request from a fork never belongs to a local branch of the same name, an open one belongs to the checkout on its head branch, and a merged or closed one only while the checkout's commit (the worktree reader's `head_sha`, or, before that reader has answered, the commit `Repository::head_oid` read from the repository's own files with no process) is exactly the pull request's head commit on that branch; a checkout that was amended, rebased or left behind after the last push therefore loses its merged pull request.
Several candidates resolve open, then merged, then closed, the most recently updated first among equals, and a checkout whose commit is not read yet takes no settled pull request.
The sidebar row, the worktree catalog, the agent rows' PR mark and the link and hand-off actions all ask that rule, and a catalog read that moves a HEAD decides the connection again.
The PRs tool and a worktree's base still list one pull request per head branch (`preferred_per_branch`).
The same generation also reads open issues, their Project Status, and PR closing references for Project Home.
The issue list uses `sort:updated-desc` and reads one sentinel beyond the 200-issue display cap so overflow is based on evidence.
Issue references accept a GitHub issue URL, `owner/repo#N`, or `#N` when the repository is known; unsupported hosts and malformed references are rejected.
A failed component read retains that component's last successful answer, including a successfully empty answer, while a successful PR read still advances if issue reading fails.
Hide stores no new credentials and runs no subprocess or file write under the runtime mutex; the five-minute ask is a comparison on the coordinator's existing wake, and the file is written by its own thread from an owned copy.
The project request event, result status and PR fields travel through revisioned `rest`; presentation reads that snapshot only.

| GitHub checks | UI |
| --- | --- |
| Every reported check succeeded, was neutral, or was skipped | Passing, green |
| Any failure, error, cancellation, timeout, or required action | Failing, red |
| At least one running, queued, waiting, pending, or requested check, with no failure | Running, yellow |
| An explicitly empty check list | No checks, gray |
| Absent check data, an unknown check kind, an unrecognized terminal result, or a merged or closed pull request whose checks were never read while it was open | Unknown, no mark |

Failure takes precedence over pending, then unknown, then passing.
A failed refresh preserves the last known PR and reports the lookup failure and last successful read time.
The visible stale notice qualifies both PR state and CI; old green checks are not presented as a fresh result.
Missing `gh`, logged-out authentication, network errors and rate limits use the reader's existing categorized diagnostics and user-facing reason.

The sidebar tree projection additionally indexes physical pane ownership when forming root rows and direct-select candidates.
This is linear in the visited project panes and agent projection per recomputation, with no queue or asynchronous work; repeated hover values do not trigger it.

### Pull request visual states

A pull request has one state, decided in the core by one rule (`model::PrState::of`, from its lifecycle, draft flag, checks and review decision) and sent on the wire as `state` on every checkout pull request, as `pr_state` on a GitHub search result and as `state` on a link panel's pull request (`contracts/snapshot-wire-enums.json`: `pr_state`).
The rule reads in this order: merged, closed, draft, failed (failed checks or a change request), mergeable (passing checks with an approved or absent review decision), and pending for the rest (checks running, none or unknown, or a review still required).
A draft is draft even with failing checks, and absent or unknown checks never imply a pass.
Each state has one token and one icon, drawn the same on every surface (operator approval, 2026-10-10): failed red (`--pr-failed`), pending amber (`--pr-pending`), mergeable green (`--pr-mergeable`), draft grey (`--pr-draft`), merged purple with the merge icon (`--pr-merged`), and closed dim (`--pr-closed`).
A change request is red wherever its word is drawn, as failed checks are.
The web turns a state into its icon, colour and word in one module (`web/src/prMark.ts`) and draws it with one component (`web/src/components/pr-mark.tsx`); no screen keeps a colour table or a classifier of its own, and the checks and review words and the stale reading live in the same module.
The state word and the accessibility label keep the meaning without relying on colour.
A mark dims while GitHub cannot be read again, and its tooltip says when it was last read.

### Pull request groups

The PRs tool (`docs/UI_BEHAVIOR.md`, The PRs tool and the PR card) lists pull requests in groups the core decides, in `herdr-core/src/agent_state/work/board.rs`, from GitHub values the five-minute read carries and from facts only Hide holds: the link record's sessions and the agents' own state.
A pull request body's headings and a repository's own scripts decide nothing, so the same rules hold in every repository, and Hide AI never changes a group or a reason.
The result is the `prs` board of each project's `agent_scope`: rows (group, fold, reason, agents, the pane a reason is about, the checkouts it is the work of, its issue), the groups with their numbers in draw order, and the count of `needs_you` rows that are not folded.

Which pull requests are listed: the operator's own, those a session of theirs made or worked on, those a review was asked of them or of their team for, and bots'; a merged one needs the first two, a closed one that was not merged is never listed, and a teammate's pull request none of these reaches is not.
"Mine" is the pull request's `author.login` equal to the signed-in login.
A pull request is left out only when every judgment says it is not the operator's: while the login is unknown, every open or merged pull request may be theirs and is listed, and while a review ask is unknown, the open pull request it concerns is listed (issue #1189).
A pull request's agents are the live agents whose session the link record names as having made or worked on its branch; sharing a folder alone ties nothing.
An agent that finished and is not yet looked at moves no group, because the unread check is the 태스크 list's (읽을 것).

Each pull request stops at the first slot that fits:

| # | Fits when | Group | Reason |
| --- | --- | --- | --- |
| 1 | the author is a bot (GitHub's `is_bot`) | Bots | the reason its other rules give |
| 2 | its agent is asking (a question, an approval or an error) | Needs You | `reply` with the agent and the question's first line |
| 3 | its agent is working but has not moved for the watch's first inactivity window (`INACTIVITY_MS`, 20 minutes) | Needs You | `check` with the agent and the time since it moved |
| 4 | its agent is working | Agent is working | `working` |
| 5 | it is in the merge queue | Merging · merged, first | `queued` with the place |
| 6 | it is a draft | Drafts | `draft` with its age |
| 7 | it is owned (mine, a session of mine touched it, a bot's) and checks failed that GitHub's `mergeStateStatus` does not call `UNSTABLE`, or a change request stands, or the state is `DIRTY` | Blocked · no agent | `failed` with the first check's name and how many more, `changes`, or `conflict` with the base |
| 8 | it is mine and the state is `CLEAN` or `HAS_HOOKS`; `UNSTABLE` too, with the failed required-or-not check named | Needs You | `merge` |
| 9 | it is mine and the state is `BEHIND` | Needs You | `update` with the base |
| 10 | it is mine and the state is `BLOCKED` | Waiting with `ci` while checks run, Waiting with `waiting_review` while a review is requested, else Needs You | `assign_reviewer` |
| 11 | a review was asked of the operator by name | Needs You | `review` with the author and the time since the ask |
| 12 | a review was asked of a team of the operator's (the search finds it, the request names no user) | Needs You, folded as `팀 리뷰 요청 N` | `team_review` |
| 13 | anything else open | Waiting | `ci` while checks run, else `waiting_review` |

A judgment that is unknown is never read as "no", and a pull request that cannot be judged is never put in a group it may not belong to.
The slots that read an unknown judgment instead place the pull request in Waiting with the reason `unjudged` and the question that was not answered (`account`, `review_requests`, `merge_queue`), in the plain colour of `checking`, and it counts toward no Needs You:

| Unknown | The pull request that waits unjudged | What still holds |
| --- | --- | --- |
| the login | any open one that no earlier slot placed and that has no session of the operator's and is not a bot's; a session's pull request still stops at slot 7 | slots 1 - 6 (a bot, an agent, the queue place, a draft) |
| the review ask | a teammate's open one the request list cannot settle (a team's request is on it) | a request naming the login (slot 11), and the pull requests of slots 7 - 10 |
| the queue place | the operator's own at slot 8, 9 or 10 (merge state clean, has hooks, unstable, behind or blocked), the states a queued pull request can be in | a conflicted, draft or still-computing one; a queued one when the queue is read |

Slot 5 fires only on a known place.
A merged pull request is Merging · merged below the queued ones, newest merge first.
When GitHub has not computed `mergeStateStatus` (`UNKNOWN`) the pull request keeps the group it stood in on the previous board with the reason `checking`, and waits when it is new, so a pull request never jumps while GitHub thinks.
A pull request nothing has changed on for 7 days (`OLD_MS`) in any group but Merging · merged is folded as `오래된 PR N` inside its group.
Folded lines (a team's review asks, old pull requests) are in no count: the PRs icon's number is these unfolded Needs You rows alone (PRD sidebar-task-lens B29), and the sidebar's Needs You is unchanged.
Within a group the order is the reason's urgency, then the newest update (the newest merge for Merging · merged), then the larger number.
Times in a reason are the instants they began, never durations, so time passing publishes nothing; the web counts them on its one clock.

`<checkout> only` keeps a row whose `checkout_ids` hold the checkout: the one on the pull request's branch, the ones its recorded sessions ran in and the ones its live agents sit in.
A project with no GitHub remote has no board, and `gh` missing or signed out is a state of the project's GitHub status, not an empty board.

### A task's stage

A task's four-step stage (`작업 → PR → 검토 → 머지`), which feeds the task's state and which no screen draws since the session record's `지금` box replaced its bar (PRD pr-card-and-session-record-redesign B14), reuses this cached PR answer through the agents' PR summaries (`agent_state/tally/tasks.rs::stage`): with any pull request the bar stands at the least advanced one that is not merged, a draft, pending, failing or closed one at two steps and a mergeable one at three, and at four once every one merged; with none it is one step while a member works in a linked worktree, and a task that is only a conversation has no bar.
A failed or unrequested lookup never becomes a merged or empty stage: a member whose PR state is unread adds nothing.
The workspace inspector uses the canonical representative agent and disconnected override; inspection itself never acknowledges an agent.

## Task identity

Conversation display history belongs to `(device, provider, native session ID)` in the existing links ledger, independently of the pane that currently runs it.
The label worker restores only title, goal, summary and PR references into a display binding; its current turn facts, wait causes, lifecycle and input target still require the existing current-reference proof.
A missing Herdr reference may use the exact foreground executable's explicit resume argument as a history locator, after the native reader proves that record's owner.
That argument is launch intent, not proof of the conversation the process currently has open: such a binding remains `unconfirmed`, spends no new analysis and supplies no current facts or control authority.
A new Herdr hook reference supersedes the locator and clears the previous display immediately; only a native read for that reference and generation confirms the binding.
No latest-session, cwd or reverse owner-hash guess is made.

The core publishes `record_connection` as `checking`, `confirmed` or `unconfirmed` when a row has display history.
Checking and unconfirmed history keeps its last title and valid PRs dimmed with the localized connection words; confirmed history removes that treatment.
A provisional PR participates in the display summary without gaining current work duty or a session-created relation; the normal merged/closed display filter still applies.
It cannot change a task's stage, and a tab's title provenance follows the core's focused pane even when several panes share a title.
Failure keeps display history and retries through the existing single reader, at most once per pane per fifteen seconds after an unsuccessful confirmation.
The row's branch may differ from a saved PR's branch; existing closed/merged filters remain, and a provisional relation never grants current PR duty or a confirmed-created mark.
This connection state is separate from hook installation diagnosis and GitHub read staleness.
A different provider or reference clears the old display, and generation fencing rejects old reads and analyses even on an A-to-B-to-A return.

The existing labels file is compatibility input, never a second session-history database.
The existing ledger writer acknowledges display saves after commit; an unindexed session or failed query remains pending and retries through the same bounded reader, and queue admission alone never settles a save.
Native title changes preserve absent fields and apply explicit clears, and generator standby suppresses display bindings along with labels.
Only a record with an explicit native ID and matching provider owner is eligible for import, repeated imports converge, and corrupt or newer labels files are preserved instead of overwritten by an empty save.
The links writer keeps the existing device separation and session retention; display metadata adds no transcript, path or complete facts copy.
Agent summaries Off hides saved goals and summaries while retaining native/manual titles and PR restoration.
Regression owners are `labels/record_tests.rs`, `labels/store.rs`, `links/ledger_store_tests.rs`, the wire locator tests, and the shared row/PR tests.


Project and Task session rows project a separate `last_prompt_at_unix_ms` from the latest dated actual request in the proven native session, whether sent by the operator or another agent.
Missing or zero timestamps draw no elapsed time; status changes and daemon restarts never manufacture one.
The existing `changed_at_unix_ms`, ordering, unread, waiting and escalation clocks keep their state semantics.
The label worker's restart, rewritten-transcript and helper regressions in `labels/tests.rs`, and `web/e2e/prompt-elapsed.spec.ts`, cover this boundary.

The core publishes one `identity_label` per agent, and every surface calls the agent by it: the sidebar row, the pane header, the ⌘K search row, the ⌃Tab Recent Panels row, the lineage chips, the Overview agent line and the request view.
The provider fallback reads the shared adapter's sidebar label or canonical Herdr kind after common alias normalization, preserving a raw unknown kind and `Agent` for no kind.
`sidebar.rs` chooses the first nonempty, sanitized value in one order for every agent:

1. a proven manual title,
2. Hide AI's generated task name (`goal`),
3. the proven native automatic title, including a name whose source cannot be distinguished,
4. the provider.

With Agent summaries off there is no goal, so the order is manual title, native automatic title, provider.
A title is manual only when its native record distinguishes a manual rename from automatic naming; a rename command alone is not that proof.
Claude's explicit empty `ai-title` or `custom-title` clears its stored slot; an absent record preserves it during an incremental read.
Readers preserve the two title slots through the label overlay instead of combining them before projection.
No reader capability is granted by the order, and an agent without a native title reader has neither native step.
The same chosen title reaches the default Agent tab label, phone list and desktop/phone notification title as well as the surfaces above.
An explicitly renamed Herdr tab keeps its separate tab-label authority ([UI_BEHAVIOR.md](UI_BEHAVIOR.md#agent-areas)).

| Agent and format read | Manual slot | Automatic or source-less slot | Native source inspected |
| --- | --- | --- | --- |
| Claude Code 2.1.288 | `custom-title.customTitle` from `/rename` | `ai-title.aiTitle`; summary/compaction records grant no manual name | [Automatic record, line 2](../hide-session/tests/fixtures/adapters/claude-2.1.288/projects/-work-app/a1b2c3d4-0000-4000-8000-000000000001.jsonl#L2) and [manual record, line 10](../hide-session/tests/fixtures/adapters/claude-2.1.288/projects/-work-app/a1b2c3d4-0000-4000-8000-000000000001.jsonl#L10); `hide-session/src/lib.rs::parse_claude_line` reads the two distinct records. |
| Codex CLI 0.160.0 | None provable from the index | Newest `session_index.jsonl.thread_name`, including a renamed name | [SessionIndexEntry and append_thread_name, lines 24-50](https://github.com/openai/codex/blob/rust-v0.160.0/codex-rs/rollout/src/session_index.rs#L24-L50) record only id, name and update time, with no manual/automatic source. |
| Grok 1.0.46 | `summary.json.generated_title` when `title_is_manual` is true | The same title when false or absent | [Native-format summary, line 16](../hide-session/tests/fixtures/adapters/grok-1.0.46/summary.json#L16); `hide-session/src/grok.rs::summary` reads the flag separately from the title. |
| OpenCode 1.18.30 | None provable from the session row | Root session `title`, including a renamed title | [SessionTable, lines 22-60](https://github.com/anomalyco/opencode/blob/v1.18.30/packages/core/src/session/sql.ts#L22-L60) has one title column and no title source. |
| OpenCode 2.0.29 | None provable from the session row | Root session `title`, including a renamed title; a session without one has none | [SessionTable, lines 22-77](https://github.com/anomalyco/opencode/blob/v2.0.29/packages/core/src/session/sql.ts#L22-L77) (`session_v2`) has one nullable title column and no title source. |
| Pi 1.0.4 | None provable from a name record | `session_info.name`, including `/name` and extension-written names | [SessionInfoEntry, lines 141-145](https://github.com/earendil-works/pi/blob/v1.0.4/packages/coding-agent/src/core/session-manager.ts#L141-L145) and [appendSessionInfo, lines 1303-1315](https://github.com/earendil-works/pi/blob/v1.0.4/packages/coding-agent/src/core/session-manager.ts#L1303-L1315) record a name without its source; the CLI and extensions share `AgentSession.setSessionName`. |
| omp 18.7.0 | First title-v1 slot `source: user`, or legacy first header `titleSource: user` | `source: auto`, or a missing source | [Title slot source parsing, lines 61-79](https://github.com/can1357/oh-my-pi/blob/e0fc1cf4ea354b445a359b37fa5eb58deaa85598/packages/coding-agent/src/session/session-title-slot.ts#L61-L79); `hide-session/src/native_file.rs::title_snapshot` reads the slot or legacy header. |
| Antigravity (agy) 1.4.0 | None provable from the index row | `conversation_summaries.title` of the conversation's row in `~/.gemini/antigravity-cli/conversation_summaries.db`, an empty title read as none | `hide-session/src/antigravity.rs::title` reads the one title column, which records no manual or automatic source. |
| Cursor | No native title reader | No native title reader | `hide-session/src/cursor.rs` and [its native fixture contract](../hide-session/tests/it/cursor.rs) explicitly exclude the sidecar title and database name. |

Codex, OpenCode, Pi and agy therefore show Hide AI's goal ahead of even a renamed native name while summaries are on, because their read format cannot prove a manual source.
A pane Fork agent made in agy shows the agent name until the forked conversation has a title: agy writes the fork's index row with an empty title and no transcript, as a Claude Code fork has neither before its first turn.
Pi names previously stored as manual, or returned by an older helper that did so, are projected as automatic under the same current session proof.
Cursor uses generated goal, then provider.
Its ordinary SQLite chat graph supplies recorded human and assistant text under the same native ID and checkout proof, while its sidecar title and database name remain outside that authority.
Injected native user records, tool output, shell turns and child graphs supply no root request, structured wait or PR sighting; absent event times stay unknown and recorded times have minute precision.
Pi 1.0.4's latest explicit empty `session_info.name` clears the old native name before the goal/provider fallback is applied.
Its recorded user/assistant messages feed the existing label worker, while tool output supplies PR sightings and injected extension/compaction context supplies no human request.
Pi records no structured question/plan wait or child identity, so no `UserTurnFact`, structured content or subagent relationship is inferred from text or `parentSession`.
omp 18.7.0 takes its current automatic/manual title from the physical first title slot or legacy first session header, with explicit empty values clearing stale names and `title_change` remaining audit history.
Same-length title updates at EOF change no event offset and replay no conversation or native question.
The existing label reader refreshes native-file titles and questions at a minimum three-second interval without requiring a Herdr lifecycle transition; failed native proofs revoke the row's authority and retry after fifteen seconds.
Unanswered native `ask` calls supply exact bounded text and choices, and only their correlated result, a later human turn or an abort clears them.
A failed current omp reread revokes native identity and question authority until a fresh proof, and nested child artifacts supply no root conversation or inferred child relationship.
Grok 1.0.46 takes its title from `summary.json`: `generated_title`, marked manual by `title_is_manual` after `/rename`, with a missing or empty title clearing the stale name; the file is read again on every poll and moves no event offset.
Its unanswered `ask_user_question` calls supply exact bounded text and option labels, and its terminal tool update, a later human turn or a cancelled turn clears them.
A Grok subagent, headless or hidden session is no pane's root, so it supplies no title, question or conversation.
Before an omp resume or fork, Hide refuses option-like IDs, non-ASCII filenames with uncertain native case matching, and pending native directory migration or orphaned backup recovery.
These action-only checks never move or repair a history; read-only labels and conversations retain their own proof, and the native CLI must resolve the pending maintenance before a fresh route can be admitted.
OpenCode 1.18.30's and 2.0.29's session `title` is a proven native title of its root session only; a subagent's child session never names the root row.
OpenCode's `question` tool part (in 2.0.29, the content item of the asking assistant message named `question`) supplies the question's text and choices while it is pending or running, and its completed or errored state clears it; the asking message stays unfinished while OpenCode waits, and the read folds it without settling it, so the answer clears the wait on a later read.
The agent's own title rides the same proof as the label: it is laid on the row only while the pane's reference proves the session it was read from.
The Herdr workspace label is never a name: it is whatever the workspace was called when it was opened, and one workspace can hold agents for several checkouts.
The Herdr agent name remains the unique control identifier that Sasu and other orchestrators assign at start, so it never enters the display ladder.
A registration's name (`AgentRecord::name`) is optional and is never a pane id.
Where a program compares names, an unnamed registration is called by its registration id (`coordination::wire_name`), never by its title: the label worker accepts a parent's request as its child's work only when the envelope header's sender equals the `parent_name` of the lineage facts, and both come from that one function, so a title that changes every turn cannot separate them ([delivery.md](delivery.md#names-on-the-wire-and-on-screen)).
Where a person reads a name (the bell line, a row) an unnamed agent is called by its title, and by its runtime's name when it has none.
Nothing publishes a session `name`, reads Codex's first human turn as a title, or renames an agent or tab.

The label is made by the core, not by a plugin and not through pane tokens: `herdr-core/src/labels/` reads each Claude, Codex, Grok, Pi, omp, Cursor or OpenCode pane's conversation, asks the background AI for the session's goal, one line for the turn and how the turn ended (`context_label.v18`), in the core's output language ([AI_PROVIDERS.md](AI_PROVIDERS.md#the-output-language)), and keeps the answer per pane (the architecture is in [ARCHITECTURE.md](ARCHITECTURE.md#agent-labels-in-the-core)).
A session with no goal and no assigned work answers `goal: null`; greetings, identity questions and capability questions create no task and no placeholder title.
The row uses the existing title priority above, and the next analysis after real work is requested sets its first goal.
A session started with a goal command is named from that goal like any other request.
Claude Code's `/goal <text>` is the operator's request with `<text>` as its words, because Claude Code marks its own answer to the command as the operator's turn (`turnOrigin: "human"`); a command it answers without that mark, such as `/model` or `/effort`, is no request, and the mark, not the command's name, decides.
Codex's `/goal` is the `<codex_internal_context source="user_goal">` message, read as the objective it quotes, or as the message itself when Codex wrote no objective to read; every other `<codex_internal_context>` source, including the `goal` reminder Codex adds to each turn while a goal is active, is Codex's own text and no person's turn.
A session read before this rule keeps what was read and is named at its next analyzed operator turn.
A goal names the concrete feature, screen or subject and its intended outcome, so neighboring sessions can be told apart; a delegated session names its own assigned part.
A goal already set cannot become null.
`goal_update: keep` preserves it verbatim, `refine` gives a vague or misleading name a more specific subject without changing the assignment, and `replace` names a new or larger assignment.
Refinement is accepted only on an operator turn whose result still belongs to the current turn, including its end; replacement retains the existing first-analysis, turn-start and skipped-start rules and requires an operator turn.
Another agent's follow-up cannot rename it, and an already specific goal stays stable through implementation, tests, merging and installation.
A local record keeps at most 800 masked characters of the requests and early replies that establish and clarify its assignment, so a terse follow-up after a restart does not lose the subject; replacing the assignment replaces that evidence, and device marks save none of this text.
It refreshes only during its original turn; later follow-ups keep that clarification even if bounded history drops the early replies.
A named legacy record without this evidence has an unknown assignment origin; it keeps rolling from its current goal and latest exchange rather than treating the session's first request as that assignment.
Evidence is established only at an accepted first or new assignment with one new attributed request; several historical requests do not prove its origin.
The line names a concrete current action, finding, result or dependency; an acknowledgment such as “proceeding as requested” adds no information and is left empty.
It prioritizes progress the operator can understand over internal implementation terms, test counts and capture counts; a technical name remains when the operator needs it to identify or act on an issue.
Rewording preserves the actual scope and pending verification: a test-fixture edit does not become a finished feature, and requested verification does not become a passed check.
With `keep`, the returned goal value is ignored even when it is null; the turn's `line` and `end` are still applied.
A line longer than 40 characters (80 in English) is cut between words and ends in `…`; a question or a block keeps its word with the cut line.
`line` and `end` still describe the current conversation: an answered greeting or identity question with no required next action has an empty line and a stopped end of `done`, while a specific question retains its requested action.
The call at a turn's start is asked only once the turn has worked 15 seconds (`labels/worker.rs`, `START_DELAY`): a measured 23% of turns end within 15 s and 38% within 30 s (2,384 Claude Code turns, 2026-10-16), and a turn that ends first is asked its end alone, which names the goal as well: a short turn that gives the pane a new job renames it, a short follow-up on the same job keeps the name, and the turn costs one request instead of two.
The old turn's sentence clears when the worker proves the next human turn, including when Herdr stays Working; a pane that has no goal yet is asked at once, so a new session is named as soon as the model answers.
How the turn ended is asked only once the agent has stopped: the call at a turn's start names the goal and the line and leaves the end `working`, and the call at its end picks the word by whose move is next, `question` (the operator's), `blocked` (stopped on a cause the line can state in one sentence: no disk, a failed CI run, a permission or login it cannot get, a tool that will not run), `waiting` (something other than a pull request still running: a background test, build, CI run or another agent), `unfinished` (nobody's, with the work left undone) or `done` (nobody's, with the work reported finished), taking the first in that order that fits.
There is no "unsure means done": a turn that did not report its work finished is `unfinished`, and a `blocked` answer with no line is read as `unfinished`.
A turn that started or handed off work that still runs is `waiting`, even when the turn itself only reported progress or answered something, unless it asks the operator; its line says what it waits for.
A request to look at the output of work that still runs (a render, a build, a list of corrections) is not a question: the agent goes on without the answer and reports when the work ends, so the turn is `waiting`; it stays a `question` when the running work cannot go on without the answer (a choice, an approval, a missing piece of information).
A turn is the person's message that began it, known by its time and its words, so a request that repeats the last one word for word is a new turn and is asked again.
`LabelOverlay::apply` lays that label onto an agent just before the runtime projects it, as `task` (the goal), `expected_reply` (the line when the turn ended on a question), `progress` (the line otherwise, which for a block is its cause), `question` (a question end on an agent that is not running) and `blocked` (a block end on an agent that is not running), and `sidebar.rs::project_agent` reads those five.
With Settings › Hide AI › Features › Agent summaries off nothing of the generated label is laid: the row uses manual title, proven native automatic title, then provider, and has no generated sentence or inferred plain-text question (D-11).
Native questions and plan waits are still read, and a pending native question still supplies its own first line.
A label is shown only for the session it was proven for.
The record keeps the provider's native session owner, proven from transcript metadata, and the Herdr reference it was proven under; the label is applied only while the pane's current provider and session reference equal that owner or that reference (`PaneRecord::proven_for`).
A new session, a reused pane, a provider change and an A to B to A switch therefore show the provider's name and no question until the new session is proven, and returning to A restores A's label.
A session whose file its agent has not written yet shows the provider's name and is read again on a bounded backoff, so its label arrives with its first message rather than with its first finished turn ([ARCHITECTURE.md](ARCHITECTURE.md#agent-labels-in-the-core)).
While a pane has no concrete reference, nothing is shown and the proof is kept for the reference's return.
A read or analysis that lands after the pane's reference moved carries an older generation and is dropped, so a late answer for the old session never reaches a row.
A restart restores a label with no read and no request when the pane's reference still proves it, and an entry the retired plugin left (`display-state.json`) is imported once on this Mac only when it had proven an owner, and is shown by the same rule.
When the pane moves after a restart, its next read starts at its last human message, except for a pane with no goal yet while summaries are on: that read starts at the conversation's beginning, because a pane with no goal is named from the operator's requests from the first one on, and the first request lies before that message.
Without it, a pane whose first turn named no goal (a greeting) and whose later turns came from another agent had nothing the operator asked in view and was never labelled again.
A turn whose context still holds no operator request is settled unanalyzed and logged once as `analysis.skipped_no_context`, never skipped in silence.
Sleeping rows preserve only labels captured from the proven current projection; older sleep records without provenance fall back to the provider.
This adds no worker, subprocess or I/O under the runtime lock: the worker's reads and analyses run off it and the projection only copies the bounded label strings, with no new notification beyond an actual projected state transition.
Regression owners: in `herdr-core/src/labels/tests.rs`, `another_session_shows_nothing_of_the_last_one_until_it_is_proven`, `an_analysis_that_lands_after_the_session_changed_is_dropped`, `a_restart_restores_the_label_with_no_read_and_no_request`, `a_pane_with_no_goal_is_still_analyzed_after_a_restart`, `an_imported_label_shows_only_for_the_session_it_was_proven_for` and `a_turn_that_ran_between_two_looks_ends_the_question`; the device path is `a_device_panes_label_is_read_through_its_helper` and its disconnect and old-helper siblings; `desktop/e2e/session-labels.spec.ts` proves the session boundary against the running app.
There is no `summary` token and no missing-summary notice: an agent without a task follows the native-title/provider priority above, and an empty line draws no placeholder sentence.
Truncation belongs to each view and does not shorten tooltip or accessibility text.
Projection adds bounded-by-metadata strings per agent to the existing snapshot burst, with no extra event, timer, worker, or subprocess.

### The request view's verb

`agent_state/work.rs` owns the row’s PR and issue association, the live PR ordering and the one row holding each PR’s duty.
`agent_state/turn.rs::verb_of` owns the verb independently of the sidebar group.
`request_view.rs` assembles the block and keeps verb timestamps through the existing ledger.
Each agent row also carries a `request` block (`herdr-core/src/request_view.rs`, PRD overview-request-view): the operator's last request with who sent it, the last reply, the row's pull requests, and one verb the request view groups by.
The verb is computed in the core from the axes above and the row's pull requests, never by the shell, and the first rule that holds wins:
an active menu or plan approval, or an unread AI question, is `answer`; an unread block is `blocked` (a read one is `stopped`); a running agent is `working`; then, over the open pull requests whose duty the row holds, failed checks are `fix` and passing, absent or unknown checks are `review`; a turn the label read as `unfinished` or as `waiting` with nothing proven to wait on (`wait` absent), on a row with no working descendants, is `stopped`; running checks are `waiting`; an unread completion, or a pull request settled since the operator's last request and since the operator last opened the row's result (`result_opened_unix_ms` in the verb record), is `result`; a row with a `wait` is `waiting`; anything else is `idle`.
Reading an AI question skips only the demand step; another applicable duty, such as failed CI, still wins at its later step.
Only the label gives `stopped` and `blocked` (D-33); `waiting` comes from `wait` alone: with summaries off, without a provider or after a failed analysis no row stops.
The block also carries the label's `line` and `end` when there is one: the request view shows the line in place of the reply (B18) and both as the expanded row's verdict (B6); `end` is pinned as `label_end` in `contracts/snapshot-wire-enums.json`.
A row's pull requests are its checkout branch's and those its session made: a tool in the session printed the address within thirty seconds of GitHub's `createdAt` (D-31), judged once by the label worker and kept with the session's facts.
An address in a reply or a request is a mention and links nothing.
A pull request on several rows gives its duty to one row, so `fix` and `review` appear once: on its branch's checkout the row nearest its lineage root (`lineage_depth`), then the one whose session printed it, then the lowest pane id; with no row there, the row whose session printed it first.
Activity never decides, so agents on one checkout that take turns working never trade a pull request; an Observer and the implementor it spawned there keep it on the Observer's row, the one the 태스크 list draws, even when the implementor's session printed it.
A settled pull request counts as live only when it settled after the operator's last request (D-43).
The verb's time is kept per pane in `core-state.json` (`request_verbs`), so a restart does not restart the wait.
Opening a finished row in the request view (`overview_open_result`) reads the pane the way a focus does and moves no focus; a demand outlives the read as everywhere else.
Who sent a request is the label worker's verdict (ARCHITECTURE.md, Agent labels in the core); a delegated child's first request is its parent's, by the parent row's title.
The session's first request is the first request any read holds, whichever read that is: an agent can write its session before its first prompt (Grok and OpenCode do, every time), so the offset stays unset through empty reads and is kept for good once set; only a rewritten conversation starts it over.
Text another program wrote, a title, a label line, a sender's name, a request or a reply, reaches the row without control characters or bidirectional controls, and a title, line or name also without the other Unicode default-ignorable code points (zero-width characters, Hangul fillers; `herdr-core/src/display_text.rs`) and on one line and a sender's name at most 64 characters; a sender whose name reads as the row's word for the operator or an unnamed agent (`나`, `에이전트`, `operator`, compared by its letters and digits after NFKC and case folding) is shown as an unnamed agent.
Regression owners: `herdr-core/src/request_view/tests.rs` for the verb, the pull request links, duty and senders, and `runtime::tests::labels` for the title ladder, the stopped and waiting verbs with the switch on and off, the open-result read and the running-checks re-read.

### A task row's session facts

`agent_state/sessions.rs` gives each agent row its own session facts, `row.state.session`: its group among the sidebar's own (Needs You, Working, Done, Idle, and Resolved for a session resolved within the last 24 hours), the line its 태스크 row draws, and whether that line is unfinished work; no PR stage and no task tag decides the group.
Which task a row belongs to, the task's state and the order the tasks stand in are `agent_state/tally/tasks.rs`'s ([A device's tasks](#a-devices-tasks)), published as `agent_scope.tasks`; there is no separate Sessions membership any more (PRD sidebar-task-lens D-16, D-40).
A Needs You row draws its `row.state.ask` (verb and what to do) in place of a line, and a blocked one, which has no ask, its cause; a Working row its label line, or while it waits on its children the line of its most recently changed working descendant; a Done row its label line.
An Idle row carries a line only for a stopped turn with work left (the same rule as the sidebar's kept line), marked `unfinished`; it draws the row's own mark, never a mark of its own (status-marks B20).
A blocked menu takes Approval before an unread AI question's Answer.
A current native `user_turn.kind = question` instead takes Answer while its session holds for a reply, including after it is read and when its content is absent.
Native plan approval keeps Approval; the typed native wait, rather than a provider name or an AI question label, distinguishes these cases.
Reading an AI question skips its demand rung and leaves a dimmed question; menu and plan approval stay in Needs You until answered.
`row.state.pr` is the row's own PR summary (PRD D-18, D-39): the live PRs it holds the duty of, never a descendant's, with the count, the worst state and each PR worst first, each state the one rule of Pull request visual states decides; a closed PR is not counted.
Without a label line the row keeps its outline but carries no invented line.
Delegated children are never tasks of their own: they are drawn under their root through every row the operator expands, at any depth (`sessions_expanded_agent_pane_ids` in core UI state holds each expanded pane).
A Hide AI row for a home is a task only while it is in Needs You, where it draws the home's turn under the agent's own ask or block cause and opens the home when chosen.

Resolve records `resolved_sessions` in core UI state and publishes the hidden row only after the existing coalesced writer acknowledges that exact save.
Failure keeps it visible with the existing actionable save error; input, activity, session replacement or pane closure invalidates a pending acknowledgement.
New operator input, a received letter or renewed working activity restores the session.
Automatic resolution requires a stopped agent, no demand and every assigned PR merged or closed; settlements before restored input cannot resolve it again.
A resolved task rests, folded under the 태스크 list's `쉬는 중`; resolutions older than 24 hours (`RESOLVED_WINDOW_MS`) leave the Resolved group and stay among the resting tasks.
The existing runtime tick advances that window without adding a timer or per-snapshot clock read.
Closing a pane prunes its resolution and input record.
Regression owners: `runtime::tests::session_state`, `request_view::tests`, `web/src/TaskLens.test.tsx` and `web/e2e/session-panel.spec.ts`.

### The session work record

A 태스크 row keeps its live group and navigation and adds a record action that opens the record in the lane's 보기 slot; it carries no missed-work flag.
The record's title follows the same summary/native/manual rules, then shows the current status, requested work with the full source requests, tabs for the session's own PRs, open findings and request history, and one flat list of direct children.
The PRs come from the record's `outputs`, filtered by each source's full session identity; a child's results are available when that child expands, and no wire field was added for this layout.
The checklist lists tasks first-asked first with the finished count over the total.
A request that asks for several independent deliverables (a numbered list, separate bullets with their own result) becomes one task per deliverable, one to six, the extras bundled into the last name; a question, an explanation, a progress check, a sub-step or a detail of one deliverable stays one task or none, and a task's name comes from the operator's request, never from the assistant's reply (PRD task-split-checklist D-01, D-02).
A split request's tasks are one group: a later request that continues any of them continues all of them, and they finish together, so items of one request show one shared state; a missed item's task name and the group's pull request and child attribution use the group's first task (D-03, D-08, D-13), independently of the record's flat presentation.
Records written before requests split keep their one task per request; nothing is split again.
The pane-header task popover keeps its direct-child count and uses the spawn turn's `spawn_task` attribution, falling back to start time when absent; the session record instead places children in one flat `함께 진행한 세션` section after the current session's own tasks and tabs.
The record filters outputs by full source session identity, and findings stay in the record that owns them, so descendants never inflate the current session's tab counts.
The task box counts direct children as `맡긴 일 N · 끝남 k` (PRD task-continuation D-08): a working child is never done; a resting child is done when it sent its parent a report letter and started no turn after it (the delivery ledger's newest report, which the runtime hands the record with the lineage facts), or when it has a merged pull request and none still open; otherwise it is done when its own tasks are all done by the task rule, or with no task when its last END is Done.
A grandchild adds no count, and while any direct child is unjudged (neither a task nor an END, or a newest turn that went unjudged and no fact above finishing it) the line leaves out `끝남 k`, never a false zero (task-split-checklist D-04, B8; task-continuation D-09).
The session record shows each task's full source request and the latest request without a task; raw requests remain available while Hide AI has not judged them or is Off.
Request history is a tab, newest four initially, and includes full text, author and time.
A request that orders finished work again ("설치하고 재측정해줘" after that install was done) continues the finished task: the END judgment is shown the tasks of the three newest finished requests beside the open ones, a split request whole and counted once (`LedgerContext.finished_tasks`, newest first), a continued task takes the turn's end, and continuing any task of a request reopens all of them, and so a Done task returns to Doing and is Done again at its next Done END.
A question, explanation, remark or progress check about a finished task is still no task and never reopens it, and a different deliverable is a new task.
Work that continues a task without being a new request is recorded against it (PRD task-continuation D-01): an answer, approval or choice replying to what the agent asked or proposed continues the task that turn worked on, by the END judgment (D-04); a turn woken by a report, progress or block letter from a descendant this session spawned is recorded by the core against the task group of the turn that spawned that descendant's branch, never one the model picks (D-02).
A finished task takes such a turn only when its END keeps it finished, so the question of the turn whose merge finished the task keeps its `!` (B3, D-07) and no letter reopens a finished task (B5).
The letter's sender is matched by the name its envelope carries against the lineage facts' child names, read the way the envelope bounds it; a watch alert, a reply, a letter from outside the lineage or a name two task groups share records nothing, and a lineage read that fails records the turn on no task rather than losing it, a store failure retrying it whole.
Lineage lives in the links worker's memory, so after a start such a turn waits until the runtime's first lineage facts arrive, which go out even when there are none.
A turn with no request of its own starts at the input that woke it, so a pull request it made or a child it spawned belongs to the task it continues (D-03).
An assigned task is Done (PRD task-continuation D-05, D-06): with at least one merged pull request and none still open, while no child its group spawned works in a live pane (a resting child, one whose newest turn went unjudged, or one no pane runs any more holds nothing; B23), whatever its last END says except unfinished or blocked, so a task whose turn ends on a separate question is Done and keeps the `!` before its box while the question stays in the `지금` box (D-07); with no pull request, or only ones closed unmerged, only when its last END is Done.
Review, failed CI, unfinished, blocked and question remain distinct states of a task that is not Done, while waiting uses Doing.
When the session's newest turn went unjudged (a failed END, or a request seen with summaries off), a task that is not Done carries `unjudged` and draws its empty box with no word until a later turn is judged; Done still comes from the facts and the last judged END, and only a turn that can move a task (the operator's or the parent's request, or a descendant's report, progress or block) leaves an unknown when its END fails, so a watch alert's or a reply's does not (D-09, B4).
Descendant results can hold the parent's assigned task open, but do not duplicate the child's chapters or requests in the parent's own work.
A task's mark is a checkbox, with `!` only while a live session asks (PRD record-card-readability D-05), including on a Done task whose last END is a question (task-continuation D-07); a child row names its agent product and expands in place, with an explicit action to open its own record.

A missed item is work nobody holds once the session closes: a problem the agent reported finding outside its assigned task, or a separate task or issue with a different result that it proposed and nobody has taken.
Measuring, checking or verifying the assigned task's result, even when the agent calls it a follow-up, the task's own remainder, merge, install and app replacement steps stay with the task verdict, the pull request and CI, and are not missed work; neither is anything already recorded in an issue or pull request by number or link.
Records written before `context_label.v10` may still hold such an item with the `leftover` kind.
Missed items are durable Open, Resolved, Issue or Dismissed records.
Dismiss can be undone and a resolved item reopened through the existing backlog actions; the session record itself shows only open findings.
An open missed item is read in its session's record, within the project's Copied history window (90 days by default); the 태스크 list draws no missed-work group (PRD sidebar-task-lens D-16).

With summaries Off, the same record shows native/manual title, native status, outputs, children and every saved raw request oldest first, the last four open and the earlier ones behind `이전 N턴`.
It hides verdicts, chapters, tasks, END labels, missed items and their actions; turning summaries on restores old judgments and leaves the Off-period requests unjudged.
This record does not change the phone surface or the 태스크 list's grouping rules.
Regression owners: `links::store::ledger::tests`, `labels::tests`, `runtime::tests::session_work`, `web/src/sessionWork.test.ts`, `web/src/SessionRecord.test.tsx`, the command-record tests in `hide-session/tests/it/conversation_cursor.rs`, and the isolated session-work-ledger browser and desktop flows.

### Delegated escalation

`agent_state/escalation.rs` reads the existing doorbell and watch decisions rather than adding delivery clocks.
A child rises when its letter is held by a blocked parent or operator draft, when three bells are exhausted and the parent is again eligible after thirty quiet seconds, when the letter becomes undelivered after sixty minutes, when the child's own pane is Herdr blocked, or when the first watch warning has had no parent response for sixty minutes.
An idle or done parent still owns the child; only a closed parent pane or absent parent agent invokes the existing orphan-root rule.
`escalation::lift` carries every raised descendant to its lineage root (PRD D-10, D-27) as a `RaisedAsk`: the verb, what to do, the originating title, the agent named, the pane Open goes to, since when, the titles from the root and the checkout.
The verb is Approval for a blocked child (Answer when it is blocked on a native question, with that question as what to do), Answer for an undelivered or unanswered letter, Confirm for an unanswered watch and Draft for a letter held by the parent's input, which names and opens that parent; a blocked parent stands for itself and adds no ask.
An Answer also names the parent that has not received the letter.
The root's asks are ordered by verb, then by age; the root enters Needs You and its second line is the lead ask, unless its own demand comes first, which an AI question or a block the operator has read no longer is.
The raised child keeps its own place in the tree, and no ancestor between it and the root changes group.
Menu blocking uses Approval, a letter or question uses Answer, an unanswered watch uses Confirm, and a draft uses Draft.
The first three causes clear on receipt or a successful bell, menu blocking on answer, and watch escalation on response, cancellation or a new child activity episode.
A letter's cause (the first three and an undelivered letter) also ends for good once the child's session takes up a request written after the letter, from the operator or a letter: the child started a new turn without the answer, so stopping again does not bring the raise back.
That time is the session's own request record (`RowFacts` `operator_request` and `other_request`), which `labels.json` keeps and a device's helper reads again after a restart, so a restarted daemon gives the same answer; an agent whose session Hide cannot read keeps the cause until receipt, reply, cancellation or expiry.
Working activity hides a letter or watch cause while it lasts, and a causal reply or child disappearance clears any cause.
Doorbell hold reasons remain diagnostics and never become UI copy; the letters-waiting mark says only that letters wait on a possible draft, never why in words ([delivery.md](delivery.md#delivering-a-held-letter-now)).
Phone groups follow these same core values: the first three causes and menu blocking send one Needs You push; undelivered letters and unanswered watches retain their existing human notice without another push.
An ordinary delegated question produces no root push: the parent hears of it from its watch one tick after the child stopped on it, as it hears of a blocked child one tick after Herdr held it ([delivery.md](delivery.md#watch-activity-and-completion)), and clearing an escalation resets the existing effective push state immediately.
Regression owners: `agent_state::escalation::tests`, `runtime::tests::lineage`, and `hided/src/mobile/push.rs`.

### Quiet pane headers

`agent_state/header.rs` publishes a working-line flag, at most one notice, at most one band, and the current pane agent's own live Working progress when it has a nonblank sentence.
The notice says why the terminal is not live and is drawn in the pane's body: the device away first, then Herdr's restoration failure, sleep, and the stream's own state (ended with its exit code, released, closing, unavailable, starting, or a wheel another client holds); a device's unavailable pane reads `reconnecting` or `helper_not_allowed` by what its link waits for.
A pane with a notice takes no answer, so it has no band, progress or working line.
The band is the operator's turn and nothing else: the pane's own Approval, Answer or Blocked first, then the lead raised descendant, then the pane's own Stopped.
A pull request's verb (Fix, Review, Merge) and a finished turn (Result) have no band: the header's PR mark and status mark already say them; Working has a thin blue line and can show its own progress, while CI wait, child wait and Idle have nothing.
A band stays until it is answered, not until it is read: `Band.read` says the operator has had the pane focused since it arrived, and a question the operator read but did not answer keeps its Answer band although the row's verb has left Answer.
An ask band is quiet text on the secondary fill with a warning rail (PRD D-43): the pane's own ask starts with its verb and says what is asked (for an Answer, the row's second line, so a native question reads as itself).
A raised descendant's `RaisedAsk` starts with its verb, then the originating agent's name and `(자식)` for a direct child or `(하위 세션)` for a deeper descendant, what to do, the unreceived parent for an Answer, the elapsed time, and Open.
Its hover gives the originating agent's product, path, checkout and wait when known; a draft names the originating child but opens the parent holding the input draft.
The duplicate `외 N건` is absent when the children button supplies raised-tree access, and remains as a fallback otherwise.
An Approval band carries no reason: the core reads no approval command and a generated sentence is not one, so the band shows the ask's own default words.
What Hide could not read, such as an approval command, is not written on the band, since the operator cannot act on its absence (design principle #13).
The band and progress row overlay the terminal so state changes never resize its PTY grid.
The identity row reads the ancestor path, the status mark, the agent's logo, title, the letters-waiting pill from `row.letters_waiting`, the pane's own PR mark from `row.state.pr`, the Not connected chip and the existing controls; the ancestors' names drop first when narrow, leaving `›` and the accessible name.
A parent's second row keeps the tree icon, direct-child count and chevron at its left; without a band the rest of the row shows its own valid Working progress, or no text when it has none.
Child names and states remain in the tree popover, while the button's accessible name and tooltip describe the live count and state counts.
An unavailable device anywhere in that known subtree preserves the last topology count and connection reason in the button's name and tooltip; without a band, it also shows that reason beside any progress.
Pending or failed relationship navigation stays visible at its popover, ancestor step or band's Open with retry where available, and in the retained Agent-area status when the source pane is no longer on screen.
Regression owners: `runtime::tests::session_state`, `runtime::tests::device_kit`, `web/src/PaneHeaderBand.test.tsx`, `web/src/PaneNotice.test.tsx`, `web/e2e/sidebar-status.spec.ts`, and the pane/lineage desktop checks.

### The second line

The core chooses the row's second line from the group, and publishes it as `detail` with `status_word_visible`; the shell draws what it is given and decides nothing.
The sentences come from the agent's label: `expected_reply` (at most 40 characters) when the turn ended on a question, the one action the operator is asked for; `progress` (at most 80 characters) otherwise, what the agent is doing, has done or waits for.
A row carries one or the other, never both.
A native question waiting now (`user_turn` of kind `question` with content, from any of the five sources above) is the third source, the `question` below: the first non-empty line of its text, one line, cut on a character boundary to the 40 characters of an `expected_reply` and ending in `…` when cut (`native_question_line` in `herdr-core/src/agent_state/turn.rs`).
It is the agent's own words, so it is shown untranslated, and it needs no Hide AI.
It stands in for an `expected_reply` and comes before `progress`, because a question asked in the middle of a turn has no reply sentence yet: the label worker's verdict on a turn's end runs only once the agent has written something after the operator's message, so a Claude `AskUserQuestion` with no text before it leaves the turn start's `progress` on the label for the whole wait, and that line describes the work, not what is asked.
An `expected_reply` still wins, being the shorter "what to do"; the worker clears it whenever the agent runs again, so one present beside a waiting question was made for this turn.
A plan waiting for approval gives no sentence: its text is a document whose first line is whatever heading the agent chose, so its row keeps the label's line or the status word.

| Group | Sentence |
| --- | --- |
| Needs You | `expected_reply`, else `question`, else `progress` |
| Done (unread) | `expected_reply`, else `progress` |
| Working | `progress`; a row with an unresolved demand, `expected_reply`, else `question`, else `progress` |
| Seen with an unresolved demand (a read demand, a delegated child's request) | `expected_reply`, else `question`, else `progress` |
| Seen (read completions, idle, unknown) | none |

A request outlives reading: a question, approval or error keeps its sentence until it is resolved, whatever group the row sits in, so a view can keep showing what is being asked after the operator has looked.

`status_word_visible` is true only when the group wanted a sentence and neither the label nor a native question carried one (a Seen row never shows the word): the status word stands in for it, so an emphasized or working row never has an empty second line and a pane that has no proven label yet reads as title and status word.
Beside a sentence the word is never drawn; the mark and the group heading already say it.
A delegated row follows the same table for its own group, which for an ordinary child is Working or Seen, so a delegated child that has stopped shows only its title unless it is still asking.
Regression owners: `second_line_is_chosen_by_group_and_a_request_outlives_reading` in `herdr-core/src/sidebar.rs`, and `a_native_question_is_the_second_line_until_the_label_says_what_to_reply` and `native_question_snapshot_is_current_without_ai_and_clears_after_answer` in `herdr-core/src/labels/tests.rs`; each source's question text is its reader's, owned by `hide-session/tests/it` (`user_turns` and `adapters`).

Beside `detail` the row carries `message`: the sentence whole up to the 80-character label cap (`MAX_LABEL_TEXT_CHARS`), absent when the label has none.
It is what the agent last said, and the last-words popover shows it only when the operator asks for it; the second line stays the one sentence the table above chooses.

The core publishes the sentence; when a view shows it is that view's presentation.
The web sidebar row keeps a request line in the warning color until the request resolves, at the mark's reduced emphasis once read, shows the sentence of an unread row as a bright line that goes once the row is read, reveals the full sentence up to two lines on the selected or hovered row with the rest in a tooltip, and otherwise draws one line (`web/src/agentRow.ts`, owned by `agentRow.test.ts`; docs/UI_BEHAVIOR.md).
The pane identity row shows the title without a status sentence; the operator's turn belongs to the core band described above.
A shell operation string (`forking…`, `reopening…`) retains its existing identity-row slot while it runs.
The accessibility label of a row and of a header always carries the status word, in the order title, agent kind, status word, sentence, so a row whose word left the screen is still read out with it.

### Search and Recent Panels

The ⌘K sheet's agent row is titled by the identity and subtitled by its checkout and the second line above; when the state chose no sentence it falls to the status word because the goal is already the title.
The pane id is not printed on the row; only a query holding `:` matches it, by the pane's Herdr id holding the query in its case, and lists those agents first ([UI_BEHAVIOR.md](UI_BEHAVIOR.md)).
A tab holding exactly one agent pane carries that agent's identity and mark into its Recent Panels row (`StripTabSnapshot.agent_identity`), derived in the core on every status, lineage, or strip rebuild pass; a tab with none or several keeps its Herdr label.
The core never renames the Herdr tab for this; the Recent Panels label is projection only.

### A device's tasks

A device's tasks (PRD overview-device-tasks D-05) group a root and every descendant on any device into one task, derived by `agent_state/tally/tasks.rs` from the same agent rows and lineage the sidebar draws; the sidebar's 태스크 list draws them (PRD sidebar-task-lens D-20), the Overview's Projects counts them, and the shell adds no state of its own.
A task's state is its root's, in this order: blocked when the root's demand is an error, Needs You and Done from the root's group, working from its activity, waiting while it waits on proven work (its descendants, a background task, a reply or a pull request's checks), and otherwise resting, or cleanup when every pull request merged and a linked worktree is still on disk.
A resolved root rests whatever its group was, and a descendant's state moves only its own row, as everywhere else: a child's ask reaches the card through the root's escalation ([Delegated escalation](#delegated-escalation)), never as a second state.
A project's Needs You counts the tasks whose root asks for the operator, blocked ones included and resolved ones not, and work another device's root delegated here is counted on the root's device alone.
A checkout names its issue or memo by the resolution below, and a started item rides its task instead of standing in the backlog.

`runtime/issues.rs` resolves workspace manual overrides, pane-family issue tokens, branch configuration, PR closing references, then the two supported branch prefixes.
`wire.rs` extracts metadata, and the core's own node reads the branch setting with the catalog's other path facts (`Call::PathFacts`) off the runtime lock.
The existing generation-driven GitHub reader fetches PRs, repository identity and open issues together; a 201st sentinel proves overflow and `sort:updated-desc` determines backlog order.
Missing closed or cross-repository linked issues use one bounded read-only GraphQL query, not one process per card.
A link GitHub cannot resolve (its repository or issue is gone, private to this login, or never existed) comes back `null` beside a `NOT_FOUND` error while the others are answered, and `gh` exits 1 for the whole answer; the node hands such an answer back as output (`hide-host/src/gh.rs`), the core leaves those links out with one `linked_issue.unavailable` diagnostic and their count, and any other GraphQL error still fails the read.
The issue addresses a session's tools printed join this query only when the issue is in the project's own repository, the one GitHub answered for its checkout (`LinkStore::ledger_issue_refs`): an address an agent printed from a test fixture, such as `acme/repo`, is not the project's issue and once failed every issue read of the project.
An enriched issue lookup may retry once without optional Project fields; basic issue facts remain usable and the Project failure stays in the diagnostic and GitHub availability path.
Linked issues take priority within the combined 200-identity project limit.
A failed generation preserves its last successful payload, including a successful empty payload.
The backlog keeps stale facts, and its section head says the read failed, why and where to fix it.

Manual issue writes reuse the purpose operation slot and token-first, Git-second writer.
Validation resolves an issue before writing, a failed Git mirror attempts to restore the previous token, and the runtime suppresses only an uncertain mutation until fresh metadata confirms replacement.
A validation failure or confirmed rollback preserves the existing manual override.
No GitHub mutation is allowed by this path.
The existing purpose mirror worker clears a branch issue setting when an observed worktree path is removed; a branch switch, detached HEAD, or unregistered project is not a removed worktree.
While a worktree stays detached, the worker retains its last known branch for cleanup if that path is later removed.
A rejected cleanup enqueue is retained for the next catalog synchronization.

## Row presentation and phone notifications

`agent_state/turn.rs::row_state` supplies each row’s attention, title emphasis, line mode, semantic tones, priority, search tone, close state and request timing.
The shell maps semantic tones to its existing tokens and translates status words; a tab carries only the mark tone it needs.
A waiting root intentionally keeps its hollow row glyph with a working tone, and a subdued chip and search result.
`row_tests::read_question_keeps_its_request_line_and_hue_without_operator_attention` and `row_tests::waiting_root_keeps_distinct_row_and_chip_decisions` pin these surface differences.

`agent_state/tally.rs::phone` projects the phone’s groups, roots, places and safe row values from the published snapshot.
`agent_state/turn.rs::push` owns the effective notification state and transition ledger; hided keeps pairing, push encryption, transport and delivery policy.
A read root question clears the server’s effective push state, while the phone’s existing open-page rule keeps its notification while the demand remains.
The phone reads the core’s `holds_notification` value, and `emphasized` controls its compact title.
The projection and push regression tests remain in `hided/src/mobile/{projection,push}.rs`, exercising the same public core functions the transport calls.
The desktop app's agent notifications run the same transitions on their own ledger (`hided/src/desktop_notices.rs`, PRD first-run-onboarding D-09), so a desktop notification and a phone push agree on what is new without either one's seed or mode moving the other.
A Hide AI row for a home is on the phone's list, and in these transitions, only while it is in Needs You, so the notification follows the group: one notice when it enters, none when the home's turn changes inside it, and none for its progress (PRD hide-ai-work-home B2, B10); the phone opens the agent's detail from it.

### With and without Hide AI

Here “on” means Agent summaries is enabled and the current session has a proven label.
Before a proven label exists, an unavailable provider or failed analysis adds no generated title or sentence; an existing proven label can remain while an analysis is unavailable.
“Off” means Agent summaries is disabled, including Hide AI being off; native session facts, Herdr lifecycle, lineage, read state and notification settings still apply.
The table describes operator-owned roots; delegated rows and the six escalation causes keep their ownership rules above.
Sentences keep their original language in every mode.

| Surface or condition | Hide AI on | Hide AI off |
| --- | --- | --- |
| Row title | Every adapter: manual title, then Hide AI goal, then proven native automatic title, then provider. See [Task identity](#task-identity). | Every adapter: manual title, then proven native automatic title, then provider. No Herdr workspace label or control name becomes a title. See [Task identity](#task-identity). |
| Pending native question | The turn mark, Question, Needs You until answered, including after reading. Second line: turn-end `expected_reply`, otherwise the question's first non-empty line (40 characters including a truncation ellipsis), otherwise `progress`. | The same turn mark, Question and Needs You until answered; its first non-empty line supplies the second line without an AI call. With no question text the status word stands in. |
| Plain-text question at the end of a turn, with no native wait | If the analysis identifies a question: the turn mark, Question, Needs You while unread, with `expected_reply`. Reading moves it to Seen unless it is still running; the request sentence remains until resolved. | Text alone creates no question, no turn mark and no Needs You. A stopped, unread Herdr `done` is in Unread with the plain dot, the word Stopped on hover and no sentence. A Herdr `idle` is Seen, not Unread. |
| Approval / Herdr blocked pane / native plan wait | The turn mark, Approval, Needs You until answered, including after reading. The second line follows `expected_reply`, native question, then `progress`; a plan document's heading is never extracted as a sentence. | The same turn mark, Approval and Needs You; without a native question sentence the word Approval stands in, including for a plan document. |
| AI-identified error or block with a named cause, without a native wait | The turn mark, Blocked, Needs You while unread; the second line is the cause in `progress`. Reading leaves the dimmed mark and cause in Seen unless it is running. | No cause is inferred from plain text and no Blocked mark is invented. Herdr's own lifecycle decides the group: an unread `done` is Done with no sentence, an `idle` is Seen. A native blocked pane still uses Approval above. |
| Working second line | `progress`; with an unresolved demand, `expected_reply`, native question, then `progress`. A quiet working row's display can hide its sentence until hover or selection. | No generated progress; Working stands in when there is no sentence. A working row with a native question still carries that question. Proven background tasks, working children and replies still being made still supply Waiting independently of AI. |
| Unread (unread) mark and second line | The green dot only for a finished verdict, the plain dot for work left or an unproven wait; the line leads with `끝남` or `일 남음` (an unproven wait is work left, D-17) and then `expected_reply`, otherwise `progress`, ending with `· 백그라운드 N` when a finished turn left background tasks (D-19). A turn that ended on its pull request's running checks is in progress until they end, and their end is the line. | The plain dot, no line; the word Stopped is said only on hover, in accessible names and in notifications. No inferred unfinished verdict; proven waits, running checks included, still prevent a premature stop. |
| Seen second line | A read unresolved demand keeps its sentence; a read completion, idle or unknown row has none. | A native unresolved question keeps its question; ordinary read completion, idle and unknown rows have no sentence. |
| Whether a root enters Needs You | Native question/approval/block, an unread AI question or named block, or one of the explicit delegated escalations. Working and Done alone do not enter it. | Native question/approval/block and the same explicit escalations; a plain-text end question alone does not enter it. |
| App icon count | Sum of root Needs You rows while its desktop switch is on and unread Done rows while its own switch is on, across devices and including folded rows. Reading Done removes it from the count. | The same count rule, with fewer inferred demands. macOS uses the Dock, Linux a supported launcher, Windows a taskbar dot with the count in its description. |
| Desktop notification title | The root's row title, capped by the host at 120 characters. | The root's row title by the off priority above, with the same cap. |
| Desktop notification body | With `detail`: translated row `status_code` word ` · ` exact `detail` ` · ` project. Examples: `Question · Choose staging? · my-app`, `Approval · Review the plan · my-app`, `Blocked · Login required · my-app`, `Done · Build fixed · my-app`, `Unfinished · Three tests remain · my-app`, `Stopped · my-app` for a stop no label judged. Without `detail` the row's word and the project (`Approval · my-app`); a frame with no `status_code` says its section (`Needs You · my-app`, `Unread · my-app`). | The same composition. Native question: `Question · Choose staging? · my-app`. Approval without a sentence: `Approval · my-app`. Plain-text end question on an unread completed turn: `Done · my-app`. |
| Phone push title and body | Title is the same root row title. With `detail`, the phone language's row status word, exact sentence and project use the desktop format. Without `detail`, the row's word and the project; a payload with no `status_code` says the section (`내 차례 · my-app`, `읽을 것 · my-app` in Korean). | The same composition from native facts only. Native question in Korean: `질문 · Choose staging? · my-app`; approval without text: `승인 · my-app`; a plain-text question on an unread completed turn: `끝남 · my-app`. |

The status prefix is the row's translated `status_code`, not a new classification: English uses Question, Approval, Blocked, Done and Stopped; Korean uses 질문, 승인 대기, 막힘, 완료 and 멈춤.
An empty project omits its separator, and the desktop host's existing 400-character body cap still applies.
Both transports copy the root's published `detail`, even when the phone's compact row hides that detail; neither recomputes a sentence or takes a descendant's text as its root's.
Each desktop group has its own switch: Needs You enables demands and its icon count, Done enables completions and its unread icon count, and either enables the permission/test row; the phone keeps its independent push mode.
Only a transition into an enabled group announces anything: a sentence changing by itself sends no second notification, and replacement tags, withdrawal, the desktop backlog of 16 and each channel's existing mode and focus rules are unchanged.

This is the operator-approved reversal (2026-10-13) of mobile-companion's “never a sentence” rule: a phone lock-screen preview can now show the agent's or generated sentence, according to the phone's preview setting.
First-run-onboarding D-11 changes with it: both channels use the same status/sentence/project order, while their existing no-sentence translations remain intact.
The payload is encrypted, terminal content is still excluded, and neither transport logs the sentence.
The phone's serialized JSON is limited to 3,993 bytes before encryption; the 86-byte aes128gcm header, one-byte delimiter and 16-byte authentication tag make the largest accepted HTTP body exactly 4,096 bytes.
An oversized payload, including project, identifiers and the existing `clear` list, fails delivery before any POST with a fixed size-limit diagnostic; the sentence is never silently shortened to fit.
The status and sentence fields are additive: a cached old worker ignores them and keeps the previous body; a new worker with an older page's cached words falls back to that page's group word, and with no stored words it can show only the sentence and project.
Human delivery notices keep their existing words and carry no row sentence.
Regression owners: `hided/src/desktop_notices.rs`, `hided/src/mobile/push.rs`, `web/src/agentNotices.test.tsx`, `web/src/mobile/language.test.tsx` and `web/src/mobile/serviceWorker.test.ts`.

### Scope projections

`agent_state/tally/scope.rs` publishes `agent_scope` on the navigator, each device, each project and each checkout.
Physical group totals preserve every reported row; Overview members use the first checkout owner and deduplicate a pane within each project, while checkout marks retain the existing last-owner rule.
Physical row references include an occurrence index so duplicate pane IDs keep their distinct labels and states; Overview and task membership retain the first source occurrence.
The retained request projection keeps its previous internal scope semantics.
The device's tasks separately take delegated children only under their root.
Disconnected devices have empty physical totals but retain their last Overview members, matching the existing rail and board behavior.
The scope cache compares owned agent rows, device connection facts and checkout membership and summaries; it restores cached values after a catalog rebuild and recomputes only when those inputs change.
The frozen screen counts are asserted by `agent_state::tally::scope_tests::physical_groups_root_headings_and_requests_keep_the_frozen_screen_values`; ownership, unchanged projection and disconnect retention are asserted by `runtime::tests::agent_scopes`.

`agent_state/tally/lineage.rs` projects the list headings, the sidebar tree, folded checkout badges and checkout trees with their two card representatives.
The sidebar uses the connected-device tree.
A done descendant turns a card yellow only when it is a root relative to that checkout.
Folded checkout lines carry status-priority tiers; the browser keeps its existing locale-aware alphabetical placement inside a tier so Korean and English labels keep their displayed order.
The cross-checkout counts, relative-root highlight and fold transitions are asserted by `runtime::tests::agent_scopes::checkout_trees_and_folded_badges_preserve_cross_checkout_lineage_and_priority`.

`agent_state/work/board.rs` owns the PR list's branch and maker association, issue chip source, ancestor rows, attention ordering, groups and open count.
The maker remains listed after moving to other work, but only the branch's agents can make the PR read as fixing or needing review.
`runtime::tests::agent_scopes::pr_board_keeps_branch_turn_separate_from_its_maker_and_tracks_issue_changes` pins that distinction and GitHub-only invalidation.
`agent_state/work.rs::row_work` supplies the selected PR, additional-PR count and ordered issue keys for expanded and folded request rows.
It preserves live-PR-first selection, checkout-task precedence and issues closed after the request; issue facts and checkout task changes invalidate the scope cache.
The agent palette receives its checkout and closing-issue keys from `tally/relations.rs`.
The device scope also carries raised sections (five Needs You rows and three Done rows before overflow) and each numbered agent's first checkout owner.
`runtime::tests::agent_scopes::raised_sections_keep_five_questions_three_completions_and_first_number_owner` pins those limits.

`agent_state/tally/close.rs` publishes the existing pane, tab, checkout and project close targets' confirmation decision, stop-work rows and outside-descendant lists and counts.
The first unknown target still takes priority over confirmation; a subtree sheet includes every target while counting only descendants outside it.
The shell chooses the current target and translates the published states; runtime close enforcement is unchanged.
A removal dialog whose target has left the catalog keeps its existing result without requesting live close consequences.
`runtime::tests::agent_scopes::close_consequences_keep_unknown_priority_and_outside_descendant_counts` pins these rules and pane-only invalidation.

`agent_state/tally/tasks.rs` publishes each project's tasks with the scope: cards in state order, their members on every device root first, the devices and projects they delegated into, the incoming entries of another device's roots, and the Needs You count.
A pane is placed once, at its first occurrence across the local and last-known remote catalogs, so the same task is never counted on two devices, and a child whose parent's machine cannot be resolved is a root of its own.
`runtime::tests::agent_scopes::a_task_is_its_root_and_every_descendant_with_their_projects_and_devices` and `runtime::tests::agent_scopes::tasks_stand_by_state_and_fold_a_merged_worktree_as_cleanup` pin these rules.

`tally.rs` also owns local and remote device counts and project-removal running-agent totals.
Device scopes publish the current listed rows by source index, preserving duplicate physical rows and local-first order, and the first checkout place for each pane.
The overall scope concatenates connected listings; a disconnected device keeps its catalog places but marks them unavailable to the live sidebar.
The PR board publishes its turn/fixing/blocked counts and the review/draft/finished-agent breakdown, including the existing precedence of a finished agent over draft status.
The scope ownership/disconnect and PR-board tests above assert these values.

The command palette reads `tally/relations.rs` groups and row depths for the selected agent, including its in-checkout ancestors, outside parent and descendants in other checkouts.
It translates their tags and resolves PR/issue labels without rebuilding lineage membership.
The relation fixture in `runtime::tests::agent_scopes` covers both the parent caption and the cross-checkout child group.

`turn.rs::AgentUse` and `tally/cleanup.rs` retain checkout-removal use counts, including the last known busy descendants on disconnected devices.
Their unknown-state and per-ancestor counting rules remain distinct from live close-sheet consequences.
The existing cleanup tests in `runtime::tests::lineage` continue to exercise the public runtime entrypoint.
Delegated-tab placement, active-agent project context, request descendant-question counts and the cleanup row's working indicator also read module-owned answers.

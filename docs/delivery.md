# Agent delivery and inactivity watches

Hide's core owns agent registration, spawning and lineage, the single mailbox and inactivity watches for local and connected device agents.
This guide is the public operating contract.
Approved implementation contracts and run state stay in the private local harness.
Human Inbox UI, relay/escalate commands, authority proof and automatic draft clearing remain follow-up work.
Derived UI escalation is separate: the core maps the existing delivery and watch facts to the six causes in [status-model.md](status-model.md#delegated-escalation), without changing delivery clocks, retries, authority or the mailbox protocol.

## Commands and caller identity

These commands, the `hide agent` commands among them, need the running daemon and a current agent pane in a registered checkout; they work without an open renderer.
`hide guide` is an agent's guide to all of them, and to the Workspace commands and Project Memory, and needs neither: it prints `hided/assets/guide.md` with the session-start rules (`hide_agent_hooks::PURPOSE_CONTEXT`) inside it, so the guide and the session note cannot say two different things, and a skill or a hook that teaches an agent Hide points at it rather than copying it.
The daemon binds the caller through the existing Workspace credential boundary and resolves the actual pane, provider and native-session identity from Herdr.
Only a pane-bound caller is accepted: a checkout-bound caller (a plain terminal, or a tool shell inside Codex's shared app-server daemon) is refused `agent_pane_required` whatever pane it names, and its next step is to run the command inside the agent's own pane, or to run that Codex without the shared daemon (`--no-daemon`).
A pane hint cannot replace that binding: a pane-bound caller whose hint names another pane is refused `caller_identity_conflict`, and an absent, ambiguous or changed occupant returns an explicit error.
Each queued command revalidates the caller's pane against the prepared Workspace and checkout context before applying or saving; a moved checkout returns `caller_context_changed`.
Mailbox callers and new recipients require a positive native-session binding; a missing binding returns `native_identity_required`.
Two missing native references in the same pane never authorize retained mail.
A live pane move may change Herdr's public pane address while keeping the same native terminal and session.
The registration stores the terminal identity separately from its address; a unique fresh observation on the same device and Herdr scope with the same native session and agent kind continues that registration, letters, watches and spawn receipt in one ledger transaction.
A missing identity, another session, a reused address or an ambiguous terminal never proves continuity.
An inherited Workspace capability follows Herdr's caller alias only after the terminal identity and shell process birth still match, and its registered checkout context must remain the same.
Target names and pane IDs resolve against the daemon's current observations.
Herdr numbers panes per machine, and an agent learns the numbers from its own machine's Herdr, so a bare pane ID (`w1:p2`) names the pane on the caller's own machine: a device agent's `w1:p2` is that device's pane, and the core's own agent's `w1:p2` is the core's.
A bare pane ID never reaches a pane on another machine, even when only that machine has the number: the command is refused `target_unavailable`.
A pane on another machine is reached by the agent's name or registration ID, or by the full address of a device's pane, `remote:<device>:pane:<id>`.
The same reading applies to the pane IDs of `--actor` and `--observer`.
A name that two machines share, such as the default `hide-claude-w1-p2`, is refused `target_ambiguous`.
Each refusal's `next_action` says so: `target_ambiguous` points to the registration ID from `hide agent list` or the full address, and `target_unavailable` says a bare pane ID names a pane on the caller's own machine and points to the name or registration ID; neither is told to retry.
A connected device recipient uses its node's pane service over that device's existing SSH link; the credential fixes its device, link, pane, sender and kind, and losing the link revokes that credential immediately.
The only ledger remains on the controlling daemon.
A disconnected device hook finishes within its two-second budget with no letters, leaving them pending in that ledger.

An installed hook carries the state folder of the daemon whose kit wrote it, independent of the agent shell's `XDG_STATE_HOME`.
The helper gives that captured route to sibling `hide` calls only when `HIDE_STATE_DIR` is absent or empty; capability validation and caller binding remain the CLI's responsibility.
This keeps a Dock-launched app reachable when the shell uses another state home or the agent omits Hide's environment from a hook.
After a state move, the next kit pass rewrites an unedited generated route to the running daemon's folder, while an operator-edited command waits for Reinstall.
Until then, a missing old daemon leaves the letter pending and records the helper's existing `intake.failed` diagnostic with code `unavailable`, rather than confirming mail it did not deliver.
See [agent-hooks.md](agent-hooks.md#what-is-written-and-where) for the argument, install versions and the separate pane-report protocol, which stays at 8 through this update.

```sh
hide request send child-name --intent task-question-1 --body 'Please report the check result.'
hide request reply letter-1 --intent task-answer-1 --body 'The check passed.'
hide request show letter-1
hide request ack letter-1
hide request cancel letter-1
hide inbox
hide watch start child-name
hide watch list
hide watch stop watch-2
hide request send parent-name --kind progress --intent task-plan-1 --body 'Plan: core first, then web.'
hide request send parent-name --kind report --intent task-complete-1 --body 'The check passed.'
```

A `request` or `block` letter waits for an answer; a `progress` letter is an interim report, such as a plan or a checkpoint, and a `report` letter says the work is finished.
Only a `report` ends the parent's watch on its sender ([Watch activity and completion](#watch-activity-and-completion)), so a child that has more to do after what it writes sends `progress`.

The recipient can acknowledge or reply; the sender can cancel.
A `request` or `block` letter waits for its answer until one of four things ends the wait: the recipient replies, the sender cancels, either side's registration ends, or 24 hours pass since it was sent ([Persistence, clocks and limits](#persistence-clocks-and-limits)).
When the last two end it, the letter keeps its `state`, `waiting_answer` turns `false`, `finished_at_unix_ms` is set, and `answer_wait_ended` says why: `party_ended` or `deadline`; it is `null` on every other letter.
The sender reads it in the answer of `hide request show` (and in every other answer that carries the letter); `hide inbox` lists the same letters as before and does not announce a wait that ended.
A letter that ended its wait without an answer no longer counts against the open-letter limit, and the recipient can still reply to it by the same rules.
It is still a retained letter: it leaves the ledger one day after its wait ended, or earlier when the 5000-letter total is full and it is the finished letter that finished first (see [Persistence, clocks and limits](#persistence-clocks-and-limits)).
Acknowledgement is the recipient's receipt for every agent kind: only the recipient's own pane and native session can acknowledge (`hide request ack` from a pane-bound caller is the only path, with no operator or helper path), so it records `hook_confirmed: true` and ends a matching report watch (a `progress` letter ends none); a reply separately closes the original request's answer wait.
Retry the same intent after an interrupted call: the same sender identity and intent return the existing letter during its retention period, including after cancellation or delivery, for one day after the letter finished or until a full ledger drops it first; a send with the same intent after that creates a new letter.
Use a new intent for a new letter.
The envelope identifies the sender and letter kind; it is not a session-level authority or anti-forgery proof.
The current first-line format is `Hide letter <id> from <name> (<agent>) [<kind>]`, with independent writer and transcript-reader examples in [delivery-envelope.json](../contracts/delivery-envelope.json).
`<name>` is the sender's [wire name](#names-on-the-wire-and-on-screen), and so are the names of a letter's and a watch's endpoints in every answer.
A batch keeps the first letter's sender attribution in the request view; neither later headers nor older delivery formats select a sender.

### The contract a calling tool checks

A tool that runs these commands from a program (such as a dispatch client) checks the installed `hide` before it calls one.
`hide version --json` answers `{"version", "commit", "contract"}`: the version the build reports, the commit it was built from (`null` for a debug build given none, or a build from a tree with no Git), and the `sha256:` digest of the command contract; neither it nor `hide contract --json` needs a daemon or a pane.
`hide contract --json` answers that contract:

- `format`: the document's own format, now 1.
- `commands`: each command's words, its positional `arguments` with their value types, `repeats_at_most` for a last argument that repeats, its `options` (each with `name`, `value` type or `null` for a switch, `required`, and the options it `requires`), what `rest` passes through after `--`, the names of the `answers` it can return, and, where the contract declares them, the `refusals` its own rule answers with; those are not every failure, since every command can also fail for the daemon, the pane or the ledger.
- `value_types`: what each value type admits (`text`, `key`, `body`, `approval`, `unsigned`, `one_of` with its `values`).
- `envelopes`: for each topic, where the printed line carries `ok`, the `answer` and the failure `code`; a failure also exits non-zero.
- `answers`: the JSON Schema of each answer, and `digest`, the `sha256:` of the document without it, its keys sorted and written compactly.

A caller compares the digest it was written against, or the commands and options it uses, and refuses a mismatch before running anything.
The CLI admits only the commands and flags the contract names (`hided/src/cli_contract.rs`), its tests hold each parser to the table, and the answer schemas come from the core's answer types (`herdr-core/src/delivery/answer.rs`), so a changed command name, option or answer field changes the exported contract.
A command whose standard output is closed by its reader (`hide agent list | head`) ends quietly with status 141 and prints nothing to standard error; `docs/ARCHITECTURE.md`, The platform layer, owns the rule.
`contracts/hide-cli.json` is the committed copy, and a test fails until it is regenerated with `target/debug/hide contract --json | python3 -m json.tool > contracts/hide-cli.json`.

## Safe intake and manual fallback

A doorbell types one line that says what arrived ([The bell line](#the-bell-line)).
It is typed when every one of these hide-owned facts holds, and the verdict reads no terminal screen, so a redrawn footer or a user's statusline cannot change it (the one screen read, of a composer held as a draft, happens before the verdict and can only release that draft, [Reading the composer](#reading-the-composer)):

- Herdr reports the pane `idle` or `done`, and has for 30 seconds; or the adapter declares no tool-letter intake, Herdr reports it `working` and the letter's wait is over ([Letters while the agent works](#letters-while-the-agent-works)).
- Hide has routed no key to the pane for 30 seconds.
- No input hide routed is newer than the later of the pane's last submission and the last time it entered `working`.
  Typed keys, a pasted attachment, a phone key and the keys of an agent's own find box all count as input, and none of them, Enter included, counts as a submission.
  A submission is a prompt hook that ran with the pane's own native session id, which the hook passes to `hide inbox --hook --session`, or a phone reply.
  A pane that entered `working` was submitted to as well, which is how an answered menu or a custom slash command that starts a turn clears the hold.
  A key after both is an unsent draft, an Esc-restored prompt or a recalled input; a pane held by one stays held until the next real prompt, or until its composer is read empty ([Reading the composer](#reading-the-composer)).
- The pane still hosts the native session the letter was written for, and the agent kind is one the bell targets.
- For Claude Code, Codex and Grok, whose session readers report turns, a read made for Herdr's current state says no native question or plan approval waits for the operator (see [A menu Herdr reads as a stop](#a-menu-herdr-reads-as-a-stop)).
- Herdr does not report the agent as starting or not ready.
  Herdr reports readiness only for an agent it launched itself (`herdr agent start`, which `hide agent spawn` uses): `launch_pending: true` while that start settles or is blocked, then `interactive_ready: true`.
  An agent the operator started by typing its program in a Herdr shell, the usual way a lead is started, carries neither flag; its readiness is unreported, not refused, and the facts above decide it.
  Only `launch_pending: true` or an explicit `interactive_ready: false` holds the letter.
  The pinned Herdr settles a start only when the agent's state moves again or it is read with `agent.get` (herdrdev/herdr#3853), so the core reads every start Hide made whose agent rests with it pending, from 3 s to 90 s after Herdr took it and a bounded number of times, and a pane that is not a bell target settles too ([ARCHITECTURE.md](ARCHITECTURE.md#the-core-the-shell-and-herdr)).
  A start Hide did not make, such as an operator's own `herdr agent start`, or one Hide made before its daemon restarted, is never read, since a read past that start's deadline would make Herdr clear the agent's name; it is logged once as `launch_settle.cold`, and its letter waits until the agent's state moves.
  A start Herdr reads as blocked (a trust or login prompt at launch) stays pending until the agent goes idle, by Herdr's contract, and its letter waits until then.
  A read after the start's deadline (120 s) is where Herdr applies the deadline and clears the agent's name, as the agent's next state change would; the read does not hold that off.

Herdr's `blocked` status holds every permission or selection menu it reports that way.
For a menu it reports as idle or done, a bell target also needs Hide's draft record or a native session wait to hold the bell.
Claude Code and Codex can be rung at rest and while working; Grok can be rung only at rest, with Hide's draft guard and the native-session wait guard below.
Repeat the complete scene matrix with the [local measurement tool](VERIFICATION.md#local-agent-bell-measurement) before enabling another target or relying on a result after a Herdr or agent CLI update.
Its ignored run report records active detection-manifest versions and native outcomes, including unsafe picker/startup effects and unknown cells.
The adapter declaration is a reviewed product decision; the tool never changes it automatically.
The shared agent adapter declares bell eligibility separately from prompt intake, spawn refusal and session format; its contract requires a submission hook and either prompt context or a turn-end intake hook for every bell, and adding prompt intake alone never widens the bell targets.
Claude Code's plan approval reads `blocked`; Codex's does not, and the session read guards it instead (next section).
Cursor is not a target: its menus have not been observed, and the agent checks leave Cursor out by the operator's decision of 2026-10-17.
Grok is a target at rest: a kind with tool letters never receives a working bell, so its plan review, which Herdr reads as `working`, is held.
Its bell uses the same Herdr `agent.prompt` submission as every bell target; its prompt hook records the submission and recognizes the saved line, but prints and confirms nothing.
The bell turn takes its pending batch at the first `PreToolUse`, or at `Stop` if it calls no tool.
The following observations were made on 2026-10-18 with Grok 1.0.52 and 1.0.62 under isolated Herdr 0.9.5 (Grok detection manifest 2026.10.05.1), answered by a scripted stand-in model.
The picker observations below used the then-current `pane.send_input` and Enter path; they are not evidence for `agent.prompt`.
Grok's three hooks did the same on both versions:

- `UserPromptSubmit` runs with the prompt and the session id, but nothing it prints reaches the model (Grok's hooks guide: "stdout of an allowing hook is discarded").
- A `PreToolUse` hook's `additionalContext` reaches the model after the tool's result, which is how Grok's letters are handed over while it works.
- A `Stop` hook's block reason reaches the model as "Stop hook feedback", and the turn goes on.

| Scene | Herdr reads | What the line did | Seen on |
|---|---|---|---|
| At rest, over the welcome panel or a suggested next line | `idle` | Submitted as one prompt | 1.0.52, 1.0.62 |
| A turn running | `working` | Waited in Grok's own queue and ran as a turn of its own after the current one | 1.0.52 |
| Shell and file permission menus | `blocked` | Typed into the shell menu, chose nothing | 1.0.52 |
| A question (`ask_user_question`) | `blocked` | Chose the first option with the line's `1` | 1.0.52 |
| Plan review (the terminal title says `Running: Plan: Exit`) | `working` while it waits | Opened a comment on the plan and was lost; the same text typed as keys approved the plan with its `a` (1.0.52) | 1.0.52, 1.0.62 |
| `/model` picker | `idle` | Taken as a model name: "Unknown model" | 1.0.52 |
| `/resume` picker | `idle` | Taken as a search; the picker stayed open | 1.0.52 |
| Folder-trust question at start, before Grok has a session | `idle`, ready | Its Enter trusted the folder; the missing native session holds the bell today (`session_changed`) | 1.0.52, 1.0.62 |

Cursor's letters at rest are read through `hide inbox`; Grok's rest bell opens a turn that takes them at a tool call or turn end, and while they work their own tool hook hands due letters over ([Letters while the agent works](#letters-while-the-agent-works)).
OpenCode, Pi and omp are not targets either, though Hide's plugin and extension give them a prompt hook: on 2026-10-07 OpenCode's previous-session picker read `done`, and a bell's Enter there opened another session, and Pi's and omp's resume pickers read `done` too.
Their letters ride their next prompt instead (below), or a tool result while they work, and a bell is reconsidered only in the change that would flip an adapter declaration, once the live-check tool has found a bell harmless in every scene that agent does not read as `blocked`.
Herdr 0.9.1 reads a built-in slash picker such as `/model` or `/resume` as `done`, not `blocked`, for both targets, and a bell typed into an open picker is accepted by it.
Hide therefore does not take the Enter that opens a picker for a submission: the pane holds as a draft, and stays held after `/clear`, `/model` or `/help` until the next real prompt runs its hook.
The same holds after an Esc or Ctrl-C that interrupts a turn, since no prompt hook runs for it.
On Grok 1.0.62 a completed background command fires `UserPromptSubmit` with the root session id and a `<system-reminder>` describing the completed task; its digest is not the saved bell line (measured against a local fake model).
Two residuals remain: a turn the operator did not start (a scheduled wake or a finished subagent, including Grok's auto-wake turns) moves the pane to `working` and clears a half-typed draft, a prompt queued while the pane works leaves it held after its turn until the next prompt submitted from rest (its hook runs while `working` and clears nothing).
A letter that cannot be belled waits, and the reason is logged as `doorbell.held` once per change with the letter and pane ids, never with its body and never on screen; the one hold that does not end on its own, a possible draft, is shown on the pane once it has rested ([Delivering a held letter now](#delivering-a-held-letter-now)).

### Letters while the agent works

A letter to an agent that is working no longer waits for its turn to end (PRD letters-to-busy-agents).
It waits five minutes of unbroken work (`doorbell::MID_TURN_WAIT_MS`), counted from the later of its arrival and the moment the pane entered `working`, so an agent that just started a long turn is not interrupted at once and a letter that arrives late in a turn is not handed over on the next tool; a letter sent with `hide request send --now` (`Letter.immediate`) skips the wait and goes at the recipient's next safe gap.
How it reaches the agent follows the kind, and one letter takes one path and is never both belled and handed over at a tool (D-10); the adapter contract holds `bell` and `tool_letters` to be each other's exception:

- Claude Code and Codex, the bell targets, take the line the doorbell submits through Herdr's `agent.prompt` while they work: both read a line submitted during a turn after the current tool call, and the line's prompt hook takes the letters as it does at rest.
  Every other guard of a bell holds as it does at rest: a draft typed since the work began, a key or a status change in the last 30 seconds, a menu, a session not read, and a `working` state Herdr has not yet confirmed (`state_change_seq`), so the composer is never typed over the operator's own words (D-09).
  A pane belled while working is not belled again for a later letter until the letter it rang for has left `pending`: submitting a bell to a working pane moves nothing Hide can see, so the second ring would otherwise follow at once (`doorbell::Rung`).
- OpenCode, Pi and omp take them at a tool result: Hide's plugin (`tool.execute.after`) and extension (`tool_result`) ask the helper's `letters` operation, which runs `hide inbox --hook --session <id> --tool`, and append the bodies to the tool's output inside `<system-reminder>`, so the model reads them before its next step; they are confirmed as a prompt's letters are: OpenCode's once it stored the completed tool part of that session with the letters still in its output (OpenCode 2's once `session.tool.success` reports the result stored with them), Pi's and omp's once the host finished the result with Hide's text still in its content and the reply after it has ended (a `message_end` handler may still replace the result itself); a result stored or finished without the text never reached the model, and its letters stay pending in Hide for the next ask.
  While letters a script handed over wait for their receipt, the host's write and then Hide's confirmation, the script asks nothing, so a letter is never written twice; a write that never comes is forgotten after ten minutes, and the letter then rides the next ask.
- Grok and Cursor take them through their own `PreToolUse` and `preToolUse` hook: a call that runs is answered with the letters as the hook's added context (`hookSpecificOutput.additionalContext`, which Grok's guide says reaches the model with the batch's tool results; `additional_context` beside `"permission": "allow"` for Cursor), and confirmed once that answer is printed.
  Every tool call reaches the hook, since neither entry carries a matcher (Grok `hide-guidance@7`, Cursor `hide-guidance@6`; an omitted matcher matches everything in Grok's guide, and Cursor's is optional), so a `--now` letter goes at the next call whatever its tool; a refused call carries none.
- Antigravity takes them through its `PreInvocation` hook, which agy runs before every model call of a turn: a call after the prompt's first is the gap after a tool, and Hide answers it with the letters due as an injected system message (`injectSteps`, as the prompt's letters go), confirmed once printed; agy has no bell, so this and the next prompt are its two paths.

The ask (`Command::Pull { point: Tool }`, node protocol 47) names the pane's own native session and is refused with `session_mismatch` for any other, since it hands letters over; it submits nothing, so a draft in the composer stays a draft (D-09).
Grok also asks at its root session's `Stop` with `hide inbox --hook --session <id> --turn-end`.
A nonempty answer becomes `{"decision":"block","reason":<letters>}`, followed by confirmation after stdout flush; Grok feeds the reason to the model and continues the turn.
A second Stop with `stopHookActive: true` makes no letter pull and ends normally even if confirmation of the first handover failed.
Tool and turn-end intake share the three-handover limit and the five-letter, 8 KiB cap.
Parallel tool hooks in one batch may pull the same claimed letter before either confirms it: the claim binds the bell turn, not an exclusive tool invocation.
The worker serializes pulls and durably counts each handover, so this at-least-once path remains bounded at three; confirmation makes later pulls empty.
An operator turn's new letter still waits five minutes unless sent with `--now`; ending that turn does not skip the wait, and its rest bell brings the letter instead.
The core answers from the caller's own observation: the letters whose wait is over for an agent Herdr reports `working`, oldest first, under the prompt pull's caps (five letters, 8 KiB), and nothing for a resting agent without a bell-turn claim, a letter still waiting, a letter past its deadline or a kind whose adapter declares no `tool_letters`; a letter it hands over is confirmed by the script, not received, so one lost in the host stays pending for the next ask (D-11).
The plugin and the extension ask at every tool result of the pane's root session, so a `--now` letter goes at the next result (B6), and skip only the results that come while letters they handed over wait for their receipt; Grok's and Cursor's hook runs per tool call and adds one CLI call there, and agy's per model call after the first.
Each handover the core answers is counted on the letter (`Letter.tool_handovers`), and after three without a receipt the letter is not handed over at a tool again: it waits for the next prompt or `hide inbox`, as a letter does after its three bells (`mailbox::TOOL_HANDOVERS`, D-11).
[PERFORMANCE_TESTING.md](PERFORMANCE_TESTING.md#letters-at-a-tool-event) owns the cost per tool call and how it is measured, and [agent-hooks.md](agent-hooks.md) each script's and hook's part.
The pending retry, the oldest-first order with the count of letters waiting, the three bell attempts, the three tool handovers above and the 60-minute deadline are the whole of it; a letter a working agent never reaches a gap for is belled or handed over once it rests, as before.
The verdict's reasons are `working`, `blocked`, `awaiting_operator`, `session_unread`, `draft`, `quiet_period`, `kind_not_belled`, `session_changed`, `pane_unavailable`, `status_not_at_rest` and `core_pending` (a core on a move's copy types into no pane until the move's link commits it, [ARCHITECTURE.md](ARCHITECTURE.md#moving-the-core)).
Between the verdict and the input the doorbell asks Herdr again and checks its own memory last, and each refusal there has its own reason: `launch_pending` and `not_ready` (Herdr's readiness), `identity_changed` (Herdr's agent in the pane has another name, kind or native session), `sequence_moved` (Herdr's status or state sequence moved), `input_after_verdict` (hide routed input to the pane) and `letter_changed` (the letter was confirmed, cancelled, expired or reserved meanwhile).
A letter refused there is tried again in the same pane episode after 5 seconds, then 10, 20, 40 and 80, then every two minutes, because readiness and the letter are facts the pane episode does not carry; a change of the pane's status, sequence, input or session tries it at once.
A failed Herdr call is not retried on that schedule, since the bell may already have been typed; it waits for the pane to move.
If the pane or its device's link goes away after such a failure (`pane_unavailable`), the wait ends: a bell that started a turn would have moved the pane, so a pane that comes back idle at the same state sequence is tried again rather than waiting for a move that never comes.
A letter still held when its deadline passes is logged once as `doorbell.expired` with the reason it last waited for.
After a hided restart, an app update or a device's return, a pane starts with a 30 second grace and no draft assumed, except one the last run held as a draft.
The draft is on the screen and nowhere else, so the one fact that cannot be read back is written down: `composer-marks.json` in the state directory (`hide_kit::layout::composer_marks`, written by `composer_store.rs` on a thread of its own, never under the runtime lock) names each pane, by device, pane id and native session, whose composer held an unsent draft in a pane the doorbell rings.
A mark is written when the draft begins and removed when it ends, a prompt hook of the pane's own session, the pane entering `working`, a composer read empty or Deliver now, so the file moves once a prompt and not once a key.
A pane first seen again with its mark starts as a draft, input newer than its submit, and stays held by it until the same events end it; a pane that is `working` when it comes back drops the mark, since a turn started while the daemon was away took the draft.
A mark of a pane that is not observed is kept for seven days, at most 256 marks are kept, and a missing, damaged or other-version file is ignored with a `composer_marks.discarded` diagnostic.
A pane with no mark is not held beyond the grace, so a restart never silences an idle pane for good.
The residual is the window the daemon is down: a prompt submitted then runs a hook that reaches nobody, and the mark stays until the pane's next prompt or turn clears it.
Two records trace what moved the pane's input clock, both without content and at most one a second per pane: `terminal.input` from the node for the keys it saw (`path`, `keys`, `bytes`, `submitted`; `path` is `write_key`, or `write_key_attachment_hold` and `write_key_asleep` when the key was held for a paste or a sleeping pane), and `delivery.input` from the core for the input it noted itself (`path` is `find`, `attachment`, `phone` or `phone_reply`).
The adapter never copies, clears or restores a draft.
Each letter has at most three durable doorbell reservations, including successful input and attempts interrupted before confirmation.
The reservation is persisted before input, and the adapter repeats the native inspection (`agent.get`: readiness, kind, session, status, sequence) after the persistence wait.
A refusal before the reservation spends none, so a letter Herdr keeps refusing is never charged an attempt.
A crash or changed pane after reservation may consume an attempt while leaving the letter pending.
Legacy records with a successful bell but no total count conservatively have no automatic attempts left; manual and prompt-hook intake remain available.

The `UserPromptSubmit` hook of a bell target reads the submitted prompt from its input payload and sends the core the SHA-256 of its canonical text (`hide inbox --hook --prompt-digest <sha256>`), never the text itself.
The core alone knows which line its doorbell typed, so it alone decides whether the prompt was the bell ([Recognizing the bell](#recognizing-the-bell)).
When it was, the pull takes the pending letters, oldest first, and after them the letters an earlier build acknowledged without a receipt (`hook_confirmed: false`), which the store's expiry pass before every request drops once their deadline passes, so such a backlog never displaces the letter the bell rang for; it emits their context, flushes stdout, then confirms those IDs.
An adapter whose prompt hook cannot add context, today Grok, instead records the pending batch under the native session and the saved bell digest, returning no context or count.
The first tool or turn-end pull in that session can hand this batch over immediately, without the five-minute wait.
Every own-session prompt replaces that claim, so a later operator prompt cannot inherit the previous bell's letters and a letter arriving after the bell submission cannot join its batch.
A prompt hook that can add context, on any other prompt including the operator's own, adds at most one line, `Hide 편지 N통 대기 중, 이 턴이 끝난 뒤 전달`, counting the letters a bell will still bring, and confirms nothing; the prompt text is never changed and no letter body reaches an operator's turn.
A letter whose three bells are spent stays pending for `hide inbox` and expires undelivered.
A payload that is truncated, unreadable or not read within 0.5 seconds counts as an operator prompt.
The pulls carry the payload's `session_id` (`sessionId` for Grok); the core takes the pull as proof the pane's composer was sent only when that session is the pane's own native session, so a stray `hide inbox --hook` from another tool in the pane clears no draft.
The session id is not a secret, so this guards against accidents and not against a hostile process in the pane (external input, D-18).
A hook that runs while Herdr already reports the pane `working` is a queued prompt being taken up and clears nothing, and a session longer than 4096 bytes (Herdr's own bound; Pi and omp name a session by its file path), starting with `-` or holding a control character is refused before it is hashed.
A device kit of an earlier build sends a `bell` flag and no digest: the core drops the flag (`Command::from_wire`), so that kit's bell turn gets only the count line and the letter stays pending until the kit is updated, while its OpenCode, Pi or omp pane still takes its letters on the next prompt; keep the kit and the app on the same build.
A foreign hook, including Claude Code's hook loaded inside Grok, receives nothing and confirms nothing.
Claude Code's hook run inside OpenCode, through an operator plugin that bridges `~/.claude/settings.json`, ends silently as a whole, so OpenCode's letters and counts are taken once, by Hide's own plugin.

An agent with a prompt hook that no bell rings for, today OpenCode through Hide's plugin and Pi and omp through Hide's extension, takes its letters on its next prompt, whoever typed it, since no turn of Hide's own will ever bring them (`mailbox::next_prompt_letters`).
Its `hide inbox --hook --session` pull hands over the letter bodies as a bell pull does and confirms nothing: the plugin attaches them to the root session's prompt as a synthetic part, and only when OpenCode reports that part written (`message.part.updated` with the part's id) does it run `hide inbox --confirm`.
OpenCode's `chat.message` runs before the prompt is stored, so a prompt OpenCode loses before that leaves its letters pending for the next prompt or `hide inbox`.
OpenCode 2's plugin asks for them from the terminal that shows the session, writes them as a synthetic message OpenCode admits ahead of the prompt, and confirms them once `session.inbox.delivered` names that message ([agent-hooks.md](agent-hooks.md#opencode-2)).
A subagent's prompt and a prompt of only synthetic text pull nothing.
The plugin forgets a pull it never saw stored after 10 minutes and keeps at most 16 waiting, so a stored part reported late can still confirm; past that the letter stays pending and comes again.
Pi's and omp's extension attaches them as a hidden custom message (`display: false`) that `before_agent_start` returns for the pane's own session, and runs `hide inbox --confirm` once the host has written that message and then the reply after it; omp prepares a submission more than once, so each attempt's message carries its own id and only the written one confirms, once.
A turn that ends before its reply leaves the letters pending, and an omp subagent's prompt pulls nothing.
An agent with no prompt hook reads letters with `hide inbox`, which shows an `ack_command` for each, and `hide request ack` is its receipt, as it is for an agent with one: it records `hook_confirmed` and ends a matching report watch, as the flushed hook confirmation does.
Transport arrival and the doorbell alone do not confirm intake.
Interruption before confirmation can repeat the same letter ID; confirmed letters do not appear again in hook context.
The hook emits at most five letters and 8 KiB of context, with a remaining-count line and `hide inbox` guidance when more are pending.
A large letter is truncated at a UTF-8 boundary and includes its ID and `hide request show` command for the complete body.
The installed prompt hook supervises one guarded internal operation with a 1.85-second deadline inside the two-second caller budget, including Memory, filesystem, output and diagnostic work.
The internal operation requires positive owner proof and inherits the runtime payload and output streams; it confirms only after its output is successfully flushed.
Failure leaves actual intake unconfirmed and attempts a rate-limited private diagnostic inside that same budget, under the cause of the step that failed ([agent-hooks.md](agent-hooks.md#what-a-hook-records-when-it-fails)).
A blocked diagnostic store or output stream can prevent the diagnostic from finishing; the outer timeout performs no filesystem or output tail that could hold submission open.
CLI output collection uses the canonical platform capture, with a 64 KiB bound and no reader thread or blocking join after timeout.
Cleanup uncertainty is a separate private diagnostic field and never authorizes confirmation.
Manual `hide inbox` and `hide request show` remain available when the hook is missing or fails.
OpenCode's plugin and Pi's and omp's extension hold the same 1.85-second prompt budget around their helper call, which itself stops at 1.65 seconds so the process start fits; past the budget the prompt goes on unchanged and the cause lands in the same private diagnostic (cause `plugin` for helper calls the plugin skipped or gave up on).

The pinned Herdr API has no atomic composer guard.
Hide checks the occupant before and after the durable reservation and checks its memory state immediately before the off-lock pane write, but direct external Herdr/TUI input can race that final write.
That residual limit is the approved D-18 boundary; external input is not represented as a Hide key event.
In Grok a direct Herdr client or another tool's keys can open `/model`, `/resume` or history without Hide knowing: a bell there can be swallowed as a model name or search.
The same picker opened through Hide leaves an unsent draft and holds the bell.
A background workflow approval card on Grok's daemon route is not yet verified: the measured 1.0.62 session used the shell route, where `await_user` produced an idle, paused workflow rather than that card.
Until a native `blocked` status or a session wait is measured for that card, do not treat it as a proven bell hold; this remains a native-route coverage gap.
A swallowed bell moves no observed state, so `Rung` prevents another ring in that episode; at most three later episodes ring before the letter waits for `hide inbox`.

### The bell line

The doorbell submits one line through Herdr's `agent.prompt`: `🦉 <writer> <kind>: <the letter's first line>`, followed by how many more letters wait for the same recipient when there are any, and ` · hide inbox` for a Codex or Grok recipient (the adapter's `bell_names_inbox` declaration):

```
🦉 label-end-fix report: label end fix opened PR #881
🦉 hierarchy-pen-scratch 보고: Pen 시안이 준비됐습니다 · 외 1통
```

The kind and the count are worded in the operator's interface language, the one Hide AI writes in (`Runtime::interface_language`: the language chosen in Settings, else this machine's primary language read once at launch, else English), which the doorbell reads with the recipient's connector under the runtime's lock from state the runtime holds, with no I/O.

| Kind | en | ko | zh-CN | ja |
| --- | --- | --- | --- | --- |
| request | request | 요청 | 请求 | 依頼 |
| block | blocked | 막힘 | 受阻 | ブロック |
| report | report | 보고 | 报告 | 報告 |
| progress | progress | 진행 | 进展 | 進捗 |
| reply | reply | 답장 | 回复 | 返信 |
| watch warning | watch | 감시 | 监视 | 監視 |
| N more waiting | ` · and N more` | ` · 외 N통` | ` · 另有 N 封` | ` · ほか N 通` |

A kind this build does not know reads as a letter (letter, 편지, 信, 手紙).
Nothing reads the words back: a letter's turn is recognized by the whole line its ledger kept when it rang ([Recognizing the bell](#recognizing-the-bell)), so a change of language between the ring and the turn changes nothing.
The writer is the name the sender registered under (`hide agent register`, `hide agent spawn --name`), else the title Hide's screens give it while its pane still hosts that session, else its runtime's name, such as Codex; never a pane id and never its registration id, which belong to the letter's envelope ([Names](#names-on-the-wire-and-on-screen)).
A watch warning is named by the agent it watches, and its first line says for how long nothing moved, that the agent stopped with a question, or that it is blocked at a prompt ([Watch activity and completion](#watch-activity-and-completion)).
Both are looked up in memory when the bell rings, on the doorbell's thread, never on the session-sync thread.
The first line is the body's first line that has anything to show, cut to 60 characters with `…`, and the writer is cut to 32; a body with no such line leaves the colon and summary out.
Agents learn from their session guidance that this first line is the letter's one-line summary, and that a prompt beginning with `🦉` brings a letter that `hide inbox` shows if the prompt did not.
The mark is U+1F989 without a variation selector; Codex CLI on Windows may show the line without the owl.

Whether the line names `hide inbox` is decided per runtime (`bell::names_inbox`), by whether its bell turn can arrive without Hide's hooks having run in the pane.
Claude Code runs every user-level hook it is given, so its prompt hook hands the turn the letter and its session guidance has already said what `🦉` means; its line ends at the summary.
Codex runs a user-level hook only once its own trust review approved that exact entry, so another tool's change to `hooks.json` silently stops Hide's, and a Codex attached to its shared app-server daemon runs hooks in the daemon's environment rather than the pane's ([agent-hooks.md](agent-hooks.md)); its bell turn can then arrive with neither the letter nor the guidance, so its line ends with ` · hide inbox`.
Grok's prompt hook observes the submission without adding letter context, so its line names `hide inbox` too; tool or turn-end intake still confirms the letters.

The line is submitted to the checked pane through Herdr's `agent.prompt` on every system, so Herdr owns how each agent's input reader finishes a paste and submits it.
The raw `pane.send_input` path could leave the bell unsent in a Windows input reader that treats the following Enter as part of its paste burst; the prompt API already supplies that reader's boundary ([herdrdev/herdr#3187](https://github.com/herdrdev/herdr/issues/3187)).
Both APIs use the same bracketed-paste wrapper; the leftover Windows wrapper problem in [herdrdev/herdr#5371](https://github.com/herdrdev/herdr/issues/5371) occurs when the shell holds the terminal, rather than the detected foreground agent this API requires.
Hide adds no cursor keys or submission delay of its own.
A manually started agent is eligible once Herdr detects it; missing readiness flags do not exclude it.
The pinned implementation checks for a known foreground agent and rejects a blocked or still-starting one before submission ([Herdr v0.9.5, `validate_agent_prompt`, lines 140-207](https://github.com/herdrdev/herdr/blob/v0.9.5/src/app/api/agents.rs#L140-L207)); a detected agent does not need to have been started through Herdr.
On macOS, Herdr 0.9.5 accepted `/help` from Claude Code 2.1.302 and Codex CLI 0.163.0 through six launch paths each: direct binary, the journey's exec launcher, a shell alias, a shell function, `npx --no-install --call`, and managed `agent.start`.
The manually started agents were accepted with `unknown` status and no readiness report once detected; npx calls before detection returned `agent_not_found` and both were accepted after detection.
This measures prompt admission for those two CLIs on macOS, not letter intake or other agents and systems.
The request asks for no turn wait and allows five seconds for Herdr's ordered PTY writes to finish, outside the runtime lock; success is only a bell attempt, and the prompt hook still has to confirm intake.
The existing draft, identity and readiness checks, three-attempt limit and retry policy remain in force, and arbitrary phone replies retain their own submission contract.
The line carries nothing a composer acts on while it is typed.
Control characters become spaces, characters that draw nothing or reorder text are dropped (`display_text::one_line`), and runs of whitespace become one space.
`@` opens the file picker in Claude Code and Codex and `$` opens Codex's skill picker, and Enter would accept the picker's choice instead of submitting, so both are typed as their full-width forms `＠` and `＄`; a backslash before Enter continues the line in Claude Code, so `\` is typed as `＼`.
`/`, `!` and `#` act only as a prompt's first character, and the line begins with `🦉`.

### Recognizing the bell

The reservation that spends one of a letter's three bells saves the exact line on the letter (`bell_line`) before it is typed.
A prompt hook's pull is the bell's exactly when its digest is that of the saved line of a `pending` letter whose recipient is the caller's own pane and native session (`mailbox::rang`).
The core and hook share `hide_agent_hooks::delivery::prompt_digest`: remove every character above U+FFFF, trim Unicode `White_Space` from its ends, then compare its SHA-256 exactly, with no agent or system branch.
Codex CLI 0.163.0's Windows input reader can drop non-BMP characters, so losing the owl or another such character leaves the bell recognizable; every BMP character, including U+FE0F, remains significant.
Older hooks' whole-line digests remain accepted against the saved line, including an existing `🔔` reservation, without migrating the ledger.
Remove whole-line compatibility only when no hook older than this build can still run; a saved `🔔` bell becomes ineligible with its letter's 60-minute delivery deadline, so it does not require a ledger migration.
New hooks send only the canonical digest.
A new hook paired with an older daemon that has not been restarted may fail to match the canonical digest and repeat the bell until the daemon runs the new build.
A person who types `🦉` and the same words before the doorbell saved them, a bell line copied into another agent's pane, and a line whose letter was already taken in or cancelled are therefore ordinary prompts.
Once the line is saved, submitting it in that pane takes the letter whoever pressed Enter: the prompt then names the letter it brings.
A second bell for the same letter saves its own line, so a hook of the first one that arrives after it is an ordinary prompt too.
The ledger refuses a saved line longer than 512 bytes or holding a control character.

### Reading the composer

Hide routes every key to a pane but cannot see the composer, so any key not followed by a submission holds the bell as a draft.
Counting keys cannot tell a draft from an erased one: Claude Code folds a paste into `[Pasted text #1 +11 lines]`, and one Backspace removes it whole.
So the screen decides, and only when it says the composer is empty.

When the doorbell's verdict for a pending letter is `draft` and the pane is at rest (Herdr reports it `idle` or `done` and current, with no key and no change of status for the 30-second quiet period), the doorbell asks Herdr once for the pane's visible screen with its styling (`pane.read` with source `visible`, format `ansi`, `wire::composer_screen_params`).
The read runs on the doorbell's own thread, off `Mutex<Runtime>` and never on the session-sync thread, at most once per draft episode (the input time that made the draft, `delivery/composer.rs`), and counts against the doorbell's eight Herdr calls a pass; a cut screen is a failed read.
The screen is judged by the agent's composer declaration (below).
Only a positive `empty` releases the draft, as [Deliver now](#delivering-a-held-letter-now) does: the input up to the read is no draft (`released_input_at_unix_ms`), the composer mark goes, and the doorbell's next pass rings with every guard it always applies.
A key or a change of status between the read and the release keeps the hold (`composer.release_refused`).
`not_empty`, `not_found`, a failed or timed-out read, an agent with no declaration and a version not checked keep today's hold, with the waiting mark after two minutes.
Each read is logged as `composer.read` with the pane, agent, version and verdict, and each draft not read as `composer.not_read` with its reason (`kind_undeclared`, `version_unknown`, `version_unchecked`, `version_failed`) once per draft; no record carries screen text.

Each agent's adapter declares where its composer sits and what an empty one shows (`hide_agent_adapter::composer`, the `composer` field of each row in `declarations.rs`), and one parser judges every agent.
Empty is a positive signature only: nothing at all, or the agent's known placeholder in its known style; never "anything faint", since Grok draws a paste in grey.
A screen with no composer where the declaration puts it is `not_found`.

| Agent | Composer | Empty |
| --- | --- | --- |
| Claude Code | the `❯` line and any lines under it, between the last two `─` rules | nothing, or only text in the dim attribute (SGR 2), since its suggestions such as `Try "…"` change |
| Codex | the `›` line and its indented lines down to a blank line | only the dim `Ask Codex to do anything` |
| Grok | the `❯` line inside the last `╭`/`╰` box, between its `│` borders; `>` for `❯` on a console Grok takes for Windows' legacy one, as a Herdr pane on Windows is | nothing, or only italic text (SGR 3): the next prompt it predicts after a turn, whose words change; typed text and the grey `[Pasted: …]` chip are never italic |
| OpenCode | the lines inside the `┃` bar above its `╹` foot, less the agent and model line | only the (128,128,128) `Ask anything…` placeholder; typed text is (238,238,238) and a paste an orange chip |
| Pi | the lines between the last two `─` rules | the rules adjacent |
| omp | the `╰─` line | nothing, or the `⇧⇥ to change thinking effort` hint; a paste shows `📄 #1` |
| Antigravity | the `>` line between the last two `─` rules | nothing after the marker |

The declarations were measured on 2026-10-21 under Herdr 0.9.5 at 60 columns (Claude Code 2.1.306, Codex 0.163.0, Grok 1.0.62, OpenCode 2.0.29, Pi 1.0.4, omp 18.7.0, Antigravity 1.4.4): type `hello 안녕하세요`, Backspace it all, paste 12 lines, Backspace twice, clear; `hide-agent-adapter/tests/it/composer.rs` judges those captures.
Grok's predicted next prompt and its Windows marker were measured on 2026-10-22 with Grok 1.0.52, the pinned version, after a turn ended (the earlier captures had none), on its own and with `GROK_FORCE_LEGACY_CONSOLE=1`, the console a Windows pane gets; its source draws the suggestion with `ghost_text_style`, dim and italic, on every system.
Only Claude Code and Codex are rung, so only their drafts are released today; the other declarations serve the agent journey's `composer` cell and a bell target added later.
Antigravity's declaration also clears the composer of an agy fork's pane before Hide types `/fork` there ([UI_BEHAVIOR.md](UI_BEHAVIOR.md), Antigravity's Fork): it is read for any agy version, since a screen it cannot confirm empty is typed into as before and logged as `fork.composer_unconfirmed`, so a declaration that no longer matches agy never stops a fork.
[INSTALL.md](INSTALL.md#what-each-system-supports) records what has not been measured (narrow panes, the light theme, Windows and Linux).

#### Catching an agent's screen change

An agent update can move or restyle its composer under the declaration, so a release is allowed only for an agent version whose declaration was seen to work.
Hide knows a version from the hook diagnosis, which reads the installed runtime's `--version` on the core's machine (`hide_agent_hooks::runtime_compatibility`, refreshed with the kit); a pane on a connected device, or an agent with no hook runtime, has no version Hide knows, and its drafts hold as before (`version_unknown`).
The first time Hide sees a version, at a moment the composer is known empty (a turn that a prompt hook of the pane's own session started has ended, and no key came since), the doorbell reads the screen once and judges it, at most one pane every five seconds.
Herdr can report the turn `working` before its hook reaches the core; a hook within ten seconds of that, with no key since the turn began, still marks the turn, while a hook after a key typed during the work is a queued prompt and marks nothing, since the composer may hold a newer draft.
`empty` passes the version; anything else fails it, and that version's drafts hold as before; a failed read decides nothing and waits for the next such moment.
Each check is logged as `composer.version_checked` with the agent, version and verdict, and kept in `composer-checks.json` in the state directory (`hide_kit::layout::composer_checks`, written by `composer_checks.rs` on a thread of its own), at most 128 versions with the oldest dropped first; a missing, damaged or other-version file is ignored with a `composer_checks.discarded` diagnostic, and every version is checked again.
The version is the installed program's, so a pane still running the program from before an update is judged by the newer version's check until it restarts.
The agent journey's `composer` cell checks every declaration nightly on each system lane ([VERIFICATION.md](VERIFICATION.md#agent-journey)).

### Delivering a held letter now

A draft is the one hold that does not end on its own: an idle pane whose composer may hold the operator's half-written prompt keeps its letters until its next real prompt, for up to the 60-minute deadline, unless its composer is read empty first ([Reading the composer](#reading-the-composer)).
So the core shows it, and nothing else about a hold reaches the screen.
A pane's letters are shown waiting when the doorbell's last reason for a pending letter there is `draft`, the pane still holds that draft, and the pane has rested for two minutes: no input hide routed and no change of Herdr status since (`doorbell::letters_waiting`, `WAITING_MARK_REST_MS`).
Every other hold (working, the quiet period, a launch, a menu) ends on its own and shows nothing.
The count is every letter held there, which the bell turn brings together, and a pane whose held letters have all spent their three bells shows nothing, since pressing could ring none.
The core decides on each doorbell pass, under the short lock the pass already takes to publish its holds, and keeps the answer on the pane's agent row (`letters_waiting`), so the mark costs one publish when it appears and one when it goes, never one per pass; each change logs `letter_mark.shown` with the pane and count, or `letter_mark.cleared`.
A key typed into the pane restarts the two minutes, so the mark leaves while the operator works there.

Deliver now is one event, `deliver_letters_now {pane_id}`.
The press is the operator saying the composer holds nothing of theirs: the pane's observation records the input it releases (`released_input_at_unix_ms`), so input up to it is no draft, the pane's composer mark is removed, and the mark leaves at once.
The doorbell's next pass then rings with every guard it always applies (Herdr's readiness, the reservation, the last memory check) and types the same line ([The bell line](#the-bell-line)); the prompt hook takes the letters as it does for any bell turn.
Nothing warns about a possible draft, because the mark sits right above the composer: a draft that is there is sent with the bell line after it.
Input after the press is a draft again and holds the letter as before.
A press the mark no longer stands for changes nothing and logs `deliver_now.refused` with `no_letter_waiting` (the doorbell has not shown it, or it left), `pane_unavailable`, or `pane_not_at_rest` (a key reached the pane or its status changed since the last pass); an accepted press logs `deliver_now.released` with the pane and count.
Neither reaches the screen, and nothing changes on the sender's side.
[UI_BEHAVIOR.md](UI_BEHAVIOR.md#agent-panes-and-the-agents-explorer) owns the pane header pill and the sidebar row's envelope that draw it.

### A menu Herdr reads as a stop

Codex's plan approval, "Implement this plan?", reads to Herdr as an ordinary stop rather than `blocked`, and a bell's Enter there picks its first item, "Yes, implement this plan", so a letter would start a plan the operator never approved (measured 2026-10-07 on Herdr 0.9.3, three runs of three, with the menu read as `done`).
What Herdr reads depends on its Codex detection manifest, and the status guard no longer holds it: the one bundled with Herdr 0.9.5 (2026.10.11.1, which the e2e fixture runs with updates off and Herdr's manifest updates have served since 2026-10-11) reads a Codex title without the working spinner as a stop when no blocking or working rule matches the screen (`osc_title_idle`), and 0.9.5 reads a Codex screen no rule matches as a stop too.
The one bundled with 0.9.3 (2026.09.23.1) read the menu `unknown`, which the status guard held (measured 2026-10-07 with Codex CLI 0.160.1 on an isolated server).
Hide reads that wait from the Codex session file instead of the screen (PRD codex-plan-approval-hold).
The label worker's session read, which already runs on each change of the pane's Herdr state, on its own thread and from where it last stopped, folds Codex's turn records into an agent-neutral turn tracker (`hide-session/src/turns.rs`): `task_started` with its `collaboration_mode_kind`, the `Plan` item a plan-mode turn proposes (or its `<proposed_plan>` reply), `task_complete`, `turn_aborted`, and each person's message.
A plan-mode turn that proposed a plan and finished, with no later turn or message from a person, waits for the operator's approval; the next `task_started` or a person's message ends the wait.
Records the parser does not recognise are not known rather than "nothing waits": a `task_started` whose mode is missing or not one it knows, a plan proposed in a turn whose mode says it runs none, and a person's messages with no turn record at all.
The rule and its basis (codex-cli 0.160.1) are written beside the Codex parser, and nothing outside it names Codex.
The answer is kept with the Herdr `state_change_seq` the read was asked under, and it holds for that state only.
Herdr can read the agent at rest before Codex writes how the turn ended; that state's read then does not settle a plan-mode turn, so the bell holds and the row shows no wait until Herdr's next state is read, because this rule adds no session reads of its own (PRD codex-plan-approval-hold B10).
The same turn tracker reads Claude Code's native `AskUserQuestion` calls and matching tool-result records.
When a supported session reader says a native question or plan approval waits, the bell holds with `awaiting_operator`.
This includes Codex 0.162.1's `request_user_input_async` question, which stays open after Codex returns to work or stops, until its reply arrives.
The hold is what keeps a wake from landing on it: in an isolated Codex 0.162.1 TUI (2026-10-15) the question is plain assistant text over an ordinary composer, and a bell-shaped line plus Enter was submitted as an ordinary prompt that started a new turn, writing no reply, so the question stayed unanswered in Codex while Hide would have read the person's message as the end of the wait.
For an agent whose read reports turns, a state that no read has settled holds the bell too, with `session_unread`: a session file not found (Codex is looked up in its seven newest day folders), a failed read, a device helper that predates the field, a read still behind Herdr's newest state, a record past the line cap the read could not structure or more native questions waiting at once than the read keeps (each until the next turn starts or a person writes; `docs/ARCHITECTURE.md`, What the last turn waits for), or a daemon that does not hold the label generator lock and so reads nothing.
That lock is one per Herdr server and taken with `flock` (`herdr-core/src/labels/generator.rs`): the operator's single daemon lacks it only while another hided follows the same Herdr server (a candidate or development daemon pointed at the operator's socket, even with a private HOME), or, after a restart that overlapped the old daemon, until its next attempt, at most thirty seconds after the old one exits; the kernel frees a crashed holder's lock.
Unknown is never read as "nothing waits".
Kinds whose session readers report no turns retain the other delivery guards; this does not enable a new bell target.
"No, stay in Plan mode" writes nothing to the session (measured with Codex CLI 0.160.1), so the wait, the hold and the row in Needs You last until Codex's next turn starts or a person's message is written.
The first start of a build with this rule resumes a stored session at its last person's message, after that turn's `task_started`, so a plan already waiting then is not known until Herdr's next state; that start holds the bell rather than ringing it.
The rule reads the session Herdr reports for the pane, so a pane reported with another session is judged by that session.
It guards against hide's own bell, not against a process of the same account, which could append records to the session file or type into the pane through Herdr directly.

## Persistence, clocks and limits

The mailbox lives at the state directory's `delivery-ledger.json`, named by `hide_kit::layout::delivery_ledger`.
An admitted mutation is atomically persisted as a private file before the daemon publishes it or returns success.
The store writes the file only when a batch of up to 32 admitted requests changed what it holds, and an unchanged pass, such as a watch tick that moved nothing, writes nothing.
A batch keeps one copy of the ledger and applies its requests to it in order; a refused request leaves the copy as it was before it, because the copy is made again from the published ledger with the requests accepted so far, which change nothing but the ledger they are given.
Each request is checked against the file's limits as it lands, by counting the ledger's encoding without keeping it, because a later request that drops a letter could bring the batch's end back under a limit an earlier one broke; a request that breaks one is refused like any other, so the batch lands as its requests would one at a time, and the bytes written are encoded once, at the batch's end.
Every reply is the one the published copy gave its request: when a refused request makes the copy be made again, each earlier request is answered from that copy, so a sender is answered the letter id the file keeps.
Startup establishes the existing trusted ancestor's directory barriers, creates and syncs at most 64 private descendants, validates the actual ledger and reestablishes its file barrier before admitting effects.
An uncertain replacement disables further delivery effects until a validated restart; corrupt bytes and incomplete directory chains remain available for recovery.
The ledger refuses a state folder that is a link or that other accounts can read; the daemon makes its own state folder private before the core starts ([ARCHITECTURE.md](ARCHITECTURE.md#hided-and-the-websocket-boundary)), so a folder an earlier build left open, as a device helper's bridge folder did on a machine a core later moved to, still loads its letters.
These barriers are the platform layer's on every system ([ARCHITECTURE.md](ARCHITECTURE.md#the-platform-layer)): on Windows each folder is flushed through a handle allowed to add an entry to it, a folder above the state folder is flushed only to keep a folder the account made in it, so `C:\` (whose `Users` the system made) and, for a standard account, `C:\Users` (which it may not flush) are passed over, and a flush that fails anywhere else leaves the ledger unavailable rather than ready without its barrier.
A corrupt, oversized or unsafe existing ledger is preserved, and commands return `ledger_unavailable` with a diagnostic instead of starting an empty ledger.
Capacity errors retain existing letters and watches.
Finished letters never refuse a new one: the one-day cleanup is the normal way a finished letter leaves, and a full total is the backstop, so a mailbox that finishes more than 5000 letters in a day keeps its newest finished letters and loses the oldest ahead of the day.
The store runs the cleanup on its own once a second when a finished letter is over a day old, including its first pass after start, so a ledger that was full of older finished letters shrinks without a new letter.
A dropped letter leaves no reference that must resolve: a reply's `reply_to` and an agent's escalation read the ledger as it is and treat a missing original as absent, as they already do after the one-day cleanup; the machine clocks follow only letters awaiting intake, which are open; and the same intent sent again after the drop is a new letter.
Every `capacity` refusal of a letter writes one `letter.capacity_refused` diagnostic with `cap` (`body`, `open`, or `total` when all 5000 are open), both panes, the body size, the letter count and the open count, never the body.

| Resource or clock | Bound |
| --- | --- |
| Pending delivery deadline | 60 minutes of the recipient's machine being reachable (wall time on the core's own machine), then `undelivered`; visible through CLI |
| Quiet period | 30 seconds since hide last routed a key and since Herdr last changed the pane's status |
| Doorbell reservations per letter | Three total, persisted across restart |
| First inactivity warning | 20 minutes without activity (30 for a code-owned observer); one minute (one tick) when the target stopped on a question or is held at a Herdr-blocked prompt and the observer is a pane |
| Second inactivity warning | First-warning time plus 60 minutes, at most two warnings per episode |
| Unanswered parent warning notification | First-warning time plus 60 minutes, once per native target and inactivity episode |
| Answer wait of a `request` or `block` | 24 hours from the send, then `answer_wait_ended: "deadline"`; an end of either registration ends it at once as `party_ended` for a letter the recipient took in |
| Intent retention and finished-letter cleanup | One day from the finish; open letters remain; a full 5000-letter total drops sooner (next row) |
| Open / retained letters | 1024 / 5000; a letter is open while it awaits intake or a reply; at 5000 a new letter drops the finished letter that finished first (ties: the earlier sent), never an open one, and is refused with `capacity` only when none is finished |
| Watches | 32 |
| Letter body / ledger file | 16 KiB / 16 MiB |
| Hook batch / context / total deadline | Five letters / 8 KiB / two seconds |

Automatic doorbells apply only to `pending` letters, and the deadline turns only a `pending` letter `undelivered`, while watch clocks remain distinct.

The deadline of a letter to an agent on another machine counts only the time that machine is reachable (PRD core-host-node-move B13, D-11): the core holds that machine's Herdr connection and its link has not ended, so a MacBook that is closed, asleep or away holds its letters at the core and they are handed over when it is back, with the hours away not counted.
The ledger keeps one clock per machine a letter awaits intake on (`node_clocks`, keyed by its registration id: the reachable time in ended stretches and the start of the one running now), and each letter records where on its machine's clock its wait began (`intake_from`); a letter to the core's own machine, and one written before this rule, waits in wall time from its creation as before.
The store reads which machines are reachable under the runtime lock on each pass (`Runtime::delivery_reach`); a node's link end is read there at once, before the sync that notices its Herdr is gone.
A stretch is written into its clock every minute while a letter waits on it, so a core that stops loses at most that minute of it, and a core reading the ledger at its start counts no time between the last write and its own start.
A machine gets a clock when a letter waits on it, and loses it once none does, it leaves, or a move makes it the core's own; each of these keeps every letter's wait, so a letter that waited 50 minutes when the core moved turns `undelivered` after 10 more, never resetting and never losing what it waited (`Ledger::observe_reach`; the move renames the clocks with their machines, `node_migration::KEYS`).
The open-letter limit bounds the letters held this way, and with them the clocks.
A bell reserved and then lost with its machine's link spends one of the letter's three attempts, as any bell that failed after its reservation does, because a failed call may still have typed it; the letter is still handed over at the agent's next turn.
A letter an earlier build acknowledged without a receipt stops awaiting intake at the same 60-minute deadline, on the store's first pass after it (within a second of startup for an existing backlog): the hook no longer hands it over, its receipt reads `null` like the legacy acknowledged records below, and it no longer counts against the open-letter limit unless it still awaits a reply.
Legacy records with missing or null `hook_confirmed` prove intake only in `delivered` state; older `acknowledged` records remain unknown, excluded from pull and subject to their previous closed-state retention rules unless still awaiting a reply.
The 24-hour answer deadline is judged in the store's same maintenance pass, which runs when some letter or registration needs it: a wait past its deadline counts as such work, so no new thread or timer exists.
A registration ending leaves a letter still awaiting intake alone: it follows the 60-minute delivery deadline above, in wall time from then on, and only a letter the recipient took in has its answer wait ended.
The first pass of a build that has this rule closes every wait older than 24 hours in the ledger it finds and leaves the younger ones; the ledger stays at version 1, and an older build ignores the new field and drops it at its next save.
Each closed wait logs one `delivery` diagnostic `answer_wait.ended` with the letter id, both agents' names and panes and the reason, never the body, and nothing reaches the screen.
There is no transition to `expired` in this contract; an undelivered letter uses the existing human notification paths without creating another letter or UI banner.
First-warning time and count persist across daemon restarts; activity resets both.

## Watch activity and completion

The registered observer receives inactivity warning letters; `hide watch assign` changes that observer without a polling interval flag.
The current observer can assign a new observer; an attested new observer can accept a handover with explicit `--approval <text>` and the current `--expected-generation <n>`.
Approval is nonblank text of at most 256 bytes without control characters, and does not replace the command's native caller binding.
For that acceptance, the requested observer must be the actual caller; `--actor` cannot select a different caller, and a missing or stale generation refuses the handover.
Activity is the later of Herdr's status-transition time and the confirmed native session file's modification time.
A local read and the device helper's `session_activity` use the same session ownership and root-confinement checks.
Session lookup has one total budget of 10000 directory entries, including skipped extensions and unmatched names, 64 visited directories and 8 MiB of retained path bytes.
Crossing a limit returns the path-free `session_capacity` outcome; the watch uses its existing status fallback and records the failure.
Below those limits, reported identity and provider ownership take precedence over cwd fallback, and each tick samples fresh metadata.
The helper returns only modification time and file size; conversation text, paths and native IDs do not appear in the activity answer.
The helper acquires the platform's `OwnerWatch` before reading arguments, stdin or files and retains it through shutdown.
A protected launch ends with its owner even while stdin stays open; a standalone launch without ownership metadata keeps its existing contract.
A missing reference or failed helper read falls back to the current status-transition evidence, includes the reason in the digest and does not suppress warnings.
That status must still be attributable to the watch's original native binding.
An uncertain acquisition or loss of native identity preserves the original watch and clocks without rebinding or accepting new file metadata.
Its digest reports `projection_unavailable` and an unavailable current status; two missing references do not prove an unchanged execution.
Proven original-target absence or a different positive native identity ends the watch.
So does a deleted folder: when the target cannot be read, the tick asks the target's machine whether the folder it runs in still exists (`Call::PathFacts`), or, for a target a sleeping or archived session holds, the folder that session would open again, and a machine that answers that it does not ends the watch and logs `watch.target_folder_gone`.
Such a target can neither be read nor opened there again, so its watch would only fail every tick; a machine that does not answer proves nothing, and the watch stays.
A target that is read, and one away with no session holding it, costs no extra call.
There is no cached response or retry during the same tick.
Three consecutive read failures produce a rate-limited diagnostic.
The ledger counts failures only up to those three (`watch::READ_FAILURES_REPORTED`): a read that keeps failing is still reported every ten minutes, and changes nothing the file holds, where counting every tick rewrote the whole file every minute while one watched target could not be read.

A target that stopped on a question it asked only in its own pane, or that Herdr holds at a blocked prompt, is told to a pane observer after one tick, 60 seconds of no activity, instead of 20 minutes, so the parent decides within about a minute whether to answer the child or ask the operator.
The fact is the target's own row while its pane runs the watched session: for a question, activity stopped, a question demand ([status-model.md](status-model.md#where-each-axis-comes-from)) and no blocked prompt; for a prompt, the row's `blocked`, whatever its activity, since Herdr reports no activity for a pane it holds at a menu.
A blocked child also rises to the operator ([Delegated escalation](status-model.md#delegated-escalation)); the parent is told as well because a native runtime can time the prompt out on its own (Claude Code's built-in `rm` check gives up after two minutes, 2026-10-16), and a parent that hears in a minute can still tell the operator or send the child a safer command.
A child that works, such as Codex while its async question stays open, keeps the 20 minutes.
A question the target already sent this observer as a `request` or `block` letter still awaiting its answer keeps the 20 minutes too, since that letter rang the observer's doorbell; a blocked prompt rang nothing, so it is told whatever letters wait.
The early warning is the episode's first warning, and only its first line differs: `Stopped with a question (watch <id>).` or `Blocked at a prompt (watch <id>).` instead of `No activity for N minutes`.
The second warning, the unanswered-parent human notice and the escalation to the operator follow from its time exactly as from a 20-minute warning, and new activity of the target starts a new episode, so one question gives at most one early warning.
The runtime reads the row it already holds when it hands each watch to the tick, under the lock that builds the tick's work; nothing is added to the tick's reads, threads or timers.

The minute tick admits at most one read per watched target, and the folder question after a failed one, at most four targets at once, each call with a five-second deadline.
Different parents watching one exact target share that tick's activity sample while keeping independent warning clocks and intent keys.
A warning rejected by capacity leaves other targets' exits and activity resets intact.
Session discovery, ownership reads and helper I/O run outside `Mutex<Runtime>`.
A tick also performs one bounded ledger encoding and bounded per-watch admission checks; it publishes only watch start, warning and end transitions.
A target exit or parent's explicit stop ends the watch.
A normal reply closes the request's answer wait and leaves the watch active.
A done target remains watched until exit, explicit stop or a completion report.
The first receipt, an acknowledgement or the actual post-flush confirmation, records `hook_confirmed: true` and ends a matching sender-parent watch for `hide request send --kind report`; a confirmation still does after cancellation or the delivery deadline, and replay after restart preserves a watch explicitly started after that receipt.
A `progress` letter is received the same way and ends no watch: it is the interim report, such as a plan, and the child is still at work after it.
A report from an unwatched sender is an ordinary letter; the parent can restart a watch explicitly.

## Code-owned identities

The `factory:` name prefix remains reserved for compatibility with historical registrations.
No Factory recipient is active in this release.

Hide itself is a code-owned identity, `hide:<role>` of kind `hide`: `hide:memory`, the origin of the agent a Memory card's promotion starts ([ARCHITECTURE.md: Project sessions and Memory](ARCHITECTURE.md#project-sessions-and-memory)), and `hide:conductor`, the sender of a mission's wakes ([Missions](#missions)).
`hide:memory` is registered on its first spawn, sends nothing and receives nothing; `hide:conductor` sends progress letters to a mission's Conductor and receives nothing; and a pane or agent name that starts with `hide:` is refused at registration like `factory:`.
A spawn is Hide AI's own when it is a handoff whose origin is that identity (`coordination::started::hide_started`): a pane agent's handoff has a pane's identity as its origin, so neither counts, and no branch or worktree name ever decides it.
Only that identity may give a spawn a `task` (`SpawnTask`: a `kind` code such as `memory_promote` and a `target` such as the card `m72`, the codes the screen words), on a handoff; any other caller that sets one is refused `task_reserved`, and the task is part of the intent, so the same intent with another task is an `intent_conflict`.
A receipt from before the task existed has none, and its agent and checkout are still Hide AI's, which the screen names `Hide AI` alone.
The kind is a closed set in code (`TaskKind`), and each known kind names its home, the screen that shows its work (`TaskKind::home`: `memory_promote` and `suggestion_apply` to the Hide AI tool's 제안 tab); a new kind cannot be added without naming one.
A kind this build does not know loads from the ledger as it was written, so a record a newer build wrote never fails the load or loses the rest of the ledger, and its agent stays an operator's row.
A new spawn of a kind with no home is refused `task_kind_unknown`, and the first time an agent of such a kind is seen the diagnostic log records `hide_ai.task_kind_unknown` once.


## The operator's View feedback

The operator is a third code-owned identity, `operator` of kind `operator` (`Actor::operator`), the sender of the letter a View feedback Send makes ([UI_BEHAVIOR.md](UI_BEHAVIOR.md#view-openers-and-comments), PRD view-opener-feedback D-17).
Only the core constructs it, and only for an operator's screen's `view_feedback_send`, which hided accepts from a desktop or web window whose connection its token proved; no pane's scoped connection, `hide` command or page script can represent that event, and a pane or agent name that is `operator` or starts with `operator:` is refused at registration like `factory:` and `hide:`.
It has no pane and reads no letters; its letter is a `report`, so it waits for no answer, and its first line reads `Hide letter <id> from operator (operator) [report]` (`contracts/delivery-envelope.json`), which `hide_session::envelope_from_operator` recognizes, so the agent reads it as the operator's own prompt and starts working.
The letter rings the recipient's doorbell and waits for rest exactly as any letter does.
Its body lists each comment in its recipient's order: a page's URL, View id, selector, element text, source location when known and screenshot path; a file's or diff's path, line range, quoted lines and note; an image's or PDF's path, page, area and screenshot path.
What a page wrote (its selector, element text and source attribute) is kept to one line of plain words, and a path, address or quoted line has its newlines and control characters as spaces, so nothing but the operator's own note can start a line in the operator's letter.
A body over the 16 KiB letter cap is written whole to an attachment file and the letter carries a summary and its path.
Screenshots and that file are attachments: they travel over the recipient device's existing link (node protocol 33's `feedback` call), land in an owner-only folder per letter under the recipient machine's `letter-attachments`, and are removed with the letter; hided sweeps orphans at start, and one machine's attachments are capped at 256 MiB, past which the Send is refused with its reason.
A Send to a device whose link is down waits like a letter and says so on the View's line.
The comments leave their View only once the letter is stored; the ledger's outcome decides the rest: delivered drops them, expired returns them marked `Not delivered`.
A Send names its own request id, and Retry names only that id (`to` `retry`): the core sends again to the agent and Views the Send asked for, so a retried Send converges on the same letter, and a moving core holds it behind its effect hold ([ARCHITECTURE.md](ARCHITECTURE.md#moving-the-core)).

## Names on the wire and on screen

An agent has two kinds of name, and each is made in one place.

- The wire name is what a program reads and compares: the envelope header, the `name` of a letter's or a watch's sender, recipient, target and observer, an agent's `name` in `hide agent show` and `list`, and the `parent_name` the label worker matches a parent's request against.
  It is the name the agent registered under, else its registration id (`agent-12`), which is stable and which `hide request send` and `--parent` already resolve; an agent that never registered is called by the only identifier Herdr gave it, its Herdr agent name or else its pane id.
  `coordination::wire_name` makes it, the writing side (`mailbox::send`, `watch::start`) and the reading side (`link_parent_facts`) both call it, and nothing else may spell it, because the label worker accepts a parent's request as the child's work only when the two strings are equal ([status-model.md](status-model.md#task-identity)).
  It never follows a screen title, which changes every turn.
- The screen name is what a person reads: the name it registered under, else the title Hide's screens give it while its pane hosts it, else its runtime's name (the bell line, the sidebar and Sessions rows).
  It is derived when it is drawn and stored nowhere.

A registration stores a name only when it has one of its own.
A parent that `hide agent spawn --parent here` registers for the caller takes the name Herdr has for the pane, and none when Herdr has none, so a pane id is never stored as a name (`AgentRecord::name` is optional, and an executor-made record drops a name that only restates its pane).
On disk the field stays a string on every record, so a build before this one reads the ledger this one writes: an unnamed record is written under its registration id, and reading a name equal to the record's own id or to its pane as no name turns that back (`AgentRecordOnDisk`).
A ledger written by an earlier build can hold a pane id as a name; reading it that way gives such a record no name, with no migration file, and a code-owned identity keeps its name, which is also its pane.
A letter keeps the endpoint names it was written with, so a letter written before this rule keeps a pane id in its history until its one-day retention ends.
`hide inbox` and a letter's envelope show the registration id of an unnamed sender, not its title, and show the pane id of a sender that never registered and that Herdr never named.

## Agent registration and spawning

`hide agent register [--check]`, `list`, `show` and `end` preserve the caller surface used by dispatch and Fork.
`register` records the participant on the caller's own machine, which the pane capability names: `--machine` may be left out, and when given it must name that same machine or the call is refused with `machine_identity_conflict`, so a caller in a connected device's pane registers as that device.
`hide agent show here` answers the caller's own registration, read only, with no renderer: the one live record whose actor is the caller's attested pane, device and session, the same match as `--parent here` (`coordination::live_self`), refused when two match (`coordination::here`).
Only a pane-bound credential can ask, as for every delivery and agent command: a checkout-bound one is refused `agent_pane_required`, and a pane-bound one whose hint names another pane is refused `caller_identity_conflict`.
A registration keeps its pane's agent session as Herdr reports it: an id, or for Pi and omp the session file's path, which lies in a folder named after the checkout's path and so passes 256 bytes in a deep checkout.
The ledger keeps a session of up to 4096 bytes, Herdr's own bound for a session path (`SESSION_LIMIT`, the bound a letter pull takes), and finds a pane's registration by the session's SHA-256, so its length never reaches a key.
A longer session is refused `session_too_long` when the record is made, never as an unavailable ledger: `hide agent register` refuses it before it reaches the daemon, as the daemon does again before it asks Herdr, and a spawn whose agent reports one closes the pane it made before it answers, with the next action to spawn with a new intent in a checkout with a shorter path.
An empty session or one holding a control character is refused `invalid_registration` by the same rule (`coordination::kept_session`).
A build before this one reads a session past 256 bytes as a damaged ledger, so going back to it after such a registration answers every delivery and agent command `ledger_unavailable` until this build or a later one runs again.
A caller with no such record is refused `participant_ended` when its record ended, `participant_session_changed` when its pane's live record belongs to another session (the pane's agent session changed), `ambiguous_participant` when two live records match, and `participant_unavailable` otherwise; a remote participant on a pane of the same name is never the local caller.
`hide agent spawn` requires `--name`, `--intent`, `--kind`, `--repo` and `--branch`, with optional `--parent`, `--machine`, `--beside`, `--path`, `--model`, `--effort`, `--brief <text>` or `--brief-file <path>`, and native arguments after `--`.
The brief is the child's first prompt: once Herdr reports the started agent ready for input (up to 30 s, `BRIEF_READY_WAIT`), the spawn submits it through `agent.prompt`, the one submission path the doorbell uses too, and only then waits for the child's session, so an agent that reports its session at its first turn (Codex, OpenCode 2) reports it on that turn and binds at once instead of waiting.
When the child binds, the same text is recorded as a letter under the spawn's intent, kind `request`, delivered and hook-confirmed, so `hide inbox`, the ledger and a later wake carry what the child was started on, and a letter the parent sends under that intent afterwards is this one.
The brief is part of the intent like `args` (another brief under the same intent is `intent_conflict`), the ledger marks it submitted (`briefed`) so the same command again never submits it twice, a blank brief or one past a letter's body limit (16 KiB) is `invalid_brief` before anything is made, and an agent that takes no input within the wait, or a submission Herdr refuses, is `brief_not_submitted` with nothing typed, where the same command submits it on the next try without starting the agent again.
The CLI reads `--brief-file` itself, in the caller's account, and refuses a file it cannot read, a blank one or one past the limit before the daemon is asked; `--brief` and `--brief-file` together are refused the same way.
`--model` and `--effort` are the agent's launch choice: the start passes them in the kind's own spelling, and every wake, fork and reopen Hide makes of the agent starts it with them again (`docs/ARCHITECTURE.md`, Agent start), while native arguments after `--` apply to this start only.
A choice the kind does not take is refused `invalid_launch_choice` before anything is created, and the choice is part of the intent, so the same intent with another choice is an `intent_conflict`.
Choose responsibility when creating the agent; it cannot be transferred afterwards.
With `--parent here` or the caller's own id, the agent is delegated: its parent owns the work, lineage is written immediately and an automatic watch starts.
Without `--parent`, the work is handed off to the operator: the agent is an independent root, has no lineage edge to the spawner and starts no automatic watch.
Both modes create a separate tab, in an existing checkout or a new worktree, without changing the current screen or keyboard focus.
With `--beside` the child opens in the caller's own tab instead: its pane is split right of the caller's (`pane.split`, `focus: false`, the parameters Fork agent to the side uses), drawn at once on this machine's screen and replaced by Herdr's layout when it arrives (a runtime that cannot draw it logs `beside.draw_skipped` and the spawn goes on), and the rest of the start (the agent, its brief, its session and the child record) is the tab path's.
Placement is independent of responsibility, so `--parent here --beside` makes a supervised child beside its parent and `--beside` alone an operator-owned root there; the spawn record keeps `beside`, and delegated-child relocation leaves such a child in its caller's tab across restarts ([ARCHITECTURE.md](ARCHITECTURE.md)).
It is only for the caller's own checkout on the caller's own device, and everything else is refused before anything is registered, reserved or created, with the next action to run the command again without `--beside`: `beside_other_checkout` when `--repo` and `--branch` (and `--path`) name no existing worktree or another one than the caller's, so a new worktree or another branch never opens beside; `beside_other_machine` when `--machine` names another device, whose Herdr cannot hold a pane in the caller's tab; and `beside_needs_agent_pane` for a code-owned caller with no pane of its own (a shell that is no agent pane is refused earlier, as every agent command is, with `agent_pane_required`); a caller's worktree that is gone by the time the pane is made is `beside_other_checkout` too, since a spawn beside never makes a worktree.
An agent on a connected device that spawns with `--beside` and no `--machine` gets its child beside it on that device's screen.
The split is labelled with a reserved pane label (`hide-spawn:v1:<spawn id>`, never shown as a name) before anything else, so the same command after a reply was lost finds that pane and makes no second one, and the label is cleared once the spawn record names the pane; a retry of a spawn whose pane is recorded, or that completed, is not asked about the checkout again; a split Herdr refuses, or a caller pane that is gone, fails the spawn with `beside_pane_unavailable`, opens no tab in its place, and logs `spawn` `beside.split_failed` with the spawn and pane ids; a split that cannot be labelled is closed.
A split whose own reply is lost before the label is written leaves a bare shell pane beside the caller that a retry does not find; the retry makes its own pane.
`--beside` is part of the intent, so the same intent with or without it is `intent_conflict`.
The no-focus request applies to opening the checkout's owner workspace as well as creating its tab; interactive workspace and tab creation keep their focus behavior.
`hide agent spawn --help` explains the modes without contacting a daemon; the removed `--no-watch` flag is refused before any effects.
Every other `hide` command ending in `--help` or `-h` prints that command's usage to stdout and exits 0; a wrong command prints one line that says what is wrong (`hide device needs a subcommand`, `unknown flag for hide ps: --x`, `--body is required and cannot be blank`), then that command's usage under `usage:`.
A command that could not reach the core prints the same next step for a code whichever topic ran it (`request_next_action`), and its stderr line stays the bare code, which the spawn guard reads.

Every `AgentView` includes `origin`, the spawner id for either mode and null for ordinary roots.
Delegation derives origin from parent; handoff stores origin separately, and it grants no parent, subtree-close or end authority.
A handed-off agent's identical self-registration or `--check` preserves its id and stored origin, without taking an origin input or changing responsibility.
A handed-off agent can end its own registration; its spawner cannot end it and receives `parent_authority_required`.
Ordinary letters and explicit watches remain available between independent agents.
The sidebar, the Overview's tasks, ancestor unread state, descendant marks and waiting-on-descendants rule read only responsibility through parent.
PR and issue panels retain the handed-off session through its own branch facts and show no delegation line from origin.

A completed spawn stores a durable receipt for its caller and intent, so retries return the same agent and preserve ended registrations and closed watches.
Changing only responsibility mode on the same intent returns `intent_conflict` before creating anything.
Only incomplete intents resume their recorded creation and registration steps; starting a new watch after completion requires explicit `hide watch start`.
Remote starts through a caller's own device use Hide's existing device start path with the same caller, modes and focus rules.

Spawn creation keeps its recovery marker until the receipt contains the checkout, pane and terminal incarnation.
The created tab then takes the registered task name; a created workspace takes the checkout branch, or the repository name for its primary checkout.
The receipt reports separate workspace and tab naming outcomes (`pending`, `applied`, `preserved`, `failed`); a failed rename does not undo the child or its watch.
The existing lineage worker retries unfinished names once per connection, in bounded batches, without a separate poller.
Repair requires the receipt's device, checkout, pane and terminal to match the current unique target and its exact recovery marker.
A newly created workspace's first tab may carry Herdr's generated label when shell replacement is unavailable.
Its creation response's exact tab id and label are saved with the first durable placement, so that unchanged label can be named too; an older receipt cannot infer this proof from a current default label.
User labels and targets with insufficient proof are preserved, and successful targets are not renamed again.
Herdr's rename API has no compare-and-set, so a user rename between the final read and write remains a race.
Completed intent replay returns the durable receipt without contacting Herdr, including while a device is offline.

`--machine <device id>` starts the child on a connected device instead of the caller's own; it works with delegation and handoff alike.
The id is the `device_id` that `hide workspace info` shows and the `machine` that `hide agent list` shows for agents there; a device label is not accepted, and the caller's own id is the same as leaving the flag out, so a spawn without `--machine` is exactly the spawn above.
`--repo` and `--path` are then paths on that device.
A spawn records the machine of its child only when that is not the caller's own, and the ledger refuses a child whose machine is not the spawn's.
A core move therefore reads the field against the caller's machine after the move (`node_migration::reown`): a spawn that names none stays so, a machine the move renames takes its new id, and one that is now the caller's own is dropped.
The copy is loaded with the ledger's own reader before it leaves the machine giving the core and again before it is placed on the machine taking it, so a move that would leave the new core without its letters, watches and registrations is rolled back instead of reported done.
The caller keeps the parent registration, the authority and the watch; the child's checkout (a worktree when the branch has none), tab, agent start, identity wait and lineage tokens are the device's, made through its Herdr API channel, and the tokens name this machine as `parent_machine`.
The agent is registered under the device, so the sidebar and the Overview's tasks show it under its parent with the device's mark, its reports and inactivity warnings reach the parent as letters over the existing device paths, and a handed-off agent records the caller as `origin`.
Before the intent is reserved or anything is made, the spawn is refused with a stable code and a next action when the id is not a device (`machine_unknown`, naming the connected device ids the caller could have used), the device is not connected or its helper cannot answer (`machine_unavailable`), `--repo` holds no repository there (`repository_unavailable`) or the agent CLI is not on the device's `PATH` (`agent_not_installed`); the checks are the node link's `repository` and `agent_installed` calls, taken off the runtime lock.
The device is part of the intent: the same intent on another device, or on none, returns `intent_conflict`, and a spawn that already completed answers from its receipt even while the device is away.
A spawn on a device makes several calls over SSH, so the CLI waits up to 60 seconds for it; one that times out still finishes, and running the same command again returns the same agent (`spawn_busy`, whose next action is to wait and run the same command again, while the first still runs).
The checks of the device only read, so they run before the process-wide spawn lock is taken and a slow device does not make local spawns answer `spawn_busy`.
Creating is single-flight and the checks are not: each of the two calls waits up to 8 seconds, and checks of different spawns may overlap.
When the device's Herdr stops answering after the spawn was reserved, the answer is `machine_unavailable`, the cause goes to the diagnostic log with the device and the spawn, and the same command continues the spawn once the device answers again.
The CLI then answers `request_timeout` and says the request may still be running.
A retry of a spawn that already created its pane needs the device only to be reachable: the repository and agent checks ran in the first attempt and are not repeated.
`--machine` is allowed only from an agent on the machine that runs Hide; an agent on a device that names any other device is refused with `machine_not_permitted`, which lists no device.
A parent other than the caller is refused with `parent_authority_required` before any device is asked.
The receipt keeps the device as an optional field of the version 1 ledger, absent for every older record.
The spawn guard is unchanged: a direct `herdr --machine …` start is still the operator's own responsibility.
The ledger stays at version 1: older records without mode load as delegation, missing origin defaults to null and legacy `no_watch` data is ignored without changing existing watches.
The unsupported reconciliation/resume/session flags and relay, escalate, graph and events commands are absent.

A registration ends when Herdr no longer has its pane, so its name and the watches on it do not outlive the pane; the ended record stays in the ledger, like one `hide agent end` ended, and still counts against the 2048-registration limit.
The core reads that from its own session sync of the host's Herdr, never from a separate poll: a pane the in-sync replica listed and then stops listing was closed by Herdr or moved to another tab, which gives it a new id and already ends the watches on it, and a pane missing from the fresh `session.snapshot` of a connect is gone for every registration made before that snapshot was asked for, which is how registrations left by panes that closed while no Hide was running end on the first connect.
Nothing ends on uncertainty: a stream that lost events, a Herdr live handoff or restart and an unreachable Herdr publish no read until a fresh snapshot replaces it, a disconnected device publishes none, a sleeping or resumed agent keeps its pane, a session asleep with its pane closed keeps its registration and the watches on it until its wake continues them or lets them go, an archived session keeps them the same way until 이어가기 continues them or the archive is deleted, an agent closed with its parent keeps them while its Reopen closed entry is listed, a registration seen continuing in another pane keeps them until the store moves them there, a registration on another Herdr socket of the same machine is not judged by this one's read, and a registration made after the snapshot was asked for waits until a read lists its pane.
The delivery store ends it with the watches on it and the answer waits of the letters it sent or received, as `hide agent end` does, and logs `agent.ended` once with the agent id, machine, pane and reason (`pane_left` or `pane_absent`); nothing reaches the screen.
A session that sleeps by closing its pane saves the registration it held, and the pane that wakes it takes that registration over: the registration keeps its id, name, parent and watch, the letters addressed to or from it follow it to the new pane, and the delivery store logs `agent.continued` with the agent id and the pane it left.
An archived session's member (PRD session-archive) is continued the same way when 이어가기 brings it back ([ARCHITECTURE.md: Session archive](ARCHITECTURE.md#session-archive)).
An agent closed together with its parent (a subtree close) keeps its registration while its Reopen closed entry is listed, and the reopen continues it the same way in the pane its conversation resumes in, so the child is the same delegated agent under the same parent; an entry that leaves the list unreopened lets its registrations end with their panes ([ARCHITECTURE.md: Reopening locally closed work](ARCHITECTURE.md#reopening-locally-closed-work)).
An agent whose CLI runs no session hook on resume (Cursor) gets its session from the core: the wake worker reports to Herdr the id the pane resumed, so the same-session check below has a session to compare ([ARCHITECTURE.md: Agent sleep](ARCHITECTURE.md#agent-sleep)).
The core decides this from the wake it started, only when the new pane runs the same native session, kind and device; any other pane is not given the registration, and a registration it cannot continue ends with the pane that held it.
Removing a device retires its cached pane read and rejects late reads from its retired coordinator; removal does not prove its panes gone or end any registration or watch.
Only the coordinator sharing the currently installed remote control connector may begin a pane read, publish delivery observations or replace the remote session and connection status.
Each check holds the runtime lock through its write, including a second check after projecting a remote session; replacing the coordinator for the same device ID rejects the old one's late snapshot and failure, while the current coordinator may bootstrap before its first connected status.

An agent started through Herdr directly bypasses Hide's responsibility record.
The Claude Code and Codex `PreToolUse` spawn guard refuses such a call in a registered checkout and offers two filled commands: delegation with `--parent here`, or operator handoff without it ([agent-hooks.md](agent-hooks.md#the-spawn-guard)).
The guard remains bypassable guidance; it does not change either mode's authority or execution path.
A launch chained behind another command is refused whole, and a launch inside `bash -c`, `$(...)`, a script or an alias is not seen, so a child started that way has no parent line, as before the guard.
A launch the guard did not see, such as one in an unregistered folder, may still be seen afterward by the passive Post observation, which records the started agent only as the operator's root with its starter as origin, never as a child ([agent-hooks.md](agent-hooks.md#passive-direct-prompt-observations)).
A spawn whose child shows no session within 5 seconds (Codex before its first turn, OpenCode 2 before its first prompt) answers `native_identity_unavailable`, and the core binds the child when Herdr shows the session ([A registration that waits for its agent's session](#a-registration-that-waits-for-its-agents-session)).
A spawn's tab runs the account's shell with Hide's shell integration where that shell is zsh, bash or fish (Windows PowerShell on Windows), and its start is typed only after the shell marked its first prompt ([ARCHITECTURE.md](ARCHITECTURE.md)); a shell stopped at a startup question within the 30 s wait is refused as `shell_starting` with that question's line, nothing typed, and the next action is to answer it in that pane and spawn again.
Herdr refuses a start as `agent_pane_busy` while anything besides the shell runs in the pane; on Windows that is any child process of the shell, which `pane.process_info` does not name unless it is an agent, so Hide's own reading keeps saying ready and only Herdr's next answer can say the pane is free.
The start asks Herdr again once every 100 ms inside the same 30 s wait, never in a burst (on Windows CI on 2026-10-22, asking with no pause sent 5,613 starts in 30 s while the spawn held the machine's spawn lock), logs `agent_start` / `pane_busy.timeout` with the count when the wait ends, and the spawn is refused `pane_busy` with nothing typed; the next action is to let that program finish or end it in that pane and run the same command again with the same intent, which starts in the same pane.
A start Herdr typed whose agent never showed (a startup question took its first letter in a bash pane, or the program exited before Herdr saw it) leaves Herdr holding the agent's name in that pane until its 120 s deadline, and Herdr gives that name to any agent that appears there meanwhile.
A spawn waits up to 30 seconds for Herdr to create the worktree, which runs `git worktree add` and can be slow on a large repository under load; when no answer comes in that time the spawn is refused `worktree_unconfirmed`, because Herdr may still finish the worktree, and the next action is to run the same command again with the same intent, which finds that worktree and continues in it.
A Codex start on a machine whose install kit has not read its Codex is refused `codex_capability_unknown` with nothing typed, because every Codex Hide starts takes `--no-daemon` only when that Codex has the shared server ([agent-hooks.md](agent-hooks.md)); the next action is to have the operator press Check again in Settings > Agents for that machine and run the same command with the same intent, which continues in the pane the spawn made, and the screen words the code in the operator's language wherever it shows a start refusal; why the read failed is logged once by the core as `kit` / `codex_capability_unread` with the machine's device id, the cause, and the exit code or a signal, never a path or Codex's own output, and not at all when the read stopped because Hide was quitting.
Running the same intent again while the name is held answers `native_identity_unavailable` and types nothing; once Herdr let it go and the shell alone holds the terminal it types the start once more (`start.resent`), and a second start that never showed, or anything else in the foreground, answers `agent_not_started`, whose next action is to look at that pane and spawn with a new intent.
A spawn about to start its agent closes the pane of every earlier spawn of the same parent, name and device that made a pane and never bound a child, where no agent shows and the shell alone holds the terminal, so a new attempt leaves no extra tab and Herdr no longer holds the name for it; each close is logged as `spawn` / `unstarted_attempt.close`, and a pane that cannot be read or closed is left to Herdr's refusal.
A spawn records the Herdr terminal its pane runs (`terminal_id` from `pane.get`) together with the pane, because Herdr hands a pane id out again once its server is replaced, so an earlier spawn's pane id can name an unrelated shell the operator opened.
An earlier pane is closed only while Herdr still reports the terminal its spawn recorded; a pane id now running another terminal is logged with the reason `pane_replaced`, and a spawn record written before terminals were recorded with `pane_identity_unrecorded`, and both panes are left open.

The existing one-second `agent.list` refresh, also requested by native events, reconciles only panes whose four lineage tokens differ; startup and reconnect perform one full pass.
The same pass writes the read axis's server mark (`hide_server`) on a pane whose saved read record or completion names the current connection's mark and that does not carry it, in the pane's lineage report when it has one ([status-model.md](status-model.md#across-a-herdr-restart)); a change in which panes want a mark replans like a moved registration.
Each server's pass reads the registrations recorded under its machine: this machine's are keyed by its node id, the same `machine` that `hide agent list` shows, and a device's by its device id, so a token Herdr drops (a live handoff drops every pane token) is written again on this machine as on a device.
An unchanged native observation and append-only registration count skip planning; unrelated letter/watch writes and label/process/catalog publications do not start a lineage pass.
The native pane and positive session select the matching retained registration, including an ended one, independently of append order.
A positive child-session replacement clears stale tokens; an absent native session or agent end leaves the pane's tokens in place, while unchanged readers reject obsolete session identities.
There is no separate lineage timer, subprocess on the input path or blocking work under `Mutex<Runtime>`.
The lineage token readers and digest contract remain unchanged; the server mark is read only by the read axis (`wire::server_mark`).

People receive notifications only for an unanswered parent inactivity warning and an overdue undelivered letter.
A first warning left unacknowledged, uncancelled and unreplied for 60 minutes triggers one human notification per native target and inactivity episode, shared across sibling watches and phone Web Push/Herdr channels.
The receipt lives on the existing warning letters and survives daemon restart and watch stop/restart; new target activity starts a new episode.
The second agent warning introduces no second human notification schedule.
An overdue letter's notification key is its ID plus the cause and sends once across both channels.
A channel that cannot reach the operator is not tried, and a failed first channel falls back to the other; two failures record a diagnostic without retry.
The phone is skipped before any send when the push key is missing (`no_vapid`), the push mode is off (`mode_off`) or is app-closed-only while a desktop or web shell is connected (`app_open`), Mobile is off (`mobile_off`) or no paired phone has a push subscription (`no_subscription`); the first that applies is the reason, and a send that fails or finds every subscription gone is `send_failed` or `subscription_gone`.
The Herdr toast goes to the machine the operator works at: a screen machine whose node dialed this core and is connected, through that node's link, and otherwise this machine's own Herdr (PRD core-host-node-remote-core D-18); a letter's doorbell to an agent on that screen machine goes through the same link, and while it is down the letter waits as it does for any disconnected device.
Herdr is skipped only when that machine has no Herdr socket (`no_socket`), because the pinned contract has no method that reads its toast setting; otherwise its answer decides, and `disabled`, `rate_limited`, `no_foreground_client` and `busy` are Herdr's own `reason` for a notification it did not show, with `call_failed` and `answer_unreadable` when it gave no usable answer.
When neither channel reached the operator, one `human.channels_failed` record in `Logs/core.jsonl` carries the notice (`letter_undelivered` or `observer_unconfirmed`), the letter id, `push`, `herdr` and the machine whose Herdr was asked (`herdr_node`); it never carries a letter's text, a phone endpoint or a key.
The claim stays consumed, so the same letter or warning is never announced again, and nothing is drawn on screen: an operator with phone push off and Herdr's toast off learns of a held letter from that record only.
These cases add no Inbox screen or automatic escalation chain.

### A registration that waits for its agent's session

An agent Hide started is registered from the session Herdr shows in its pane, and some agents show none until their first turn.
Codex 0.163.0 runs the session hook that Herdr's integration reports from only when its first turn starts, for `codex fork <id>` as for a new start, and OpenCode 2 makes no session before its first prompt; Claude Code runs its hook when it starts, so its spawns and forks register at once.
Instead of failing after a fixed time, such a registration waits in the ledger (`identity_waits`, `coordination::identity_wait`) until Herdr shows the session, and the core finishes it then.

`hide agent spawn` still waits up to 5 seconds for the child's session.
When none shows in that time it records the wait and answers `native_identity_unavailable`, an error with no agent, whose next action says the agent started and Hide registers it when it reports its session; the pane and the agent stay as they are, and nothing is typed again.
Until the child is bound, `hide agent list` does not show it, the parent has no watch on it and its pane carries no lineage tokens.
Once Herdr shows the session in the spawn's pane under the spawn's agent name, the lineage writer binds the child as the spawn's own answer would: the registration with its parent, or for a handoff its origin, the initial watch of a delegated child, and the lineage tokens on its pane, logged as `identity_wait.bound` with the wait, the agent and the pane.
Running the same command with the same intent then answers that child from the spawn's receipt and types nothing.
Asked again before the session shows, it answers `native_identity_unavailable` and keeps the one wait; asked after the session shows but before the core settled the wait, it binds the child itself, and the core's settle finds that same child, so either order leaves one registration and no wait.

Fork agent records the wait once the fork's agent has started: the source is registered then, while it is the execution that was forked, and the fork once Herdr shows the fork's own session, as the operator's root whose origin is the source ([status-model.md](status-model.md#where-a-parent-comes-from)).
The source's consent is the wait, so a fork whose source closed while the fork rested still names it as its origin.

A wait ends without a registration when its pane closes (`pane_closed`), when its agent ends and Herdr no longer lists the agent's name in the pane (`agent_ended`), or when another session shows in that pane (`pane_taken`).
A bind the ledger refuses ends the wait with that refusal, such as `parent_unavailable` for a spawn whose parent registration ended, and so does a registration that cannot be made of the agent as Herdr shows it, so nothing is asked again for an agent that cannot be registered.
A late session past what a registration keeps ends the wait as `session_too_long`, and the pane a spawn made for it is closed, as the spawn's own answer closes it; a fork's pane is the operator's and stays.
Each end is logged as `identity_wait.ended` with the wait and the reason, and nothing reaches the screen.
An agent's absence is judged only by an agent list read after the wait was recorded, because a list asked before may predate the agent's start.
A settle that failed, such as one the store did not take while a core move held its effects, is logged as `identity_wait.settle_failed` and asked again at the next agent list, as a failed token write is; a settle converges, so a repeat binds or ends the wait once, and a queued settle is never asked twice.
A core move carries each wait with the machine it names (`node_migration::KEYS`), so that machine's lineage writer judges it under its new id.
At most 64 waits are held at once: a spawn past that answers `identity_waits_full`, whose next action is to give one of the waiting agents a first prompt or close its pane, and a fork past it is logged as `registration_failed` with that code.
The waits ride on the existing one-second agent refresh, with no timer, no Herdr read of their own and no work under `Mutex<Runtime>`; an unchanged list with unchanged waits plans nothing.
A ledger with no wait writes no `identity_waits` field, and a build from before waits ignores the field.

## What runs now: `hide ps`

`hide ps` prints every live pane and the agent in it as one tree: project, checkout, tab, then pane, with each delegated agent under the agent that delegated it.

```text
herdr-ide  /Users/me/projects/herdr-ide
└─ checkout main
   └─ tab Agents
      ├─ w9J:pHA  conductor-spec [agent-12829]  claude  going  "Conductor 관리"
      │  └─ wVD:p2  hide-ps [agent-12894]  claude  going  "hide ps"  (watched by conductor-spec)  @ worktree feat/hide-ps · tab hide-ps
      └─ w9J:p12  terminal  "zsh"  <- you
```

Every pane is printed once.
A pane whose agent has a live parent (the lineage the sidebar draws, [status-model.md](status-model.md#where-a-parent-comes-from)) is printed under that parent, followed by `@` and where it lives when that is not the parent's tab, and is not printed again in its own tab; a tab or checkout that holds only such panes is not printed.
An orphan, whose parent is gone, is a root again and prints in its own tab.
A row reads: the pane id on its machine, the agent's name and registration id, its kind, its state, its title in quotes, the open mission it conducts or works a node of, and who watches it; a pane with no agent reads `terminal` and its own title.
A Conductor is named as one only while its pane runs the Conductor's session (the map's `present`), and a worker by its registration; when the mission record could not be read, the answer's `missions_unreadable` is true and the tree ends with a line that says so, rather than drawing no mission as if there were none.
The state is the row's mark as every surface draws it ([status-model.md: The mark](status-model.md#the-mark)): `turn`, `going`, `waiting`, `done`, `unread`, `quiet`, `unknown` or `asleep`, and `disconnected` for every agent on a device that does not answer.
A device's projects carry `@ <device>` on their line; its panes are the last ones Hide had from it while it does not answer.
After the tree, `Agents not in a pane` lists the live registrations no pane on screen holds (a sleeping agent, an older registration of a pane that now runs another session), and `--all` adds `Ended agents`, the ended registrations.
`--here` keeps every pane of the project of the pane it runs in, with its agents' delegated agents wherever they live (a pane of the project whose parent lives in another project is drawn in its own tab), and is refused `here_unavailable` from a caller with no pane in a project Hide shows.
`--json` prints the same answer as one line in the `agent` envelope, `{"ok":true,"value":{"panes":[…],"agents":[…],"missions_unreadable":false}}`, flat rather than nested: each pane carries its machine, project, checkout, tab, `you`, and its agent with `parent_machine` and `parent_pane` for the nesting; its schema is the contract's `ps` answer.
The tree uses box-drawing characters on every system: Rust writes UTF-8 to a pipe and UTF-16 to a Windows console, so no console Hide supports needs a fallback.

The answer is what the core already holds: the navigator's projects, tabs and agent rows, each device's last session, the mission map and the delivery ledger.
It asks Herdr and the devices nothing; under the runtime's lock it copies one row per pane and per agent row, and the ledger join and serialization run after the lock (`herdr-core/src/runtime/ps.rs`).
Like `hide agent catalog`, it needs the running daemon and a Hide credential but no open renderer, and a caller that is not an agent pane gets the same answer with no `you`.

## Missions

A Conductor's mission (PRD software-factory-conductor) is a record the core keeps in `~/.hide/state/missions.json` and changes only on `hide mission` commands from the Conductor's own pane; `hide mission help` is the Conductor's guide, which the `hide-conductor` skill the kit installs points at, as do `hide guide`, the `hide` skill and the session guidance ([agent-hooks.md](agent-hooks.md#what-the-kit-puts-down)); the help owns the commands and the Conductor's judgment around them (how to cut a mission, what a worker's brief says, the report's four parts).
Only a pane-bound credential can ask, refused `agent_pane_required` otherwise, and a pane conducts at most one open mission (`mission_already_conducting`).
A mission belongs to the repository Project of the checkout it was created in (`mission_project_unknown`); asked before the machine's projects are first read, it is refused `mission_projects_loading`, and a retry a few seconds later goes through.
Every other pane is refused `mission_not_conductor`, and a closed or cancelled mission refuses changes (`mission_closed`, `mission_cancelled`).
`hide mission take <id>` makes the caller the Conductor only once the previous one is gone; while it is there the answer is `mission_conducted` with its name.
A take hands the mission's workers to the caller's registration before it answers, and a take whose handover cannot be queued is refused with that reason and can be asked again: each worker's start, its child record (its parent, or its origin when it was started without one) and the watch on it become the new Conductor's, so the same intent asked again answers the worker it already has, and the worker's reports and warnings reach the new Conductor.
A new Conductor with no registration at the take is handed them when it registers to start its first worker, in the same reserve.
Letters already sent to the former Conductor stay in its inbox.
`hide mission reopen` of a done mission while its Conductor conducts another open one is refused `mission_already_conducting`, so a command or a start that names no mission always means the one open mission.
Each command answers one JSON line, and the same command again changes nothing more.
A plan is admitted only when every worker that takes a model names one (`mission_model_missing`, whose next action names the three places a model comes from: the candidate's fixed model, the catalog, or the catalog's `cli_default_model`); a worker of an agent that takes no model keeps `model: null`, so the map and the start never read an empty choice.
A mission takes Settings › Conductor's verification strength (`default_strength`: `fast`, `normal` or `thorough`) and its choice of checkers from another company than the node's worker (`other_vendor`) when it is created, and `hide mission show` shows them as `strength` and `other_vendor`, so a settings edit applies from the next mission.
A plan's `strength` changes the mission's before or during the mission, with why in its one-line `strength_why`, and each change records a `strength` event with that line; a plan that names none keeps it.
The strength is the Conductor's guidance for which nodes it gives a review or a run-it check and how many review rounds ([`hide mission help`](../hided/assets/mission/help.md)); the core enforces none of it, and the gate reads only each node's lines and `checks`.
A mission created before verification strength keeps the review and verify defaults it recorded (`checkers`) as they were written, shown by `hide mission show`, and has no strength until a plan names one.
`hide agent catalog` reads what the core holds, and Hide AI reads the model lists only while someone looks at them (`ai.rs`), so a catalog asked before Settings › Agents or a start panel was opened asks for one read the way the start panel does (`ask_ai_catalog_read`, observed until that read lands) and the daemon asks the core again every 250 ms, off the lock, for up to 10 seconds (`hided::core::CATALOG_WAIT`) while the answer says `models_read_pending`; after that an unread list is answered `models_unavailable_reason: models_read_timeout` beside `cli_default_model`, the provider snapshot's model where `cli_default` is true, so a plan made from it is never empty.

`hide mission decide --line <question>` puts the decision the Conductor waits on, and may add `--why <one line>`, a choice per `--option <text>` (at most 5, one line of at most 120 characters each), `--recommend <n>`, the recommended choice counted from 1, and `--node <key>`, the node it stopped (PRD mission-plan-visibility B32); `--clear` takes it away.
A node the plan does not have is `mission_node_unknown`, a recommendation outside the choices `mission_recommend_invalid`, too many choices `mission_options_too_many`, a choice that is empty, too long or more than one line `mission_option_invalid`, and `--why`, `--option`, `--recommend` or `--node` without `--line` `mission_question_without_line`, each with its next action; `--line` alone is as it was.
Each new question gets a key of the mission's own; the same command again changes nothing.
`hide mission node <key>` records a direct change node's commit on main with `--commit <sha>` (7 to 64 hex digits, `mission_commit_invalid`) and a verify node's `--verdict pass|fail` and `--checks <passed>/<total>` (`mission_checks_invalid` when passed exceeds total or the total is 0); on another kind of node they are refused `mission_field_kind_invalid`, and `--no-commit`, `--no-verdict` and `--no-checks` clear them (B33).

The operator answers a decision from the map's banner (B26-B28, D-30): the shell sends `mission_answer` with the mission, the question's key (0 for a decision from before keys) and the choice counted from 1 or a written answer of at most 500 characters.
The core keeps the answer on the mission, so a restart neither loses nor repeats it, and the doorbell's thread types it into the Conductor's conversation as the operator's own input, `[판단 답] <question> → <answer>` in the interface language, under a letter's rules: at once while the Conductor rests, after five minutes of its work as a letter waits, never over a draft, a menu or a question of its own ([Letters while the agent works](#letters-while-the-agent-works)).
An answer to a question the Conductor has since changed, or to a mission that is closed, is refused and only logged (`mission.answer_refused`); the same question answered again keeps the first answer.
Only a Conductor whose agent the doorbell rings takes an answer from the banner (`conductor.takes_answers`); any other is refused `mission_answer_unsupported`, and the banner sends the operator to the conversation instead.
An answer while no Conductor runs the mission, or while the mission's device does not answer, is refused and the banner says why (`answer_failed`: `conductor_gone`, `device_disconnected`); an accepted answer whose Conductor has been gone for 30 seconds before it is typed is dropped with `conductor_gone` and the event `answer_undelivered` (a shorter absence, such as the panes not yet read after a restart, only waits), the reason showing only while no Conductor runs the mission or the device still does not answer, and one the doorbell could not type goes back with `send_failed`, so the operator can answer again.
An answer whose `agent.prompt` Herdr refused, did not answer in time or answered in a shape Hide cannot read may already be in the conversation: it stays on the mission as `unconfirmed` with the event `answer_unconfirmed` (the record holds it so from before the prompt, so a Hide that stops before Herdr's reply does not type it again either), nothing types it again on its own and another answer to the same question is refused, and it is typed once more only when the operator sends it again from the banner (`mission_answer` with `resend`), which a second press finds already sending.
A Conductor that changes its question before reading an answer drops it, recorded as `answer_undelivered`, as does closing the mission or its last merge while an answer waits; clearing a decision that had no banner answer records `answered` from the conversation.
Answer events carry the node the question named, so they show in that node's row.

Each mission keeps its events beside the record, in `~/.hide/state/mission-events/<mission>.json` (D-15, D-22): the first plan with its node count, a replan with the nodes it changed and the reason one of them carries, each node's move to a state with its line, a further attempt, a pull request it names first and its merge, a commit on main, a decision asked with its choices, an answer from the banner or the conversation, one that was not delivered and one typed without confirmation, a change of the verification strength with its reason, the report and the cancel.
The same change again records nothing; at most 500 events are kept, the oldest dropped and counted (`mission_events.dropped` in the log), and recording never refuses the change it records (B21, B31).
The events go with their mission when it is deleted, and a start deletes any file of a mission the record no longer holds; an event's line is cut to 600 characters.

A spawn by a pane that conducts an open mission is that mission's worker:

- Before `hide mission start` it is refused `mission_not_started`.
- A spawn whose intent names a node of the plan (`<mission-id>:<key>`) without a brief is refused `mission_brief_missing`: a node's worker starts on the brief its spawn carries ([Agent registration and spawning](#agent-registration-and-spawning)), because a letter cannot wake a session that has not run its first turn (`session_unread`, above) and the worker would stand at an empty prompt until a person typed; a worker under another intent, such as a reviewer, is not held to it.
- A worker aimed at a machine other than the one the mission's project is on is refused `mission_machine_refused`.
- With worker candidates in Settings › Conductor when the mission was created (it keeps them; an edit applies from the next mission), a worker that is none of them is refused `mission_candidate_refused`: its agent, its model where a candidate fixes one or the CLI's default where it names none, and its effort where a candidate fixes one must match; an automatic effort is the Conductor's.
- The ledger counts the mission's live workers when it reserves the spawn, under the same lock, and refuses one past the mission's cap `mission_cap_reached` with the cap; the Conductor is not counted.
  A reserved spawn that has made no step for ten minutes and has no child stopped partway and is no longer counted, so a retry under a new intent can start; asked again later under its own intent, it takes a place under the cap again or is refused `mission_cap_reached`, so it never runs one worker past the cap.
- While the record cannot be read nothing counts the workers, and the guide tells the Conductor that answers `mission_unavailable` to start none and tell the operator.

Pull request changes wake the Conductor: when a node's pull request gets a new head commit, a review asking for changes, or is merged, the core records a wake on the mission and `hide:conductor` sends the Conductor one progress letter, which waits for no answer, naming the change and `hide mission show`.
A wake stays in the record until its letter is in the Conductor's inbox: a failed send, a restart and a mission whose Conductor is gone keep it, and the next GitHub read or `hide mission take` sends it.
Its intent is `mission:<id>:<node>:<number>:<event>:<head>:<n>:<conductor>`, with `-` for the number of an outside branch, where `n` counts the mission's wakes and `conductor` is the Conductor's registration (its pane when it has none), so a wake sent again after a restart is the same letter, the same change asked for twice is two, and a Conductor that took the mission over gets its own; a cancelled mission wakes no one.
On a mission closed as `done`, a new head or a review asking for changes reopens it, unless its Conductor conducts another open mission then, which gets the letter and leaves the mission done; its merge is the end it waited for and wakes no one.
A `merge_wait` mission closes as `done` on the GitHub read that finds every pull request of its stack merged or closed, whether or not its Conductor is there, and a close `--as merge_wait` of a stack that is already so, or of a mission with no pull request, is `done` at once; a node that failed or was dropped is out of the stack.
The intent carries the first 12 characters of the head, which `n` already makes unique, so it stays under the 256-byte intent limit.
A mission keeps at most 32 unsent wakes, dropping the oldest as `mission.wake_dropped`; one thread sends them, at most 256 at once, so the lock never waits on a delivery.
A change that would make the saved record larger than a start reads back is refused `mission_full`, as a full book is.
Deleting a Conductor's session from the archive deletes its missions with it.

### Finish lines

A node is done only when its finish lines are (PRD mission-map-git-criteria D-09 to D-16, D-23, B1-B15); the core is the one place that holds this, and no hook refuses a call or ends a turn over it (D-12).

- A plan is admitted only when every node other than an outside one has one to three lines of its own in `criteria` (`mission_criteria_missing`, `mission_criteria_too_many`, each naming the node).
  A plan's `criteria` written as one string is read as one line: that is the transition for a plan written from the guide before node criteria, and it goes once no open mission predates them.
- On admission the core adds the lines it judges: `check` on every change node of a plan with a `check` command, and `review` or `verify` for each kind the node's `checks` names (one to five agents, `all` or `majority`, `mission_checkers_invalid` otherwise).
  `ci` is added on the GitHub read that first finds checks on the node's pull request; a pull request GitHub reports no checks for has none.
  A replan keeps a line's record while its text and kind stay the same, and the `check` line's only while the plan's check command does.
- `hide mission record <node> <line> --pass|--fail --evidence <text> [--commit <sha>] [--rerun]` is the line's owner's: the node's worker for its own lines and `check`, the checker bound with `hide mission node <key> --reviewer|--verifier <agent-id>` for its line, and the Conductor only for a node that never had a worker registration, recorded as the Conductor's (D-23).
  Any other caller is refused `mission_criterion_not_yours`, a checker that is the node's own worker `mission_checker_is_worker`, an agent id the ledger does not know `mission_checker_unknown`, and a line the node does not have `mission_criterion_unknown`.
  Each record keeps who made it (role, name, registration) and when, and carries its evidence line; a blank or missing one is `mission_evidence_missing` (B5).
- `check` needs the commit it ran on (`mission_commit_required`) and is recorded from a pane in the checkout it ran in on the mission's machine (`mission_checkout_unknown`); the node keeps that checkout as its `folder`, and the line passes only while the catalog reads that checkout at the recorded commit, whoever works there now (B7, D-16).
  Once the node's pull request is merged, its merged head is the commit instead; a head Hide cannot read, a checkout gone from the catalog or a new worker on the node leaves the line open, and a pull request's head is never taken for the folder's.
  The same result on the same commit changes nothing.
  After a failure on one commit, one more record there is its rerun: a pass shows `rerun 1`, and a second failure sends a node in review, waiting or planned back to working; another record after that is `mission_rerun_spent`, and a new commit gives the rerun again (D-14, B13).
  `ci` is GitHub's answer for the pull request's head as it is now: a new head opens it again, a pass after a failure on the same head counts its rerun, and a failure after that rerun, whether the rerun failed or passed, spends it, sends the node back to working, and leaves later runs on that head unread (D-15); Hide adds no GitHub write.
- A review or verify line passes by its rule over the checkers bound to it, so `2/3` shows until the rule is met (B6).
- `hide mission skip <node> <line> --reason <text> [--decision <key>]` is the Conductor's: an empty reason is `mission_skip_reason_missing`, and `check` and `ci` are `mission_criteria_protected` unless `--decision` names a question about that node which the operator answered from the banner or whose clear recorded an answer (B11, B12); the line keeps its reason and, for those, the decision.
- `hide mission defect <node> --found-by <node> --line <text>` adds the defect as a line of the node it names (at most 16, `mission_defects_too_many`), opens that node again (`reopened`, a `reopened` event, back to working), and the same defect again changes nothing (D-21, B21).
- `--status done` on a node with a line open or failed is refused `mission_criteria_unmet` with the lines, judged on the node as the change would leave it: a pull request named in the same change takes its CI from the last list first, and a new worker opens the check again (B9).
  A close as `done` or `merge_wait` with a node neither done nor dropped is `mission_nodes_unfinished`, so a failed node holds the close until it is dropped, and one whose done node no longer meets every line now is `mission_criteria_unmet` with `<node>: <lines>; …` (B10); a cancel is never held.
  A waiting mission closes itself, on a GitHub read or on its branches reaching main, by the same rule as that close: every node done or dropped, and every done node's lines still met, so a node a defect sent back to work, or one set failed, keeps it waiting.
- A `merge_wait` mission waits on its `direct` changes as on its pull requests: each done node with no pull request is read, as an outside branch is, on every read while the mission runs or waits, and the commit found in the default branch is kept with the branch it was read for (`in_main`, shown in the view for `→ main <sha7>` as soon as it is there, beside the `commit` the Conductor recorded); a close as `merge_wait` with every such branch already read in main is done at once, as a stack already merged is.
  That proof stands only for that branch while the node stays done: another branch, the node back at work, the branch read past main again, a read that could not answer it (the node shows `unread`, logged `mission.branch_proof_unread`) or the mission reopened drops it, and an answer asked before such a change or before a reopen is dropped too; the next read that finds the branch in main gives it back.
  A node that names no branch is marked unread with why and keeps the mission waiting.
- The view gives each node its lines as they stand now (`criteria`), how many pass or were skipped (`criteria_met`, what the gate counts) and how many of those were skipped (`criteria_skipped`) and how often it was reopened; a skip records a `skipped` event.
- A mission made before node criteria is read `exempt`: its nodes and close are not held, and it shows as it was (D-19).

An outside node (`kind: external`, `external: {"pr": <n>}` or `{"branch": <name>}`, no worker, delivery, criteria or checks, `mission_external_invalid` otherwise) is followed to main by the core and never by its Conductor, which cannot set it done (D-20, B19, B20).
A pull request the project's list holds is settled on each GitHub read; any other is read with one `gh pr view <n> --json state`, and a branch by its commit against the default branch, locally or as last fetched, or the head of a pull request GitHub merged, off the lock, one read per project at a time and 32 nodes per read.
A read starts on the plan, the start, a reopen, a close as `merge_wait` or a node change of a waiting mission, a `direct` change turning done or named another branch while its mission runs, each GitHub read, each background fetch answer and each change in the local worktrees, and goes on at once with the nodes it left out until every node followed then was read once, so `n` followed nodes take `ceil(n / 32)` reads back to back with no trigger between them.
A trigger that comes while the project's read is out, such as a plan that adds a node or a node moved to another branch, is kept, one per project, and starts another pass as soon as the one out ends; an answer, read or unread, starts none, so a node Hide cannot read waits for the next trigger rather than being asked again in a loop.
Each node is one `gh pr view` or at most four git calls, each bounded by 60 s, so the worst case until every node is read is `n` x 4 x 60 s when every call runs to its bound; with local git answering at once the 33 nodes of one plan are read in two reads right after the plan is admitted.
The default branch is the worktree reader's ([UI_BEHAVIOR.md](UI_BEHAVIOR.md), the default branch rule), so a repository with no remote follows the branch its main worktree has out.
Merged or in main is done, closed without a merge is failed, which blocks the nodes after it, and either wakes the Conductor (`outside_done`, `outside_failed`); open waits, and one Hide could not read waits too, marked `unread` in the view while the reason goes to the log (`mission.outside_unread`).

## Plans

A plan (PRD hide-plan) is one native session's own list of steps, which the agent puts up and keeps current with `hide plan`; `hide plan help` is its guide, which the `hide-plan` skill the kit installs points at ([agent-hooks.md](agent-hooks.md#what-the-kit-puts-down)).
Only a pane-bound credential can ask (`agent_pane_required`), on this machine or from a connected device's pane through its helper, as a letter does, and the plan is keyed by the pane's native session (device, agent, session id): a resumed session goes on with it, a fork starts with none, and only that session changes it.
The commands are `set` (`--file <plan.json>` or `--json <plan>`, with `--ask [why]` and `--reason <text>`), `node <key>` (`--status planned|working|done|dropped`, `--line <text>`), `approve`, `drop`, `show [<agent>]` and `help`; each answers one JSON line, `{"ok":true,"result":{"session","plan","context"}}`, and the same command again changes nothing more.
A step has a `key`, a `title`, the keys it `depends_on` and a `status`, and may carry a one-line `line` and an `output` of `pr`, `file` or `none`.
The graph is checked by the same module the missions use (`herdr-core/src/dag.rs`): a key or title that is malformed, a dependency that names no step and a cycle are refused `plan_invalid` with where; more than 64 steps is `plan_too_large`, and a step waits on at most 8.
A step set `done` while a step it waits on is neither done nor dropped is refused `plan_order_refused` with the step it waits on; a dropped step holds nothing below it.
The current step is the topmost working one, else the topmost planned one; `n/m` counts done over done, working and planned, and a dropped step is in neither.
A `set` over a plan whose steps are not all finished keeps the steps it left out as dropped and marks every new, renamed or dropped step changed, with `--reason` (or `--ask`'s why) as the change; a `set` over a finished plan replaces it whole.
A plan put up with `--ask` waits for the operator's review (`proposed`); a plan without it is `approved`, and a later `set` without `--ask` keeps the review it had.
The operator answers in the conversation: the agent runs `approve` when the operator agrees, `set --ask` again with the change asked for, or `drop` when told not to; there is no button that approves.
`approve` and `drop` naming another agent are refused `plan_not_yours`, while `show` of another agent reads its plan; a name or id that matches no running registration is `plan_agent_unknown`.
While the saved plans have not been read back yet the answer is `plan_unavailable` and the read starts again; a session Hide cannot name yet is `plan_session_unknown`.

The core keeps the plans beside each session's record in the link store (`ledger_plans` in `links.sqlite3`, schema 9), written by the link worker off the lock and read back once when the worker starts; a plan goes with its session when the session is deleted or pruned.
The book holds at most 128 plans: a new session's plan takes the place of the one updated longest ago whose session runs in no pane (`plans.evicted`), and with every one of them on screen it is refused `plan_full`.
A finished plan stays in the session's record, and the record's view carries it as the map draws it.

A proposed plan is the agent's question: while the agent rests with no question of its own and is not working, its row carries a question with no words of its own, only its subject (`ask_subject`, `{cause: "plan_review", steps}`), which each surface words in its own language, so the pane's ask band, the sidebar's second line, the bell and the phone follow the ask's own path, and the agent's own question comes first ([status-model.md](status-model.md#a-plan-waiting-for-review-is-a-question)).
Every `hide plan` answer and the prompt hook's letter pull carry a small summary of the pane's plan and mission work (`plan_context`): the plan's review, its current and next step by key and title, its count, whether it is finished and when it was updated; the mission node, its scope and range when the session is a mission's worker; and a Conductor's counts.
Its shape is fixed in `contracts/delivery-envelope.json` (`plan_context`), which the core's summary and the hook's reading are both tested against; the hook keeps it in the pane's record and refuses from that record alone ([agent-hooks.md](agent-hooks.md#the-panes-plan-record)).

## Verification

Use the worktree-owned entrypoints described in [BUILD.md](BUILD.md#two-entrypoints).
Focused checks are:

```sh
bash scripts/verify-cargo.sh test-scoped -p herdr-core --lib delivery:: -- --nocapture
bash scripts/verify-cargo.sh test-scoped -p hide-session --lib session_activity -- --nocapture
bash scripts/verify-cargo.sh test-scoped -p hided --test it node_session_activity::
```

A filtered run must execute the expected named tests; zero selected tests is a failed check.
The remote mailbox lane (`hided/tests/it/remote_delivery.rs`) waits for the daemon's own doorbell to ring each recipient and hands that pane's prompt hook the line the letter kept, so it crosses the bell's recognition over the node link; its provider panes carry one finished Claude conversation each, because the doorbell rings a Claude Code pane only once its session read says nothing waits.
The helper executable tests use private homes and fixture transcripts, pair JSON-line responses by request ID, and cover activity success/refusal, privacy, kit coexistence, normal exit and abrupt owner loss.
Each fixture launch and capture shares an absolute five-second deadline and a combined 64 KiB output cap; provider sessions and the installed helper are never used.
The full Rust test and lint lanes still apply to the final committed head.
Actual Linux and Windows OS-contract runner results are required for the state-machine, ledger and activity portability claim; declaring a workflow does not prove it passed.
On Windows the `windows check` lane opens the ledger from a real state folder, registers two agents, sends a letter and reads it from the recipient's inbox and back from disk (`runtime::delivery::tests::agents_register_and_a_letter_reaches_the_inbox_through_the_saved_ledger`), and its `real_herdr` daemon must have saved its ledger at startup.
Real TUI delivery requires an isolated Herdr server, private HOME/state, precisely identified candidate processes and disposable provider sessions with observed native identity and registered lineage.
Register only an actual parent session and spawn its child through `hide agent spawn`; never seed a capability or coordination ledger.
Exercise idle/done delivery to a Claude pane with a statusline and to a Codex pane, a letter whose first line carries `@`, `$`, `/`, `!`, `#`, backticks, a trailing backslash and more than 60 characters of Korean (its bell is submitted as one prompt and its hook takes the letter), the same line typed by a person before the doorbell rings (an ordinary prompt: the letter stays pending until its own bell), working delay, a pending letter held while a permission, question or plan menu is open and delivered after it is answered, each target's menus read as Herdr `blocked` or, for Codex's plan approval, held as `awaiting_operator`, hook confirmation/restart, capacity/corruption, watch clocks/reset/exit/reply and helper privacy/fallback.
Label protocol fixtures separately from actual provider runtime observations.
Measure matched baseline/candidate input latency and idle/driven load through [PERFORMANCE_TESTING.md](PERFORMANCE_TESTING.md); a headless or socket-only check does not prove native presentation.
Own every fixture process with a deadline and teardown, preserve the operator's app/server/panes/hooks, and remove private authentication caches after the owned agents exit.
Run evidence stays under ignored `agents/runs/`; it is never committed with this guide.

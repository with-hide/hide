# Agent delivery and inactivity watches

Hide's core owns agent registration, spawning and lineage, the single mailbox and inactivity watches for local and connected device agents.
This guide is the public operating contract.
Approved implementation contracts and run state stay in the private local harness.
Human Inbox UI, relay/escalate commands, authority proof and automatic draft clearing remain follow-up work.
Derived UI escalation is separate: the core maps the existing delivery and watch facts to the six causes in [status-model.md](status-model.md#task-identity), without changing delivery clocks, retries, authority or the mailbox protocol.

## Commands and caller identity

These commands, the `hide agent` commands among them, need the running daemon and a current agent pane in a registered checkout; they work without an open renderer.
`hide guide` is an agent's guide to all of them, and to the Workspace commands and Project Memory, and needs neither: it prints `hided/assets/guide.md` with the session-start rules (`hide_agent_hooks::PURPOSE_CONTEXT`) inside it, so the guide and the session note cannot say two different things, and a skill or a hook that teaches an agent Hide points at it rather than copying it.
The daemon binds the caller through the existing Workspace credential boundary and resolves the actual pane, provider and native-session identity from the engine.
Only a pane-bound caller is accepted: a checkout-bound caller (a plain terminal, or a tool shell inside Codex's shared app-server daemon) is refused `agent_pane_required` whatever pane it names, and its next step is to run the command inside the agent's own pane, or to run that Codex without the shared daemon (`--no-daemon`).
A pane hint cannot replace that binding: a pane-bound caller whose hint names another pane is refused `caller_identity_conflict`, and an absent, ambiguous or changed occupant returns an explicit error.
Each queued command revalidates the caller's pane against the prepared Workspace and checkout context before applying or saving; a moved checkout returns `caller_context_changed`.
Mailbox callers and new recipients require a positive native-session binding; a missing binding returns `native_identity_required`.
On the core's own engine every supported agent's integration reports the pane's session when it starts or the pane switches to another, so its mailbox identity is there before its first turn ([agent-hooks.md](agent-hooks.md#which-session-is-the-panes)).
Two missing native references in the same pane never authorize retained mail.
A pane's public address can change while its native terminal and session stay the same.
The registration stores the terminal identity separately from its address; a unique fresh observation on the same device and engine scope with the same native session and agent kind continues that registration, letters, watches and spawn receipt in one ledger transaction.
A missing identity, another session, a reused address or an ambiguous terminal never proves continuity.
An inherited Workspace capability follows the pane's caller alias only after the terminal identity and shell process birth still match, and its registered checkout context must remain the same.
Target names and pane IDs resolve against the daemon's current observations.
Each machine's engine numbers its panes, and an agent learns the numbers from its own machine's engine, so a bare pane ID (`w1:p2`) names the pane on the caller's own machine: a device agent's `w1:p2` is that device's pane, and the core's own agent's `w1:p2` is the core's.
A bare pane ID never reaches a pane on another machine, even when only that machine has the number: the command is refused `target_unavailable`.
A pane on another machine is reached by the agent's name or registration ID, or by the full address of a device's pane, `remote:<device>:pane:<id>`.
The reference the shell copies for a device's pane (Copy name and ID, a dragged agent row, a pane's `@`) carries that full address, so an agent anywhere can read, type into or write to it as written; a pane of the caller's machine is copied bare.
The session-start text and `hide guide` (`PURPOSE_CONTEXT`) say so, and the `pane_unavailable` and `agent_unavailable` refusals name the full address in their `next_action`.
The same reading applies to the pane IDs of `--actor` and `--observer`.
A name that two machines share, such as the default `hide-claude-w1-p2`, is refused `target_ambiguous`.
A letter's target `parent` names the sender's own parent, the registration that answers for it, because a child is never told its parent's id; it wins over an agent registered under that name, and a sender with no parent is refused `no_parent`, whose `next_action` says no agent answers for it.
Each refusal's `next_action` says so: `target_ambiguous` points to the registration ID from `hide agent list` or the full address, and `target_unavailable` says a bare pane ID names a pane on the caller's own machine and points to the name or registration ID; neither is told to retry.
A letter may also go to a registered agent Hide put to sleep, by its name, registration ID or the pane it sleeps in (PRD mission-worker-sleep B1): the agent no longer runs, so no observation answers for it, and the registration whose pane keeps its sleep record for the same session, or whose ID a dormant record holds, is the recipient ([ARCHITECTURE.md: Sleep by mission, letter and command](ARCHITECTURE.md#sleep-by-mission-letter-and-command)).
The letter is pending at once; a `request` or `block`, and the operator's Done on a person's turn, wakes the agent with its conversation resumed, at most four such wakes at a time, in the order the letters came, and the letter is handed over by the usual bell once the agent is back, while any other `report` or `progress` waits for whatever wakes it next (`Letter::wakes_a_sleeper`).
A wake refused before it typed anything, for a cause that passes on its own (the pane's machine could not take the input now or read its shell, a program still held the pane's terminal, another change of its holder held its turn, or the link lost the answer), is tried again after 5 seconds and then 10, three wakes in all, so a passing refusal no longer ends a letter at once (with-hide/hide-private#660, #675).
A wake that fails otherwise (it may have typed the start, its folder is gone, its profile, launch choice or name is refused, its shell had not finished starting or waits for a key), or the third wake refused for a passing cause, leaves the `request` and `block` letters that waited on it `undelivered` at once, which ends their answer waits, and the sender gets a letter from `hide:wake` naming the recipient and why, read at its next tool call even while it works; the next `request` or `block` tries the wake again.
A connected device recipient uses its node's pane service over that device's existing SSH link; the credential fixes its device, link, pane, sender and kind, and losing the link revokes that credential immediately.
The only ledger remains on the controlling daemon.
A disconnected device hook finishes within its two-second budget with no letters, leaving them pending in that ledger.

An installed hook carries the state folder of the daemon whose kit wrote it, independent of the agent shell's `XDG_STATE_HOME`.
The helper sends its hook requests, the pull and the confirmation alike, to that captured route only when `HIDE_STATE_DIR` is absent or empty; capability validation and caller binding remain the daemon's ([agent-hooks.md](agent-hooks.md#what-the-hook-returns-and-reports-back)).
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
hide inbox --ack letter-9~3f9a1c2b7d4e
hide watch start child-name
hide watch list
hide watch stop watch-2
hide request send parent-name --kind progress --intent task-plan-1 --body 'Plan: core first, then web.'
hide request send parent-name --kind report --intent task-complete-1 --body 'The check passed.'
hide request send child-name --intent task-stop-1 --body 'Stop and report what you have.' --force
```

A `request` or `block` letter waits for an answer; a `progress` letter is an interim report, such as a plan or a checkpoint, and a `report` letter says the work is finished.
Only a `report` ends the parent's watch on its sender ([Watch activity and completion](#watch-activity-and-completion)), so a child that has more to do after what it writes sends `progress`.
`hide guide` tells an agent to send `progress` at a plan, a change of direction or before a long wait, not at every step, since its recipient reads only its newest three in full ([One bundle per handover](#one-bundle-per-handover)).

The recipient can acknowledge or reply; the sender can cancel.
A `request` or `block` letter waits for its answer until one of four things ends the wait: the recipient replies, the sender cancels, either side's registration ends, or 24 hours pass since it was sent ([Persistence, clocks and limits](#persistence-clocks-and-limits)).
When the last two end it, the letter keeps its `state`, `waiting_answer` turns `false`, `finished_at_unix_ms` is set, and `answer_wait_ended` says why: `party_ended` or `deadline`; it is `null` on every other letter.
The sender reads it in the answer of `hide request show` (and in every other answer that carries the letter); `hide inbox` lists the same letters as before and does not announce a wait that ended.
A letter that ended its wait without an answer no longer counts against the open-letter limit, and the recipient can still reply to it by the same rules.
It is still a retained letter: it leaves the ledger one day after its wait ended, or earlier when the 5000-letter total is full and it is the finished letter that finished first (see [Persistence, clocks and limits](#persistence-clocks-and-limits)).
Acknowledgement is the recipient's receipt for every agent kind: only the recipient's own pane and native session can acknowledge (`hide request ack` from a pane-bound caller is the only path, with no operator or helper path), so it records `hook_confirmed: true` and ends a matching report watch (a `progress` letter ends none); a reply separately closes the original request's answer wait.
Retry the same intent after an interrupted call: the same sender identity, intent, recipient, kind and body return the existing letter during its retention period, including after cancellation or delivery, for one day after the letter finished or until a full ledger drops it first; a send with the same intent after that creates a new letter.
The same intent with another recipient, kind or body is refused `intent_conflict` with nothing stored, as a spawn is when its intent is reused with another mode or machine, so a letter is never answered with an earlier one that went elsewhere; the recipient is matched by the name, registration id or pane id the send names, so a retry still converges after the recipient left.
A report is the same letter when the body sent is the beginning of the stored one, which gained the list of Views its sender opened.
A reply is the same letter when it answers the same letter with the same body; a reply under an intent that was used for a send, for a reply to another letter, or for another body is refused `intent_conflict` too.
Use a new intent for a new letter, including the same text for another recipient.
The envelope identifies the sender and letter kind; it is not a session-level authority or anti-forgery proof.
The current first-line format is `Hide letter <id> from <name> (<agent>) [<kind>]`, with independent writer and transcript-reader examples in [delivery-envelope.json](../contracts/delivery-envelope.json).

### Who may read a letter

`hide request show <id>` answers a letter's sender and its recipient, and every other caller is refused `letter_unavailable`, as for a letter that does not exist; its `next_action` says to ask the agent that named the letter for its body.
The other readers are the agents bound to a mission node, its worker (`agent_id`) and the checkers bound to its lines (its reviewer and verifier): each may read a letter between the mission's Conductor and another agent bound to that same node, either way.
A Conductor hands a node's work around by a letter's id, the worker's design to the reviewer and the review to the verifier, so without it the checker was refused the letter it was pointed at and asked for the text again (with-hide/hide#2298), as the worker once was for its reviewer's verdict (with-hide/hide-private#656).
The core works the pairs out under its lock from the mission book at each call (`MissionBook::node_letters_read_by`, `Runtime::node_exchanges`): the reader is bound to the node, the other party of the letter is the Conductor of the same mission, and its writer or recipient is another agent bound to that node.
It hands them to the store as data (`Authority::also_reads`), and the store alone decides the read (`mailbox::show`), so what a read answers is the same for every reader.
An agent of another node, of another mission or of no mission, an agent its node no longer binds, and a letter of the node's agents to anyone but the Conductor, such as the worker's letter to its reviewer, are read by nobody beyond the letter's own two parties.
The grant is a read and nothing else: taking the letter in, cancelling it and answering it stay with its sender and recipient (`mailbox::authorized`), and an agent of the node that tries is refused `letter_unavailable` as before.
`hide-core/src/runtime/tests/mission_letters.rs` holds the boundary.
`<name>` is the sender's [wire name](#names-on-the-wire-and-on-screen), and so are the names of a letter's and a watch's endpoints in every answer.
A batch keeps the first letter's sender attribution in the request view; neither later headers nor older delivery formats select a sender.

### The contract a calling tool checks

A tool that runs these commands from a program (such as a dispatch client) checks the installed `hide` before it calls one.
`hide version --json` answers `{"version", "commit", "contract"}`: the version the build reports, the commit it was built from (`null` for a debug build given none, or a build from a tree with no Git), and the `sha256:` digest of the command contract; neither it nor `hide contract --json` needs a daemon or a pane.
`hide contract --json` answers that contract:

- `format`: the document's own format, now 1.
- `commands`: each command's words, its positional `arguments` with their value types (`"required": false` on one it takes without, which leaves out every argument after it, and `value_with` naming the option that widens what it admits: `hide pane send`'s text is `text`, and `paste_text`, line breaks and tabs too, with `--paste`), `repeats_at_most` for a last argument that repeats, its `options` (each with `name`, `value` type or `null` for a switch, `required`, the options it `requires`, and `repeats_at_most` for one given more than once), `needs_one_of` for a command that needs at least one of the arguments and options it names (`hide pane send`: its text, a `--key` or `--enter`), what `rest` passes through after `--`, the names of the `answers` it can return, and, where the contract declares them, the `refusals` its own rule answers with; those are not every failure, since every command can also fail for the daemon, the pane or the ledger.
- `value_types`: what each value type admits (`text`, `key`, `body`, `paste_text`, `approval`, `unsigned`, `one_of` with its `values`).
- `envelopes`: for each topic, where the printed line carries `ok`, the `answer` and the failure `code`; a failure also exits non-zero.
- `answers`: the JSON Schema of each answer, and `digest`, the `sha256:` of the document without it, its keys sorted and written compactly.

A caller compares the digest it was written against, or the commands and options it uses, and refuses a mismatch before running anything.
The CLI admits only the commands and flags the contract names (`hided/src/cli_contract.rs`), its tests hold each parser to the table, and the answer schemas come from the core's answer types (`hide-core/src/delivery/answer.rs`), so a changed command name, option or answer field changes the exported contract.
A command whose standard output is closed by its reader (`hide agent list | head`) ends quietly with status 141 and prints nothing to standard error; `docs/ARCHITECTURE.md`, The platform layer, owns the rule.
`contracts/hide-cli.json` is the committed copy, and a test fails until it is regenerated with `target/debug/hide contract --json | python3 -m json.tool > contracts/hide-cli.json`.

## Safe intake and manual fallback

A doorbell types one line that says what arrived ([The bell line](#the-bell-line)).
It is typed when every one of these hide-owned facts holds, and the verdict reads no terminal screen, so a redrawn footer or a user's statusline cannot change it (the one screen read, of a composer held as a draft, happens before the verdict and can only release that draft, [Reading the composer](#reading-the-composer)).

Whether the agent is at rest, works or waits for the operator is the core's one turn, the same answer its row is drawn from ([status-model.md](status-model.md#where-each-axis-comes-from)): the hook's turn first, then the session read, then the engine (`agent_state::turn_projection`).
The runtime lays it on the pane's delivery observation where it lays the hook's and the session read's facts, and it stands only for the pass that last observed the pane and the engine state it was laid under (`Observation::projected_turn`).
A sync pass observes the engine's list for delivery and lays that pass's turn in one step under the lock: for this machine the session ingest observes the payload before it lays the pane moves, the label overlay and the hook's facts on it (`ingest_observed_session`), and for a device its coordinator's first step does the same before the session is projected outside the lock (`observe_device_session`).
No ask and no bell therefore meets an observation without its turn, and a turn laid by an earlier pass is never read with a newer observation: each observation carries its pass's number (`observed_generation`), so a question the session read finds while the engine's state and sequence stay the same never rings on the turn before it.
Only that step lays a turn (`ObservedPass`): every other ingest, such as an editor's close status read with the overlay the runtime held before, observes nothing and lays nothing.
A letter holds as `turn_unprojected` only while a pane move this machine runs draws the moved pane as it was before the move (`overlay_pane_moves`), which the screen draws too, until the move's barrier is released.
Whether the pane takes input now is apart from the turn, and a row shown at rest is never read as safe to type into: the gates below on the engine's own state, Hide's input clocks and the composer decide that.

- The turn has no approval prompt and no question or plan approval of the agent's own waiting.
- The turn is at rest; or it works, the adapter declares no tool-letter intake and the letter's wait is over, counted from when the turn began to work ([Letters while the agent works](#letters-while-the-agent-works)).
- The engine knows the agent's state (`idle`, `done`, `working` or `blocked`, with a state sequence), and does not hold it at its blocked prompt, where its prompt operation refuses (`agent_blocked`): a pane the engine reads `blocked` under a turn whose hook reported no prompt is held `not_ready`.
  A kind that takes letters at its tools is not rung while the engine reads it `working`, whatever the turn says, since the engine reads Grok's plan review as work.
- Neither the engine's state nor the turn changed in the last 30 seconds.
- Hide has routed no key to the pane for 30 seconds.
- No input hide routed is newer than the later of the pane's last submission and the last time it entered `working`.
  Typed keys, a pasted attachment, a phone key and the keys of an agent's own find box all count as input, and none of them, Enter included, counts as a submission.
  A submission is a prompt hook that ran with the pane's own native session id, which the hook passes to `hide inbox --hook --session`, or a phone reply.
  A pane that entered `working` was submitted to as well, which is how an answered menu or a custom slash command that starts a turn clears the hold.
  A key after both is an unsent draft, an Esc-restored prompt or a recalled input; a pane held by one stays held until the next real prompt, or until its composer is read empty ([Reading the composer](#reading-the-composer)).
- The pane still hosts the native session the letter was written for, and the agent kind is one the bell targets.
- For Claude Code, Codex and Grok, whose session readers report turns, a read made for the engine's current state says no native question or plan approval waits for the operator (see [A menu the engine reads as a stop](#a-menu-the-engine-reads-as-a-stop)).
- The engine does not report the agent as starting or not ready.
  It reports readiness for every agent it lists: launching (`launch_pending`) while a start it typed settles or is blocked, not ready while the program takes no input (it has not shown, its holder could not say what holds the terminal, the terminal is editing lines, or the 3 s since it took the keys or had its question answered have not passed), and ready otherwise (`PaneAgent::readiness`, `PaneAgent::takes_input`).
  An agent the operator started by typing its program in a shell, the usual way a lead is started, has no start of its own, so only whether its program takes input decides it.
  Only a launching or a not ready report holds the letter.
  The engine ends a pending start itself, at its settle (3 s after the start, once the program takes input and its screen rests) or at its 120 s deadline, and the coordinator reads the session again at that moment, so the core reads the start settled and the letter is judged again ([ARCHITECTURE.md](ARCHITECTURE.md#agents-on-the-cores-own-engine)).
  A start whose screen asks a question (a trust or login prompt at launch) stays pending past the deadline until the question is answered, and its letter waits until then; answered past the deadline, the start ends at once and the agent reads not ready for the 3 s its next screen takes to draw.

The engine's `blocked` status holds every permission or selection menu it reports that way.
For a menu it reports as idle or done, a bell target also needs Hide's draft record or a native session wait to hold the bell.
Claude Code and Codex can be rung at rest and while working; Grok can be rung only at rest, with Hide's draft guard and the native-session wait guard below.
Enabling another target needs its menus measured on the pinned CLI, with the unsafe and unknown cells named in its pull request; the local measurement tool that once repeated that scene matrix is retired.
The adapter declaration is a reviewed product decision, never changed by a measurement.
The shared agent adapter declares bell eligibility separately from prompt intake, spawn refusal and session format; its contract requires a submission hook and either prompt context or a turn-end intake hook for every bell, and adding prompt intake alone never widens the bell targets.
Claude Code's plan approval reads `blocked`; Codex's does not, and the session read guards it instead (next section).
Cursor is not a target: its menus have not been observed, and the agent checks leave Cursor out by the operator's decision of 2026-10-17.
Grok is a target at rest: a kind with tool letters never receives a working bell, so its plan review, which the engine reads as `working`, is held.
Its bell uses the same engine prompt submission (`prompt_agent`) as every bell target; its prompt hook records the submission and recognizes the saved line, but prints and confirms nothing.
The bell turn takes its pending batch at the first `PreToolUse`, or at `Stop` if it calls no tool.
The following observations were made on 2026-10-18 with Grok 1.0.52 and 1.0.62 under isolated Herdr 0.9.5 (Grok detection manifest 2026.10.05.1), answered by a scripted stand-in model.
The picker observations below used the then-current `pane.send_input` and Enter path; they are not evidence for `agent.prompt`.
Grok's three hooks did the same on both versions:

- `UserPromptSubmit` runs with the prompt and the session id, but nothing it prints reaches the model (Grok's hooks guide: "stdout of an allowing hook is discarded").
- A `PreToolUse` hook's `additionalContext` reaches the model after the tool's result, which is how Grok's letters are handed over while it works.
- A `Stop` hook's block reason reaches the model as "Stop hook feedback", and the turn goes on.

| Scene | Herdr 0.9.5 read | What the line did | Seen on |
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
Measured under Herdr 0.9.1, a built-in slash picker such as `/model` or `/resume` read as `done`, not `blocked`, for both targets, and a bell typed into an open picker was accepted by it.
Hide therefore does not take the Enter that opens a picker for a submission: the pane holds as a draft, and stays held after `/clear`, `/model` or `/help` until the next real prompt runs its hook.
The same holds after an Esc or Ctrl-C that interrupts a turn, since no prompt hook runs for it.
On Grok 1.0.62 a completed background command fires `UserPromptSubmit` with the root session id and a `<system-reminder>` describing the completed task; its digest is not the saved bell line (measured against a local fake model).
Two residuals remain: a turn the operator did not start (a scheduled wake or a finished subagent, including Grok's auto-wake turns) moves the pane to `working` and clears a half-typed draft, a prompt queued while the pane works leaves it held after its turn until the next prompt submitted from rest (its hook runs while `working` and clears nothing).
A letter that cannot be belled waits, and the reason is logged as `doorbell.held` once per change with the letter and pane ids, never with its body and never on screen; the one hold that does not end on its own, a possible draft, is shown on the pane once it has rested ([Delivering a held letter now](#delivering-a-held-letter-now)).

### Letters while the agent works

A letter to an agent that is working no longer waits for its turn to end (PRD letters-to-busy-agents).
It waits five minutes of unbroken work (`doorbell::MID_TURN_WAIT_MS`), counted from the later of its arrival and the moment the pane entered `working`, so an agent that just started a long turn is not interrupted at once and a letter that arrives late in a turn is not handed over on the next tool; a letter sent with `hide request send --now` (`Letter.immediate`) skips the wait and goes at the recipient's next safe gap.
How it reaches the agent follows the kind, and one letter takes one path and is never both belled and handed over at a tool (D-10); the adapter contract holds `bell` and `tool_letters` to be each other's exception:

- Claude Code and Codex, the bell targets, take the line the doorbell submits through the engine's prompt operation while they work: both read a line submitted during a turn after the current tool call, and the line's prompt hook takes the letters as it does at rest.
  A turn its hook reported ended while the engine still reads the screen as working is at rest for the bell as on the row; the draft and quiet gates, read from the engine's and Hide's own clocks, still hold it, and a turn the agent runs without a prompt hook (a background task's wake) takes the line after its current tool call as any mid-turn line.
  Every other guard of a bell holds as it does at rest: a draft typed since the work began, a key or a status change in the last 30 seconds, a menu, a session not read, and a `working` state the engine has not yet confirmed (`state_change_seq`), so the composer is never typed over the operator's own words (D-09).
  A pane belled while working is not belled again for a later letter until the letter it rang for has left `pending`: submitting a bell to a working pane moves nothing Hide can see, so the second ring would otherwise follow at once (`doorbell::Rung`).
- OpenCode, Pi and omp take them at a tool result: Hide's plugin (`tool.execute.after`) and extension (`tool_result`) ask the helper's `letters` operation, which asks for them as `hide inbox --hook --session <id> --tool` does, and append the bodies to the tool's output inside `<system-reminder>`, so the model reads them before its next step; they are confirmed as a prompt's letters are: OpenCode's once it stored the completed tool part of that session with the letters still in its output (OpenCode 2's once `session.tool.success` reports the result stored with them), Pi's and omp's once the host finished the result with Hide's text still in its content and the reply after it has ended (a `message_end` handler may still replace the result itself); a result stored or finished without the text never reached the model, and its letters stay pending in Hide for the next ask.
  While letters a script handed over wait for their receipt, the host's write and then Hide's confirmation, the script asks nothing, so a letter is never written twice; a write that never comes is forgotten after ten minutes, and the letter then rides the next ask.
- Grok and Cursor take them through their own `PreToolUse` and `preToolUse` hook: a call that runs is answered with the letters as the hook's added context (`hookSpecificOutput.additionalContext`, which Grok's guide says reaches the model with the batch's tool results; `additional_context` beside `"permission": "allow"` for Cursor), and confirmed once that answer is printed.
  Every tool call reaches the hook, since neither entry carries a matcher (Grok `hide-guidance@7`, Cursor `hide-guidance@6`; an omitted matcher matches everything in Grok's guide, and Cursor's is optional), so a `--now` letter goes at the next call whatever its tool; a refused call carries none.
- Antigravity takes them through its `PreInvocation` hook, which agy runs before every model call of a turn: a call after the prompt's first is the gap after a tool, and Hide answers it with the letters due as an injected system message (`injectSteps`, as the prompt's letters go), confirmed once printed; agy has no bell, so this and the next prompt are its two paths.

The ask (`Command::Pull { point: Tool }`, node protocol 47) names the pane's own native session and is refused with `session_mismatch` for any other, since it hands letters over; it submits nothing, so a draft in the composer stays a draft (D-09).
Grok also asks at its root session's `Stop`, as `hide inbox --hook --session <id> --turn-end` does.
A nonempty answer becomes `{"decision":"block","reason":<letters>}`, followed by confirmation after stdout flush; Grok feeds the reason to the model and continues the turn.
A second Stop with `stopHookActive: true` makes no letter pull and ends normally even if confirmation of the first handover failed.
Tool and turn-end intake share the three-handover limit and the prompt's bundle and 8 KiB cap ([One bundle per handover](#one-bundle-per-handover)).
Parallel tool hooks in one batch may pull the same claimed letter before either confirms it: the claim binds the bell turn, not an exclusive tool invocation.
The worker serializes pulls and durably counts each handover, so this at-least-once path remains bounded at three; confirmation makes later pulls empty.
An operator turn's new letter still waits five minutes unless sent with `--now`; ending that turn does not skip the wait, and its rest bell brings the letter instead.
The core answers from the caller's own observation: the letters whose wait is over for an agent whose turn works, as the core decides it for the engine's current state, as one bundle under the prompt pull's 8 KiB cap, and nothing for a resting agent without a bell-turn claim, a letter still waiting, a letter past its deadline or a kind whose adapter declares no `tool_letters`; a letter it hands over is confirmed by the script, not received, so one lost in the host stays pending for the next ask (D-11).
That turn is the one the sync pass laid in the step that made the observation ([Safe intake and manual fallback](#safe-intake-and-manual-fallback)), so an ask that lands while a pass runs, as a `Stop` hook does the moment the agent stops on screen, which is when the engine's state change starts the next pass, reads the last pass's turn or the new one and never none.
The plugin and the extension ask at every tool result of the pane's root session, so a `--now` letter goes at the next result (B6), and skip only the results that come while letters they handed over wait for their receipt; Grok's and Cursor's hook runs per tool call and adds one CLI call there, and agy's per model call after the first.
Each handover the core answers is counted on the letter (`Letter.tool_handovers`), and after three without a receipt the letter is not handed over at a tool again: it waits for the next prompt or `hide inbox`, as a letter does after its three bells (`mailbox::TOOL_HANDOVERS`, D-11).
[PERFORMANCE_TESTING.md](PERFORMANCE_TESTING.md#letters-at-a-tool-event) owns the cost per tool call and how it is measured, and [agent-hooks.md](agent-hooks.md) each script's and hook's part.
The pending retry, the oldest-first order with the count of letters waiting, the three bell attempts, the three tool handovers above and the 60-minute deadline are the whole of it; a letter a working agent never reaches a gap for is belled or handed over once it rests, as before.
A letter whose three bells are spent with none taken ends `undelivered` at once instead of at that deadline ([below](#letters-while-the-agent-works)).
The verdict's reasons are `working`, `blocked`, `awaiting_operator`, `session_unread`, `draft`, `quiet_period`, `kind_not_belled`, `session_changed`, `pane_unavailable`, `status_not_at_rest`, `not_ready` (the engine's blocked prompt under a turn that shows none), `turn_unprojected` (a pane whose pass laid no turn for it, as a pane move draws it), `announced` (a bell already typed in this turn names the letter: at rest it named every waiting letter, and mid-turn the next bell waits until the agent has taken the one it was typed for) and `core_pending` (a core on a move's copy types into no pane until the move's link commits it, [ARCHITECTURE.md](ARCHITECTURE.md#moving-the-core)).
Between the verdict and the input the doorbell asks the engine again and checks its own memory last, and each refusal there has its own reason: `launch_pending` and `not_ready` (the engine's readiness), `identity_changed` (the engine's agent in the pane has another name, kind or native session), `sequence_moved` (the engine's status or state sequence moved), `input_after_verdict` (hide routed input to the pane: its `input_generation` moved, the count a `hide pane send` fences its writes with too) and `letter_changed` (the letter was confirmed, cancelled, expired or reserved meanwhile).
A letter refused there is tried again in the same pane episode after 5 seconds, then 10, 20, 40 and 80, then every two minutes, because readiness and the letter are facts the pane episode does not carry; a change of the pane's turn, sequence, input or session tries it at once.
A pane episode is the core's turn and when it last moved, the engine's state sequence, the input hide routed and the session (`doorbell::Episode`), so a turn the hook alone starts or ends is a new episode while the engine's status stays the same.
A bell that failed before the engine was asked to type it (a failed read of the agent, or a reservation the delivery store refused or did not answer within 5 seconds) typed nothing, and is tried again on the same schedule; each try spends one of the letter's three bells once its reservation is saved, and `doorbell.reservation_failed` records the store's own code (`delivery_timeout`, `capacity`, `delivery_unavailable` or its refusal).
A resting agent's pane has no reason to move, so a letter that waited for one after such a failure stayed pending until its deadline (with-hide/hide-private#307).
A refusal from the engine typed nothing, whoever refused: the agent's prompt operation, or a device's node for a pane it has no session for (`pane_input_unavailable`) or input it could not hold (`pane_input_limit`, `pane_input_cancelled`, `node_busy`), so it is tried again on the same schedule ([The input ladder](#the-input-ladder)).
A bell whose answer was lost (it never came, or could not be read) may already have been typed, so it is not typed again on that schedule; `doorbell.failed` says which of the two it was (`maybe_typed`), and the letter keeps the failure (`Letter.bell_failure`: its code, whether it may have been typed, and when).
A later failure that typed nothing, such as a refused Enter, keeps that `maybe_typed` and its time, since the line it may have typed is still where it was.
The agent's own record takes such a bell within the record wait it gives any typed input (`typed::RECORD_WAIT_MS`, 15 s), so the next try comes then and reads the agent's composer first (`composer::shows`, logged `doorbell.composer_after_lost`).
The bell's line and nothing else there gets its Enter and is not typed again (`doorbell.entered`); an empty composer is rung as any letter; anything else, the operator's words among it, is never typed into or submitted and holds as a `draft`; a screen that cannot be read holds as `composer_unconfirmed`.
The Enter spends a bell as a typing does, so a resting agent that never moves neither waits for the deadline nor gets the line twice (with-hide/hide#2180).
If the pane or its device's link goes away after a failed bell (`pane_unavailable`), the wait ends: a bell that started a turn would have moved the pane, so a pane that comes back idle at the same state sequence is tried again rather than waiting for a move that never comes.
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
The reservation is persisted before input, and the adapter repeats the native inspection (`Engine::delivery_agent`: readiness, kind, session, status, sequence) after the persistence wait.
A refusal before the reservation spends none, so a letter the engine keeps refusing is never charged an attempt.
A crash or changed pane after reservation may consume an attempt while leaving the letter pending.
The post-reservation check takes its verdict under the runtime lock and releases that lock before recording a refusal through the delivery writer, so input arriving during persistence cannot block the mailbox behind its own receipt.
Legacy records with a successful bell but no total count conservatively have no automatic attempts left; manual and prompt-hook intake remain available.

The `UserPromptSubmit` hook of a bell target reads the submitted prompt from its input payload and sends the core the SHA-256 of its canonical text (`hide inbox --hook --prompt-digest <sha256>`), never the text itself.
The core alone knows which line its doorbell typed, so it alone decides whether the prompt was the bell ([Recognizing the bell](#recognizing-the-bell)).
When it was, the pull takes the pending letters as one bundle ([One bundle per handover](#one-bundle-per-handover)), each sender's pending letters before the letters an earlier build acknowledged without a receipt (`hook_confirmed: false`), which the store's expiry pass before every request drops once their deadline passes, so such a backlog never displaces the letter the bell rang for; it emits their context, flushes stdout, then confirms those IDs.
The pull marks the pending letters it hands over as handed (`Letter.handed`), saved before it answers, and no bell is typed for a handed letter again, whether or not the confirmation arrives: the confirmation is a second hook request inside the hook's last 400 ms (`CONFIRM_RESERVE`), which a loaded machine can miss, and on 2026-11-01 one such miss, when the confirmation was still a second `hide` process, had the doorbell type the same letter three times (letter-19680, load 110 to 140).
The hook starts no `hide` to confirm, on any system: on Windows a `hide` start took one second warm and 18 to 25 seconds the first time a new copy ran, inside a call no deadline ends, and Codex dropped the hook's whole answer at its eight-second limit (with-hide/hide-private#663).
The confirmation is taken only from the letters' recipient, its own pane and session, as `hide inbox --confirm`'s is (`mailbox::recipient_of_all`); a bundle whose ids do not fit one hook request is confirmed in as many as they take, each whole.
A handed letter that no confirmation took in is confirmed once its pane rests after the handover ([A confirmation that missed its hook](#a-confirmation-that-missed-its-hook)), or rides the next bell's batch for the same recipient, a tool or turn-end gap where the adapter has one, or `hide inbox`; it is not counted in the waiting line below and its sender is never told it was not taken ([Input an agent did not take](#input-an-agent-did-not-take)).
Only that pull marks a letter handed: a hook with no context to print (Grok's claim below) hands nothing over at the prompt, and a session file that records the bell line shows only that the line was submitted.
At its deadline a handed letter that no confirmation took in ends `delivered` with `hook_confirmed` null, which its sender reads as probably received, and still waits for its answer; a handed report ends its parent's watch then, as its confirmation would have.
It is never `undelivered`, so neither its sender nor the operator is told it failed, and a confirmation that comes later still records the receipt; a letter whose recipient's wake failed is `undelivered` whether or not it was handed ([status-model.md](status-model.md)).
An adapter whose prompt hook cannot add context, today Grok, instead records the pending batch under the native session and the saved bell digest, returning no context or count.
The first tool or turn-end pull in that session can hand this batch over immediately, without the five-minute wait.
Every own-session prompt replaces that claim, so a later operator prompt cannot inherit the previous bell's letters and a letter arriving after the bell submission cannot join its batch.
A prompt hook that can add context, on any other prompt including the operator's own, adds at most one line, `Hide 편지 N통 대기 중, 이 턴이 끝난 뒤 전달`, counting the letters a bell will still bring, and confirms nothing; the prompt text is never changed and no letter body reaches an operator's turn.
A letter whose three bells are spent with none taken does not wait for its deadline: once no bell can bring it, because none was typed and the agent rests, or because the agent rested the record wait since the last one that was or may have been typed, recorded none, and a fresh read of its session found none, it ends `undelivered` as a letter whose recipient's wake failed does (`doorbell.spent`, `hide-core/src/runtime/spent_bells.rs`).
It is judged only once the result of its last bell is recorded: the doorbell rings a letter one bell at a time and records how each ended, typed, failed, or held after its reservation, before it reserves the next, and the letter counts those results (`Letter.bell_ended`), so a third bell still being typed is never cut off.
A bell left on its way, by a core that stopped or crashed or by a result the store did not take, is ended at the start of the doorbell's next pass, before it rings anything (`doorbell.interrupted`), and that pass rings nothing for its letter: it is recorded as a bell that may have been typed (`bell_interrupted`), so the next pass reads the composer for its line before any bell, and a letter with none left is judged as above.
Its sender gets one `progress` letter with `--now` from `hide:input` under the intent `letter-undelivered:<id>`, so a restart that sends it again converges on the first: it names the recipient, the pane its bells were rung in, why (the last bell's failure code, `not_recorded`, or `hook_not_taken`) and what to do now, read the pane with `hide pane read <pane>` and send the letter again with a new `--intent` once it shows its input box, or give the work to another agent.
A bell line the agent's session recorded without any prompt hook of Hide's handing the letter with it (`hook_not_taken`) is judged too, once its bells are spent: the agent read the bell and not the letter, so its sender is told to give the work to another agent, since a letter sent there again ends the same way until the operator fixes Hide's hooks in that pane.
It is the only letter the sender gets for the last bell: the unconfirmed-input notice of that bell is not sent beside it (`Runtime::spent_bells_tell`).
A recipient whose record Hide cannot hear (hooks not connected, session unread) is never judged so: its letter stays pending for `hide inbox` and its deadline, and a handed letter is never `undelivered` (above).
A payload that is truncated, unreadable or not read within 0.5 seconds counts as an operator prompt.
The pulls carry the payload's `session_id` (`sessionId` for Grok); the core takes the pull as proof the pane's composer was sent only when that session is the pane's own native session, so a stray `hide inbox --hook` from another tool in the pane clears no draft.
The session id is not a secret, so this guards against accidents and not against a hostile process in the pane (external input, D-18).
A hook that runs while the engine already reports the pane `working` is a queued prompt being taken up and clears nothing, and a session longer than 4096 bytes (`SESSION_LIMIT`; Pi and omp name a session by its file path), starting with `-` or holding a control character is refused before it is hashed.
A device kit of an earlier build sends a `bell` flag and no digest: the core drops the flag (`Command::from_wire`), so that kit's bell turn gets only the count line and the letter stays pending until the kit is updated, while its OpenCode, Pi or omp pane still takes its letters on the next prompt; keep the kit and the app on the same build.
A foreign hook, including Claude Code's hook loaded inside Grok, receives nothing and confirms nothing.
Claude Code's hook run inside OpenCode, through an operator plugin that bridges `~/.claude/settings.json`, ends silently as a whole, so OpenCode's letters and counts are taken once, by Hide's own plugin.

An agent with a prompt hook that no bell rings for, today OpenCode through Hide's plugin and Pi and omp through Hide's extension, takes its letters on its next prompt, whoever typed it, since no turn of Hide's own will ever bring them (`mailbox::next_prompt_letters`).
Its pull (`hide inbox --hook --session`) hands over the letter bodies as a bell pull does and confirms nothing: the plugin attaches them to the root session's prompt as a synthetic part, and only when OpenCode reports that part written (`message.part.updated` with the part's id) does it have the helper confirm them, by the helper's own hook request.
OpenCode's `chat.message` runs before the prompt is stored, so a prompt OpenCode loses before that leaves its letters pending for the next prompt or `hide inbox`.
OpenCode 2's plugin asks for them from the terminal that shows the session, writes them as a synthetic message OpenCode admits ahead of the prompt, and confirms them once `session.inbox.delivered` names that message ([agent-hooks.md](agent-hooks.md#opencode-2)).
A subagent's prompt and a prompt of only synthetic text pull nothing.
The plugin forgets a pull it never saw stored after 10 minutes and keeps at most 16 waiting, so a stored part reported late can still confirm; past that the letter stays pending and comes again.
Pi's and omp's extension attaches them as a hidden custom message (`display: false`) that `before_agent_start` returns for the pane's own session, and has the helper confirm them, by its own hook request, once the host has written that message and then the reply after it; omp prepares a submission more than once, so each attempt's message carries its own id and only the written one confirms, once.
A turn that ends before its reply leaves the letters pending, and an omp subagent's prompt pulls nothing.
An agent with no prompt hook reads letters with `hide inbox`, which shows the same bundle and one `ack_command` for the whole of it, and that acknowledgement, or `hide request ack` for one letter, is its receipt, as it is for an agent with one: it records `hook_confirmed` and ends a matching report watch, as the flushed hook confirmation does.
Transport arrival and the doorbell alone do not confirm intake.
Interruption before confirmation can repeat the same letter ID; confirmed letters do not appear again in hook context.
The hook emits one bundle of at most 8 KiB of context, with a remaining-count line and `hide inbox` guidance when letters are left out.
A large letter is truncated at a UTF-8 boundary and includes its ID and `hide request show` command for the complete body.

#### A confirmation that missed its hook

A hook's confirmation is the only place the fact that its letters reached the agent is known, and the hook lives only as long as the agent waits for it, so a confirmation that does not reach the core in time is kept on the pane's machine and the core asks that machine for it rather than waiting for another hook.
A command hook (Claude Code, Codex, Grok, Cursor, Antigravity), or the helper of OpenCode's, Pi's and omp's scripts, whose confirmation did not reach the core in time after its letters reached the agent (its deadline passed, or no daemon or connection answered: `Failure::transient`) keeps their ids and the session they reached in a record for its pane (`hide_agent_hooks::owed`, `delivery::confirm_or_owe`).
The doorbell asks the node of the pane's machine for that record (`Call::OwedConfirmations`, answered by `owed::take`, which forgets it) once the engine reports the pane stopped in a state it entered after the letter was handed (`Letter::handed_at_unix_ms`, the core's clock, against the time its state last moved), so the hook that handed the letter has ended and whatever it kept is there; a turn that began and ended between two reads counts, which a short turn on a device does, and a state the engine does not know is not a rest.
The core takes in each letter of the record's session that still waits for its receipt, which ends it `delivered` with its receipt and ends the watch a report ends (`Doorbell::settle_owed`, `mailbox::confirm_owed`), with no prompt after it.
A record the node handed over and the store did not take is kept by the doorbell and submitted again at its next pass, without asking the node again, since the node has forgotten it.
On 2026-11-08 a first letter to a resting Claude Code on a device whose helper had just been placed missed its confirmation this way and stayed pending: the first run of the `hide` the hook then started to confirm took longer than the 400 ms it had, and the turn end's hook, which then sent what was owed, missed its 2 seconds the same way; the confirmation is a hook request now (with-hide/hide-private#663), and the record keeps the misses a loaded machine or a slow link still has.
The same path serves this machine, through the core's own node, and every device, on every system, since the record and the node's answer are the same code there.
Only a pane with a letter a hook handed over unconfirmed, at its prompt (`Letter::handed`) or at a tool (`Letter::tool_handovers`), is asked, and about each letter once in each state the pane rests in: a node that answered is asked again only after the pane's state moves again, since a hook of a later turn can keep what it missed then; an ask the link did not carry or whose answer it lost (not connected, busy, unknown) is made again after the bell's retry delay (5 s doubling to 2 minutes), and an ask the node refused, as an older node of the same protocol that does not know it refuses it, is not made again (`delivery.owed_ask_refused`, with the link's failure code), and the node is asked off the runtime's lock on the doorbell's thread, never on the session-sync thread.
A letter its node refused, or answered nothing for in every state it was asked in, or a record of another session, which ended in the pane, waits as before: a later bell's batch, a tool or turn-end gap or `hide inbox` brings it, and at its deadline a handed letter ends delivered without a receipt.
The record is one small file per pane under the hooks' own records (`hide_kit::layout::owed_confirmations`, `owed-confirmations/` in the records folder of the daemon the hook asks, [agent-hooks.md](agent-hooks.md#where-the-hooks-keep-their-records), or an installation's own folder in it; a node resolves the records folder of the state folder and installation it serves and names only what sits below it, `hide_kit::layout::owed_confirmations_in`), holding one bundle of ids (at most 1024, 64 KiB read) and the session they reached.
A record older than a letter's deadline from its first miss (60 minutes, `DELIVERY_EXPIRY_MS`) is dropped unsent, at the node's ask or at any pane's next miss, since the core has ended its letters; a damaged record is dropped the same way, and one the system does not let the node read at that moment is left for a later ask.
At most 64 panes owe at once: past that a new miss is not kept and is recorded under the cause `owed_capacity`, and its letters end at their deadline as before.
A letter a script handed at a prompt is not marked handed, so no ask is made for it: it rides the next prompt again, as before, and its record is dropped at its deadline.

#### One bundle per handover

Every path that hands an agent its waiting letters gives them as one bundle, whatever their senders: the prompt hook, the tool-result and pre-tool handovers, the turn-end handover and `hide inbox` (`mailbox::bundle`, the one place it is decided, so every agent kind and system gets the same bundle).
It exists because one chatty sender used to fill every handover: on 2026-11-03 a mission's Conductor received 505 letters in about 13 hours, 143 of them progress letters from one worker, and read each stale checkpoint in full, five at a time, while 109 of them were still unread when that worker's next one arrived.

- The letters are grouped by sender, each sender's group where its first letter comes, the letters a bell rings for first.
- A `request` or `block` is always shown in full and never folded: a folded question would go unanswered until it expires and its sender would stay stuck.
- Of every other kind (`progress`, `report`, `reply` and the letters of Hide's own roles), each sender's newest three (`SHOWN_PER_SENDER`) are shown in full, and its older ones are folded into one line before them: the sender, how many in the interface language's words for the bell line's waiting count ([The bell line](#the-bell-line): `외 N통`, `and N more`, `另有 N 封`, `ほか N 通`), and `hide request show <id>` with the newest eight folded ids.
- A sender with three or fewer such letters is shown exactly as before: all in full, no fold line.
- A folded letter is neither deleted nor rewritten; it stays in the ledger, opens with `hide request show <id>`, and `hide inbox` lists every folded id under `folded`.

Taking the bundle takes every letter in it, folded ones included, with one confirmation: a hook's flushed confirmation as before (a confirmation, a hook's or `hide inbox --confirm`'s, takes at most 1024 ids, `BUNDLE_LETTERS`, which the open-letter limit keeps a bundle under), and for an agent with no prompt hook the one `ack_command` `hide inbox` prints, `hide inbox --ack <bundle>`.
The bundle token is the newest listed letter's id, `~`, and the first 12 hex digits of the SHA-256 of every pending letter the answer listed, shown or folded, in the ledger's order (`mailbox::bundle_token`), so it takes exactly the letters that answer carried: a letter that came after it stays pending, and a set below its newest that is not the one listed, such as one taken meanwhile by a hook, is refused `bundle_changed` with nothing taken, and `hide inbox` again prints the bundle as it is now.
A folded `report` still ends its parent's watch.
`hide inbox` gives the caller's own letters that went undelivered apart, under `returned`, so a letter coming back never reads as one received: each names the recipient that did not get it, and `bell_failure` why its last bell failed when one did.
The 8 KiB cap still holds: each sender's fold line goes before its letters, and a sender whose fold line no longer fits, or a letter the cap leaves out, is neither shown nor taken; it waits for the next handover with the remaining-count line, so a folded letter is never shown in full later because its newer letters were taken first.
When the questions and blocks alone pass the cap, the line and `hide inbox` guidance are what is left, as before.
A connected device whose kit is older than this build is not held up by bundles: its `hide` sends no `bundle` on a pull or `hide inbox` (`Command::Pull`, `Command::Inbox`, `HookCommand::LetterPull`), and the core answers it as before bundles (`mailbox::Bundling::Legacy`): oldest first, nothing folded, at most five letters a handover, which is all its hook confirms, and `hide inbox` with a `hide request ack` per letter, since that `hide` has no `--ack`.
So the operator can update the app before a device, and the device's letters keep flowing five at a time until its kit follows; the path goes once no kit older than this build can run.
The installed prompt hook supervises one guarded internal operation with a 1.85-second deadline inside the two-second caller budget, including Memory, filesystem, output and diagnostic work.
The internal operation requires positive owner proof and inherits the runtime payload and output streams; it confirms only after its output is successfully flushed.
Failure leaves actual intake unconfirmed and attempts a rate-limited private diagnostic inside that same budget, under the cause of the step that failed ([agent-hooks.md](agent-hooks.md#what-a-hook-records-when-it-fails)).
A blocked diagnostic store or output stream can prevent the diagnostic from finishing; the outer timeout performs no filesystem or output tail that could hold submission open.
CLI output collection uses the canonical platform capture, with a 64 KiB bound and no reader thread or blocking join after timeout.
Cleanup uncertainty is a separate private diagnostic field and never authorizes confirmation.
Manual `hide inbox` and `hide request show` remain available when the hook is missing or fails.
OpenCode's plugin and Pi's and omp's extension hold the same 1.85-second prompt budget around their helper call, which itself stops at 1.65 seconds so the process start fits; past the budget the prompt goes on unchanged and the cause lands in the same private diagnostic (cause `plugin` for helper calls the plugin skipped or gave up on).

The engine has no atomic composer guard.
Hide checks the occupant before and after the durable reservation and checks its memory state immediately before the off-lock pane write, but external input to the pane (another terminal attached to it, for one) can race that final write.
That residual limit is the approved D-18 boundary; external input is not represented as a Hide key event.
In Grok another terminal's or another tool's keys can open `/model`, `/resume` or history without Hide knowing: a bell there can be swallowed as a model name or search.
The same picker opened through Hide leaves an unsent draft and holds the bell.
A background workflow approval card on Grok's daemon route is not yet verified: the measured 1.0.62 session used the shell route, where `await_user` produced an idle, paused workflow rather than that card.
Until a native `blocked` status or a session wait is measured for that card, do not treat it as a proven bell hold; this remains a native-route coverage gap.
A swallowed bell moves no observed state, so `Rung` prevents another ring in that episode; at most three later episodes ring before the letter waits for `hide inbox`.

### Forced letters

`hide request send … --force` puts a letter into the recipient's input now, even while it works (PRD engine-bundle-1 D-5, D-14, B4); there is no separate prompt command, and the letter is recorded, answered and retried like any other.
It is how an agent hands another agent input, and the letter's record keeps the relationship between the two.

1. Before anything is recorded, the daemon reads the recipient's visible screen off the runtime lock and judges it by the agent's composer declaration ([Reading the composer](#reading-the-composer)), for any version of the agent (`delivery/composer.rs`, `forced_clear`).
   Text there refuses the send with `composer_not_empty`, and a screen that cannot be read empty (no declaration for the kind, no composer where it is declared, as under a menu or a dialog, or a failed or cut read) with `composer_unconfirmed`; nothing is recorded or typed, each refusal is logged as `delivery.forced_refused` with the pane and the code, and its `next_action` says to send again without `--force`.
   A recipient Hide put to sleep has no input to read and is refused `force_target_asleep`; an ordinary `request` or `block` wakes it.
2. The letter is then recorded as one sent with `--now` and marked `forced` (`Letter.forced`), and the doorbell's next pass, within 250 ms, judges it without the kind's bell declaration, the mid-turn wait, an unsettled session read, Hide's draft record and the quiet period (`doorbell::judge`); it still waits while the engine reads the agent `blocked`, while the agent's own question or plan approval waits for the operator, and, for a kind that takes letters at its tools, while it works, since its next tool call hands the letter over.
3. Right before it types the bell line, the doorbell reads the composer again, inside its eight engine calls a pass, because someone can start typing between the send and the bell.
   Text there, or a screen not read empty, types nothing: the letter loses its `forced` mark and stays pending as a `--now` letter behind every ordinary guard, and `delivery.forced_deferred` is logged with the letter, the pane and the reason, never the screen's text.
   Until the next pass the letter is held as `draft` for text and `composer_unconfirmed` for an unread screen.

The bell line names `hide inbox` for a kind the doorbell does not ring on its own, whose prompt hook then takes the letter as it takes any prompt's.
The residual is the [external-input race](#safe-intake-and-manual-fallback) every bell has: a key that lands between the second read and the bell's submission.
On the core's own engine that race is closed from the submission on: the bell is typed as a letter (`PaneHolders::letter`), and the operator's keys typed into the pane from the line until its Enter was handed over are held, 64 KiB and 3 s at most, and written after the Enter in the order typed, so none lands inside the line and none is lost (PRD engine-bundle-3-core-owns D-13, B31); text already in the composer still refuses or defers the letter on both engines as above, and Hide never clears it.
On either engine a typed letter counts as taken only once the agent's own record shows it, and one the agent did not take is handled as [Input an agent did not take](#input-an-agent-did-not-take) says, never shown as delivered.

### The bell line

The doorbell submits one line through the engine's prompt operation: `🦉 <writer> <kind>: <the letter's first line>`, followed by how many more letters wait for the same recipient when there are any, and ` · hide inbox` for a Codex or Grok recipient (the adapter's `bell_names_inbox` declaration):

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
The line's parts (the mark, each language's kind words and waiting-letter phrase, the inbox tail and the name and summary limits) are named once in `hide-session/src/bell_line.rs`, which the bell writes from and whose transcript readers tell the line from the operator's own words by its whole shape (`is_bell_line`), so Project Memory and a session's link facts never take a letter's wake for the operator's request (`docs/project-sessions-and-memory.md` in with-hide/hide-private).

Whether the line names `hide inbox` is decided per runtime (`bell::names_inbox`), by whether its bell turn can arrive without Hide's hooks having run in the pane.
Claude Code runs every user-level hook it is given, so its prompt hook hands the turn the letter and its session guidance has already said what `🦉` means; its line ends at the summary.
Codex runs a user-level hook only once its own trust review approved that exact entry, so another tool's change to `hooks.json` silently stops Hide's, and a Codex attached to its shared app-server daemon runs hooks in the daemon's environment rather than the pane's ([agent-hooks.md](agent-hooks.md)); its bell turn can then arrive with neither the letter nor the guidance, so its line ends with ` · hide inbox`.
Grok's prompt hook observes the submission without adding letter context, so its line names `hide inbox` too; tool or turn-end intake still confirms the letters.

The line is submitted to the checked pane through the engine's prompt operation (`prompt_agent`) on every system, so the engine owns how each agent's input reader finishes a paste and submits it: the text goes in as one paste when the program has turned bracketed paste on, then Enter follows 300 ms later (`PROMPT_SUBMIT_DELAY`).
Hide adds no cursor keys or submission delay of its own.
A manually started agent is eligible once the engine detects it; it needs no start of its own, and the engine refuses a blocked or still-starting one (`agent_blocked`, `agent_not_ready`) before it writes anything.
On macOS, Herdr 0.9.5 accepted `/help` from Claude Code 2.1.302 and Codex CLI 0.163.0 through six launch paths each: direct binary, the journey's exec launcher, a shell alias, a shell function, `npx --no-install --call`, and managed `agent.start`.
The manually started agents were accepted with `unknown` status and no readiness report once detected; npx calls before detection returned `agent_not_found` and both were accepted after detection.
This measured prompt admission for those two CLIs on macOS under Herdr, not letter intake or other agents and systems.
The request asks for no turn wait and allows five seconds for the engine's ordered PTY writes to finish, outside the runtime lock; success is only a bell attempt, and the prompt hook still has to confirm intake.
The existing draft, identity and readiness checks, three-attempt limit and retry policy remain in force, and arbitrary phone replies retain their own submission contract.
The line carries nothing a composer acts on while it is typed.
Control characters become spaces, characters that draw nothing or reorder text are dropped (`display_text::one_line`), and runs of whitespace become one space.
`@` opens the file picker in Claude Code and Codex and `$` opens Codex's skill picker, and Enter would accept the picker's choice instead of submitting, so both are typed as their full-width forms `＠` and `＄`; a backslash before Enter continues the line in Claude Code, so `\` is typed as `＼`.
The sender and the summary carry no character above U+FFFF and no `·`: Codex CLI on Windows drops the former as it receives the line, so they are left out before the spaces are folded and its transcript holds the typed line less only the owl, and `·` is typed as `∙`, so the line's own ` · ` only ever opens the waiting-letter or inbox tail; a sender with nothing left to show is written as `…`.
This makes the line's shape unambiguous for the transcript readers (`hide-session/src/bell_line.rs`) rather than leaving them to guess; a line an earlier build typed can still carry either, and one whose summary was only an emoji or began with `· ` may read as the operator's.
`/`, `!` and `#` act only as a prompt's first character, and the line begins with `🦉`.

### Recognizing the bell

The reservation that spends one of a letter's three bells saves the exact line on the letter (`bell_line`) before it is typed.
A prompt hook's pull is the bell's exactly when its digest is that of the saved line of a `pending` letter whose recipient is the caller's own pane and native session (`mailbox::rang`).
The core and hook share `hide_agent_hooks::delivery::prompt_digest`: a prompt is first read as the words typed and pasted, in order (`hide_session::pasted::typed_text`, the wrapper Claude Code records, [Input an agent did not take](#input-an-agent-did-not-take)), then every line break (CR, LF, U+2028 and U+2029) and every character above U+FFFF is removed, a tab is read as four spaces, Unicode `White_Space` is trimmed from its ends, and its SHA-256 is compared exactly, with no agent or system branch.
The core reads the three texts it compares by that one rule: the prompt a hook reports, a person's message in a session file, and the text Hide typed.
The agent journey's own bell check (`scripts/agent_journey/cells.py`, `bell_turn`) applies the line-break, U+FFFF and trim steps and not the paste reading, because a bell is one line far below the size Claude Code wraps.
Line breaks identify nothing: a terminal paste sends each as CR, an agent records LF, and Grok on Windows takes a pasted brief without them (the fake-model agent journey, 2026-11-01), while a bell is one line.
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

When the doorbell's verdict for a pending letter, or for the operator's answer to a mission's decision, is `draft` and input is quiet (the engine reports it `idle`, `done` or `working` and current, with no key and no change of status for the 30-second quiet period), the doorbell asks the engine once for the pane's visible screen with its styling (`Engine::read_screen`).
The read runs on the doorbell's own thread, off `Mutex<Runtime>` and never on the session-sync thread, at most once per observed draft and state generation (`delivery/composer.rs`), and counts against the doorbell's eight engine calls a pass; a cut screen is a failed read.
The screen is judged by the agent's composer declaration (below).
Only a positive `empty` releases the draft, as [Deliver now](#delivering-a-held-letter-now) does: the input up to the read is no draft (`released_input_at_unix_ms`), the composer mark goes, and the doorbell's next pass rings with every guard it always applies.
A key, even with the same timestamp, a session change or a change of status or state sequence between the read and the release keeps the hold (`composer.release_refused`).
A working pane uses the same positive-empty decision as a resting pane; version calibration still runs only after a known-empty turn has ended.
`not_empty`, `not_found`, a failed or timed-out read, an agent with no declaration and a version not checked keep today's hold, with the waiting mark after two minutes.
Each read is logged as `composer.read` with the pane, agent, version, verdict and, for a screen not read empty, the parser step that stopped it (`reason`, below), and each draft not read as `composer.not_read` with its reason (`kind_undeclared`, `version_unknown`, `version_unchecked`, `version_unconfirmed`) once per draft; no record carries screen text.

Each agent's adapter declares where its composer sits and what an empty one shows (`hide_agent_adapter::composer`, the `composer` field of each row in `declarations.rs`), and one parser judges every agent.
Empty is a positive signature only: nothing at all, or the agent's known placeholder in its known style; never "anything faint", since Grok draws a paste in grey.
A screen with no composer where the declaration puts it is `not_found`.
A screen not read empty names the step that stopped it (`composer::Miss`): `no_rule` or `no_top_rule` (no rule, or none above the bottom one), `no_region` (no box, bar or marked line), `no_marker`, `no_trailer`, `placeholder_style` (a placeholder's text in another style, which for Claude Code is any text not dim) or `not_placeholder` (text that is no placeholder).

| Agent | Composer | Empty |
| --- | --- | --- |
| Claude Code | the `❯` line and any lines under it, between the last `─` rule and the rule above it, which may carry the session's name | nothing, or only text in the dim attribute (SGR 2), since its suggestions such as `Try "…"` change |
| Codex | the `›` line and its indented lines down to a blank line | only the dim `Ask Codex to do anything` |
| Grok | the `❯` line inside the last `╭`/`╰` box, between its `│` borders; `>` for `❯` on a console Grok takes for Windows' legacy one, as a pane on Windows is | nothing, or only italic text (SGR 3): the next prompt it predicts after a turn, whose words change; typed text and the grey `[Pasted: …]` chip are never italic |
| OpenCode | the lines inside the `┃` bar above its `╹` foot, less the agent and model line | only the (128,128,128) `Ask anything…` placeholder; typed text is (238,238,238) and a paste an orange chip |
| Pi | the lines between the last two `─` rules | the rules adjacent |
| omp | the `╰─` line | nothing, or the `⇧⇥ to change thinking effort` hint; a paste shows `📄 #1` |
| Antigravity | the `>` line between the last two `─` rules | nothing after the marker |

A paste of many lines that an agent keeps as one mark is declared with the mark's words and how it counts the lines (`Composer::pasted`, `PasteMark`), so a brief Hide typed is read as still in the composer when the composer shows that mark alone with the brief's count: Claude Code `[Pasted text #1 +11 lines]` for twelve lines (it counts line breaks), Antigravity `[Pasted text #1 +12 lines]`, OpenCode `[Pasted ~12 lines]`, Pi `[paste #1 +12 lines]` and Grok `[Pasted: 12 lines]` (they count lines), from the same captures.
Codex shows a paste as its text, and omp's `📄 #1` carries no count that ties it to the text, so neither declares one.

The declarations were measured on 2026-10-21 under Herdr 0.9.5 at 60 columns (Claude Code 2.1.306, Codex 0.163.0, Grok 1.0.62, OpenCode 2.0.29, Pi 1.0.4, omp 18.7.0, Antigravity 1.4.4): type `hello 안녕하세요`, Backspace it all, paste 12 lines, Backspace twice, clear; `hide-agent-adapter/tests/it/composer.rs` judges those captures.
A rule above the composer may carry a label: Claude Code 2.1.314 draws a session's own name, given with `claude --name` or `/rename`, in its top rule as `──── name ─`, drops the run of `─` before a name too long for the pane (` long na… ─`), and after `/color` draws the `─` in that colour and the name in black on it; the engine's and Hide's agent names do not reach it, nor does the title Claude Code makes up for a session.
So a top rule is either a line of `─` alone or a label between a space and a closing run of `─`, every `─` of it drawn in the bottom rule's own colour (a palette or 24-bit one), which typed text never is; what lies between the rules is judged as before.
Measured on 2026-10-30 under Herdr 0.9.5 at 60 columns, the six steps above and the empty composer after a turn, for a named and an unnamed session, plus the coloured and cut names (`claude-2.1.314-*` captures); agy 1.4.11 draws nothing in its rule after `/rename`, and Pi 1.0.4 writes in its top rule only while it works or when a draft overflows (`↑ 3 more`), never over an empty composer at rest.
Grok's predicted next prompt and its Windows marker were measured on 2026-10-22 with Grok 1.0.52, the pinned version, after a turn ended (the earlier captures had none), on its own and with `GROK_FORCE_LEGACY_CONSOLE=1`, the console a Windows pane gets; its source draws the suggestion with `ghost_text_style`, dim and italic, on every system.
Only Claude Code and Codex are rung, so only their drafts are released today; the other declarations serve the agent journey's `composer` cell and a bell target added later.
Antigravity's declaration also clears the composer of an agy fork's pane before Hide types `/fork` there ([UI_BEHAVIOR.md](UI_BEHAVIOR.md), Antigravity's Fork): it is read for any agy version, since a screen it cannot confirm empty is typed into as before and logged as `fork.composer_unconfirmed`, so a declaration that no longer matches agy never stops a fork.
[INSTALL.md](INSTALL.md#what-each-system-supports) records what has not been measured (narrow panes, the light theme, Windows and Linux).

#### Catching an agent's screen change

An agent update can move or restyle its composer under the declaration, so a release is allowed only for an agent version whose declaration was seen to work.
Hide knows a version from the hook diagnosis of the machine the pane is on, which reads the installed runtime's `--version` there (`hide_agent_hooks::runtime_compatibility`).
The core's own machine's is refreshed with the kit.
A device's helper answers the same `HookDiagnosis` call from its own account and its own program search, the same on macOS, Linux and Windows, and the device's coordinator asks it as soon as the device is connected and again a minute after each answer (`kit::DeviceDiagnosisReader`, a worker thread outside the runtime lock, never gated on a window being attached, since letters are delivered without one).
The core keeps the last answer for each registered device (`Runtime::ingest_device_hook_diagnosis`) and drops it with the device, so a pane on a device is read by the version its own machine has installed, never by the core's or another device's; a read the device cannot answer keeps the last one.
An agent with no hook runtime, a device not read yet, or a program that answers no version has no version Hide knows, and its drafts hold as before (`version_unknown`).
A version's check is kept for the agent and version and not for the machine, so a device whose Claude Code is a version that already passed on another machine reads by that check; the composer is read again right before every bell on any version (`clear_to_type`), so a screen that draws differently on one system holds the letter there rather than ringing over it (with-hide/hide-private#654).
At a moment a pane's composer is known empty (a turn that a prompt hook of the pane's own session started has ended, and no key came since), the doorbell reads the screen of a version not yet passed and judges it, once per pane and moment and at most one pane every five seconds.
Hide's own typing counts against that moment as a key does: a bell, a mission answer, a brief typed again or its Enter, and a 마무리 맡기기's words stand in the composer until the agent takes them, so the pane records when Hide last typed there (`Runtime::note_hide_typing`, under the lock before the typing) and a hook moment before it is no known-empty one.
Typing into a pane that has had no prompt hook yet is not recorded, since every hook moment there comes after it: a spawn's brief, a task start's first prompt, and agy's `/fork` with the composer clearing before it in the fork's new pane.
The next one is the end of the turn the typed input's own prompt hook starts; an input the agent never takes leaves the pane unchecked until a later hook, which only delays a check (with-hide/hide-private#600, where a check read the bell Hide had just typed into an idle Codex as `not_empty` and waited a minute).
The engine can report the turn `working` before its hook reaches the core; a hook within ten seconds of that, with no key since the turn began, still marks the turn, while a hook after a key typed during the work is a queued prompt and marks nothing, since the composer may hold a newer draft.
`empty` passes the version for good.
Anything else is a miss, not a verdict on the version: one pane can show a dialog, a named session's rule or another screen the declaration does not place while every other pane reads fine, and on 2026-10-30 one such check turned Claude Code 2.1.314's release off for every pane all day.
The version's drafts hold as before (`version_unconfirmed`), and it is checked again at a later known-empty moment, one minute after its first miss, the wait doubling with each further miss up to an hour, so a version whose screen did change costs one read an hour; a failed read is no miss and waits for the next moment.
Each check is logged as `composer.version_checked` with the agent, version, verdict, the step that stopped a miss (`reason`), the misses so far and the wait before the next check (`retry_after_ms`), and kept in `composer-checks.json` in the state directory (`hide_kit::layout::composer_checks`, written by `composer_checks.rs` on a thread of its own) with the misses and the last one's step, so the cause outlives the log's rotation; at most 128 versions are kept, the oldest dropped first.
A file an earlier build wrote, whose failed checks were final, reads as those versions not yet confirmed, checked at the next moment; a missing, damaged or other-version file is ignored with a `composer_checks.discarded` diagnostic, and every version is checked again.
The version is the installed program's, so a pane still running the program from before an update is judged by the newer version's check until it restarts.
The agent journey's `composer` cell checks every declaration nightly on each system lane ([VERIFICATION.md](VERIFICATION.md#agent-journey)).

### Delivering a held letter now

A draft is the one hold that does not end on its own: an idle pane whose composer may hold the operator's half-written prompt keeps its letters until its next real prompt, for up to the 60-minute deadline, unless its composer is read empty first ([Reading the composer](#reading-the-composer)).
So the core shows it, and no other hold reaches a pane.
A running mission's Conductor is the one recipient whose other holds reach the screen too, because every worker and checker of the mission waits on it: the holds the operator can lift (a pane or device Hide cannot reach, an approval the Conductor waits for, a draft, three bells unanswered) for a letter two minutes old or more, and a letter that ended undelivered until its sender gets a later one through, are counted on the mission's map and in its 내 차례 with what to do (`delivery::held`, read from the holds the doorbell publishes, never a verdict of its own; with-hide/hide-private#678).
A pane's letters are shown waiting when the doorbell's last reason for a pending letter there is `draft`, the pane still holds that draft, and the pane has rested for two minutes: no input hide routed and no change of the engine's status since (`doorbell::letters_waiting`, `WAITING_MARK_REST_MS`).
Every other hold (working, the quiet period, a launch, a menu) ends on its own and shows nothing.
The one exception to that is the operator's own answer to a mission's decision, which waits for the same holds with the operator looking at it: the banner says what it waits for (`AnswerView.hold`: `working`, `typing` for a draft or a composer not read empty, `recent` for the quiet period after the Conductor's work or the last input, `approval` for a menu the Conductor itself waits at, or the plain wait for every other hold) and, for all but `approval` and the plain wait, offers `지금 보내기`.
The press is a `mission_answer` with `now: true` for the question's key; it is accepted only while the doorbell holds that answer and once (`mission.answer_now`, a refusal is `mission.answer_now_refused` in the log), and the doorbell then judges the answer as a forced letter ([Forced letters](#forced-letters)): past the Conductor's work, the quiet period and Hide's draft record, with the composer read empty right before the typing, never over text on its screen.
When the composer is not read empty the press is dropped (`AnswerHold::Forced` goes back to the earlier reason) and the answer waits as before, so the screen is not read again every few seconds.
The press lives in memory like the hold does (`Missions::answers_forced`, kept only for an answer still waiting).
The count is every letter held there, which the bell turn brings together, and a pane whose held letters have all spent their three bells shows nothing, since pressing could ring none.
The core decides on each doorbell pass, under the short lock the pass already takes to publish its holds, and keeps the answer on the pane's agent row (`letters_waiting`), so the mark costs one publish when it appears and one when it goes, never one per pass; each change logs `letter_mark.shown` with the pane and count, or `letter_mark.cleared`.
A key typed into the pane restarts the two minutes, so the mark leaves while the operator works there.

Deliver now is one event, `deliver_letters_now {pane_id}`.
The press is the operator saying the composer holds nothing of theirs: the pane's observation records the input it releases (`released_input_at_unix_ms`), so input up to it is no draft, the pane's composer mark is removed, and the mark leaves at once.
The doorbell's next eligible pass then rings with every guard it always applies (the engine's readiness, the actual composer, the reservation, the last memory check) and types the same line ([The bell line](#the-bell-line)); the prompt hook takes the letters as it does for any bell turn.
The press releases the recorded draft hold only: actual text, a menu or an unreadable composer still holds the letter until the composer reads empty.
Nothing clears or submits the operator's draft.
Input after the press is a draft again and holds the letter as before.
A press the mark no longer stands for changes nothing and logs `deliver_now.refused` with `no_letter_waiting` (the doorbell has not shown it, or it left), `pane_unavailable`, or `pane_not_at_rest` (a key reached the pane or its status changed since the last pass); an accepted press logs `deliver_now.released` with the pane and count.
Neither reaches the screen, and nothing changes on the sender's side.
[UI_BEHAVIOR.md](UI_BEHAVIOR.md#agent-panes-and-the-agents-explorer) owns the pane header pill and the sidebar row's envelope that draw it.

### A menu the engine reads as a stop

Codex's plan approval, "Implement this plan?", reads to the engine as an ordinary stop rather than `blocked`, and a bell's Enter there picks its first item, "Yes, implement this plan", so a letter would start a plan the operator never approved (measured 2026-10-07 on Herdr 0.9.3, three runs of three, with the menu read as `done`).
What the engine reads depends on the screen rules of Codex's agent definition (`hide-agent-adapter/agents/codex.toml`), and the status guard no longer holds it: they read a Codex title without the working spinner as a stop when no blocking or working rule matches the screen (`osc_title_idle`), and a Codex screen no rule matches reads as a stop too.
The manifest bundled with Herdr 0.9.3 (2026.09.23.1) read the menu `unknown`, which the status guard held (measured 2026-10-07 with Codex CLI 0.160.1 on an isolated server).
Hide reads that wait from the Codex session file instead of the screen (PRD codex-plan-approval-hold).
The label worker's session read, which already runs on each change of the pane's engine state, on its own thread and from where it last stopped, folds Codex's turn records into an agent-neutral turn tracker (`hide-session/src/turns.rs`): `task_started` with its `collaboration_mode_kind`, the `Plan` item a plan-mode turn proposes (or its `<proposed_plan>` reply), `task_complete`, `turn_aborted`, and each person's message.
A plan-mode turn that proposed a plan and finished, with no later turn or message from a person, waits for the operator's approval; the next `task_started` or a person's message ends the wait.
Records the parser does not recognise are not known rather than "nothing waits": a `task_started` whose mode is missing or not one it knows, a plan proposed in a turn whose mode says it runs none, and a person's messages with no turn record at all.
The rule and its basis (codex-cli 0.160.1) are written beside the Codex parser, and nothing outside it names Codex.
The answer is kept with the engine's `state_change_seq` the read was asked under, and it holds for that state only.
A label worker that starts, with a daemon that starts or a device link that opens, runs on a new engine that counts its states from the start again, so it shows no answer it kept, and owes or settles no notice, until it has read each session once, continuing from where the last read stopped with the turns it had folded (`PaneRecord::forget_engine_state`).
The engine can read the agent at rest before Codex writes how the turn ended; that state's read then does not settle a plan-mode turn, so the bell holds and the row shows no wait until the engine's next state is read, because this rule adds no session reads of its own (PRD codex-plan-approval-hold B10).
The same turn tracker reads Claude Code's native `AskUserQuestion` calls and matching tool-result records.
When a supported session reader says a native question or plan approval waits, the bell holds with `awaiting_operator`.
This includes Codex 0.162.1's `request_user_input_async` question, which stays open after Codex returns to work or stops, until its reply arrives.
The hold is what keeps a wake from landing on it: in an isolated Codex 0.162.1 TUI (2026-10-15) the question is plain assistant text over an ordinary composer, and a bell-shaped line plus Enter was submitted as an ordinary prompt that started a new turn, writing no reply, so the question stayed unanswered in Codex while Hide would have read the person's message as the end of the wait.
For an agent whose read reports turns, a state that no read has settled holds the bell too, with `session_unread`: a session file not found (Codex is looked up in its seven newest day folders), a failed read, a device helper that predates the field, a read still behind the engine's newest state, a record past the line cap the read could not structure or more native questions waiting at once than the read keeps (each until the next turn starts or a person writes; `docs/ARCHITECTURE.md`, What the last turn waits for), or a daemon that does not hold the label generator lock and so reads nothing.
That lock is one per engine and taken by the node with `flock` (`hide_host::label_lock`, asked for by `hide-core/src/labels/generator.rs`): the operator's single daemon lacks it only while another hided follows the same engine (a candidate or development daemon pointed at the operator's engine folder, even with a private HOME), or, after a restart that overlapped the old daemon, until its next attempt, at most thirty seconds after the old one exits; the kernel frees a crashed holder's lock.
Unknown is never read as "nothing waits".
When the read refused for a reason reading again cannot clear (anything but a session file not found yet or one that changed during the read: a record over a cap, an owner that does not match, a helper too old), the hold would last until the letter expires, so Hide says so instead of only logging it: the sender of each pending letter held on it gets one `progress` letter from `hide:delivery` naming the recipient, the reader's reason and the `hide request send <recipient> --force` that types the letter now (intent `session-unreadable:<letter>`, so a restart that sends it again converges on the first; Hide's own roles and the operator get none), the recipient's pane wears the `session_unreadable` connection mark ([status-model.md](status-model.md#not-connected-and-what-fixes-it)), and a mission answer held on it shows `session_unreadable` on the banner ([Missions](#missions)).
The letter itself stays pending and rings as usual once a read lands; the refusal is kept per pane for the session it was read under, by the labels worker (`refuse_read`), and goes with another session or a read that lands (`hide-core/src/runtime/unreadable_letters.rs`).
Kinds whose session readers report no turns retain the other delivery guards; this does not enable a new bell target.
"No, stay in Plan mode" writes nothing to the session (measured with Codex CLI 0.160.1), so the wait, the hold and the row in Needs You last until Codex's next turn starts or a person's message is written.
The first start of a build with this rule resumes a stored session at its last person's message, after that turn's `task_started`, so a plan already waiting then is not known until the engine's next state; that start holds the bell rather than ringing it.
The rule reads the session the engine records for the pane, so a pane reported with another session is judged by that session.
It guards against hide's own bell, not against a process of the same account, which could append records to the session file or type into the pane with `hide pane send` or from another terminal attached to it.

Before typing a letter into an agent whose composer has a declaration, Hide also reads that composer and requires it to be empty, using the same parser as a forced send.
This protects text even when a late status update covered the key in the draft record.
A nonempty, unreadable or truncated screen keeps the letter pending without spending a bell; retry uses the existing bounded backoff, and a new key or state generation invalidates the read before any write.
This adds one off-lock screen read per eligible attempt within the doorbell's existing eight-attempt pass cap, with no added work per key.

### The input ladder

Hide types a running agent's brief or letter bell line through the engine's guarded prompt operation (`hide-core/src/delivery/ladder.rs`).
Each supported agent declares its input rungs once, in `hide-agent-adapter` (`AgentAdapter::input`): an optional launch prompt for the first input, then typing for an already running agent; the adapter's contract test fails a declaration that does not end in typing or repeats a rung.
Every route expands a tab to four spaces before handing the text to the agent (`hide_agent_hooks::delivery::input_text`), the same rule the launch argument and prompt identity use.
An editor's own tab width cannot then change the native record: omp 18.7.0's Windows editor replaced a pasted tab with three spaces, so a brief whose answer arrived still had no matching record.
The original brief and its digest stay unchanged; a three-space record still fails to match, and neither a successful send nor an answer alone confirms intake.

The first input a start brings, a spawn's brief, a task's first prompt or a promoted Memory card, follows one standard for every agent on every system:

1. By default it is typed once the agent's screen is drawn and takes input, the same for every agent and system, with no rule of its own for any of them.
   An agent takes input once its screen rests at least 3 s after its start (`launch_pending`, `engine/own/agents.rs`, `START_SETTLE`).
   The engine also waits until the agent's program reads keys, which its pane holder reads as the terminal's line editing turned off ([ARCHITECTURE.md](ARCHITECTURE.md#agents-on-the-cores-own-engine)): text typed before it waits in the terminal's line, whose Enter reaches the program as a line break, and Pi 1.0.4 on a Linux runner kept a brief typed 3 s after its start line in its editor, unsubmitted, with no session file written (nightly 41789009557).
   Once the program reads keys the 3 s start again, since OpenCode 2.0.29 dropped a brief typed in the second before its input box drew, but the 120 s start deadline does not: it runs from the start, so a program that keeps taking and giving back the keys ends its start there; and a program that never turns line editing off takes no letter past the deadline either.
   On Windows the holder reads the console's line input through a short-lived helper attached to the pane's console, never by attaching itself, so a console's close cannot end the holder and the agent with it.
   An agent whose definition names a ready screen (`[start] ready`) also waits until its screen has shown it: OpenCode 2.0.29 reads keys before its model loads and refused the Enter of a brief typed then, so its start waits for the model line under its composer.
   A question on its screen keeps it waiting for the operator's answer, since what is typed there answers it, and the answer starts the 3 s again, since the agent draws its next screen after it: Claude Code's folder trust question, whose first choice is "No, exit", took a brief typed into it on a Windows device as that answer and quit, and a brief typed at once after "Yes" was lost while Claude Code 2.1.318 drew its prompt.
2. It rides the start as the agent's launch prompt only in the cells the agent journey proves, an agent on a system, declared once: the agent's `input` names the launch prompt, the start runs on this machine, and the pane's node says the program takes its arguments whole (`hide-core` `start_prompt::launched`, the one decision every start asks): by its name on macOS and Linux, and by its file on Windows, npm's `.cmd` launcher read as the program it runs (below).
   A device's start is typed until a journey cell proves it, and so is a Windows start whose program is a batch file Hide cannot read as npm's launcher, or whose command line would pass Windows's 32,767 characters.
3. The daily agent canary runs those cells against each CLI's latest release ([VERIFICATION.md](VERIFICATION.md#the-agent-canary)); a cell that fails goes back to the default by changing its declaration, and nothing else changes.

[INSTALL.md](INSTALL.md#what-each-system-supports) shows each agent on each system as launch or typed, with the run that proves it.

A fresh start can carry its brief as a launch prompt.
Every subsequent brief, letter and operator answer uses the engine's prompt submission on the pane's owning machine (`delivery::ladder::deliver`).
The doorbell decides when; the input path submits once, off the runtime lock.

| Answer | Meaning | What follows |
| --- | --- | --- |
| Sent, `typed` | the engine acknowledged submission | the agent's own record decides intake |
| NotYet, `engine_parameters`, `engine_refused` or `engine_unreachable` | nothing was typed: the request could not be put to the engine, the engine refused, or its machine could not be reached; a refusal is the engine's word that it wrote nothing (`EngineError::Refused`, the node's `HolderError::Refused`), whether the agent's (`agent_blocked`, `agent_not_ready`) or a node's (`pane_input_unavailable`, `pane_input_limit`, `pane_input_cancelled`, `node_busy`) | the doorbell may retry; a brief fails with `brief.prompt_refused` |
| Lost, `engine_format` or `engine_call_failed` | an answer that never came or could not be read, so the submission may have arrived | never retry blindly; the agent's record decides, then the composer ([Letters while the agent works](#letters-while-the-agent-works)) |

The durable reservation still saves `lost` before sending and records the outcome afterwards.
A save that never lands therefore cannot make a possibly submitted input free to send again (engineering principle 11).
Only input saved as `typed` may be resubmitted once someone answered the agent's screen.
Existing `channel` records from earlier builds remain non-retryable: Codex can still consume an already queued message after Hide upgrades.
No new input is sent to `codex queue`, whose success means queued for a later turn and cannot satisfy delivery during ongoing work.
Each send logs `input.rung` with its route and pane identity, never the text.

| Agent | Rungs | What the agent's own rung is | macOS | Linux | Windows |
| --- | --- | --- | --- | --- | --- |
| Claude Code | launch prompt, typed | none Hide can use: its messaging socket holds a message for a session Hide starts (below) | yes | yes | yes |
| Codex | launch prompt, typed | running input is typed so it reaches the current turn after the running tool | yes | yes | yes |
| Antigravity | launch prompt, typed | none Hide can address: its language server takes a message only with a start flag of its own and a port read from its log | yes | yes | yes |
| Grok | launch prompt, typed | none declared | yes | yes | yes |
| OpenCode | typed | none: OpenCode 2.0.29 submits a start prompt (`--prompt=<text>`) as soon as its model list arrives, without waiting for the configuration that names the operator's model, so a start that loses that race sends the brief to the first model of the list, OpenCode's own free hosted one (below) | yes | yes | yes |
| omp | launch prompt, typed | none declared | yes | yes | yes |
| Pi | typed | none: Pi 1.0.4 holds a start prompt, but reads one that starts with `@` as a file to attach, `--` or not, and exits 1 when there is none, so its brief is typed rather than changed or split by its first character | yes | yes | yes |
| Cursor | typed | none declared | yes | yes | yes |

The launch prompt carries only a start's first input, as the agent's start argument, and only in the cells of the standard above, each an agent measured to hold it through its own start screens; every later input goes through the next rung.
Measured 2026-11-01 on macOS with an isolated HOME, a private terminal server and the fake model:

- Codex 0.166.0 holds a start prompt through its "Hooks need review" screen and submits it once that is answered; its prompt hook records the argument's U+2028 line breaks.
  A controlled multi-step turn on Codex 0.166.0 (2026-11-03) took typed input at its next model step before the turn completed; queued input waited for the whole turn to complete.
  This is why running input uses the engine's prompt submission, while an existing queued send is never repeated.
- Claude Code 2.1.316 holds a start prompt through its folder trust screen; its prompt hook records the argument's U+2028 as line breaks, so the brief's own form is its record.
  Its messaging socket (`CLAUDE_CODE_MESSAGING_SOCKET`, opened with the token its SessionStart hook receives) runs a message with the exact text in a session that asks for permissions, but holds it, unrun, in a session that bypasses permission prompts unless the sender attests a matching permission mode or the session sets `crossSessionInbound` (Claude Code 2.1.302 in the agent journey and 2.1.316 by hand, 2026-11-01).
  Hide starts its agents with full access, so a letter written there waited for review instead of running; Hide attests nothing it cannot prove and changes no setting of the operator's, so Claude Code's later input is typed until Claude Code offers a way in for such a session.
- Antigravity 1.4.12 holds `-i <prompt>` through its trust dialog and records the U+2028 form in its transcript.

Measured 2026-11-02 the same way, each CLI under a sandbox that refuses to open a browser, with prompts of Hide's own form that start with `-` or `@`, hold raw tabs and line breaks, or are 16,350 bytes long:

- Grok 1.0.70 holds `-- <prompt>` through its folder trust question and submits it once after "Yes" ("No, quit" exits with nothing submitted); the pinned 1.0.52 shows no trust question; its prompt hook records every form exactly, the 16 KiB one included, and a prompt that is a subcommand's name (`version`) stays a prompt.
- OpenCode 2.0.29 shows no screen at a new folder's start and records `--prompt=<prompt>` exactly in its session database, `@` included; `--prompt -x` as two words prints its help and exits 0, which a first prompt or a promoted Memory card that started with `-` met before this change too.
  But its terminal submits a start prompt once its model list and its recent models are read, not its configuration: the model a session takes is the start's `--model`, else the configuration's, else a recent one, else the first of the list, so a start whose configuration arrives after the list sends the prompt to that first model, OpenCode's own free hosted one where no policy hides it.
  The agent journey's first OpenCode start on macOS did (run 41415065150, 2026-11-02: "Build · Flint 40B Free" while its configuration named the journey's model, which the later starts of the same run took), so OpenCode's brief is typed, after its screen is drawn, and the journey's OpenCode configuration denies every provider but the fake one.
- omp 18.7.0 holds `-- <prompt>` through its five-step first-run setup and under its update notice, submits it once and records every form exactly, attaching the file an `@` names while keeping the text whole.
- Pi 1.0.4 holds `-- <prompt>` through every answer to its folder trust question and records it exactly, but `@README.md …` as one word is read as a file path and the start exits 1, `--` or not.
- No agent submitted a launched prompt twice, and each draws Hide's U+2028 line breaks as spaces, so a brief of many lines shows as one line in its screen while its record keeps the breaks.

A launched brief is never typed or typed again: the spawn records it as launched (`BriefLaunched`, `brief_input.route` `launch`) with the words its start adds to carry it (`launch_words`) before it starts the agent, so answering the start screen cannot send it twice, and it is taken once the agent records it.
Its record is owed from the moment the started agent takes input (`ladder::ready`, up to 30 s; `BriefLaunched` saved again moves `typed_at_unix_ms` there), as a typed brief's is from its typing, so a cold start of twenty seconds raises no notice; an agent not seen ready keeps the start's moment (`brief.launch_ready_unseen`).
A start typed again for the same spawn carries those saved words and asks nothing again, so a retry cannot start the agent without its brief while the parent is told the start holds it; a record an earlier build saved without its words is given them as that build gave them, with no machine asked.
The argument carries line breaks as U+2028 and a tab as four spaces (`start_prompt::prompt_argument`); Codex and agy record it as given and Claude Code with LF, and the one canonical rule the brief and every record are read by drops line breaks, U+2028 and U+2029 included, and reads a tab as four spaces ([Recognizing the bell](#recognizing-the-bell)), so each record is the brief.
Until then its child shows as waiting to its parent, and the parent's notice says the start holds the brief and submits it once its screen is answered.
What a start can carry is one rule for every start Hide makes with a prompt (`hide-core` `start_prompt::carried`): a spawn's brief, a task's first prompt and a promoted Memory card, which is a spawn whose brief is the card's prompt.
A start whose native arguments give a prompt already, or a prompt the argument refuses (a control character, past 64 KiB), carries none: the input is typed instead (`brief.launch_refused` for a brief), and a task whose first prompt the argument refuses fails before anything starts (`agent_start.invalid_prompt`) while the surface keeps the text.
Whether the agent is installed, and how its program starts, is asked of the pane's machine before every start (`Call::AgentInstalled`, answered with a `StartForm`), never kept, since an update of the agent can move what its launcher runs; a spawn here whose program the node does not find, a `.ps1` alone on Windows among them, which the pane's PowerShell still runs, starts by its name with its brief typed, as it did before the node was asked.
A task's first prompt that does not ride the start is typed once the agent takes input (`ladder::ready`, up to 30 s, then `ladder::typed_only`), with no retype and no notice, since the operator is looking at the tab they opened; an agent that took no input in time, a typing the engine refused, and one whose answer was lost and may have landed settle the start alike as unconfirmed with `agent_start.prompt_not_typed`, whose wording asks the operator to look at the agent's terminal and type the prompt only if it is not there, and start nothing again, since the agent runs.
On Windows a program that is a batch file runs through `cmd.exe`, which reads the start line again: past 8,191 characters it prints "The command line is too long." and starts nothing (every agent's 16 KiB brief in the agent journey on Windows, 2026-11-02), it expands `%`, and it reads `"` and, outside quotes, `& | < > ^` as its own.
npm installs every CLI's command as such a file, a `.cmd` beside a `.ps1` and a `sh` script, so the node reads the `.cmd`, the file its `PATHEXT` search finds, as the program it runs (`hide_platform::programs::Start`, one place for every caller, Hide AI's children included): Node beside the launcher, else the first on the search with `.JS` left out of `PATHEXT`, as the launcher picks it, then the options the script's first line gives Node and the script, or the package's executable itself when its command is one.
It reads only npm's own template (cmd-shim 4 to 8, npm 7 onwards), matched whole, and checks the file it names against the package's `package.json` `bin`; anything else, pnpm's and Yarn's launchers among them, which set `NODE_PATH` for their program, is a batch file Hide cannot read, logged with the reason (`platform.launcher_unresolved`), whose start goes by the program's name and whose first input is typed.
The `.ps1` and the `sh` script are never read.
A Windows start names its program's file rather than its name (`StartForm::Direct`, `hide_platform::programs::START_NAMES_FILE`), because PowerShell finds a name by its own rule, the `.ps1` before the `.cmd` beside it, so the file the node read would not be the file that ran; the core's own engine types it as `Start-Process` with one Windows command line (`hide_shell::quote::windows_argument`), which no shell reads again, so `%`, `&`, `^` and `"` reach the program as given in Windows PowerShell 5.1 and PowerShell 7 alike.
A Wake, a fork and a reopen ask the node the same way before their start and name the same file (`hide-core` `start_prompt::start_program`): their lines carry arguments too, such as Claude Code's `--settings '{"skipDangerousModePermissionPrompt":true}'`, which Windows PowerShell 5.1 hands to npm's `.ps1` without its quotes, so a Wake of an npm-installed Claude Code by its name failed with "Invalid JSON provided to --settings" (the agent journey on Windows, run 42046728301).
A start of a definition whose program is not the one the node was asked about (Cursor's `cursor-agent`, asked as `cursor`) goes by its name.
A brief is at most 16 KiB, and a launched one rides the start whole.
A start line past the terminal's canonical limit waits for the shell's line editor ([ARCHITECTURE.md](ARCHITECTURE.md)).
A typed brief is the typed rung's: Claude Code, Codex, Grok and agy took a typed 16 KiB brief once, but OpenCode 2.0.29, whose every brief is typed, showed none of it, omp 18.7.0 held it as a pasted-text chip it never submitted, and Grok on Windows recorded it without its line breaks (the agent journey, run 41398130161, 2026-11-02); each such brief is told to its parent as not yet taken ([Input an agent did not take](#input-an-agent-did-not-take)), and [INSTALL.md](INSTALL.md#what-each-system-supports) keeps the gap.
The agent journey's `start` cell launches a 16 KiB brief that starts with `-` and its `launch_brief` cell one that starts with `@` and names a file the repository does not have, each over lines and tabs, for every agent that declares the launch prompt, on Linux, macOS and Windows, where the journey runs each such CLI as the runner installed it, npm's launcher or its executable, with the agent's environment on the daemon whose panes inherit it (`scripts/agent_journey/ci.py`, `runs_as_installed`); each requires the agent's own record to be the brief, once, and the daily agent canary runs them against each CLI's latest release ([VERIFICATION.md](VERIFICATION.md#the-agent-canary)).
A launch prompt and a `codex queue` message are command-line arguments: the text is in the process's argument list while it runs, readable by the account's own processes, and the start line, typed into the pane's shell, is kept in that shell's history as any command the operator runs there is.

### Input an agent did not take

The engine can read an agent ready for input while a screen of the agent's own takes whatever is typed: a hooks review, an update notice, a trust question.
On 2026-10-31 a Codex 0.166 reviewer started while another program's hook entries sat at the front of `~/.codex/hooks.json`, so Codex opened its "review these hooks" screen; Herdr read it ready, the spawn typed the brief with `agent.prompt`, logged `brief.submitted`, and the brief was gone, while Codex never registered and every retry of the spawn answered `native_identity_unavailable`.
A successful prompt, or a way in that answers nothing ([The input ladder](#the-input-ladder)), proves only that the text reached the pane, so a spawn's brief and a letter's bell sent that way are taken once the agent itself records them, never by reading its screen (design principle 13, `hide-core/src/delivery/typed.rs`):

- Claude Code, Codex and Grok record it in their prompt hook: a pull at the prompt point whose digest is the typed text's (`prompt_digest`, the same canonical digest a bell is recognized by) confirms the brief of the spawn whose pane it came from (`Runtime::confirm_brief_by_hook`), before the pull's own identity check, because a child that never registered has no identity yet.
  Claude Code 2.1.316 records a paste of four lines or more, or of 800 characters or more, wrapped, in its prompt hook and its session file alike: `\n\n<pasted_content id="d974">\n` + the pasted text + `\n</pasted_content id="d974">\n`, the id four lowercase hex digits of its own; a shorter paste arrives as typed (measured 2026-11-01 in a private Herdr with `agent.prompt`, the path Hide types a brief by).
  Inside the block, and in typed words beside one, it writes a backslash after each `<` that starts a `pasted_content` tag (`<\pasted_content`, `<\/pasted_content`), so a brief that quotes another paste cannot close its own block.
  `hide_session::pasted` reads a block as Claude Code's own reader does (the open tag with a valid id and a line break, the first close tag with the same id, up to two line breaks taken on each side), puts the pasted text in its place, undoes that backslash, and reads again until nothing changes, so a brief that quotes a paste reads the same whether Claude Code wrapped it or not.
  The digest of what Hide typed then matches with no agent branch; before it, every Claude Code brief of four lines or more stayed unconfirmed, and the journey's one-line brief never showed it, so its `start` brief is now a markdown brief of many lines past 800 characters ([VERIFICATION.md](VERIFICATION.md#agent-journey)).
- OpenCode 1 and 2, Pi, omp and Antigravity record it as the person's message in their session file, which the label worker already reads on each change of the pane's engine state; the read keeps the digests of the last eight person messages (`SessionFacts.prompt_digests`), and one equal to the brief's confirms it (`Runtime::typed_input_work`).
  A newest person message that ends with the brief, read by the same rule, written after the first typing (within the two clocks' 2-second slack, `CLOCK_SLACK_MS`; a typing again moves only the last typing's time, `TypedInput::first_typed_at`) and not one Hide saw the operator submit, confirms it too, because the composer can hold text before the typing; the operator's own `don't fix tests` after a lost `fix tests` is not its record: Antigravity on Windows took a terminal report (`27;1$y`) into its composer ahead of a pasted brief (the fake-model agent journey, 2026-11-01); a message cut past 8 KiB (`MAX_TEXT_BYTES`) is never read this way, so such a brief that long confirms only by its digest.
  omp writes that message only when its turn ends, so for omp a turn the core reads starting after the typing is its record of the brief (`AgentId::turn_takes_brief`, `input.confirmed` with `by: turn`): a long first turn is never read as a lost brief, and a freshly spawned omp has no other work to start a turn for.
- Any agent whose hook reports its turn also records a brief by starting a turn after the typing that nothing else could have started: when a pass sees the core's turn working by the agent's own hook (the engine's reading alone never counts) since after the typing, with no input routed to the pane since the typing (the brief's own prompt is not counted) and no letter's bell typed there since (`bell_typed_at_unix_ms`), that typing is confirmed as omp's is (`input.confirmed` with `by: turn`, logged `input.turn_started`), so its parent is not told, its row asks nothing and it is never typed again (engineering principle 11), even when the prompt digest does not match.
  The core remembers the typing (`plans.input_turn_started`, bounded by the unconfirmed briefs) until the store confirms it, so a turn that ends first changes nothing.
  It misses a turn shorter than one pass, input during the turn, a restart of the core and an agent whose hook reports no turn; those briefs are judged by their record alone.
- A letter is taken as before: its prompt hook pull, tool intake or turn-end intake moves it out of `pending` ([Recognizing the bell](#recognizing-the-bell)).
  Its bell is recorded once the letter is handed, its prompt hook having asked with the bell line's digest ([Safe intake and manual fallback](#safe-intake-and-manual-fallback)).
  A bell whose prompt hook never reached the core within its budget (letter-19699, 2026-11-01) is recorded too when the recipient's session file holds the bell line as its newest message from another party, written after that bell was typed (within `CLOCK_SLACK_MS`, `Runtime::bell_line_recorded`): its sender is told nothing and its row asks nothing.
  That record shows the line was submitted, not that the letter's body reached the agent, so it takes nothing in and marks nothing handed, and the next bell, whose prompt hook prints the body, still rings; the same line recorded before the bell, such as one sender's earlier letter with the same first line, is not its record.
  A Claude Code session is read from the folder Claude Code names for the agent's cwd, each character that is not an ASCII letter or digit turned into `-` and a name past 200 characters cut with its hash (`hide-session`'s `project_directory`); turning only `/` into `-` missed every cwd with a dot, an underscore or a space, every worktree under `~/.hide` among them, and on 2026-11-08 judged a device agent's recorded bell lines `not_recorded`.
  Once the letter's last bell is spent with none of them handed, its recorded lines are what ends it: it is `undelivered` with `hook_not_taken` and its sender gets the one letter of [spent bells](#letters-while-the-agent-works), not a pending letter no bell will bring.

Typed input with no record is unconfirmed once the agent rested for `RECORD_WAIT_MS` (15 s) since the typing or since its last state change, whichever is later.
Whether the agent works or rests is the core's turn (`agent_state::turn_projection`: its hook first, then its session file, then the engine), the answer its row and the doorbell read, never the engine's own status, so a hook that says the agent works while the engine reads it idle owes no record and has nothing typed again, and a hook that says the turn ended while the engine still reads it working starts the wait ([status-model.md](status-model.md#where-each-axis-comes-from)); a state no source names counts as rest, so an agent the engine has not detected behind its trust screen is still judged.
Its last state change is the later of the engine's and the core's turn's.
Between a pass's observation of the pane and the turn that pass lays, and while the pane's read is stale, the core has no turn for it (`Standing::Unsettled`), and nothing moves: no notice is sent, nothing is typed, and a fresh session read already asked is not asked again.
What is shown stays as the doorbell's last pass found that typing (`plans.input_verdicts`, `Runtime::input_standing`): a row that showed `브리프 확인 안 됨` (or `편지 확인 안 됨`) keeps exactly that ask, one that showed none shows none, and `hide agent show` keeps the same `phase`; a typing no pass has judged yet reads `awaiting_record`.
A state ends it, never a timer: the next agent list, read every second while the engine answers, lays the turn of a fresh read; a stale read ends with the list that confirms the agents again, and meanwhile the row also shows the pane's activity unknown ([status-model.md](status-model.md#where-each-axis-comes-from)); a pane the engine no longer lists leaves the observations, and its input with it.
That bound caps the wait for a record and judges no screen: a prompt hook records at the submission itself and a session read lands within seconds of the agent's next state change, and an agent that works on the input owes no record until it rests again.
An agent can write its session after the state change the label worker read on, so before the verdict the core asks this machine's session-sync thread for one fresh read of that pane's session (`Runtime::take_session_rereads`, `labels::Worker::read_again`) and waits `REREAD_WAIT_MS` (5 s) for it; a record that read finds ends the input with no notice.
The judgment reads only the record, the core's turn for the pane, the engine's gate on input, the input Hide routed to the pane and, at rest, the agent's composer; it holds no per-question screen pattern.

Unconfirmed is not proof of loss.
When the agent's record source is not available, its input cannot be confirmed at all: an agent that records in its prompt hook whose hooks Hide reads as Not connected (`hooks_not_connected`), or an agent that records in its session whose session Hide has not read (`session_unread`).
Such input is never typed again, since the agent may have taken it, and the parent is told it cannot be confirmed rather than that it was lost.

1. The parent hears first.
   For a spawn's brief the notice goes to the spawn's parent, for a letter to its sender when that is an agent, as a `progress` letter with `--now` from the code-owned identity `hide:input`, under the intent `input-unconfirmed:<id>:<attempt>`, so a retry of the notice converges.
   Its first line is the one the operator reads in a wake summary: `<agent> has not confirmed its brief: read its screen with hide pane read <pane>.`, or `<agent> cannot confirm its brief (its hooks are not connected): ...` when the source is not available.
   The wording follows how the input was sent (`SendRoute`): a launched brief's first line is `<agent> has not taken its brief yet: ...` and says the start holds it; input handed to the agent's own way in reads the same and says the agent's input queue keeps it; a send whose answer was lost reads `<agent> cannot confirm <input> (Hide lost the answer to its send): ...`; each says Hide does not send it again.
   The rest names `hide agent show` for what the child waits on, says to answer the screen with `hide pane send <pane> --key <name>` or text and `--enter`, that Hide types the input again once the agent is ready (or, without a source, that it does not), and that answering a trust or approval screen is the parent's call: Hide does not check consent, the operator sets how much agents may approve in Settings > Agents, and a login is left to the operator.
   A notice that cannot be sent is logged `input.notice_unsent` and tried again after two minutes.
   The notice for a brief's last try, once all three are spent with no record (a try is a typing or the Enter a composer still showing the brief got), says the delivery failed: `<agent> did not get its brief: Hide tried 3 times and found none of them in the agent's session record, or could not read that record, so Hide does not type it again.`, naming only what Hide knows, since an empty record and one it could not read look the same to it, then how to read the screen and type it there by hand; `input.undelivered` logs it once with the reason `not_recorded`, parent or not.
   A letter's undelivered end is its own state, at its deadline or once its bells are spent with none taken, when its sender gets the `hide:input` letter that says so ([Letters while the agent works](#letters-while-the-agent-works)); its notices before then keep the wording above.
2. The operator sees it when no parent can clear it.
   The child's row is laid an ask (`AskSubject::InputUnconfirmed { input: brief | letter, spent, held }`) and rises in Needs You as the seventh escalation cause, `input_unconfirmed`, verb Answer ([status-model.md](status-model.md)), at once for a handed-off child, a letter with no agent sender or a parent no longer observed, and otherwise after the parent's grace of two minutes (`PARENT_GRACE_MS`) or once the typings are spent.
   The row reads `질문 · 브리프 확인 안 됨` (`편지 확인 안 됨` for a letter, `· 재시도 다 씀` once spent) in the interface language, and `브리프 못 받음` only when a composer read saw the brief still sitting there (`held`); the phone words it the same way.
3. A typed brief still in the composer gets its Enter; otherwise it is typed again when nothing moved on its pane since the typing or after someone answered the screen, and a brief sent any other way never is.
   At rest, by the same gate a bell is typed through (`doorbell::unheld_turn` and `doorbell::takes_keys`: the core's turn at rest with no approval prompt and no question or plan approval of the agent's own, the engine knowing the state and holding no blocked prompt, and a kind that takes letters at its tools not read working by the engine), quiet for the quiet period, the doorbell reads the agent's composer once the engine reports the same agent ready (`typed::composer_readable`, `delivery::composer::shows`).
   When it shows exactly the brief and nothing else (`Composer::holds`, which compares a one-line composer by the brief's first line), the submission was lost, and Hide presses Enter (`input.entered`); text and Enter on a choice screen could pick an option, which a composer showing the brief rules out.
   A composer that shows only the agent's own mark for a paste with the brief's line count shows the brief too (`Composer::pasted`, below): OpenCode 2.0.29 on Windows kept an 18-line brief as `[Pasted ~18 lines]`, its Enter lost while its model loaded, and before this read it held as `composer_not_empty` until the operator came (agent journey run 41951563729).
   When it is empty, the brief is typed again if nothing moved on the pane since the last typing (`typed::unmoved_since_typing`: neither the engine's state nor the core's turn changed after it, and the engine has not read the agent working since), the program having dropped the text while it started, or if someone answered the screen after the last typing (`Observation::answered_at_unix_ms`: the operator's keys, a phone reply, or a parent's `hide pane send` whose every write landed) (`input.retyped`).
   A pane that moved with nobody answering may have had the text taken by a screen of its own, so its empty composer holds (`moved_unanswered`); an agent that worked since the typing took the brief, and its record or its turn confirms it; anything else in the composer holds (`composer_not_empty`), and a held retype is logged `input.retype_held` with its reason.
   The engine types a retype only into a program that reads keys and whose start settled, as it does the first typing ([The input ladder](#the-input-ladder)).
   A `hide pane send` that was refused, stopped partway or lost an answer never allows a retype, even where its text landed.
   Either action spends a typing, reserved in the ledger (`SpawnRecord.brief_input.attempts`) before it is sent, so a crash between the reservation and the key still counts, and there are at most three (`INPUT_ATTEMPTS`): past them the input waits in Needs You with `spent`.
   A letter's bell keeps its own three bells, and a bell is never rung twice in one pane episode (its turn, state sequence, routed input and session), so it too rings again only after the pane moved, or, after a bell whose answer was lost, once the record wait passed and its composer reads empty.
4. The record ends it, whichever typing it records.
   `BriefConfirmed` sets the confirmation once and refuses later typings with `brief_confirmed`, so a record that arrives late, after the input was typed again, never causes another copy (engineering principle 11), and the notice owed for a typing is sent once (`noticed`).

The state lives in the delivery ledger beside what it describes, with no second store: a spawn keeps `brief_input`, a letter `bell_typed_at_unix_ms`, `input_noticed`, its last bell's failure (`bell_failure`) and how many bells ended (`bell_ended`), and a letter's notices wait while a bell is on its way; which input a composer read saw held, which fresh session read was asked and which letters with spent bells are owed an end and a note are the core's memory of this run only.
The decision runs on the doorbell thread and the core's delivery work, and the engine is reached only through its boundary (`Engine`); nothing is read or sent under the runtime lock.
A forced letter (`--force`) is judged the same way once its bell is typed.

omp 18.7.0 loses a submission typed right after it starts: the engine reads it ready before its session is up, and in 2 of 3 local runs the brief stayed in its composer, unsubmitted.
So a spawn waits until the engine records an omp child's native session, the identity a registration waits for, before typing its brief (`AgentId::brief_after_session`, `brief.session_unseen` when it never comes), and the composer Enter above catches any submission still lost.

Residuals.
An agent whose hooks were declined (Codex's "skip" on its hooks review) never records a prompt hook while Hide still reads its integration connected, so its brief stays unconfirmed: the parent and then the operator hear of it, and Hide types it again only after new input reaches the pane or when nothing moved since the typing, at most three times.
An answer typed through a client other than Hide (another terminal attached to the pane) is not input Hide routed, so it does not allow a retype; the operator's keys in Hide and `hide pane send` do.
The [external-input race](#safe-intake-and-manual-fallback) every bell has applies to a retype too.

How soon each agent records a brief, in seconds after the typing.
The local runs were measured 2026-10-31 on macOS at the pinned CLIs with the journey's isolated setup (private Herdr and `hided`, private HOME, the fake model held 20 s on the brief's turn so the take and the answer are apart).
The CI runs are the agent journey's fake-model lane (`agent-journey.yml`, run 41198805395 at this change), whose `start` cell spawns with `--brief` and requires the brief recorded after one typing ([VERIFICATION.md](VERIFICATION.md)):

| Agent | Record | Local macOS | Rested after (local) | CI Linux | CI macOS | CI Windows |
| --- | --- | --- | --- | --- | --- | --- |
| Claude Code | prompt hook | 2.7 | 21.7 | 0.4 | 0.5 | 1.0 |
| Codex | prompt hook | 0.0 | 20.3 | 0.5 | 1.2 | 2.7 |
| Grok | prompt hook | 4.2 | 22.3 | 0.3 | 0.4 | 3.4 |
| Pi | session file | 1.0 | 20.6 | 1.8 | 1.9 | 1.6 |
| omp | session file | 25.6 to 28.3 (3 runs of 3 with the session wait; before it, 1 of 3 at 1.6) | 25.6 to 26.9 | 1.7 | 1.8 | 2.5 |
| OpenCode 2 | session file | not run locally ([VERIFICATION.md](VERIFICATION.md)) | | 1.0 | 2.2 | 1.6 |
| Antigravity | session file | not run locally ([VERIFICATION.md](VERIFICATION.md)) | | 1.7 | 1.8 | 1.8 |

Every agent records at the take or, omp locally, at most 1.4 s after it rests from the brief's turn, well inside `RECORD_WAIT_MS`: the wait starts at the agent's rest, so a working agent owes no record, and omp's turn itself confirms its brief.
In the run before the fresh session read (41195283433), Antigravity on Windows took the brief but the session read that would have seen it ran before its session write; the parent was told at about 16 s and the brief was typed a second time at about 31 s.
With the fresh read before the verdict, the same lane recorded it after one typing on all three systems.
The Codex 0.163.0 start with another program's entries at the front of `hooks.json` reproduced the incident in the local setup: without this change the brief went to the hooks review and nothing followed; with it the parent was told, and after its screen was answered through `hide pane send` the brief was typed again and the prompt hook confirmed it.

### A blocked child

A delegated child held at a blocked prompt is its parent's to handle first, and the operator's only once the parent had a minute (`hide-core/src/runtime/blocked_children.rs`).
The prompt is whatever made its row `blocked`: its hook's wait for a permission or an answer, a question or plan approval its session read holds, or the engine's prompt ([status-model.md](status-model.md#where-each-axis-comes-from)).

1. The parent hears at once.
   The session projection pass that first reads the child blocked records the block's start, and the next coordinator tick (250 ms) sends the parent one `progress` letter with `--now` from the code-owned identity `hide:blocked`, under the intent `child-blocked:<pane>:<start>`, so a working parent reads it at its next tool call and a repeated send converges on the first.
   Its first line is `<child> is blocked at a prompt: read it with hide pane read <pane>.`; the rest says to answer it with `hide pane send <pane> --key <name>` or text and `--enter`, or to send the child a safer instruction, that its runtime may time the prompt out on its own (Claude Code's built-in `rm` check gives up after two minutes, 2026-10-16), when the operator is asked, and the same words on answering another agent's screen as the [unconfirmed-input notice](#input-an-agent-did-not-take).
   The note goes to the parent observed in its pane, running the session the child's lineage names, or to that pane's agent asleep, whose mailbox keeps it for its next wake; a `progress` letter wakes no one.
   A parent Hide cannot address, and a note that cannot be sent, are logged (`child_blocked.note_unaddressed`, `child_blocked.note_unsent`) and not tried again: the grace runs, and the operator gets the child when it ends.
   A parent that is itself blocked keeps the note pending like any letter, and the note raises nothing, since only a child's own letters do.
   A child that declared a person's turn gets no note: it skips its parent and rises at once ([A person's turn](#a-persons-turn)).
2. The root is raised after the grace.
   The child's lineage root enters Needs You with the child's ask (`child_blocked`) once the same block has lasted `CHILD_BLOCKED_GRACE_MS`, 60 seconds from its start (`agent_state/escalation.rs`); the session projection's one deadline, checked on the existing coordinator tick, wakes at that moment, and the ask's time is the block's start.
   A minute leaves the parent time to act and the operator time to answer before a runtime that times its prompt out at two minutes does; a two-minute grace would lose that prompt.
   During the grace the child draws `waiting` and its root only turns unread through the descendant signal ([status-model.md](status-model.md#the-mark)), and every other escalation cause keeps its own clock.
3. A block ends when the core reads the child settled and not blocked (a numbered state it knows, `agent_state::settled`), when another known session runs in its pane, or when the pane goes; the next block is a new start, a new note and a new grace.
   A pane still being restored (sequence 0, `unknown`, no session named yet, its lineage not given back yet) and a disconnected device's last row keep the block, so a device whose link comes back keeps the same start, deadline and note.
   A block that ends and returns between two reads of the agent list, about a second apart, reads as one block.

Known limit, accepted as a rare edge that clears itself: the block's start is the core's memory, so a restart of hided while a child is blocked starts its grace again once, taking its root out of Needs You until a minute after the restored pane reads blocked, and may send its parent one more note.
Keeping the start across a restart would need a durable record in the delivery ledger, which this does not add.

Regression owners: the escalation tests in `agent_state/escalation.rs` (`a_blocked_child_rises_only_once_its_grace_from_the_blocks_start_ends`, `within_its_grace_a_blocked_child_still_rises_for_another_cause`), the block tests in `runtime/blocked_children.rs`, and `runtime/tests/blocked_children.rs` for the note, the grace's end on the tick, a parent that cannot be told and a device's reconnect.

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
| Pending delivery deadline | 60 minutes of the recipient's machine being reachable (wall time on the core's own machine), not counting the time the recipient sleeps or wakes, then `undelivered`; visible through CLI |
| Quiet period | 30 seconds since hide last routed a key and since the engine last changed the pane's status |
| Doorbell reservations per letter | Three total, persisted across restart |
| First inactivity warning | 20 minutes without activity (30 for a code-owned observer); one minute (one tick) when the target stopped on a question or is held at an engine-blocked prompt and the observer is a pane |
| Second inactivity warning | First-warning time plus 60 minutes, at most two warnings per episode |
| Unanswered parent warning notification | First-warning time plus 60 minutes, once per native target and inactivity episode |
| Answer wait of a `request` or `block` | 24 hours from the send, then `answer_wait_ended: "deadline"`; an end of either registration ends it at once as `party_ended` for a letter the recipient took in |
| Intent retention and finished-letter cleanup | One day from the finish; open letters remain; a full 5000-letter total drops sooner (next row) |
| Open / retained letters | 1024 / 5000; a letter is open while it awaits intake or a reply; at 5000 a new letter drops the finished letter that finished first (ties: the earlier sent), never an open one, and is refused with `capacity` only when none is finished |
| Watches | 32 |
| Letter body / ledger file | 16 KiB / 16 MiB |
| Hook bundle / context / total deadline | 1024 letters taken, each sender's three newest shown beside its questions and blocks / 8 KiB / two seconds |

Automatic doorbells apply only to `pending` letters, and the deadline turns only a `pending` letter `undelivered`, while watch clocks remain distinct.

The deadline of a letter to an agent Hide put to sleep does not run while it sleeps or wakes (PRD mission-worker-sleep D-13): the store's pass reads the asleep recipients from the core's sleep records each time (`Runtime::delivery_asleep`), expires none of their letters, and on the pass after one wakes moves each of its waiting letters' start by the time it slept, once (`Ledger::resume_woken`), so a letter that waited 50 minutes before the sleep has 10 left and one sent during it the whole hour; on a machine with a clock the sleep is taken as reachable time, which only ever leaves a letter more of its hour.
A recipient whose wake failed is not asleep for the store, and its letters' hour runs again from the failure.
A `request` or `block` whose bell already rang, or whose one wake started before the current sleep, wakes no one and its hour runs through the sleep; a `report` or `progress` waits for the next wake paused, rung or not (`Sleeper::pausing`).
The ledger keeps no new field for it, and a restart reads the sleepers again from the records.
The deadline of a letter to an agent on another machine counts only the time that machine is reachable (PRD core-host-node-move B13, D-11): the core is connected to that machine's engine and its link has not ended, so a MacBook that is closed, asleep or away holds its letters at the core and they are handed over when it is back, with the hours away not counted.
The ledger keeps one clock per machine a letter awaits intake on (`node_clocks`, keyed by its registration id: the reachable time in ended stretches and the start of the one running now), and each letter records where on its machine's clock its wait began (`intake_from`); a letter to the core's own machine, and one written before this rule, waits in wall time from its creation as before.
The store reads which machines are reachable under the runtime lock on each pass (`Runtime::delivery_reach`); a node's link end is read there at once, before the sync that notices its engine is gone.
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
Activity is the later of the engine's status-transition time and the confirmed native session file's modification time.
A status transition is a status other than the last settled one the watch read, or another state sequence on the same engine connection (`watch::StateRead`), since a sequence compares only within one engine ([status-model.md](status-model.md#across-an-engine-restart)).
A device's link that comes back is no activity: its engine restores the target's pane at sequence 0, then `unknown`, which are no state, then at a sequence of its own, so a target that rested through any number of reconnects keeps its quiet episode, its warnings' hours and the human notice they owe.
A target found in another state once the link is back is one transition, as one found on the same link is, and its parent hears of the quiet after it once.
The engine connection a watch last read on is held in memory only: a watch loaded from the file compares its first reading by status.
While the link is down the watch reads the target's last observation, as for any failed read, and warns by it.
A local read and the device helper's `session_activity` use the same session ownership and root-confinement checks.
Session lookup has one total budget of 10000 directory entries, including skipped extensions and unmatched names, 64 visited directories and 8 MiB of retained path bytes.
Crossing a limit returns the path-free `session_capacity:<resource>:<limit>` outcome (a native header over the 1 MiB naming cap is `session_capacity:line_bytes:1048576`, never a session that is missing); the watch uses its existing status fallback and records the failure.
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

A target that stopped on a question it asked only in its own pane is told to a pane observer after one tick, 60 seconds of no activity, instead of 20 minutes, so the parent decides within about a minute whether to answer the child or ask the operator.
The fact is the target's own row while its pane runs the watched session: activity stopped, a question demand ([status-model.md](status-model.md#where-each-axis-comes-from)) and no blocked prompt, where the question is not Hide's own about input it typed (`AskSubject::InputUnconfirmed`, laid on the row as a question but told to the parent by its notice, [Input an agent did not take](#input-an-agent-did-not-take)).
A target held at a blocked prompt is not the watch's: its parent is told at once by a note of its own and its root is raised after a grace ([A blocked child](#a-blocked-child)), so the watch keeps its ordinary window for it.
A child that works, such as Codex while its async question stays open, keeps the 20 minutes.
A question the target already sent this observer as a `request` or `block` letter still awaiting its answer keeps the 20 minutes too, since that letter rang the observer's doorbell.
The early warning is the episode's first warning, and only its first line differs: `Stopped with a question (watch <id>).` instead of `No activity for N minutes`.
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
A report ends with the Views its sender opened that are still open, each with its id, what it shows and whether the operator has seen it, and how to read one or show it again (PRD unseen-agent-views D-21); the list stays inside the letter's size cap and ends with how many more there are when it is cut, and a repeated report adds nothing.

## Code-owned identities

The `factory:` name prefix remains reserved for compatibility with historical registrations.
No Factory recipient is active in this release.

Hide itself is a code-owned identity, `hide:<role>` of kind `hide`: `hide:memory`, the origin of the agent a Memory card's promotion starts ([ARCHITECTURE.md: Project sessions and Memory](ARCHITECTURE.md#project-sessions-and-memory)), `hide:conductor`, the sender of a mission's wakes ([Missions](#missions)), `hide:delivery`, which tells a sender its letter waits on a session Hide cannot read ([Letters while the agent works](#letters-while-the-agent-works)), and `hide:input`, which tells a parent or sender that input Hide typed is not confirmed ([Input an agent did not take](#input-an-agent-did-not-take)), and a sender that a letter whose bells are spent was not delivered ([Letters while the agent works](#letters-while-the-agent-works)).
Each `hide:<role>` that starts agents (`hide:memory`, `hide:desk`) is registered by its first spawn without a parent, which takes that record as the origin, since Hide has no pane to report (`coordination::executor::spawn`, `Actor::hide_owned`); `hide:memory` sends nothing and receives nothing; `hide:conductor` sends progress letters to a mission's Conductor and receives nothing; and a pane or agent name that starts with `hide:` is refused at registration like `factory:`.
A spawn is Hide AI's own when it is a handoff whose origin is that identity (`coordination::started::hide_started`): a pane agent's handoff has a pane's identity as its origin, so neither counts, and no branch or worktree name ever decides it.
Only that identity may give a spawn a `task` (`SpawnTask`: a `kind` code such as `memory_promote` and a `target` such as the card `m72`, the codes the screen words), on a handoff; any other caller that sets one is refused `task_reserved`, and the task is part of the intent, so the same intent with another task is an `intent_conflict`.
A receipt from before the task existed has none, and its agent and checkout are still Hide AI's, which the screen names `Hide AI` alone.
The kind is a closed set in code (`TaskKind`), and each known kind names its home, the screen that shows its work (`TaskKind::home`: `memory_promote` and `suggestion_apply` to the Hide AI tool's 제안 tab); a new kind cannot be added without naming one.
A kind this build does not know loads from the ledger as it was written, so a record a newer build wrote never fails the load or loses the rest of the ledger, and its agent stays an operator's row.
A new spawn of a kind with no home is refused `task_kind_unknown`, and the first time an agent of such a kind is seen the diagnostic log records `hide_ai.task_kind_unknown` once.


### Desk

`hide:desk` is Desk's agent (PRD Desk 1차, [ARCHITECTURE.md: Desk](ARCHITECTURE.md#desk)): the process the core starts for the agent the operator chose, which has no pane and no checkout.
Its `hide` presents a capability hided binds to that process's pid and start time (`Binding::Desk`), recorded with the caller id `hide:desk` (`workspace_control::DESK_CALLER`) and the context `desk_context`: this machine, workspace and checkout `hide:desk`, no path.
A delivery command on it (`hide agent`, `hide request`, `hide inbox`, `hide watch`) acts as `Actor::hide("desk")` while the core says Desk runs a session (`desk_authority`), and on this machine only; with Desk ended it is refused `desk_not_running`, and the capability is revoked with the process.
Every Workspace, View, Memory, mission or plan command refuses it `desk_has_no_checkout`, so Desk opens nothing on the operator's screen and reads no Project's Memory.
A spawn Desk makes without `--parent` is a handoff whose origin is `hide:desk`, registered by Desk's first spawn: the agent is the operator's own, and `hide_started` leaves it out of Hide AI's work; one with `--parent` is refused `invalid_request`, since it would skip Desk's places and send its asks to Desk, which no screen shows, and one with words for the agent's own CLI after `--` is refused `desk_spawn_args`, since they can lift the new session's permissions and Desk's Codex runs a `hide agent spawn` on its rules, which read only a command's start.
`hide:desk` is in `code_owned_authority_current` while this daemon runs Desk, so a spawn waiting in its queue starts after a restart; its capability still holds only while its process runs.
A spawn of Desk's past its places (Settings › Labs › Desk, 8 at most) is taken into Desk's queue in `desk.json` and answered `{"queued":true,"intent","position","next_action"}` without running; asking again with the same intent answers the same place.
The core starts the first waiting spawn as `hide:desk` on a thread of its own when a place frees, and a spawn whose start failed waits in the queue as failed until the operator retries it; a retry resumes the same intent, so the ledger's unfinished record converges.
The core watches each session Desk started with `hide:desk` as its observer once its spawn completed; its warnings are code-owned and reach no person's notification.
The job counts as watched once the watch's answer came back; a refused ask is logged (`desk.watch_refused`) and asked again on a later publish, three times at most.

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
  It is the name the agent registered under, else its registration id (`agent-12`), which is stable and which `hide request send` and `--parent` already resolve; an agent that never registered is called by the only identifier the engine gave it, its agent name there or else its pane id.
  `coordination::wire_name` makes it, the writing side (`mailbox::send`, `watch::start`) and the reading side (`link_parent_facts`) both call it, and nothing else may spell it, because the label worker accepts a parent's request as the child's work only when the two strings are equal ([status-model.md](status-model.md#task-identity)).
  It never follows a screen title, which changes every turn.
- The screen name is what a person reads: the name it registered under, else the title Hide's screens give it while its pane hosts it, else its runtime's name (the bell line, the turn card's letters, the sidebar and Sessions rows).
  A name Hide made up for the pane (`hide-<kind>-<pane>`, a wake or a fork name, `fork::hide_made_name`) only spells a pane id, so it counts as none (`AgentRecord::screen_name`).
  It is derived when it is drawn and stored nowhere.

A registration stores a name only when it has one of its own.
A parent that `hide agent spawn --parent here` registers for the caller takes the name the engine has for the pane, and none when it has none, so a pane id is never stored as a name (`AgentRecord::name` is optional, and an executor-made record drops a name that only restates its pane).
On disk the field stays a string on every record, so a build before this one reads the ledger this one writes: an unnamed record is written under its registration id, and reading a name equal to the record's own id or to its pane as no name turns that back (`AgentRecordOnDisk`).
A ledger written by an earlier build can hold a pane id as a name; reading it that way gives such a record no name, with no migration file, and a code-owned identity keeps its name, which is also its pane.
A letter keeps the endpoint names it was written with, so a letter written before this rule keeps a pane id in its history until its one-day retention ends.
`hide inbox` and a letter's envelope show the registration id of an unnamed sender, not its title, and show the pane id of a sender that never registered and that the engine never named.

## A person's turn

`hide agent ask-human auth|permission <where>` declares a step only a person can take, `hide agent ask-human --clear` takes it down, and `hide agent show` reads it as the `human_turn` waiting cause ([status-model.md: A person's turn an agent declares](status-model.md#a-persons-turn-an-agent-declares)).
Only a registered agent in its own pane may declare (`agent_pane_required` otherwise); a kind other than the two is refused `human_kind_unknown` with the next action of a letter to the parent, a place that is empty or longer than 300 characters `human_place_required` or `human_place_too_long`; any other place is kept as written on one line, a URL's query and fragment included, and leaves the ledger with its declaration.
The answer is `{outcome, agent, machine, declaration, operator_notified}`: `declared`, `unchanged` for the same kind and place, `replaced` for another, `cleared`, and `nothing_to_clear` when nothing stood, which is no failure.
Only the operator's identity settles a declaration (`operator_authority_required` otherwise), by its id, so a settle of one that no longer stands does nothing.
Done sends the declaring agent one letter from the operator, kind `report` under the intent `ask-human-done:<id>`, first line `운영자가 처리함: <인증|허용>` without the place, since the letter stays after the declaration is gone, so two presses or two surfaces send one letter; it wakes the agent when it sleeps, the one report that does (`Letter::wakes_a_sleeper`), and waits for no answer, so the row has no reply to wait for after it.
The letter is saved before the declaration goes, so a letter that could not be saved leaves the declaration for another press, and the diagnostic log says why (`ask_human.settle_failed`); a press while one is being written starts nothing more.
When the agent's pane is gone, neither drawn nor asleep, no letter can reach it, and Done takes the declaration down with none (`ask_human.done_unaddressed`); a pane still drawn that Hide has not yet seen, as right after a restart, keeps the declaration for another press.
The operator's own message to the agent takes the declaration down with no letter.
The diagnostic records `ask_human.*` carry the agent, its lineage root, its device, the declaration id and the kind, never the place.

## Agent registration and spawning

`hide agent register [--check]`, `list`, `show` and `end` preserve the caller surface used by dispatch and Fork.
`register` records the participant on the caller's own machine, which the pane capability names: `--machine` may be left out, and when given it must name that same machine or the call is refused with `machine_identity_conflict`, so a caller in a connected device's pane registers as that device.
`hide agent show here` answers the caller's own registration, read only, with no renderer: the one live record whose actor is the caller's attested pane, device and session, the same match as `--parent here` (`coordination::live_self`), refused when two match (`coordination::here`).
Only a pane-bound credential can ask, as for every delivery and agent command: a checkout-bound one is refused `agent_pane_required`, and a pane-bound one whose hint names another pane is refused `caller_identity_conflict`.
`hide agent show <id>` adds `state`, what the agent is doing now, from the row the core already projects for the sidebar, `hide agent list` and `hide ps` ([status-model.md](status-model.md)), with no status of its own (`hide-core/src/runtime/agent_show.rs`): `activity` is the row's `working`, `stopped` or `unknown`, and `unknown` whenever the core cannot read the agent now (no row for the registration's session, a stale agent read, a device that does not answer), never `stopped`; `lifecycle` is the life below; `observed_at_unix_ms` is when the core last read the agent's state, which holds until a later read changes it.
`waiting` is null only when the core reads the agent waiting on nothing, `{"cause":"unknown"}` when it cannot read it, and otherwise `{cause, on, since_unix_ms, letter_id, spawn_id, text}`: the cause is one the core already names, in the order every surface ranks them, a person's turn the agent declared (`human_turn`, with `human` holding its `id`, `kind`, `place` and `declared_at_unix_ms`), an escalation cause that raised the row (`draft`, `parent_blocked`, `bell_exhausted`, `undelivered`, `observer_unconfirmed`, or the child's own `approval` or `question` for a blocked child) or a question the core laid on it (`plan_review`, `mission_decision`, `mission_attention` with the node's title as its text, `input_unconfirmed`), all waiting on `operator`, then the agent's own demand (`question`, `approval`, `blocked`), which waits on its parent's registration id for a delegated child and on `operator` for a root, then typed input whose parent or sender still has its grace (`input_unconfirmed` on that agent).
`text` is a short line of the row's when it has one, copy for a reader; `cause` is what to act on.
`pending_inputs` lists each brief or letter Hide typed into the agent that it has not recorded: `{kind, spawn_id, request_id, attempts, max_attempts, phase, typed_at_unix_ms, noticed_at_unix_ms}`, `phase` `awaiting_record` while no record is owed yet, `unconfirmed` once it is, `exhausted` with no typing left ([Input an agent did not take](#input-an-agent-did-not-take)); a recorded input is left out.
`hide agent show` also takes a spawn's id or the intent the caller spawned under, read only: once the spawn's child registered it answers that registration, and before it answers `{"spawn":{id,intent,name,kind,machine,pane,child},"state"}`, the pane's state with `lifecycle` null, so a child that never registered, such as a Codex held on its hooks review, can be read and answered; another caller's intent of the same words is `agent_unavailable`.
Any other key is a name or a pane, as a letter's recipient is named, a bare pane id being one of the caller's own machine (`coordination::shown_record`): the one live registration named so or in that pane, else the last one that was and has ended, answered with `runtime` and `lifecycle` `ended`; two live ones are `target_ambiguous`.
A key that names nothing, such as the number of an id without its `agent-`, is `agent_unavailable`, whose next action says how agents are named and does not tell the caller to retry, since the same key answers the same.
A spawn that stopped partway after it made its pane (`native_identity_unavailable`, `brief_not_submitted`, `agent_not_started`, `pane_busy`) carries `"spawn":{"id","pane"}` beside its code and next action (`executor::incomplete_spawn`), printed by the CLI as `error.spawn`, and running the same command again resumes that spawn and never starts a second child.
`hide agent list` answers every registration the ledger keeps, ended ones included, and `hide agent list --live` only those not ended (`running`, `asleep` or `closed`); a plain list answers as it always did, so a caller that reads ended registrations from it keeps them.
`list` and `show` give each registration's life as `runtime`: `running`, `asleep` while Hide holds its agent asleep (put to sleep, waking, or a wake that failed, in its pane or with its pane closed), `closed` while its pane is closed and its Reopen closed entry is listed, and `ended` once the record ended; `connection` is `connected` only while it runs.
The life has one source, the core's sleep records read for the registration's pane and session or the dormant record that holds it, and the closes and Reopen closed entries that hold it (`Runtime::sleeping_registrations`, beside `asleep_route` and `closed_target`, which letters to a sleeper or a closed agent read), and one decision (`coordination::life`), which `hide ps` reads too, so the two never answer differently for the same agent.
An agent whose end still runs is `running` until the end lands, and one the engine lists again in its pane is `running` from that read on (`every_answer_reads_a_sleeping_agent_asleep_and_running_once_it_wakes` in `hide-core/src/runtime/tests/mission_worker_sleep.rs`).
A registration keeps its pane's agent session as the engine records it: an id, or for Pi and omp the session file's path, which lies in a folder named after the checkout's path and so passes 256 bytes in a deep checkout.
The ledger keeps a session of up to 4096 bytes (`SESSION_LIMIT`, the bound a letter pull takes), and finds a pane's registration by the session's SHA-256, so its length never reaches a key.
A longer session is refused `session_too_long` when the record is made, never as an unavailable ledger: `hide agent register` refuses it before it reaches the daemon, as the daemon does again before it asks the engine, and a spawn whose agent reports one closes the pane it made before it answers, with the next action to spawn with a new intent in a checkout with a shorter path.
An empty session or one holding a control character is refused `invalid_registration` by the same rule (`coordination::kept_session`).
A build before this one reads a session past 256 bytes as a damaged ledger, so going back to it after such a registration answers every delivery and agent command `ledger_unavailable` until this build or a later one runs again.
A caller with no such record is refused `participant_ended` when its record ended, `participant_session_changed` when its pane's live record belongs to another session (the pane's agent session changed), `ambiguous_participant` when two live records match, and `participant_unavailable` otherwise; a remote participant on a pane of the same name is never the local caller.
`hide agent archive <id> [--actor <id>]` does what `세션 보관` does for the session registration `<id>` holds ([UI_BEHAVIOR.md](UI_BEHAVIOR.md)), through the same admission as the screen's `archive_session` event (`Runtime::admit_archive`), and answers `{"archived":"<id>","pane_id":"<pane>"}` once the archive begins: its save first, its closes after.
The agent itself or its parent may ask, as for `hide agent sleep`; Desk may ask for any live registration (`desk_authority`), and archiving a working session is one of its irreversible kinds, which always waits on the operator's card ([UI_BEHAVIOR.md](UI_BEHAVIOR.md), Desk).
A session that is working is refused `archive_confirmation_required`, with the next action to ask the operator to archive it from Hide's screen, where they confirm stopping the work, and the screen shows nothing for that refusal; Desk's archive is not refused once the operator allowed its card (`한 번 허용`, or `이 대화에서 계속 허용` for the rest of the conversation), since that card is the confirmation.
Every other refusal of the screen's archive is the same kind with `_` for the dot (`archive_delegated`, `archive_session_unknown`, `archive_device_disconnected`, `archive_busy` and the rest), beside `agent_unavailable`, `agent_ended`, `parent_authority_required`, `actor_identity_conflict` and `agent_pane_required`.
Asking again while the archive runs answers the same success and closes nothing twice.
`hide agent spawn` requires `--name`, `--intent` (or `--plan-node <key>`, which hands a step of the caller's plan and leaves the intent to the core, [Plans](#plans)), `--kind`, `--repo` and `--branch`, with optional `--parent`, `--machine`, `--beside`, `--path`, `--profile`, `--model`, `--effort`, `--brief <text>` or `--brief-file <path>`, and native arguments after `--`.
The brief is the child's first prompt: once the engine reports the started agent ready for input (up to 30 s, `BRIEF_READY_WAIT`), the spawn submits it through the engine's prompt operation, the one submission path the doorbell uses too, and only then waits for the child's session, so an agent that reports its session at its first turn (Codex, OpenCode 2) reports it on that turn and binds at once instead of waiting.
Once the child is bound and the brief went in, whichever comes last (`coordination::first_letter`, reading `BriefPhase`), the same text is recorded as a letter under the spawn's intent, kind `request`, delivered and hook-confirmed, once: a letter the parent already holds under that intent stays as it is, so `hide inbox`, the ledger and a later wake carry what the child was started on, and a letter the parent sends under that intent afterwards is this one.
Hide's own record of the brief (`briefed` after it types the brief again, the agent's confirmation) can land after the parent ended; it is saved then, the letter the ended parent already holds under the intent (read from its ended registration, which names who sent it and grants nothing new) stays as it is, and with none no letter and no answer wait is made, logged `spawn.first_letter_skipped` with `reason` `parent_ended`, or `parent_unregistered` when the ledger has no registration for the parent at all.
A live parent's letter that is refused (`capacity`, an invalid actor) still refuses the record that would have written it, as before.
The brief is part of the intent like `args` (another brief under the same intent is `intent_conflict`), the ledger marks it submitted (`briefed`) so the same command again never submits it twice, a blank brief or one past a letter's body limit (16 KiB) is `invalid_brief` before anything is made, and an agent that takes no input within the wait, or a submission the engine refuses, is `brief_not_submitted` with nothing typed, where the same command submits it on the next try without starting the agent again.
The refusal's `spawn` carries a `cause`, logged with it: `question` when the agent's screen still asked a question of its own when the wait ran out (a folder's trust, a numbered menu such as Codex's Windows sandbox setup, which reads as `blocked` and so never takes input), `not_ready` when it did not take input otherwise, `unread` when the engine could not say, and `agent_ended` when the engine no longer listed the agent, logged as `brief.agent_ended`; for that one the same command starts the agent once more, as for any start that ended at its shell.
An agent that still runs is watched as the spawn's from then on, by the same wait as one whose session comes late ([A registration that waits for its agent's session](#a-registration-that-waits-for-its-agents-session)), so its end is logged with the brief it never took, and an operator's own first turn binds it; the spawn stays open until its brief is typed, so the same command still types the brief once, and only then answers from the receipt (with-hide/hide-private#717).
A brief the engine took is not yet a brief the agent took: the child's own record confirms it, and one the agent has not recorded reaches the parent, then the operator, gets the Enter it lost when it still sits in the composer, and is otherwise typed again only after its screen was answered ([Input an agent did not take](#input-an-agent-did-not-take)).
The CLI reads `--brief-file` itself, in the caller's account, and refuses a file it cannot read, a blank one or one past the limit before the daemon is asked; `--brief` and `--brief-file` together are refused the same way.
`--model` and `--effort` are the agent's launch choice: the start passes them in the kind's own spelling, and every wake, fork and reopen Hide makes of the agent starts it with them again (`docs/ARCHITECTURE.md`, Agent start), while native arguments after `--` apply to this start only.
`--profile <name>` names the agent's profile on the machine it starts on, by its name regardless of case, `default` for the built-in one; without it the child runs with its caller's profile when the caller is the same agent on the same machine, else with the agent's default profile there (`docs/ARCHITECTURE.md`, Starting an agent with a profile).
A name the machine does not list for that agent is `profile_gone` before anything is reserved; the profile is part of the intent like `args`, and the profile the agent started with is kept on the spawn's record, which every wake, fork and reopen of the agent starts it with again.
A mission node's worker spawned with no `--profile` starts with the item its plan names ([Missions](#missions)); that item is not part of the intent, so the same command asked again is the same spawn, one that finished answers its child without reading the plan or the profiles again, and the same intent with a `--profile` it did not send is `intent_conflict`.
A choice the kind does not take is refused `invalid_launch_choice` before anything is created, and the choice is part of the intent, so the same intent with another choice is an `intent_conflict`.
Choose responsibility when creating the agent; it cannot be transferred afterwards.
With `--parent here` or the caller's own id, the agent is delegated: its parent owns the work, lineage is written immediately and an automatic watch starts.
Without `--parent`, the work is handed off to the operator: the agent is an independent root, has no lineage edge to the spawner and starts no automatic watch.
Both modes create a separate tab, in an existing checkout or a new worktree, without changing the current screen or keyboard focus.
A new worktree's branch starts where New worktree's does: from `origin/<branch>` as last fetched when the repository folder's own branch has no commits origin lacks, else from that branch ([UI_BEHAVIOR.md](UI_BEHAVIOR.md)); on a device whose helper is not connected the spawn cannot read that and leaves the start to the engine, the folder's own HEAD.
A spawn a mission admitted into the mission's own project starts its new worktree from the mission's destination instead of the repository folder's own branch, by the same rule ([Missions](#missions)); a destination the repository does not have refuses the spawn with Git's reason, and nothing else is started from in its place.
A spawn into another repository of that machine starts from that repository's own branch, since the destination is a branch of the mission's project; a repository the worktree catalog does not list yet, as after a restart no window has followed, is compared by the path the spawn named.
The destination a mission records at its creation is the branch the worktree catalog last read for the checkout, with no Git call of its own: with no window showing the project, or right after a `git switch`, that can be the branch from before, so the guide has the Conductor compare `도착` with the checkout's branch and name the right one in the plan.
With `--beside` the child opens in the caller's own tab instead: its pane is split right of the caller's (the engine's split, unfocused, the call Fork agent to the side uses), drawn at once on this machine's screen and replaced by the engine's layout when it arrives (a runtime that cannot draw it logs `beside.draw_skipped` and the spawn goes on), and the rest of the start (the agent, its brief, its session and the child record) is the tab path's.
Placement says who answers for the agent: one beside its caller is the operator's to watch, an operator-owned root with the caller as its origin, so `--beside` with `--parent` is refused `beside_with_parent` before anything is checked, registered or created, and a supervised child opens in a tab of its own; a child beside its parent that an earlier build made keeps its parent, and delegated-child relocation leaves it in its caller's tab across restarts ([ARCHITECTURE.md](ARCHITECTURE.md)).
It is only for the caller's own checkout on the caller's own device, and everything else is refused before anything is registered, reserved or created, with the next action to run the command again without `--beside`: `beside_other_checkout` when `--repo` and `--branch` (and `--path`) name no existing worktree or another one than the caller's, so a new worktree or another branch never opens beside; `beside_other_machine` when `--machine` names another device, whose engine cannot hold a pane in the caller's tab; and `beside_needs_agent_pane` for a code-owned caller with no pane of its own (a shell that is no agent pane is refused earlier, as every agent command is, with `agent_pane_required`); a caller's worktree that is gone by the time the pane is made is `beside_other_checkout` too, since a spawn beside never makes a worktree.
An agent on a connected device that spawns with `--beside` and no `--machine` gets its child beside it on that device's screen.
The split is labelled with a reserved pane label (`hide-spawn:v1:<spawn id>`, never shown as a name) before anything else, so the same command after a reply was lost finds that pane and makes no second one, and the label is cleared once the spawn record names the pane; a retry of a spawn whose pane is recorded, or that completed, is not asked about the checkout again; a split the engine refuses, or a caller pane that is gone, fails the spawn with `beside_pane_unavailable`, opens no tab in its place, and logs `spawn` `beside.split_failed` with the spawn and pane ids; a split that cannot be labelled is closed.
A split whose own reply is lost before the label is written leaves a bare shell pane beside the caller that a retry does not find; the retry makes its own pane.
`--beside` is part of the intent, so the same intent with or without it is `intent_conflict`.
The no-focus request applies to opening the checkout's owner workspace as well as creating its tab; interactive workspace and tab creation keep their focus behavior.
`hide agent spawn --help` explains the modes without contacting a daemon; the removed `--no-watch` flag is refused before any effects.
Every other `hide` command ending in `--help` or `-h` prints that command's usage to stdout and exits 0; a wrong command prints one line that says what is wrong (`hide device needs a subcommand`, `unknown flag for hide ps: --x`, `--body is required and cannot be blank`), then that command's usage under `usage:`.
A command that could not reach the core prints the same next step for a code whichever topic ran it (`request_next_action`), and its stderr line stays the bare code, which the hooks' pre-tool guards read.

Every `AgentView` includes `origin`, the spawner id for either mode and null for ordinary roots.
Delegation derives origin from parent; handoff stores origin separately, and it grants no parent, subtree-close or end authority.
A handed-off agent's identical self-registration or `--check` preserves its id and stored origin, without taking an origin input or changing responsibility.
A handed-off agent can end its own registration; its spawner cannot end it and receives `parent_authority_required`.
Ordinary letters and explicit watches remain available between independent agents.
The sidebar, the Overview's tasks, ancestor unread state, descendant marks and waiting-on-descendants rule read only responsibility through parent.
PR and issue panels retain the handed-off session through its own branch facts and show no delegation line from origin.

A completed spawn stores a durable receipt for its caller and intent, so retries return the same agent and preserve ended registrations and closed watches.
The CLI waits up to 85 seconds for a spawn on its own machine, against 10 for other commands: the spawn's own answer time (`coordination::SPAWN_ANSWER_WAIT`, 45 s, below), because a Codex start with its brief commonly takes 10 to 15 seconds, the up to 30 seconds a spawn may wait for a kit pass on the child's machine (`coordination::HOOK_PASS_WAIT`, which moves the spawn's answer time by what it waited and which the CLI adds to its own wait so that refusal arrives first), and the ordinary 10.
A spawn that still answers `request_timeout` goes on without its caller: running the same command with the same intent returns its agent or its reason, and a spawn that stopped partway is logged as `spawn.unfinished` with the spawn and the reason.
Changing only responsibility mode on the same intent returns `intent_conflict` before creating anything.
Only incomplete intents resume their recorded creation and registration steps; starting a new watch after completion requires explicit `hide watch start`.
Remote starts through a caller's own device use Hide's existing device start path with the same caller, modes and focus rules.

Spawn creation keeps its recovery marker until the receipt contains the checkout, pane and terminal incarnation.
The created tab then takes the registered task name; a created workspace takes the checkout branch, or the repository name for its primary checkout.
The receipt reports separate workspace and tab naming outcomes (`pending`, `applied`, `preserved`, `failed`); a failed rename does not undo the child or its watch.
The existing lineage worker retries unfinished names once per connection, in bounded batches, without a separate poller.
Repair requires the receipt's device, checkout, pane and terminal to match the current unique target and its exact recovery marker.
A newly created workspace's first tab may carry the engine's generated label when shell replacement is unavailable.
Its creation response's exact tab id and label are saved with the first durable placement, so that unchanged label can be named too; an older receipt cannot infer this proof from a current default label.
User labels and targets with insufficient proof are preserved, and successful targets are not renamed again.
The engine's rename has no compare-and-set, so a user rename between the final read and write remains a race.
Completed intent replay returns the durable receipt without contacting the engine, including while a device is offline.

`--machine <device id>` starts the child on a connected device instead of the caller's own; it works with delegation and handoff alike.
The id is the `device_id` that `hide workspace info` shows and the `machine` that `hide agent list` shows for agents there, which is the device's machine id ([ARCHITECTURE.md](ARCHITECTURE.md#a-devices-id)); the SSH alias the device was added with names it too and is read as its id, an alias two devices share is refused with `machine_unknown` and their ids, a device label is not accepted, and the caller's own id is the same as leaving the flag out, so a spawn without `--machine` is exactly the spawn above.
`--repo` and `--path` are then paths on that device.
A spawn records the machine of its child only when that is not the caller's own, and the ledger refuses a child whose machine is not the spawn's.
A core move therefore reads the field against the caller's machine after the move (`node_migration::reown`): a spawn that names none stays so, a machine the move renames takes its new id, and one that is now the caller's own is dropped.
The copy is loaded with the ledger's own reader before it leaves the machine giving the core and again before it is placed on the machine taking it, so a move that would leave the new core without its letters, watches and registrations is rolled back instead of reported done.
The caller keeps the parent registration, the authority and the watch; the child's checkout (a worktree when the branch has none), tab, agent start, identity wait and lineage tokens are the device's, made through its engine, and the tokens name this machine as `parent_machine`.
The agent is registered under the device, so the sidebar and the Overview's tasks show it under its parent with the device's mark, its reports and inactivity warnings reach the parent as letters over the existing device paths, and a handed-off agent records the caller as `origin`.
Before the intent is reserved or anything is made, the spawn is refused with a stable code and a next action when the id is not a device (`machine_unknown`, naming the connected device ids the caller could have used), the device is not connected or its helper cannot answer (`machine_unavailable`), `--repo` holds no repository there (`repository_unavailable`) or the agent CLI is not on the device's `PATH` (`agent_not_installed`); the checks are the node link's `repository` and `agent_installed` calls, taken off the runtime lock.
The device is part of the intent: the same intent on another device, or on none, returns `intent_conflict`, and a spawn that already completed answers from its receipt even while the device is away.
A spawn on a device makes several calls over SSH, so its answer time is 60 seconds (`DEVICE_SPAWN_ANSWER_WAIT`), and the CLI waits that, the same kit pass wait and the ordinary 10; one that times out still finishes, and running the same command again returns the same agent (`spawn_busy`, whose next action is to wait and run the same command again, while the first still runs).
The checks of the device only read, so they run before the process-wide spawn lock is taken and a slow device does not make local spawns answer `spawn_busy`.
Creating is single-flight and the checks are not: each of the two calls waits up to 8 seconds, and checks of different spawns may overlap.
When the device's engine stops answering after the spawn was reserved, the answer is `machine_unavailable`, the cause goes to the diagnostic log with the device and the spawn, and the same command continues the spawn once the device answers again.
The CLI then answers `request_timeout` and says the request may still be running.
A retry of a spawn that already created its pane needs the device only to be reachable: the repository and agent checks ran in the first attempt and are not repeated.
`--machine` is allowed only from an agent on the machine that runs Hide; an agent on a device that names any other device is refused with `machine_not_permitted`, which lists no device.
A parent other than the caller is refused with `parent_authority_required` before any device is asked.
The receipt keeps the device as an optional field of the version 1 ledger, absent for every older record.
The ledger stays at version 1: older records without mode load as delegation, missing origin defaults to null and legacy `no_watch` data is ignored without changing existing watches.
The unsupported reconciliation/resume/session flags and relay, escalate, graph and events commands are absent.

`hide agent end <id>` ends the registration with the watches on it, then closes the pane it ran in on its machine, which ends the pane's holder, its shell, the agent and every program under them ([ARCHITECTURE.md: The pane holder](ARCHITECTURE.md#the-pane-holder)); `--keep-pane` ends only the registration and leaves the pane running.
The pane closes only while it is still the registration's: one the engine no longer has, or whose id now runs another terminal than the one the registration recorded, answers `"pane": "gone"`, one that shows another conversation now stays open with `"pane": "other_agent"`, and a pane of a registration from before Hide recorded terminals stays open with `"pane": "left"` when no agent shows in it, since nothing proves it is still the registration's.
A registration whose agent never reported its session (a Codex worker before its first turn) cannot be told from another conversation once its pane shows one, so such a pane answers `other_agent` and stays open with its holder; end it from the pane, or close the pane.
The close is the engine's own (`hide-core/src/coordination/end.rs`), as a spawn closes the panes of its unstarted attempts: it enters no Reopen closed, and a tab it empties closes and the operator's screen moves to its neighbour, as for any tab the engine closed.
The conversation that ran there is kept as an archived session of its own under 보관됨, where 이어가기 resumes it as a new root ([ARCHITECTURE.md: Session archive](ARCHITECTURE.md#session-archive)).
The answer says `"conversation": "archived"`, or `"none"` when no agent showed in the pane, its checkout is not listed or Hide does not know its session (the `agent.end_pane` line says which).
At 256 archived sessions, admission makes room from the oldest archived root at rest, taking its descendants with it through the existing delete path; only when no eligible roots can make enough room does the answer say `"archive_full"`, also logged as `agent.end_not_archived`.
Until both the plan and mission books have been read successfully, an admission needing eviction also answers `"archive_full"`, with `session_archive.eviction_unavailable` naming the unavailable book.
If an already queued delete fails, its record stays as on manual delete; the next admission removes enough eligible roots to recover the cap, including that retained excess.
A record Hide could not form, or an eviction whose deletes could not be enqueued, answers `"none"` and is logged there as `archive_refused`.
The pane closes all the same.
The conversation is archived before the pane closes, so a close that ends the pane but not the command still leaves it under 보관됨, and its row stands there once the pane left its machine's listing; a close that failed with the pane still there takes the record back, since the conversation runs on in it.
The session the agent registered with proves the conversation when Hide's label reader has not confirmed the session file yet, as for an agent ended right after its first turn.
The registration ends first, so a close the engine does not take answers `pane_close_failed` with the registration ended and logs `agent.end_pane_failed`, and the same command closes the pane; every outcome is logged as `agent.end_pane` with the agent, machine and pane.
The pane is read from the engine's topology, and its close ends the pane on its node within 50 s (`hide_node_link::link_panes::END_WAIT`: the holder's close, 20 s, then its tree's pids), which the CLI waits beyond an ordinary command (`END_ANSWER_WAIT`).
Every open pane holds one of its machine's 128 pane holders (`hide-kit/src/panes/reservation.rs`), so a finished agent ended this way gives its holder back, and one kept with `--keep-pane`, or asleep in its pane, keeps it.

A registration ends when the engine no longer has its pane, so its name and the watches on it do not outlive the pane; the ended record stays as history until room is needed at the 2048-registration limit.
At that limit a new registration drops the oldest ended registration nothing still names, together with the finished spawn records naming it; at the 4096-spawn limit a new spawn drops the oldest finished spawn record nothing still holds (`coordination::room`).
Oldest means first registered or reserved, the order the ledger keeps; neither record stores an end time.
A spawn has finished when its child ended, or, without a child, its parent ended.
Eligibility comes from the published ledger: a record ended or finished only in the batch being saved waits for publication before a later batch may drop it.
This keeps a live record visible to a concurrent mission binding until that binding can no longer be admitted; a batch with no previously finished eligible record still answers `capacity`.
Registrations named by other registrations, unfinished or held spawns, identity waits, human turns, open letters or watches stay, and the loaded mission and plan books protect the records they still need; their contracts live in with-hide/hide-private `docs/missions.md` and `docs/plans.md`.
A book not yet read or unreadable is handled conservatively: every registration stays until the mission book is known, and mission or plan spawns stay while their book is unknown.
Sleeping, archived and reopenable sessions keep registrations that have not ended, so they are never eviction candidates.
When nothing can go, the new operation still answers `capacity` and writes one diagnostic with the registration and spawn counts.
Only retained history can answer an old registration id or replay an old spawn intent; an evicted registration is unavailable, and a spawn intent evicted with its finished record is a new intent on retry, as for a dropped finished letter.
The core reads that from its own session sync of the host's engine, never from a separate poll: a pane the in-sync replica listed and then stops listing was closed, and a pane missing from the fresh session snapshot of a connect is gone for every registration made before that snapshot was asked for, which is how registrations left by panes that closed while no Hide was running end on the first connect.
Nothing ends on uncertainty: a stream that lost events, an engine restart and an unreachable engine publish no read until a fresh snapshot replaces it, a disconnected device publishes none, a sleeping or resumed agent keeps its pane, a session asleep with its pane closed keeps its registration and the watches on it until its wake continues them or lets them go, an archived session keeps them the same way until 이어가기 continues them or the archive is deleted, an agent closed through Hide keeps them while its Reopen closed entry is listed, a registration seen continuing in another pane keeps them until the store moves them there, a registration on another engine of the same machine is not judged by this one's read, and a registration made after the snapshot was asked for waits until a read lists its pane.
The delivery store ends it with the watches on it and the answer waits of the letters it sent or received, as `hide agent end` does, and logs `agent.ended` once with the agent id, machine, pane and reason (`pane_left` or `pane_absent`); nothing reaches the screen.
One other thing ends a registration whose pane is still there: its engine restored the pane after a restart, as a shell, and did not resume the agent that ran in it (a resume it could not type on either of its two tries, a resume it typed whose program ended without the agent running, a definition that says no resume, a conversation open in another pane; [ARCHITECTURE.md: The core's own engine](ARCHITECTURE.md#the-cores-own-engine)).
The engine names such an agent in its session until an agent shows in the pane, and the core puts the registration on the ones the store has still to end (`Runtime::end_unresumed`), logged as `agent.ended` with the reason `agent_not_resumed`: the pane runs nothing of it, so `hide agent show` reads it ended, where it read it running with its activity unknown and no letter or watch could reach it.
A registration whose agent is on screen in that pane now, that belongs to another conversation than the one that was not resumed (both name one), that another engine of the machine holds, or that a sleep record holds asleep is left as it is.
A session that sleeps by closing its pane saves the registration it held, and the pane that wakes it takes that registration over: the registration keeps its id, name, parent and watch, the letters addressed to or from it follow it to the new pane, and the delivery store logs `agent.continued` with the agent id and the pane it left.
An archived session's member (PRD session-archive) is continued the same way when 이어가기 brings it back ([ARCHITECTURE.md: Session archive](ARCHITECTURE.md#session-archive)).
An agent closed through Hide, alone or together with its parent (a subtree close), keeps its registration while its Reopen closed entry is listed, and the reopen continues it the same way in the pane its conversation resumes in, so it is the same agent under the same parent, with its letters and the watch on it ([ARCHITECTURE.md: Reopening locally closed work](ARCHITECTURE.md#reopening-locally-closed-work)).
While closed it reads `closed`, the watch on it is neither read nor warned about, it holds no place under its mission's or its plan's cap, and a letter to it is taken and waits with no bell, its hour running, until the reopen carries it to the new pane.
Reopen closed is the operator's act and is not admitted against those caps, as the operator's wake of a plan child is not: the agent it brings back counts again from the next spawn, which the cap then refuses if it is full (with-hide/hide-private `docs/plans.md` and `docs/missions.md`); no agent reopens a closed agent, since a letter to it only waits.
An entry that leaves the list unreopened, a reopen that started a new conversation instead of resuming it, and a restart of the core, which keeps no entry, let its registration end with its pane.
An agent whose CLI runs no session hook on resume (Cursor) gets its session from the core: the wake worker reports to the engine the id the pane resumed, so the same-session check below has a session to compare ([ARCHITECTURE.md: Agent sleep](ARCHITECTURE.md#agent-sleep)).
The core decides this from the wake it started, only when the new pane runs the same native session, kind and device; any other pane is not given the registration, and a registration it cannot continue ends with the pane that held it.
Removing a device retires its cached pane read and rejects late reads from its retired coordinator; removal does not prove its panes gone or end any registration or watch.
Only the coordinator sharing the currently installed remote control connector may begin a pane read, publish delivery observations or replace the remote session and connection status.
Each check holds the runtime lock through its write, including a second check after projecting a remote session; replacing the coordinator for the same device ID rejects the old one's late snapshot and failure, while the current coordinator may bootstrap before its first connected status.

A spawn whose child shows no session within 5 seconds (Codex before its first turn, OpenCode 2 before its first prompt) answers `native_identity_unavailable`, and the core binds the child when the engine shows the session ([A registration that waits for its agent's session](#a-registration-that-waits-for-its-agents-session)).
A spawn's tab, like the first tab of a new worktree the web starts an agent in, runs the account's shell with Hide's shell integration where that shell is zsh, bash or fish (Windows PowerShell on Windows), and its start is typed only after the shell marked its first prompt ([ARCHITECTURE.md](ARCHITECTURE.md)); a shell that has not finished starting (marked its first prompt, handed the terminal back and readied its line editor) within what the spawn's answer time leaves it, at most 30 s, is refused as `shell_starting` with how many whole seconds the wait it was given came to (that wait, rounded up, never the clock past it, so a 30 s wait reads 30) and the pane's last line, when it shows one, and nothing typed; the `shell_wait.timeout` line says which of the three it waited on.
The CLI waits for a spawn's answer as long as the core gives itself, one constant both read (`SPAWN_ANSWER_WAIT`, 45 s, or `DEVICE_SPAWN_ANSWER_WAIT`, 60 s, on a device, plus the CLI's ordinary wait), so the refusal reaches the caller instead of a `request_timeout` while the shell still waits; the waits after the start, for the agent's session and for it to take its brief, take what is left of that time, so an agent that never takes input is answered `brief_not_submitted` in time as well.
The spawn then closes the tab it opened, unless a start was already typed there, and takes the pane off its spawn record, so the same command with the same intent opens a fresh tab; the next action is to find what the account's shell startup files wait on by starting a login shell in a terminal of one's own, then run the same command again.
A start of a profile whose program did not start (`profile_refused` after its line was typed: the system refused the program past every check, such as a binary for another processor; docs/ARCHITECTURE.md, Starting an agent with a profile) leaves its pane a shell holding nothing of the start, and the spawn closes that tab the same way, so a failed spawn leaves no shell behind; one whose result is not known (`profile_unconfirmed`) keeps its pane, where the program may run.
A spawn refused for its profile, before its pane or after, answers the profile's own code (`profile_gone`, which a profile being removed answers too, `profiles_unsupported`, `profile_refused`, `profile_unanswered`, `profile_unconfirmed`, `launch_records_full`) with the machine's or the executor's sentence as its detail, and a next action of its own: only `profile_unanswered` asks for the same intent again, since the others fail the same way until the profile or its program changes.
The engine refuses a start as `agent_pane_busy` while anything besides the shell holds the pane's terminal; on Windows that is any child process of the shell, which the pane's process list does not name unless it is an agent, so Hide's own reading keeps saying ready and only the engine's next answer can say the pane is free.
The start asks the engine again once every 100 ms inside the same wait, never in a burst (on Windows CI on 2026-10-22, asking with no pause sent 5,613 starts in 30 s while the spawn held the machine's spawn lock), logs `agent_start` / `pane_busy.timeout` with the count when the wait ends, and the spawn is refused `pane_busy` with nothing typed; the next action is to let that program finish or end it in that pane and run the same command again with the same intent, which starts in the same pane.
A start the engine typed whose agent never showed (a startup question took its first letter in a bash pane, or the program exited before the engine saw it) leaves the engine holding the agent's name in that pane until its 120 s deadline, and the engine gives that name to any agent that appears there meanwhile.
A spawn waits up to 30 seconds for the engine to create the worktree, which runs `git worktree add` and can be slow on a large repository under load; when no answer comes in that time the spawn is refused `worktree_unconfirmed`, because the engine may still finish the worktree, and the next action is to run the same command again with the same intent, which finds that worktree and continues in it.
A Codex start on a machine whose install kit has not read its Codex is refused `codex_capability_unknown` with nothing typed, because every Codex Hide starts takes `--no-daemon` only when that Codex has the shared server ([agent-hooks.md](agent-hooks.md)); the next action is to have the operator press Check again in Settings > Agents for that machine and run the same command with the same intent, which continues in the pane the spawn made, and the screen words the code in the operator's language wherever it shows a start refusal; why the read failed is logged once by the core as `kit` / `codex_capability_unread` with the machine's device id, the cause, and the exit code or a signal, never a path or Codex's own output, and not at all when the read stopped because Hide was quitting.
Before anything is made for the child, on this machine or a device alike, the spawn judges from that machine's last kit report whether the agent would start with Hide's hook (`runtime::start_hook`): a child started without it never registers, and a Codex that has not trusted Hide's hooks stops at its "Hooks need review" screen and takes the brief as its own keys (with-hide/hide-private#661, #662).
Where a kit pass may put the hook in place (a hook not yet in or outdated, an agent folder that may have been made since the last pass, a Codex whose trust the last pass could not record, a machine whose first pass still runs), the spawn asks that machine's kit worker for one pass, waits off the runtime lock up to 30 seconds for its answer, and judges again.
What is still not in place is refused with nothing made or typed: `agent_hook_off` (switched off in Settings > Agents for that machine), `agent_not_set_up` (the agent has not made its own folder there; start it there once and sign in), `agent_hook_untrusted` (choose Trust all and continue on Codex's Hooks need review screen there), `agent_hook_missing` (press Reinstall for that machine in Settings > Devices), `agent_hook_unconfirmed` (the pass did not answer in time) and `agent_hook_unchecked` (the kit could not be read or its pass stopped, which that machine's row in Settings > Devices says); each next action ends with running the same command again with the same intent.
A kit that should run and gave no answer is never read as ready: a failed read or a stopped pass asks for a pass like a missing hook, and a pass whose call fails leaves the last report beside its failure, so the start is refused as unchecked rather than for what that older report said.
Desk and Memory, which start their agents through a spawn, show every one of these as one line saying the agent's hook is not ready on that device (`agent_hook_not_ready`), and a surface that words a start's reason says to check that device in Settings.
Each refusal is logged as `spawn` / `hook.refused` with the device, the agent kind, the intent and the stage that decided it (`report`, `pass` or `wait`), and a wait that ran out also as `hook.pass_unanswered`.
A machine whose kit does not run by design or by the operator's choice (no consent or an older one, an unsupported platform, a daemon outside the app) is not refused here, nor is a helper that predates agents, and an attempt that already typed its start is not judged again.
Running the same intent again while the name is held answers `native_identity_unavailable` and types nothing; once the engine lets it go and the shell alone holds the terminal it types the start once more (`start.resent`), and a second start that never showed, or anything else in the foreground, answers `agent_not_started`, whose next action is to look at that pane and spawn with a new intent.
A spawn about to start its agent closes the pane of every earlier spawn of the same parent, name and device that made a pane and never bound a child, where no agent shows and the shell alone holds the terminal, so a new attempt leaves no extra tab and the engine no longer holds the name for it; each close is logged as `spawn` / `unstarted_attempt.close`, and a pane that cannot be read or closed is left to the engine's refusal.
A spawn records the terminal its pane runs (`terminal_id`, as the engine reports the pane) together with the pane, because an engine whose saved session is lost hands a pane id out again, so an earlier spawn's pane id can name an unrelated shell the operator opened.
An earlier pane is closed only while the engine still reports the terminal its spawn recorded; a pane id now running another terminal is logged with the reason `pane_replaced`, and a spawn record written before terminals were recorded with `pane_identity_unrecorded`, and both panes are left open.

The existing one-second `agent.list` refresh, also requested by native events, reconciles only panes whose four lineage tokens differ; startup and reconnect perform one full pass.
The same pass writes the read axis's server mark (`hide_server`) on a pane whose saved read record or completion names the current connection's mark and that does not carry it, in the pane's lineage report when it has one ([status-model.md](status-model.md#across-an-engine-restart)); a change in which panes want a mark replans like a moved registration.
Each server's pass reads the registrations recorded under its machine: this machine's are keyed by its node id, the same `machine` that `hide agent list` shows, and a device's by its device id, so a token the engine lost is written again on this machine as on a device.
An unchanged native observation and append-only registration count skip planning; unrelated letter/watch writes and label/process/catalog publications do not start a lineage pass.
The native pane and positive session select the matching retained registration, including an ended one, independently of append order.
A positive child-session replacement clears stale tokens; an absent native session or agent end leaves the pane's tokens in place, while unchanged readers reject obsolete session identities.
There is no separate lineage timer, subprocess on the input path or blocking work under `Mutex<Runtime>`.
The lineage token readers and digest contract remain unchanged; the server mark is read only by the read axis (`wire::server_mark`).

People receive notifications only for an unanswered parent inactivity warning, an overdue undelivered letter and a mission node that waits on the operator with no Needs You row to lay it on ([Missions](#missions)).
A first warning left unacknowledged, uncancelled and unreplied for 60 minutes triggers one human notification per native target and inactivity episode, shared across sibling watches and sent through the phone's Web Push.
The receipt lives on the existing warning letters and survives daemon restart and watch stop/restart; new target activity starts a new episode.
The second agent warning introduces no second human notification schedule.
An overdue letter's notification key is its ID plus the cause and sends once.
A phone that cannot be reached is not sent to, and a failed send records a diagnostic without retry.
The phone is skipped before any send when its desktop pause choice is on while an attached desktop host reports system input within ten minutes, unlocked and awake, and an attached desktop window enables Needs You notifications (`computer_in_use`), Mobile is off (`mobile_off`) or no paired phone has a Web Push subscription or an iOS app registration (`no_subscription`); the first that applies is the reason, and a send that fails (a Web Push phone while the push key is missing included) or finds every subscription gone is `send_failed` or `subscription_gone`.
The core's own engine shows no notification, so the phone is the one channel.
When the phone was not reached, one `human.channels_failed` record in `Logs/core.jsonl` carries the notice (`letter_undelivered`, `observer_unconfirmed` or `mission_attention`), the letter id and `push` (the reason); it never carries a letter's text, a phone endpoint or a key.
The claim stays consumed, so the same letter or warning is never announced again, and nothing is drawn on screen: an operator with phone push off learns of a held letter from that record only.
These cases add no Inbox screen or automatic escalation chain.

### A registration that waits for its agent's session

An agent Hide started is registered from the session the engine shows in its pane, and some agents show none until their first turn.
Codex 0.163.0 runs the session hook that Hide's integration reports from only when its first turn starts, for `codex fork <id>` as for a new start, and OpenCode 2 makes no session before its first prompt; Claude Code runs its hook when it starts, so its spawns and forks register at once.
Instead of failing after a fixed time, such a registration waits in the ledger (`identity_waits`, `coordination::identity_wait`) until the engine shows the session, and the core finishes it then.

`hide agent spawn` still waits up to 5 seconds for the child's session.
The agent runs by then, so an agent list the engine does not answer inside that wait is logged as `identity.read_failed` and read again, never taken as the spawn's failure: a spawn that ended on that read left a running child that no wait and no registration tracked.
When none shows in that time it records the wait and answers `native_identity_unavailable`, an error with no agent, whose next action says the agent started and Hide registers it when it reports its session; the pane and the agent stay as they are, and nothing is typed again.
Until the child is bound, `hide agent list` does not show it, the parent has no watch on it and its pane carries no lineage tokens.
Once the engine shows the session in the spawn's pane under the spawn's agent name, the lineage writer binds the child as the spawn's own answer would: the registration with its parent, or for a handoff its origin, the initial watch of a delegated child, and the lineage tokens on its pane, logged as `identity_wait.bound` with the wait, the agent and the pane.
Running the same command with the same intent then answers that child from the spawn's receipt and types nothing.
Asked again before the session shows, it answers `native_identity_unavailable` and keeps the one wait; asked after the session shows but before the core settled the wait, it binds the child itself, and the core's settle finds that same child, so either order leaves one registration and no wait.

Fork agent records the wait once the fork's agent has started: the source is registered then, while it is the execution that was forked, and the fork once the engine shows the fork's own session, as the operator's root whose origin is the source ([status-model.md](status-model.md#where-a-parent-comes-from)).
The source's consent is the wait, so a fork whose source closed while the fork rested still names it as its origin.

A wait ends without a registration when its pane closes (`pane_closed`), when its agent ends and the engine no longer lists the agent's name in the pane (`agent_ended`), or when another session shows in that pane (`pane_taken`).
A bind the ledger refuses ends the wait with that refusal, such as `parent_unavailable` for a spawn whose parent registration ended, and so does a registration that cannot be made of the agent as the engine shows it, so nothing is asked again for an agent that cannot be registered.
A late session past what a registration keeps ends the wait as `session_too_long`, and the pane a spawn made for it is closed, as the spawn's own answer closes it; a fork's pane is the operator's and stays.
Each end is logged as `identity_wait.ended` with the wait and the reason, and for a spawn's wait the spawn and what became of its brief (`SpawnRecord::brief_phase`: `none`, `not_typed`, `confirmed`, or `sending`, `typed`, `launched`, `queued` or `lost` with `_unconfirmed`, never "undelivered" for a brief that may have arrived); `agent.ended` says the same of a spawn's registered child, and nothing reaches the screen.
A bind of a spawn whose brief is not typed yet registers the child and leaves the spawn open, so the same intent types the brief once and completes it.
An agent's absence is judged only by an agent list read after the wait was recorded, because a list asked before may predate the agent's start.
A settle that failed, such as one the store did not take while a core move held its effects, is logged as `identity_wait.settle_failed` and asked again at the next agent list, as a failed token write is; a settle converges, so a repeat binds or ends the wait once, and a queued settle is never asked twice.
A core move carries each wait with the machine it names (`node_migration::KEYS`), so that machine's lineage writer judges it under its new id.
At most 64 waits are held at once: a spawn past that answers `identity_waits_full`, whose next action is to give one of the waiting agents a first prompt or close its pane, and a fork past it is logged as `registration_failed` with that code.
The waits ride on the existing one-second agent refresh, with no timer, no engine read of their own and no work under `Mutex<Runtime>`; an unchanged list with unchanged waits plans nothing.
A ledger with no wait writes no `identity_waits` field, and a build from before waits ignores the field.

## Pane commands

`hide pane read <pane> [--lines <n>]`, `hide agent read <agent> [--lines <n>]`, `hide pane send <pane> [<text>] [--key <name>]... [--enter] [--paste]`, `hide pane open --beside [--focus] -- <command>` and `hide workspace purpose <text>` are an agent's reads of and input to the panes of its machine, and its checkout's purpose (PRD engine-bundle-1 D-5, D-13).
They need the running daemon and a pane-bound caller, like the letter commands; a caller outside a Hide pane is refused by the credential boundary, and a checkout-bound one with `pane_required`.
The daemon admits the caller's pane and checkout on the core's owner thread (`Runtime::prepare_pane_command`), and the engine calls run on the caller's task, off the runtime lock, through the engine boundary (`hide_core::pane_command`); nothing they do is recorded in the ledger.

- They reach every pane of the machine they name: a bare pane id is a pane of the caller's machine, as `hide ps` lists it, and `remote:<device>:pane:<id>` a device's; an agent is named as a letter's target is, by its name, registration id or pane, and two agents of one name are `target_ambiguous`.
- A read prints the pane's last 120 lines unwrapped and without styling (`recent_unwrapped`), at most 2,000 (`lines_out_of_range` past it); `--json` prints the answer line `{"ok":true,"value":{"pane","text","truncated"}}` instead.
  `hide pane read <pane> --source visible --format ansi [--json]` reads the whole visible screen with its physical rows and styling through the engine's existing `read_screen` operation, without taking over its terminal connection.
  Both options are required together, and this whole-screen form refuses `--lines`; the ordinary read and `hide agent read` keep their defaults.
- `hide pane send` types the text, then presses the keys in order, then Enter, and answers `{"pane","text_sent","keys_sent","enter_sent"}`: what the engine took, never what the agent did with it (`enter_sent` is an Enter written, not a prompt taken).
  The keys are Hide's own names, `esc`, `enter`, `tab`, `shift-tab`, `up`, `down`, `left`, `right`, `backspace` and `ctrl-c` (`engine::keys::PaneKey`), which the engine turns into the keys the pane's program takes; at most 32 a send (`too_many_keys`).
  Text without `--paste` is one line with no control character (`text_invalid`), since a line break or an Escape would act as a key nobody named; with `--paste` it keeps its line breaks (each sent as a CR, as a terminal pastes) and tabs, never an Escape that could end the paste early, and goes in as one paste, which the engine brackets when the program asked for bracketed paste.
  A paste is taken only by an agent the agent journey's `composer` cell measured taking one whole (`AgentId::takes_one_paste`): every agent but Grok on Windows, and nothing on a pane with no agent or a device whose system Hide does not know; anything else is refused `paste_unsupported` before anything is sent, never typed line by line in its place.
  Keys alone or Enter alone are a send; nothing at all is `nothing_to_send`, and every limit is checked before any write.
  Sends to one pane run one at a time, and a send to the pane of an agent being put to sleep is refused `pane_asleep` (the input gate, [ARCHITECTURE.md](ARCHITECTURE.md#agent-sleep)).
  Right before its first write the send asks the two rules a held letter waits on: input in the pane since its last submission that no send wrote (`Observation::person_draft`) and text the agent's composer reads now (`composer::forced_clear`, [Forced letters](#forced-letters)); only both together, a draft a person may still be writing, refuse it with `composer_not_empty`.
  Whose input is whose is counted, not timed: every input noted in a pane raises its `input_generation`, and a send's own write marks the generation it left (`pane_send_generation`), so text a send left in the composer does not hold that agent's next keys, and a key a person types after it, or while the write was on its way, stays theirs (`Runtime::note_pane_send_write`).
  The send reads the generation before its composer read and checks it again right before each write; input that came in between stops it there with `operator_typing`, nothing more written.
  Measured on Codex 0.166 on 2026-11-01: `/hooks` with `--enter` in one send left `/hooks` in the composer, and a second send of `--key enter` alone opened it; the text's keystrokes and the Enter reach the engine as two writes milliseconds apart, as they did before keys existed.
  A screen no one typed into since the agent last took input, such as a hooks review, a trust question or a menu, takes it even where the screen reads as text, as Codex 0.166's hooks review does by drawing its list cursor `›` where its composer prompt would be, since that is the screen a send exists to answer; a pane with no agent is not read.
  That is the only refusal for what the pane shows: a send is never refused because the agent asks a question, an approval or a trust, so one agent may answer or approve for another.
  Hide does not certify that consent; the operator decides how much agents approve through each agent's permission choice in Settings > Agents.
  The text and the keys are two writes: keys the engine refused after it took the text answer `keys_not_sent`, a write whose answer was lost `send_unconfirmed`, since it may have landed, and a person's input between them `operator_typing`; each of the three carries `sent`, `{text_sent, keys_sent, enter_sent}`, what is known to have landed (`pane_command::PARTIAL`), in the CLI's `error.sent`. A refused first write is `pane_unavailable` with nothing sent.
  Every send is logged as `pane_command.sent` with the caller's pane, the target pane, the text's byte count, the number of keys, `enter`, `paste` and the outcome, never the text or a key name.
  Typing into an agent's pane is input, not a letter: it has no reply, retry or record; each write the engine took, or whose answer was lost, is input routed to the pane (`pane_send`), which Hide's draft guard and the quiet period count, and only a send whose every write landed answered the agent's screen (`Observation::answered_at_unix_ms`), the answer after which a brief the agent has not confirmed may be typed again ([Input an agent did not take](#input-an-agent-did-not-take)); a send that stopped partway or lost an answer answered nothing.
- `hide pane open --beside` splits the caller's pane to its right, unfocused, in the caller's checkout, waits up to 10 s for the new shell to hold its terminal alone, as an agent start does, types the command as one line of at most 1,000 bytes (`command_too_long`, under the shortest terminal line a supported system keeps whole) and answers the new pane's id; `--focus` then focuses it.
  One word after `--` is the command line as written, so `-- 'npm run dev | tee log'` keeps its shell syntax; several are one argument each, and the CLI quotes every word a shell would expand for its own machine's shell (a POSIX shell, PowerShell on Windows, `install::command_line_for_shell`), so `-- sh -c 'echo a b'` keeps its three arguments.
  On Windows that is PowerShell, the default shell of a pane; with another `default_shell`, such as cmd.exe or Git Bash, pass the command as one word ([INSTALL.md](INSTALL.md#what-each-system-supports)).
  A split whose answer never came is `pane_open_unconfirmed`, and a shell that did not take the command is `shell_not_ready` with the pane open.
- `hide workspace purpose` writes the checkout's purpose token on its engine workspace, the one the checkout row's Set purpose writes, at most 40 characters on one line (`purpose_too_long`, `purpose_invalid`).
- Every refusal is one line, `{"ok":false,"error":{"code","message"}}`, whose message is the next step, with a non-zero exit; a limit is checked by the CLI before it asks the daemon and by the daemon again.
- They work the same on macOS, Linux and Windows: each step is an engine call, and the command line is typed into whatever shell the pane runs.

## Removing a worktree: `hide workspace remove`

`hide workspace remove <path> [--delete-branch] [--force]` removes a linked worktree of the caller's machine the way the delete dialog does ([UI_BEHAVIOR.md](UI_BEHAVIOR.md)): it closes the checkout's panes in the engine first, so the engine's workspace for it closes with them, and then Git removes the folder.
A plain `git worktree remove` deletes the folder and leaves that workspace and its panes open, and after a restart they come back with a directory that is gone; an agent removes a checkout Hide shows with this command instead (`.claude/skills/merge-green/SKILL.md`, step 5), and keeps `git worktree remove` for a temporary worktree Hide never opened, such as `scripts/premerge-check.py`'s.

- It is the dialog's own removal, not a second one: the runtime admits the caller under its lock (`Runtime::worktree_remove_command`), answers the dialog's questions from the command's switches, and starts the same `remove_worktree` path with its deletion gate, the host's recheck, the pane close and Git (`Runtime::start_git_worktree_removal`, [ARCHITECTURE.md](ARCHITECTURE.md)).
  The daemon then waits on the removal's record until it ends (`hide_core::worktree_command::answer`) and answers `{"removed","repository","branch","branch_result"}`, where `branch_result` is `kept`, `deleted` or `remains` (Git refused to delete it).
- The path is resolved from the working directory and followed through links when the folder exists, so it names the folder Git lists; a folder already gone is named as written, and a row whose folder Git no longer lists, which only its panes keep, is closed the same way, which is how a workspace a raw `git worktree remove` left behind is cleaned up.
- Without `--force`, work the folder would lose (uncommitted or untracked files, a worktree or an ignored repository inside it, a status Git could not read) is refused `worktree_remove_needs_force`, naming what would be lost; `--force` accepts it, as ticking Discard does, and Git removes with `--force`.
- `--delete-branch` takes the local branch with the folder; a branch with commits its base lacks is refused `worktree_remove_branch_unmerged` unless `--force` also accepts that loss (`git branch -D`), and a base or unnamed branch is refused `worktree_remove_branch_kept`.
- An agent still working, starting, waiting, or asking a question or an approval in the worktree is refused `worktree_remove_agent_working` with its name; a finished, idle or stopped agent's pane closes with the worktree, as the dialog closes it.
- The main checkout and a locked worktree are refused `worktree_remove_blocked`, the caller's own checkout `worktree_remove_own_checkout`, and another removal in progress `worktree_remove_busy`; nothing is closed for any refusal, and none of them reaches the operator's screen.
- A removal that stopped partway answers how it ended, as the dialog's notice does: the host's reason (`worktree_remove_stopped`), programs still running in the folder after its panes closed (`worktree_remove_still_running`, with their names and pids), a pane that appeared or a worktree that changed while its panes closed, a close the engine did not confirm, a host that could not be asked, or a result that never came.
- Once Git said it removed the worktree, what is on disk decides: the host lists the worktrees again and looks for the folder, and when that list cannot be read, as on a nearly full disk, the folder alone decides, so a folder that is gone answers removed rather than `worktree_remove_stopped` with a worktree said to remain (with-hide/hide#2296, `hide_host::worktrees::remove_worktree_stopping`).
- Running the same command again converges: a removal of that path still running is waited for rather than started twice, and one that already ended answers its end; the daemon waits at most 280 s (`REMOVE_WAIT`), past the host's own bounds, and a removal that outlasts it answers `worktree_remove_pending` and keeps running.
- On a connected device the path is that device's, its engine closes the panes and its helper rechecks and removes, as the dialog does there; the command needs no open shell and works the same on macOS, Linux and Windows.

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
A registered agent's state is its life first, the one `hide agent list` gives ([Agent registration and spawning](#agent-registration-and-spawning)): `ended` for an ended registration, then `disconnected` on a device that does not answer, then `asleep` while Hide holds it asleep and `closed` while its pane is closed for Reopen closed, and the row's mark only while it runs; a live registration no row draws reads `unknown`.
A device's projects carry `@ <device>` on their line; its panes are the last ones Hide had from it while it does not answer.
After the tree, `Agents not in a pane` lists the live registrations no pane on screen holds (a sleeping agent, an older registration of a pane that now runs another session), and `--all` adds `Ended agents`, the ended registrations.
`--here` keeps every pane of the project of the pane it runs in, with its agents' delegated agents wherever they live (a pane of the project whose parent lives in another project is drawn in its own tab), and is refused `here_unavailable` from a caller with no pane in a project Hide shows.
`--json` prints the same answer as one line in the `agent` envelope, `{"ok":true,"value":{"panes":[…],"agents":[…],"missions_unreadable":false}}`, flat rather than nested: each pane carries its machine, project, checkout, tab, `you`, and its agent with `parent_machine` and `parent_pane` for the nesting; its schema is the contract's `ps` answer.
The tree uses box-drawing characters on every system: Rust writes UTF-8 to a pipe and UTF-16 to a Windows console, so no console Hide supports needs a fallback.

The answer is what the core already holds: the navigator's projects, tabs and agent rows, each device's last session, the mission map and the delivery ledger.
It asks the engine and the devices nothing; under the runtime's lock it copies one row per pane and per agent row, and the ledger join and serialization run after the lock (`hide-core/src/runtime/ps.rs`).
Like `hide agent catalog`, it needs the running daemon and a Hide credential but no open renderer, and a caller that is not an agent pane gets the same answer with no `you`.

## Missions

Moved to a private document: `docs/missions.md` in with-hide/hide-private.

## Plans

Moved to a private document: `docs/plans.md` in with-hide/hide-private.

## Verification

Use the worktree-owned entrypoints described in [BUILD.md](BUILD.md#two-entrypoints).
Focused checks are:

```sh
bash scripts/verify-cargo.sh test-scoped -p hide-core --lib delivery:: -- --nocapture
bash scripts/verify-cargo.sh test-scoped -p hide-session --lib session_activity -- --nocapture
bash scripts/verify-cargo.sh test-scoped -p hided --test it node_session_activity::
```

A filtered run must execute the expected named tests; zero selected tests is a failed check.
The remote mailbox lane (`hided/tests/it/remote_delivery.rs`) waits for the daemon's own doorbell to ring each recipient and hands that pane's prompt hook the line the letter kept, so it crosses the bell's recognition over the node link; its provider panes carry one finished Claude conversation each, because the doorbell rings a Claude Code pane only once its session read says nothing waits.
The helper executable tests use private homes and fixture transcripts, pair JSON-line responses by request ID, and cover activity success/refusal, privacy, kit coexistence, normal exit and abrupt owner loss.
Each fixture launch and capture shares an absolute five-second deadline and a combined 64 KiB output cap; provider sessions and the installed helper are never used.
The full Rust test and lint lanes still apply to the final committed head.
Actual Linux and Windows OS-contract runner results are required for the state-machine, ledger and activity portability claim; declaring a workflow does not prove it passed.
On Windows the `windows check` lane opens the ledger from a real state folder, registers two agents, sends a letter and reads it from the recipient's inbox and back from disk (`runtime::delivery::tests::agents_register_and_a_letter_reaches_the_inbox_through_the_saved_ledger`), and its daemon must have saved its ledger at startup.
Real TUI delivery requires an isolated engine, private HOME/state, precisely identified candidate processes and disposable provider sessions with observed native identity and registered lineage.
Register only an actual parent session and spawn its child through `hide agent spawn`; never seed a capability or coordination ledger.
Exercise idle/done delivery to a Claude pane with a statusline and to a Codex pane, a letter whose first line carries `@`, `$`, `/`, `!`, `#`, backticks, a trailing backslash and more than 60 characters of Korean (its bell is submitted as one prompt and its hook takes the letter), the same line typed by a person before the doorbell rings (an ordinary prompt: the letter stays pending until its own bell), working delay, a pending letter held while a permission, question or plan menu is open and delivered after it is answered, each target's menus read as engine `blocked` or, for Codex's plan approval, held as `awaiting_operator`, hook confirmation/restart, capacity/corruption, watch clocks/reset/exit/reply and helper privacy/fallback.
Label protocol fixtures separately from actual provider runtime observations.
Measure matched baseline/candidate input latency and idle/driven load through [PERFORMANCE_TESTING.md](PERFORMANCE_TESTING.md); a headless or socket-only check does not prove native presentation.
Own every fixture process with a deadline and teardown, preserve the operator's app/server/panes/hooks, and remove private authentication caches after the owned agents exit.
Run evidence stays under ignored `agents/runs/`; it is never committed with this guide.

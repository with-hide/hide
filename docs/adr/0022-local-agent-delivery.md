# 0022. Local agent delivery and inactivity watches

- Status: Accepted
- Date: 2026-10-04
- Code: `hide-core/src/delivery/`, `hide-core/src/runtime/delivery.rs`

## Decision

### Local agent delivery and inactivity watches

Receipt-proven spawn names use the existing lineage worker and delivery writer.
An append-only receipt count and placement revision guard naming admission, so unrelated letters and unchanged native observations build no naming plan.
Each batch admits at most 32 receipts, with one active and one pending batch and at most 4,096 attempted receipt ids per connection.
Each receipt needs at most four one-second engine calls and a five-second durable checkpoint wait, all outside Runtime and the session-sync thread.
The worker checks shutdown between receipts; stopping can wait for the current receipt's bounded calls.
Workspace and tab outcomes are persisted separately from spawn completion, and completed replay does no naming I/O.

The core's delivery worker owns a separate private ledger and returns mutation success only after atomic persistence outside `Mutex<Runtime>`.
Runtime holds an immutable ledger projection; one bounded writer queue and separately owned doorbell/watch producers stop and join with Core outside the lock.
Beside it Runtime holds the letter index the turn card reads (`delivery::correspondence::Correspondence`): each pane's letters and started agents by time, interim reports apart, each line keyed by the agent on its other side, built by the delivery worker from the ledger it just saved, before it takes the lock, so the lock compares one digest and lays the rows again (`sync_request_rows`) only when the digest moved; a delivery step that changes no card line, such as a bell or a receipt, lays nothing.
The existing pane input path adds only timestamp assignments (the last input, from a key, paste, phone write or agent-find key; a submission is stamped only by a phone reply or a prompt hook of the pane's own session); the observation adds the time the pane entered `working`; watch snapshots publish start, warning and end transitions.
The daemon exposes typed `hide request`, `inbox` and `watch` commands through the existing caller-bound Workspace boundary without requiring a renderer.
The doorbell decides from hide-owned facts only (`doorbell::judge` over the pane's observation, never a screen read) and writes at rest or after the existing mid-turn patience, and prompt-hook stdout flush precedes durable intake confirmation.
The observation carries what the pane's session read says it waits for in the engine's current state (`doorbell::Turn`), taken from the label overlay the coordinator computes before it observes the payload; an agent whose read reports turns is held while a native question or plan waits (`awaiting_operator`) or while no read settled the current state (`session_unread`).
The doorbell's pass also decides which panes show letters waiting on a possible draft (`doorbell::letters_waiting`), in memory under the short lock it already takes to publish its holds and bounded by the open letters, and publishes only when a mark appears or goes; the shell's Deliver now is one event that releases the draft and leaves the ringing to the next pass ([delivery.md](../delivery.md#delivering-a-held-letter-now)).
The doorbell also reads the composer of a pane it holds on a possible draft, once per observed draft and state generation, after input is quiet at rest or working, and only for an agent version whose composer check passed: the pane read (`Engine::read_screen`) runs on the doorbell's thread off the lock, within its eight work attempts per pass, is judged by the adapter's composer declaration (`hide_agent_adapter::composer`), and a positive `empty` releases the draft as Deliver now does, so the verdict itself still reads no screen ([delivery.md](../delivery.md#reading-the-composer)).
Before an eligible typed bell, an agent with a composer declaration must also read empty through the same guard as a forced letter, even when the draft record is clear, because a delayed status update may cover a key; this read is off-lock, within the existing per-pass budget and retry backoff, and the native and input-generation checks still run before the write.

An agent's readiness is three-valued at the engine boundary (`engine::answers::Readiness`): only a launch in progress or an explicit not-ready refuses, and the unreported readiness of an agent started by hand is decided by those facts; a refusal after the verdict is retried on a capped backoff off the lock ([delivery.md](../delivery.md#safe-intake-and-manual-fallback)).
A letter has no delivery deadline: it ends delivered, superseded, `no_recipient` when its agent ends or its pane closes with no agent after it, or `expired` only at the update (PRD letters-missions-simplify D-04, D-15); watch warnings use 20-minute inactivity, one tick for a target its own row shows stopped on a question, and first-warning time plus 60 minutes, with persistent counts and activity reset; a child held at a blocked prompt is told to its parent at once and raised to the operator after a minute instead ([delivery.md: A blocked child](../delivery.md#a-blocked-child)).
The helper's metadata-only session activity uses the same native ownership proof as the local reader.
A confirmed report to its parent ends the sender's watch immediately; an unanswered parent warning uses the existing human notification path once per episode, and a letter that ends `no_recipient` is its sender's alone (B7).
Each letter keeps a durable `hook_confirmed` receipt separate from cancellation.
Acknowledgement can come only from the recipient's own pane and session, so it is that receipt; it still counts as checking a watch warning and does not schedule another doorbell.
A letter an earlier build acknowledged with `hook_confirmed: false` stays in the bell turn's batch after the pending letters and counts as open until its agent ends (`Ledger::end_letters_to`) or a receipt comes.
Automatic doorbells remain limited to pending letters.
The first receipt ends a matching report watch; an actual post-flush confirmation still does when cancellation, a newer version or its agent's end arrived between pull and confirmation.
Cancellation and every end prevent a new pull; an already-printed in-flight confirmation remains valid, preserving cancellation and supersession while changing `no_recipient` to delivered.
Replayed confirmation after restart preserves a watch explicitly started after that first receipt; a report whose acknowledgement left no receipt does not suppress spawn's initial watch.
Legacy letters with an absent or null field prove confirmation only in `delivered` state; an older `acknowledged` record cannot distinguish manual acknowledgement from confirmed intake.
Older acknowledged records remain excluded from pull and keep their existing closed-state retention rules unless they still await a reply; the missing receipt never becomes delivery proof.
The same core owns registration, spawning, lineage, the mailbox and inactivity watches; no external coordinator or conflict probe remains.
A registration ends when its host's published session stops listing its pane: each publish hands the published pane set to `Runtime::observe_delivery`, which compares it with the previous one only when it moved (`coordination::gone_registrations`), and the delivery store ends what it found in its next transaction; a connect forgets the host's panes and notes the ledger's next id before asking for the snapshot, so a snapshot judges only registrations made before it.
[delivery.md](../delivery.md) owns the CLI, capacities, privacy, completion reports, remote delivery, manual fallback and the residual external-input race.

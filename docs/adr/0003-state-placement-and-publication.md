# 0003. State placement and publication

- Status: Accepted
- Date: 2026-10-04
- Code: `hide-core/src/persistence.rs`, `hide-core/src/reader.rs`, `hide-core/src/input_gate.rs`, `hide-core/src/sidebar.rs`, `hide-core/src/view_feedback.rs`, `hide-core/src/runtime/view_feedback.rs`, `web/src/components/elapsed.tsx`

## Decision

### State placement and publication

The standalone `hide-relay` service owns its room membership, connection routing, admission windows and byte counters in one memory-only state object.
It does not link the core, read a credential store or persist state; protocol endpoints alone own Noise keys and ciphertext authentication.
Its listener owns every connection task, each task owns its registration and bounded output queue, and dropping or stopping the service retires them all.
[RELAY_PROTOCOL.md](../RELAY_PROTOCOL.md#server-budgets-and-refusals) owns the refusal, restart and resource bounds, with real socket regressions in `hide-relay/tests/it`.

The front Workspace's Git header is ephemeral core state (`runtime/checkout_git.rs`), projected only into that device/path's `workspace_view.git`, never persisted or inferred from visibility-gated Changes rows.
The existing Changes pump drives two single-flight `BackgroundRead` readers (`checkout_git.rs`): one owning-node `Call::CheckoutGit` for the whole checkout, and one core-node exact-SHA GitHub read using fixed allowlisted arguments.
Changing the target or channel advances a generation; ingests compare that identity and the CI repository and complete ordered SHA list before publishing, while a HEAD change immediately clears the preceding CI result.
The core consumes the node's configured origin URL into a private validated provider repository identity and removes the raw URL before retaining or publishing header facts, so URL userinfo never reaches the renderer.
The node performs one porcelain-v2 no-renames status and one log read of its captured exact HEAD, with no base comparison or inactive checkout scan; status output is capped at 8 MiB, the newest five commits together at 64 KiB, and overflow is unavailable, never clean.
Git reads run off-lock at two-second intervals and a three-second node budget; only changed facts, failure/recovery, target changes or freshness transitions notify, and successful identical reads renew an internal deadline without publishing a frame.
CI reads GitHub's aggregate statusCheckRollup for at most five exact SHAs in one fixed allowlisted GraphQL query, with oid and returned commit URL validated against each request identity; provider errors remain unavailable.
One request every 30 seconds is retained only for the front target, with one node `gh` command bounded at 15 seconds and one link wait at 20 seconds.
The pump joins both admitted readers outside Runtime's mutex on shutdown; a superseded read may finish within that bound and its answer is discarded, with no additional job queued for each intermediate selection.
Five seconds without a successful Git observation marks retained data stale; CI ages at 90 seconds and a disconnected shell also labels retained state stale locally.

The read axis's look has one owner, `hide-core/src/runtime/look.rs`: its five inputs, the focus pane the operator chose, which counts while this machine is in front, the device in front, whether a window of this machine shows its own screen, the paired machines whose view of this one shows and the agents phones have open, are written only through its setters, and `Runtime::change_look` is the one boundary a selection completes at; the focus and the front carry the screen they were chosen on, and one chosen on this machine's window counts only while a window of it shows its own screen, one chosen in another machine's view only while that machine shows it.
The look is compared once at the outermost selection's end, under the lock that selection already holds, so a worker that moves the look after its own change has it read in that change, with no thread, timer or list of entries to keep.
`navigator.focused_device_id` is the front's projection on the wire, written with it and never read for a decision; `docs/status-model.md`, Hide owns the read axis, owns the rule and its regression tests.

Choose the authority, publication need and durability of a value separately before adding a field, store or worker.
`Mutex<Runtime>` serializes Hide's accepted domain and UI transitions; it is not the place to perform the reads, writes or analyses that produce them.
An off-lock store remains core-owned and must name its scope, freshness rule, resource bound and shutdown owner; a second copy in the shell cannot become an authority.

| Decision | Placement and allowed work | Existing precedent |
| --- | --- | --- |
| A value decides the accepted UI or domain transition | Keep the authoritative in-memory value in `Runtime`; admit one typed event and apply a completed worker result under a short lock. | [Stored primary checkout](0034-stored-primary-checkout.md#stored-primary-checkout), [Workspaces in the web shell](0050-web-workspaces.md#workspaces-in-the-web-shell); `runtime/workspace_view.rs`. |
| Producing a value needs filesystem, SQLite, network, a subprocess or substantial analysis | Take owned request data under the lock, perform the work in the existing reader/worker boundary, and validate scope or generation when applying the result. | [Project sessions and Memory](../ARCHITECTURE.md#project-sessions-and-memory), [Agent labels in the core](0021-agent-labels.md#agent-labels-in-the-core); `runtime/memory.rs`, `labels/analyzer.rs`, `reader.rs`. |
| A transition changes what the operator can see | Publish the changed state through the existing coalescing notifier; a wake, unchanged read or repeated observation alone is not a reason to publish. | [The core, the shell and Herdr](0002-core-shell-and-engine.md#the-core-the-shell-and-herdr); `handle.rs::ChangeNotifier`, `session_sync/coordinator.rs::agent_tick_needs_publish`. |
| A value must survive restart but is not a screen transition | Save it without publishing a new frame merely to announce the save. | [Agent sleep](0020-agent-sleep.md#agent-sleep): the last-look stamp is saved when visited and on its minute pass; `runtime/agent_sleep.rs::tick_agent_sleep`. |
| Time passing can be derived from an accepted value | Publish the reference time once and derive the display in memory. | [Agent labels in the core](0021-agent-labels.md#agent-labels-in-the-core): `last_prompt_at_unix_ms` and the shared elapsed clock in `web/src/components/elapsed.tsx`. |
| Accepted UI state needs persistence | Reuse the coalesced save: one active worker and one pending flag per store, copying state under the lock and serializing, writing, syncing and renaming outside it. | [Projects and Overview cost contract](../PERFORMANCE_TESTING.md#projects-and-overview-cost-contract); `runtime/agents.rs`, `runtime/workspace_view.rs`, `persistence.rs`, `workspace_views.rs`. |

On a high-frequency path such as keystroke ingress, added observational work is limited to bounded in-memory writes.
It must not introduce lock waiting, disk access, a subprocess or large serialization into that path.
The existing short Runtime admission is not permission to add another contended lock or blocking observer; pass owned data to an existing off-lock owner when work needs those resources.
Snapshot capture takes immutable typed sections under the lock and `handle.rs::snapshot_delta_payload` transfers them out of the core owner thread.
The WebSocket transport encodes the selected sections and its envelope once on a blocking transport worker; internal roots and phone followers use the retained typed state without decoding transport bytes.
Each rest field has its own revision and `Arc`, with navigator selection and lists, terminal metadata fields and status fields stamped independently; an unchanged value keeps its allocation and a changed value alone is copied.
Usage lives in its own top-level snapshot section, revisioned independently of navigation; later deltas carry it only when limits or ledger values change.
An omitted usage section preserves the previous value; an empty `nodes` array clears it, and a future revision cursor resets to a full snapshot.
The shell stores usage separately, so a usage-only delta preserves navigation identity and an unrelated menu or layout change neither serializes nor compares historical day arrays.
Pane input never enters the coalesced persistence queue: it must not be dropped or merged to reduce observer cost.
The save workers' pending flags bound the number of pending saves, not the byte size of the saved state; these stores have no general serialized-byte cap.
`Core::drop` joins the UI and Workspace-view save workers outside the mutex, but forced termination cannot promise a pending save reached disk (`handle.rs`).
Before adding resident work, use the [tick/timer/watcher ledger and cost review](../PERFORMANCE_TESTING.md#resident-ticks-timers-and-watchers).

#### Sidebar tab names

`TabSnapshot.sidebar` serializes the existing `TabNaming` as a manual name and stable tab number, using the same automatic-number convention as the tabbar.
The automatic task label remains the tabbar's own projection; the shell never guesses a sidebar name from that label.
The shell derives physical agent counts from the snapshot and keeps a root's complete subtree under that root's tab, even for cross-tab or cross-device children.
`CheckoutSnapshot.agent_groups` publishes all named and ungrouped memberships, draft identities and start receipts independently of the one group drawn in `workspace_view.agent_layout`.
The sidebar joins these memberships to real tabs and agents, so a hidden group's tabs, empty groups, drafts and terminal-only tabs remain reachable without a second membership store in the shell.
Group disclosure is page-local presentation; group selection and tab membership are core state, and opening a tab or pane selects its owning group and area in the same navigation transition.
A tab has no disclosure state: the former `sidebar_collapsed_tabs` field and its `sidebar_tab_expanded` event are gone, and a state file that still carries the field loads without error and loses it on the next save.
Deriving sidebar rows adds no event, I/O, timer or subprocess to typing or terminal echo; group mutations save through the existing Workspace-view owner described below.

#### View openers and comments

Who opened a View and the operator's comments on Views are core state (PRD view-opener-feedback, `hide-core/src/view_feedback.rs`, `hide-core/src/runtime/view_feedback.rs`).
A View's opener lives on its display in `workspace-views.json`, kept by device, terminal id, native session and provider, never by pane id; the pane is found again from terminal and session each time a snapshot is made, and an opener found nowhere is shown closed.
It is written only by a workspace control open whose pane caller Hide proved and by a terminal-link open that carries the clicked pane, and a navigate the operator types clears it; a caller with no proven pane writes none, which `opener.none` in the diagnostic log records with its reason.
A terminal-link open from a pane no agent runs in writes no opener but the pane's terminal (`Display::link_terminal`, by device and terminal id; `OpenedBy` in `view_feedback.rs` is who an open belongs to), so the View is that pane's in its `N views`, its desk tab's `owner_pane` and pair view, with no mark and never unseen (PRD lane-desk-pair-view D-54).
The last open names the owner both ways: a link's open clears the agent's mark (`opener`) and keeps the session's list (`openers`), so the agent's own content still counts it, and an agent's open clears `link_terminal`; `owner_pane_of` is the one rule the path line, pair view and the `N views` rows read.
A checkout's comments live beside its Views in the same file under `feedback`, saved by the existing coalesced save: at most 50 per View and 500 per checkout, each note at most 4000 characters, each quoted range at most 20 lines, checked in the core; each add, edit, delete, undo, relocate and Send is one event.
The workspace-view snapshot carries `feedback` only while there are comments or a Send to say: the comments of the Views on screen, the recipient groups, the opened Views per pane, and the last Send's state; each display carries its unsent count.
Finding a comment's lines again after a save is the shell's work on the text it already holds, reported once per change as a `relocate` event, so nothing reads a file for it under the lock or on a keystroke.
A Send runs on one feedback worker off the lock with at most 64 jobs: it reads the screenshots from the node that staged them, places them on the recipient's node, then stores the letter as the operator ([delivery.md](../delivery.md#the-operators-view-feedback)); no screenshot byte passes through an event, the lock or a snapshot.
Before the core, hided keeps each operator window's comments and Sends on one queue per window, at most 256 waiting and four of them with a screenshot: it stages a comment's screenshot off the socket loop and hands the events to the core in the order the window made them, so a Send never overtakes the comment before it, and a full queue is refused to that window as an error.

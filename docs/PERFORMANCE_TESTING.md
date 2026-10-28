# Performance and rendering verification

Read this before investigating terminal lag, flicker, selection, scrolling, resize, tab switching, CPU, memory, or snapshot/attach performance.
This is a repeatable verification procedure, not a record that any particular build passed.
Keep raw evidence and the run verdict under `agents/runs/<slug>/`; never commit traces, recordings, profiles, transcripts, or screenshots.
Use [CONTRIBUTING.md](../CONTRIBUTING.md) for delivery gates and [dev-runtime.md](dev-runtime.md) for bundle identity.

## Verification layers and current CI coverage

There are three complementary layers; none replaces the other two.
This table describes the checked-in workflows, not a claim that a particular PR's remote run passed.

| Layer | What it catches | Current execution |
| --- | --- | --- |
| Deterministic regression tests and structural checks | Blank repaint buffers, cache bounds, incorrect state transitions, blocking work in forbidden paths | The repository invariant checks run on every PR and main push in `.github/workflows/pr.yml`, and the Rust suite on every main push and on each PR whose plan includes it (docs/TESTING.md, "Which lanes a pull request runs"); `design-contract.yml` adds static design checks |
| Automated end-to-end | Real window launch, attach to a private Herdr server, IPC between the desktop host and `hided`, occluded-window rendering, packaging mistakes | Playwright drives the web shell in a browser and the desktop app through `desktop/e2e/fixture.ts`; the web suite runs in the six Linux `web-e2e` shards and, for its `@platform` tests, in one Windows job, the desktop app in the two macOS `desktop-e2e` shards of `pr.yml`, each when the pull request's plan includes it, and `nightly.yml` calls `verify` with every lane and configures the `@platform` web tests on macOS and Windows, the full desktop suite on macOS and its `@platform` tests on Linux and Windows, with tracked flaky tests blocking nightly; Linux desktop uses Xvfb; `verify` requires the pull request's jobs |
| Controlled performance comparison | Warm/cold latency distributions, periodic stalls, CPU/lock contention, sustained RSS, and cost that grows with process uptime | Local matched baseline/candidate measurements (`scripts/web-shell-measure/run.sh`); `nightly.yml` measures one fixed fixture every night through `perf.yml` on macOS ([The nightly performance lane](#the-nightly-performance-lane)), which keeps a dated record and fails only on an invalid measurement; no pull request runs or requires a performance job |

The repository-invariant Python tests run in the `policy` lane of `pr.yml`, which every plan includes.
Fixture/replay commands under `scripts/web-shell-measure/` do not become CI gates merely because this guide lists them; the nightly lane runs one of them, on its fixed fixture.
Check the workflow before claiming any of them runs automatically.
The per-OS schema/runtime contracts and the package jobs run in a pull request only when its plan names them; actual executed jobs, failures and skips establish a particular head's coverage.
PR macOS queue comparisons use actual executed jobs (exclude skipped reusable placeholders), recorded SHA/time windows and sample counts, and report run wall time and runner cost separately.
A before/after observational sample with different workloads or little concurrent queueing does not establish the concurrent-PR p90 target.
Hosted desktop automation and private hook fixtures do not establish physical IME, first-launch security prompts or real agent hook behavior.

### Maintenance and review policy

1. Every change runs the applicable CI regression gates; a rendering/performance bug gets a deterministic regression test when it can reproduce the observed failure economically.
2. Changes to terminal drawing, selection, input/scroll routing, snapshot delivery, geometry, focus, or attach lifecycle also require the affected `desktop/e2e/*.spec.ts` or `web/e2e/*.spec.ts` scenario before being called verified.
   When a scenario cannot be automated yet, exercise it manually against the packaged app and record that the check was manual.
3. Changes to caching, scheduling, snapshots, locking, or claims of improved speed/memory require the affected controlled baseline/candidate measurement (`scripts/web-shell-measure/run.sh`) as well.
   Apply it only when making a memory or latency claim; do not impose it on unrelated documentation changes.
4. The change author records those results in the PR's existing Evidence section, and the reviewer checks coverage and exclusions as well as CI.
5. Keep the regression test beside its component, reusable measurement tools in `scripts/`, procedure and comparison policy here, and each run's raw evidence under `agents/runs/`.
   Never turn a one-off trace or historical number into a hardcoded universal latency limit.

## Whole-app audit coverage

Start a broad audit with this matrix and retain one row per condition in the run verdict.
A row names an existing fixture or manual seam; it does not authorize a build, foreground interaction, remote installation or provider call.
A current-only audit is valid: record the exact current build identity, load, actual topology and catalog size, renderer/driver mode, warm-up, observation window and exclusions for every measured row.
For a before/after claim, additionally use matched baseline/candidate conditions and record both identities; a current-only observation does not establish an improvement or regression.
Choose thresholds from the current approved claim before a run; old PRD identifiers and summary booleans do not supply whole-app acceptance.

| Family | Conditions and existing seam | Evidence and remaining boundary |
| --- | --- | --- |
| Idle | Same private topology with a page attached; no page attached; and resident work enabled or disabled explicitly. `resources.py`, `ws-count.mjs`, private `/health` client counts and the resident ledger. | CPU-time deltas and root/tree RSS; changed snapshot sections and reader/child activity. No clients disables screen-only reads, but does not stop every resident owner. |
| Terminal input and output | `MEASURE_SCENARIO=keys`, with `MEASURE_KEY_COUNT` set explicitly; idle and driven key echo separately. `single`, `multi`, `areas2` and `areas3` cover output, retained terminals and multiple Agent areas. | Page-clock keydown-to-write distributions and exact ordered keys; callback intervals, output completion and resource samples separately. Legacy CLI-return echo cannot establish a calibrated latency result. |
| Scaling and catalogs | `MEASURE_SCENARIO=topology` at `MEASURE_SCALE=operator` and `double`; the opt-in `responsive_state::connected_catalog_measurement` covers populated local/device catalogs. | Report actual project/worktree/tab/pane/agent counts, changed versus unchanged publication cost, semantic-section bytes, owner capture and transport encoding. Synthetic reported agents are not real provider tasks. |
| Launch and lifecycle | `desktop/e2e/desktop.spec.ts` for app lifetime, reattach and relaunch; `hided/tests/it/terminal_children.rs` for owned attach cleanup; `keys` for page detach/reconnect. | Functional ownership tests plus separately measured cold/warm launch, real client retirement/reacquisition and attach counts; separately open a sixth distinct tab and revisit the released tab. Navigating to `about:blank` is not native last-window or browser-process retirement. |
| Long uptime and retained memory | Same workload on a fresh and aged private daemon; controlled daemon restart retaining the private Herdr panes; eleven one-minute RSS observations. | Process age, endpoints, distributions, sample coverage and owner/cache sizes. Ten minutes does not establish hour-scale stability or a leak-free app. |
| Files, Explorer, Changes and Overview | Existing worktree/delete/cleanup specs, filesystem fixtures and native private fixtures; compare small/large catalogs, cold/warm reads and scoped file/Git changes. | Operation-to-visible-result timing, actual node/subprocess reads, wire sections and CPU/RSS. A bounded walk or an eligibility test is not responsiveness or reclaim evidence. |
| Overview and navigation | `overview-device.spec.ts`, `work-prs.spec.ts` and recent navigation/restart specs. | Local event counts, unchanged layout and relayout separately; many projects and Korean/English wrapping. Functional assertions do not establish menu latency. |
| Memory and usage | Unchanged, append and first-backfill sources; prompt hooks and historical usage fixtures. | Source bytes, read/write/provider counts, queue bounds, wire fan-out and input timing separately. |
| Browser Views | `desktop/e2e/browser.spec.ts` and `browser-cdp.spec.ts`; private local/device pages; admit, hide, restore, revoke and evict under existing page caps. | Native captures and separate page/renderer RSS, request/connection counts and lifecycle measurements; cross the live-page eviction boundary and track configured session identities/profile bytes separately. Live-page caps do not bound lifetime session partitions. Continuous title churn can report once per page per 100 ms window; inventory/geometry serialization cost is separate from the live-page cap. Functional CDP control does not prove browser paint or native resource retirement. |
| Phone and notifications | `web/e2e/mobile.spec.ts`, `mobile-settings.spec.ts` and `hided/tests/it/mobile.rs`; zero, one and multiple subscriptions, root status transitions and quiet repeats. | Per-subscriber fan-out, admitted batches/queued bytes, shutdown ownership and event-to-visible/received timing separately. A delayed transport check needs a scoped fixture seam, since the sender admits production push endpoints. Fixture state and encrypted-payload caps do not establish batch bounds or real phone, push-provider or network latency. |
| Settings/kit | Existing Settings/kit fixtures and the resident ledger; closed/open views, unchanged polls, queued jobs and owner shutdown. | Actual provider/node/child calls, operation completion, transition publications and idle/driven CPU/RSS. Fixture answers do not establish authenticated provider, CI or installation performance. |
| Remote and hooks | Guarded `device` and `remote-core` scenarios; existing remote mailbox/core-move tests; installed hook command boundary, ordinary versus driven calls. | Record core/screen/device roles, clocks, consent folders, node/SSH child ownership and cleanup. Synthetic providers or loopback SSH prove their fixture boundaries only. Real devices and authenticated providers need a separately authorized run. |

Select the smallest existing seam that reaches the claim.
The measurement runner and native tools have platform prerequisites; a passing Windows/Linux functional lane does not supply unmeasured latency, RSS, IME or compositor evidence on that system.
Mark an unsupported or unrun measurement cell explicitly instead of borrowing another system's result.

For tab retention, record the expected tab/pane identities and compare both attached metadata and live renderer instances before and after admitting a sixth distinct tab.
A maximum-count assertion alone can pass when every previously retained instance was discarded.
Exercise creating the new tab and selecting an existing sixth tab as distinct transitions, retaining the state deltas around a failure.
On revisit, check restored text and new ordered input separately from retained membership; text recovery does not establish preserved scroll, selection or a warm renderer.

### Invariant and regression ownership

| Invariant | Current executable owner | What it does not prove |
| --- | --- | --- |
| No subprocess under Runtime, single reader driver and refresh admission | `scripts/check-capability-readers-off-lock.sh`, `scripts/tests/test_capability_reader_gate.py`, in the PR `policy` lane | This structural pattern check covers named readers and call shapes; it is not a runtime I/O, lock-duration or latency measurement. Inspect an owner whose worker boundary the check does not cover. |
| A burst is announced once, and snapshot capture clears the latch before locking | `herdr-core/src/handle.rs`, `runtime::tests::snapshot_delta::agent_notes_state_the_view_authority_and_the_announcement_rule` | Notes/source ordering does not measure end-to-end publication delay or every interleaving. |
| Unchanged semantic sections keep identity; changed fields converge; legacy clients still receive complete rest | `runtime::tests::snapshot_delta`, `web/src/store.test.ts`, `hided/tests/it/handshake.rs` | Byte/state assertions do not measure owner-channel queueing, transport latency or native drawing. |
| Hidden retained terminals survive a tab switch without resizing or losing output | `runtime::tests::snapshot_delta` retained-view tests, affected web/desktop app scenarios | A retained buffer does not establish scroll, selection, sustained output or native paint latency. |
| A sixth-tab creation and a checkout switch preserve the still-retained tabs across acknowledgement/layout order, and a released pane keeps its `released` entry; closure, expiry and disconnect still retire their own state | `runtime::tests::terminal` sixth-tab, unconfirmed-creation and checkout-switch cases; `web/e2e/s2.spec.ts` sixth-tab journey (owner-opening creation, every received `terminal.panes` list, two revisit cycles) and checkout-switch journey | Exact membership, every-list and same-node checks prove retention; they do not measure long-term heap stability or tab-switch paint latency. |
| A held local hook diagnosis does not stop initial or later topology; its owner serializes installation and stops before joining | `session_sync::tests::held_hook_diagnosis_does_not_hold_startup_or_settings_topology`, `runtime::tests::device_kit` diagnosis lifecycle cases | Held protocol replies prove independence and ownership, not natural disk latency or a whole-app latency limit. |
| WebGL activation failure or persistent context loss preserves the same usable terminal; transient restoration and pane disposal keep their own lifetimes | `web/e2e/terminal-webgl.spec.ts` with real context-loss events and private PTY input | Software WebGL proves browser recovery, not native hardware stability, physical input or GPU throughput. |
| Owned terminal children end with daemon/link owners | `hided/tests/it/terminal_children.rs` and remote/core-move journeys | A functional teardown assertion does not establish zero survivors for every OS or owner-death mode. |
| A Claude or Codex transcript keeps its header directory and listed facts only while its stamp (device, inode, length, modification time) holds; when the stamp moves the file is read on from where its last read stopped through the label and Memory readers' `ConversationCursor`, from its start when it got shorter or was replaced, and lists what a cold read lists; a read that failed is read again from the start on the next listing; a file that leaves its home's listing is forgotten | `hide-session/src/catalog.rs` tests `a_claude_transcript_keeps_its_facts_until_its_stamp_moves`, `an_appended_transcript_lists_what_a_cold_read_lists`, `a_shorter_or_replaced_transcript_is_listed_from_its_start`, `a_record_appended_in_two_writes_is_counted_once_it_is_whole`, `a_transcript_longer_than_one_read_window_is_listed_whole`, `an_appended_transcript_is_read_on_from_where_its_last_read_stopped`, `a_transcript_that_could_not_be_read_is_listed_once_it_can_be`, `a_listing_forgets_the_facts_of_a_file_it_no_longer_finds`; cache in `hide-session/src/file_cache.rs` | A same-length rewrite with its modification time restored keeps the old facts, and bytes rewritten in place before where the last read stopped are not read again, as the label and cursor readers already accept; the cost of a listing is measured only in the Project Memory cost contract below. |
| A Memory pass skips the session walk of a Project whose listed sessions (each with its file's stamp), their live, closed and delegated sets, Hide AI's route and the Project's retry hold are what they were when a walk left nothing to read, record or queue, until the first pending session would turn quiet; Retry, a Hide AI settings change and Memory turned off forget the memo | `runtime::memory_analysis::tests` `a_walk_whose_sessions_did_not_move_is_skipped_until_one_moves_or_comes_due`, `a_skipped_walk_comes_due_when_its_first_pending_session_turns_quiet` | A session with no stamp (OpenCode's, an older node's row) walks its Project on every pass; a store change made outside the pass (a cursor or queue row written by hand) is not seen by a skipped walk; the card and evidence checks keep their own schedules and are not skipped. |
| Measurements refuse production device routes before dialing | `scripts/tests/test_measure_device_guard.py`, `device-guard.sh` | Guard fixtures do not establish a real device's speed, login or cleanup. |
| Echo and frame summaries refuse invalid or incomplete data; echo acceptance requires an explicit threshold | `scripts/tests/test_measure_echo_summary.py` | Synthetic records prove the verdict rules, not a measured latency, physical input or native presentation. |
| The nightly lane fails only on a fixture other than its own, an invalid or a missing measurement, and files each failed check by name | `scripts/tests/test_measure_nightly.py`, `scripts/tests/test_ci_flaky_report.py` | Synthetic run files and GitHub answers prove the verdict and filing rules, not a hosted runner's timing or a measured figure. |

Rust owners run in the planned Rust lane, app scenarios in their planned web/desktop/remote lanes, and script owners in `policy`.
Read actual job outcomes and retries; this mapping describes checked-in ownership and does not claim an executed PASS.

## Resident ticks, timers and watchers


Conversation display restoration reuses the labels reader and links writer, adding no timer, input callback or coordinator I/O.
A missing hook reference admits one serialized process-info locator read (one-second deadline), the existing bounded native read, and one local metadata RPC (one-second deadline); failures retry no sooner than fifteen seconds.
Only native owner plus current hook proof confirms a binding, and the same pane/provider/reference generation fences every result.
An unchanged known owner reuses its display metadata without another ledger query, and unchanged metadata is not written again.
Compatibility memory retains at most 512 short display records and twenty PRs per record; ledger queries return twenty PRs, use the existing sixteen-call mailbox and existing bounded write queue, and deletion follows existing session retention.
The labels reader is joined by its coordinator owner and the links writer by its existing owner; their existing single-flight admission bounds pending restoration work.
No new work occurs per keystroke or terminal frame, and publication follows actual display connection or metadata transitions.


Use this ledger when deciding whether a feature can reuse an owner instead of adding another permanent loop.
The source paths are the authority for cadence, admission and cleanup; a duration is a scheduling interval or deadline, never a measured latency guarantee.
The ledger covers core/daemon background schedules, the desktop host's discovery watch and the shared elapsed display clock.
Finite request deadlines, UI debounce/animation timers and the upstream Herdr server's internal schedules are outside this ledger.
An active-worker limit does not cap queue length, retained bytes or all work done by a tick.
The missing bounds and shutdown gaps below describe the current implementation; this documentation change does not add them.
Off-lock does not mean off-coordinator: changed-root Repository calls and queued Background AI settings writes below remain gaps against the nonblocking session-sync rule, not permission to repeat that pattern.
Hook diagnosis uses the existing kit worker so a held read no longer blocks topology application.

| Work and source owner | Cadence or trigger | Work per wake | Bound and known missing cap | Stop or release |
| --- | --- | --- | --- | --- |
| Product statistics: `hided/src/product_analytics.rs`; agent starts from the link worker (`links/worker.rs`, `runtime/links.rs`) | Explicit page connection, actual Hide AI backend invocation, or a link-worker pane pass that wrote a session's first `pane_first_at`; no timer, terminal-input hook, session-sync thread or snapshot observer. | Enqueue one small enum and timestamp; one off-lock worker serializes and sends to PostHog. An agent start takes the runtime lock only to clone the sink handle and enqueues after releasing it; the first-seen read is one primary-key lookup per pane fact inside the pane pass's existing transaction. No snapshot publication for a capture. | 128 waiting events, one in-flight HTTP request, five-second global timeout, no redirects, retries or disk spool. | The core-role owner drops and joins the worker off Runtime; opt-out or shutdown clears waiting events, and a started send may finish. |
| Workspace View memory: `runtime/view_bookmarks.rs`, `runtime/view_areas.rs`, `runtime/workspace_view.rs`, `persistence.rs` | Existing Workspace sync after accepted UI changes and before snapshot reads; no new timer. | Compare borrowed shown/active ids against the last observation and allocate nothing while they match; a newly shown tab brings its bookmarked display to the front in the existing frame. Changed reconcile/admission scans at most 32 checkout displays, while unchanged reconcile uses its generation guard; an open at the cap closes the oldest closable display with one more such scan per display it closes, only when the cap is reached. A background caller's open records the selection, then restores it with one more layout change before publication. No new I/O or process under Runtime. | 256 retained Workspaces; existing 64-tab Agent admission; 32 displays, six areas and depth three in the checkout's one layout. Closed records and resources are pruned on authoritative topology. Browser caps remain 12 live pages; the desktop bounds 65536 retained displays (256 per Workspace) and 65792 scopes, one per area that holds a display plus each Workspace's empty front area. One active save and one pending flag; no independent stored-byte cap. | Runtime owns the list, the bookmarks and the browser inventory; authoritative prune drops a closed tab's bookmark, the desktop retains or evicts pages under its existing LRU, and `Core::drop` joins the save worker outside the mutex. Forced termination does not guarantee a pending save reached disk. |
| Session coordinator: `herdr-core/src/session_sync.rs`, `session_sync/coordinator.rs` | Operation tick 250 ms; `agent.list` 1 s; catalog due at 30 s; reconnect backoff 100 ms to 5 s. | Advance pending operations, compare telemetry and publish only changed state or a due catalog; due socket reads run off-lock. | One coordinator per followed server; active-tab recovery allows 8 reads; a start Hide made that Herdr holds open is read with `agent.get` from 3 s to 90 s after Herdr took it, at most 8 times, once a second, 4 panes per batch, one batch in flight, 64 starts tracked and 256 start records kept (`session_sync/launch_settle.rs`); the coordinator `mpsc::channel` mailbox has no numeric capacity. | `SessionSyncHandle::drop` sends Stop and joins; coordinator exit stops its subscription; loss of the weak runtime owner also ends the loop. |
| Session escalation, resolve and pane headers: `runtime/session_state.rs`, `agent_state/{escalation,sessions,header}.rs` | Accepted agent, delivery, PR and pane-state transitions; local-day deadline checked on the existing coordinator tick. | Clone current session rows and derive escalation, membership and headers in memory under Runtime; compare before publishing. Repeated controlling transport frames add no header pass. Resolve writes through the existing coalesced off-lock state saver; no new worker, process or I/O under Runtime. | One pending save frame; resolve records follow current panes and session/activity guards, held raises follow pending letters, remembered folds admit current project/device keys and retire on explicit removal. These collections have no independent numeric entry cap. | Existing coordinator/save-worker shutdown; pane closure and session/activity changes retire resolve state, letter retirement clears a held raise. The web reuses the shared elapsed clock below. |
| Agent sleep: `runtime/agent_sleep.rs`, `agent_sleep.rs` | Minute decision on the local coordinator tick, and on each dialing node's coordinator tick for that node's panes; committed visits update last look. | Inside the minute, a due-time comparison; when due, inspect pane/agent state, persist changed last-look stamps and schedule eligible ends. | At most 4 automatic ends in flight; the full decision scans the current agents and has no separate per-pass agent-count cap. | Decision polling ends with the coordinator; individual end/wake work is finite and separate from this timer. |
| Lineage reconciliation: `coordination/lineage.rs` | Existing one-second native agent refresh, also requested by native events, and startup/reconnect; no new timer or general replica-publication trigger. | Reuse the existing native-agent equality result, append-only registration count and a hash of the wanted server marks, which the same lock that reads the ledger builds in O(agents) from the published rows (`Runtime::wanted_server_marks`, no I/O); a mark is a token in the pane's existing patch or a patch of its own, written once per pane per connection, and a plan past 2,048 patches leaves the rest to the pass after its batch is acknowledged; settled observations with no successful write awaiting native confirmation only drain bounded completions and compare the count. A native change, new registration, failed write or outstanding successful write triggers O(registrations + agents) indexed planning and differing-token I/O outside Runtime; a read begun before that write completed cannot confirm it, while a newer read can repair missing tokens. Registered targets are selected across the supplied native list, with at most one patch per registered pane; letter/watch writes and label/process/catalog publications do not trigger planning. | One worker per followed server, one active and one pending batch; 2,048 patches per batch, 4,096 completions and 2,048 cached signatures; one-second API deadline per changed pane. | Drop raises its stop flag, closes admission and joins outside Runtime; cancellation skips the remaining patches after the current one-second API read. |
| Spawn naming: `coordination/naming.rs`, existing lineage writer | Receipt count or durable placement revision changes, and reconnect; no new timer. | An unchanged observation checks one queued-slot signal before comparing two integers; a saturated queue builds no plan, and the worker releases the slot when it receives the batch. Admission scans at most the 4,096 receipts, on the changed boundary only; current receipt outcomes are reread before work so queued reconnect repair skips completed targets. No I/O under Runtime or on session sync. | One active and one queued batch, each at most 32 receipts; 4,096 attempted ids per connection; each receipt at most four one-second API calls and one five-second checkpoint. | Same lineage owner; stop checked between receipts, so shutdown may wait for the current naming receipt instead of only one metadata call. |
| Hook token restore: `coordination/hook_tokens.rs` | Existing one-second native agent refresh of the local coordinator, and the list read right after each connect; no new timer and no replica-publication trigger. | Each refresh scans the tokens of the listed agents already in memory, takes no lock and reads no file. A pane that lacks `hide_hooks` and whose agent Hide has an adapter for is queued once for each connection; the worker then reads that pane's record under the shared side of `panes.lock` and sends one `pane.report_metadata`, and sends it again when the record changed meanwhile (at most three more times). Nothing runs under `Runtime` and nothing is published. | One worker, one batch in flight and one queued; 256 panes per batch (the rest wait for the next refresh, with one `restore.deferred` record a connection); 512 waiting completions; three attempts for a failed pane per connection, then a `restore.gave_up` record; one-second API deadline and one-second lock wait per pane; the pane map is pruned to the listed agents on every refresh. | Drop raises its stop flag, closes admission and joins outside Runtime; cancellation skips the remaining panes after the current one. |
| Process names: `session_sync/process_info.rs`, `hide-herdr-client/src/lib.rs` | Focus, agent-state or subscription-generation change, or 30 s after a settled read. | Sequential `pane.process_info` reads on one off-lock worker; reject obsolete generations and publish only changed names. | At most 5 attached tabs' focused panes and one worker; each small response has a 1 s read deadline and 64 KiB frame cap. Connection setup depends on the connector; local Unix has no explicit setup deadline here. | Reader Drop cancels between batch reads and joins the current read off-lock; it does not interrupt an in-flight connection attempt or impose a platform-independent batch/join deadline. |
| Label follow-ups and generator retry: `labels/worker.rs`, `labels/generator.rs`, `labels/analyzer.rs` | Changed agent/session; native-file current metadata repeats no sooner than 3 s after a successful read, and other starting turns get one 3 s follow-up; unavailable transcript retry at 15 s, the generator lock asked of the server's node again every 30 s, standing by or holding, a linked node's ask on a short thread of its own. | Check retained panes, schedule transcript reads and analyses, drain results and persist label records off-lock. Native-file metadata shares the serialized transcript read and publishes only changed title, question or proof; an unchanged read adds no analysis, save or notification. The same read folds an agent's turn records (Codex) into a fixed-size turn tracker kept beside the checkpoint, and the delivery observation reads the overlay the publish already builds; no read, file or process is added for it (PRD codex-plan-approval-hold D-08). A Claude turn end owes its session read (`PaneRecord::turn_end_owed`, one `Option<u64>` per pane, not saved): it changes only on a pane's state transition or a read result, the read it waits for is the one that transition already queues, and the release rides the publish that read causes, so no frame, read or timer is added (#902). | One transcript read and one analysis submission per server; one analyzer executes per Core instance, shared by its followed servers; waiting panes and analyzer `mpsc` have no fixed numeric queue cap. | Coordinator drops its transcript reader and joins it; core analyzer shutdown cancels the provider, answers queued jobs and joins. The turn-end debt ends when its read lands or fails (`DEVICE_READ_TIMEOUT` bounds the read), the agent works again, or its pane or agent is forgotten. |
| Capability readers: `session_sync/coordinator.rs`, `ports.rs`, `usage.rs`, `ai.rs` | UI-attached gate for Ports and Background AI; Ports 5 s after completion; Usage initial 1 s, visible-window refresh 5 min or popover opening after 60 s, with separate headless demand from Memory or usage limits; Background AI probes 30 s with 2 s changed-request spacing. | Run due `lsof`, credential/network/transcript reads or provider probes on workers outside Runtime; ingest changed answers. | Ports use one `BackgroundRead` worker with a 30 s node-call timeout argument; the Unix node bounds each of its two `lsof` calls at 10 s; Usage has one `BackgroundRead` per deduplicated declared source (two), including Codex; AI probes use one `BackgroundRead`. Usage bodies cap at 256 KiB, session tails at 2 MiB each, and parsed windows at sixteen; no general aggregate response-byte cap covers the other readers. | Coordinator ends polling; Usage Drop cancels and joins its reads outside Runtime; generic `BackgroundRead` Drop does not cancel or join an in-flight read. |
| Hook diagnosis: `kit.rs` | One startup read on the existing kit pump; then due one second after completion while both UI-attached and Settings-observed; successful installation requests its own read. | The same worker serializes `HookDiagnosis` with kit jobs outside Runtime and publishes only changed answers. | One call at a time and one next-read deadline; no added thread, timer or pending-refresh queue. The node-call timeout argument remains 10 s, not a measured latency or hard end-to-end filesystem cutoff. | Core stops admission before closing the fixed node and joins the pump off-lock; a retiring reply cannot publish. A read may delay kit status but cannot block the session coordinator's topology work. |
| Background AI settings save: `session_sync/coordinator.rs`, `hide-ai/src/settings.rs` | A queued settings-save intent on the existing coordinator; no independent timer. | Create directories, serialize, write a temporary file and rename it synchronously outside Runtime; no fsync. | One inline save at a time; no explicit filesystem deadline, so the coordinator cannot consume events during the write. | No separate cancellable save worker; coordinator shutdown waits for the current inline operation. |
| Historical usage: `usage_ledger/` | One worker, no work per input or terminal frame; discovery every 30 s, 3 s while Usage is open; changed publications every 30 s, 5 s while open. | Own-node reads, private SQLite transactions, pricing and calendar queries stay off Runtime; a short lock installs only changed prepared consumption cells, preserving independently read account limits. | One 1 MiB / 10 ms read step, 20 ms rest with a visible window, source read steps paused otherwise; stored engine reports are still read on their report interval while detached; 10,000 candidates per agent, 1,000,000 events per device, 100,000 daily rows and 512 MiB store; cells bounded at 20/53 weeks and six agents; no conversation bodies leave the node. | Core owns and joins the ledger and price workers outside Runtime; latest visibility flags replace pending hints rather than queueing them. |
| Git facts watcher: `worktrees.rs`, `hide-host/src/git_watch.rs`, `hide-platform/src/watch.rs` | OS changes and a one-second heartbeat; reports drain only through the UI-attached worktree reader, and with no renderer attached the watch is ended (`git_watch.rested`) and started again on the first read after one attaches, which reads every project once it is watching; 300 ms quiet period; an ended watch restarts after 30 s when that reader runs. | Changed requested roots trigger up to 64 synchronous `Repository` calls on the coordinator before the `BackgroundRead`; unchanged roots return early. The node filters Git facts and reports changes; the core then requests one worker read of affected projects; overflow invalidates watched projects. This is not per-tick git spawning. | First 64 projects watched; 4,096 queued OS paths become an overflow; the core keeps at most 256 reports per watch (`GIT_WATCH_MAILBOX`) and never a heartbeat, and a report past it is dropped and kept as one re-read of every project (`git_watch.mailbox_full`). Repository calls have a 10 s timeout argument each; the watch call has a 24 h timeout argument, so report generation is not indefinite. One background read; no cap on the full project request vector. | The last renderer detaching ends the watch on the coordinator's next wake (`WorktreeReader::rest`). A changed watched set starts a replacement before stopping the old call. A stopped watch's forwarding thread ends on the node's next report and is joined by the reader; the reader's end waits up to 3 s for them and names any still running in `git_watch.stop_unconfirmed`. A generic in-flight read may finish after the driver ends. |
| Background fetch: `herdr-core/src/fetch.rs` (`FetchPump`), `runtime/fetch.rs`, `hide-host/src/worktrees.rs` (`fetch_remote`) | One-second pump tick; nothing is due while no renderer is attached; the front repository at most every 60 s and again on `fetch_look` after 60 s, the others at most every 5 min; failures back off 1, 2, 5 and 15 min, and a credential or network failure waits for `fetch_look`. | Each tick takes the lock once to apply ended fetches and list due ones (a walk of the catalog's registered repositories, no I/O); each due fetch is one `Call::FetchRemote` on a thread of its own to the node holding the repository, which runs one bounded `git fetch` off every lock; a changed outcome refreshes the checkout header once and announces only when that header changed. The Git facts watcher then sees the moved refs and re-reads by its own rule. | One fetch per node at a time, on the link's background lane (two slots and two node workers beside the four machine calls, `BACKGROUND_CONCURRENCY`), so a fetch never takes a header or worktree read's slot; the first 64 registered repositories (`FETCH_REPOSITORY_LIMIT`, one `fetch.over_limit` diagnostic past it); Git stops at 60 s (`FETCH_TIME`) with 64 KiB of output, the call at 75 s (`FETCH_TIMEOUT`); per-repository state is kept only for listed repositories with the fetch on. | Core Drop stops and joins the pump; a fetch thread still out is detached and ends on its call bound, its answer dropped. |
| Device Git watches: `herdr-core/src/fetch.rs` (`watch_devices`), `worktrees.rs` (`DeviceGitWatch`), `runtime/device_catalog.rs` | The fetch pump's one-second tick; the device's watch reports and its one-second heartbeat; 300 ms quiet period; no watch while no renderer is attached. | The same lock pass that lists due fetches names each connected device's watched common directories from what its last worktree read answered (no node call); off the lock each watch is reconciled and drained, and only devices with settled changes take the lock again to mark those repositories and start the device's existing one-at-a-time worktree read, which measures again only the worktrees whose HEAD, index or base refs moved (`reuse_unchanged`; a full measure of a 32-worktree repository took 48-51 s against 0.16 s reused); a failed read is tried again, every repository, after 30 s on the same tick (`retry_device_reads`). | One watch per connected device, on the link's background lane, over its first 64 repositories (`GIT_WATCH_LIMIT`), 256 reports per watch (`GIT_WATCH_MAILBOX`); one device worktree read at a time, a change during it read once after it. | A device that leaves the listed set or reconnects over a new link drops its watch on the pump thread, which waits up to 3 s for its forwarding thread (`git_watch.stop_unconfirmed`); the pump's end drops every watch the same way. |
| Changes pump: `changes.rs`, `runtime/projects.rs::changes_target` | Pump 250 ms; read on changed input or 2 s after settlement while a renderer is attached and persisted Explorer, Changes, open-diff or new-tab state needs it; with no renderer attached the request is none, and once that empty answer has landed the pump reads nothing until a request returns. | Take the front checkout request, start one off-lock host read and ingest its scoped result. | One active `BackgroundRead`, current request coalesces changes; host-call deadline 30 s; no additional pump queue of requests. | Pump Drop sends Stop and joins the pump; its generic in-flight host read is not joined by `BackgroundRead`. |
| Kit pump: `kit.rs`, `runtime/kit.rs` | Pump 250 ms; launch/apply intent, or 5 s status refresh while Settings is observed. | Take one local kit job under the lock, run installation/status outside it and ingest the result; devices use their own workers. The agent search stats about 25 shell startup files per pass and starts the login shell (`$SHELL -ilc`, 10 s deadline) only on its first pass, after one of those files changed, or a minute after a failed ask; a re-read with nothing changed starts no process. | One local job at a time and one worker per device; device call deadline 180 s; no fixed global device-worker count cap in this pump. | Local pump Drop sets the child stop flag, sends Stop and joins; a device worker exits when work/connection/weak owner ends, without a retained join handle. |
| Terminal service: `hide-node/src/terminal/mod.rs` (one per node) | Timer thread wakes at the earliest due retry, settled observer resize or folded input report; retry delays 5, 10, 20 and 30 s on this machine, none on a device. | Under the service's own lock (never `Mutex<Runtime>`): start a due attach, re-attach a settled observer, release a folded input report; the attach itself runs on the session's threads. | 64 sessions; at most 4 automatic retries per failure; 64 KiB held per key holder and 8 creation requests (plus 8 the core has not named, 10 s each); one input report per pane per second between the at-once ones. | Service Drop stops and joins the timer thread and drops every session, whose reaper releases its attach and ends the child. |
| New pane shell wait: `runtime/terminal_input.rs` (`herdr-core-shell-reading`) | Herdr's answer for a new tab, split or a Task's plain terminal tab whose creation request is still pending, answered inside the wait's bound; no timer of its own. | Every 50 ms one `pane.process_info` read and one own-node `LineInput` call, each cut to what is left of the wait; a lookup under the runtime lock before each read and one lock to resolve the request. | One worker per waiting request, so at most `INPUT_REQUEST_LIMIT` (8) at once; each ends at most 2 s after its request opened. | Ends at the shell reading keys or at the deadline, and at its next read when its request no longer waits (discarded, evicted, re-held) or the runtime is gone. |
| Terminal reports: `herdr-core/src/terminal_reports.rs` | Wakes on a report. | Take every waiting report into the runtime under one lock and notify once. | One pump thread; past 16,384 waiting reports a report replaces the waiting one about the same subject, and past 65,536 a report about a new subject is dropped and logged (`terminal.report_dropped`). | Pump Drop sends Stop and joins. |
| Device terminal writer: `hide-node/src/terminal/device.rs` (one per device link) | Wakes on a control, view, redraw or key for the device. | Write the next waiting line down the link; a plain run of one pane's keys goes as one line of at most 16 KiB. | 256 KiB of unsent keys and 2,048 waiting runs per pane, 64 panes with keys waiting, each past which only that pane ends (B18); 8,192 lines or 8 MiB of controls, views and redraws per link, past which what waited is dropped and the link is ended as stalled (D-19). | Ends with its link or when its node is dropped; a stall ends the link through the host. |
| Terminal hub: `hided/src/terminal_hub.rs` | Wakes a client on output or a redraw to ask. | Queue a shared chunk per client; a client's task sends at most 256 chunks or 4 MiB of text per frame. | 512 chunks and 2 MiB per pane ring, 1 MiB unsent per pane per client after its latest full frame, chunks shared, so under 3 MiB plus one waiting full frame per pane for eight clients; a chunk past 4 MiB is not drawn; a redraw asked at most every 2 s per pane. | A client's entry leaves with its socket; a pane's ring leaves when its node forgets it. |
| Terminals relay tap: `hided/src/relay.rs` (one per linked node) | Every output chunk of a pane that is not the node's own, on the producer's thread; the relay's writer wakes on it. | Encode the chunk to base64 outside the tap's lock, then queue it; the writer sends at most 256 items or 1 MiB of text per message and asks the redraws the tap owes. | 4 MiB of base64 output per node, full frames included: a few screens' full frames and their output after them, past which a node that reads nothing is drawn again rather than queued for. Output that would pass it drops the backlog; a full frame that would pass it drops the waiting output of the panes that waited longest until it fits; either redraws those panes from a full frame asked on the writer's next send, never while it is stalled. A full frame alone over 4 MiB is not relayed. Pane ends (`forget`) are never dropped. | The tap closes and empties when its relay ends or a second relay of the node replaces it. |
| Project Memory pass: `runtime/memory_analysis.rs`, `runtime/operations.rs` | A pass every 60 s on operation ticks, at once when a live session's pane opens or closes; triggers that arrive while a pass runs set one deadline, so they start one pass after it; a session must be unchanged for 30 minutes, or its pane closed, before analysis. | Decide that a pass is due under the lock, then walk Projects, read sessions and SQLite and call the provider off-lock, reporting each Project's state and finishing under the lock. Each Project's listing still runs; its per-session walk (one stat node call, two cursor reads and a no-op upsert per session) is skipped while the walk memo above holds, so an idle pass costs the listings and the card checks. | One pass at a time; 1,000 calls a day, the usage limit and input/storage limits belong to the [Memory cost contract](#project-memory-cost-contract). | Runtime Drop cancels the pass's provider call; the pass thread has no retained join handle, so cancellation is not a join guarantee. |
| Project Memory tool reads: `runtime/project_memory.rs` | `memory_open`, `memory_expand`, `memory_search`, a Memory write, or a receipt batch while a Project is open. | Take owned request data under the lock; open the store read-only and read the Project's cards (one card's sources and previous text only when shown in full) off-lock; land the answer under the lock and publish only when the section changed. | One opened-Project read in flight with at most one waiting; 10,000 cards per Project. | A read whose generation is behind never lands; the worker thread has no retained join handle. |
| Memory promotion tracker: `runtime/memory_promotion.rs` | Changed GitHub catalog, a promotion or outdated action, and the existing operation tick at most once a minute while an attempt runs or cleanup waits. | Read open rows and, after invalidation or restart, ended branches off-lock; inspect current PR and agent ownership under brief Runtime locks. The same off-lock worker retries eligible cleanup, at most sixteen branches per check; held shared branches remain pending. One effects gate serializes cleanup with retry reservation, and a busy gate defers tracking or refuses a start without waiting under Runtime. Unseen-agent spawn recovery keeps its existing five-second call and five-minute grace. Only changed store facts refresh opened Memory. | One check in flight and one waiting, one effects gate, one open promotion per card and three open suggestions per Project. The carried retirement map and full row/branch scans have no independent numeric cap; sixteen caps effects, not retained entries or scan work. | Stops minute checks after running attempts and retryable cleanup end; protected, dirty or committed outdated branches are kept without further cleanup polling. Runtime owns the carried map and gate; worker exit releases the gate, and a panicking check clears its flight. The worker has no retained join handle. |
| Link record worker: `links/worker.rs`, `links/store.rs`, `runtime/links.rs` | Condvar wake on changed facts or a panel request; list changed session files every 15 s, every 3 s while a panel is open (60 s overlap, a 90-day backfill once); pane spans re-applied every 60 s; each ready device asked at most every 60 s, a failed one left for 60 s; a store that failed to open retried every 15 s; prune every 30 min; summary published at most every 2 s. | Upsert the handed facts, read one queued session file for one 20 ms turn then rest 30 ms, answer the open panel after a write, and publish `link_summaries` or `link_panel` only when the value changed; local and connected-device facts are fingerprinted under the lock, so unchanged inputs hand nothing. Ledger writes and output reconciliation run on this worker; a changed record refreshes at most 33 cached identities (the root and 32 descendant edges). No `git` process and no read under `Mutex<Runtime>`. | One worker per Core and one write connection; the mailbox holds the latest facts of each kind and one latest panel request; one session file read up to 1 MiB per turn, resumed from its cursor on the next; a device answers at most 8 reads per call with a 10 s deadline; the file is capped at 64 MiB (`links_store_full`); the panel carries 200 lines. Ledger updates queue at most 512, cached contexts hold 512 identities, and at most 16 context reads are in flight; a record tree has 2,000 requests/nodes and 32 descendant edges, 200 flows or missed items per session, and summaries cover 16 projects with 2,000 sessions each. Crossing a ledger bound records a diagnostic. A list page names at most 2,000 files (newest first, walking at most 100,000 entries); the next page is listed only once the queue has drained. | `LinkWorker::drop` (dropped in `Drop for Core` after the search worker) marks stop, wakes and joins; a turn in progress finishes its one read first. |
| Local Explorer watch: `hided/src/watch.rs` | 200 ms poll of the core's own node and 200 ms coalescing window, or target-change notification; 2 s after a failed poll; with no renderer attached the target is none (`WatchService::set_drawn`), and the target the snapshot asked for is kept for the first renderer back. | Stamp the focused checkout's watched folders in one `Call::Stamps` to the core's own node, which runs it on a thread of its own (one thread start per poll, among at most 64 own-node calls at once) and broadcast one changed-folder frame per coalesced change; ignore an overtaken target. | 64 folders via the shared watch selection; one request at a time; 64 broadcast frames. | Clearing the target stops I/O, not the timer; last-renderer detach clears it with the core's `ui_attached`. The task ends with the daemon's async runtime. |
| Device Explorer watch: `hided/src/watch.rs` | 2 s or target-change notification; missed interval ticks are skipped; no target while no renderer is attached. | Stamp the selected visible device Explorer's folders in one awaited helper request; ignore an overtaken target. | 64 folders via the shared watch selection; one request at a time; no queue of periodic helper reads. | Clearing the target stops I/O, not the timer; the task has no separate stop handle and ends with the daemon's async runtime. |
| Pane-capability sweep/bootstrap: `hided/src/pane_auth.rs` | Sweep 5 s; bootstrap on accepted connection. | Sweep expired/unregistered checkout references on a blocking worker and answer admitted bootstrap requests. | 64 capabilities, 8 queued accept slots and 8 active bootstraps; the accept thread can hold one additional accepted stream while waiting to send, and the kernel listener backlog is separate. Sweep workers are spawned without a concurrency cap or join handle. | Shutdown ends the serving loop; `CloseOnDrop` closes its listener; registry revoke/Drop removes references; a started sweep may still finish. |
| A device's panes on its node link: `hide-host/src/panes.rs`, `hided/src/node_panes.rs` | Event-driven; the node sweeps its references every 5 s. | One blocking job per admitted proof or reader-callback control reply and one task per stream on the daemon; one thread per caller on the node. Reader callbacks only try admission and enqueue; no UI notification is added. Stream chunks are base64 on the link, written off the reader. | 8 streams per link, 64 pending 48 KiB chunks per stream (overflow ends the stream), 16 callers per node; per daemon, 32 proof jobs and 32 reader-callback refusal/close jobs, each allowance covering queued plus running work. Proof overflow answers `bridge_busy`; control overflow records `control_full` and closes only its link. | Every job owns its admission permit until it exits; a failed reply records `control_unanswered` and closes its link. Link close immediately rejects its credentials and wakes reader cleanup to revoke them and drop streams; the node's input ending stops its listener, streams and waiting proofs, and removes its folder. |
| Hook proof-and-answer: `hided/src/hook_request.rs`, `workspace_cli.rs`, `node_panes.rs` | One letter pull, Memory ask or read-only Workspace guidance request per hook child; no timer. | Reuse the bootstrap worker and fresh kernel proof, then the existing core read outside Runtime; a node forwards proof and command in one exchange. No WebSocket upgrade for these reads, no new subprocess in the daemon, and no additional UI notification. Persistent stream proofs still require fresh node/link/pane identity and fall back to inspection after ten seconds from receipt. | 16 KiB request and 64 KiB answer, the existing 8 local bootstrap workers or 16 node callers and 32 core proof jobs; overflow refuses rather than adding a queue. The Memory child keeps its 1,200 ms transport bound, and the core's read has one 1 s deadline, set as the daemon starts the read, that the owner-thread wait, a device call's slot wait, write and answer, and the store read all share, so none of its work outlives the child that gave up on it. | Letter and Memory tokens are internal one-shots revoked before answering, including on failure or unwind. Workspace guidance preserves the existing persistent reference lifetime and claims it only after acceptance; failed delivery revokes newly issued authority. A sent command is never retried on another route. The letter and Memory children retain separate cancellation and deadlines. |
| Browser-route reaper: `hided/src/browser_routes.rs` | 2 s. | Check retained route owners/load identities and release stale routes sequentially. | 12 routes, 4 route builds and 8 file requests per built route. | Shutdown exits the reaper and drains/closes its routes. |
| Node role's link watch: `hided/src/node_role.rs` (`watch`) | 2 s, only in a node-role hided (a placement record names its core). | Compare the wall and monotonic clocks and read the network address set; with the link down and a change, wake the dial only after an unreachable dial or a lost link; with the link up and a change, one ping to the core within 3 s; while the last failure was `unreachable`, resolve the core's name and read its SSH banner within 1 s as a whole. | One thread, plus at most one name resolution on a thread of its own that ends when the resolver answers; one ping or one banner read at a time; the dial backs off 2 s to 60 s. | Joined when the node role drops. |
| Link stream close worker: `hide-node/src/ssh/link_streams.rs` (one per link of a node that dials the core, started at its first close) | Wakes on a stream a runtime thread closed; a plain thread tells its own close. | One `LinkClose` call down the link, waiting at most 5 s for its answer. | 36 waiting notices (every stream a link holds), past which a notice is dropped with `link_stream.close_unsent` and the node's end closes with the link. | Ends with the link (its queue is dropped when the link ends) or when the link's last clone is dropped; it holds the link only while it tells one notice. |
| Node link heartbeat and attach limit: `hided/src/node_role.rs`, `hided/src/attach.rs` | The node writes a ping every 10 s; the attach role reads with a 30 s limit. | One line each way. | One per link. | The attach role exits with its channel, the core's socket or the 30 s limit; the node's writer ends with the link. |
| Supervisor and its roles: `hided/src/supervisor.rs`, `seat.rs` (PRD core-host-node-move) | Event-driven: a role ends, a move or a move back is asked for, or a pending core's lease runs out. | Run one role on the seat at a time (the core, the node, or the move's screen between them); a move's steps run on blocking threads, never under `Mutex<Runtime>`. | One supervisor and one role per process; one move at a time (`MoveControl`); a pending core whose move never linked gives its copy back after 60 s (`PENDING_LEASE`); a core stop has 20 s (`CORE_STOP_WITHIN`), past which the process exits unsuccessfully. | The process's stop ends the role it runs; a role that ends closes its sockets (`AppState::role_ended`). |
| Unlinked move retry: `hided/src/supervisor.rs` (`resolve_unlinked`) | Every 2 s (`RETRY_EVERY`) while the machine taking the core cannot be asked whether the move's link committed. | One `hided core-move` status call over SSH, bounded by the driver's 60 s step deadline (`STEP_TIMEOUT`). | One call at a time; no cap on the number of asks: this machine stays a node and asks until the peer answers, with the window showing the move waiting. | Ends when the peer answers committed or not committed, or with the process. |
| Core login item: `hided/src/login_item.rs`, `hide-platform/src/user_agents/launchd.rs` (launchd owns the process) | launchd: at login, and after a non-zero exit at most every 10 s (`KeepAlive` on an unsuccessful exit, `ThrottleInterval` 10). | `hided core-login`: start the machine's Herdr server when it does not answer, on the account's search with the login shell asked (up to its 10 s deadline), waiting at most 10 s for it, then run the core. | One job per state folder (`dev.withhide.core`, a hashed suffix for another folder); a core that exits 0 after its folder stopped or retired for a move back is not restarted. | Removed by the move back's retirement (`login_item::remove`); otherwise it stays until removed by hand. |
| Account search: `hide-platform/src/programs.rs` (`account_path_now`, `ACCOUNT_REREAD`) | At a daemon's start (`warm_account_path`) or a process's first `programs::command`, then on a call once the last read is 10 s old; no timer. | Stat the login shell's startup files, and start the login shell only when one changed (10 s deadline). | One read at a time, process-wide; only a process's first command waits for it, at most 11 s (`FIRST_READ_WITHIN`), and later callers take the last answer. | The read's thread ends with the read, a panic included. |
| Core output cap: `hided/src/login_item.rs` (`CORE_LOG_LOOK_EVERY`), `hide-platform/src/process.rs` (`set_output_aside`) | At the start of `hided core-login`, then every 60 s on one `core-log` thread for as long as that core runs. | One `fstat` of standard error and one of `Logs/core.log`; past 1 MiB a rename to `core.log.1` and two `dup2` onto a new private file. | Two files of at most 1 MiB and what 60 s writes past it; a child started before a set-aside keeps the file it was given. | Ends with the process; a system that sets no output aside (Windows) stops at the first look. |
| Core start and update wait: `hided/src/login_item.rs` (`EXIT_LOOK_EVERY`), `core_move/starter.rs` | Once a second while a newly started or replaced core is awaited. | Ask launchd whether the new job exited, and read the core's health. | 30 s (`STARTED_WITHIN`), after which an update starts the previous build again; one start or update at a time under the handover record's lock. | Ends with the answer or the deadline. |
| Login agent unload: `hide-platform/src/user_agents/launchd.rs` (`unload`) | 100 ms (`UNLOAD_POLL`) while a `bootout` that answered 0, 36 or 37 has left the job's teardown under way. | One `launchctl print` asking whether launchd still knows the job. | 30 s from the `bootout` (`STOPPED_WITHIN`), past which the unload fails with the time it waited; one unload per install, update or removal. | Ends when launchd no longer knows the job, at the deadline, or at the caller's stop. |
| Move check job: `hided/src/core_move/preflight.rs` | 100 ms while the one-shot check job in the GUI session runs. | Look for the job's answer file, and every fifth look whether the job exited without one. | 30 s (`LOGINS_ANSWER_WITHIN`); one job per check run. | The run unloads the job, and its `checks-<pid>` folder is removed on every path. |
| Delivery node clocks: `herdr-core/src/delivery/` (`node_clocks`), `Runtime::delivery_reach` | No timer of its own: read on the delivery store's existing maintenance pass. | Read under the runtime lock which machines are reachable, and advance the clock of each machine a letter waits on. | One clock per machine a letter waits on, inside the 1,024 open letters. | A clock goes with the last letter waiting on its machine, or when the machine leaves or becomes the core's own. |
| Daemon idle check: `hided/src/server.rs`, `env.rs` | 1 s check; Herdr probe no more often than 30 s with a 2 s request deadline. | With no clients, check Mobile keep-alive and Herdr reachability before advancing idle expiry. | One serial idle task; default expiry 600 s; keep-alive can disable it; no overlapping probe queue. | Idle expiry signals shutdown; normal server completion aborts the idle task; process/runtime exit also ends it. |
| Mobile push batches: `hided/src/mobile/mod.rs`, `mobile/push.rs` | An admitted announcing root transition for subscribed paired phones; independent of the desktop renderer. | Build notice-by-phone jobs and spawn one blocking task per burst, retaining a Mobile Arc and sending its jobs sequentially. | Encrypted records cap at 4,096 bytes; each POST has a 15 s timeout and refuses redirects. There is no application-level pending-batch/count/byte cap; a blocking-worker limit does not cap waiting tasks or whole-batch duration. | The batch JoinHandle is discarded; Mobile shutdown neither cancels nor joins admitted batches, and sends do not check stopping between jobs. |
| Mobile reconciliation/retention: `hided/src/mobile/mod.rs`, `mobile/phones.rs`, `mobile/store.rs` | 3 s observation clock; reconcile while enabled Settings is watched, or every 60 s when enabled/removal owed; phone sweep hourly. | Serialize reconciliation and sweep phones not seen for seven days, except live connected phones; changes publish existing Mobile state. | Pairing admits at most 4 phones and refuses further pairing with `phone_limit`; persisted-file loading has no equivalent count or byte validation. One reconciliation executes under the lock, but spawned reconciliation calls waiting for it have no fixed admission cap or shared coalescing flag; sweep work follows the loaded phone vector. | Mobile shutdown signals its stopping channel and removes the owned serve entry after any reconciliation; crash cleanup waits for next startup. |
| Phone connection/detail: `hided/src/mobile/phone.rs`, `mobile/mod.rs` | Live ping 20 s; open pane detail refresh 1 s; a phone's own `ping` after 20 s of its silence or when it returns to the foreground. | Ping/check silence, read the open detail's pane/conversation, and answer a phone's `ping` with one admission read and no core lock. | 2 live connections per phone, 8 queued detail frames, 16 KiB inbound frame, 10 s send bound and 45 s silent limit. | Socket close/revoke/eviction ends its loop; dropping the open detail aborts its detail task. |
| Desktop agent notices: `hided/src/desktop_notices.rs`, `mobile/mod.rs` (`follow`, `judge_pass`), `server.rs` | No timer of its own: the mobile follower, kept alive while Mobile follows a phone or a desktop window has either desktop notification switch on, wakes once per core notification burst. | The follower's one `core.snapshot` read off the lock per burst, shared with the phones; the desktop transitions judge the already-built projection only when it changed, and an announce or a clear is one serialized `agent_notice` frame broadcast to the desktop windows that asked for it. | One follower per daemon; one `Transitions` state; a 16-frame broadcast backlog per window, past which a lagging window misses frames and logs `agent_notice.lagged` (a missed frame costs a notification, never a wrong one); the judged states follow the projection's agents, and an agent that left is kept for its return for 10 minutes (`VANISHED_TTL`), with no separate count cap; the desktop host keeps at most 16 shown notifications to withdraw (`NOTIFY_KEPT`). | The last window that wanted it turning its switch off or closing releases the demand and resets the transitions; the follower parks when neither side wants it and ends with Mobile's stopping channel. |
| Desktop daemon watch: `desktop/src/main/host.ts`, `spawn.ts` | Health recheck 2 s after each response; lost-daemon status poll 3 s after completion. | One HTTP health read or attach-only `hide status`; discovery replaces the watch. | One watch timer and one CLI child; health deadline 1.5 s, status deadline 5 s; 2 health misses mark lost. | Host quit or new discovery clears the timer; quit kills its in-flight CLI child and leaves detached daemons alone. |
| GitHub project re-read and store: `runtime/projects.rs` (`reread_stale_github`, `read_sighted_pull_requests`), `labels/worker.rs` (`note_sighted`), `github.rs`, `github_store.rs` | UI-attached gate, like the other capability readers: no read and no clock comparison while no window is attached. Then a comparison on each coordinator wake; a project is asked for again 5 min after its last answer, or 30 s after it when the operator looks at it (window focus or project brought to the front; `read_when_looked_at`), never for a repository answered "no GitHub remote" whose remotes the catalog knows, until those change or a refresh asks, and not while its previous ask is unanswered; one `gh` pass per changed request. A pull request a local session printed is compared on the labels path whether or not a window is attached, inside the lock section that already hands the worker the pull request times, and only when a read found one; its `gh` read still waits for the gate. | Compare the instants of at most 64 projects (`GithubClock`), advance the generation of the due ones, and let the single reader read only those; after an answer that changed, copy the snapshot under the lock and write it on the store thread. Per transcript read, the worker checks each sighting against the times it holds (no lock); per exchange that carries sightings, the runtime looks each up in the times and its remembered set, and scans the read projects' first pull request and the navigator's panes once per new address; each clock comparison also drops the remembered addresses that grew old and the waiting projects no longer read (at most 64 entries). | 64 projects, the one in front and the ones a screen named first (a count past it is logged as `projects.over_limit` when it changes); at most 32 sightings wait in the worker between two exchanges, the newest kept and the rest logged as `read.sighted_capped`; at most 64 owned addresses remembered for 15 minutes or until an answer holds them, one past it not read and logged as `read.sighted_over_limit`; one reader worker; one store thread with one pending slot, so a burst writes the latest; the file is capped at 32 MiB on read. | The store's Drop joins its thread, so a queued write lands before quit; the reader keeps its existing `BackgroundRead` ownership. |
| GitHub advisory poll: `herdr-core/src/github_poll.rs`, `runtime/github_poll.rs` (`github_poll_request`, `ask_polled_changes`), the coordinator's `github_poll_reader` | UI-attached gate, like the other capability readers, and this machine's projects only. One worker pass every 60 s (`BackgroundRead` interval and spacing); on each coordinator wake the runtime builds the request under the lock and compares the polled projects' marks. | Per wake: one pass over at most 64 projects' last answers to build the request (owned strings, no I/O), and at most eight marked paths compared by the look rule; a pass's held projects (at most eight) flip their status to stale once, and an already stale one publishes nothing. Per pass: at most eight projects, each one conditional `gh api` for the open pull request list and two for each of at most four pending-check head commits, one `gh` at a time on the worker; a 304 costs no rate limit and a full reply is headers only (`--silent`). A pending head commit's combined checks result (one more `gh api --jq` read per moved list, a hundred check runs a page) differing, or the open list's digest (number, head commit and draft flag of each pull request, one more `gh api --jq` read after the list's tag moved) differing, marks the project; the existing reader reads it (nothing is published by the poll itself). A comment, label, review or `updated_at` that moves only the list's tag marks nothing. | 8 projects per pass and 4 pending head commits per project (72 entity tags and 8 digests at most, kept only for the last request's projects and endpoints); a project paused after a failure for 60 s doubling to 15 min, at most eight pauses; one marked path per read project (64); a pass stops at 45 s and after any `gh` the node bounds at 15 s; below a tenth of the rate limit nothing is polled until its reset. | The worker is the reader's one `BackgroundRead` thread; the reader's drop sets a flag the pass reads before each `gh`, so the worker ends after the command it is in. State and tags are in memory only and go with the reader. |
| Elapsed display clock: `web/src/components/elapsed.tsx` | One shared 1 s timer per window while subscribed. | Update the time and notify elapsed-span subscribers in memory; no core snapshot. | One timer; listener count has no fixed numeric cap and fan-out follows the mounted elapsed spans. | Last unsubscribe clears the timer; window destruction ends it. |

Agent notice sentences add one copy of the published `detail` (at most 80 characters) per phone-projected row, outside the runtime lock; the notice judgment borrows those rows and copies title, status, sentence and project only for a root transition that announces.
No input handler, tick, timer, subprocess, I/O, extra snapshot read or new receiver is added; a changed sentence alone publishes no notice, and the existing follower and demand cleanup own the extra values.
Phone fan-out stays per subscribed phone and desktop fan-out per requesting window; accepted encrypted push bodies are capped at 4,096 bytes, with an oversized JSON payload failing before POST through the existing delivery diagnostic.
The transition/disappearance ledgers and pending `clear` tags still have no separate count cap; the desktop's 16-frame backlog and host's 16 kept notices remain their existing limits, not limits on those ledgers.

Worktree, GitHub and disk readers also run on changed explicit requests; those requests are not extra permanent timers (`worktrees.rs`, `github.rs`, `disk.rs`).
Their command, walk and storage bounds remain in the feature cost contracts below and must still be reviewed when changed.
Agent registration and spawn records share the durable delivery worker: 2,048 agent records, 4,096 spawn intents, 128 native arguments totaling 8 KiB, the 16 MiB ledger bound and a 64-slot admitted-operation queue.
The shared `BackgroundRead` has one in-flight request and observes the latest desired request at its next poll; only owners that explicitly cancel and call `join_pending` obtain that shutdown guarantee (`reader.rs`, `session_sync/process_info.rs`).

## The spawn guard hook on a shell call

The `Bash` branch of the `PreToolUse` matcher runs `hide-agent-hooks` before every shell call a Claude Code or Codex agent makes, so it is a high-frequency path that every agent pays and no operator sees (PRD herdr-spawn-guard B13).
Per input it adds one process start: the helper reads the payload (at most 256 KiB, waiting at most 0.5 seconds), runs a byte test for `herdr` or `HERDR_BIN_PATH` over the whole payload (its `cwd` and transcript path too, so under a checkout whose path contains `herdr` every call also pays the JSON parse and the lexer, microseconds that start nothing), and exits with no output when neither is there, before it parses JSON or starts a child; the [project guardrails](#the-guardrails-on-a-pre-tool-call) read their policy file first.
A call that mentions `herdr` without launching an agent (`pane split`, `agent list`) pays one JSON parse and the shell lexer, and still starts nothing.
Only a launch in a Herdr pane starts a child: one `hide workspace bootstrap`, owned through `hide_platform::process` with the guard's 2.5 second deadline, a 16 KiB output cap, and an end of the child on success, failure and timeout alike.
The guard keeps no state, runs no timer or worker, takes no lock of the runtime, publishes nothing and fans out no notification; the daemon sees one `bootstrap` request per refused launch, the call `SessionStart` already makes.
The retained data is the refusal log, capped at 256 KiB with one rotation, and the throttled diagnostic store `delivery` already caps; crossing a cap rotates or suppresses, and never refuses a call.
A defect, a missing `hide` or a daemon that does not answer lets the shell call run, so the worst the guard can add to an ordinary call is its own bounded wait, never a refusal.

Measure it at the command boundary: the installed command line run as the runtime runs it (`sh -c`, the payload on stdin), wall time per call from the caller's side, against a process-spawn baseline, with the first calls discarded as warm-up (ten, and five for the real `hide` row).
Report p50, p95 and the load average before and after, and report the idle and driven (launch) paths separately.
Time the exact installed command through `sh -c` with the payload on stdin and retain the bounded measurement script and raw observations in the run directory.
Keep the command/binary hash, OS, load, sample count, warm-up exclusions, failures and path classification with the result.
The ordinary shell-call, Herdr-mention, stand-in refusal and real-daemon refusal rows are separate workloads.
Helper return time does not prove the agent's time to first token or its runtime's hook scheduling latency.
Observe the actual supported runtimes separately when claiming native behavior.
Regression owners are `hide-agent-hooks/tests/it/spawn_guard.rs` (nothing spawned for a payload without `herdr`, a slow `hide` stays under the budget) and the parser tests in `src/spawn_guard.rs` (the parser is `hide-shell`).


## Letters at a tool event

While an agent works its letters reach it at a tool event ([delivery.md](delivery.md#letters-while-the-agent-works)), which makes a tool call a high-frequency path for six agents.
Per tool result, OpenCode's plugin and the Pi and omp extension start one helper (`hide-agent-hooks <agent> letters`), which starts one sibling `hide inbox --hook --session <id> --tool`, a credential bootstrap and one typed hook request to the daemon; the core hands it to the delivery worker like a prompt pull, which reads the ledger it holds and renders at most five letters, with no subprocess and no network, and writes the ledger only when the pull's maintenance changed a letter.
The scripts bound it: only the pane's root session asks, with a 1.5 second budget past which the result goes on unchanged, inside the cap of eight running helpers, and a result that comes while letters the script handed over wait for their receipt pays a map read and no helper.
Grok's and Cursor's hook runs per tool call (their entries carry no matcher from version 6; Grok now uses `hide-guidance@7`, Cursor stays at 6); a call that runs adds the same `hide inbox` call inside the guard's 2.5 second budget, and a refused call adds nothing.
Grok's version 7 also starts one helper per `UserPromptSubmit`, which calls `hide inbox --hook --session <id> --prompt-digest <digest>` but prints and confirms nothing.
A root `Stop` with a session and `stopHookActive: false` adds one `hide inbox --hook --session <id> --turn-end`, and one confirmation command only when it printed letters.
Both events share the helper's total 2.5 second budget, including a 400 ms confirmation reserve; the subagent count runs after the Stop intake, so a stalled intake can delay that count by up to 2.5 seconds.
A repeated Stop with `stopHookActive: true` makes no pull and still updates the count.
The core saves only changed bell claims or handover counts; repeated observation of the same bell turn adds no claim write.
agy's `PreInvocation` hook already runs per model call; a call after the prompt's first adds the same `hide inbox` call inside the prompt hook's 1.65 second budget.
On Windows, agy's command starts one Windows PowerShell process between its existing `cmd` shell and the helper, at each selected hook event, to carry the command through agy's quote escaping ([agent-hooks.md](agent-hooks.md#antigravity-hides-hook-block)).
This shell runs only for that event; it adds no resident worker, timer, retained state, pending queue or notification, and leaves the helper's input and deadlines unchanged.
agy owns the command's existing eight-second timeout; this wrapper adds no separate descendant-cancellation guarantee.
Measure its cold call and warmed caller-side p50, p95 and maximum against a cmd command that actually runs the same helper, reporting the old escaped command's skipped helper separately rather than counting that skipped work as a working baseline.
Claude Code and Codex start no tool-letter helper: their letters are typed by the doorbell, on its own thread, as at rest.
Their passive Post helper cost is recorded separately below.
Nothing here keeps state, runs a timer or publishes; the retained data is the throttled delivery diagnostic.

Measure it at the command boundary as the spawn guard is measured: the helper's `letters` operation with its JSON on stdin, and the hook with each agent's documented tool payload, against a real daemon that holds no letter for the pane, wall time per call from the caller's side with the first calls discarded as warm-up, p50 and p95, the load average before and after; that is the cost per tool call, since every call asks.
The PRD that landed it set no bar (D-12); the figures measured on macOS are in the pull request that landed it, and a later change to this path reports its own.
Regression owners are `hide-agent-hooks/tests/opencode/plugin.test.mjs` and `tests/pi-extension/extension.test.mjs` (no ask while a handover waits for its receipt, a late helper leaves the result unchanged) and `tests/it/grok_cursor_hooks.rs` (nothing asked outside a pane or without a session).

## Passive Post observations

Claude Code, Codex, Grok and Antigravity add one native helper start per selected shell Post event; their ordinary result pays the bounded input read and local JSON/command filter, with no observation CLI child.
OpenCode 1/2 and Pi/omp filter the actual shell command inside the existing result handler before starting an observation helper.
Only a recognized direct prompt or agent start starts an owned sibling `hide observation --hook` (and, for starts, one more with `--start`), carrying one array of at most 64 observations of at most 8 KiB each within one shared 1.2-second send budget; the child asks the daemon once per observation.
The helper transiently bounds command text to 64 KiB and result text to 128 KiB, discards both after parsing, and never resends the original prompt.
No observation adds a Pre RPC, spool, timer, polling thread or detached process.
Plugin/extension helpers share the existing eight-child admission cap, 1.5-second outer budget and shutdown cleanup; saturated helpers report a bounded lost diagnostic and preserve the tool result.
Native hooks share the 256 KiB input cap and 500 ms input deadline, and always print no permission or replacement result.
The existing proof-bound daemon admission owns freshness and endpoint validation, and the existing links worker owns metadata storage; their queue, TTL and storage limits are described with the admission implementation.
No runtime mutex is acquired by this parser and it publishes no notification itself.
Measure ordinary Post startup, the existing ordinary Pre path and a matched candidate separately through their installed `sh -c` command, with the same payload size, process baseline, warm-up exclusions and caller-side p50/p95 methodology above.
A stand-in observation CLI measures local parsing and owned child transport only, not native provider scheduling or real proof admission.
Regression owners are `observation.rs` and the OpenCode and Pi/omp result fixtures; native provider event emission remains a separate observed result.

## The guardrails on a pre-tool call

The project guardrails run in the same helper, before the spawn guard, on every shell call and, from matcher version eight, on every file tool call of Claude Code, Codex, Grok and Cursor (PRD agent-guardrails B27, [agent-hooks.md](agent-hooks.md#project-guardrails)); a file call that started no helper before now starts one.
Per input they add one read of the policy file (at most 8 MiB, a few hundred bytes in practice), in any agent session, a Herdr pane or not, and, when no project of the machine has a protection on, nothing more: no JSON parse of the payload, no Git read, no child.
With a protection on, the call pays one JSON parse, the shell lexer for a shell call, and reads of the repository's own files (`hide_project::git`, no process) for each path it names; only a command that would discard changes starts `git --no-optional-locks status`, once per checkout and call, owned through `hide_platform::process` with a 2 second budget and a 4 KiB output cap; at most 512 words of one shell call are matched against the protected paths.
OpenCode's plugin and the Pi and omp extension read the policy in their own process, again when its file changes, and start no helper for a file tool while nothing is on.
The guardrails keep no state, run no timer or worker and publish nothing; the retained data is the shared refusal log and one `policy.unreadable` line per ten minutes.

Measure it as the spawn guard is measured, with the helper's own CPU time (`getrusage` of the child) beside wall time, because on a loaded machine wall time measures the scheduler: each runtime's file call and shell call, under an empty policy and under a policy whose only project is elsewhere, against `/usr/bin/true` measured the same way.
B27's bar is the no-protection call's p95 at or under 15 ms on macOS; report the load average with it, and treat a run whose `/usr/bin/true` p95 is above that bar as not measuring the helper.

## Resident work cost review

The active checkout Git header reuses the Changes pump's 250 ms wake, with one Git reader (two-second spacing after settlement) and one independent CI reader (30 seconds after settlement), never a reader per tab or repository.
Each wake adds scalar/request comparisons bounded by five commit identities and three short lock acquisitions; it does no machine work under the lock or on the input/session-sync thread.
Only the front Git checkout with an attached UI is admitted; Home, plain folders, detached renderers and inactive checkouts read nothing.
One Git job and one CI job may run, and only the latest target is pending; old generations and old SHAs cannot publish over their replacements.
Git retains counts, at most five subjects and five CI summaries, not paths from its status scan or a per-checkout cache; the node bounds status output at 8 MiB and log output at 64 KiB and reports overflow as failure.
Git's three-second budget and CI's one 20-second link wait bound shutdown joins; underlying node `gh` work has its existing 15-second command timeout and no new authentication path.
The existing gh pipe reader still lacks a retained-byte cap; this is a pre-existing boundary limitation, and the fixed query returns at most five oids, aggregate states and provider commit URLs rather than raw check lists.
Notifications fan out only on an actual target, fact, error/recovery or stale-state transition; identical status reads renew only the internal freshness deadline.
Regression owners are `checkout_header_reads_file_edits_unborn_head_and_tracking_ref_changes`, `header_fences_late_git_ci_and_reconnect_results_and_ages_without_false_clean`, and `web/e2e/checkout-git-header.spec.ts`; browser/native timings and captures belong to the run record, not an inferred responsiveness verdict.

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
6. Update this ledger and the [state/process ownership guide](ARCHITECTURE.md#state-placement-and-publication), then record applicable regression checks and any matched measurements in Evidence.

For example, a reader with one active job and one latest pending request has bounded pending work only if its request/answer sizes and owner count are bounded too.
If code lacks a bound, write that limitation in Review and the ledger; do not invent one or quietly alter behavior in a documentation-only change.
The [maintenance policy](#maintenance-and-review-policy) decides when actual app scenarios and controlled baseline/candidate measurements are required.
Unrelated documentation changes need focused documentation/invariant checks and must not claim measured performance parity.

## 1. Define the claim before running anything

Write the symptom, exact reproduction sequence, expected visible result, affected clients, and baseline/candidate revisions in the run record.
Separate these questions: does the content render correctly, how long does an internal stage take, how long until the user sees the requested change, and does retained memory grow?
A faster median does not prove flicker is fixed; a bounded cache does not prove lower p95 latency.
Agree on an acceptance threshold and measurement boundary before comparing results.
Historical measurements from another build or machine load are context, not universal pass thresholds.

Use the same pane content, grid, font/text scale, display, refresh rate, build configuration, and interaction sequence in both builds.
Compare idle and driven conditions separately, distinguish cold-start from warmed-up draws, and alternate baseline/candidate trials to expose changes in machine load.
Report sample count, nearest-rank p50/p95, maximum, excluded observations, and trial conditions, not only the best run.
If the conditions cannot be matched, label the result a functional reference or an uncontrolled observation, not performance parity.

## 2. Identify the build and protect the operator

Before visual checks, record the exact executable path, source revision, dirty diff, bundled Herdr version, build configuration, and PID for `hided` and, for the desktop app, its dev or packaged Electron process.
Inspect running processes rather than assuming the build launched from this checkout is the visible one.

```sh
pgrep -fl hided
```

The operator's app and an isolated candidate may run simultaneously; there is no global single-instance requirement.
Identify the candidate by executable path, PID, and exact window ID before capture or interaction, and re-resolve them after any restart.
Keep the operator's app running by default; building and automated tests do not require quitting it.
Never quit, restart, activate, or otherwise manipulate the operator's app, panes, or server for QA without explicit coordination; never kill all matching processes.
Prove the private server, socket, `HIDE_STATE_DIR` and `HIDE_DESKTOP_USER_DATA_DIR` are isolated before running both apps; ambiguous targeting or shared state blocks the affected check, not the operator's work.
Background exact-window screenshots need not activate the candidate, but foreground keyboard, IME, drag and focus scenarios can interrupt the logged-in user's work; coordinate a bounded foreground QA window for those, or use a separately authorized machine/session.

Build with the existing scripts:

```sh
bash scripts/verify-web.sh web build
bash scripts/verify-cargo.sh release
HIDE_VERSION='<candidate-version>' bash scripts/verify-web.sh desktop package
```

Replace `<candidate-version>` with the intended local candidate version before packaging.

`verify-cargo.sh release` builds the release `hided`, `hide` and `hide-agent-hooks` binaries; release `hided` embeds `web/dist`, so run `bash scripts/verify-web.sh web build` first.
`bash scripts/verify-web.sh desktop package` (`desktop/scripts/package.mjs`) runs that release build, fetches the pinned Herdr through `scripts/fetch-herdr-runtime.sh`, and packages `desktop/out/hide-darwin-<arch>/hide.app`; it refuses to produce an app if any binary it ships is missing or not executable.
For an unpackaged dev run, `pnpm --dir desktop dev` finds this worktree's `target/{debug,release}/hide`.
Archive baseline and candidate from separate worktrees so each keeps its own `target/` and `desktop/out/`; never redirect build output with `CARGO_TARGET_DIR` or share it across revisions.
Building, testing, installing, launching, committing, and merging are separate states; report each accurately.

## 3. Isolate runtime state before making fixtures

Read the current official CLI and socket references required by [AGENTS.md](../AGENTS.md#herdr-api-contract), and check the bundled binary's contract before using integration commands.
Use that exact binary for the private server, fixture CLI, and reference TUI, not whichever executable happens to be on PATH.
Every invocation, including cleanup, must use the same explicit routing environment:

| Input | Required isolation |
| --- | --- |
| `HOME` | A private home under the run directory, including kit and retirement state |
| Provider homes | Private `CLAUDE_CONFIG_DIR`, `CODEX_HOME` and platform home/config roots; exclude inherited credential and provider routing overrides |
| Provider executable search | A controlled tool `PATH`, private login-shell startup path and verified fixture resolution for supported provider names; a private HOME alone does not restrict executable discovery |
| `HERDR_SESSION` | A unique run-owned session name |
| `HERDR_SOCKET_PATH` | An unused short absolute socket path |
| `HERDR_CONFIG_PATH` | A private config file under the run directory |
| `XDG_CONFIG_HOME`, `XDG_STATE_HOME` | Private configuration and state roots under the run directory |
| `HERDR_PANE_ID`, `HERDR_TAB_ID`, `HERDR_WORKSPACE_ID` | Clear inherited identifiers before launching the fixture |
| `HERDR_ENV` | Clear inherited nesting marker when starting the standalone reference TUI |
| `HIDE_STATE_DIR`, `HERDR_BIN_PATH` | A run-owned `hided` state directory and pinned Herdr binary path |
| `HIDE_DESKTOP_USER_DATA_DIR` | A run-owned Electron user-data directory for the desktop app |

`desktop/e2e/fixture.ts` builds exactly this environment and refuses to launch unless `HOME`, `HIDE_STATE_DIR`, `HIDE_DESKTOP_USER_DATA_DIR`, and `HERDR_SOCKET_PATH` all resolve under the run's own temporary directory; `web/e2e/herdr-fixture.ts` does the same for the private Herdr server and clears the inherited `HERDR_PANE_ID`/`HERDR_TAB_ID`/`HERDR_WORKSPACE_ID`/`HERDR_ENV` identifiers.
The state directory must also be private (`0700` on Unix): the delivery ledger refuses an existing directory readable by another account, and timing hooks against a daemon logging `ledger.load_failed` measures a failed letter pull rather than working delivery.
`scripts/web-shell-measure/isolated-env.sh` establishes that permission before starting the daemon.
Follow that same pattern for an ad hoc fixture: forward `HERDR_SESSION`, `HERDR_CONFIG_PATH`, `XDG_CONFIG_HOME`, `XDG_STATE_HOME` and the resolved socket to every child Herdr process, so the server it starts and the core that reads it stay inside the same routing boundary.
Check the pinned runtime's path behavior when updating it.
The tested session layout stores sessions under `<XDG_CONFIG_HOME>/herdr/sessions/<HERDR_SESSION>`; changing `HERDR_CONFIG_PATH` alone does not isolate session data.
The client socket inserts `-client` before `.sock`; allow room for that suffix in the platform's Unix socket path limit.
Do not repurpose the shell's own `HOME`; pass a private HOME explicitly to fixture children, and do not assume a private local socket disables SSH discovery.
Inspect remote registrations, automatic SSH connection attempts, and remote client state too; an unexpected remote connection is an isolation failure to resolve before interacting.
SSH devices are registrations kept under the daemon's state directory (the operator's is `~/.hide/state`), so a run-owned `HIDE_STATE_DIR` is what keeps a scenario that does not exercise remote behavior from making an SSH connection attempt.
Do not edit the operator's SSH configuration or stop remote services to make a local fixture pass.

For a no-provider run, reuse the existing private provider shims and system-tool path pattern in `web/e2e/{herdr,hided,platform}-fixture.ts`.
Create the private executable set from the supported-provider declarations, keep inherited environment values to an explicit fixture allowlist, and link required tools individually instead of appending an account's executable directory.
Set private login-shell startup files to the same fixture-bin plus system-tool PATH, and repeat the actual login-shell resolution check after the runner has created its shell startup files.
Check the actual executable each supported provider name resolves to, its inherited and login-shell paths, and its private configuration roots before allowing the daemon's discovery, usage or Background AI work.
`hide-platform/src/programs.rs` searches the login-shell path, inherited tool path and usual installation folders; restricting only one of those inputs does not prove real provider binaries are unreachable.
Record the approved fixture executables and verify that fallback resolution cannot escape to an installed provider; never use a real model or authenticated probe to establish no-provider isolation.
Synthetic status or authentication answers from a fixture shim establish only that fixture's response, not real authentication or provider availability.
`--isolated-headless` selects browser mode and skips operator topology observation; it does not enforce this provider boundary.
Unexpected real-provider discovery or execution invalidates a no-provider measurement even when no authenticated request or model turn is established; stop and retire only the run-owned processes before correcting the fixture.

Save process/socket ownership before launch, prove the private server has zero workspaces before creating fixtures, and verify that the operator server gained no QA connection.
Use explicit fixture IDs, not current-focus shortcuts, for mutations.
Record operator agent count separately from private pane/agent and attached-child counts: isolation does not remove shared machine load.

## 4. Reproduce with real interactions

Sidebar tab segments are snapshot-derived, not a terminal-input path (`web/src/sidebarTabs.ts`).
Each affected checkout scans its tabs, physical panes, agents and visible ownership rows when those inputs change; terminal echo changes none of those inputs.
The feature adds no resident process, timer, queue, saved state or per-key I/O.
Sidebar grouping tests own count/root separation and cross-tab subtree ownership, and the web and desktop `sidebar-tab-groups.spec.ts` journeys own the live geometry, rename/reset and native-window observation, without implying a latency or memory improvement.

`desktop/e2e/*.spec.ts` and `web/e2e/*.spec.ts` are the primary reproduction path: Playwright drives the real window against an isolated `hided` and a private, pinned Herdr server, and both run in CI.
Read the closest existing spec before writing a new one; `desktop/e2e/browser.spec.ts` covers the occluded-window case; the fixture launches every desktop app with `--disable-backgrounding-occluded-windows` and without activating it (see [BUILD.md](BUILD.md#the-desktop-app)), so an occluded window keeps painting for capture.

For manual QA on a packaged or dev build that a spec cannot yet reach, use the tools and procedure in [VERIFICATION.md: Manual native QA](VERIFICATION.md#manual-native-qa); it owns which tool can address the candidate without reaching the operator's app.
Do not activate, raise, move or unminimize the window merely to obtain a screenshot without coordination.
Occluded-window capture is not proof of minimized, hidden, or off-Space capture support; report blank, stale, or unavailable frames explicitly rather than silently focusing the app.
Confirm the exact PID/window before each mutation, prefer fresh element IDs, and verify the result with another observation.
Do not automate credentials, unlock prompts, or authentication.

Exercise ordinary shell history, a long Claude transcript, and a long Codex transcript separately: their repaint and scroll behavior can differ.
Use disposable fixtures; do not resume or send input to an operator's live agent.
Capture real screenshots for visible claims; a running process or passing unit test is not UI evidence.
For flicker, record a bounded window region and inspect the resulting dimensions and frame rate before analysis; a blank-frame detector must be calibrated against the fixture's background and known populated/blank frames.

## 5. Measure the boundary actually under discussion

### CPU, lock contention, and idle work

Sample `hided`'s PID and its private Herdr server in separate short windows during both idle and driven conditions:

```sh
/usr/bin/sample <hided-pid> 3 -file <run-dir>/hided-sample.txt
/usr/bin/sample <private-server-pid> 3 -file <run-dir>/server-sample.txt
uptime
ps -p <hided-pid>,<private-server-pid> -o pid,ppid,%cpu,rss,etime,command
```

Retain contemporaneous load, process lists, selected pane/grid, attached children, and refresh rate.
Inspect symbolication before calculating mutex-wait ratios; predominantly `???` frames mean no usable answer, not zero contention.
State the denominator and thread when reporting a wait fraction, and distinguish waiting on the runtime mutex from time spent holding it.
During idle observation, inspect snapshot publications, `rest` revisions, attach counts, and git subprocess activity rather than inferring no work from a static UI.

A `hided` with no window is a separate idle condition, and it is the daemon's resident state: it lives as long as its Herdr server answers (ARCHITECTURE.md, The daemon lives with its Herdr), so it runs while the app is closed.
The historical usage ledger pauses source and price reads without a visible window; its owned thread waits for the latest visibility hint with a one-second maximum sleep.
Its public price table uses at most one persisted attempt a day, fetched by the own node from one fixed URL with a five-second deadline and 16 MiB cap; no usage payload or credentials are sent.
For first-backfill acceptance compare the same isolated candidate and terminal workload with an empty source and with the real source exposed read-only, recording source bytes/files, completion time, bounded/skipped status, idle and driven CPU, and echo samples separately.
Do not copy the full history or allow the candidate to write to the operator's source folders; record the read-only enforcement and exact candidate/window identities with the run.
With no web or desktop renderer attached, the coordinator rests screen-only foreground-process, hook-diagnosis, ports, worktree, GitHub, disk and Background AI reads.
The Changes pump and the Explorer watch follow the same attach state from their own owners: `changes_target` is none and both Explorer watch targets are none while no renderer is attached, and the persisted selection brings both back with the first renderer.
Historical usage source/price work pauses, but its stored engine report can still be read on the existing report interval; mobile and notification work keeps its own demand.
The coordinator gate also ends the Git watch (`git_watch.rested`), so no report waits for a reader that is not coming; the first read after a renderer attaches starts it again and reads every project, since a change may have landed while it was off.
Provider usage is a separate gate: `runtime/usage_limit.rs::reads_usage_headless` keeps it eligible for Memory or usage-limit demand, on the usage reader's own schedule.
Label and other non-screen resident owners keep their existing admission and stop rules; no renderer does not mean that all background work stops.
The root follower parks screen snapshot reads/sends while it has no clients and catches up when a client arrives; independent followers and core state publications are separate owners.
Report attached and detached conditions separately, including agent count, enabled Memory/usage work, links, phones and notifications, and verify the screen-only readers and root follower against their actual demand.
A change to the coordinator's readers or to the root follower has to keep this: a new reader that feeds a screen belongs in the gated block of `run_coordinator`, and `ui_attached` is sent by the daemon only, never by a window.

### Memory and long-uptime degradation

For an RSS claim, take eleven one-minute samples across ten minutes per build and retain process lists, endpoints, range, and median.
`scripts/web-shell-measure/memory.py` records one instant: browser-tree RSS, hided RSS and page JS heap.
`resources.py --memory-series` records the eleven one-minute process-tree RSS observations; it does not record a JS-heap series.
Keep pane count, occlusion, warm-up, and workload comparable; memory pressure can lower RSS without an allocation improvement.

A build that is fast at launch can still become slow after hours, because cost that grows with process age is invisible in any short window taken on a fresh process.
Whenever the report is "it got slow", record `hided`'s uptime first and treat a fresh-launch measurement as the baseline, not the answer.

```sh
ps -p <hided-pid> -o pid,etime,%cpu,rss,command
vmmap --summary <hided-pid> | grep -E "Physical footprint"
```

On macOS, `ps %cpu` is a decaying average over up to a minute, not a process-lifetime average; `etime` records age.
Use cumulative CPU `TIME` differences divided by monotonic elapsed time for matched observation windows, as `resources.py` does, and state whether the sample covers the root or its live descendants.
Compare peak and current physical footprint separately; a larger peak indicates a past transient working set and does not establish the cause of current latency.
The restart test separates accumulated state from workload: quit `hided`, relaunch it against the same private server so the panes and their processes are unchanged, and repeat the same sample window within a minute.
A hot path that disappears on relaunch and returns only after hours is accumulated state and needs an ownership or bounding fix, not a faster implementation of the same path.
A hot path that is equally hot on the fresh process is workload, and belongs to the boundaries above.
Record uptime, the restart time, and both sample windows in the run directory, and state which of the two conclusions the evidence supports.

## 6. Preserve the architecture while fixing the cause

- Keep subprocesses, blocking I/O, and large serialization outside `Mutex<Runtime>`.
  `snapshot_delta_payload` (`herdr-core/src/runtime/snapshot_delta.rs`) takes owned data under the lock; `serialize_snapshot_delta` serializes without a runtime to lock.
  Extend `PrecomputedCatalog`, `CatalogCache`, and `PathIndex` (`herdr-core/src/session_sync.rs`, `herdr-core/src/workspace.rs`) rather than adding per-tick or per-tab git calls; stale precomputation keeps the accepted catalog.
  The catalog's `BackgroundRead` worker asks the core's own node about every path in bounded `Call::PathFacts` chunks (at most `PATH_FACTS_LIMIT` paths and `PATH_FACTS_ANSWER_LIMIT`, 16 MiB, per answer; a larger answer is refused and records `catalog.path_facts_failed`).
  One read runs and changed inputs coalesce to the latest request; a completion wakes the existing republish channel, with no additional timer.
  The current connection and wanted paths fence fact acceptance; registrations and worktrees independently fence the prepared catalog at final application, so display metadata changes do not starve path facts and no delayed answer replays old topology.
  Identical facts keep their pointer and changed topology while a refresh runs repeats no catalog build when its catalog inputs are unchanged.
  Cancellation is checked between chunks; shutdown joins the current reader call outside Runtime, whose ten-second deadline bounds its wait, rather than claiming the whole multi-chunk read ends in ten seconds.
  The existing local node implementation can retain its detached host work and capacity slot after that wait times out; cancelling the reader does not interrupt that underlying host operation.
  The collected facts and request set remain proportional to the registered/session paths; the chunk byte bound is not a whole-request byte bound.
- Keep the session-sync thread that applies Herdr's events free of blocking work (PRD instant-pane-topology D-13).
  A socket read that opens its own connection goes to the sync-read worker (`session_sync/sync_reads.rs`, one read of each kind in flight), a subprocess, a node read that runs one or an HTTP request to a `BackgroundRead` reader (`herdr-core/src/reader.rs`), and a file write that ends in an fsync to its store's own thread (the label store); the work's output and interval stay what they were.
  An event that arrives while one of them runs is applied at once, which the topology scenario's churning agents exercise.
- Keep a publish's cost independent of the agent, tab and checkout counts when the catalog's inputs did not move (D-18).
  The catalog is rebuilt only on changed registrations, spaces or worktrees or its refresh window, the purpose mirror syncs only when that catalog or the unconfirmed created purposes changed, and a path is resolved by the node only when the set of paths asked changes (above), never by the core per publish; an agent status change repeats none of them.
  A new pane's cwd read answers publish only when the read changed a cwd, the pane was shown at a sibling's cwd other than the one it confirms, or the publish before it was held for that pane (PRD grid-ahead-of-herdr D-12): a split or a new tab publishes once with a stand-in cwd and the answer that confirms that same cwd publishes nothing, where it repeated the whole publish before (21 of 21 answers at operator scale changed nothing).
  Measure a change to this at both scales of the topology scenario, idle and with its agent churn, against a baseline: idle CPU and RSS must not grow, and a status-change publish must not double when the agents do.
- A drawn-ahead change adds one publish, a bounded line (`GEOMETRY_QUEUE_LIMIT`, `INPUT_HOLD_LIMIT_BYTES`) and one `pane_op.timing` record per operation, and nothing per frame or per tick.
  A terminal's PTY is resized once per drawn layout change its rectangle takes part in, at the drawing, and once more only when Herdr's layout differs from the drawing (PRD grid-ahead-of-herdr D-03, D-04); nothing holds a grid for Herdr's confirmation, and nothing compares a held set per publish.
  For a resized pane the record's `first_frame_ms` is the first frame after its resize went out and may precede `applied_ms`; a change that leaves a pane's grid where it was sends no resize and its record ends `incomplete`.
- Announce once per burst and clear the `ChangeNotifier` (`herdr-core/src/handle.rs`) latch before taking the snapshot lock.
  Read-then-clear can swallow a concurrent change.
- Size snapshot traffic by changes: rarely-changing revisioned `rest` and per-event scalars; terminal output never rides a snapshot (it goes from the node to the hub, see `docs/ARCHITECTURE.md`, The terminal path).
  The shell negotiates `semantic-rest-v1`, whose deltas send changed navigator/status fields and lists rather than the whole `rest` object; a client without that negotiation still receives legacy complete `rest` when it moves.
  An unused heartbeat can still create unnecessary changed-field traffic, notification or capture work; count semantic-section bytes separately and never infer a full navigator resend from one changed scalar.
  Initial snapshots and cursor resets carry complete state, so distinguish them from steady-state deltas.
- Keep the keyboard path to the byte bridge: a key goes from the web socket to the router to the pane's writer and never takes `Mutex<Runtime>` (PRD core-host-node-terminal B5).
  The node adds the operator-submit check per key (`hide-node/src/terminal/input.rs`, `submits`, PRD overview-request-view D-19): a chunk over 64 bytes is skipped and a shorter one is scanned in place.
  The core hears a key only as an input fact: the first key after a quiet second, an Enter and a key that moves the keyboard at once, and the keys between folded into one report a second later; a report is one push onto the report channel, and the core takes waiting reports into the runtime in one batch and publishes once.
- Terminal selection tracking (`web/src/terminals.ts`) retains one original range, its selected rows and Copy text per pane while the highlight is clipped, bounded by the grid at selection time.
  A wheel still coalesces into one scroll dispatch per animation frame; tracking adds no core notification, timer or input queue.
  Only a pending scroll installs a write callback: it reads the selected rows first, and searches the current screen only after those rows change.
  A settled clipped selection retains its original ends without adding work to ordinary output; clearing, replacing, resizing or parking the selection releases its tracking state.
  `web/e2e/terminal-selection-scroll.spec.ts` owns the partial-edge return and original Copy text regression alongside the existing continuous-scroll and offscreen-copy journey.
- Keep async operation records bounded by active intent and conflict scope; a tab's geometry operations wait in its line, one with Herdr and at most `GEOMETRY_QUEUE_LIMIT` behind it.
  A close or topology mutation uses an absolute five-second stage deadline; expiry becomes a caller-visible unknown result and never schedules a destructive resend.
  Status checks are read-only and are started only for an ambiguous close or an explicit status action, so unknown activity does not become a polling loop.

Follow engineering principles 1, 7, 12, and 13: remove obsolete paths, reuse existing mechanisms, test observable outcomes, and fix the failure class.
For timing tests, distinguish a deterministic policy threshold from eventual UI delivery under scheduler load.
Do not weaken an externally promised deadline to make a flaky test pass.

## 7. Regression gates, cleanup, and verdict

Use the existing suite wrappers, then the applicable gates in [CONTRIBUTING.md](../CONTRIBUTING.md):

```sh
bash scripts/verify-cargo.sh test
bash scripts/verify-web.sh
```

Run the full relevant suite before delivery, and retain pre-fix failure plus post-fix success for the specific regression when feasible.
Do not substitute a unit test for end-to-end coverage or an end-to-end smoke test for long-duration/load coverage.

Stop only owned recording/logging processes, the test app, fixture clients, and the explicitly routed private server.
Verify their exit and socket cleanup before removing or trashing only the exact recorded private state paths.
Never use broad process-name kills, a workspace root as a deletion target, or an unscoped `herdr server stop`.
Leave the operator's app untouched unless its shutdown was explicitly coordinated; in that case restore it afterward.
Verify the owned candidate exited, the operator's app remains available, and operator server ownership is unchanged; multiple independently identified instances are not themselves a verification failure.

The run verdict must include:

- Exact revisions, bundles, configuration, PIDs, runtime pin, and isolation evidence.
- Reproduction steps and observed results per client/scenario, with local evidence paths.
- Metric boundaries, distributions, sample counts, exclusions, load, and comparison conditions.
- Regression tests and gates run, failures, and remaining checks explicitly marked unrun or blocked.
- Cleanup/restoration evidence and whether the fix was merely committed, built, installed, or merged.

Use a qualified verdict when coverage is bounded: "no whole-body blanking observed in these recordings" is supportable; "all performance issues resolved" is not.

## Web shell echo and frame measurement

Published figures and their measurement boundaries live in the dated [performance observations](PERFORMANCE_RESULTS.md).
That report is an observed candidate result, not a new threshold or a replacement for the measurement contract here.

`scripts/web-shell-measure/run.sh` exercises the product `hided` with separate screen-key and driven-output workloads.
Baseline and candidate must use the same driver revision, fixture, browser mode and measurement boundary.
It owns every process it starts: an isolated pinned Herdr server on a socket inside a run-specific mode-0700 directory under `/tmp` (`isolated-env.sh`, the same routing table as section 3), one linked Git checkout and workspace with a `stty -echo -icanon; cat` pane, the release `hided` with its embedded `web/dist`, and one Google Chrome with an automatically assigned CDP port on the page opened with `?probe=1`.
The runner reads Chrome's CDP port from its own profile and sends the page URL over standard input so the token is absent from the Chrome command line.
The operator's socket is only read, before and after, for the topology counts written beside the results.

Echo is three trials of fifty screen-key samples from `key-echo.mjs`, each timed from renderer keydown to xterm write completion on the page's clock.
The collector records the boundary, expected sample count and successful completion explicitly.
`summarize.py echo` rejects missing, incomplete, negative, nonfinite or nonnumeric samples and an unproven clock boundary; one invalid trial prevents an aggregate verdict, and invalid input exits nonzero.
A valid run reports distributions with `pass: null` until the caller supplies `--max-p95-ms N`, the acceptance limit agreed before the run; a failed explicit limit also exits nonzero.
No historical S0 value is a default threshold.
The removed CLI-return collector mixed clocks and could start after output was already parsed, so its historical files cannot pass the current summary contract.
Neither the screen-key samples nor xterm write completion prove physical-key-to-screen latency or compositor presentation.

Frames (PRD B12) is one 120 s window with each driven pane printing a sequence-numbered line every 8 ms while a `requestAnimationFrame` loop injected through CDP records every frame's `dt`.
The collector reads each pane's parsed buffer at most twice a second, retaining the first and last advancing sequence in each third of the window; at most six panes retain 36 progress records.
Sequence and retirement records accept trailing terminal-cell spaces, while prefixes, embedded command text and records from another run are not evidence.
This observer cost is part of the measured workload and must be identical in a baseline and candidate comparison.
The frame summary requires advancing output from the same run in every pane and every third, a matching completed window of at least 120 s, valid durations, and confirmed writer launch and retirement before evaluating the existing 16.7 ms budget and one-percent fraction.
CLI success, WebSocket traffic and a final pane tail are diagnostics, not proof that a driven workload persisted.
The progress observations establish sampled workload coverage, not an output-rate or input-latency guarantee.
Writers run in the foreground and finish with a run-specific marker; cleanup interrupts every attempted writer, checks its final marker, cancels the owned frame callback and closes the CDP connection.
Each writer has a deadline beyond the measurement window, and unconfirmed retirement fails the run so the enclosing private-server owner can finish cleanup.
Failure still emits the collected frames, workload observations and bounded pane diagnostics as an invalid measurement document, with a nonzero exit status.
Diagnostics retain at most 4,096 UTF-16 code units per pane tail before and after stopping, plus one native tail on failure, for at most six panes; these reads are outside the measured window.
An output-only frame window contains no injected echo markers; use the separate `keys` scenario for input timing while other panes print.
A callback interval remains distinct from native presentation.

Run it as `HIDE_MEASURE_RUN_DIR=agents/runs/<slug>/measure/<attempt> bash scripts/web-shell-measure/run.sh` after `bash scripts/verify-web.sh web build` and `bash scripts/verify-cargo.sh release`.
For an unattended comparison, append `--isolated-headless`: Chrome has no native window and the runner records that operator topology observation was skipped, without contacting the operator socket.
Use that same browser mode for baseline and candidate, and report it with the results; a headless measurement does not prove native presentation.
`MEASURE_SCENARIO=areas2` or `areas3` shows two or three Agent tabs through the real area menu, each with one pane.
All shown panes receive the same line-every-8-ms driver during the frame window; idle key echo is measured separately before it.
`resources-idle.json` and `resources-driven.json` record twenty one-second CPU-time deltas and RSS sums for the owned hided, Herdr and Chrome process trees, independently of the echo and frame samples.
Each sample also keeps the root's own RSS and each child's RSS by its accounting name, because a child caught between fork and exec reads as a second copy of its parent's pages: a bound on hided's memory reads `hided_self_rss_max_kb` (hided alone), and the tree sum says what its children add.
For `single`, `multi`, `areas2` or `areas3`, append `--memory-series` after `--isolated-headless` to extend the driven frame window to ten minutes and collect eleven one-minute process-tree RSS samples in `memory-series.json`.
The `keys`, `device`, `remote-core` and `topology` branches exit before that sampler; the flag does not add a series to them.
A first run opens on Main (PRD S6), so the harness uses the Projects sidebar to open its linked fixture checkout before measuring.
The fixture workspace reports the checkout's `hide_owner` token before startup so the core reuses the prepared pane rather than creating a different workspace.
The Projects control is a tab, so readiness reads `aria-selected`; opening the fixture is one click after its checkout row appears.
At a scale the fixture's project is the one without agents, so its row waits behind the No agents fold, its project's own fold and its empty or inactive worktrees; the harness opens those folds on the fixture's project alone, each once, because a fold is the core's state and a second click closes it.
Cleanup removes the private socket's label-generator lock as well as its socket before removing the owned socket directory.
The run directory keeps `identity.txt` (head, dirty count, binary hash, Herdr and Chrome versions, load), `echo-*.json`, `frames.json`, both summaries and the owned PID status at cleanup.
The harness resolves that directory to an absolute path before starting child panes, so their private HOME, state, and checkout paths remain valid after the pane changes directory.
`MEASURE_SCENARIO=multi` is the S2 shape (PRD web-shell-pivot-s2 D-08): the measured pane shares its tab with four splits and four more tabs are shown once each so the core holds five attached tabs before the shell returns to the measured tab; `page.json` records the pane, split and tab counts, the attached pane ids and the live xterm instances the run started from (every attached pane keeps its instance parked while its tab is hidden, D-05).
`memory.py` samples resident memory twice, after the shape settled and after the driven window (`memory-settled.json`, `memory-after-frames.json`): the Chrome process tree's RSS sum, hided's RSS, the page's JS heap and its live instance count; the two samples say whether the parked instances grow the tab under load, which is the D-05 revisit trigger.
The driver and the marker still go to the one measured pane, so the other panes are idle shells with mounted xterm instances.
This five-tab fixture does not cross attach eviction.
`runtime::tests::terminal::only_the_last_five_shown_tabs_keep_their_panes_attached` proves the core policy; separately visit a sixth tab, require the oldest attachment and renderer instance to retire, revisit for restored content and new ordered input, then repeat the cycle.
Core metadata assertions alone do not prove renderer disposal or long-lived heap stability.
The same validity checks and explicit echo-threshold contract apply to every fixture shape.
A headed run opens Chrome with backgrounding/occlusion switches disabled; a headless run has no native window.
Record that mode and actual callback cadence with the load, and do not focus or move the operator's windows to improve a measurement.
Neither the switches nor a callback count prove physical compositor presentation.

### Pane topology latency

`MEASURE_SCENARIO=topology` measures how long a split, a zoom and unzoom, a pane close, a new tab and a tab switch take to reach the screen (PRD instant-pane-topology D-15, B23).
It adds a second tab to the measured workspace, and with `MEASURE_SCALE=operator` or `double` `scale.sh` first fills the private server to that scale (D-18): 43 workspaces, 62 tabs, 66 panes and 30 agents of which 5 print a line every 50 ms, or twice each (86, 124, 132, 60, 10); every other workspace is its own Git checkout, agents are reported with `herdr pane report-agent`, and while the run lasts one idle agent changes state every two seconds, the agent-status-only publish D-18 prices.
The run records `resources-idle.json` before anything moves, one echo trial under the agent churn (`echo-summary.json`, the typing half of B24), and `topology.mjs`'s rounds (`MEASURE_TOPOLOGY_ROUNDS`, 20 by default), then skips the frame window.
The initial resource sample precedes topology/status churn, while `memory-after-topology.json` follows the driven phase.
This scenario currently collects no continuously sampled driven CPU interval; its initial resource result is not driven CPU evidence.
Keys are CDP `Input.dispatchKeyEvent` events, which Chrome delivers to the focused terminal as an ordinary renderer keydown, so each chord takes the web shell's own keydown path and its browser chord; a tab switch is a click on the tab.
Every time is on the page's clock from the chord's first keydown (or the click): `screen_ms` is the first animation frame whose DOM shows the change (the canvas's pane count, its zoom flag or the shown tab), and `frame_ms` the first frame after it in which the terminal the change is about shows it (text in a new pane, a new grid in a resized one).
`topology-summary.json` gives each operation's p50, p95 and max to screen and to frame.
`MEASURE_HIDED_BIN` points the run at another hided, such as a baseline built from the merge base, so baseline and candidate share the fixture; run both headless (`--isolated-headless`) under the same scale and report the load beside each.
Herdr's share and Hide's are read from the candidate's `pane_op.timing` lines in the run's `hide-state` diagnostic log (Drawn ahead of Herdr in [ARCHITECTURE.md](ARCHITECTURE.md#drawn-ahead-of-herdr)), which the baseline does not write.

### The nightly performance lane

`nightly.yml` calls `perf.yml`, whose one job (`performance (macOS) / topology operator`, on `macos-15`) measures the same fixture every night against the product `hided` built from that night's `main`.
It builds the web shell and the release `hided` at the fixed `target/release/hided`, fetches the pinned Herdr, and runs `run.sh --isolated-headless` with `MEASURE_SCENARIO=topology`, `MEASURE_SCALE=operator` and `MEASURE_TOPOLOGY_ROUNDS=20`, the run directory under the job's workspace.
The browser is the hosted image's Google Chrome, headless; `run.sh` starts it with `--remote-debugging-port=0` and reads the port Chrome chose from `DevToolsActivePort` in the run's own profile.
`perf.yml` also has a `workflow_dispatch`, so a branch can be measured alone without starting every nightly lane.
Nothing a pull request runs depends on it, and no pull request runs it.

The fixture is the measurement's identity, so it is fixed:

| Part | Contract |
| --- | --- |
| Topology | `scale.sh` at `operator`: 43 workspaces and 43 projects (the measured checkout, a linked worktree of the fixture repository, and 42 others that are each their own Git checkout), 62 tabs (the measured tab and the switch target, one root tab in each other workspace, 18 more), 66 panes (one per tab and four splits) |
| Agents | 30 reported with `herdr pane report-agent`, idle at first; 5 of their panes print a line every 50 ms after the idle sample, and one idle agent changes state every 2 s through the echo and the topology rounds |
| Idle resources | `resources-idle.json`: 20 one-second samples of the owned `hided`, Herdr and Chrome trees before anything moves |
| Echo | One trial of 50 screen keys on the measured pane under that churn, renderer keydown to xterm write on the page's clock |
| Topology | 20 rounds of split, zoom, unzoom, close and new tab by chord, and two tab switches by click, each to screen and to terminal, with Herdr's share and Hide's from `pane_op.timing` |
| Memory | `memory-settled.json` and `memory-after-topology.json` |
| Browser and runner | Headless Google Chrome at 1280 × 900 on the `macos-15` hosted image |
| Driver | `scripts/web-shell-measure/` at the run's commit |

`scale.sh` writes the counts the private server answers beside the ones the scale asks for (`scale.json`), and a run whose counts differ is not this fixture.
Changing any row of this table, `scale.sh`'s operator numbers or the driver's measurement boundaries starts a new series: do it in its own pull request that says so.
Each record carries the driver's hash, the runner image, the Chrome, Herdr and `hided` identities and the load, so a reader compares only records of one series.

`scripts/web-shell-measure/nightly.py` judges the run and writes its record, `nightly-summary.json`.
It derives every figure again from the raw files through `summarize.py`, writes the same table to the job's summary page with the record beneath it, and fails the job in three cases only:

- the fixture is not the lane's: another scenario, scale or browser mode, or a private server whose topology is not the one `scale.sh` asked for;
- a measurement is invalid by `summarize.py`'s own contract: an echo with missing, incomplete, negative or nonnumeric samples or an unproven clock boundary; or
- a measurement is missing, because the run stopped before it.

No figure is a limit.
The guide's echo comparisons are relative to a matched baseline, never absolute, so the echo is reported with `pass: null` and no `--max-p95-ms`.
`summarize.py topology` has no validity contract, so a topology timeout is counted in the table and the record and is not judged.
The frame window, whose 16.7 ms budget and one-percent fraction are the one limit this guide states, does not run on the hosted runner: headless Chrome calls animation frames at a nominal 60 Hz whatever the load, so every interval sits on the budget and the strict `dt > 16.7` check counts the browser's scheduler.
The dated headless runs in [PERFORMANCE_RESULTS.md](PERFORMANCE_RESULTS.md#headless-sustained-output-observation) failed it with 38.8% and 24.9% of intervals over while their p50 and p95 were both about 16.7 ms.
The frame window therefore stays a local check on a native display, and the lane covers it as follows:

| Measurement | Nightly lane |
| --- | --- |
| Idle resources, echo under churn, topology latencies, memory | Covered, recorded without a limit; invalid or missing fails the job |
| Frame window (16.7 ms budget) | Not covered: headless 60 Hz frame scheduling on the hosted runner |
| Driven CPU, key count, windowless, device and remote core | Not covered: not part of the `topology` scenario |

A failed check is annotated as `Failed measurement` (`ci-flaky-report.py --measurement`), and the nightly report opens one `nightly-failure` issue per check, comments on it each night it fails again and closes it when a scheduled run passes the lane ([TESTING.md](TESTING.md#nightly-failures)).
Its candidates are every pull request merged since the lane's last green nightly, because a measurement can move with any change; a run that stopped is filed by its failed step instead, with the lane's workflows and scripts as candidates.
The authors of the candidates look first.
A fixture failure means the driver, the pin or the runner no longer builds the fixture; an invalid echo means keys stopped reaching the terminal the way the probe measures, a product defect candidate.
When no candidate explains it, for instance a new runner image or Chrome that the record's identity shows, the issue goes to the operator, who decides whether the series has to start again.
A green run proves the measurement was valid, not that performance held; read the trend.

Each run keeps its run directory, without the throwaway Chrome profile, as the artifact `perf-run-<date>-<run id>-<attempt>` for 30 days, and its record alone as `perf-record-<date>` for 90.
Read a series with `gh run download <run id> -n perf-record-<date>`; nothing about a run is committed.

The job costs about 22 runner minutes a night on `macos-15`, most of it the release build.
Its first run on a branch, with no Rust cache, took 21.8 minutes: about a minute of setup, a minute for the web shell, 16.4 minutes for the release `hided` (14.4 in another run) and 3.6 for the measurement.
Main's run saves the release dependencies to the cache, which saves about a minute and a half at most, because `herdr-core` and `hided` themselves are most of the build.
The job stops at 45 minutes and the build at 30, so a stalled run is cut short rather than holding a runner.

### Populated local and device catalog comparison

The ignored Rust integration journey `responsive_state::connected_catalog_measurement` reuses the private SSH delivery fixture and the existing CDP topology, key echo, key count, resource and WebSocket drivers.
Run it through `scripts/verify-cargo.sh test-scoped -p hided --test it responsive_state::connected_catalog_measurement -- --ignored --nocapture`.
It requires `HIDE_E2E_HERDR_BIN`, an explicit worktree `target/release` directory in `HIDE_E2E_CLI_DIR`, that worktree's current `RESPONSIVE_BINARY_HEAD`, and fixed `RESPONSIVE_NODE_BIN` and `RESPONSIVE_CHROME_BIN` executables.
`RESPONSIVE_MEASURE_LABEL` names a new local-only run directory; `RESPONSIVE_MEASURE_SCALE` chooses `small` or `operator`.
The source and executed binaries are fingerprinted separately, because the owned SSH fixture strips and signs its staged copies; the record also includes the driver files, browser, local and remote Herdr, installed private helper, process identities and actual Herdr topology.
Build the measured release binaries at the recorded source head before starting, freeze the driver files, and interleave baseline and candidate runs under the same scale, browser, viewport, DPR and host workload.

Each project has forty branches, forty private fixture pull requests and a linked checkout, and one connected private device has a populated session.
No real GitHub account is used: the fixture's `gh` answers those exact catalog commands and refuses other commands.
The visible tab has five panes; operator scale has 43 workspaces, 62 tabs, 66 panes and thirty reported agents.
Catalog preparation and all topology changes finish before the observation windows.
Idle has no printers or agent churn, then separate windows observe echo, repeated zoom, eight agent status transitions, continuous output with zoom and echo, and a thousand ordered screen keys.
Four split panes print in small scale and those four plus one offscreen pane print in operator scale, using one finite Python process per pane.
Each window records one-second root and process-tree CPU deltas, RSS, host load and application WebSocket payload bytes by type and semantic section.
JSON section sizes exclude their enclosing punctuation, and payload bytes exclude WebSocket framing and compression.
The final window navigates the only page to `about:blank`, records the owned daemon's resources while output continues, then reconnects the page; Chrome remains alive, so this does not observe native window retirement.
An empty page wire record and the driver's declared client count do not measure the daemon's actual connected clients.
Verify subscriber retirement and reacquisition separately by sampling the private daemon's `/health` client count and PID as owned renderer tabs open, close, reopen and close, and verify native last-window retirement in its own isolated desktop journey.
The source-build record does not establish a native screenshot or physical key-to-paint latency.
The page drivers retain their page-clock DOM, terminal grid and echo endpoints, and an owned native candidate is verified separately.

After the page windows finish, the fixture ends its measured daemon before a separate observation-only core probe attaches to the same private Herdr.
Build that probe with the pinned toolchain on PATH using `python3 scripts/web-shell-measure/build-core-cost.py <worktree> legacy` for the baseline and `typed` for the candidate.
It links the measured worktree's existing release libraries; its identity records their hashes, source head, compiler and common driver hash without changing either product's source or Cargo configuration.
The real Local node supplies the populated catalogs, and the adapter preserves its reader, progress, lifecycle and path identity behavior.
One actual PathFacts call is held for the same 500 ms observation interval in both builds while Herdr confirms a zoom; the core observation and later convergence are recorded separately from page latency.
A hundred sidebar state changes record the owner read and transport encode costs: the baseline owner read includes its first JSON encoding and the transport includes its parse and envelope encoding, while the typed candidate captures immutable data and encodes once at transport.
Measurement waits for both the populated local catalog and a remote session with workspaces, and saves that actual initial inventory.
Each update preserves the UI event's required expansion, selection and shortcut fields; each measured snapshot must carry the requested sidebar width, recorded beside that sample.
Dispatch's changed result alone proves neither a valid payload nor that the requested state was applied; any mismatched observation invalidates the run.
These clocks exclude observation parsing, daemon owner-channel queueing, sockets, terminal drawing and native paint, so only their combined processing cost is directly comparable.

The fixture owns its private home, state, sockets, SSH service, staged binaries and child processes, and ends them on success or failure.
The page driver caps concurrent children at three and terminates each within three minutes; the outer journey allows at most ten minutes for that driver.
Operator topology and catalog preparation each have an explicit three-minute fixture bound.
These limits bound a test's resources, not a product acceptance latency.
Failed setup or changed driver files invalidate a comparison; keep the run as diagnosis evidence and exclude it from performance results.

### Screen key echo, lost keys and the terminal path's cost

`key-echo.mjs` types into the measured pane's terminal with CDP `Input.dispatchKeyEvent`, which takes the shell's own keydown path and its `/ws` `key` frame.
Each sample is one key: the marker `aNNNN` is typed and waited for, then its last key `z` is typed, and the sample is the xterm write completion that first shows `aNNNNz` minus that key's keydown, both read on the page's clock (the keydown by a capture-phase listener the harness installs on `window`); the pane runs `stty -echo -icanon; cat`, so the drawn character is the pane's echo.
The CDP dispatch hop before the keydown is left out; these figures characterize the path from renderer keydown onward, and do not establish physical-key-to-screen latency or the cost of input delivery before that listener runs.
The harness refuses a sample that is nonfinite or negative on that one clock, or whose keydown never reached the page, rather than counting it.
Beside each sample it keeps the same completion minus the driver's own clock before the key was sent (`node_t0_samples`) and the driver's offset from the page's clock, read by a round trip bracketed on the driver's clock (`clock_offset_ms` per hop), and `summarize.py echo` reports both per trial; they are a cross-check, not the measurement.
Until 2026-10-10 the sample was that two-clock value: the driver and the page each fix their time origin when they start, and the constant between them changed per run (+75, -21 and -22 ms in three runs of PR #898's comparison), so key echo numbers measured before then carry an unknown per-run offset; only a large one showed, as impossible samples.
The key collector also records every `/ws` frame the page received while it typed, by type (`ws_frames_by_type`), so a key that made the core publish shows as a `delta` frame (B5).

`key-count.mjs` types `MEASURE_KEY_COUNT` keys (10,000 for the claim) one CDP `Input.dispatchKeyEvent` each, the same key path as `key-echo.mjs`, spelling distinct markers (`mNNNNN` and a space, Enter after every 100th, `#` last), then reads the pane's logical lines back from the pane's own isolated Herdr and compares them with what was typed: `exact` when they match key for key, otherwise how many keys never arrived (`keys_lost`), arrived outside the longest in-order common run (`keys_reordered`) or arrived without being typed (`keys_unexpected`), with the markers that are missing for a reader (D-08).
CDP `Input.insertText` does not reach the shell's key path, so it cannot stand in for typing.
It never stops at the first loss.
`MEASURE_PANE_READ` replaces the read command, which is how a device pane is read from the device's isolated Herdr over SSH; `MEASURE_KEY_INTERVAL_MS` spaces the keys (0 by default: as fast as CDP takes them, in order).

`MEASURE_SCENARIO=keys` runs both on the shape the terminal claims are made on: the measured pane and four splits in one tab, five attached panes.
It records, in order: idle resources over `MEASURE_IDLE_SECONDS` (60) in `resources-idle.json`; the idle key echo (`key-echo-idle.json`); every pane printing a line per 8 ms (a private `drive.py` beside `cat`) while resources are sampled over `MEASURE_DRIVEN_SECONDS` (120, `resources-driven.json`), with the page's `/ws` frames by type, the full redraws it asked for (`ws-driven.json`) and the core log's terminal and redraw records before and after (`driven-diagnostics-*.json`); the key echo again on the measured pane, back to a plain `cat` so its marker arrives whole, while the other four keep printing (`key-echo-driven.json`); the key count when `MEASURE_KEY_COUNT` is set (`key-count.json`); and the page navigated to `about:blank` while the panes print for `MEASURE_WINDOWLESS_SECONDS` (60, `resources-windowless.json`, browser omitted from that sampler), then navigated back, with each pane's drawn text length (`reopen-panes.json`), the page's frames and full-redraw requests around that navigation away and back (`ws-reopen.json`, `ws-count.mjs`) and the core log's terminal, redraw and resume records (`reopen-diagnostics.json`, `diag-counts.py`).
`summarize.py resources` reads hided's median one-second CPU and its largest RSS from a resources file; `summarize.py echo` reads a key echo file like an echo file.
Chrome remains alive during this detach/reconnect phase; verify private `/health` client retirement and reacquisition independently, and use the isolated desktop lifetime journey for native last-window or browser-process exit.
Use valid trial distributions with the current matched comparison; supply an explicit echo threshold only when that absolute limit belongs to the agreed claim.

A comparison runs the baseline and the candidate `hided` (release builds, each from its own worktree, `MEASURE_HIDED_BIN`) in alternating trials on the same machine, idle and driven reported separately, with nearest-rank p50 and p95 per condition and the load recorded beside each trial; the claim's thresholds are the PRD's (B2: p50 and p95 no more than 5 ms above the baseline; B21: median CPU no more than 10% above and largest RSS no more than 32 MiB above; B19: windowless median CPU no more than 10% above), never a past run's figure.
Keep `--isolated-headless` the same for both builds.
A device pane is measured the same way from a screen whose isolated hided has the device registered with consent, reaching a private sshd, an isolated Herdr and a separate HOME and state folder on the device; the operator's own device account, daemons and Herdr are never the target.
`MEASURE_SCENARIO=device` does this: it writes the private sshd's alias `MEASURE_DEVICE_ALIAS` (for `MEASURE_DEVICE_USER` at `MEASURE_DEVICE_HOST` on `MEASURE_DEVICE_PORT` with the key file `MEASURE_DEVICE_IDENTITY`) into the private HOME's `~/.ssh/config` and copies `MEASURE_DEVICE_KNOWN_HOSTS` (its host key; Hide checks `~/.ssh/known_hosts` only) beside it, starts hided with the consent folders `MEASURE_DEVICE_HELPER_ROOT` and `MEASURE_DEVICE_CLI_DIR` inside the device account, registers and fronts the device as the shell does (`device-front.mjs`, which also waits for the device's helper to be ready, since its terminals ride that link; `MEASURE_DEVICE_WAIT_HOST=0` skips that wait for a build from before the node link, whose device terminals run on their own SSH sessions), and runs the key echo (`key-echo-device.json`) and, with `MEASURE_KEY_COUNT`, the key count (`key-count-device.json`) on the device's one fixture pane; with `MEASURE_DEVICE_DRIVEN_SECONDS` it then splits four more panes on the device, each printing a line per 8 ms, and runs the key echo again on the measured pane (`key-echo-device-driven.json`), closing the splits afterwards; `device-herdr.sh` runs the device's isolated Herdr (the pinned binary in the private HOME's `~/.local/bin`) on `MEASURE_DEVICE_SOCKET` over the same alias.
Before anything dials the device, `device-guard.sh` refuses any target but the isolated account: the private sshd's port `MEASURE_DEVICE_PORT` is never 22, the recorded host keys name that host and port only, the private HOME `MEASURE_DEVICE_HOME` and the socket must be under `/tmp`, and the consent folders absolute inside that HOME, because hided spells a `~/` folder from the SFTP home, which is the account's own home even where the private sshd gives its commands another HOME (`docs/VERIFICATION.md`, A device check).
The guard never reads a caller's SSH configuration: hided's parser and OpenSSH read some spellings apart (`Port=22841`, a tab, `Include`, `Match`), so a file OpenSSH resolves to the private port can send hided to port 22; the guard writes the alias in one plain form instead, and `device-herdr.sh` passes the same parts to `ssh` with no configuration file.
`MEASURE_SCENARIO=remote-core` measures a screen whose core runs on another machine (PRD core-host-node-remote-core B5-B7, B21): it is the `keys` shape with this machine's hided in the node role, its core on the measured device.
`core-host.sh` uploads `MEASURE_CORE_BIN` (the measured hided unless set) to `MEASURE_CORE_PROGRAM` and starts it there with the state folder `MEASURE_CORE_STATE`, the node id `MEASURE_CORE_NODE`, no Tailscale, and the device's isolated Herdr, then stops only that process when the run ends; the run writes the placement record naming it, starts this machine's node with the node id `MEASURE_SCREEN_NODE` (`measure-screen`), and fronts its checkout on the core through the node's `/ws` (`core-front.mjs`).
The keys run's resources are the node-role hided's, its key echo and key count are on this machine's pane as the core names it, and a key echo (`key-echo-core.json`) and key count (`key-count-core.json`) follow on the core machine's own pane in `MEASURE_CORE_FIXTURE`.
For it the guard takes no consent folders and requires instead that the core's program and state folder are absolute paths inside `MEASURE_DEVICE_HOME` and that its node id is a plain name.
Fingerprint the device's real `~/.hide`, `~/.local/bin`, `~/Library/LaunchAgents` and Herdr configuration (hash, modification time and the processes that hold them) before and after, and record both.

## Scoped browser gateway discovery

Browser inventory area identity is projected in the existing changed-layout generation pass, one visit per area and display, with no additional notification or timer.
Each sync also compares borrowed identities for at most 256 saved Workspace layouts against the current connected checkout catalog, with no allocation when the scope is unchanged.
A scope transition advances that same generation and rebuilds the inventory; removed or disconnected checkouts retain their saved layouts but retain no native page authority.
Positive area scopes are collected in that same pass (at most 256 Workspaces times 257 areas: one per area that holds a display, and an empty front area), with no allocation or extra notification on unchanged generations.
Changed generations build one bounded map of the previous scopes to retain their incarnations; missing scopes receive the current generation on regrant, so coalesced revocations cannot preserve old capabilities.
Session and catalog mutation boundaries use that same borrowed comparison and immediately discard revoked scope incarnations; only a scope transition allocates a bounded checkout set and advances the generation, without a tree reconcile or another notification.
The daemon's gateway registration retains at most four app process identities; each discovery or action checks those bounded identities outside the Runtime mutex.
Discovery and browser actions share eight admission permits; exceeding the cap reports `browser_control_busy` rather than queuing more work.
Discovery does loopback HTTP outside the core owner thread, with no proxy or redirect, an eight-second deadline and a 16 KiB answer cap.
Browser creation reuses Workspace prepare/read/commit and its existing retry-record cap; checkout file reads remain outside the Runtime mutex.
Idle pages add no discovery work, and ordinary terminal input, snapshots and tab selection do not start gateway requests.

## Projects and Overview cost contract

The shared input surface and empty-state renderer add no timers, tasks, I/O or core state.
Row hover/focus remains local to visible controls.
Search migration retains its existing filtering and result-ID reconciliation cost; it does not add another search index or per-keystroke subprocess.
These visual transitions neither dispatch runtime events nor mark the snapshot rest payload dirty.
Choice controls publish only a changed selection; repeated activation of the selected option has no action.
Their work scales with the small visible choice set, not the retained project catalog.

Project activity and checkout-pane context reuse canonical agent projection, current topology and cached worktree HEAD metadata.
The existing single `git log -1` read now returns timestamp and subject together; no timer or additional Git subprocess is introduced.
A changed projection indexes agents and visits retained panes once, then sorts projects and each project's checkouts by cached keys.
Cost is O(agents + panes + projects log projects + sum(checkouts log checkouts)); Overview reads the existing checkout/agent aggregation and does not duplicate pane rows.
No work is scheduled by hover or by an unchanged agent-list tick.
Identical catalog snapshots settle to the same ordering and revision, so the shell receives no redundant navigator update.

UI persistence reuses the runtime worker context with one active save and one pending flag; serialization, write, fsync and rename occur outside Runtime's mutex.
Sequential writes prevent an older state from overwriting a newer one; intermediate UI saves may coalesce, while pane input never enters this queue.
A write failure publishes the existing caller-visible save error.
Dropping `Core` (`herdr-core/src/handle.rs`) stops producers and joins the last save outside the mutex before the process exits; forced process termination does not guarantee a pending save.
Standalone unit runtimes without a worker context retain synchronous persistence outside any shared runtime mutex.

A pin is one more key in the same sort and one more exclusion in the same fold pass; `workspace_pin_set` re-sorts the projected list in place and persists through the existing off-lock save, and the removal counts ride the checkout summary pass rather than a second visit of the panes.
Closing a project's panes for `Remove project…` runs on the same worker pattern as worktree deletion, outside the mutex, with one in-flight close per project.
Regression owners are `projects_follow_authoritative_activity_and_identical_snapshots_settle`, `overview_tracks_live_checkout_panes_and_drops_retired_lineage`, `pinned_projects_lead_their_device_in_activity_order`, `pinning_a_registration_reorders_the_row_and_persists_the_flag`, and the `removing_a_registration_*` and `removing_registration_*` tests.
Manual acceptance in the desktop app uses many private projects, Search and disclosure, live pane retirement/movement, pin and unpin, and registration removal with and without open panes.
Measure baseline and candidate idle/driven work separately with the same project/pane count; tests alone do not prove desktop responsiveness.

Sidebar sticky-header invalidations from scroll, resize and direct section changes schedule at most one animation frame per mounted list (`web/src/sidebarSticky.ts`).
An idle list schedules no frame; a burst adds one constant-time pending-frame check per event, followed by one list rectangle and one rectangle per section, all read before changed paint-marker writes.
This publishes no core state and starts no process, timer loop or notification fan-out.
The section scan follows the mounted list and has no independent numeric section cap; unmount removes listeners, disconnects both observers and cancels the pending frame.
Focus reveal reads current header rectangles once per focus change so the deferred paint marker cannot conceal a row; fully visible alternatives still avoid scrolling.
Regression owners are `sidebarSticky.test.ts`, `sidebarFollowsFocus.test.tsx` and the sticky-header journey in `web/e2e/projects-sidebar.spec.ts`; these establish functional scheduling and visibility, not measured latency or RSS parity.

### Project worktrees, disk and cleanup

No screen opening asks for a worktree read: every local Git project is read in the background, a repository's watch scopes the re-reads as below, and `overview_refresh` names one project to read whole again.
The previous catalog remains visible while that read runs.
An accepted changed worktree answer announces its facts and loading completion after releasing the runtime mutex, even when the following catalog rebuild leaves pane topology unchanged; unchanged or rejected answers announce nothing.
The open-dialog regression in `web/e2e/worktree-delete.spec.ts` checks lock recovery, loading completion and repository-list A-to-B-to-A consent on the existing renderer connection.
`behind_upstream` rides the same `rev-list --left-right --count @{u}...HEAD` call that already counted unpushed commits, so a fetched-side count costs no extra process, and `created_at_unix_ms` is one `stat` of the worktree's gitdir in the same background pass off the mutex.
The catalog pass is bounded by the worktree count; a project with many worktrees pays one status, one rev-list and one stat per worktree whose inputs moved per change, never per tick or per agent update.
A Git HEAD, index or ref change is scoped to its own repository: the OS watcher groups a burst into one generation change, so that project alone is re-read and every other project is answered from the worker's last read.
Within that repository the node keeps each worktree's last facts with what they were measured from (the listed HEAD, branch and lock, the worktree's index file, its base, its upstream, and the commits the base, `origin/<base>` and the upstream name, from one `for-each-ref`), so a watch read measures only the worktrees whose inputs moved and answers the rest from that memo (`hide_host::worktrees::Measured`, one per node, the last 64 repositories read).
The daemon's own node, a device's helper and a node that dialed its core also keep the memo in their state folder (`worktree-facts/`, `hide_kit::layout::worktree_facts`), one file a repository and the files of the last 64 repositories written: a file is replaced whole, off the memo's lock, when a read changed what it keeps, and read for a repository the node has not read since it started; a file of another version, or one that does not read, keeps nothing and fails no read (`worktrees.facts_unread`).
An explicit read (`overview_refresh`) measures every worktree again, the longest unmeasured first.
A project's first read since the core started, before any explicit read, may reuse, so a restarted daemon measures only the worktrees whose inputs moved; an edit made while it was down that moved no input shows at the next explicit read, as one made while it runs does.
Idle repositories do not run Git commands or sample working-tree files; content-only edits are reflected at the next explicit read.
On 2026-10-14 a 32-worktree repository took 48 to 51 s per whole read at load 15 to 24 (`hide_host::worktrees::read` timed against the live checkout), and was read whole again on every commit in any of its worktrees, past the core's 60 s wait 16 times in 89 minutes; a read that may reuse, with nothing moved, answered in 0.16 s.
The same checkout with 39 worktrees, read by a fresh memo as after a restart at load 11 to 36, ran into the 45 s read bound with 7 worktrees unmeasured with an empty folder, and answered all 39 in 0.24 s with the file a previous read left, nothing having moved since (24 s when that file lacked 7 worktrees and live agents had moved others).
`a_commit_in_one_project_does_not_rerun_status_in_another`, `a_commit_in_one_worktree_measures_that_worktree_alone_until_a_refresh` and `idle_and_working_tree_edits_do_not_reread_but_manual_refresh_does` own these boundaries, and `a_restarted_node_reuses_from_its_folder_only_what_did_not_move`, `a_file_of_another_version_or_that_does_not_read_keeps_nothing`, `the_folder_keeps_the_files_of_the_last_repositories_written` and `a_first_read_since_start_reuses_and_one_after_an_explicit_ask_does_not` own the restart.
A finished worktree removal follows the same scope: its row leaves the catalog under the lock with no Git call, the coordinator rebuilds the rows on its next wake, and the reader re-reads only the removed worktree's repository; `a_finished_removal_drops_its_row_at_once_and_an_older_read_cannot_bring_it_back` owns this.
The linked-worktree facts pass also measures ignored repository boundaries off the runtime mutex once for each linked worktree it measures.
Each scan has hard caps of 2,000,000 steps, 30 seconds and 1,024 names, and its time is also whatever is left of its worktree's 30 s.
The scan verifies each repository it finds with one `git rev-parse`, so it is the read's largest cost: a worktree holding 550 fixture repositories in ignored run folders took 15 s, against 0.3 to 1.4 s for one holding none.
One repository read on the node has one absolute 45 s deadline (`WORKTREES_READ_TIME`) starting at public `read`, including main-worktree discovery, preliminary Git metadata and the subsequent measurements; `read_project`, entered with a known root, starts the same budget at its own boundary.
Every preliminary Git command uses only the remaining read time, capped at its existing 15 s, and expired reads start no further command; standalone Git and registration helpers retain their own 15 s limit.
This leaves owned-child and pipe cleanup inside the core's 60 s wait (`WORKTREES_TIMEOUT`), and one worktree's measurement uses at most 30 s (`WORKTREE_MEASURE_TIME`), so a slow worktree leaves the others time.
`a_public_read_spends_one_deadline_on_discovery_metadata_and_measurement` calls the real public boundary with delayed Git answers in an isolated process on every system; it owns the overall bound and caller-visible unavailable result.
A worktree its own 30 s cut short keeps that answer, named as not fully measured, until its inputs move or an explicit read; one the read's 45 s cut short, or never reached, answers its last facts if nothing moved, is named unmeasured otherwise, and is measured first by the next read.
`a_worktree_that_does_not_answer_is_named_and_the_next_is_still_measured` and `a_read_out_of_time_keeps_what_did_not_move_and_names_what_did` own these bounds.
It skips Git metadata and directory links; a failed or capped scan publishes an unavailable fact rather than an empty list.
A confirmed deletion uses the existing single removal slot and one preflight worker before any pane close; the same host check runs again before guarded removal.
No scan is added to pane input, a snapshot tick, hover or an unchanged catalog read, and only phase transitions notify the shell.
Every `git` the catalog runs is bounded by `GIT_DEADLINE` (15 s), and by what is left of its worktree's time, and drained off-thread past the pipe buffer; a repository that outruns it reports its status unavailable and a `git.deadline_exceeded` diagnostic rather than holding the other projects' answer, which a status over evicted iCloud files once did for minutes.
Group ordering, chips and search are pure functions of the accepted snapshot; agent status updates redraw rows and never recompute the catalog.
List rows use the existing lazy-loading and search keyboard patterns.

Disk reuses `DiskReader` (`herdr-core/src/disk.rs`), triggered by the right panel's request, a cleanup sheet's project and the Overview's Projects tab (`disk_watch`, `herdr-core/src/runtime/disk_sweep.rs`), with one inflight read and coalesced pending input; the walk itself runs on the checkouts' node (`hide-host/src/disk.rs`, the `disk_usage` call), which reports each checkout as it finishes.
The filesystem walk counts `st_blocks * 512`, partitions nested checkout/shared-Git roots by longest ownership, and deduplicates `(device, inode)` across components.
It counts a symlink's own allocation without following it and rejects alias roots rather than escaping the declared boundary.
Each checkout root has its own bound of thirty seconds and one million visited/pending entries; a root that runs out or cannot be read has no total and remains visible beside a confirmed subtotal, and never takes another root's answer with it.
A whole read has its own cap too, ten million visited entries and five minutes, and only files with more than one link enter the `(device, inode)` set (capped at one million); the roots the read did not reach before that cap stay unavailable, each with a `disk.measure_failed` diagnostic carrying its checkout and reason code.
A project with some unmeasured checkouts shows the layer subtotals of the measured ones only, beside `confirmed_bytes`, and no total.
Each finished root is handed to the coordinator as soon as it is measured, laid over the previous answer, so a large project fills row by row; the whole answer replaces the partial ones when the read ends.
The same walk sorts a checkout's blocks by layer (`hide-host/src/disk_layers.rs`): the ignore rules (`hide_host::index::IgnoreRules`, the rules the file index uses, global excludes file included) name the candidate folders, and a signed `CACHEDIR.TAG` or a known ecosystem's marker file in the same parent vouches for a build cache or dependency folder; a candidate holding another repository, a link, or no proof is `other`.
Nothing here runs a `git` process or reads a file beyond a directory listing, the `.gitignore` of each source folder and one tag per candidate, and the free space of the volume comes from one `statvfs` per read.
This is allocated disk accounting, not physical reclaim estimation for APFS clones.
A new measurement replaces the last one; no timer, hover or per-row subprocess measures disk.
The Projects tab's sweep measures the projects it shows one at a time in its order, keeps each answer by path while the daemon runs and treats it as fresh for ten minutes, so reopening the tab within that time measures nothing; a hidden tab sends `disk_watch []`, which ends the sweep after the measurement in flight.
`the_tab_measures_its_projects_one_at_a_time_in_its_order`, `a_failed_measurement_keeps_the_last_size_and_the_sweep_goes_on` and `the_cleanup_sheet_s_project_is_measured_first_even_with_the_tab_hidden` own the sweep.

Cleanup uses the existing action worker context with one active review/removal, never the Runtime mutex, for Git, disk and fresh schema-decoded Herdr snapshots.
Cleanup protects both launch `cwd` and current `foreground_cwd` from the generated snapshot contract; it does not change navigation projection policy.
Review reads are non-mutating; confirmation rechecks each target immediately before `git worktree remove` without force.
Git and Herdr have no shared atomic filesystem transaction: a state change after the last Herdr check cannot be reserved against by this contract.
The UI therefore describes a fresh eligibility check rather than a permanent unused guarantee; Git independently refuses dirty or locked removal.
A stale, missing or failed check is caller-visible and never becomes permission to delete.
Completed intents are retained until dismissal; duplicate confirmation does no work, and retry through a fresh review excludes already removed targets.

The disk cleanup names a project (`cleanup_review {workspace_id}`) and shares that one lane: one active review or run per daemon, published on the reviewed project's snapshot, never under the Runtime mutex.
What is in use is read on the worker when the review opens and again when it is confirmed: the checkout agent summary the lock already holds, one `pane.process_info` per non-agent terminal pane of the project (capped at 256 panes; past it the read fails rather than guessing), and one `lsof` listener sample (each `lsof` stopped at ten seconds, which fails the read closed); a read that fails leaves the review with a `usage_error` and nothing selectable.
Every move is judged from a fresh read: the checkout's facts are copied again under the lock and its panes and ports are read again for each worktree and for each checkout's cells, after the folders were judged and right before they move, and a registered checkout the runtime has no facts about reads `unverified` and loses nothing.
The tracked-file check lists the whole index once and compares by case- and Unicode-normalized spelling, so no argument list can overflow and a folder the file system spells differently from the index is still found; the repository look shares one 20 million entry, 300 second budget across the run.
While a worker runs the daemon takes no dismissal, no second review and no confirmation, and the per-move reads copy only the checkouts' in-use facts under the lock, and a worker that unwinds publishes a terminal `failed` or `complete` snapshot and frees the lane.
The wire carries codes and sizes only: `result` is `removed`, `skipped` or `failed` with a `result_code` and `bytes`, and the English reasons stay in the diagnostic log.
The run waits only for the trash entries it created.
The in-use answer is published before the slower worktree eligibility checks (Git per worktree, GitHub only for a branch not already an ancestor of main), so build caches can be chosen while those run.
A confirmation moves each chosen folder into `<git-common-dir>/hide-removed` with one rename after judging it again from the files (still ignored, still vouched for, no link on its path, no repository inside, no tracked file from one `git ls-files -z` per checkout), then the existing trash sweep deletes and the run waits for the trash to empty before it reads the volume's free space; the core decides which worktree and folder may go and the checkouts' node does every read and move (`hide-host/src/cleanup.rs`: one `judge_folders` call per checkout that spends the run's shared nested-repository allowance, which the core carries from one checkout to the next, one `set_aside_folder` call per folder, one clean `worktree_remove_clean` per worktree and one `drain_trash` call); the confirmation is accepted once because it moves the phase from `review` to `removing` under the lock.
Regression owners are the `live::cleanup` tests for in-use reads, exclusion codes, cell rechecks (`a_folder_that_changed_after_the_measurement_is_kept_and_named`), repeat convergence and the single `ls-files`, and `a_cleanup_confirmation_only_selects_what_the_review_allows_and_runs_once`.

Regression owners include `overview_open_section_focuses_the_checkout_and_switches_the_panel_in_one_event`, `agent_start_in_checkout_reports_through_the_task_operation_slot`, `behind_upstream_is_absent_without_an_upstream_and_counts_the_fetched_side`, `linked_worktrees_carry_their_creation_time_and_the_main_worktree_none`, disk filesystem fixtures and cleanup filesystem fixtures.
Manual acceptance in the desktop app additionally covers the Projects tab's rows and disk cells, narrow Korean/English wrapping of project and branch names, `…`/`?` cells, and cleanup review/cancel/exclusion/success/stale refusal in private fixtures only.

## Project Memory cost contract

Opening Sessions starts one bounded background catalog read for the focused Project through its own node; filtering and searching operate on the retained snapshot and start no subprocess.
One Memory pass at a time runs on a worker, every 60 s or at once when a live session's pane closes, and walks every Memory-enabled Project on the core and each linked device; a session is analyzed only after its file has been unchanged for 30 minutes or its pane closed (PRD project-memory-v2 D-07).
A pass reads each session's size and time once (one local stat through the node; for an OpenCode session one read-only database read of its proof, message count and newest write, about 0.5 ms each measured over 100 sessions of 100 messages in a debug build), at most one bounded receipt read and, when analysis is due, one bounded conversation read per session; an unchanged session reads no transcript byte, writes no SQLite row and makes no provider call, and a pass that changed nothing publishes nothing.
An OpenCode read is bounded like a JSONL increment: at most 200 messages, 512 parts and 1 MiB of row data, under OpenCode's own SQL work cap.
Each listing of a home's Claude and Codex transcripts keeps the header directory and the listed facts of every transcript whose stamp is unchanged, so an unchanged transcript is not opened or parsed again; an appended transcript is read on from the checkpoint its last read left (the `ConversationCursor`'s offset, file identity and oversized-record scan, never transcript bytes), one 1 MiB window at a time up to the size its stamp saw, so its cost follows what was appended and not its size; a transcript that got shorter or was replaced, or whose read failed, is read from its start.
What a listing keeps per transcript is that checkpoint and the facts it shows (the first request cut to 160 characters, two times, two counts, the titles), for at most the listing's 10,000 files; the map's lock is held for a lookup or an insert, never across a read.
Read through the cursor, a cold listing also counts a Codex compaction record longer than the 256 KiB line cap as the context reset a shorter one already is, and holds a Claude command record at the end of the file until its reply is written, as the label reader does.
On 2026-10-28 a debug listing of the herdr-ide Project from a private home of clones (395 sessions of 1,740 transcripts) cost 1.42-1.46 s of CPU with 13 transcripts (114 MiB) appended 2 KiB each, against 0.045-0.067 s read on from their checkpoints; a cold listing cost 32-33 s before and 42 s after (the oversized-record scan and the per-record parse the cursor does), once per transcript per process.
A pass leaves a Project's per-session walk out while nothing it decides from moved (the invariant above), so an idle pass reads no session stat and no cursor row.
On the same home with a copy of the operator's store (three enabled Projects, 428 sessions, Hide AI off), once the receipt cursors of the sessions that were live when the store was copied had caught up (about 40 passes of 1 MiB reads, in both builds), a debug idle pass cost 0.25-0.29 s before and 0.18-0.20 s after, of which the three listings are 0.12-0.14 s.
A pass after 13 transcripts were appended cost 1.52-1.55 s before and 0.25-0.29 s after.
Idle, a pass over an enabled Project with N sessions costs N stats, N cursor reads and no write; driven, it adds one bounded read per session written since its cursor and, past the quiet rule, one provider call per 64 KiB group.
Memory call admission uses one immediate SQLite transaction on the existing worker, reading the day's two counters and, for analysis only, due re-verification and promotion checks up to the 500-call reservation bound.
That query can scan more existing rows than the matches it returns; its cost depends on retained cards and Projects, and it adds no transcript scan, subprocess, timer or work to terminal input or snapshots.
Checking start, completion and an actionable deferral publish only a changed core state; each of the existing four re-verifications and three promotion checks per Project can request one coalesced open-Memory refresh as its answer lands.
Cancellation and worker completion clear the active card, while budget holds clear when work resumes or no check is due, and provider holds also clear when settings change.
The daily cap counts a call when it is admitted, so a failing provider spends it too; that is the runaway bound, and the provider's backoff keeps failures to a few dozen a day.
An intent-local session failure keeps its unfinished cursor but rotates its durable queue turn; the existing Project failure record and backoff throttle retry without keeping other Projects behind that session.
Account/provider readiness, usage and resource-cap pauses preserve the durable next position; intent-local failures yield to peers.
Hide AI off still permits receipt and evidence reads, but creates no analysis queue entry and requests no usage refresh for analysis.
A reader-rejected native record is diagnosed and skipped while valid later records and the source checkpoint continue.
Normalized event text is capped at 256 KiB before chunk allocation; an event past it records its Project/session/offset, measured bytes and limit without content, and is skipped rather than retried forever.
Accepted events preserve their complete text through 36 KiB groups and the four-group session quantum.
At the active-card cap, only new growth is rejected: accepted merges and corrections commit with the batch, its cursor advances, and `analysis.session_cards` reports the rejection count.

With no window attached, the usage reader runs on its own 5-minute schedule for 30 minutes after a pass last had a session due, so the usage limit has a figure headless (the usage reader also runs headless while Hide AI is paused by the limit or an agent worked in the last 30 minutes, at the same one read per provider every 5 minutes); that is at most one usage read per provider every 5 minutes, and none while no session is due.
Session file discovery, incremental reads, SQLite work, redaction, provider requests, hook-config writes, and JSON serialization stay outside `Mutex<Runtime>`.
One incremental transcript read takes at most 1 MiB, one retained JSONL line is capped at 256 KiB, and catalog or archive detail parsing rejects a complete session above 64 MiB.
One Project/session/content-hash intent is idempotent, and background input is capped at 64 KiB a request, one pass in flight, 1,000 analysis calls a machine a UTC day, 10,000 active cards and 20,000 receipt rows per Project; an agent past the operator's usage limit (90% of its week by default) is skipped and, with every listed agent past it, background analysis waits; the limit adds one mutex write when a figure or the limit changes and snapshot fields that move only then.
Receipts a live pane's label read hands up wait at most 256 and are recorded on one owned thread; closed sessions waiting for analysis are capped at 256.
Repository comparison reuses the analysis worker and its bounded document reader; creation adds at most one comparison call per extraction request and at most 15% additional input, while that same answer classifies new-card conflicts and existing cards reuse their check calls.
A conflict adds one bounded node Git date read and one bounded source conversation read, with no subprocess or store work under the runtime lock and no work on terminal input.
Comparison excerpts reach snapshots only for the expanded card; rows carry the path, dates and authority metadata.
A batch reservation covers at most five cards and starts one agent; the existing tracker handles their conditional transitions, with the same coalesced Memory refresh and cleanup rules.
`analysis.session_cards` records session counts and extraction/document input characters; compare fixture and operator measurements separately rather than treating scripted-provider counts as live usage.
Connection direction adds no worker, timer or store: Memory admission reads the live connection's bounded reader facts under the lock (at most the supported agent set), without a node call.
Each pass visits the registered devices once; a missing link or Memory reader leaves its closed sessions queued, and an unsupported reader records a diagnostic only when its reason changes.
The last refusal is retained at most once per registered device; recovery or removal drops its entry on the next pass, with no additional node call or UI notification.
All transcript and Project reads still run on the existing worker through the device's own node, and the existing per-core call and queue caps cover both directions.
The Memory snapshot section adds no work until a Project's Memory is opened or a live session was given a card: opening, expanding a card or searching is one read on a worker, coalesced like the Sessions read; full card detail, files and sources are sent only for the one card shown in full.
The lane's rail is always drawn and its Memory icon's count reads the front Project, so the rail opens that Project's Memory once per Workspace visit (and again when its window comes forward after another window named another Project), whether or not the Memory tool is shown: one coalesced worker read per visit, with no timer, subprocess or terminal-input work.
Active promotion rows additionally send a preview capped at 512 Unicode characters per card (at most 2,048 UTF-8 bytes), cut from the cards that worker already reads, under the existing 10,000-card Project cap.
Grouping and agent association are render-time lookups over retained rows and agent PR facts, with no new store read, subprocess, pending queue, terminal-input work or notification fan-out.
Running promotion elapsed labels reuse the window's existing shared clock; only their text changes with time, and the last subscriber releases that clock.
What each live pane's session was given is read again, one indexed query per live session, after a receipt batch, a pane change or a Memory change, one read in flight and one waiting.
Crossing a cap is logged, never an enlarged queue; the operator sees only a state they can act on.

The former copied Project-wide Conversation history remains removed: its raw-body index and 30-second refresh are not restored.
Overview History searches retained canonical requests on demand through the existing session-work worker, with the bounds described below.
The old `session-search.sqlite3` is opened read-only only to read each project's retention choice, and a missing file or table costs nothing beyond that open.

`UserPromptSubmit` is a separate high-frequency boundary.
The hook accepts at most 256 KiB of input and asks the core with one `hide workspace memory` beside its letter pull, carrying the prompt's first 8 KiB; the core uses the attested checkout's prepared Project and at most 1024 bytes of confirmed task context, opens its store read-only, checks schema and the writer's indexed projection certificate, and requests at most sixty local FTS candidates within the earlier of the caller expiry and the 100 ms retrieval deadline.
The sixty candidates carry two more columns, the operator's distinct sessions and last time behind each card's current text (`memory_sources` read through its primary key, which starts with the card id, joined to each source's revision through `memory_revisions`' unique `(item_id,revision)` and grouped by the words' digest, so each is a lookup of the card's few source rows; the operator's busiest card has 13), and are ordered in memory by score times the root of that weight, with no stored count and no write; schema 21's digest column is written by the analysis, never by a hook.
The body selector keeps at most three whole items and 600 estimated body tokens; the final escaped envelope, including wrappers and receipts, is trimmed to a shared 1200 estimated rendered-token limit, off the runtime lock.
These are local estimates, not provider billing tokens.
The core sets a one-second Memory answer deadline; `CoreHandle::memory_scope` bounds its owner reply with the remaining expiry, and remote admission, write and answer share one absolute deadline.
SQLite's progress handler observes the invocation expiry, and retrieval also enforces its local 100 ms allowance.
Memory and letter reads run beside each other: Memory waits until 1,250 ms after the inner hook starts, and the 1,650 ms intake budget reserves its last 400 ms for confirmation after a successful stdout flush.
The prompt supervisor bounds the inner helper at 1,850 ms from the outer helper's start; 1,250 ms is a Memory read budget, not the whole hook duration.
The core's lookup starts no model, embedding, transcript scan, child process, network request, repository discovery, full-store scan, or database write.
Missing, locked, corrupt, stale, unresolved, unreachable or over-deadline inputs exit successfully with empty context so prompt submission continues.
`SessionStart` reads the Project's index under the same read-only and fail-open ownership and the same Memory read budget: at most 20 one-line rule, decision, lesson and preference cards and no body.
The index reads every eligible card of the Project, as its salience order already did, now with the same two source lookups per card, and keeps the 20 with the highest salience times weight; the 10,000-card cap bounds it.
On 2026-10-28, against a copy of the operator's store (462 eligible cards, 1,208 sources) in a release build with a warm page cache, the index read took 0.6 ms at the median before and 1.6 ms after, and the prompt match 4.4 ms and 4.6 ms, out of 30 rounds of eight real prompts; linear in cards, the index at the 10,000-card cap is about 35 ms, inside the 100 ms retrieval deadline.
Reading the two columns for the current text only, grouped by the words' digest, cost 2.5 ms of CPU for all 494 active cards of that copy in the `sqlite3` shell, against 2.0 ms for the ungrouped count.
Its separate Workspace probe has a two-second CLI timeout and runs beside Memory when its thread starts; later count-report calls also have their own two-second timeouts.
The installed SessionStart entry has an eight-second runtime timeout, so the Memory budget is not a whole-session-hook deadline.
`hide memory show|search|forget|undo|outdated|placed|building|unbuilt|skipped` is one store read or write on hided's blocking task; `add` is one provider call bounded at 50 s.

Regression owners are the `hide-project` identity tests, `hide-session` provider-neutral catalog and cursor tests, `hide-memory` Project-isolation, lifecycle, convergence, FTS transaction, redaction, ranking and budget tests, `hide-agent-hooks` fixture/config/fail-open tests, and core atomic-event and editor-preview tests.
Manual acceptance in the desktop app uses one exact worktree-local `hided` PID and window against a private Herdr server and private app state.
Record hook idle and driven timing separately, including sample count and failures, and record provider request count, queue/inflight bounds, child descendants, and RSS separately from the prompt path.
An automated deadline test proves bounded return under its fixture conditions; it does not prove every storage device, link or interaction answers inside the wait.
Report the hook's Memory time separately for a pane on the core's machine and one on a node, and the prompt hook's worst case beside it.

### Overview Work, backlog, History and Usage reads

The Overview's tasks are the core's (`agent_state/tally/tasks.rs`), rebuilt only with the agent scope cache's compared input, so an unchanged frame derives no task and adds nothing to the high-frequency path.
The shell sorts and filters them into sections in one memoized pass (`taskSections` in `web/src/overviewTasks.ts`) over the projects, rows and filter it reads, and an unchanged input does not regroup cards.
The core issue projection uses accepted catalog and metadata only; it schedules GitHub work when a screen names a project (`github_request`) or on an explicit refresh; the one timer it adds is the re-read of every local Git project in the ledger above, five minutes after a good answer, 30 seconds doubling to five minutes after a failed one (a read that missed its pull requests or its issues), and never on the clock for a repository answered "no GitHub remote" whose remotes the catalog knows.
A window gaining focus or a project brought to the front sends one `github_request` with `look`, which adds no timer: the core compares the clock of that one project and asks the existing reader again only when the last good answer is 30 seconds old and nothing is outstanding, so repeated focus changes cost one comparison under the lock and at most one read per project per 30 seconds.
The advisory poll (ledger above) adds one worker pass a minute while a window is attached: at most eight projects, a handful of conditional `gh api` calls each, one `gh` at a time, and nothing published by the poll itself, so an unchanged minute creates no frame and no read.
A 304 does not count against the rate limit (measured on `gh` 2.76: twenty 304s moved the core `used` counter by 0, a 200 moves it by 1), so the idle spend is zero requests an hour and a busy repository spends one full answer per change it sees.
Its rate-limit floor is a tenth of the account's limit (500 of 5,000): the poll's own spend is a few full answers an hour, so the floor protects the operator's other tools and not the poll.
The GitHub reader keeps its existing single worker and per-project generation cache.
A project's pull request read is two concurrent `gh pr list` calls under the same 15-second limit, so it adds one short-lived thread per read and no timer; a read asks for the checks of the open pull requests only, and the cache keeps at most one check result per listed pull request.
Each read's `pull_requests.ok`, `pull_requests.empty` or `pull_requests.failed` diagnostic carries `duration_ms`, which is where a slow read is found afterwards.
Tying a pull request to a checkout compares the checkout's already-held HEAD with at most 200 listed pull requests under the lock, with no Git process or file read.
At most 200 linked identities and backlog entries are retained per project; one extra list result reports overflow.
Closed and cross-repository identities are resolved in one bounded query.
Manual writes use the existing task-operation slot and a terminating worker; cleanup shares the bounded purpose mirror queue.
Manual acceptance in the desktop app includes the device switch, overlay dismissal, task sections and their folds, the record beside the tabs, issue linking, stale facts, narrow widths and mixed Korean/English labels.
`runtime::tests::issues` owns projection memoization, stage priority, deduplication, issue precedence and transition-only refresh regressions.
`runtime::tests::agent_scopes` owns the task, stage and state rules, and `runtime::tests::backlog` owns one add per intent, one create per device with eight waiting, and a retry that finds the issue an unconfirmed create made.

Opening the Overview adds one `github_request` per local Git project it shows, which costs nothing when the core already holds that project's answer, and the Projects tab's `disk_watch`; hover, focus, filter changes and popovers are local component state that publishes nothing.
The backlog is a pure function of the snapshot (`backlogOf` and `filterRows` in `web/src/overviewBacklog.ts`), one pass over the project's items, and its filter is a pass over its rows.
Source-qualified Local and GitHub rows share that pass, with status, source, author, assignee, labels and query applied before each project's initial fifty visible rows.
Wide issue or record detail retains its list mounted and inert, so Back restores local list state without another catalog read.
Issue comments use the existing detail worker, fifty per requested page and two hundred retained per item, with four thousand characters per comment; pagination is explicit and adds no timer.

Work derives its graph from canonical agent, lineage, verified launch and bounded delivery facts, never from terminal output or letter bodies.
The graph delivery projection keeps at most 2,000 metadata edges, independently of the existing 5,000-item delivery ledger, and publishes only changed facts.
Work places its operator-centred force map synchronously on the main thread in `overviewMapLayout.ts` (d3-force), over at most four hundred nodes, only when the structure key changes: a fixed 120 ticks of Barnes-Hut charge, collision and ties, so the cost is O(ticks × n log n) and never loops.
Measured on an Apple Silicon Mac in the unit-test runtime, 75 nodes took about 15 ms, 225 about 67 ms and 399 about 140 ms; placing one new session beside 398 held ones took about 130 ms, because held sessions still take part in each tick.
A drag runs the same simulation on d3's animation timer only while a circle is held or settling, about two seconds after release, and the view stops it when it unmounts or replaces the map.
On a 2026 MacBook in vitest, the slowest shape measured was one root with 398 children at about 43 ms, a 255-session binary team about 5 ms, and 399 roots or 200 one-session projects under 2 ms.
Each admitted project reserves an access position before the remaining room is shared in identity order, so a hidden member can be selected without another layout request.
Protected current members reserve additional positions before admission, independently of the one selected-member position.
Lines carry no labels; each is a straight segment between two orb centres computed during redraw, and team outlines are convex hulls of at most one team's orbs.
Drag updates one local offset map and redraws the bounded nodes and edges; endpoint lookup uses a node-ID index rather than scanning the node array for each edge.
Selection may reproject the existing bounded relationship metadata but starts no daemon request, provider read, inference, polling clock or pending-layout queue.
The layout key contains representative identity, drawn parents, access slots and interface scale, excluding labels, marks, filters, hiding, hover, selection and correspondence.
The map has no replay or decorative animation, so idle has neither a loop nor a clock.

`overviewGraphModel.ts` creates a stable structure key so status, model and elapsed-text updates do not schedule another layout.
`overviewMapLayout.ts` refuses more than four hundred layout nodes and starts no worker.
A failed layout leaves the fifty-row list usable with Retry; hover, zoom, folding, hiding and selection add no core event or reader.
Unmount releases the theme and interface-scale observers rather than leaving background work running.

History queries the retained request ledger through the existing session-work worker and its sixteen-RPC admission and 512-command queue, without a new resident worker, native session scan or polling interval.
One cancellable 180 ms debounce belongs to the mounted view; a newer query replaces its demand and request-id and connection-generation checks reject stale answers.
The literal query is bounded to two hundred characters, and device, immutable original project, agent, status and time criteria run in SQL before `LIMIT 51`.
One page retains fifty sessions with at most two snippets each and one continuation cursor, ordered by activity and stable session identity.
The UI retains one result page and at most two hundred cursor entries; it does not accumulate every page's results.
Search uses the existing 64 MiB ledger, retention choice, five-hundred-character saved request and two-thousand-request per-session caps, and original requests saved while summaries were Off remain searchable without reclassification.
Generated titles and task summaries are opt-in and do not restore the retired raw-transcript search index.
Request navigation carries the exact saved offset, centers that one retained request and applies literal highlighting locally; it schedules no native-file search.

Usage's daily/model comparison uses the existing historical-usage worker and unchanged discovery and publication cadence in the ownership ledger above.
The existing pricing pass adds at most O(rows log 4096) work for a shared date/agent/model/source map, retaining the newest 4,096 model keys and reporting truncation while complete daily sums remain independent.
No per-call model list is copied into every agent range; one prepared bounded model list is installed with the existing consumption publication, using the same short Runtime lock and outside-lock serialization.
The mounted view filters and sums those prepared amounts for seven, thirty, ninety or retained-year dates, selected agent and day; it does not reprice, read native sessions or increase polling.
The daily and model tables mount at most thirty top-level records per page, while the chart mounts only the chosen bounded calendar range.
The existing minute countdown remains the only Usage UI clock, and leaving Usage clears its existing open hint and timer.
These bounds and functional regressions establish ownership and capped work, not native responsiveness or RSS acceptance.
Measure matched baseline and candidate idle and driven workloads separately before making a performance claim.
An item's panel reads where it came from once when it opens (`backlog_origin`, one slot per item, at most 32), and the Start dialog reads its issue once (`issue_detail_request`, `gh issue view` on a worker off `Mutex<Runtime>`); a later read of the same item replaces its slot so only the newest answer lands.
The ⌘K GitHub search is one `github_search` event per query the operator commits, never per keystroke and never on a timer; its `gh search prs` and `gh search issues` calls (two calls however many projects, at most 20 `--repo`, 15 s each, after one `gh repo view` per project the reader has not read) run one after another on a single worker off `Mutex<Runtime>`, one search at a time with a newer request replacing the one waiting, and it adds only the `issue_work.search` slot (at most 20 pull requests and 20 issues) to the revisioned `rest` section; `runtime::tests::issues` owns the lifecycle and `github::tests` the allowlist and parser.
The sidebar's 태스크 list (PRD sidebar-task-lens B22) is `taskList` in `web/src/taskList.ts`, derived from the device's `agent_scope.tasks` and memoized on that value, the agent rows, projects, archived rows and the view option.
Its resting subset adds O(r log r) sorting by the last prompt timestamp to the existing grouping work, where r is the number of resting rows; this runs inside the existing `TaskLens` memo, adds no I/O, timer, notification or pending-work queue, and leaves open tasks in the core's order.
The core rebuilds the tasks only with the scope cache's compared input (`agent_state/tally/tasks.rs`), the list draws no rows while the 프로젝트 list or a hidden sidebar shows, its elapsed clock updates only its own text, and the 보기 slot adds one read per item the operator opens, the card's or the record's, on the event that opens it.
The sidebar's PR list (PRD work-tab-pr-lens B44, B53, sidebar-task-lens D-17, lane-two-columns B41) is `prListModel` in `web/src/prWork.ts`, one pass over the core's board of each project in scope, memoized on the snapshot; the core computes the groups and reasons in `agent_state/work/board.rs` from the `gh pr list` answer it already holds and the agents' state, with no process, file or network read under the runtime lock, and sends only the pull requests the lens shows, so the snapshot grows with that bounded list.
The lens adds three reads to the five-minute generation (the login, the review-requested search and one `HidePrFacts` query) and nothing per row; a card adds one GraphQL read when it opens or its head moves, at most eight cards are kept, and a card's summary is one background-router call per head, off the lock.
Its other reads are a pull request's body and feedback when 맡기기 or 새 이슈 만들기 opens (one `pr_feedback_read`, `gh pr view` on a worker), and its only writes follow a confirmation; hover, focus, unfolding and the half-second cards are screen state.


### Pane movement cost contract

Header dragging and menu navigation remain page-local until a committed move or an unattached-tab hover read.
Pointer updates render only when the candidate destination changes; one 450 ms hover timer requests text, and Escape or a changed scope cancels that preview.
The core admits at most sixteen active moves or unsettled publication barriers, reserves both participating tabs and shares the ordinary 64-tab admission count for a new destination.
It retains at most 128 move receipts in the existing Workspace view journal, saves before mutation off Runtime, and uses finite workers with a weak runtime owner rather than adding a resident timer or subprocess.
Each native operation has a five-second API deadline and journal acknowledgement a fifteen-second deadline; a recovery check reads actual topology and never resends the original mutation.
The final publication barrier overlays only the participating tabs until a newly started coordinator bootstrap can replace it, adding one authoritative refresh per settled move and no per-key work.
Hover reads have one active worker and one latest pending request, at most 64 panes, 4,096 characters per pane, a 64 KiB API response cap, a two-second per-read deadline and a five-second batch deadline.
Cancellation is checked between reads; an already admitted read can finish, and a stale answer cannot replace the current preview.
The page indexes terminal identities once per changed catalog and readdresses an existing xterm instance without recreating its input, scroll or selection state.
Unchanged input callbacks add no catalog scan, subprocess or notifier fan-out, and terminal retention still follows the existing attached-tab limit.
Regression owners are `runtime::tests::pane_move`, delivery continuity tests, `web/e2e/pane-move.spec.ts` and `desktop/e2e/pane-move.spec.ts`; controlled measurement remains separate from these functional claims.

### Terminal path links

`web/src/terminalLinks.ts` reads at most three rows on either side of the hovered row and retains the existing maximum of sixteen joined spellings per token.
Each joined spelling is read once, in time linear in its length: one pass finds its edges and precomputes bracket depth and the next quote for every offset, so each start's readable spellings are found without rereading.
From at most 64 starts it reads at most 64 spellings for a path, and adds the names between its four outermost starts and ends.
A token offers at most 96 spellings across its joins, every original among them, so it has at most 192 logical cwd/root lookups; past the bound the shortest spellings go, which loses a link and never makes a wrong one.
Character classes come from a 128-entry ASCII table and a per-character cache of at most 4,096 entries for the rest.
Edges come from Unicode classes, not a list of marks, and a run of Hangul, kana or Han is cut only where its script meets the link, never character by character.
The selected range maps UTF-16 slice boundaries onto the buffer's glyph cells, retaining both halves of a wide glyph and refusing a cut inside a combining cell.

`web/src/terminalLinkProvider.ts` spends one invocation's budget on originals before inferred spellings.
Deduplication and fresh cache hits precede admission: at most 512 new unique logical paths reach the desktop host, in batches of at most 64.
A budget overflow records `path_budget_exceeded` with unique lookup, cache-miss, limit and skipped counts, without output text or paths.
A failed or skipped higher-precedence candidate leaves that group unresolved instead of selecting an unproven shorter spelling.
The native host distinguishes confirmed `ENOENT`/`ENOTDIR` absence from other filesystem failures, which reject the batch and reach the existing count/reason diagnostic without paths.
Answers retain the existing ten-second TTL and 512-entry cache cap.
An invocation holds its own bounded answer set while the shared cache is pruned, so a cache eviction during resolution cannot change that invocation's result.
The terminal's right-click target lookup reuses this parser, invocation budget and cache over the same bounded rows; it runs once when a menu opens, never per input or render frame.
An xterm hover retains one resolved target and cell range for that terminal, so an OSC 8 label and a plain-text link share the click and menu routes.
An HTML menu checks checkout-root ownership through the same path cache to offer Open source only inside a registered checkout; choosing the action rechecks ownership before dispatch.
The menu holds one target and discards a late result when that menu was dismissed or replaced.
There is no new input, snapshot, notification or remote-filesystem work; one accepted menu result updates that menu, and a chosen HTML open sends one core event through the existing Browser View boundary.
Native path authority and the operating-system modifier route are unchanged.

Regression owners are `web/src/terminalLinks.test.ts` (finite grammar, per-token bounds and cell ranges), `web/src/terminalLinkProvider.test.ts` (original precedence, missing/failed/over-budget checks, batching, cache and device boundaries), and `desktop/e2e/terminal-links.spec.ts` (real file opening and native path/link interaction).
For a matched dense-line comparison, record grid columns, token count and fixture spellings, then measure cold and warm hover-to-pointer latency separately with the same window and input sequence.
Count logical candidates, cache misses and IPC batches separately from actual native `realpath` and `stat` invocations; missing paths perform no successful stat, and cache hits perform neither.
With the existing `probe=1` QA seam, the provider records count-only `path_probe` diagnostics in the bounded diagnostic log.
Native IPC unique paths count admitted cache misses, not all logical candidates; a warm hover can have logical candidates while sending no host requests.
The original baseline has no count diagnostic, so record that limitation and cross-check its logical candidates with its original parser on the identical fixture cells.
Capture the exact candidate PID/window without activating it, and distinguish a renderer pointer/file-opening observation from an operating-system handler replaced by a recorder.

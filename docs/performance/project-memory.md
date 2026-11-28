# Project Memory, Overview reads and terminal path costs

- Code: `hide-host/src/memory.rs`, `hide-session/`, `web/src/overviewTasks.ts`, `web/src/terminalLinks.ts`, `web/e2e/pane-move.spec.ts`
- Index: [PERFORMANCE_TESTING.md](../PERFORMANCE_TESTING.md)

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
One session is admitted at most 72 requests a UTC day and every session together at most 80 requests in any hour ([ARCHITECTURE.md: Project sessions and Memory](../ARCHITECTURE.md#project-sessions-and-memory)); admission adds one row and two counts through `analysis_admissions`' `(project_id,provider,session_id,at_ms)` and `(at_ms)` indexes to the immediate transaction it already takes, on the pass worker and never under `Mutex<Runtime>`, and the table keeps the last 24 hours, which can span two UTC days, so at most 2,000 rows, two UTC days of the daily cap's 1,000; the hold table keeps yesterday's and today's sessions.
Both are safety limits for the pathological case, set from the operator's store from 2026-10-15 to 10-28.
A group is one extraction call and at most one comparison call; on 2026-10-28 the store counted 170 analysis calls for 121 groups, about 1.4 a group, so with no failed request 72 are 36 groups at the least and about 51 at that rate; a failed request and its retry spend more of them.
Its 435 session-days wrote a median of 3 groups, p90 14, p95 19, p99 35 and at most 52, so even at two calls a group the day's limit defers 4 session-days (under 1%).
80 requests an hour are 40 groups at the least when none fails; of its 218 active hours, the 10 above 40 groups were catch-ups: 10-18 to 10-20, before the four-group quantum, when a long session was analyzed whole in one pass (the busiest hour, 133 groups from 11 sessions), and 10-22 15:00 to 17:00 KST, after the schema 16 to 20 migration queued sessions again (up to 49 sessions an hour); from 10-23 the busiest of 89 active hours wrote 30 (about 42 calls).
On 2026-10-12 a schema catch-up wrote 580 groups in two hours (with-hide/hide-private#65); at 80 calls an hour, 40 to 57 groups, the same backlog spreads over 10 to 15 hours and leaves the check share untouched that morning.
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
The hook accepts at most 256 KiB of input and asks the core with one Memory hook request of its own beside its letter pull, starting no program, carrying the prompt's first 8 KiB; the core uses the attested checkout's prepared Project and at most 1024 bytes of confirmed task context, opens its store read-only, checks schema and the writer's indexed projection certificate, and requests at most sixty local FTS candidates within the earlier of the caller expiry and the 100 ms retrieval deadline.
The sixty candidates' operator sources are read in one more query (`memory_sources` through its primary key, which starts with the card id, each source's revision through `memory_revisions`' unique `(item_id,revision)` and its session's start through `session_sources`' primary key, so each card is a lookup of its few source rows; the operator's busiest card has 13, and no card keeps more than 1,000, `hide_memory::CARD_SOURCE_LIMIT`), counted in memory per card, and the candidates are ordered by score times the root of that weight, with no stored count and no write; schema 21's digest column is written by the analysis, never by a hook.
The body selector keeps at most three whole items and 600 estimated body tokens; the final escaped envelope, including wrappers and receipts, is trimmed to a shared 1200 estimated rendered-token limit, off the runtime lock.
These are local estimates, not provider billing tokens.
The core sets a one-second Memory answer deadline; `CoreHandle::memory_scope` bounds its owner reply with the remaining expiry, and remote admission, write and answer share one absolute deadline.
SQLite's progress handler observes the invocation expiry, and retrieval also enforces its local 100 ms allowance.
Memory and letter reads run beside each other: Memory waits until 1,250 ms after the inner hook starts, and the 1,650 ms intake budget reserves its last 400 ms for confirmation after a successful stdout flush.
The prompt supervisor bounds the inner helper at 1,850 ms from the outer helper's start; 1,250 ms is a Memory read budget, not the whole hook duration.
The core's lookup starts no model, embedding, transcript scan, child process, network request, repository discovery, full-store scan, or database write.
Missing, locked, corrupt, stale, unresolved, unreachable or over-deadline inputs exit successfully with empty context so prompt submission continues.
`SessionStart` reads the Project's index under the same read-only and fail-open ownership and the same Memory read budget: at most 20 one-line rule, decision, lesson and preference cards and no body.
The index reads every eligible card of the Project, as its salience order already did, and then one more query: the operator's sources behind every active card's current text, each joined to its revision and its session by key, tallied in Rust into sessions and the last time said; it keeps the 20 with the highest salience times weight.
The 10,000-card cap and the 1,000-source card limit bound the rows it can read, but not within the deadline: that bound is ten million rows, so the read looks at the caller's expiry every 256 rows read and every 256 cards tallied and ends with the deadline outcome, as SQLite's progress handler already ends a statement on the hook's connection.
The cards and their sources are read in one read transaction, which a WAL reader holds without blocking the writer.
The prompt match runs the same query for its matched candidates alone, at most 60, after the fit gate.
The fit adds, per prompt, one count of the Project's active cards on `memory_items_project_state` and one FTS5 count for each prompt word some candidate holds (at most the 24 words a prompt is read for), each stopping at a quarter of the cards, where a word weighs nothing; a word no candidate holds is not counted, and the caller's expiry is checked before each count.
The brief check adds, per prompt, one SHA-256 of the whole prompt in the hook (at most the 256 KiB of input it parses, as the letter pull already hashes it), and in the core a walk over the spawn records under the runtime lock that copies the briefs typed or launched into the caller's pane, usually none or one of at most 16 KiB each; the two digests per brief are taken off the lock, and a matching brief ends the answer before the store is opened.
On 2026-11-28, against a copy of the operator's store (906 active cards in the Project) in a debug build at load averages 31 to 37, three rounds of 539 of the operator's own prompts counted 4 words at the median and 20 at most, and the counts took 0.6 ms at the median, 2.4 ms at the 95th percentile and 7.9 ms at most, against 14.6 ms, 27 ms and 63 ms for the whole prompt retrieval; no idle or release-build measurement was taken.
On 2026-10-28, against a copy of the operator's store (462 eligible cards; the query read 699 source rows behind 430 of them) in a release build with a warm page cache, the old and new builds ran interleaved, three times each, 30 rounds of eight real prompts, while other builds held the load average between 23 and 31: the index read took 0.6 ms at the median before and 2.4 ms after (95th percentile 0.7-0.9 ms and 2.6-6.0 ms), and the prompt match 4.3 ms and 4.8 ms.
Linear in cards, the index at the 10,000-card cap is about 50 ms, inside the 100 ms retrieval deadline; under that load the slowest prompt round took 45 ms before and 54 ms after, and a round over the deadline returns empty context like any other over-deadline read.
Its separate Workspace probe has a two-second CLI timeout and runs beside Memory when its thread starts; later count-report calls also have their own two-second timeouts.
The installed SessionStart entry has an eight-second runtime timeout, so the Memory budget is not a whole-session-hook deadline.
`hide memory show|search|forget|undo|outdated|placed|building|unbuilt|skipped` is one store read or write on hided's blocking task; `add` is one provider call bounded at 50 s.

Regression owners are the `hide-project` identity tests, `hide-session` provider-neutral catalog and cursor tests, `hide-memory` Project-isolation, lifecycle, convergence, FTS transaction, redaction, ranking and budget tests, `hide-agent-hooks` fixture/config/fail-open tests, and core atomic-event and editor-preview tests.
Manual acceptance in the desktop app uses one exact worktree-local `hided` PID and window against a private state folder and private app state.
Record hook idle and driven timing separately, including sample count and failures, and record provider request count, queue/inflight bounds, child descendants, and RSS separately from the prompt path.
An automated deadline test proves bounded return under its fixture conditions; it does not prove every storage device, link or interaction answers inside the wait.
Report the hook's Memory time separately for a pane on the core's machine and one on a node, and the prompt hook's worst case beside it.

### Overview Work, backlog, History and Usage reads

The Overview's tasks are the core's (`agent_state/tally/tasks.rs`), rebuilt only with the agent scope cache's compared input, so an unchanged frame derives no task and adds nothing to the high-frequency path.
The shell sorts and filters them into sections in one memoized pass (`taskSections` in `web/src/overviewTasks.ts`) over the projects, rows and filter it reads, and an unchanged input does not regroup cards.
The core issue projection uses accepted catalog and metadata only; it schedules GitHub work when a screen names a project (`github_request`) or on an explicit refresh; the one timer it adds is the re-read of every local Git project in the ledger above, five minutes after a good answer, 30 seconds doubling to five minutes after a failed one (a read that missed its pull requests or its issues), and never on the clock for a repository answered "no GitHub remote" whose remotes the catalog knows.
A window gaining focus or a project brought to the front sends one `github_request` with `look`, which adds no timer: the core compares the clock of that one project and asks the existing reader again only when the last good answer is 30 seconds old and nothing is outstanding, so repeated focus changes cost one comparison under the lock and at most one read per project per 30 seconds.
The advisory poll (ledger above) adds one worker pass every 15 seconds while a window is attached, reading the project in front each time and every other project once a minute: at most eight projects, a handful of conditional `gh api` calls each, one `gh` at a time, and nothing published by the poll itself, so an unchanged pass creates no frame and no read.
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
The core rebuilds the tasks only with the scope cache's compared input (`agent_state/tally/tasks.rs`), the list draws no rows while the 프로젝트 list or a hidden sidebar shows, its elapsed clock updates only its own text, and a card on the desk adds one read per card the operator opens, the item's or the record's, on the event that opens it.
The sidebar's PR list (PRD work-tab-pr-lens B44, B53, sidebar-task-lens D-17, lane-two-columns B41) is `prListModel` in `web/src/prWork.ts`, one pass over the core's board of each project in scope, memoized on the snapshot; the core computes the groups and reasons in `agent_state/work/board.rs` from the `gh pr list` answer it already holds and the agents' state, with no process, file or network read under the runtime lock, and sends only the pull requests the lens shows, so the snapshot grows with that bounded list.
The lens adds three reads to the five-minute generation (the login, the review-requested search and one `HidePrFacts` query) and nothing per row; a card adds one GraphQL read when it opens or its head moves, at most eight cards are kept, and a card's summary is one background-router call per head, off the lock.
Its other reads are a pull request's body and feedback when 맡기기 or 새 이슈 만들기 opens (one `pr_feedback_read`, `gh pr view` on a worker), and its only writes follow a confirmation; hover, focus, unfolding and the half-second cards are screen state.


### Pane movement cost contract

Header dragging and menu navigation remain page-local until a committed move or an unattached-tab hover read.
Pointer updates render only when the candidate destination changes; one 450 ms hover timer requests text, and Escape or a changed scope cancels that preview.
The core admits at most sixteen active moves or unsettled publication barriers, reserves both participating tabs and shares the ordinary 64-tab admission count for a new destination.
It retains at most 128 move receipts in the existing Workspace view journal, saves before mutation off Runtime, and uses finite workers with a weak runtime owner rather than adding a resident timer or subprocess.
Each native operation has a five-second API deadline and journal acknowledgement a fifteen-second deadline; a recovery check reads actual topology and never resends the original mutation.
A move whose answer the page does not have settles by itself and is never left for the operator to check: the page asks the core for that check after fifteen seconds without an answer (again every fifteen seconds while still unanswered and connected) and as soon as an operation reads `unknown`, and the core ends the move as `completed` where the pane is at its destination and as `failed`, retryable, wherever else it is or when its position cannot be read.
A check for a move the core has no receipt of carries the request the page sent and ends it `failed` as not received (receipts are saved before any effect, so nothing was moved); a check of a move already settled lists its answer again for a minute, so a page that missed it clears.
Only a failure reaches the screen, with its Retry; the page draws no "check position" bar or button, and the detail goes to the diagnostic log under `pane_move.check_*` and `pane_move.request_not_received` with the request id.
The final publication barrier overlays only the participating tabs until a newly started coordinator bootstrap can replace it, adding one authoritative refresh per settled move and no per-key work.
Hover reads have one active worker and one latest pending request, at most 64 panes, 4,096 characters per pane, a 64 KiB API response cap, a two-second per-read deadline and a five-second batch deadline.
Cancellation is checked between reads; an already admitted read can finish, and a stale answer cannot replace the current preview.
The page indexes terminal identities once per changed catalog and readdresses an existing xterm instance without recreating its input, scroll or selection state.
Unchanged input callbacks add no catalog scan, subprocess or notifier fan-out, and terminal retention still follows the existing attached-tab limit.
Regression owners are `runtime::tests::pane_move`, delivery continuity tests, `web/e2e/pane-move.spec.ts` and `desktop/e2e/pane-move.spec.ts`; controlled measurement remains separate from these functional claims.

### The doorbell's lines in a terminal

`terminalBellLines.ts` reads a pane's screen after a frame to mark the rows that carry the doorbell's line.
The contract: `afterFrame` and a theme change only ask (`queueBellLines`), and a pane's screen is read at most once per animation frame however many writes the frame brought; the read is the viewport's rows once and nothing else, with no scrollback and no subprocess; a screen with no owl in it makes no marker and no decoration; and decorations are replaced only when the rows that carry a line, the width, the height, the words' color or the band's color moved, or a decoration or marker was disposed or left its line (a resize or a holder's reset does that); each replacement disposes the decorations and the markers they hung on, since disposing a decoration leaves its marker registered, so the markers held never exceed the rows drawn (two decorations hang on each, the band and the words) and are none once the terminal is forgotten.
The cost is one `translateToString` per viewport row, plus a cell scan only for a row that shows an owl or begins with two spaces.
Measure it in a real xterm in the browser: write output at the rate a flooding pane delivers, count the reads per animation frame from `buffer.getLine` calls inside the frame callback, count the decorations made, and time the callback with the owl absent and present against a build with the read switched off; report the load.
`web/src/terminalBellLines.test.ts` holds the counted parts (one read per frame, no decoration without an owl, no replacement when nothing moved) against a stand-in terminal, and the markers held and the redraw after a resize against a real xterm.
Observed 2026-11-11 in headless Chromium 153 on a machine at load 73, so a controlled-load run is still owed (a functional reference, not performance parity): 240 frames of four writes of 200 rows into a 120 x 50 screen (960 `afterFrame` calls) made 240 reads, never more than one per frame; a screen with no owl made 0 decorations; the read's callback took a p95 of 0.1 ms, the page's timer resolution, and at most 0.6 ms; and the median frame time over five alternating trials was 16.79 ms with the read off, 16.86 ms with it on and no owl, and 16.79 ms with owl rows on screen, a frame loop paced at 60 Hz that hides anything under a millisecond.

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

A URL's join across rows reads the holder's soft-wrap rows (`web/src/terminalWrap.ts`), which the shell keeps as one byte per screen row, replaced when a fragment carries them; a hover reads them for its own rows and asks nothing.
The holder reads each row's wrap bit on every capture (two parser calls per row, at most one capture per 16 ms per connection) and writes the sequence only on a full fragment or when the bits changed: a frame that moved no wrap, every key echo that does not wrap among them, carries no added byte; one that did carries 14 bytes plus one per four rows (about 24 for 40 rows, against about 6 KB for a full 120x40 frame), at most once per frame (`encode::tests::a_delta_says_the_wraps_again_only_when_they_change`).
The link ends (`LINK_END_OSC`) follow the same rule on every system: a capture reads each row's last written cell and, only behind a row whose last cell carries a link, the next row's leading blanks up to its first written cell (O(rows) per capture, O(rows x cols) at most when every row ends with a link, at most 512 x 512, and no parser call, since the rows were already read), keeps the last frame's bits while none moved, and writes 14 bytes plus one per four rows only on a full fragment or when they changed (`encode::tests::a_delta_says_the_link_ends_again_only_when_they_change`); the shell keeps them as one more byte per screen row, read by the same hover.
Regression owners are `web/src/terminalLinks.test.ts` (finite grammar, per-token bounds and cell ranges), `web/src/terminalLinkProvider.test.ts` (original precedence, missing/failed/over-budget checks, batching, cache and device boundaries), and `desktop/e2e/terminal-links.spec.ts` (real file opening and native path/link interaction).
For a matched dense-line comparison, record grid columns, token count and fixture spellings, then measure cold and warm hover-to-pointer latency separately with the same window and input sequence.
Count logical candidates, cache misses and IPC batches separately from actual native `realpath` and `stat` invocations; missing paths perform no successful stat, and cache hits perform neither.
With the existing `probe=1` QA seam, the provider records count-only `path_probe` diagnostics in the bounded diagnostic log.
Native IPC unique paths count admitted cache misses, not all logical candidates; a warm hover can have logical candidates while sending no host requests.
The original baseline has no count diagnostic, so record that limitation and cross-check its logical candidates with its original parser on the identical fixture cells.
Capture the exact candidate PID/window without activating it, and distinguish a renderer pointer/file-opening observation from an operating-system handler replaced by a recorder.

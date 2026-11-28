# 0028. Usage accounting

- Status: Accepted
- Date: 2026-10-13
- Code: `hide-session/src/usage/`, `hide-core/src/usage_ledger/`, `hide-core/src/runtime/usage_ledger.rs`, `hide-core/src/agent_state/tally/project_status.rs`

## Decision

### Historical usage accounting

`Call::UsageFiles` lists native candidates and `Call::UsageRead` returns one bounded, content-free usage step through the node boundary.
Candidate and read paths use `hide_platform::path` wire spelling, converted to native paths only by the node that reads them.
Both require the live link's independently advertised `ReaderFeature::Usage`; the node repeats that check before dispatch.
This optional reader adds no protocol-version requirement: an older peer grants no usage reader and is refused before a call is sent.
Cursor grants none, and protocol 24's frozen reader facts remain unchanged.

The usage worker owns one writer and reads only its supplied own-node link, alternating agents so a large history cannot starve another.
Each turn performs one bounded read and transaction, then rests for 20 ms; native JSONL parsing itself yields at 10 ms and never exceeds one MiB including its checkpoint witnesses.
That MiB holds the witnesses, Grok's `summary.json` and always room for one whole record at the 256 KiB line cap, so every step reads a record or starts passing one over; a `summary.json` too large to leave that room (over 785,407 bytes) is refused as `usage_read_Capacity`, never read past.
OpenCode's usage read sizes each row from its record header (`octet_length`) before loading it, so a message or part past the 64 KiB row bound is skipped as `record_capacity` and the cursor moves past it; measuring with `length(CAST(data AS BLOB))` loaded the value, and one row past SQLite's 1 MiB value limit failed every read and held the cursor on it.
OpenCode 2.x writes one assistant message per step and sets its tokens and cost once, when the step ends, so a completed assistant message is one usage event, keyed by its message id and placed in its session's checkout; a step still running, or one that failed without usage, is read again or passed, and a completed step with no usage is skipped as `usage_missing`.
The row holds its whole content, which a step that ran a long tool can make several MiB, so SQLite's JSON functions read the step's few fields in place and only those come back, each text cut where it is taken.
A row is scanned up to 8 MiB, and one read scans at most 8 MiB of rows in all: a row that would pass that total waits, the cursor still before it, and the next read takes it, while the first row of a read is always scanned so a read moves on.
A row over 8 MiB is skipped as `record_capacity`, which the worker logs as `usage_records_skipped`, so its step is reported as unread and never vanishes.
A database 2.x migrated from a 1.x one holds both sets of tables, with every migrated message in `session_message` under the id `message` still holds, so one cursor walks the messages of both in time order and the table that holds a message decides how it is counted.
A message the 1.x `message` table holds is counted from its step-finish parts, keyed by part id exactly as before the upgrade, so a ledger filled by an earlier read gets the same keys again and counts nothing twice.
A message only `session_message` holds was written after the upgrade and is counted the 2.x way, keyed by its message id.
A 2.x database with only one of `message` and `part`, or either without a column Hide reads, is no state 2.x makes and is refused as `usage_read_Schema`.
A database of neither OpenCode schema is refused as `usage_read_Schema`.
A failed listing or read is logged as `usage_list_failed` or `usage_read_failed` with the node's `reason` (its refusal code, such as `usage_read_Database`, or how the link failed), and the agent is listed again 30 seconds later.
Two replaceable visibility flags control its work: without a window it rests; candidate discovery runs every 30 seconds, or 3 while usage is open, and changed reports publish every 30 or 5 seconds respectively.
Its owner signals the condition variable and joins it on shutdown, including startup failure and an idle wait.
Complete uncapped listings retire deleted file checkpoints without deleting their totals or recent dedup authority.

`usage_ledger::store` owns `usage.sqlite3`, named by `hide_kit::layout::usage_store` under the core state directory.
Normalized usage events contain native call identity, model, time, session, working folder, provider, tokens and recorded cost, with no conversation fields.
A stored call holds each of those once, in the columns its readers use (the calls format, with-hide/hide-private#672): its key as 32 bytes, its node by the number of a `nodes` row, its tokens and cost as numbers, and its session and working folder by the number of a `places` row, one per (node, agent, session, folder), where a call that named no folder keeps an empty one, which no folder can be.
Nothing reads a call but the dedup lookup, a Claude call's later view, the projects' costs and the pruning, and none of them needs the whole event, so a call is about a fifth of the whole-event JSON version 3 kept beside a hexadecimal key: the operator's 638,000-call store of 512 MiB was 96 MiB converted and compacted (19%), under the 512 MiB bound with five machines of that history.
Whether a later view bills the same call is read from the stored values (time, model, tokens, cost, provider, incomplete), the same values version 3 hashed into a fingerprint column it no longer needs.
An ingestion transaction adds a new call and its daily `(native node, agent, local day, model)` totals together with the file checkpoint; repeated calls add nothing, and changed completed calls keep the first accounting value and report a count-only conflict.
Claude streaming records retain each token field's maximum for that native call, replacing its daily contribution and whole-call pricing band in the same transaction without another call; a later view that raises no maximum writes nothing and counts as a repeat.
A persisted checkpoint distinguishes a yielded read from EOF, so a restart cannot freeze an unread suffix.
Discarded capacity-overflow events leave a durable per-agent limited marker; a future timestamp is counted as skipped without poisoning its valid batch neighbours.
The day is assigned in the core's time zone at first ingestion and stays assigned after the time zone changes.
New calls use the current core time zone.

Each agent's first backfill may fill older days within the 400-day retention window, and its event keys stay until that backfill's durable completion marker is written.
After completion, whole days before the 45-day event horizon are frozen: replayed or late calls before that boundary are counted as skipped and never change those totals.
The boundary is retained as a monotonic instant so a clock rollback or time-zone change cannot admit a call whose dedup key already expired.
Maintenance removes at most 512 expired calls, 512 places and 512 expired daily rows per turn; the call bound is one million per native node, the candidate bound 10,000 per agent, the daily-row bound 100,000, the native-node bound 32 and the database bound 512 MiB, with each refusal reported rather than silently growing storage.
A place keeps its newest call's time and is pruned once that call and every other expired call of its agent are gone, so a place outlives no call of its own and a node's places never outnumber its calls.
The store uses SQLite WAL and full synchronous commits, and refuses a damaged or newer store without rebuilding it.
The `calls_node_at` index on `(node, at DESC)` serves the projects' costs below and the pruning; beside a table without rowids it carries the primary key `(node, agent, key)` too, ascending after the newest-first time, so a read of a node's newest calls ordered by time, agent and key sorts nothing.

A version 3 store is converted in place, never rebuilt, and never all at once.
Its first open by a build of the calls format adds the new tables and a node for each device in one transaction and leaves `events` and its indexes as they were; a store an older build filled to its 512 MiB has no free page for those tables, so there the open makes them in the first step's transaction, after it deletes that step's calls from `events` (`a_version_3_store_at_its_page_limit_opens_and_converts`); the device's count in `devices` stays the one count of its calls in both tables, which a move leaves alone; from then on the worker moves at most 4,096 calls a step, each step one transaction that deletes them from `events` and writes them as calls (`Store::convert`), until `events` is empty.
Until `events` is empty the worker reads, counts and prunes no call and the projects' costs stay pending, since a call still waiting in `events` would not be found as already counted; the daily totals and Hide AI's tables stay readable throughout.
A step that fails or is ended changes nothing, and the next step, in the same process or after a restart, takes the same calls again; a damaged call fails its step with its cause, logged and on the usage page, and is tried again a minute later rather than skipped.
The operator's 638,000 calls took 156 steps, 92 seconds in all and 2.1 seconds the slowest, on a Mac running other builds at the same time.
Moving leaves the old calls' pages free, and SQLite reuses free pages but never returns them, so whenever more than half the file is free pages, after a conversion or at any open, the worker compacts it once with `VACUUM` and empties the write-ahead log (`Store::compact`, its own two-minute budget, four seconds for the operator's store on that Mac); a compaction that fails is logged and the store stays as it was, larger, until the next open tries again.

Going back to a build from before the calls format, at any point and halfway through a conversion too, keeps every record this build wrote and every count true, and coming back within 45 days counts each call made in between once; outside that rule are a longer stay and a session file its agent removed in the meantime (Claude Code removes a session 30 days after its last activity unless `cleanupPeriodDays` says otherwise), whose calls are not counted.
A device at its limit of a million calls is outside it too: the older build skips the stay's calls past the limit as it always has, marks the agent limited and logs `usage_event_capacity`, and this build does not count them again.
The store's version stays 3, the oldest layout a build reads and writes it correctly in, and every table a version 3 build uses stays as it made it, `events` empty, so such a build opens the store as its own and records Hide AI's calls into `ai_daily`, which the calls format did not change.
A native call it would count goes into `events`, which it reads to find a call already counted and which no longer holds one, so it could count a call this build already counted; the store refuses that write (the trigger `events_closed`), and the refusal rolls back the older build's whole ingest, checkpoint included, so it counts no native call while it runs and its usage page says the ledger failed.
It still finds, merges a later view into and prunes the calls a halfway conversion left in `events` as it always did, taking each it prunes off the device's count in `devices`, which counts the device's calls in both tables, so the count stays the number of calls there are and the limit of a million calls a device is one value for both formats (`an_older_build_pruning_a_halfway_conversion_keeps_one_true_count`).
Back on this build the conversion goes on, and the stay's calls are read from their native files after the checkpoints this build left and counted once, a later view of a call already counted merged as ever (`an_older_build_counts_no_call_into_a_converted_store_and_the_return_counts_each_once`).
A change that keeps an older build able to open the store adds its tables, indexes and refusals beside the ones it uses without raising the version, as Hide AI's tables and the calls format do; one an older build would misread raises it, and that build then refuses the store (`usage_store_newer`) and loses Hide AI's calls of that time when it exits.
Native physical node identities and their qualified source paths stay unchanged when the core's owner changes.

The same store retains the public price cache and the last network-attempt time, so a restart cannot bypass the daily fetch bound.
Daily totals hold four fixed token-only context bands, not another table of call identities, preserving tier-aware repricing across event expiry within the existing row and database caps.
The separately owned price-fetch thread asks the own node for `Call::UsagePriceFetch`, refused for devices.
The node owns the bounded public HTTP read; the core parses the returned body and the ledger's sole writer persists the table before publishing it.
Shutdown closes the own-node link before joining the ledger and price threads, so those joins do not wait for an in-flight HTTP answer.
Closing also releases the node's label-generator locks, even while unfinished calls retain node references; a lock acquisition completing after close releases its file instead of retaining the role.
The teardown-only `AiRelease` and `LabelUnlock` requests remain callable after close under the existing call cap and timeout, because their owners may drop later; closing does not wake or replace their cleanup answers.
The node's unfinished HTTP work retains its capacity slot and five-second deadline until it ends.

`usage_ledger::ai_usage` is the other writer's input: routers tell its bounded `AiUsageQueue` after each attempt, and the same ledger worker drains it at most once a second into `ai_daily`/`ai_state` of `usage.sqlite3` (`store/ai.rs`).
It reloads those rows and publishes only when a usage page is visible, so a hidden window costs the queue push and the batched write only; `docs/AI_PROVIDERS.md`, Counting Hide AI's own usage, owns what counts and the caps.

#### Project status and cost

The Project tool's counts and 흘린 것 (PRD project-panel-results D-04, D-05, B1-B17) are derived, never stored, and reach the shell as `agent_scope.status`.
`agent_state/tally/project_status.rs` builds them inside the scope cache (`agent_state/tally/scope.rs`) from what that cache already compares: the PRs list's board rows, each checkout's Git facts (`GitInput`), the facts a 흘린 것 row reads (`StrayInput`: branch, base branch, changed file count, head, last commit and pull request checks), the last session the record says ran in each worktree with something left (`recorded_runs`, looked up for those worktrees only) and the 놔두기 in `ui_state.stray_dismissals`.
So a publish whose inputs did not move rebuilds nothing, and nothing on that path reads a file, runs a process or takes another lock.
`stray` makes a row for a worktree off the default branch with changes, unpushed commits (the rule the worktree's mark reads) or an open, non-draft pull request whose checks failed, unless a live row in its panes works or waits on the operator (`AgentUse`); its name, pane and agent come from the newest live row there, else the recorded run, and its head binds a 놔두기.
`strays` keeps at most `STRAY_LIMIT` (64) rows, newest first, `strays_total` counts all and is the rail's number, and the first crossing logs one `project_status`/`stray_capacity` diagnostic per project.
`runtime/strays.rs` owns the three events: `stray_dismiss` stamps the row's head and failing pull request into `ui_state.stray_dismissals` (persisted, keyed by checkout id and moved with the checkout by `node_migration`), keeping at most `MAX_STRAY_DISMISSALS` (256), the oldest dropped with a `stray.dismissal_capacity` diagnostic; `stray_restore` removes the stamp; `stray_hand_off` takes a request id, refuses one it already holds, and writes the fixed words through its `spawn_stray_prompt` (the engine's `prompt_agent` on a worker, admitted by the input gate) or starts an agent in the checkout through `agent_start_in_checkout`; `status.stray_hand_offs` keeps the last 16 receipts (`sending`, `sent`, `failed`) and every settle logs `stray.hand_off`.
A local agent that stops working has its project's worktrees read again (`read_worktrees_after_turns`), since Git's watch does not see an edit left unstaged (B17).
`git.dots` keeps at most `DOT_LIMIT` (96) marks; the counts stay exact, and each checkout's own scope carries its mark uncapped (`git_mark`, the one rule `checkout_mark`), which the worktree table lists.
The card `remains` names a worktree; while it is the card in front of the drawn desk (`shown_card`), `remains_target` reads that worktree's changes, in front or not.

A project's cost (`usage_ledger/project_cost.rs`) is prepared on the ledger worker, never under `Mutex<Runtime>`.
The runtime tells the worker each local project's checkout paths (`feed_usage_projects`, on a catalog replacement, notifying only on a change); the worker reads the last eight days of its own node's `calls` (`store::recent_calls`, newest first by `calls_node_at` and calls of one time by agent and key, a total order since `(node, agent, key)` is the primary key, so a capped read keeps the same calls under any plan; at most `EVENT_LIMIT` 200,000 calls with a two-second progress budget, `limited` past it), keeps the seven days ending today, gives a call to the project whose deepest checkout holds its session's folder (`files::path_inside_root`, decided once per folder), and prices each (agent, day, model) and (agent, session, model) group with `prices::cost` as a day is priced.
Each call is decoded, added to its groups by `project_cost::Preparation` and dropped, so a build holds the groups and the folder owners, not the calls.
It rebuilds when the projects or the day change, and when ingestion added or rewrote a call of those eight days or a new price table dirtied it, at most once per `COST_REBUILD` (30 s); a backfill of older history and pruning, which removes only calls past the 45-day horizon, leave it as built; a failed build keeps the last value, stays due, is tried again after the same wait and logs each new cause once.
The result rides the usage frame as `nodes[].projects`, absent until a build succeeded so a tile tells a cost not read yet from a read that found nothing (B33), with `projects_failed` while the latest build failed or the usage store did not open; a project that ran no call is not listed, and an agent with an unpriced call is named in `unpriced_agents` with its priced calls still summed and the total marked `partial`; the daily totals, the Overview's Usage and the calendars below are unchanged, and a project on a connected device has no cost here.
The build also keeps each session's spend in a project (`SessionSpend`, core only: its priced amount, new tokens, first and last call and working time, the gaps between its calls up to `WORK_GAP_MS`, five minutes, added up), and Hide AI's own seven days from the `ai_daily` rows (`hide_ai_week_micro_usd` on the frame), which belong to no project.
The runtime keeps the worker's spends (`ledger_spends`) and makes each project's `tasks`, `tasks_total` and `week` in `runtime/project_tasks.rs` with `usage_ledger::tasks::build`, a pure function over the spends, the record's summaries (each session's parent, the pull requests it made or worked on, its folder, the line of its last judged turn and when it last ran `git commit`), the missions, the live rows (who works or waits, the first line of what each last said), the catalog's pull requests and each checkout's left changes and whether its commits are in the default branch.
The session reader marks a tool call that runs `git commit` (`named_prs::runs_git_commit`, past git's own options, in every agent's shell call) and the store keeps one `ledger_sightings` row of kind `commit` per session with the last call's time.
A link store from before version 14 is read again from each file's start, as versions 10 and 12 were, so the sessions already read get their `git commit` calls too.
The Project tool's `남은 것` has a read of its own beside the checkout in front (`Runtime::remains_request`, `ingest_remains`, a second `ChangesReader` on the changes pump): the shown worktree's changes and committed group by its path, with no row and no diff, published as the rest section `remains` with the workspace and checkout it was read for, since a nested worktree's folder lies inside its primary's, while the viewing column shows it and emptied when it closes.
A worktree whose folder this daemon did not open is published as unreadable (`host.root_not_opened`) rather than left waiting, and a failed read keeps the list already shown, marked stale.
A session counts in one task: the mission its Conductor leads, else the nearest forebear that made a pull request or has no parent, so the tasks never sum past the total (D-09, B28); at most `TASK_LIMIT` (200) are sent, costliest first.
It runs when the worker reports, when the record's summaries change and when the agent scopes' compared input moves (it holds the agents, pull requests and Git facts a result reads, and the minute; `Cache::input_generation`), and does no I/O beyond reading the time zone once.
Each call gathers every project's facts (`tasks::Facts`, plain data) and builds a project again only when its facts or the worker's spends differ from its last build (`TasksBuilt`), so an agent's state change rebuilds nothing whose facts it did not move; a build resolves each session's checkout once, a frame whose result is unchanged is not sent, and a project whose tasks cross a cap logs `project_tasks_capacity` once.

`usage_ledger::query` prepares both the 20-week and 53-week calendars on the worker, including every agent selection; the shell formats and draws their prepared values.
The same pricing pass prepares one shared model list keyed by recorded civil date, agent, model and session-or-Hide-AI source.
It retains at most 4,096 keys in deterministic newest-first order within the retained-year calendar, with `models_limited` when older keys are omitted; the complete daily sums are independent of this detail cap.
Each model amount uses the actual reported tokens and the same price resolution as its daily amount, and an unnamed native model remains null rather than being inferred from an agent.
Hide AI model rows count per-model reports; its existing headline and feature rows continue to count backend attempts, including separate unknown-call counts that create no model amount.
The shell filters and sums these prepared records for seven, thirty, ninety or retained-year dates and a selected agent or day, without native reads, repricing or additional polling.
Each range sends inclusive `calendar_start` and `calendar_end` civil dates and only days with recorded calls, including calls with zero new tokens.
The mounted calendar fills omitted dates as level-zero cells, pending while that range is reading; it does not recalculate totals, prices, quartiles, streaks or recorded-day contributions.
This keeps empty history from repeating hundreds of empty amount records on every attach or reconnect.
Only the core's own native-node rows enter these calendars.
New tokens include input, output and cache writes, never cache reads; reasoning is already part of output.
Nonzero closed days determine nearest-rank quartiles, or quarter-of-maximum thresholds below seven active closed days, so today's unfinished spike cannot recolor history.
The calendar starts on the first record's week, capped to the surface's week count, and stays anchored 45 days back during first backfill; agent selection preserves the calendar and the other agents' shares.
Empty days during backfill remain pending, zero-call ranges remain no data, and cost reasons carry at most eight distinct unpriced model names plus the omitted count, together with price-loading, stale-price and incomplete-usage facts.
Agent hover costs cover retained history even when its first days precede the displayed calendar.
Provider provenance is bounded to 32 distinct native ids, and crossing that bound refuses the projection rather than omitting an id silently.
Calendar arithmetic uses civil dates across daylight-saving transitions, and a current streak may include yesterday while today is still empty.

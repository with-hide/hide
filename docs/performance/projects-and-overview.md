# Projects and Overview costs

- Code: `hide-core/src/disk.rs`, `hide-core/src/runtime/disk_sweep.rs`, `hide-host/src/disk.rs`, `hide-host/src/cleanup.rs`, `web/e2e/projects-sidebar.spec.ts`
- Index: [PERFORMANCE_TESTING.md](../PERFORMANCE_TESTING.md)

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
Dropping `Core` (`hide-core/src/handle.rs`) stops producers and joins the last save outside the mutex before the process exits; forced process termination does not guarantee a pending save.
Standalone unit runtimes without a worker context retain synchronous persistence outside any shared runtime mutex.

A pin is one more key in the same sort and one more exclusion in the same fold pass; `workspace_pin_set` re-sorts the projected list in place and persists through the existing off-lock save, and the removal counts ride the checkout summary pass rather than a second visit of the panes.
Closing a project's panes for `Remove project…` runs on the same worker pattern as worktree deletion, outside the mutex, with one in-flight close per project.
Closing the panes left in a linked worktree removed outside Hide (`runtime/gone_worktrees.rs`) adds, to each catalog reconcile under the mutex, one visit of this machine's checkouts with a lookup in the node's path answers the coordinator already brought, and an agent pass (`cleanup_facts`) only for a project with a gone row that has panes; the node read, the in-use read and the close run on one worker per gone row outside the mutex, started once while the row stands, so the pending work is bounded by the gone rows drawn and nothing is announced beyond the panes the engine reports closed.
Regression owners are `projects_follow_authoritative_activity_and_identical_snapshots_settle`, `overview_tracks_live_checkout_panes_and_drops_retired_lineage`, `pinned_projects_lead_their_device_in_name_order_whatever_their_activity`, `pinning_a_registration_reorders_the_row_and_persists_the_flag`, and the `removing_a_registration_*` and `removing_registration_*` tests.
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
The catalog pass is bounded by the worktree count; a project with many worktrees pays, per change, one status for each worktree whose index moved and the commit questions (counts, line totals, merged, upstream, last commit) for each worktree whose commits moved, never per tick or per agent update.
A Git HEAD, index or ref change is scoped to its own repository: the OS watcher groups a burst into one generation change, so that project alone is re-read and every other project is answered from the worker's last read.
A read a Git watch asks for starts no sooner than 5 s after the last read landed (`worktrees::WATCH_READ_SPACING`), on this machine and on a device: an agent's `git status`, `git add` or commit moves an index, and a repository whose agents keep running Git moved one every few hundred milliseconds, so it was read again the moment each read landed; now every change inside the wait is that one read, and a busy repository is read once per 5 s plus the read's own time.
An explicit read, a new project or base, a reconnect and a failed read's retry are not held.
The 300 ms a watch waits to group one Git command's writes counts from a repository's first change, since a quiet period counted from the last never ended while agents kept running Git, and the device's repositories were then never read again (measured on a loopback device with a change every second: no read in 24 s).
`watch_changes_inside_the_spacing_are_one_read_after_it`, `a_repository_that_keeps_changing_settles_after_its_first_change` and `a_watched_change_reuses_unmoved_worktrees_and_a_failed_read_is_tried_again` own the spacing.
Within that repository the node keeps each worktree's last facts with what they were measured from (the listed HEAD, branch and lock, the worktree's index file, its base, its upstream, and the commits the base, `origin/<base>` and the upstream name, from one `for-each-ref`), so a watch read measures only the worktrees whose inputs moved and answers the rest from that memo (`hide_host::worktrees::Measured`, one per node, the last 64 repositories read).
Within a moved worktree a watch read asks Git again only what moved: a worktree whose index alone moved runs one `git status` and keeps its commit answers, a refusal such as two histories with no merge base included, since Git answers the same commits the same way however often it is asked, and its ignored-folder scan, which no Git input follows; a moved HEAD, base or upstream asks the commit questions again, and an explicit read asks everything again.
`a_watch_read_asks_git_again_only_about_commits_that_moved`, `a_watch_read_keeps_the_ignored_folder_scan_and_an_explicit_read_scans_again` and `commit_facts_out_of_time_are_not_kept_for_reuse` own this.
The daemon's own node, a device's helper and a node that dialed its core also keep the memo in their state folder (`worktree-facts/`, `hide_kit::layout::worktree_facts`), one file a repository and the files of the last 64 repositories written: a file is replaced whole, off the memo's lock, when a read changed what it keeps, and read for a repository the node has not read since it started; a file of another version, or one that does not read, keeps nothing and fails no read (`worktrees.facts_unread`).
An explicit read (`overview_refresh`) measures every worktree again, the longest unmeasured first.
A project's first read since the core started, before any explicit read, may reuse, so a restarted daemon measures only the worktrees whose inputs moved; an edit made while it was down that moved no input shows at the next explicit read, as one made while it runs does.
Idle repositories do not run Git commands or sample working-tree files; content-only edits are reflected at the next explicit read.
On 2026-10-14 a 32-worktree repository took 48 to 51 s per whole read at load 15 to 24 (`hide_host::worktrees::read` timed against the live checkout), and was read whole again on every commit in any of its worktrees, past the core's 60 s wait 16 times in 89 minutes; a read that may reuse, with nothing moved, answered in 0.16 s.
The same checkout with 39 worktrees, read by a fresh memo as after a restart at load 11 to 36, ran into the 45 s read bound with 7 worktrees unmeasured with an empty folder, and answered all 39 in 0.24 s with the file a previous read left, nothing having moved since (24 s when that file lacked 7 worktrees and live agents had moved others).
`a_commit_in_one_project_does_not_rerun_status_in_another`, `a_commit_in_one_worktree_measures_that_worktree_alone_until_a_refresh` and `idle_and_working_tree_edits_do_not_reread_but_manual_refresh_does` own these boundaries, and `a_restarted_node_answers_from_its_folder_and_measures_only_what_moved`, `a_file_of_another_version_or_that_does_not_read_keeps_nothing`, `the_folder_keeps_the_files_of_the_last_repositories_written` and `a_first_read_since_start_reuses_and_one_after_an_explicit_ask_does_not` own the restart.
A finished worktree removal follows the same scope: its row leaves the catalog under the lock with no Git call, the coordinator rebuilds the rows on its next wake, and the reader re-reads only the removed worktree's repository; `a_finished_removal_drops_its_row_at_once_and_an_older_read_cannot_bring_it_back` owns this.
The linked-worktree facts pass also measures ignored repository boundaries off the runtime mutex once for each linked worktree an explicit read measures, or a watch read measures for the first time; a removal scans again before it deletes anything.
Each scan has hard caps of 2,000,000 steps, 30 seconds and 1,024 names, and its time is also whatever is left of its worktree's 30 s.
The scan verifies each repository it finds with one `git rev-parse`, so it is the read's largest cost: a worktree holding 550 fixture repositories in ignored run folders took 15 s, against 0.3 to 1.4 s for one holding none.
One repository read on the node has one absolute 45 s deadline (`WORKTREES_READ_TIME`) starting at public `read`, including main-worktree discovery, preliminary Git metadata and the subsequent measurements; `read_project`, entered with a known root, starts the same budget at its own boundary.
Every preliminary Git command uses only the remaining read time, capped at its existing 15 s, and expired reads start no further command; standalone Git and registration helpers retain their own 15 s limit.
This leaves owned-child and pipe cleanup inside the core's 60 s wait (`WORKTREES_TIMEOUT`), and one worktree's measurement uses at most 30 s (`WORKTREE_MEASURE_TIME`), so a slow worktree leaves the others time.
`a_public_read_spends_one_deadline_on_discovery_metadata_and_measurement` calls the real public boundary with delayed Git answers in an isolated process on every system; it owns the overall bound and caller-visible unavailable result.
A worktree its own 30 s cut short keeps that answer, named as not fully measured, until its inputs move or an explicit read; one the read's 45 s cut short, or never reached, answers its last measurement, moved or not, and only one never measured is named unmeasured; the next read measures the longest unmeasured first.
A repository whose moved worktrees did not fit in its read once named the same rows unmeasured on every read.
`a_worktree_that_does_not_answer_is_named_and_the_next_is_still_measured` and `a_read_out_of_time_answers_each_worktree_its_last_measurement` own these bounds.
A repository that cannot be listed, a worktree Git cannot fully describe and a device read that fails are each logged (`worktree_list.failed`, `worktree_status.unavailable`, `worktrees.read_failed`) when that starts or its reason changes, never again on each answer that repeats it, so a failure that stays is one line; `a_failure_is_logged_when_it_starts_or_changes` owns this.
It skips Git metadata and directory links; a failed or capped scan publishes an unavailable fact rather than an empty list.
A confirmed deletion uses the existing single removal slot and one preflight worker before any pane close; the same host check runs again before guarded removal.
No scan is added to pane input, a snapshot tick, hover or an unchanged catalog read, and only phase transitions notify the shell.
Every `git` the catalog runs is bounded by `GIT_DEADLINE` (15 s), and by what is left of its worktree's time, and drained off-thread past the pipe buffer; a repository that outruns it reports its status unavailable and a `git.deadline_exceeded` diagnostic rather than holding the other projects' answer, which a status over evicted iCloud files once did for minutes.
Group ordering, chips and search are pure functions of the accepted snapshot; agent status updates redraw rows and never recompute the catalog.
List rows use the existing lazy-loading and search keyboard patterns.

Disk reuses `DiskReader` (`hide-core/src/disk.rs`), triggered by the right panel's request, a cleanup sheet's project and the Overview's Projects tab (`disk_watch`, `hide-core/src/runtime/disk_sweep.rs`), with one inflight read and coalesced pending input; the walk itself runs on the checkouts' node (`hide-host/src/disk.rs`, the `disk_usage` call), which reports each checkout as it finishes.
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

Cleanup uses the existing action worker context with one active review/removal, never the Runtime mutex, for Git, disk and fresh engine snapshots.
Cleanup protects both launch `cwd` and current `foreground_cwd` from the generated snapshot contract; it does not change navigation projection policy.
Review reads are non-mutating; confirmation rechecks each target immediately before `git worktree remove` without force.
Git and the engine have no shared atomic filesystem transaction: a state change after the last engine check cannot be reserved against by this contract.
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
Regression owners are the `worktree_cleanup` tests for in-use reads, exclusion codes, cell rechecks (`a_folder_that_changed_after_the_measurement_is_kept_and_named`), repeat convergence and the single `ls-files`, and `a_cleanup_confirmation_only_selects_what_the_review_allows_and_runs_once`.

Regression owners include `overview_open_section_focuses_the_checkout_and_switches_the_panel_in_one_event`, `agent_start_in_checkout_reports_through_the_task_operation_slot`, `behind_upstream_is_absent_without_an_upstream_and_counts_the_fetched_side`, `linked_worktrees_carry_their_creation_time_and_the_main_worktree_none`, disk filesystem fixtures and cleanup filesystem fixtures.
Manual acceptance in the desktop app additionally covers the Projects tab's rows and disk cells, narrow Korean/English wrapping of project and branch names, `…`/`?` cells, and cleanup review/cancel/exclusion/success/stale refusal in private fixtures only.

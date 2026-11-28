# 0011. Pane records and reconcile

- Status: Accepted
- Date: 2026-10-31
- Code: `hide-kit/src/panes.rs`, `hide-kit/src/panes/`, `hide-holder/src/record.rs`

## Decision

### Pane records and reconcile

A holder outlives its node, so what owns it is its record, not the node (PRD engine-bundle-2-pane-holder D-13, D-14, D-17).
Each pane of a state folder has one, `<state>/holders/records/<pane>.json` (`hide_kit::layout::pane_record`), in a versioned format (`hide_holder_protocol::record`, format 2, at most 64 KiB): the pane id, the state folder it belongs to (the folder's own file identity, which a rename keeps and a copy does not), the start's incarnation, the holder's and the program's pid with the start time each had, the protocol and version folder it runs, the digest of its command, the boot it started in, its phase (`reserved`, `preparing`, `running`, `closing`, `ended`) and, once ended, why, with the exit code and any pids left.
Format 1 running records remain readable and retain their format when updated.
Format 2 can also carry the reservation's connection owner, logical request number, original request digest and recovery folder, without persisting its command or environment; `reserved` requires this metadata and names no process.
The node exposes reserve, exact-incarnation start and cancel as typed pane calls, with the original request digest computed before shell integration changes its command.
Only the worker changing that reservation from `reserved` to `preparing` expands the shell and starts its holder; an old incarnation or a cancelled request is refused before expansion.
Cancel can precede reserve and writes a terminal record for that same intent, so a delayed reserve cannot recreate it.
Each connection issues increasing logical creation numbers, distinct from transport call IDs, and uses the same number for reserve, start and cancel.
Holder renewal carries a number from that same issuer (`PaneCall::Renew`, protocol 93), admitted before any close or replacement and used through the replacement's reserve and start; the node never issues an independent number behind the core's counter.
The core retires a renewal after a definite answer, including `current` and `kept`; an unknown answer or unsettled replacement retains its pending number until owner turnover, as a node-issued replacement did.
Its owner keeps the highest observed number and at most 128 unresolved entries, including gaps for issued calls not yet received; a jump that would exceed the bound or overflow is refused before advancing the window.
A completed number is absence below that high-water mark, so a late reserve, start or cancel cannot return even after its ended record is pruned.
Retirement closes admission under the same owner mutex as worker admission and completes only after every admitted worker has finished its mutation and cleanup; no I/O holds that mutex.
Unresolved ended reservations remain pinned in the record cap, while confirmed retirement releases them on the same owner, allowing more than 128 cumulative creations without growing a history.
A definitely unsent creation also retires its issued number; losing the retirement answer retains its core slot until the same retirement is confirmed.
The core admits eight active and eight waiting creation intents before placement or durable reservation, and retains the original placement and request through unknown reserve/start/cancel outcomes.
Only confirmed durable reservation permits publication; authority and parent existence are checked again after that answer, and failed publication cancels the exact intent.
One intent state serializes the cancellation claim with topology publication, so a cancellation selected before publication cannot leave a visible cancelled pane.
Placement starts, completion and withdrawal belong to that same intent; callbacks run outside all engine locks, and cancellation waits for an in-flight callback before withdrawing exactly the successful placements and retiring the request.
A placement still in flight at the existing start bound leaves cancellation unanswered and retains its admission until a later resolution observes actual cleanup.
Closing selects and marks its panes in one topology hold before node I/O; a late split into a closing pane or a new tab in a wholly closing workspace is refused, while a pane published before selection is included in a tab or workspace close.
The in-memory closing mark remains after an unresolved close, so another close resolves the original scope instead of admitting new children behind it.
Cancellation and a late successful start both finish terminal-service cleanup before their request guards leave.
A later close of a started pane uses record ownership and incarnation checks, without reopening its retired creation request.
A new owner reports interrupted reservations with their original pane ID, incarnation and recovery folder; legacy ended records report the incarnation but need a saved layout for the folder.
`hide_kit::panes::Panes` is the node's side of it, a library hide-node calls; its diagnostics (`pane_holder.*`, naming the pane, the incarnation and the holder's pid, never the pane's output) go to the sink the caller gives.

A connection acquires the state folder's exclusive `records/.owner-lock` through the platform file-lock implementation before crash repair or fresh reconciliation.
Its `PaneSlot` and each admitted node request share the bound holder adapter, which retains that lock through the request's last mutation and cleanup, even when its caller times out or disconnects.
Closing the connection permanently closes admission and releases only its own reference; queued requests cannot reactivate it, and an acquisition finishing after close cannot install a new owner.
A new connection cannot reconcile or allocate pane IDs until every previous worker has finished and it acquires this same lock; failure to acquire is an unresolved outcome, never evidence that the old work ended.
The stable lock file is never removed or replaced, and node start, close, forget and reconcile require the acquired ownership context.
Holder processes retain their separate exact-incarnation record claim and do not inherit connection ownership.
The pure liveness query counts records without pruning them.
A failed or incomplete record scan refuses connection ownership; a completed scan with individual unreadable records retains the bound owner and usable panes.
Those unreadable IDs remain present, count against the record cap and refuse their own starts without changing their bytes.

Every change of a record, a start's reservation, the count against the cap and a removal happen under one lock file, `records/.lock`, and nothing that waits on another process runs under it: a holder starts, a tree ends and a connection opens outside it, and what they found is written under it after a fresh read that shows the record has not moved.
A start is keyed by its pane id and original request digest: concurrent claims create one holder, a running match answers that holder, and a still-preparing match reports an unresolved start.
An ended reservation refuses the original start; a deliberate replacement names its previous incarnation and starts only once the earlier processes are confirmed gone.
Legacy records retain their prior adoption and restart path until replaced by a format 2 reservation.
Cancellation writes its intent under the same record lock that matches the request, before any process cleanup.
The holder's final write and the node's lost-holder settlement both preserve that decision; confirmed cleanup ends as terminated, while unconfirmed leftovers remain closing and cancellation reports an unresolved outcome.
A cancelled holder's final write still leaves its record closing, because it cannot prove its own exit while writing; only the node's cleanup followed by an exact-record compare and replace completes it.
A holder that already wrote ended but still runs follows this same closing path, including when cleanup refuses or the connection disappears before it runs.
While something of the earlier start runs on, its holder by pid and start time or on Unix a member of its program's session (elsewhere a pid its end says was left), the start is refused with those pids.
A record of another state folder or one that cannot be read refuses the start.
A state folder keeps at most 128 holders: every record not ended counts, and so does one that cannot be read, since it may name a holder; the 129th start is refused `holder_cap` with a `pane_holder.start_refused` line, however many ask at once.
Closing a pane whose start has not written its holder in yet writes it `closing`, which stops that start, and answers that the pane is still starting.
Ended records stay so the next node can say why a pane ended, at most 128 of them, the oldest removed first with its log, except one whose holder still runs or whose pane left processes running (its program's session on Unix, and every live pid its end names even when that session was read), which is kept and logged (`pane_holder.ended_kept`); a forget refuses such a record the same way.
A restart that names the incarnation it replaces and finds that record gone therefore starts the pane under a new incarnation and logs it (`pane_holder.replaced_record_missing`): closing a restore's finished panes writes them newer ends, so the cap can drop an interrupted record before its restart reaches it, and nothing of the dropped one runs.
The incarnation comes from a counter of its own (`records/next`), so a pane started again never reuses an earlier start's number; a process moves the counter on 64 at a time and gives those numbers out itself, so the counter is written once in 64 starts.
A record and the counter are replaced whole with `write_file`'s `WriteOptions::UntilReboot` (see the platform layer): on macOS they flush to the device but not past its cache, since their holders cannot outlive a reboot, and a full device flush per write cost a restore of 66 panes about a second.
What a power loss may leave instead, an empty, cut-off or older file, never stops the panes starting again: a node that opens the folder removes every record that cannot be parsed and was last written before this boot began (`process::boot_time`; `pane_holder.record_removed`), the reconcile ends an older readable one as of an earlier boot, and a counter that cannot be read or is behind an incarnation a record holds is moved past all of them (`pane_holder.incarnations_mended`); a record damaged in this boot, or of a format this build does not know, is left and reported as before, since its holder may run (`Panes::mend_after_a_crash`, `records_a_power_loss_left_empty_cut_off_or_old_do_not_stop_the_panes_starting`).
A start's version folder name is the hash of the holder's bytes, taken again only when one of its files changed (identity, length or time), so a start does not read and hash the holder each time; a folder is still made only under the name its bytes give.
Each start's `pane_holder.started` line gives when it found its folder, reserved its record, started the holder and read its `ready` (`prepared_ms`, `reserved_ms`, `spawned_ms`, `ready_ms`, `took_ms`), and the holder's `holder.started` line when it wrote itself in, listened, started the program and wrote `running` (`claimed_ms`, `listening_ms`, `spawned_ms`, `running_ms`), so a slow start says where its time went.

The start writes the record `preparing` and starts the holder with the record's path, the lock's and the incarnation (`--record`, `--record-lock`, `--incarnation`); the holder writes itself in before it starts the program, opens its parser and listens at its socket, then starts the program and writes the program and `running` in one write, and only while the record still reserves its start, so a holder whose start was given up on, or one of an earlier start of the pane, never writes over it.
A holder whose own start fails after it wrote itself in records `start_failed` with the reason.
A start whose holder said `failed`, ended without a word (its report pipe closed with no line, logged as that rather than as silence), or said nothing within 10 s reads the record again under the lock: a holder that wrote itself `running` meanwhile (a late `ready`) is taken as started; a reservation whose holder is gone is removed; one whose holder still runs is ended `start_unconfirmed` and that holder ended whole outside the lock.
When the pane ends, by a terminate or by its program's exit with no node connected, the holder writes why (`terminated`, `program_exited` with the exit code, `tree_end_unconfirmed` with the pids left, `holder_failed`) before it exits.
It waits at most 5 s for the lock; an end it cannot write is logged for a person reading the log (`holder.record_unwritten`, with the reason, the incarnation, exit code and pids left), and the next reconcile settles the pane as one whose holder was lost.

A node reconciles when it starts, judging up to eight panes at once (a worker that cannot be started is logged and its share left to the others) and taking up none after 30 s (`RECONCILE_BUDGET`): a judgment under way runs to its own bound, the longest a close's at about 42 s, and what was not taken up is answered pending for the next reconcile, which the node asks for until `Reconciled::is_complete`.
On this Mac (load average 8 to 14) 128 panes whose holders were killed, with trees that ignore the hangup, were settled in 8.8 s.
A holder killed while the daemon runs is settled by that same start, which the daemon asks for rather than settling the pane a second way: an attach that cannot connect asks `Panes::holder_gone`, which reads and changes nothing and answers `true` only for a `running` record of this folder whose holder is confirmed gone (`Identity::gone`: no process under its pid, or one that started at another time; a process table that cannot be read, or a holder that is merely slow or busy, answers `false` and the attach is tried again).
The daemon then starts `hide restart --daemon <pid> --started <start>` as a resident child (`holder_restart`, logged as `daemon.restart_for_lost_holder`), naming itself by pid and start time, so the forced end `hide restart` makes of a daemon that does not stop in time never reaches it; the command ends that daemon and starts the next beside itself while the holders keep running, all under `connect.lock`, and does nothing when that daemon was stopped or replaced since the request, so a successor another `hide connect` started meanwhile is never ended.
A request is admitted at most once in ten minutes across restarts, the check and the record of it, written before the helper starts and withdrawn if the start fails, under one lock of the state folder (`holder-restart.lock`, `holder-restart`), so the panes one loss takes at once ask once; a pane lost while a request is suppressed stays unreachable until the next restart or start.
The desktop host follows the new daemon as it follows any other (`daemon.lost`, then `daemon.found`).

- A record whose holder runs, pid and start time both, is adopted; a `closing` one is asked to end again, ended by its pids when nothing answers for it at its socket (on Unix its own session and the session each of its children leads, found before the holder ends so they are not lost to reparenting, and on Windows its tree, whose job ends the rest), and stays `closing` for the next reconcile while it refuses or runs on past its bound.
- A record whose holder is gone is ended `holder_lost`; on Unix what the program's session left, found by the program's recorded pid and start time (`Session::left_by`), is ended first, and its socket and lock file are removed (the lock waited out for up to 2 s, for Windows). Leftovers it cannot end keep the record `closing` with their pids for the next reconcile.
- A `preparing` record with no holder written in is a start under way for 10 s, then ended `start_interrupted`.
- A record ended `start_unconfirmed` whose holder still runs in this boot, by pid and start time, has that holder ended whole.
- A record from another boot is ended `machine_restarted` and nothing it names is touched, whichever state folder it names as its owner: the owner is the folder's volume and number, and macOS may number a volume anew at each boot, so the record is taken as this folder's (an ended one keeps its end and takes the new owner).
- A record that cannot be read, of a format this build does not know, of another state folder in this boot, or of a protocol older than this build speaks to is left as it is and reported, never taken for no record.

The records are the only way to a holder: a node starts one only with its record's path, the holder writes itself into that record under the lock before it starts its program, and a record is removed only once nothing it names runs, so every holder a node started is named by a record.
The reconcile therefore reads the records and nothing else; a holder whose record a person deleted by hand runs on until its program ends, as a tmux session does once its socket is deleted, and the next start of its pane binds over its socket.
A terminate whose connection ends with no `Closed` (a holder closes a connection whose output it could not hand over, and a terminate it had not read goes with it), or whose connection is closed `taken_over` because another connection took the pane in the same moment (a pane's terminal attaches as its close is asked for), is sent again on a new connection while the holder runs, until the close's 20 s bound (`pane_holder.terminate_again`).
A `taken_over` close is the connection's and never the pane's: read as the end, it left a running holder behind a close the node reported as done (with-hide/hide-private#912).

A pid reused within one boot is told apart by the start time recorded with it, and a session id is not reused while a member of the session lives.
What remains is a process whose pid and start time both repeat, which the clocks make negligible within one boot (`start_time` in [The platform layer](0005-platform-layer.md#the-platform-layer)), and on Unix a program the holder had started but not yet written in when it was killed, which the kernel's hangup ends unless it ignores it.

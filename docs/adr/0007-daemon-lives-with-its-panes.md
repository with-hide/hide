# 0007. The daemon lives with its panes

- Status: Accepted
- Date: 2026-11-05
- Code: `hided/src/server.rs`, `hided/src/build_id.rs`, `hided/src/env.rs`, `hided/src/state_move.rs`

## Decision

### The daemon lives with its panes

`hided` stays up while a pane holder of its machine may still run, with or without a window or phone, so letters, agent states and labels keep going when the app is closed (PRD labels-in-hided D-18): the holders outlive the daemon by design, and the daemon is what keeps their letters and states moving.
The idle exit still counts ten minutes (`idle_secs`), but only while no client is connected and no holder pane may run; the daemon counts its machine's holder records every five seconds off the async thread (`hide_kit::panes::Panes::live`, `watch_idle` in `hided/src/server.rs`), a record it cannot read counts as running, and a count that fails keeps the daemon, since ending it would leave panes with no core.
`--keep-alive` and the paired-phone rule behave as before, and an automation that is on holds the daemon as a paired phone does ([Automations](0031-automations.md#automations)).
`SIGTERM` still ends the daemon gracefully, which cancels the running label analysis and its provider process before it exits.

A build is the SHA-256 of the `hided` executable's bytes (`hided/src/build_id.rs`), hashed once when the daemon starts and reported as `build` on `/health`.
`hide connect` hashes the `hided` shipped beside it and compares: the same build is attached to as it is, and a live daemon of another build is stopped (`SIGTERM`, five seconds, then `SIGKILL`) and replaced by a new one, so the window and its daemon are always one build after an app update (D-19).
The state file records the daemon's process start beside its pid (`pid_started`, from `hide_platform::process::start_time`), and `hide connect` and `hide stop` act on the pid only while that start still matches: a pid another process has reused, another account's included, reads as gone and is never signalled, and its stale state is cleared; a state written by an earlier build has no start and is judged by liveness alone.
Each `hide` command checks the environment keys it reads, and the sets live in code (`hided/src/env.rs`), not in the command's body.
The daemon and every command that acts on its whole configuration check every registered key and report all the invalid ones at once.
`hide stop` and `hide status --json` read only the state folder keys (`STATE_FOLDER_KEYS`: `HOME`, `HIDE_STATE_DIR`, `XDG_STATE_HOME`), and `hide status` also `HIDE_IDLE_SECS` (`STATUS_KEYS`), the idle time it prints when `/health` gives none.
An invalid key outside a command's set, such as an opener or a port, therefore never keeps a running daemon from being asked about or stopped.
An invalid key inside the set refuses with that key's name and exit status 2; it is never replaced by a default, since a default state folder would name another daemon.
`hided/tests/it/connect_stop.rs` holds each of these from the command's exit status and output.
A daemon that will not stop is a `start_failed` answer, never a silent attach to the old one, and the replacement is logged as `daemon.replacing` or `daemon.replace_failed`.
Only a desktop package's own `hide`, in macOS `Contents/Resources` or Windows/Linux `resources` (`hide_kit::bundled_kit_dir`), replaces a daemon: any other `hide`, a development build or a copy, answers `other_build` with both builds named and attaches to nothing, because the installed app's daemon is the operator's (`daemon.other_build_refused`).
Only the daemon of the state folder the CLI was given is looked at, so a development or e2e daemon elsewhere is never replaced, and stopping a daemon leaves its panes' holders and their agents running.
Each connect holds `connect.lock` in the state folder from looking at the daemon until the one it attaches to answers, so two connects never both replace it; a stop removes `hided.json` only while it still names the stopped pid and never unlinks `hided.lock`, so a daemon started since keeps its state and its instance lock.
A daemon that has ended counts as stopped before its starter reaps it, although Linux still lists an unreaped process's start time.
Before any of that, a connect whose state folder is the default `~/.hide/state` moves the legacy default `~/.local/state/hide` into place once (PRD hide-home-layout D-05; `hided/src/state_move.rs`): under the legacy folder's own `connect.lock`, which an older `hide connect` takes too, it stops the daemon the legacy `hided.json` names, then renames the whole folder in one `rename(2)`, so there is never a copy in two places.
That pid is signalled only once it is proven to be the daemon: a state that recorded its daemon's process start is proven by the kernel's answer for that pid, as `hide stop` judges it, so a daemon too busy to answer `/health` within its one-second request is still stopped and the folder still moves (issue 863); a state an earlier build wrote without a start (before v0.3.0) is proven only by `/health` answering as that pid, because liveness alone also passes a reused pid.
The folder moves only while its `hided.lock` is held across the rename: after stopping a proven daemon, the connect waits at most two seconds for that lock, because macOS can stop reporting a dying process's identity before its descriptors close.
The platform lock helper owns this bounded wait and the acquired lock; an unproven holder is still refused immediately and never signalled, and a lock that stays held answers `start_failed` without moving anything.
A legacy folder that is a link or not this account's is not moved, one other accounts can write to is refused with the `chmod go-w` that fixes it (only when it would move; beside an existing new folder it is just left), the moved folder is made 0700, `~/.hide` must be a private folder of this account, and a daemon that will not stop leaves both folders as they were and answers `start_failed`.
`HIDE_STATE_DIR` naming `~/.hide/state` itself counts as the default, since that is the value `hide connect` hands the daemon it starts.
When `~/.hide/state` already exists nothing is merged: the new folder is used, and each daemon start logs `state.legacy_left` with both paths while the legacy folder remains.
A folder `HIDE_STATE_DIR` or `XDG_STATE_HOME` chose is never moved (D-04).

With no window attached the daemon does only the work that keeps labels current (D-20, B29).
`hided` counts web and desktop renderers, another machine's view among them, tells the core `ui_attached` when the first arrives and when the last leaves (under one lock, so two windows arriving together cannot leave the core told the opposite of the final count), and refuses `ui_attached` from a client, because a window that sent it would pause the readers every other window draws ([Which connection may send what](0041-peer-connection.md#which-connection-may-send-what)).
While none is attached the session-sync coordinator rests the readers that only feed what a window draws: foreground processes, provider usage, ports, GitHub and disk, and the Background AI probe; the worktree catalog is read either way, because missions read it.
Label work, which runs on the coordinator's own wakes, goes on, and the root follower skips its snapshot reads while no client is connected and catches up with one read when the next client arrives (`RootFollower::resume`).
The readers resume on the coordinator's next wake after a window attaches.
Local hook diagnosis belongs to the existing kit pump, not the session coordinator: one startup read, another after a successful apply, and a refresh due one second after completion while a window observes Settings.
A device's hook diagnosis is read the other way round, by that device's own coordinator (`kit::DeviceDiagnosisReader`, one `BackgroundRead` worker per device, due a minute after its last answer, window or none), because the doorbell reads a pane's composer only for an agent version Hide knows and a device's panes have no other way to say theirs ([delivery.md](../delivery.md#catching-an-agents-screen-change)); the core keeps the last answer per registered device (`Runtime::ingest_device_hook_diagnosis`), drops it with the device, announces nothing, and a link that is down or a call that fails leaves the last answer in place.
The pump serializes diagnosis and installation on its existing worker, keeps one next-read deadline instead of a tick queue, and publishes only changed answers after releasing Runtime.
Core shutdown stops pump admission before closing its fixed node, rejects replies from that retiring owner and joins the pump outside Runtime.
`hided/tests/it/connect_build.rs` and `hided/tests/it/handshake.rs` (`the_daemon_outlives_the_idle_window_while_its_herdr_answers`, `the_daemon_exits_after_the_idle_window_when_its_herdr_is_gone`, `a_window_cannot_say_whether_windows_are_attached`) own these rules.

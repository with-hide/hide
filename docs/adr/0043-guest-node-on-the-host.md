# 0043. A guest node on the host

- Status: Accepted
- Date: 2026-11-19
- Code: `hided/src/guest_node/`, `hided/src/guests/`, `hided/src/peer_inbox.rs`, `hide-core/src/guest_panes.rs`

## Decision

### A guest's panes on the host

A guest's work runs on this machine in the guest's own node (`hided node serve --guest`), and this machine's daemon reaches its panes only through the tap that node serves (`hided/src/guests/tap.rs`), whether or not the guest is connected.
The tap is one local connection per guest: the node says which panes exist, each one's grid and its output, and the daemon writes back a screen's keys, pastes, sizes, redraws and a close; a paste goes as the `paste` frame only to a node that welcomed tap 2 (`hide_peer_protocol::tap::PASTE_TAP`), and to an older node, which skips a kind it does not know, as the keys it was.
A window ends one pane with the daemon-only client event `guest_pane_close`, taken from a window of this machine or the screen another machine shows of it (the `Window` class, `hided/src/server/event_class.rs`), since either already types into that pane and only a machine holding an open grant to this one draws such a screen: the daemon finds the guest by the address's device id, refuses an address that is no listed pane (`pane_unavailable`), writes the close to that guest's tap (`Guests::close_pane`), whose node ends the pane and tells the guest's engine it ended, so the guest's own tab of it closes as when its program exits, and a guest not linked then closes it from the pane's record (`HolderRecovery::closed`) when it links again or its core starts anew, never with a new shell; refuses it `node_unavailable` when that guest's node is not connected or the write fails, since the pane then still runs, and answers that window alone with `guest_pane_close_result` under the call's `request_id`, so the window moves only once the close reached the node (`web/src/guestCloseCall.ts`); a page loaded before this machine updated sends `{pane_id}` alone, and hided still closes it and answers that page as it was built to, nothing on a close and an error frame on a refusal; a guest's panes also end together with its removal ([A guest taken off the host](#a-guest-taken-off-the-host)).
The node also says the folder each pane was started in, read once from the pane's holder record, in a `folder` frame after the pane's own (`hide_peer_protocol::tap`, kind `0x93`, at most 4096 bytes); a host that predates the frame skips its kind, and the daemon keeps the folder with the pane, spelled through its links as a caller's own folder is, so the two are compared as the same path.
The daemon keeps each guest's pane list in the guest's `h/panes.json`, written whole on every change and held to 128 panes, hands output to the screens' hub under the pane's `guest:` address, and installs the tap in the terminal router (`Router::install_guest`) so a screen's keys and sizes for that address go to it.
The core holds the lists as `navigator.guest_panes`, keyed by the guest's device id with its connection key as a field and each pane with its folder once the node said it and its name, branch, mark and agent once the guest's core told them (`work_names`, [Whose work is whose between machines](0044-whose-work-between-machines.md#whose-work-is-whose-between-machines)), from the daemon-only `guest_panes` event that replaces the list whole and that the daemon refuses from a client; `hide device list` answers it and prints each pane's address under its machine, with the panes a machine left after it was taken off under its id.
The node keeps a session on every pane its holders' records say may run, so the host draws a pane whether or not the guest's core looks at it, and the guest's core's attach and release only decide whether a pane's output also goes up the node link.
A size the host's screen writes on the tap resizes the pane, as the guest's core's resize does, so a pane has one grid, the one the screen that sized it last draws at; a size taken only as the view would leave the program at the guest's grid and the node would refuse each of its frames as the wrong size, leaving the host's screen blank.
It lists each by the id its core names it by (`Holders::live_pane_ids`), never by the name its record's file has, so the host, the node's sessions and the guest's core name one pane one way.
With no link, from its start to its first one too, nothing the node reports waits for a core that is not there, and its diagnostics go to the host's log.
After its own restart the daemon lists each guest's panes again from the file and starts the node of a guest that still has panes from the build it last ran (`h/node.json`), with the guest away.
A screen of this machine draws a guest's pane and types into it by its `guest:` address, and `hide pane send` types text and Enter into it through the tap once the node is there to take them; this machine keeps no text of a guest's pane, so a read is refused as `guest_pane_read`, a named key as `guest_pane_keys` and a paste as `paste_unsupported`.

### A guest's build on the host

A guest's node runs from the guest's own build, which the host has only once the guest sent it.
The guest asks on the management stream for its node (`node_start`, naming the build by its id); a host that lacks the build answers `build_missing`, the guest sends it once on a file stream, the list of its files and then each file's bytes in that order, and asks again when the host says it is placed (`hided/src/peer_dial.rs`, `ready` and `upload`; `Guests::file`).
The host takes the bytes a piece at a time (`hide_node::ssh::host::guest_build::place`): each file is written to `<name>.part` beside its place with its hash counted as the bytes arrive, and takes its name only when they are the bytes the build names, so no file is held in memory and a part an earlier try left is replaced.
A file already there with those bytes is kept, and its bytes in the upload are read past and checked the same way, so the files after it stay where the list says.
A build is never too slow to arrive, only silent: each read of the upload has 30 s (`UPLOAD_QUIET_WITHIN`), because a relay carries about 2 MiB a second and the largest build, 512 MiB, takes four minutes.
The guest in turn waits `READY_WITHIN` for each stretch that is the host's own, from its ask and again from the build being placed, and 120 s after its last byte for the host to say so; the time the upload takes is counted by neither.
Nothing resumes: a link that drops part way costs the whole build again at the next try, on a relay against its monthly limit.
Before a build is placed the host makes room for it (`Guests::make_room`), keeping only that build and the one the guest's node last ran from, so a build that never brings a node up goes when the next arrives.
A retry of a node left stopped (`node_retry`) is a start like the first: it is counted against the guests at work and told `host_full` at the cap.

Both machines read the management stream through one reader that keeps what it has of a message when a wait on it is dropped (`hide_peer::wire::MessageReader`), because each waits on it beside something else: the host beside the node's standing and its events, the guest beside its link's close and its inbox's clock.
`wire::read_message` is for a stream nothing else is waited on with.

### A guest's hook entries

A guest's agents report to the guest's core through hook entries of the guest's own installation in the host account's agent settings (PRD D-77 (4), (5); [agent-hooks.md](../agent-hooks.md#more-than-one-hide-on-an-account)).
The guest's node writes them: it runs from the guest's install root (`<state>/g/<key>/i/<build>/hided`), names its installation by that root (`guest-root:<install root>`, `hide_kit::layout::install_of_root`), and answers its core's kit requests as a device's node does, over a target that keeps to the hook entries and plugin files (`KitPlace::Guest`, `hide_host::kit::run_guest`, `hide_kit::guest_target`); a node that runs from anywhere else installs nothing.
The entries name `<install root>/current`, which the pass points at the build that runs, so a guest's next build needs no rewrite, and a guest's panes name that installation and its own `hide` (`HIDE_INSTALL`, `HIDE_INSTALL_CLI`).
The account's `hide` command, its `PATH`, its skills and its login agents stay the host's own Hide's.
The host's Hide takes the entries out, by Hide's marker and that installation's name (`Guests::unhook`, `hide_kit::remove_guest`): when the guest's node has stopped and no pane of the guest is left, when the guest is taken off the machine, and at the daemon's start for what a node that ended without it left.
So the entries are there while the guest's node runs, which is while the guest is connected or has a pane here, not only while it has an agent's pane: an agent reads its settings when it starts, so the entries have to be there before a guest's first pane is, and the node cannot tell an agent's pane from a shell's.
What that costs is one start of the guest's helper at every agent event on the account, the host's own agents' among them: anywhere but in a pane that names the guest's installation the helper reads its environment and arguments and ends, with no file or socket touched (`hide_agent_hooks::route::foreign_pane`).
[What a guest may take](#what-a-guest-may-take) has what a start takes and the number of guests it allows.

### What a guest may take

Every bound on a guest's work is a count, and this is the whole list (PRD D-77 (7)).
Memory and CPU are not promised: a guest's node took about 40 MB and 0.5 MB more per pane, and a pane's holder 7 to 16 MB, in a debug build.

| What | Cap | At the cap | Named in |
| --- | --- | --- | --- |
| Guests at work on one host, a guest being at work while its node runs or a pane of it is left | 4 | the next guest's node is not started, by a first ask or by a retry of one left stopped, and the guest is told `host_full`, which its device shows as a reason; a guest already at work is never refused its own node | `launch::MAX_GUEST_NODES`, `Guests::within_the_cap` |
| Guests a host keeps a place for | 32 | those with no node and no pane are dropped, and past that a guest is refused | `hide_peer::limits::MAX_GRANTS`, `Guests::node_for` |
| Panes of one guest, each in one holder | 128 | the node starts no holder past it, and the host lists none | `hide_kit::panes::HOLDER_CAP`, which `hide_core::guest_panes::MAX_PANES` is |
| Folded heads of guests' work in this machine's 프로젝트 list | 256 | folding one more drops the oldest fold whose machine or folder is no longer listed, else lets the oldest fold open again | `hide_core::guest_panes::MAX_FOLDS` |
| Streams on one connection | 72, with a cap for each kind | a stream past it is refused | `hide_peer::limits::MAX_STREAMS` |
| Output a node keeps waiting for a slow host, on the tap | 4 MiB | what waits is dropped and each pane is drawn whole again | `tap::CAP_BYTES` |
| Letters a node keeps while its core is away | 4,096 events and 8 MiB | the sender is refused `outbox_full`; nothing is dropped | `event::BOX_MAX_EVENTS`, `event::BOX_MAX_BYTES` |
| Events a node hands the host before the guest's stream takes them | 64 | the node's output is left unread, which holds its next event in its box | `guests/node.rs`, `EVENTS_WAITING` |
| Crashes of a node the host starts again | 5 in 600 s | the node is left stopped and the guest's device shows the reason with a retry | `launch::CRASHES`, `launch::CRASH_WINDOW_SECS` |
| Starts of a node for a holder it lost | 1 in 600 s | a pane lost sooner stays unreachable until the node's next start; the host's own daemon never restarts for it | `launch::HOLDER_LOST_RESTART_SECS`, `launch::EXIT_RESTART` |
| One build a guest sends | 512 MiB in 64 files | the build is refused before a byte of it is placed | `build::MAX_BYTES`, `build::MAX_FILES` |
| Builds of one guest a host keeps | 2 | room is made before a build is placed: every build goes but that one and the one the node last ran from | `launch::BUILDS_KEPT`, `Guests::make_room` |

The first row is set by the hook entries.
A guest at work adds one start of its helper to every agent event on the host's account, and the guests together are to add no more than a second to one event on the slowest system Hide supports, for an agent that runs its hooks one after another.
That system is Windows, where an entry runs in PowerShell: on a GitHub Windows runner (2026-11-19, a debug build, four runs of ten starts each) a guest's entry took 225 to 290 ms at the median started as Claude Code starts it, and as Codex does 145 to 167 ms through Windows PowerShell and 263 to 275 ms through PowerShell 7, of which the shell alone took 140 and 240 ms; the first start of a new helper file took 2.6 to 2.9 s.
Four guests therefore add about one second there at the most, and one start's time for an agent that runs its hooks at once.
On a Mac a start took about 5 ms in a release build, so four guests add about 20 ms.
The `windows check` lane prints the figures on every run (`windows_hook_command::a_guests_entries_say_nothing_and_cost_one_start_each`), beside what PowerShell alone takes.

### A guest taken off the host

A guest's work on this machine follows its grant.
The pairing tells the guests' service who holds one whenever that changes and once at the daemon's start (`Guests::granted`, from `Pairing::paired_changed`), and a guest that holds none is marked as taken off in its `h/removed`, which is what the next start of the daemon goes on with.
Its node is told to end after its last pane (`ToNode::End(AfterLastPane)`): the panes stay, on this machine's screen only, and take no new link.
When the operator chose to end that work with the removal (`pairing_remove` with `end_work`, `Guests::end_work`), the node is told to end now instead, and a node that is not running is started from the build it last ran only to end the panes its holders still run; this needs no connection to the guest, because the node holds the panes' records.
A node told to end now reports each pane as gone on its tap and waits up to 2 s for the tap to take that before it exits, so the host's list does not keep panes that no longer run.
The two words come on two threads and are applied one at a time, and the grant list's word never weakens an "end now" already given.
Once a guest that was taken off has no pane and its node has stopped, its place among the guests, the screens' way to its panes and its whole `g/<key>` folder are removed; a folder of a guest with no grant that nothing was started from since the daemon's start is removed in the same sweep.
A guest that is paired again before that has its mark cleared and its node started again, since the one that was running takes no new link.
Of a guest's builds the host keeps the one that runs and the one before it ([A guest's build on the host](#a-guests-build-on-the-host)).

A holder of a guest's pane that dies without saying why is settled by that guest's node alone (PRD D-77 (6)).
A node's start ends such a pane's record as lost and wakes its agent, as a daemon's start does for its own panes, so the node ends itself with exit code 71 and the host starts it again at once, every other pane staying in its holder (`guest_node::restart_for_lost_holder`, `guests::node::follows`).
The node asks so at most once in 600 s, counted in its own folder across its starts, and the host counts each such end with the node's crashes, so a build that asks at every start is left stopped like one that crashes.

### A guest node's events while its core is away

A pane of a guest's node reaches the guest's core through the pane service of the node's link, which lives only as long as a link does.
While no link is served, the same `hide` asks the node itself on `away.sock` in the node's folder (`hided/src/guest_node/away.rs`), bound for the node's whole life: one line each way, both ends the guest's own build.
A letter (`hide request send`) from a caller this machine's kernel finds in the pane it names is kept in the node's box, with that pane's terminal, and answered `queued`; every other command needs the core's judgement and is answered `owner_offline` at once, and so is a credential the pane asks for with no link (`workspace_cli::bootstrap`).

The box is `box.jsonl` and `acked` in the node's folder (`hided/src/guest_node/event_box.rs`), in the format `hide_peer_protocol::event` names because two builds of one guest read it: the node that wrote it and the one that replaces it.
An event is appended and synced before its sender is answered, numbered from 1 with no gaps in a box named once; an event of the same kind and key that is still kept is the same event, so a send tried again adds nothing.
At 4096 events or 8 MiB a new event is refused `outbox_full` and nothing kept is dropped; one event is at most a management message.
A line is kept and sent as the text it was written as, so a field a later build added reaches the guest's core unchanged, and a whole line this build cannot read refuses the box and leaves the file as it is: the node then names no `events` capability and answers a letter `outbox_unavailable`.

The guest's core asks for the box on the management stream it holds beside its node link (`hided/src/peer_dial.rs`, `hold`): it names how far it recorded (`resume`), the host tells the node on its input (`ToNode::Resume`), and the node writes every kept event, and each new one as it is made, on its output.
The host relays each line to the stream that asked as an `event` message with the line's own fields (`GuestNode::relay`), at most 64 waiting; a stream that takes nothing for 30 s is ended and the node is told `away`, so events wait in the node's box and never in the host's memory.
The guest records each event durably (`hide_peer::inbox`), acknowledges it, and only then applies it; the acknowledgement goes back to the node, which drops what was recorded.
A core whose place the box cannot follow (it names another box, or a place the box dropped or never reached) gets what is kept in a box started fresh, numbered from 1, and the product's own keys make a repeat nothing.
One management stream per guest is open at a time: a new link ends the earlier stream on both machines before it opens the next (`Held`).
A connection has one place for a node link too, so the host lets the earlier management stream go only once the guest's node link has ended there, for at most 1 s (`LINK_ENDS_WITHIN`): a link opened the moment the one before it closed finds both places free, where it was once refused `limit` (seen in CI on Linux, run 44527417085).

This build keeps one kind of event, `letter`, and applies it as the command its sender would have run had the core been there (`hided/src/peer_inbox.rs`), under the letter's intent, which makes a second apply the same letter.
The node proves the sender when the letter is written (`hide_host::panes::prove_caller`, the check a pane's credential gets) and records the terminal it ran in, which names one life of one pane; which agent that is, is the core's own knowledge (`Runtime::prepare_kept_letter`): the agent it observes in that terminal, else the one it last registered there.
So a letter is applied as its agent whether or not the pane still runs, and a worker's report written before the host closed its pane reaches its parent; nothing of the caller is checked again when it applies (`Authority::proven`, `Proven::WhenWritten`), because that was settled when it was written.
A letter is given up, and logged as unappliable, only when what it names is gone: its recipient, or a sender the core registered in no terminal and whose pane it does not find within 60 s of the link's return; a core that cannot write a letter just then is asked again for that long.
A pane's end and an agent's completion are not events: they reach the core as the node's state when its link is read again, which is the same however often it is read.

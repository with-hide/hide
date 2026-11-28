# 0036. Node and device identity

- Status: Accepted
- Date: 2026-10-07
- Code: `hide-core/src/node.rs`, `hide-core/src/runtime/device_names.rs`, `hide-node/src/opener.rs`, `scripts/check-core-touches-no-machine.py`

## Decision

### The core's node id

The core names its own machine by a node id, never by the literal `local` (PRD core-host-node D-21; `hide-core/src/node.rs`).
The id is the machine's own id, `hide_platform::host::machine_id`, lowercased and checked to `[0-9a-z-]{1,64}`, read once when `hided` starts (`NodeId::of_this_machine`) and handed to the core in `CoreOptions::node_id`.
It is the id of the navigator's own device row, whose `kind: "local"` stays as its role, and the `device_id` of this machine's registrations, Workspaces, letters, labels and Workspace views, the device half of every project id (`hide_project::project_id`), and the device half of a folder project's owner mark.
An event or a `hide` command that names no device still means the core's own node, so the wire keeps its shape and only the value of this machine's `device_id` changed; a `device_id` of `local` now names an unknown device, and no device is ever registered as `local` or as the node's own id ([A device's id](#a-devices-id)).
The web shell reads the id from the navigator's own device row (`localDeviceId` in `web/src/snapshot.ts`); before the first snapshot names it the id is empty, which an event carries as no `device_id`.
A device's ids are scoped to it as `remote:<device>:<kind>:<id>`, while the core's own node keeps the engine's bare ids, and one module builds and reads that form: `hide_node_link::address`, re-exported as `hide_core::address`, which `hide-node`'s router, `hided` and the core all call and nothing else formats or splits.
It lives in `hide-node-link` because the router cannot depend on the core.
A pane is a `PaneAddress`, either a pane of the core's own node or a device's pane by the device's own id; a typed `remote:<device>:pane:<id>` from a `hide` command parses into the same type, and a `remote:` id that names no pane is refused rather than read as a local pane.
Three spellings name a pane that involves another machine, and they are three different things:

| Address | What it names | Who answers |
|---|---|---|
| `remote:<device>:pane:<id>` | A pane on that machine that this core runs: its engine made it, in this core's node there | This core, through its node link |
| `guest:<device>:pane:<id>` | A pane on this machine that the other machine's core runs: that core's work, in its guest node here ([A guest's panes on the host](0043-guest-node-on-the-host.md#a-guests-panes-on-the-host)) | This machine's daemon, through the guest node's tap, as the host's own view of work running here, which its operator may see, type into and close |
| `core:<device>:pane:<id>` | A pane the other machine's core owns, asked of that core over a grant | The other machine's core, which decides |

`guest:` is the third variant of `PaneAddress` (`PaneAddress::Guest`), read and written only in `hide_node_link::address`; it is not `remote:` because two machines that open each other have both, and each names its panes without the other, so the same `<id>` can be either.
A guest's pane is no pane of this core's engine, so everything that takes a pane id to the engine reads it as a pane the engine does not have.
Two pane ids are the same pane only on the same device (`address::same_pane`), so one raw pane id on two machines is two panes.
An answer hands a caller each pane in the form that reaches it from where the caller runs, never a machine and a raw id to join: `address::pane_for_caller` names a pane bare on the caller's own machine, `remote:` on any other, and `core:<this machine>:pane:<id>` for an agent of another core that asks this one (`address::Reader`), and `address::named_pane_id` reads each back, `remote:<node>:pane:<id>` as the node's own pane ([delivery.md](../delivery.md)).
Each agent and pane row publishes its machine as `device_id` and the pane's id on that machine as `device_pane_id`, and each editor tab its machine as `device_id`; a raised ask names the device its pane is on as `open_device_id`, because a disconnected device publishes no rows to find it by.
These are derived fields, written by `Runtime::stamp_row_addresses` as a snapshot is taken and never compared (`model::Derived`), so a stamp is never a change and a projection that has not stamped yet is equal to one that has.
The web reads them and never reconstructs a machine from an id; every reader that names a row's session (a pane head's record button, a 태스크 row, `revealTask`) finds the device on the row, because the session's ledger row is kept under the device id and a session named with the core's own node id for a device's pane finds no record.
The desktop host learns it from the shell's browser sync (`node`) and keys the shared loopback partition and the pages it loads directly on it.
A workspace whose `hide_owner` mark was written for `local` is still read as this node's folder (`OwnerOpen::on_node`), so a folder project opened before the change keeps its Workspace.

A state folder written under `local` is converted before the core starts (`hide_core::node_migration::convert`, called from `hided`'s `start_daemon` after it holds the instance lock).
`core-state.json`, `workspace-views.json` and `labels.json` are rewritten whole through a temporary file and a rename, and the delivery ledger's rows in its store (`delivery.sqlite3`), into which a folder's `delivery-ledger.json` is imported first, as the core's start would import it; that old file is only read, so the build before node ids still finds its own ledger ([delivery.md](../delivery.md#persistence-clocks-and-limits)); Project Memory and the link record (`links.sqlite3`) re-key their project ids, and the link record its device names and records each replaced project id for the old session search file's retention (`retention_aliases`), in one SQLite transaction each; the old session search file is never written.
The originals are copied under `node-migration-backup/<unix ms>/` first; a start that resumes an unfinished conversion keeps them in that same folder and copies each store once, so retries add no copies, and recording the same replaced ids again changes nothing.
`node.json` is written last, so a start interrupted before it converts again, and a start of an already converted folder writes nothing.
A store that cannot be converted stops the daemon with the file and the reason, written to the Logs file at once with the daemon's pid; `hide connect`, which sees its daemon exit before answering, reads that record and answers `state_refused` with the file, which the desktop host's failure page shows under the reason. A JSON file its own loader cannot read is left to that loader, which already reports it.
A store whose loader has a size limit (the delivery ledger's 16 MiB) is checked before anything is written: one the longer node id would push past that limit stops the start with every file as it was, and one already past it is left to its loader.
The project pairs the session search index is re-keyed with come from the node's Projects in Project Memory and the folders registered under either name, so a start retried after a later store failed still finds them; a row whose new key is already taken is a stale copy and is dropped and counted.
`node.json` records the node that owns every unqualified key in the folder (a pane id, a tab id, a checkout path), whichever machine runs the core.
A folder that names another node is refused today only because nothing yet reads another node's state; a core that does extends this rule without changing the file.
`KEYS` in `node_migration.rs` lists every machine-bound key a store written before this change can hold, with how it is covered, rewritten or owned through `node.json`; that set is closed, since a store written since names the node id or holds unqualified keys `node.json` covers.
Its contract test holds a value at every key the table names and fails when a rewritten key still names `local`, when an owned key changes, or when a converted file still carries `"local"` anywhere.
`UNBOUND` beside it names the rest of a state folder, which holds no machine key, and a test fails on any file in a converted folder that neither list names.
Project ids hash the node id with the root, so a Memory receipt a transcript carries from before the conversion no longer matches a project id and is not honoured.

### A device's id

A machine has one id wherever it is seen: its machine id, which is its node id when it runs a core and its device id when a core dials it (with-hide/hide-private#671; `hide-core/src/runtime/device_names.rs`).
Every key that names a device is derived from that id (its projects' digests, its panes' and tabs' `remote:<device>:` addresses, its letters, its Memory Projects, its engine folder), so the same machine's records keep one name when it changes role.
`register_device` takes no id: an SSH device is registered under an id its alias makes (`device_command::device_id_for`, the one fold of an alias) and marked `naming: "pending"` (`model::DeviceNaming`).
Two readings name the machine on every connection: the start script's probe, read on the device before anything is installed and bound into the consent (`HostIdentity::machine_id`), and the node's Hello (`MachineIdentity`), both from `hide_platform::host::machine_id`.
A Pending device's first connection checks that both readings agree, that the id is a node id, that it is not the core's own machine and that no other registration has it or reaches it, then closes the link and renames the registration and every choice the core state saves of it to that id (`naming: "named"`); one already under its machine id is named in place.
The new id connects only after a save of the core state that holds it has succeeded (`acknowledge_device_naming`, in both save paths), so a core that stops before the save finds the Pending registration and names it again, and nothing is ever written under an id the file does not hold; Retry asks for the save again.
A Named device opens only when this connection's probe reads its id and its node reports the same, or reports none because it could not read it; anything else closes the link with `EstablishError::MachineName`, which is not retried, since the alias now reaches another machine and only removing it and adding the new one changes that.
`--machine`, `hide device remove` and `hide device retry` take a device's id or the SSH alias it was added with (`Runtime::machine_named`): the alias stays the name people use, and an alias two registrations share is refused with their ids.
WSL distributions keep `wsl-<distribution>-<digest>`: a distribution runs no core, distributions imported from one tarball share `/etc/machine-id`, and the consent names a distribution by its install id rather than a machine id ([What each system supports](../INSTALL.md#what-each-system-supports)).
A node that dials the core is already registered by its node id.

A device an earlier build registered under its alias is renamed once, at the start, after the node conversion and before the core is created (`node_migration::name_devices`, called from `start_core_role`).
Its machine id is the one its consent bound; a registration whose consent bound none connects once, binds it (`host.consent_machine_bound`), is held with "Restart Hide to finish naming this device", and is renamed by the next start.
Each device is checked against every store before anything is written: its machine id is an id, not the core's and not another registration's, no other registration reached the same machine, Project Memory holds no Project of its roots under the new id (Memory is re-keyed strictly, so card numbers stay), the link record holds no rows under it, its engine folder does not exist under both ids, every registered project's root is known to some store, and every JSON store renames to one its loader reads with nothing left naming the old id.
A device a check refuses keeps everything under its old id and is held unconnected, with the reason in the start's diagnostic record (`node_migration.devices_held`), so nothing new is ever written under the old id.
The plan is written to `device-ids.json` first, then Project Memory, the link record (with the retention the old session search file keeps under each old Project id), the delivery ledger, labels, Workspace views, missions, Local issues, composer marks and hook facts, the engine folders under `engine/devices/`, and `core-state.json` last; then the plan is removed.
Each store's original is copied once to `node-migration-backup/device-ids-<unix ms>/`, every step leaves what it already renamed as it is, and a start that finds the plan resumes it to the same result; a write that fails stops the start and names the file, as the node conversion does.
`name_devices` maps a store through the `KEYS` rows (`node_migration::id_map`'s `map_id`, `apply_pattern`, `map_views`, `map_project_settings`), so a key added to the table is renamed with the rest.

### The core and its nodes

The core decides and keeps state; it never touches a machine (PRD core-host-node D-21).
Every file, process, socket and session-file read is a node's, asked through one contract, `NodeLink` (`hide-node-link`), addressed by node id (`Runtime::node_link`): the core's own node answers today, and a device's helper answers for its machine.
The contract crate sits below both sides, so the core holds only the trait and the wire types, and the machine side (`hide-node`, over `hide-host`) never depends on the core.
The core's own node is `hide_node::Local`, which answers in hided's process through `hide_host::serve`; the core gets it in `Core::create`.
The node answers `PaneInspect` from the pane's holder record; local and linked nodes read the same `hide_host::pane_peer` terminal id, shell pid and kernel start proof on the machine that owns the pane, without accepting a caller-selected socket.
Closing it refuses new calls and wakes pending callers with `Unknown` through their existing one-answer channels; it neither polls nor starts a cancellation thread.
There are at most 64 admitted machine calls, and a timed-out or closed caller does not free its slot until the actual work ends.
The weak answer-sender list is pruned on admission and bounded by those same slots; closing one node affects its clones, never another node.

| The core's | The node's |
| --- | --- |
| Every decision, the runtime and its snapshot | Files and folders: lists, bytes, saves, stamps, path facts (`Call::PathFacts`), new projects (`Call::ProjectCreate`), the project guardrails' policy file (`Call::GuardrailPolicy`) |
| Its own stores: UI state, saved Views, the delivery ledger, labels, the link record, local issues, saved GitHub answers, its log, and their one-time node-id conversion | Processes: git, `gh`, the AI CLIs, ports, agent sleep's process groups, the file opener (`hide-node/src/opener.rs`) |
| Which node a call goes to | Session files, the install kit, pane credential proofs and the bootstrap socket (`hide-node/src/pane_proof.rs` here, `hide-host/src/panes.rs` on a device) |

`scripts/check-core-touches-no-machine.py` holds the line: production code under `hide-core/src`, that opens a file, starts a process, opens a socket or reads a session file fails it, unless the file is one of the core's stores, listed with its reason, or something a later layer of the PRD moves out, listed with that layer; each listed file carries how many machine touches it keeps, so a new one in a listed file fails too.
The SSH transport is the node's (`hide-node/src/ssh/`): the system `ssh`, which a node starts and owns.
The core reaches a device only through `hide_node_link::device::{DeviceConnector, DialedTransport}` for a device it dials and `DeviceTransport` with an `Arrived` node for a node that dials it (`hided` builds the connector and hands over each arrival, `Core::create` takes the connector) and keeps the decoding of what a device's engine sends (`remote.rs`).
No later layer is listed any more: layer 3 moved the terminal attach child and the clipboard image's removal to the node, and layer 4 moved the label generator's lock (`Call::LabelLock`); an entry that no longer reaches a machine fails the check, so none outlives its layer.

Every `git` a node runs starts at `hide_host::git_run` (PRD git-freshness D-11), because nothing Hide runs has a person at a terminal: `GIT_TERMINAL_PROMPT=0`, empty `GIT_ASKPASS` and `SSH_ASKPASS` with `SSH_ASKPASS_REQUIRE=never`, `GCM_INTERACTIVE=never` with `-c credential.interactive=never`, `--no-optional-locks` and a closed stdin, the same on macOS, Linux and Windows.
A run that reaches a remote (a fetch, a clone) starts through `git_run::remote`, which keeps the operator's own ssh in Git's order (`GIT_SSH_COMMAND`, then `core.sshCommand` as `git config` reads it in that repository, then `GIT_SSH`, then `ssh`) and, when that command is `ssh`, adds `BatchMode=yes`, a connect timeout and keepalives after the operator's own options, so their key and options stand and an option they set wins; a wrapper or `plink` runs as configured, with nothing to answer a prompt.
`git_run::output` ends the whole process tree at its deadline (15 s for a read, 10 minutes for a change such as a checkout, a fetch's own bound) and refuses an answer past its output cap (256 MiB unless the caller names less); a clone keeps its own streaming run and stall clock but starts from the same command.
A credential helper still answers from what it already holds, which is how a background fetch uses the operator's login without asking.
`git_run`'s own test fails when product code in `hide-host` spells a Git process any other way.

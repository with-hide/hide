# Runtime architecture

The architecture is kept as decision records under [docs/adr/](adr/), one file per decision; [0001](adr/0001-record-architecture-decisions.md) says how a record is written, replaced and checked.
Read this table, then the records whose Code paths cover the files you change.
The headings after the table keep every section name and anchor this file had, so a reference such as `docs/ARCHITECTURE.md, The pane holder` still lands here and names its record.

| Record | Code |
| --- | --- |
| [0001. Record architecture decisions](adr/0001-record-architecture-decisions.md) | `docs/adr/`, `scripts/check-adr.py` |
| [0002. The core, the shell and the engine](adr/0002-core-shell-and-engine.md) | `hide-core/src/runtime.rs`, `hide-core/src/handle.rs`, `hide-core/src/engine/contract.rs`, `hide-core/src/session_sync/` |
| [0003. State placement and publication](adr/0003-state-placement-and-publication.md) | `hide-core/src/persistence.rs`, `hide-core/src/reader.rs`, `hide-core/src/input_gate.rs`, `hide-core/src/sidebar.rs`, `hide-core/src/view_feedback.rs`, `hide-core/src/runtime/view_feedback.rs`, `web/src/components/elapsed.tsx` |
| [0004. Drawn ahead of the engine](adr/0004-drawn-ahead-of-the-engine.md) | `hide-core/src/runtime/drawn_ahead.rs`, `hide-core/src/runtime/pane_prediction.rs`, `hided/src/pane_sizes.rs`, `hide-node/src/terminal/input.rs`, `web/src/keyTarget.ts` |
| [0005. The platform layer](adr/0005-platform-layer.md) | `hide-platform/`, `desktop/scripts/app-identity.mjs` |
| [0006. Resident process ownership](adr/0006-resident-process-ownership.md) | `desktop/src/main/spawn.ts`, `desktop/src/main/host.ts`, `hided/src/supervisor.rs`, `hide-node/src/terminal/holder.rs`, `hide-node/src/ssh/host.rs`, `hide-host/src/serve.rs` |
| [0007. The daemon lives with its panes](adr/0007-daemon-lives-with-its-panes.md) | `hided/src/server.rs`, `hided/src/build_id.rs`, `hided/src/env.rs`, `hided/src/state_move.rs` |
| [0008. Updating the core](adr/0008-updating-the-core.md) | `hided/src/core_update.rs`, `hided/src/node_role.rs`, `hided/src/started_cores.rs` |
| [0009. hided and the WebSocket boundary](adr/0009-hided-websocket-boundary.md) | `hided/src/server.rs`, `hided/src/server/`, `hided/src/node_panes.rs`, `hided/src/pane_auth.rs`, `contracts/hided-ws.schema.json` |
| [0010. The pane holder](adr/0010-pane-holder.md) | `hide-holder/`, `hide-holder-protocol/`, `vendor/portable-pty/` |
| [0011. Pane records and reconcile](adr/0011-pane-records-and-reconcile.md) | `hide-kit/src/panes.rs`, `hide-kit/src/panes/`, `hide-holder/src/record.rs` |
| [0012. The node's holder panes](adr/0012-node-holder-panes.md) | `hide-node/src/terminal/holder.rs`, `hide-node/src/terminal/holder/` |
| [0013. The core's own engine](adr/0013-core-own-engine.md) | `hide-core/src/engine/own/`, `hide-node/src/terminal/engine_panes.rs`, `hide-node-link/src/holders.rs` |
| [0014. The terminal path](adr/0014-terminal-path.md) | `hide-node/src/terminal/`, `hided/src/terminal_hub.rs` |
| [0015. Moving a live pane](adr/0015-moving-a-live-pane.md) | `hide-core/src/runtime/pane_move.rs`, `hide-core/src/runtime/pane_move_preview.rs` |
| [0016. Explicit terminal attachments](adr/0016-explicit-terminal-attachments.md) | `hide-core/src/runtime/attachments.rs`, `hide-host/src/attachments.rs`, `hided/src/attachments.rs`, `hide-node-link/src/attachments.rs`, `web/src/attachments.ts` |
| [0017. Agent adapters](adr/0017-agent-adapters.md) | `hide-agent-adapter/`, `contracts/agent-adapters.json`, `hide-session/src/thread/`, `web/src/agentAdapters.ts` |
| [0018. Agent definition files](adr/0018-agent-definition-files.md) | `hide-agent-adapter/agents/`, `hide-agent-adapter/src/definition/`, `hide-agent-adapter/build.rs`, `hide-screen-rules/`, `scripts/check-agent-screen-rules.py` |
| [0019. Agent profiles and accounts](adr/0019-agent-profiles-and-accounts.md) | `hide-agent-adapter/src/account.rs`, `hide-core/src/agent_profiles.rs`, `hide-core/src/agent_profiles/`, `hide-node-link/src/profiles.rs`, `hide-host/src/profiles.rs`, `hide-core/src/launch.rs`, `scripts/check-agent-folders.sh` |
| [0020. Agent sleep](adr/0020-agent-sleep.md) | `hide-core/src/agent_sleep.rs`, `hide-core/src/agent_sleep/`, `hide-core/src/agent_sleep_engine.rs`, `hide-core/src/runtime/agent_sleep.rs`, `hide-core/src/memory_pressure.rs` |
| [0021. Agent labels in the core](adr/0021-agent-labels.md) | `hide-core/src/labels/`, `hide-session/src/label_transcript.rs` |
| [0022. Local agent delivery and inactivity watches](adr/0022-local-agent-delivery.md) | `hide-core/src/delivery/`, `hide-core/src/runtime/delivery.rs` |
| [0023. Bots](adr/0023-bots.md) | `hide-core/src/access.rs`, `hided/src/access.rs`, `hided/src/access_cli.rs`, `hided/src/access_screen.rs`, `web/src/toolCells.ts` |
| [0024. Session archive](adr/0024-session-archive.md) | `hide-core/src/session_archive.rs`, `hide-core/src/runtime/session_archive.rs` |
| [0025. Session plans](adr/0025-session-plans.md) | `hide-core/src/plans.rs`, `hide-core/src/dag.rs`, `hide-core/src/runtime/plans.rs` |
| [0026. Session threads](adr/0026-session-threads.md) | `hide-core/src/runtime/session_thread.rs`, `hide-core/src/input_queue.rs`, `hide-core/src/runtime/input_queue.rs`, `hide-core/src/operator_input.rs` |
| [0027. The link record](adr/0027-link-record.md) | `hide-core/src/links/`, `hide-core/src/runtime/links.rs`, `hide-session/src/links.rs`, `hide-session/src/named_prs.rs`, `hided/src/session_cli.rs` |
| [0028. Usage accounting](adr/0028-usage-accounting.md) | `hide-session/src/usage/`, `hide-core/src/usage_ledger/`, `hide-core/src/runtime/usage_ledger.rs`, `hide-core/src/agent_state/tally/project_status.rs` |
| [0029. Reopening locally closed work](adr/0029-reopening-closed-work.md) | `hide-core/src/runtime/closed_registrations.rs`, `hide-core/src/runtime/tree_close.rs`, `hide-core/src/runtime/tree_reopen.rs`, `hide-core/src/recent_closed.rs` |
| [0030. Conductor missions](adr/0030-conductor-missions.md) | `hide-core/src/missions.rs`, `hide-core/src/mission_store.rs`, `hide-core/src/runtime/missions.rs`, `web/src/MissionScreen.tsx` |
| [0031. Automations](adr/0031-automations.md) | `hide-core/src/automations.rs`, `hide-core/src/automations/`, `hide-core/src/runtime/automations.rs`, `hided/src/automation_cli.rs` |
| [0032. Desk](adr/0032-desk.md) | `hide-core/src/desk.rs`, `hide-core/src/runtime/desk.rs`, `hide-core/src/runtime/desk/`, `hide-desk/`, `hided/src/desk_host.rs` |
| [0033. Checkout owner workspace](adr/0033-checkout-owner-workspace.md) | `hide-core/src/checkout_owner.rs`, `hide-core/src/checkout_owner/`, `hided/src/hook_request.rs`, `hide-core/src/agent_hooks/facts.rs` |
| [0034. Stored primary checkout](adr/0034-stored-primary-checkout.md) | `hide-core/src/runtime/projects.rs`, `web/src/snapshot.ts` |
| [0035. Code hosts](adr/0035-code-hosts.md) | `hide-core/src/code_host.rs`, `hide-core/src/code_host/`, `hide-core/src/github/`, `hide-core/src/gitlab/`, `scripts/check-code-host-seam.py` |
| [0036. Node and device identity](adr/0036-node-and-device-identity.md) | `hide-core/src/node.rs`, `hide-core/src/runtime/device_names.rs`, `hide-node/src/opener.rs`, `scripts/check-core-touches-no-machine.py` |
| [0037. A device's ssh](adr/0037-device-ssh.md) | `hide-node/src/ssh/device.rs`, `hide-node/src/ssh/openssh.rs`, `hide-node/src/ssh/askpass.rs`, `hide-node/src/ssh/bootstrap.rs`, `hide-core/src/runtime/device_sign_in.rs` |
| [0038. WSL distributions as devices](adr/0038-wsl-devices.md) | `hide-node/src/ssh/wsl.rs` |
| [0039. A core on another machine](adr/0039-core-on-another-machine.md) | `hided/src/placement.rs`, `hided/src/attach.rs`, `hided/src/node_role.rs`, `hided/src/screen_sign_in.rs`, `web/src/coreLink.ts` |
| [0040. Splitting a moved core's two machines](adr/0040-splitting-a-moved-core.md) | `hided/src/moved_core.rs`, `hided/src/node_role.rs` |
| [0041. The peer connection](adr/0041-peer-connection.md) | `hide-peer-protocol/`, `hide-peer/`, `hide-peer-iroh/`, `hided/src/peer_host.rs`, `hided/src/peer_dial.rs`, `hided/src/peer_core.rs`, `hided/src/pairing.rs`, `contracts/peer-link.json` |
| [0042. Another machine's screen](adr/0042-another-machines-screen.md) | `hided/src/remote_screen.rs`, `hided/src/screen_net.rs`, `desktop/src/main/remoteScreens.ts`, `desktop/src/preload/remote.ts` |
| [0043. A guest node on the host](adr/0043-guest-node-on-the-host.md) | `hided/src/guest_node/`, `hided/src/guests/`, `hided/src/peer_inbox.rs`, `hide-core/src/guest_panes.rs` |
| [0044. Whose work is whose between machines](adr/0044-whose-work-between-machines.md) | `hide-core/src/whose_work.rs`, `hide-core/src/peer_work.rs`, `hided/src/work_names.rs`, `hided/src/desktop_notices.rs` |
| [0045. Switching a helper device to its machine's own Hide](adr/0045-helper-to-own-hide.md) | `hide-core/src/runtime/device_switch.rs`, `hided/src/core_install/`, `web/src/settings/DeviceSwitch.tsx` |
| [0046. Device catalogs](adr/0046-device-catalogs.md) | `hide-core/src/device_catalog.rs`, `hide-core/src/runtime/device_catalog.rs` |
| [0047. Device file hosts and document saves](adr/0047-device-file-hosts.md) | `hide-host/`, `hide-core/src/node_access.rs`, `hide-core/src/files.rs`, `hide-core/src/runtime/documents.rs` |
| [0048. Panes, tabs and the attach window in the web shell](adr/0048-web-panes-tabs-attach.md) | `web/src/terminals.ts`, `web/src/PaneGrid.tsx`, `web/src/terminalView.ts`, `web/src/ws.ts` |
| [0049. Settings and workspace management in the web shell](adr/0049-web-settings-workspace-mgmt.md) | `web/src/SettingsSheet.tsx`, `web/src/settings/`, `web/src/settings.ts`, `hide-core/src/runtime/guardrails.rs`, `hide-core/src/project_settings.rs` |
| [0050. Workspaces in the web shell](adr/0050-web-workspaces.md) | `web/src/WorkspaceScreen.tsx`, `web/src/workspace.ts`, `web/src/Overview.tsx`, `hide-core/src/tasks.rs` |
| [0051. The shortcut registry](adr/0051-shortcut-registry.md) | `web/src/shortcuts.ts`, `web/src/keyboard.ts`, `desktop/src/main/menu.ts` |
| [0052. The `$HOME` filesystem boundary](adr/0052-home-boundary.md) | `hided/src/boundary.rs` |
| [0053. The checkout-root boundary and the Explorer frames](adr/0053-checkout-root-boundary.md) | `hided/src/index.rs`, `hided/src/watch.rs`, `hided/src/spawn.rs`, `web/src/watch.ts`, `web/src/search.ts` |
| [0054. The Explorer, editor and viewers](adr/0054-explorer-editor-viewers.md) | `web/src/explorer.ts`, `web/src/ExplorerTree.tsx`, `web/src/editor/` |
| [0055. Retiring Home](adr/0055-retiring-home.md) | `hide-host/src/home.rs`, `hide-core/src/runtime/home_retire.rs` |
| [0056. The desktop host](adr/0056-desktop-host.md) | `desktop/src/main/`, `desktop/src/preload/`, `desktop/static/` |
| [0057. The install kit](adr/0057-install-kit.md) | `hide-kit/`, `hide-host/src/kit.rs`, `hide-core/src/kit.rs`, `hide-core/src/runtime/kit.rs` |
| [0058. Herdr an earlier Hide left on a machine](adr/0058-herdr-left-behind.md) | `hide-host/src/kit.rs`, `hide-core/src/herdr_import.rs` |
| [0059. The mobile companion](adr/0059-mobile-companion.md) | `hided/src/mobile/`, `web/src/mobile/`, `hide-relay-protocol/` |
| [0060. The iOS app host](adr/0060-ios-app-host.md) | `mobile/ios/`, `mobile/scripts/`, `web/src/mobile/appHost.ts` |
| [0061. The Android app host](adr/0061-android-app-host.md) | `mobile/android/`, `mobile/scripts/` |
| [0062. Product usage statistics](adr/0062-product-usage-statistics.md) | `hide-core/src/product_analytics.rs`, `hided/src/product_analytics.rs`, `site/public/privacy.html` |

## Product usage statistics

Moved to [ADR 0062](adr/0062-product-usage-statistics.md#product-usage-statistics).

## State placement and publication

Moved to [ADR 0003](adr/0003-state-placement-and-publication.md#state-placement-and-publication).

### Sidebar tab names

Moved to [ADR 0003](adr/0003-state-placement-and-publication.md#sidebar-tab-names).

### View openers and comments

Moved to [ADR 0003](adr/0003-state-placement-and-publication.md#view-openers-and-comments).

## The core, the shell and Herdr

Moved to [ADR 0002](adr/0002-core-shell-and-engine.md#the-core-the-shell-and-herdr).

## Code hosts

Moved to [ADR 0035](adr/0035-code-hosts.md#code-hosts).

## Stored primary checkout

Moved to [ADR 0034](adr/0034-stored-primary-checkout.md#stored-primary-checkout).

## Project sessions and Memory

Moved to a private document: `docs/project-sessions-and-memory.md` in with-hide/hide-private.

## Moving a live pane

Moved to [ADR 0015](adr/0015-moving-a-live-pane.md#moving-a-live-pane).

## Drawn ahead of the engine

Moved to [ADR 0004](adr/0004-drawn-ahead-of-the-engine.md#drawn-ahead-of-the-engine).

## Checkout owner workspace

Moved to [ADR 0033](adr/0033-checkout-owner-workspace.md#checkout-owner-workspace).

## Local agent delivery and inactivity watches

Moved to [ADR 0022](adr/0022-local-agent-delivery.md#local-agent-delivery-and-inactivity-watches).

## Explicit terminal attachments

Moved to [ADR 0016](adr/0016-explicit-terminal-attachments.md#explicit-terminal-attachments).

## Reopening locally closed work

Moved to [ADR 0029](adr/0029-reopening-closed-work.md#reopening-locally-closed-work).

## Agent sleep

Moved to [ADR 0020](adr/0020-agent-sleep.md#agent-sleep).

### Sleep under memory pressure

Moved to [ADR 0020](adr/0020-agent-sleep.md#sleep-under-memory-pressure).

### Sleep by mission, letter and command

Moved to [ADR 0020](adr/0020-agent-sleep.md#sleep-by-mission-letter-and-command).

## Session archive

Moved to [ADR 0024](adr/0024-session-archive.md#session-archive).

## Conductor missions

Moved to [ADR 0030](adr/0030-conductor-missions.md#conductor-missions).

## Automations

Moved to [ADR 0031](adr/0031-automations.md#automations).

## Session plans

Moved to [ADR 0025](adr/0025-session-plans.md#session-plans).

## Session threads

Moved to [ADR 0026](adr/0026-session-threads.md#session-threads).

### A session's model and effort

Moved to [ADR 0026](adr/0026-session-threads.md#a-sessions-model-and-effort).

### The operator's waiting line

Moved to [ADR 0026](adr/0026-session-threads.md#the-operators-waiting-line).

## The link record

Moved to [ADR 0027](adr/0027-link-record.md#the-link-record).

### Session work records

Moved to [ADR 0027](adr/0027-link-record.md#session-work-records).

## Historical usage accounting

Moved to [ADR 0028](adr/0028-usage-accounting.md#historical-usage-accounting).

### Project status and cost

Moved to [ADR 0028](adr/0028-usage-accounting.md#project-status-and-cost).

## Agent adapters

Moved to [ADR 0017](adr/0017-agent-adapters.md#agent-adapters).

### Agent definition files

Moved to [ADR 0018](adr/0018-agent-definition-files.md#agent-definition-files).

### Agent profiles and accounts

Moved to [ADR 0019](adr/0019-agent-profiles-and-accounts.md#agent-profiles-and-accounts).

#### `[account]` in a definition file

Moved to [ADR 0019](adr/0019-agent-profiles-and-accounts.md#account-in-a-definition-file).

#### `resolve`

Moved to [ADR 0019](adr/0019-agent-profiles-and-accounts.md#resolve).

#### Who does what

Moved to [ADR 0019](adr/0019-agent-profiles-and-accounts.md#who-does-what).

#### The node's profile file

Moved to [ADR 0019](adr/0019-agent-profiles-and-accounts.md#the-nodes-profile-file).

#### Changing a machine's profiles

Moved to [ADR 0019](adr/0019-agent-profiles-and-accounts.md#changing-a-machines-profiles).

#### Starting an agent with a profile

Moved to [ADR 0019](adr/0019-agent-profiles-and-accounts.md#starting-an-agent-with-a-profile).

#### A profile account's usage, sign-in and the account a hook reports

Moved to [ADR 0019](adr/0019-agent-profiles-and-accounts.md#a-profile-accounts-usage-sign-in-and-the-account-a-hook-reports).

## The platform layer

Moved to [ADR 0005](adr/0005-platform-layer.md#the-platform-layer).

## Resident process ownership

Moved to [ADR 0006](adr/0006-resident-process-ownership.md#resident-process-ownership).

## The pane holder

Moved to [ADR 0010](adr/0010-pane-holder.md#the-pane-holder).

### Pane records and reconcile

Moved to [ADR 0011](adr/0011-pane-records-and-reconcile.md#pane-records-and-reconcile).

### The node's holder panes

Moved to [ADR 0012](adr/0012-node-holder-panes.md#the-nodes-holder-panes).

### The core's own engine

Moved to [ADR 0013](adr/0013-core-own-engine.md#the-cores-own-engine).

#### A device on the core's own engine

Moved to [ADR 0013](adr/0013-core-own-engine.md#a-device-on-the-cores-own-engine).

#### Agents on the core's own engine

Moved to [ADR 0013](adr/0013-core-own-engine.md#agents-on-the-cores-own-engine).

#### Importing a Herdr session into the core's own engine

Moved to [ADR 0013](adr/0013-core-own-engine.md#importing-a-herdr-session-into-the-cores-own-engine).

## Desk

Moved to [ADR 0032](adr/0032-desk.md#desk).

### Desk's commands

Moved to [ADR 0032](adr/0032-desk.md#desks-commands).

## Bots

Moved to [ADR 0023](adr/0023-bots.md#bots).

## Herdr an earlier Hide left on a machine

Moved to [ADR 0058](adr/0058-herdr-left-behind.md#herdr-an-earlier-hide-left-on-a-machine).

## hided and the WebSocket boundary

Moved to [ADR 0009](adr/0009-hided-websocket-boundary.md#hided-and-the-websocket-boundary).

### The daemon lives with its panes

Moved to [ADR 0007](adr/0007-daemon-lives-with-its-panes.md#the-daemon-lives-with-its-panes).

### The core's node id

Moved to [ADR 0036](adr/0036-node-and-device-identity.md#the-cores-node-id).

### A device's id

Moved to [ADR 0036](adr/0036-node-and-device-identity.md#a-devices-id).

### The core and its nodes

Moved to [ADR 0036](adr/0036-node-and-device-identity.md#the-core-and-its-nodes).

### A device's ssh

Moved to [ADR 0037](adr/0037-device-ssh.md#a-devices-ssh).

### WSL distributions as devices

Moved to [ADR 0038](adr/0038-wsl-devices.md#wsl-distributions-as-devices).

### A core on another machine

Moved to [ADR 0039](adr/0039-core-on-another-machine.md#a-core-on-another-machine).

### Splitting a moved core's two machines

Moved to [ADR 0040](adr/0040-splitting-a-moved-core.md#splitting-a-moved-cores-two-machines).

### The peer connection

Moved to [ADR 0041](adr/0041-peer-connection.md#the-peer-connection).

### Another machine's screen

Moved to [ADR 0042](adr/0042-another-machines-screen.md#another-machines-screen).

### Which connection may send what

Moved to [ADR 0041](adr/0041-peer-connection.md#which-connection-may-send-what).

### A guest's panes on the host

Moved to [ADR 0043](adr/0043-guest-node-on-the-host.md#a-guests-panes-on-the-host).

### Whose work is whose between machines

Moved to [ADR 0044](adr/0044-whose-work-between-machines.md#whose-work-is-whose-between-machines).

### A guest's build on the host

Moved to [ADR 0043](adr/0043-guest-node-on-the-host.md#a-guests-build-on-the-host).

### A guest's hook entries

Moved to [ADR 0043](adr/0043-guest-node-on-the-host.md#a-guests-hook-entries).

### What a guest may take

Moved to [ADR 0043](adr/0043-guest-node-on-the-host.md#what-a-guest-may-take).

### A guest taken off the host

Moved to [ADR 0043](adr/0043-guest-node-on-the-host.md#a-guest-taken-off-the-host).

### A guest node's events while its core is away

Moved to [ADR 0043](adr/0043-guest-node-on-the-host.md#a-guest-nodes-events-while-its-core-is-away).

### The two cores' stream

Moved to [ADR 0041](adr/0041-peer-connection.md#the-two-cores-stream).

### Switching a helper device to its machine's own Hide

Moved to [ADR 0045](adr/0045-helper-to-own-hide.md#switching-a-helper-device-to-its-machines-own-hide).

### Updating the core

Moved to [ADR 0008](adr/0008-updating-the-core.md#updating-the-core).

### A paired machine's core stays on

Moved to [ADR 0008](adr/0008-updating-the-core.md#a-paired-machines-core-stays-on).

### The terminal path

Moved to [ADR 0014](adr/0014-terminal-path.md#the-terminal-path).

### Agent labels in the core

Moved to [ADR 0021](adr/0021-agent-labels.md#agent-labels-in-the-core).

### Optional private features

The public workspace and the official free app do not link a Factory engine or expose Factory commands.
A separate private repository keeps the removed implementation and internal development resources.
There is no runtime replacement or Pro protocol in this release.
The legacy store files are left where they are; their presence starts no engine.
Memory retires its own worktrees through the ordinary `Call::WorktreeRetire` machine operation.

### The mobile companion

Moved to [ADR 0059](adr/0059-mobile-companion.md#the-mobile-companion).

### Panes, tabs and the attach window in the web shell

Moved to [ADR 0048](adr/0048-web-panes-tabs-attach.md#panes-tabs-and-the-attach-window-in-the-web-shell).

### Settings and workspace management in the web shell

Moved to [ADR 0049](adr/0049-web-settings-workspace-mgmt.md#settings-and-workspace-management-in-the-web-shell).

#### Project guardrails

Moved to [ADR 0049](adr/0049-web-settings-workspace-mgmt.md#project-guardrails).

### Workspaces in the web shell

Moved to [ADR 0050](adr/0050-web-workspaces.md#workspaces-in-the-web-shell).

### The `$HOME` filesystem boundary

Moved to [ADR 0052](adr/0052-home-boundary.md#the-home-filesystem-boundary).

### The checkout-root boundary and the Explorer frames

Moved to [ADR 0053](adr/0053-checkout-root-boundary.md#the-checkout-root-boundary-and-the-explorer-frames).

### The Explorer, editor and viewers

Moved to [ADR 0054](adr/0054-explorer-editor-viewers.md#the-explorer-editor-and-viewers).

### Device file hosts and document saves

Moved to [ADR 0047](adr/0047-device-file-hosts.md#device-file-hosts-and-document-saves).

### The install kit

Moved to [ADR 0057](adr/0057-install-kit.md#the-install-kit).

#### The store of skills and MCP items

Moved to [ADR 0057](adr/0057-install-kit.md#the-store-of-skills-and-mcp-items).

### Device catalogs

Moved to [ADR 0046](adr/0046-device-catalogs.md#device-catalogs).

### Retiring Home

Moved to [ADR 0055](adr/0055-retiring-home.md#retiring-home).

### The iOS app host

Moved to [ADR 0060](adr/0060-ios-app-host.md#the-ios-app-host).

### The Android app host

Moved to [ADR 0061](adr/0061-android-app-host.md#the-android-app-host).

### The desktop host

Moved to [ADR 0056](adr/0056-desktop-host.md#the-desktop-host).

### The shortcut registry

Moved to [ADR 0051](adr/0051-shortcut-registry.md#the-shortcut-registry).
